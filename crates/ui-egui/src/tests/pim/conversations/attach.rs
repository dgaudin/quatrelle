// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: files and pictures going with a message. The paperclip asks the window for the
//! system's file dialog, whose answer comes back as chips over the box (name and size, a cross
//! that takes each away); a picture pasted is written to the user's own folder and goes inside
//! the message (a big one as a file); Enter sends them with the text, or alone; what isn't taken
//! (an eleventh, a file too big, empty or not one) is said under the box; nothing is attached
//! while a message is edited; a send refused comes back with its files, and the drafts of other
//! chats keep theirs. A message going out with files says how far its upload is, or why it wasn't
//! sent. With `QUATRELLE_FRAME_PNG` set to a folder, the box with chips and a message uploading
//! are painted there, light and dark.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use quatrelle_engine::model::pim::teams::{ChatMessage, FILE_MAX, OUTBOX_MAX, OutKind, OutPart, OutState, Outgoing, PICTURE_INLINE_MAX, Unsent};

use super::super::list::follow_up::{Textures, frame_with_size};
use super::chat::*;
use super::writing::{SIZE, asked, enter, focus_box, in_box, one_frame, sends, typed, writing_app};
use super::*;
use crate::conversations::{TeamsAct, add_files, add_picture, draft};
use crate::{Action, FileAsk};

/// The file dialogs asked for, the answer the next frame gets, the picture on the clipboard and
/// how many times it was read.
#[derive(Default)]
struct Desk {
    asked: Vec<FileAsk>,
    answer: Option<Vec<PathBuf>>,
    clipboard: Option<Vec<u8>>,
    reads: usize,
}

/// A desktop whose file dialog and clipboard the test answers for.
struct Desktop(Rc<RefCell<Desk>>);

impl crate::Services for Desktop {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn open_path(&mut self, _: &Path) -> Result<(), String> {
        Ok(())
    }
    fn ask_file(&mut self, ask: FileAsk) -> bool {
        self.0.borrow_mut().asked.push(ask);
        true
    }
    fn take_file_answer(&mut self) -> Option<Vec<PathBuf>> {
        self.0.borrow_mut().answer.take()
    }
    fn clipboard_picture(&mut self) -> Option<Vec<u8>> {
        let mut desk = self.0.borrow_mut();
        desk.reads += 1;
        desk.clipboard.clone()
    }
}

/// The support chat open with `messages` on that desktop: the app, its folder, a folder of files
/// to attach in it, and the desk.
fn attach_app(name: &str, messages: Vec<ChatMessage>) -> (MailApp, PathBuf, PathBuf, Rc<RefCell<Desk>>) {
    let (mut a, dir) = writing_app(name, messages);
    let desk = Rc::new(RefCell::new(Desk::default()));
    a.services = Box::new(Desktop(desk.clone()));
    let files = dir.join("files");
    std::fs::create_dir_all(&files).unwrap();
    (a, dir, files, desk)
}

/// A file `name` of `size` bytes in `folder`.
fn file(folder: &Path, name: &str, size: usize) -> PathBuf {
    let path = folder.join(name);
    std::fs::write(&path, vec![b'x'; size]).unwrap();
    path
}

fn path_of(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// What goes with the open chat's message: (name, size, a picture).
fn parts(a: &MailApp) -> Vec<(String, u64, bool)> {
    a.ui.conversations.drafts.get(CHAT).map(|d| d.parts.iter().map(|p| (p.name.clone(), p.size, p.picture)).collect()).unwrap_or_default()
}

fn part(name: &str, size: u64, picture: bool) -> (String, u64, bool) {
    (name.to_string(), size, picture)
}

/// The path of the open chat's part `i`.
fn part_path(a: &MailApp, i: usize) -> PathBuf {
    a.ui.conversations.drafts.get(CHAT).and_then(|d| d.parts.get(i)).map(|p| p.path.clone()).expect("the part")
}

/// What the box asked of the window.
fn teams_acts(acts: Vec<Action>) -> Vec<TeamsAct> {
    acts.into_iter()
        .filter_map(|x| match x {
            Action::Teams(t) => Some(t),
            _ => None,
        })
        .collect()
}

/// Where the pane's control named `label` is, in the middle of it.
fn control(ctx: &egui::Context, a: &mut MailApp, label: &str) -> Option<egui::Pos2> {
    ctx.enable_accesskit();
    let input =
        RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE.0 as f32, SIZE.1 as f32))), ..Default::default() };
    let mut acts = Vec::new();
    let mut out = ctx.run_ui(input, |ui| {
        egui::CentralPanel::default().show(ui, |ui| crate::panes::conversations::chat(&a.session, &mut a.ui, ui, &mut acts));
    });
    out.textures_delta.clear();
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    let b = nodes.iter().find(|(_, n)| n.label() == Some(label)).and_then(|(_, n)| n.bounds())?;
    Some(egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
}

