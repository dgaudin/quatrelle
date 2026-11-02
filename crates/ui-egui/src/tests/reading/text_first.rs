//! UI tests: a big message's text ahead of the whole of it (from upstream 5951534).

use super::*;
use egui::accesskit::Role;
use quatrelle_engine::mime::{self, Draft, PendingFile};
use quatrelle_engine::model::{EmailAddress, Event as Mail};

/// What a frame shows, as AccessKit reads it: each node's role, label and place.
struct Shown(Vec<(Role, String, Option<egui::Rect>)>);

impl Shown {
    fn says(&self, words: &str) -> bool {
        self.0.iter().any(|(_, l, _)| l.contains(words))
    }

    fn spinners(&self) -> usize {
        self.0.iter().filter(|(r, _, _)| *r == Role::ProgressIndicator).count()
    }

    /// The middle of the one node labelled `label`.
    fn at(&self, label: &str) -> egui::Pos2 {
        let found: Vec<egui::Rect> = self.0.iter().filter(|(_, l, _)| l == label).filter_map(|(_, _, b)| *b).collect();
        assert_eq!(found.len(), 1, "{label}: {:?}", self.0.iter().map(|(_, l, _)| l).collect::<Vec<_>>());
        found[0].center()
    }
}

fn shown(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> Shown {
    shown_sized(ctx, a, events, egui::vec2(1280.0, 800.0))
}

fn shown_sized(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>, size: egui::Vec2) -> Shown {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
    Shown(
        tree.nodes
            .iter()
            .map(|(_, n)| {
                let label = n.label().or(n.value()).unwrap_or_default().to_string();
                let place = n.bounds().map(|b| egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)));
                (n.role(), label, place)
            })
            .collect(),
    )
}

fn click(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) {
    press(ctx, a, at, egui::PointerButton::Primary);
}

fn press(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2, which: egui::PointerButton) {
    let button = |pressed| Event::PointerButton { pos: at, button: which, pressed, modifiers: Modifiers::NONE };
    frame(ctx, a, vec![Event::PointerMoved(at)]);
    frame(ctx, a, vec![button(true)]);
    frame(ctx, a, vec![button(false)]);
}

/// The first message of the sample opened as one not downloaded yet, its text here (`d`, with
/// `files` to come) and its whole message not: the key, and that message's source.
fn in_its_text_phase(a: &mut MailApp, d: &Draft, files: &[PendingFile]) -> (MessageKey, Vec<u8>) {
    let key = a.session.list.first().map(|m| m.key.clone()).unwrap();
    let st = a.session.store.as_ref().unwrap();
    let whole = st.read_body(&key).unwrap();
    st.remove_body(&key);
    a.session.open_message(&key);
    let from = EmailAddress::new("Sam", "sam@studio.example");
    let raw = mime::text_first(d, &from, NOW, "<r@x>", files).unwrap();
    a.session.apply(Mail::TextFirst { key: key.clone(), raw });
    (key, whole)
}

/// The worker's last step: the source on disk, then the event.
fn whole_arrives(a: &mut MailApp, key: &MessageKey, raw: &[u8]) {
    a.session.store.as_ref().unwrap().write_body(key, raw).unwrap();
    a.session.apply(Mail::Body { key: key.clone(), raw: raw.to_vec() });
}

fn rushes() -> (Draft, PendingFile) {
    let d = Draft { subject: "Rushes".into(), body: "Here's the reel.".into(), ..Default::default() };
    let file = PendingFile { name: "reel.mov".into(), mime: "video/quicktime".into(), size: 12 << 20, ..Default::default() };
    (d, file)
}