/// A screenshot pasted: egui hands the window its pixels (Ctrl+V itself never reaches it).
fn screenshot(color: egui::Color32) -> (Event, Vec<u8>) {
    let shot = egui::ColorImage::filled([4, 3], color);
    let png = crate::reading::png_of(&shot).expect("a PNG");
    (Event::PasteImage(std::sync::Arc::new(shot)), png)
}

/// A message of the support chat going out, numbered `seq`, with `parts`.
fn going(seq: u64, text: &str, state: OutState, parts: Vec<OutPart>) -> Outgoing {
    Outgoing {
        seq,
        local: format!("l-{seq}"),
        chat: CHAT.into(),
        body: para(vec![words(text)]),
        mentions: Vec::new(),
        what: OutKind::Send,
        state,
        parts,
    }
}

fn out_file(name: &str, size: u64, sent: u64) -> OutPart {
    OutPart { name: name.into(), size, picture: false, sent }
}

fn shown_going(a: &mut MailApp, items: Vec<Outgoing>) {
    a.session.teams_apply(teams::Event::Outbox { chat: CHAT.into(), items, took: None });
}

/// The pasted pictures' files of the open chat's box, removed (they're in the user's own folder).
fn remove_pasted(a: &MailApp) {
    let folder = quatrelle_engine::store::scratch_dir("teams");
    for p in a.ui.conversations.drafts.values().flat_map(|d| &d.parts) {
        if p.path.parent() == folder.as_deref() {
            let _ = std::fs::remove_file(&p.path);
        }
    }
}

/// A click on the paperclip asks the window for files to attach to this chat; the system's dialog
/// opens, and the two files it answers are chips over the box, with their names and sizes.
#[test]
fn the_paperclip_asks_for_files_and_shows_them_as_chips() {
    let (mut a, dir, files, desk) = attach_app("teams-attach-clip", vec![]);
    let ctx = egui::Context::default();
    settled_at(&ctx, &mut a, SIZE);
    let clip = control(&ctx, &mut a, "Attach a file").expect("the paperclip");
    let asks = teams_acts(click_pane(&ctx, &mut a, clip, SIZE));
    assert!(matches!(asks.as_slice(), [TeamsAct::Attach { chat }] if chat == CHAT), "{asks:?}");
    for t in asks {
        a.act(Action::Teams(t));
    }
    assert_eq!(desk.borrow().asked, [FileAsk::Attach], "the system's dialog opened");
    assert!(parts(&a).is_empty(), "nothing until it's answered");
    let (report, notes) = (file(&files, "report.pdf", 2048), file(&files, "notes.txt", 100));
    desk.borrow_mut().answer = Some(vec![report, notes]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(parts(&a), [part("report.pdf", 2048, false), part("notes.txt", 100, false)]);
    let texts = settled_at(&ctx, &mut a, SIZE);
    for w in ["report.pdf", "2 KB", "notes.txt", "100 B"] {
        assert!(has_text(&texts, w), "{w}: {texts:?}");
    }
    let (chip, hint) = (rect_of(&texts, |t| t == "report.pdf").unwrap(), rect_of(&texts, |t| t == "Write a message").unwrap());
    assert!(chip.bottom() <= hint.top(), "over the box: {chip:?} {hint:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A chip's cross takes it away, and the message goes without it.
#[test]
fn a_chip_is_removed_by_its_cross() {
    let (mut a, dir, files, _) = attach_app("teams-attach-remove", vec![]);
    let (report, notes) = (file(&files, "report.pdf", 2048), file(&files, "notes.txt", 100));
    add_files(draft(&mut a.ui.conversations, CHAT), vec![report, notes.clone()]);
    let ctx = egui::Context::default();
    settled_at(&ctx, &mut a, SIZE);
    let cross = control(&ctx, &mut a, "Remove: report.pdf").expect("its cross");
    click_pane(&ctx, &mut a, cross, SIZE);
    assert_eq!(parts(&a), [part("notes.txt", 100, false)], "the other stays");
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    assert!(!has_text(&texts, "report.pdf") && has_text(&texts, "notes.txt"), "{texts:?}");
    focus_box(&ctx, &mut a);
    let acts = typed(&ctx, &mut a, vec![enter()]);
    assert_eq!(sends(&acts), vec![json!({"chat": CHAT, "text": "", "bold": [], "italic": [], "files": [path_of(&notes)]})]);
    let _ = std::fs::remove_dir_all(dir);
}

/// "here", a file and a screenshot pasted (egui hands the window its pixels): the screenshot itself
/// is the picture, the clipboard isn't read again; Enter sends them together, the file under
/// `files` and the picture under `pictures`; the box and its chips are empty after.
#[test]
fn enter_sends_text_files_and_pictures() {
    let (mut a, dir, files, desk) = attach_app("teams-attach-send", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![Event::Text("here".into())]);
    let report = file(&files, "report.pdf", 2048);
    add_files(draft(&mut a.ui.conversations, CHAT), vec![report.clone()]);
    // Something else on the clipboard by now: never what goes.
    desk.borrow_mut().clipboard = Some(quatrelle_engine::sample::sample_png());
    let (paste, png) = screenshot(egui::Color32::RED);
    let asks = teams_acts(typed(&ctx, &mut a, vec![paste]));
    assert!(matches!(asks.as_slice(), [TeamsAct::Paste { chat, .. }] if chat == CHAT), "{asks:?}");
    for t in asks {
        a.act(Action::Teams(t));
    }
    let pasted = part_path(&a, 1);
    assert_eq!(pasted.parent(), quatrelle_engine::store::scratch_dir("teams").as_deref(), "in the user's own folder");
    let name = pasted.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
    assert!(name.starts_with("pasted-") && name.ends_with(".png"), "{name}");
    assert_eq!(std::fs::read(&pasted).ok(), Some(png), "the screenshot pasted");
    assert_eq!(desk.borrow().reads, 0, "the clipboard isn't read again");
    let acts = typed(&ctx, &mut a, vec![enter()]);
    assert_eq!(
        sends(&acts),
        vec![json!({"chat": CHAT, "text": "here", "bold": [], "italic": [], "files": [path_of(&report)], "pictures": [path_of(&pasted)]})]
    );
    assert_eq!((in_box(&a), parts(&a)), (String::new(), vec![]), "the box and its chips empty after");
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    assert!(!has_text(&texts, "report.pdf"), "{texts:?}");
    let _ = std::fs::remove_file(&pasted);
    let _ = std::fs::remove_dir_all(dir);
}

/// No text, a file: Enter sends the file alone.
#[test]
fn files_alone_can_be_sent() {
    let (mut a, dir, files, _) = attach_app("teams-attach-alone", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    let report = file(&files, "report.pdf", 2048);
    add_files(draft(&mut a.ui.conversations, CHAT), vec![report.clone()]);
    let acts = typed(&ctx, &mut a, vec![enter()]);
    assert_eq!(sends(&acts), vec![json!({"chat": CHAT, "text": "", "bold": [], "italic": [], "files": [path_of(&report)]})]);
    assert!(parts(&a).is_empty());
    // Nothing at all: nothing sent.
    assert!(sends(&typed(&ctx, &mut a, vec![enter()])).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// Eleven files picked: ten go with the message, and the box says at most ten go; a picture
/// pasted then isn't taken either. Typing lets the sentence go.
#[test]
fn an_eleventh_file_isnt_added_and_the_box_says_so() {
    let (mut a, dir, files, _) = attach_app("teams-attach-eleven", vec![]);
    let paths: Vec<PathBuf> = (1..=11).map(|i| file(&files, &format!("f{i:02}.txt"), 10)).collect();
    add_files(draft(&mut a.ui.conversations, CHAT), paths);
    let names: Vec<String> = parts(&a).into_iter().map(|(n, ..)| n).collect();
    assert_eq!(names, (1..=10).map(|i| format!("f{i:02}.txt")).collect::<Vec<_>>());
    let ctx = egui::Context::default();
    let said = "At most 10 files and pictures go with a message";
    assert!(has_text(&settled_at(&ctx, &mut a, SIZE), said));
    add_picture(draft(&mut a.ui.conversations, CHAT), &quatrelle_engine::sample::sample_png());
    assert_eq!(parts(&a).len(), 10, "no picture past ten either");
    assert!(has_text(&settled_at(&ctx, &mut a, SIZE), said));
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![Event::Text("x".into())]);
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    assert!(!has_text(&texts, said), "said until the text changes: {texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A file past 100 MiB, an empty one and a folder aren't taken, each said by its name; one of
/// exactly 100 MiB is.
#[test]
fn a_file_too_big_or_empty_isnt_added() {
    let (mut a, dir, files, _) = attach_app("teams-attach-sizes", vec![]);
    let sized = |name: &str, len: u64| {
        let path = files.join(name);
        std::fs::File::create(&path).unwrap().set_len(len).unwrap();
        path
    };
    let (big, most) = (sized("big.bin", FILE_MAX + 1), sized("most.bin", FILE_MAX));
    let empty = file(&files, "empty.txt", 0);
    let folder = files.join("folder");
    std::fs::create_dir_all(&folder).unwrap();
    add_files(draft(&mut a.ui.conversations, CHAT), vec![big, empty, folder, most]);
    assert_eq!(parts(&a), [part("most.bin", FILE_MAX, false)]);
    let ctx = egui::Context::default();
    let texts = settled_at(&ctx, &mut a, SIZE);
    for said in ["big.bin is bigger than Teams takes here (100 MB)", "empty.txt is empty", "folder can't be read"] {
        assert!(has_text(&texts, said), "{said}: {texts:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// A picture pasted of 1,000 bytes goes inside the message; one past 4 MiB as a file; each a file
/// of the user's own, readable by them only. A screenshot pasted asks the window to add it; text
/// pasted is text.
#[test]
fn a_pasted_picture_is_a_chip_and_a_big_one_goes_as_a_file() {
    let (mut a, dir, _, _) = attach_app("teams-attach-picture", vec![]);
    let big = vec![7u8; usize::try_from(PICTURE_INLINE_MAX).unwrap() + 1];
    add_picture(draft(&mut a.ui.conversations, CHAT), &[7u8; 1_000]);
    add_picture(draft(&mut a.ui.conversations, CHAT), &big);
    let (small_path, big_path) = (part_path(&a, 0), part_path(&a, 1));
    let name = |p: &Path| p.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
    assert_eq!(parts(&a), [part(&name(&small_path), 1_000, true), part(&name(&big_path), PICTURE_INLINE_MAX + 1, false)]);
    assert_eq!(std::fs::metadata(&big_path).map(|m| m.len()).ok(), Some(PICTURE_INLINE_MAX + 1));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&small_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the user's only");
    }
    let ctx = egui::Context::default();
    let texts = settled_at(&ctx, &mut a, SIZE);
    assert!(has_text(&texts, &name(&small_path)) && has_text(&texts, "1000 B"), "{texts:?}");
    focus_box(&ctx, &mut a);
    let acts = typed(&ctx, &mut a, vec![enter()]);
    assert_eq!(
        sends(&acts),
        vec![json!({"chat": CHAT, "text": "", "bold": [], "italic": [], "files": [path_of(&big_path)], "pictures": [path_of(&small_path)]})]
    );
    let asks = teams_acts(typed(&ctx, &mut a, vec![screenshot(egui::Color32::RED).0]));
    assert!(matches!(asks.as_slice(), [TeamsAct::Paste { chat, .. }] if chat == CHAT), "{asks:?}");
    let asks = teams_acts(one_frame(&ctx, &mut a, vec![Event::Paste("words".into())], 1));
    assert!(asks.is_empty(), "{asks:?}");
    assert_eq!(in_box(&a), "words");
    let _ = (std::fs::remove_file(small_path), std::fs::remove_file(big_path));
    let _ = std::fs::remove_dir_all(dir);
}

/// While one's own message is edited there's no paperclip, a paste asks nothing, and nothing can
/// be added (an edit takes no file); the chips of the message set aside come back with it.
#[test]
fn no_file_while_editing_a_message() {
    let mine = said("m2", user(ME, "Riley Moreno"), NOW - 300, para(vec![words("hello")]));
    let (mut a, dir, files, _) = attach_app("teams-attach-editing", vec![mine]);
    let ctx = egui::Context::default();
    let report = file(&files, "report.pdf", 2048);
    focus_box(&ctx, &mut a);
    assert!(control(&ctx, &mut a, "Attach a file").is_some(), "the paperclip, not editing");
    typed(&ctx, &mut a, vec![key(Key::ArrowUp, Modifiers::NONE)]);
    assert_eq!(in_box(&a), "hello", "editing");
    assert!(control(&ctx, &mut a, "Attach a file").is_none(), "no paperclip while editing");
    assert!(teams_acts(typed(&ctx, &mut a, vec![screenshot(egui::Color32::RED).0])).is_empty(), "a paste asks nothing");
    add_picture(draft(&mut a.ui.conversations, CHAT), &quatrelle_engine::sample::sample_png());
    add_files(draft(&mut a.ui.conversations, CHAT), vec![report]);
    assert!(parts(&a).is_empty(), "nothing added");
    typed(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(control(&ctx, &mut a, "Attach a file").is_some(), "the edit let go: the paperclip is back");
    let _ = std::fs::remove_dir_all(dir);
}

/// A pasted picture's file goes when the box lets the picture go: its cross clicked, its draft let
/// go when drafts are many, or pushed past ten by a send refused coming back before it.
#[test]
fn a_pasted_picture_let_go_leaves_no_file() {
    let (mut a, dir, files, _) = attach_app("teams-attach-let-go", vec![]);
    let png = quatrelle_engine::sample::sample_png();
    let ctx = egui::Context::default();
    // Its cross.
    add_picture(draft(&mut a.ui.conversations, CHAT), &png);
    let crossed = part_path(&a, 0);
    let name = crossed.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
    settled_at(&ctx, &mut a, SIZE);
    let cross = control(&ctx, &mut a, &format!("Remove: {name}")).expect("its cross");
    click_pane(&ctx, &mut a, cross, SIZE);
    assert!(parts(&a).is_empty());
    assert!(!crossed.exists(), "its file is gone: {}", crossed.display());
    // Pushed past ten by a refused send's ten files.
    add_picture(draft(&mut a.ui.conversations, CHAT), &png);
    let pushed = part_path(&a, 0);
    a.session.teams_apply(teams::Event::Unsent { count: OUTBOX_MAX as u32 });
    let ten: Vec<String> = (1..=10).map(|i| path_of(&file(&files, &format!("f{i:02}.txt"), 10))).collect();
    a.act(Action::Run("teams.send", json!({"chat": CHAT, "text": "ten", "files": ten})));
    assert_eq!(parts(&a).len(), 10);
    assert!(!pushed.exists(), "its file is gone: {}", pushed.display());
    // Its draft let go: the drawn longest ago, once every draft holds something.
    let old = "19:old@thread.v2";
    add_picture(draft(&mut a.ui.conversations, old), &png);
    let evicted = a.ui.conversations.drafts.get(old).and_then(|d| d.parts.first()).map(|p| p.path.clone()).expect("its picture");
    for i in 0..40 {
        draft(&mut a.ui.conversations, &format!("19:c{i:02}@thread.v2")).text = "kept".into();
    }
    assert!(!a.ui.conversations.drafts.contains_key(old), "its draft went");
    assert!(!evicted.exists(), "its file is gone: {}", evicted.display());
    for p in [crossed, pushed, evicted] {
        let _ = std::fs::remove_file(p);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// A send the engine refuses comes back in the box with its text, its file and its picture (their
/// sizes read again; one no longer there left out, and said), and why; too many files is said in
/// the box's own words.
#[test]
fn a_refused_send_comes_back_with_its_files() {
    let (mut a, dir, files, _) = attach_app("teams-attach-refused", vec![]);
    let (report, shot) = (file(&files, "report.pdf", 2048), file(&files, "shot.png", 1_000));
    let gone = files.join("gone.pdf");
    a.session.teams_apply(teams::Event::Unsent { count: OUTBOX_MAX as u32 });
    let params =
        json!({"chat": CHAT, "text": "here", "bold": [], "italic": [], "files": [path_of(&report), path_of(&gone)], "pictures": [path_of(&shot)]});
    a.act(Action::Run("teams.send", params));
    assert_eq!(in_box(&a), "here");
    assert_eq!(parts(&a), [part("report.pdf", 2048, false), part("shot.png", 1_000, true)]);
    a.session.teams_apply(teams::Event::Unsent { count: 0 });
    let ctx = egui::Context::default();
    let texts = settled_at(&ctx, &mut a, SIZE);
    for w in ["report.pdf", "shot.png", "Too many messages are waiting", "gone.pdf can't be read"] {
        assert!(has_text(&texts, w), "{w}: {texts:?}");
    }
    // Eleven files: the engine's refusal said in the box's words, once; ten come back.
    a.ui.conversations.drafts.remove(CHAT);
    let eleven: Vec<String> = (1..=11).map(|i| path_of(&file(&files, &format!("f{i:02}.txt"), 10))).collect();
    a.act(Action::Run("teams.send", json!({"chat": CHAT, "text": "all", "files": eleven})));
    let said = "At most 10 files and pictures go with a message";
    assert_eq!(a.ui.conversations.drafts.get(CHAT).and_then(|d| d.said()), Some(said));
    assert_eq!(parts(&a).len(), 10);
    let texts = settled_at(&ctx, &mut a, SIZE);
    assert_eq!(texts.iter().filter(|(t, _)| t == said).count(), 1, "{texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Each chat's box keeps its own files: the open one sends its own, another's stay in its draft,
/// and a draft with files isn't let go as an empty one when drafts are many.
#[test]
fn drafts_of_other_chats_keep_their_files() {
    let (mut a, dir, files, _) = attach_app("teams-attach-drafts", vec![]);
    let other = "19:other@thread.v2";
    let (report, notes) = (file(&files, "report.pdf", 2048), file(&files, "notes.txt", 100));
    add_files(draft(&mut a.ui.conversations, other), vec![report.clone()]);
    add_files(draft(&mut a.ui.conversations, CHAT), vec![notes.clone()]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    let acts = typed(&ctx, &mut a, vec![enter()]);
    assert_eq!(sends(&acts), vec![json!({"chat": CHAT, "text": "", "bold": [], "italic": [], "files": [path_of(&notes)]})]);
    let kept = |a: &MailApp| a.ui.conversations.drafts.get(other).map(|d| d.parts.iter().map(|p| p.path.clone()).collect::<Vec<_>>());
    assert_eq!(kept(&a), Some(vec![report.clone()]), "the other chat's file stays");
    for i in 0..25 {
        draft(&mut a.ui.conversations, &format!("19:c{i:02}@thread.v2"));
    }
    assert_eq!(kept(&a), Some(vec![report]), "not let go as an empty draft");
    let _ = std::fs::remove_dir_all(dir);
}

/// A message whose files are going up says how far (84 of 200 bytes: 42 %), lists its files, and
/// offers Cancel; before any size is known it says "Uploading…".
#[test]
fn a_message_being_uploaded_shows_how_far_it_is() {
    let (mut a, dir, _, _) = attach_app("teams-attach-uploading", vec![]);
    let ctx = egui::Context::default();
    shown_going(&mut a, vec![going(7, "The figures", OutState::Uploading, vec![out_file("a.pdf", 100, 84), out_file("b.pdf", 100, 0)])]);
    let texts = settled_at(&ctx, &mut a, SIZE);
    for w in ["The figures", "a.pdf", "b.pdf", "100 B", "Uploading… 42 %", "Cancel"] {
        assert!(has_text(&texts, w), "{w}: {texts:?}");
    }
    assert!(!has_text(&texts, "Sending…") && !has_text(&texts, "Retry"), "{texts:?}");
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    let cancel = rect_of(&texts, |t| t == "Cancel").expect("Cancel");
    assert_eq!(asked(&click_pane(&ctx, &mut a, cancel.center(), SIZE), "teams.discard"), vec![json!({"seq": 7})]);
    shown_going(&mut a, vec![going(7, "The figures", OutState::Uploading, vec![out_file("a.pdf", 0, 0)])]);
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    assert!(has_text(&texts, "Uploading…"), "{texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// "Not sent" for a file says which reason, with Retry and Delete; one the outbox refused to take
/// with its files (number 0) can only be deleted: its files are attached again.
#[test]
fn not_sent_for_a_file_says_which_reason() {
    let (mut a, dir, _, _) = attach_app("teams-attach-not-sent", vec![]);
    let ctx = egui::Context::default();
    settled_at(&ctx, &mut a, SIZE);
    for (reason, said) in [
        (Unsent::FileGone, "A file is no longer there, or has changed"),
        (Unsent::FileTooBig, "A file is bigger than Teams takes here (100 MB)"),
        (Unsent::NotShared, "OneDrive didn't share the file with everyone in this conversation"),
    ] {
        let not_sent = OutState::NotSent { reason, code: None };
        shown_going(&mut a, vec![going(9, "The report", not_sent, vec![out_file("report.pdf", 2048, 0)])]);
        let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
        for w in ["Not sent", said, "report.pdf", "Retry", "Delete"] {
            assert!(has_text(&texts, w), "{reason:?}: {w}: {texts:?}");
        }
    }
    let not_sent = OutState::NotSent { reason: Unsent::TooMany, code: None };
    shown_going(&mut a, vec![going(0, "The report", not_sent, vec![out_file("report.pdf", 0, 0)])]);
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    assert!(has_text(&texts, "Attach the files again to send it") && has_text(&texts, "Delete"), "{texts:?}");
    assert!(!has_text(&texts, "Retry"), "{texts:?}");
    // Retry asked all the same (a script): the engine's refusal said in the box's words.
    a.act(Action::Run("teams.retry", json!({"seq": 0, "local": "l-0"})));
    assert_eq!(a.ui.conversations.drafts.get(CHAT).and_then(|d| d.said()), Some("Attach the files again to send it"));
    let _ = std::fs::remove_dir_all(dir);
}

/// The box with a file and a pasted picture as chips, over a message going up and one not sent
/// for its file, light and dark.
#[test]
fn the_chips_and_an_upload_in_light_and_dark() {
    let messages = vec![said("m1", user("u-ana", "Ana Lima"), NOW - 600, para(vec![words("Can you send the figures?")]))];
    let (mut a, dir, files, _) = attach_app("teams-attach-frames", messages);
    shown_going(
        &mut a,
        vec![
            going(5, "Here they are", OutState::Uploading, vec![out_file("figures.xlsx", 3_000_000, 1_000_000), out_file("chart.pdf", 1_000_000, 0)]),
            going(6, "And the notes", OutState::NotSent { reason: Unsent::FileGone, code: None }, vec![out_file("notes.docx", 30_000, 0)]),
        ],
    );
    let plan = file(&files, "quarterly plan with a long name.pdf", 120_000);
    add_files(draft(&mut a.ui.conversations, CHAT), vec![plan]);
    add_picture(draft(&mut a.ui.conversations, CHAT), &quatrelle_engine::sample::sample_png());
    let ctx = egui::Context::default();
    let pictures_dir = std::env::var_os("QUATRELLE_FRAME_PNG").map(PathBuf::from);
    let mut tex = Textures::default();
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        a.ui.dark = Some(dark);
        for _ in 0..6 {
            frame_with_size(&ctx, &mut a, &mut tex, vec![], None, SIZE);
        }
        let png = pictures_dir.as_ref().map(|d| d.join(format!("teams-attach-{theme}.png")));
        let (_, shapes) = frame_with_size(&ctx, &mut a, &mut tex, vec![], png, SIZE);
        let texts = texts_of(&shapes);
        for w in ["Uploading… 25 %", "figures.xlsx", "chart.pdf", "Not sent", "A file is no longer there, or has changed", "117 KB"] {
            assert!(has_text(&texts, w), "{theme}: {w}: {texts:?}");
        }
    }
    a.ui.dark = Some(false);
    remove_pasted(&a);
    let _ = std::fs::remove_dir_all(dir);
}