/// A message whose text came first shows it, its files named and sized, says they're
/// downloading, and offers no Save all; the whole message, once it comes, takes its place.
#[test]
fn a_message_text_first_says_its_files_are_coming() {
    let (mut a, dir) = app("text-first");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let (d, file) = rushes();
    let other = PendingFile { name: "notes.txt".into(), mime: "text/plain".into(), size: 300, ..Default::default() };
    let (key, whole) = in_its_text_phase(&mut a, &d, &[file, other]);
    frame(&ctx, &mut a, vec![]);
    let s = shown(&ctx, &mut a, vec![]);
    assert!(s.says("Here's the reel"), "the text");
    assert!(s.says("Downloading the attachments"));
    assert!(s.says("2 attachments"));
    assert!(!s.says("Save all"), "nothing to save yet");
    whole_arrives(&mut a, &key, &whole);
    frame(&ctx, &mut a, vec![]);
    let s = shown(&ctx, &mut a, vec![]);
    assert!(!s.says("Downloading the attachments"));
    assert!(!s.says("Here's the reel"), "the whole message took its place");
    let _ = std::fs::remove_dir_all(dir);
}

/// An invitation whose text came first (a Graph one without its type, or any message with an
/// `.ics` file) has no card yet, and none is kept as missing: the card shows once the whole
/// message comes, with no calendar change needed.
#[test]
fn an_invitation_shows_its_card_once_the_whole_message_comes() {
    let (mut a, dir) = app("text-first-invite");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let (d, _) = rushes();
    let ics = PendingFile { name: "invite.ics".into(), mime: "text/calendar".into(), size: 600, ..Default::default() };
    let (key, _) = in_its_text_phase(&mut a, &d, &[ics]);
    frame(&ctx, &mut a, vec![]);
    let s = shown(&ctx, &mut a, vec![]);
    assert!(s.says("Here's the reel") && !s.says("Grading session"), "no card yet");
    let calendar = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:grading-1@studio.example\r\n\
                    DTSTAMP:20261101T090000Z\r\nDTSTART:20261105T100000Z\r\nDTEND:20261105T110000Z\r\nSUMMARY:Grading session\r\n\
                    ORGANIZER:mailto:sam@studio.example\r\nATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:me@studio.example\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let whole = format!(
        "From: Sam <sam@studio.example>\r\nTo: me@studio.example\r\nSubject: Rushes\r\nMessage-ID: <r@x>\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nHere's the reel.\r\n\
         --b\r\nContent-Type: text/calendar; method=REQUEST; name=invite.ics\r\nContent-Disposition: attachment; filename=invite.ics\r\n\r\n\
         {calendar}--b--\r\n"
    );
    whole_arrives(&mut a, &key, whole.as_bytes());
    frame(&ctx, &mut a, vec![]);
    let s = shown(&ctx, &mut a, vec![]);
    assert!(s.says("Grading session"), "the card: {:?}", s.0.iter().map(|(_, l, _)| l).filter(|l| !l.is_empty()).collect::<Vec<_>>());
    let _ = std::fs::remove_dir_all(dir);
}

/// A picture shown in the text waits for the whole message: a spinner, not the placeholder, and
/// nothing kept of it, so it shows once the whole message comes.
#[test]
fn a_picture_waits_for_the_whole_message() {
    let (mut a, dir) = app("text-first-picture");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let html = "<p>The still we chose:</p><p><img src=\"cid:still@x\"></p>";
    let d = Draft { subject: "Still".into(), body: "The still we chose:".into(), html: Some(html.into()), ..Default::default() };
    let still = PendingFile { name: "still.png".into(), mime: "image/png".into(), size: 4096, content_id: Some("still@x".into()), inline: true };
    let (key, _) = in_its_text_phase(&mut a, &d, &[still]);
    frame(&ctx, &mut a, vec![]);
    let s = shown(&ctx, &mut a, vec![]);
    assert!(s.says("The still we chose"));
    assert!(!s.says("[picture]"), "no placeholder");
    assert!(s.says("Downloading the attachments"), "said though no file is listed");
    assert_eq!(s.spinners(), 2, "the files' and the picture's");
    // The whole message: the picture in it.
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(40, 30, image::Rgba([200, 90, 30, 255])))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let picture = mime::InlinePicture { cid: "still@x".into(), name: "still.png".into(), mime: "image/png".into(), data: png };
    let whole = mime::build(&Draft { inline: vec![picture], ..d }, &EmailAddress::new("Sam", "sam@studio.example"), NOW, "<r@x>").unwrap();
    whole_arrives(&mut a, &key, &whole);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let id = key.to_id();
    assert!(
        a.ui.shown_pictures.iter().any(|(name, t)| name.contains(&id) && t.is_some()),
        "the picture shows: {:?}",
        a.ui.shown_pictures.iter().map(|(n, t)| (n, t.is_some())).collect::<Vec<_>>()
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// The whole message couldn't be had: the text stays, with why and Try again.
#[test]
fn a_failure_keeps_the_text() {
    let (mut a, dir) = app("text-first-failed");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let (mut d, file) = rushes();
    d.html = Some("<p>Here's the reel.</p><p><img src=\"cid:still@x\"></p>".into());
    let still = PendingFile { name: "still.png".into(), mime: "image/png".into(), size: 4096, content_id: Some("still@x".into()), inline: true };
    let (key, _) = in_its_text_phase(&mut a, &d, &[file, still]);
    a.session.apply(Mail::BodyFailed { key, error: "the server hung up".into() });
    frame(&ctx, &mut a, vec![]);
    let s = shown(&ctx, &mut a, vec![]);
    assert!(s.says("Here's the reel"), "the text stays");
    assert!(s.says("The attachments couldn't be downloaded: the server hung up"));
    assert!(s.says("Try again"));
    assert!(!s.says("Downloading the attachments"));
    // Nothing waits for ever: the picture that won't come is its placeholder.
    assert_eq!(s.spinners(), 0);
    assert!(s.says("[picture]"));
    let _ = std::fs::remove_dir_all(dir);
}

/// A chip clicked while its file is on its way opens no preview: it says the file is coming.
#[test]
fn a_chip_says_it_is_coming() {
    let (mut a, dir) = app("text-first-chip");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let (d, file) = rushes();
    in_its_text_phase(&mut a, &d, &[file]);
    frame(&ctx, &mut a, vec![]);
    let more = shown(&ctx, &mut a, vec![]).at("More options");
    a.session.notices.clear();
    click(&ctx, &mut a, more - egui::vec2(120.0, 0.0));
    assert!(a.ui.attachment_view.is_none(), "no preview");
    assert_eq!(a.session.notices.last().map(|n| n.text.as_str()), Some(quatrelle_engine::STILL_COMING));
    let _ = std::fs::remove_dir_all(dir);
}

/// Reply with Meeting would leave out whoever has no address yet: it waits for the whole message.
#[test]
fn reply_with_meeting_waits_for_the_whole_message() {
    let (mut a, dir) = app("text-first-meeting");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let (mut d, file) = rushes();
    d.to = vec![EmailAddress::new("Ann Lee", "")];
    in_its_text_phase(&mut a, &d, &[file]);
    frame(&ctx, &mut a, vec![]);
    let more = shown(&ctx, &mut a, vec![]).at("More actions");
    click(&ctx, &mut a, more);
    let item = shown(&ctx, &mut a, vec![]).at("Reply with meeting");
    a.session.notices.clear();
    click(&ctx, &mut a, item);
    assert!(a.ui.dialog.is_none(), "no meeting");
    assert_eq!(a.session.notices.last().map(|n| n.text.as_str()), Some(quatrelle_engine::STILL_COMING));
    let _ = std::fs::remove_dir_all(dir);
}

/// In a message's own window too, a chip says its file is coming, and the reading pane is left
/// as it was: the window's message isn't opened there again (nor fetched again).
#[test]
fn a_chip_in_a_window_says_it_is_coming() {
    let (mut a, dir) = app("text-first-window");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let key = a.session.list.first().map(|m| m.key.clone()).unwrap();
    let plain = a.session.list.iter().find(|m| m.key != key && !m.has_attachments).map(|m| m.key.clone()).unwrap();
    a.session.store.as_ref().unwrap().remove_body(&key);
    a.run("message.popOut", json!({"key": key.to_id()}));
    a.run("list.open", json!({"key": plain.to_id()}));
    let (d, file) = rushes();
    let raw = mime::text_first(&d, &EmailAddress::new("Sam", "sam@studio.example"), NOW, "<r@x>", &[file]).unwrap();
    a.session.apply(Mail::TextFirst { key: key.clone(), raw });
    assert!(a.session.windows.iter().any(|w| w.key == key && w.text_first));
    frame(&ctx, &mut a, vec![]);
    let more = shown(&ctx, &mut a, vec![]).at("More options");
    a.session.notices.clear();
    click(&ctx, &mut a, more - egui::vec2(120.0, 0.0));
    assert_eq!(a.session.notices.last().map(|n| n.text.as_str()), Some(quatrelle_engine::STILL_COMING));
    let r = a.session.reading.as_ref().unwrap();
    assert!(r.key == plain && r.parsed.is_some(), "the reading pane is as it was");
    assert!(!a.session.take_jobs().iter().any(|j| matches!(j, quatrelle_engine::model::Job::FetchTextFirst { .. })), "nothing fetched again");
    let _ = std::fs::remove_dir_all(dir);
}

/// A long error in a narrow window still leaves Try again in reach.
#[test]
fn try_again_stays_in_a_narrow_pane() {
    let (mut a, dir) = app("text-first-narrow");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let size = egui::vec2(900.0, 700.0);
    frame_sized(&ctx, &mut a, vec![], size);
    let (d, file) = rushes();
    let (key, _) = in_its_text_phase(&mut a, &d, &[file]);
    let error = "the server answered with something long: ".to_string() + &"a gateway between here and there gave up ".repeat(8);
    a.session.apply(Mail::BodyFailed { key, error });
    frame_sized(&ctx, &mut a, vec![], size);
    let s = shown_sized(&ctx, &mut a, vec![], size);
    let button = s.0.iter().find(|(_, l, _)| l == "Try again").and_then(|(_, _, b)| *b).expect("Try again");
    assert!(button.right() <= size.x && button.left() >= 0.0, "{button:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Services that count the times a place to save is asked for.
struct Asking(std::rc::Rc<std::cell::Cell<usize>>);

impl crate::Services for Asking {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
    fn save_path(&mut self, _: &str) -> Option<std::path::PathBuf> {
        self.0.set(self.0.get() + 1);
        None
    }
    fn pick_folder(&mut self) -> Option<std::path::PathBuf> {
        self.0.set(self.0.get() + 1);
        None
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Err("refused".into())
    }
}

/// Save As on a file still coming says so before asking where: there'd be nothing to save.
#[test]
fn save_as_says_it_is_coming_before_asking_where() {
    let (mut a, dir) = app("text-first-save");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let asked = std::rc::Rc::new(std::cell::Cell::new(0));
    a.services = Box::new(Asking(asked.clone()));
    let (d, file) = rushes();
    in_its_text_phase(&mut a, &d, &[file]);
    frame(&ctx, &mut a, vec![]);
    let more = shown(&ctx, &mut a, vec![]).at("More options");
    press(&ctx, &mut a, more - egui::vec2(120.0, 0.0), egui::PointerButton::Secondary);
    let item = shown(&ctx, &mut a, vec![]).at("Save As…");
    a.session.notices.clear();
    click(&ctx, &mut a, item);
    assert_eq!(asked.get(), 0, "nowhere asked");
    assert_eq!(a.session.notices.last().map(|n| n.text.as_str()), Some(quatrelle_engine::STILL_COMING));
    // The rest that asks where: the same.
    for act in
        [crate::Action::SaveAll, crate::Action::SaveAttachments(vec![0]), crate::Action::SaveMessage, crate::Action::ExportPdf, crate::Action::Print]
    {
        a.act(act);
    }
    assert_eq!(asked.get(), 0, "nowhere asked");
    let _ = std::fs::remove_dir_all(dir);
}
