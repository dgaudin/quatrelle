//! UI tests: writing messages (the editor, recipients, the address book, date and words, several at once).

use super::*;

mod people_field;

/// A name typed into To over a big mailbox (16 folders of 4,000 messages, too many to count
/// again at the start): no frame reads the mailbox whole. The message's first frame starts the
/// address book's build; every frame after takes a slice, reading a folder or two, the window
/// drawing between them; the suggestions show once the book is complete, a few frames on.
/// Built whole, as it was, the frame of the first letter typed read every folder from disk at
/// once, and the window froze for as long as that took (seconds, on a mailbox of hundreds of
/// thousands).
#[test]
fn a_name_typed_into_to_never_reads_a_big_mailbox_in_one_frame() {
    let (mut a, dir) = app_big("suggest-big", 16, 4000);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert!(!a.session.contacts_pending(), "no message being written: the book isn't built");
    assert_eq!(a.session.cached_folders().len(), 1, "the Inbox on screen");
    a.run("mail.new", json!({}));
    let (mut frames, mut most, mut longest) = (0, 0, std::time::Duration::ZERO);
    while frames < 40 {
        let before = a.session.cached_folders();
        let began = std::time::Instant::now();
        frame(&ctx, &mut a, if frames == 2 { vec![Event::Text("ann1".into())] } else { vec![] });
        longest = longest.max(began.elapsed());
        most = most.max(a.session.cached_folders().iter().filter(|k| !before.contains(k)).count());
        frames += 1;
        if frames > 3 && !a.session.contacts_pending() {
            break;
        }
    }
    assert!(frames < 40, "the book is built in a few frames");
    assert!(most <= 4, "a frame reads the folder its slice is in, the next two at most: {most}");
    assert_eq!(a.session.compose.as_ref().unwrap().to, "ann1");
    frame(&ctx, &mut a, vec![]);
    let list =
        ctx.memory(|m| m.areas().visible_layer_ids().into_iter().filter(|l| l.order == egui::Order::Foreground).find_map(|l| m.area_rect(l.id)));
    assert!(list.is_some(), "the suggestions show once the book is built");
    println!("{frames} frames, the longest {longest:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_editor_formats_as_you_see_it_with_outlooks_keys() {
    use quatrelle_engine::mime::rich::{Align, List, PICTURE};
    let (mut a, dir) = app("editor");
    a.services = Box::new(Recorder(Default::default()));
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ann@x.example", "subject": "Plan", "body": ""}));
    frame(&ctx, &mut a, vec![]);
    let id = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    let compose_id = a.session.compose.as_ref().unwrap().id;
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![]);
    let c = |a: &MailApp| a.session.compose.clone().unwrap();
    // Bold on, then typing: bold text, and no marks in it.
    frame(&ctx, &mut a, vec![key(Key::B, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![Event::Text("Hello".into())]);
    assert_eq!(c(&a).body, "Hello");
    assert!(c(&a).styles.style(0).bold && c(&a).styles.style(4).bold);
    // Off again: plain.
    frame(&ctx, &mut a, vec![key(Key::B, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![Event::Text(" world".into())]);
    assert_eq!(c(&a).body, "Hello world");
    assert!(!c(&a).styles.style(7).bold);
    // Select "Hello" and underline it.
    let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(5))));
    state.store(&ctx, id);
    frame(&ctx, &mut a, vec![key(Key::U, Modifiers::COMMAND)]);
    assert!(c(&a).styles.style(2).underline && !c(&a).styles.style(7).underline);
    // Bullets (Ctrl+Shift+L), then undo (Ctrl+Z) and redo (Ctrl+Y).
    frame(&ctx, &mut a, vec![key(Key::L, Modifiers::COMMAND | Modifiers::SHIFT)]);
    assert_eq!(c(&a).styles.para(0).list, List::Bullet);
    frame(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    assert_eq!(c(&a).styles.para(0).list, List::None);
    frame(&ctx, &mut a, vec![key(Key::Y, Modifiers::COMMAND)]);
    assert_eq!(c(&a).styles.para(0).list, List::Bullet);
    // Ctrl+R aligns right while writing (it doesn't reply), Ctrl+E centres.
    frame(&ctx, &mut a, vec![key(Key::R, Modifiers::COMMAND)]);
    assert_eq!(c(&a).id, compose_id, "still the same message");
    assert_eq!(c(&a).styles.para(0).align, Align::Right);
    frame(&ctx, &mut a, vec![key(Key::E, Modifiers::COMMAND)]);
    assert_eq!(c(&a).styles.para(0).align, Align::Center);
    // Bigger (Ctrl+]), on "Hello" selected again (undo leaves the cursor collapsed).
    let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(5))));
    state.store(&ctx, id);
    frame(&ctx, &mut a, vec![key(Key::CloseBracket, Modifiers::COMMAND)]);
    assert_eq!(c(&a).styles.style(0).size(), 12);
    // Sent as HTML with what was seen.
    let (text, html, _) = c(&a).bodies();
    let html = html.unwrap();
    assert!(text.starts_with("• Hello world"), "{text}");
    assert!(html.contains("<ul>") && html.contains("<u>Hello</u>") && html.contains("text-align:center"), "{html}");
    // A screenshot pasted: Ctrl+V comes as the picture itself (there's no text on the
    // clipboard), and it goes in as a picture.
    let shot = egui::ColorImage::filled([4, 3], egui::Color32::RED);
    frame(&ctx, &mut a, vec![Event::PasteImage(std::sync::Arc::new(shot))]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(c(&a).inline.len(), 1, "the screenshot is in the message");
    // Ctrl+Shift+B: the address book.
    frame(&ctx, &mut a, vec![key(Key::B, Modifiers::COMMAND | Modifiers::SHIFT)]);
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(a.ui.dialog, Some(Dialog::SelectNames(_))));
    a.ui.dialog = None;
    // A pasted picture goes in at the cursor.
    a.act(crate::Action::PastePicture);
    frame(&ctx, &mut a, vec![]);
    assert!(c(&a).body.contains(PICTURE) && c(&a).inline.len() == 2);
    a.run("compose.format", json!({"plain": true}));
    frame(&ctx, &mut a, vec![]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_address_book_adds_people_to_to_cc_and_bcc() {
    let (mut a, dir) = app("names");
    let ctx = egui::Context::default();
    a.session.prefs.new_mail_window = false;
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.clone().unwrap();
    a.ui.dialog = Some(Dialog::select_names(&c, "to"));
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_mut() else { panic!("no dialog") };
    assert!(f.search.list.iter().any(|x| x.email == "zoe@studio.example"), "everyone mail was exchanged with: {:?}", f.search.list);
    f.search.query = "zoe".into();
    f.search.stale = true;
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_mut() else { panic!("no dialog") };
    assert!(!f.search.list.is_empty() && f.search.list.iter().all(|x| x.email.contains("zoe") || x.name.to_lowercase().contains("zoe")));
    f.to = "Zoe Park <zoe@studio.example>; ".into();
    f.cc = "ann@x.example".into();
    // OK: the fields go to the message.
    let p = json!({"to": f.to.trim(), "cc": f.cc.trim(), "bcc": ""});
    a.act(crate::Action::Submit("compose.update", p));
    let c = a.session.compose.clone().unwrap();
    assert!(c.to.contains("zoe@studio.example") && c.cc == "ann@x.example");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn delay_delivery_and_replies_to_draw_while_writing() {
    let (mut a, dir) = app("compose-delay");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    // Delay Delivery shows for a server account (the Outbox sends it later), with its time.
    let id = a.session.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    a.session.prefs.new_mail_window = false;
    a.run("mail.new", json!({}));
    a.run(
        "compose.update",
        json!({"account": id, "to": "ann@example.org", "subject": "Later", "replyTo": "desk@example.org", "sendAfter": NOW + 3600}),
    );
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(a.session.compose.as_ref().is_some_and(|c| c.send_after == Some(NOW + 3600) && c.reply_to == "desk@example.org"));
    assert_eq!(crate::reading::parse_local("2026-10-16", "08:30"), Some((20_742, 510)));
    assert_eq!(crate::reading::parse_local("2026-02-30", "08:30"), None);
    assert_eq!(crate::reading::parse_local("2026-10-16", "24:00"), None);
    let _ = std::fs::remove_dir_all(dir);
}

/// Writing: today's date or the time goes in at the cursor (a plain message stays plain, and
/// Undo takes it out); Word count counts the message, or what's selected.
#[test]
fn the_date_goes_in_and_words_are_counted() {
    let (mut a, dir) = app("date-words");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let choices = crate::rich_edit::date_choices(&a.session.clock);
    assert_eq!(choices.len(), 4);
    let iso = quatrelle_engine::model::dates::day_string(NOW.div_euclid(86_400));
    assert_eq!(choices[1], iso);
    assert!(choices[0].contains(&quatrelle_engine::model::dates::civil(NOW, &a.session.clock).year.to_string()));
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.as_mut().unwrap();
    c.body = "Hello  world.\n\nSecond paragraph here".into();
    c.plain = true;
    let mut ed = crate::rich_edit::Editor::new(c.id);
    ed.sel = 5..5;
    crate::rich_edit::apply(&mut ed, c, crate::rich_edit::Op::Insert(format!(" {iso}")));
    assert_eq!(c.body, format!("Hello {iso}  world.\n\nSecond paragraph here"));
    assert!(c.plain, "a plain message stays plain");
    assert_eq!(ed.sel, 5 + 1 + iso.chars().count()..5 + 1 + iso.chars().count());
    // Past the end, or a selection replaced.
    ed.sel = 10_000..10_000;
    crate::rich_edit::apply(&mut ed, c, crate::rich_edit::Op::Insert("!".into()));
    assert!(c.body.ends_with('!'));
    let n = crate::rich_edit::count_words(&c.body);
    assert_eq!((n.words, n.paragraphs, n.lines), (6, 2, 3), "{n:?}");
    assert_eq!(crate::rich_edit::count_words(""), crate::rich_edit::Counts::default());
    let w = crate::rich_edit::count_words("naïve café — 東京 2026 don't");
    assert_eq!((w.words, w.chars_no_spaces), (6, "naïvecafé—東京2026don't".chars().count()), "{w:?}");
    // Shown where the message is being written (its own window), as Address Book is.
    assert!(Dialog::WordCount(n, false).for_compose());
    a.ui.dialog = Some(Dialog::WordCount(n, false));
    frame(&ctx, &mut a, vec![]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Several messages written at once, each in its own window: a second New mail keeps the first;
/// what a window does goes to its own message; each keeps its editor (selection, undo); a
/// dialog opened from one shows in that one.
#[test]
fn several_messages_are_written_at_once() {
    let (mut a, dir) = app("several-composes");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "First", "to": "ana@soundlab.example"}));
    let first = a.session.compose.as_ref().unwrap().id;
    frame(&ctx, &mut a, vec![]);
    a.ui.editor.as_mut().unwrap().sel = 3..3;
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let second = a.session.compose.as_ref().unwrap().id;
    assert_ne!(first, second);
    assert_eq!(a.session.all_composes().count(), 2, "the first is kept");
    // Its window's action goes to it, not to the one acted on last.
    a.act(crate::Action::ForCompose(first, Box::new(crate::Action::Run("compose.update", json!({"subject": "First, changed"})))));
    let subjects: Vec<String> = a.session.all_composes().map(|c| c.subject.clone()).collect();
    assert!(subjects.contains(&"First, changed".to_string()) && subjects.contains(&String::new()), "{subjects:?}");
    // Its editor came back with it.
    assert_eq!(a.ui.editor.as_ref().map(|e| (e.id, e.sel.clone())), Some((first, 3..3)));
    // A dialog from the first's window belongs to it.
    a.act(crate::Action::ForCompose(first, Box::new(crate::Action::Dialog(Dialog::SaveChanges))));
    assert_eq!(a.ui.dialog_for, Some(first));
    a.ui.dialog = None;
    // Closing one leaves the other.
    a.act(crate::Action::ForCompose(second, Box::new(crate::Action::Run("compose.close", json!({"save": false})))));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.all_composes().map(|c| c.id).collect::<Vec<_>>(), vec![first]);
    // An action for a message gone does nothing.
    a.act(crate::Action::ForCompose(second, Box::new(crate::Action::Run("compose.update", json!({"subject": "ghost"})))));
    assert_eq!(a.session.compose.as_ref().unwrap().subject, "First, changed");
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Closing Quatrelle with messages being written asks first; Save keeps each as a draft.
#[test]
fn closing_with_messages_written_asks_and_keeps_them() {
    let (mut a, dir) = app("save-on-close");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Half done"}));
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Also half done"}));
    let close = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
        viewports: std::iter::once((egui::ViewportId::ROOT, egui::ViewportInfo { events: vec![egui::ViewportEvent::Close], ..Default::default() }))
            .collect(),
        ..Default::default()
    };
    let mut out = ctx.run_ui(close, |ui| a.ui(ui));
    out.textures_delta.clear();
    assert!(matches!(a.ui.dialog, Some(Dialog::SaveOnClose(2))), "asked");
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Run("compose.keepAll", json!({})));
    let drafts = a.session.account("sample").and_then(|x| x.folder(quatrelle_engine::model::FolderRole::Drafts)).map(|f| f.path.clone()).unwrap();
    let subjects: Vec<String> = a.session.folder_state("sample", &drafts).messages.iter().map(|m| m.subject.clone()).collect();
    assert!(subjects.contains(&"Half done".into()) && subjects.contains(&"Also half done".into()), "{subjects:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The message window's ribbon has Outlook's tabs, each drawing its groups over the message
/// being written (in the reading pane here).
#[test]
fn the_compose_ribbon_has_outlooks_tabs() {
    use crate::rich_edit::ComposeTab;
    let (mut a, dir) = app("compose-ribbon");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ann@x.example", "subject": "Plan", "body": "Hello"}));
    for tab in [ComposeTab::Message, ComposeTab::Insert, ComposeTab::Options, ComposeTab::FormatText, ComposeTab::Review] {
        frame(&ctx, &mut a, vec![]);
        if let Some(ed) = a.ui.editor.as_mut() {
            ed.tab = tab;
        }
        frame(&ctx, &mut a, vec![]);
        assert!(a.ui.editor.as_ref().is_some_and(|e| e.tab == tab), "{tab:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// A picture copied from a web page comes with its HTML alone: it's pasted as the picture, not
/// its address; HTML with text is pasted as text. A picture copied whole from a message is the
/// picture as it was, and a screenshot goes in a message as a PNG.
#[test]
fn pictures_go_through_the_clipboard_whole() {
    assert!(crate::reading::picture_alone("<html><body><img src=\"https://example.com/a.png\"></body></html>"));
    assert!(!crate::reading::picture_alone("<p>Hello <img src=\"cid:x\"></p>"));
    assert!(!crate::reading::picture_alone("<p>Just text</p>"));
    let shot = egui::ColorImage::filled([40, 30], egui::Color32::from_rgb(10, 120, 200));
    let png = crate::reading::png_of(&shot).unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    let back = crate::reading::color_image(&png).unwrap();
    assert_eq!((back.size, back.pixels[0]), ([40, 30], egui::Color32::from_rgb(10, 120, 200)));
    assert!(crate::reading::png_of(&egui::ColorImage::filled([0, 0], egui::Color32::RED)).is_none());
}

/// Attach Item: open on messages from the Inbox, a click picks one, Attach sends it along as a
/// file and closes the dialog; Contacts lists the address book.
#[test]
fn attach_item_picks_a_message_and_attaches_it() {
    let (mut a, dir) = app("attach-item");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let id = a.session.compose.as_ref().map(|c| c.id).unwrap();
    a.act(crate::Action::ForCompose(id, Box::new(crate::Action::AttachItem(quatrelle_engine::ItemKind::Message))));
    let size = egui::vec2(1280.0, 800.0);
    // Where a text is drawn last: in the dialog, over the list that shows the same subjects.
    let at = |shapes: &[egui::Shape], text: &str| {
        shapes.iter().rev().find_map(|s| match s {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.pos + egui::vec2(4.0, 6.0)),
            _ => None,
        })
    };
    let subject = "Storyboard review on Thursday";
    // Listed, and still once it has faded in (it moves while it does).
    let mut row = None;
    for _ in 0..30 {
        let now = at(&frame_shapes(&ctx, &mut a, vec![], size), subject);
        if now.is_some() && now == row {
            break;
        }
        row = now;
    }
    let row = row.expect("the Inbox's messages are listed");
    let click = |a: &mut MailApp, pos: egui::Pos2| {
        let button = |pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        frame(&ctx, a, vec![Event::PointerMoved(pos)]);
        frame(&ctx, a, vec![button(true)]);
        frame(&ctx, a, vec![button(false)]);
    };
    click(&mut a, row);
    let Some(Dialog::AttachItem(f)) = &a.ui.dialog else { panic!("the dialog stays open") };
    assert_eq!(f.picked.len(), 1, "picked");
    let attach = at(&frame_shapes(&ctx, &mut a, vec![], size), "Attach").expect("Attach");
    click(&mut a, attach);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.dialog.is_none(), "closed");
    let att = a.session.compose.as_ref().unwrap().attachments.clone();
    assert_eq!(att.len(), 1);
    assert_eq!(att[0].mime, "message/rfc822");
    assert_eq!(quatrelle_engine::mime::parse(&att[0].data).unwrap().subject, subject);
    // The address book, under Contacts.
    a.act(crate::Action::ForCompose(id, Box::new(crate::Action::AttachItem(quatrelle_engine::ItemKind::Contact))));
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::AttachItem(f)) = &a.ui.dialog else { panic!("open again") };
    assert!(!f.rows.is_empty() && f.rows.iter().all(|r| r.when == 0), "contacts listed: {:?}", f.rows);
    let _ = std::fs::remove_dir_all(dir);
}

/// Closed from the taskbar while minimized, when eframe runs `logic` alone and no `ui`: the close
/// still waits for the question, and the window comes back to ask it (craft-libs' quit crate:
/// eframe closed the window, and the messages being written were lost).
#[test]
fn closing_while_minimized_still_asks() {
    let (mut a, dir) = app("save-on-close-minimized");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Written while minimized"}));
    frame(&ctx, &mut a, vec![]);
    let viewport = egui::ViewportInfo { minimized: Some(true), events: vec![egui::ViewportEvent::Close], ..Default::default() };
    let close = RawInput { time: Some(99.0), viewports: std::iter::once((egui::ViewportId::ROOT, viewport)).collect(), ..Default::default() };
    let mut out = ctx.run_logic(&close, |ctx| a.logic(ctx));
    let commands = out.viewport_commands.remove(&egui::ViewportId::ROOT).unwrap_or_default();
    assert!(commands.contains(&egui::ViewportCommand::CancelClose), "the close waits: {commands:?}");
    assert!(commands.contains(&egui::ViewportCommand::Minimized(false)), "the window comes back: {commands:?}");
    assert!(matches!(a.ui.dialog, Some(Dialog::SaveOnClose(1))), "asked");
    // The window back, the same close comes in again with the next pass (eframe keeps the
    // input logic alone saw): asked once, still.
    let mut out = ctx.run_ui(close, |ui| a.ui(ui));
    out.textures_delta.clear();
    assert!(matches!(a.ui.dialog, Some(Dialog::SaveOnClose(1))));
    let _ = std::fs::remove_dir_all(dir);
}

/// macOS quits through `terminate:` rather than closing the window: a close nothing holds open
/// is taken over (once, the window kept for the quit), one a question holds open isn't, and a
/// frame with no close leaves the quit alone. Elsewhere `quit` says no and eframe closes.
#[test]
fn a_close_becomes_the_platform_quit_unless_something_asks() {
    let close = || RawInput {
        viewports: std::iter::once((egui::ViewportId::ROOT, egui::ViewportInfo { events: vec![egui::ViewportEvent::Close], ..Default::default() }))
            .collect(),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    // Nothing being written: the quit runs, and the window waits for it.
    let (mut a, dir) = app("quit-takes-over");
    let quits = std::cell::Cell::new(0);
    let mut out = ctx.run_logic(&close(), |ctx| {
        a.logic(ctx);
        assert!(MailApp::quit_instead_of_close(ctx, || {
            quits.set(quits.get() + 1);
            true
        }));
    });
    assert_eq!(quits.get(), 1);
    assert!(out.viewport_commands.remove(&egui::ViewportId::ROOT).unwrap_or_default().contains(&egui::ViewportCommand::CancelClose));
    // A platform without a quit of its own: nothing taken over, eframe closes.
    let out = ctx.run_logic(&close(), |ctx| assert!(!MailApp::quit_instead_of_close(ctx, || false)));
    assert!(out.viewport_commands.get(&egui::ViewportId::ROOT).is_none_or(|c| !c.contains(&egui::ViewportCommand::CancelClose)));
    // No close asked for: no quit.
    let _ = ctx.run_logic(&RawInput::default(), |ctx| assert!(!MailApp::quit_instead_of_close(ctx, || panic!("no close, no quit"))));
    // A message being written: the question holds the window, and the quit waits for the answer.
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Not lost to Cmd+Q"}));
    frame(&ctx, &mut a, vec![]);
    let _ = ctx.run_logic(&RawInput { time: Some(50.0), ..close() }, |ctx| {
        a.logic(ctx);
        assert!(!MailApp::quit_instead_of_close(ctx, || panic!("asked first")));
    });
    assert!(matches!(a.ui.dialog, Some(Dialog::SaveOnClose(1))));
    let _ = std::fs::remove_dir_all(dir);
}

/// A file dragged from the desktop onto a message being written is attached to it.
#[test]
fn files_dropped_on_a_message_are_attached() {
    use std::path::{Path, PathBuf};
    /// What the desktop hands over for one file let go over the window.
    #[derive(Debug)]
    struct LetGo {
        from: PathBuf,
    }
    impl egui::DroppedFile for LetGo {
        fn bytes(&self) -> Result<Vec<u8>, String> {
            let read = std::fs::read(&self.from);
            read.map_err(|why| format!("{}: {why}", self.from.display()))
        }

        fn path(&self) -> &Path {
            self.from.as_path()
        }
    }
    let (mut a, dir) = app("drop-attach");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let file = dir.join("dropped notes.txt");
    std::fs::write(&file, b"hello").unwrap();
    let dropped: Vec<egui::DroppedFileHandle> =
        vec![std::sync::Arc::new(LetGo { from: file }), std::sync::Arc::new(LetGo { from: "relative.txt".into() })];
    let input = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
        dropped_files: dropped,
        ..Default::default()
    };
    // The message is in a window of its own, or in the main one where there are none: either
    // way the drop reaches it.
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    frame(&ctx, &mut a, vec![]);
    let names: Vec<String> = a.session.all_composes().flat_map(|c| c.attachments.iter().map(|x| x.name.clone())).collect();
    assert_eq!(names, ["dropped notes.txt"], "the file with no path to read is left");
    let _ = std::fs::remove_dir_all(dir);
}

/// From offers the account's own address and the mailboxes it opens (a shared one); another
/// address is typed under the menu's list, marked by the field when it isn't one, and taken with
/// Enter.
#[test]
fn from_offers_shared_mailboxes_and_another_address() {
    let (mut a, dir) = app("send-from");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    // The Inbox as a shared mailbox's (Exchange marks such folders).
    for f in a.session.accounts.iter_mut().flat_map(|x| x.folders.iter_mut()).filter(|f| f.path == "INBOX") {
        f.mailbox = Some("support@studio.example".into());
    }
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    let own = a.session.senders().first().unwrap().label.clone();
    // The From button, on the row of the address it's from.
    let button = |ctx: &egui::Context, a: &mut MailApp, shown: &str| {
        let row = settled_text_at(ctx, a, shown, size).expect("who it's from");
        frame_shapes(ctx, a, vec![], size)
            .iter()
            .find_map(|s| match s {
                egui::Shape::Text(t) if t.galley.text() == "From" => {
                    let at = t.pos + t.galley.rect.center().to_vec2();
                    ((at.y - row.y).abs() < 4.0 && at.x < row.x).then_some(at)
                }
                _ => None,
            })
            .expect("the From button")
    };
    let from = button(&ctx, &mut a, &own);
    click_at(&ctx, &mut a, from, 1, size);
    let shared = settled_text_at(&ctx, &mut a, "support@studio.example", size).expect("the shared mailbox offered");
    click_at(&ctx, &mut a, shared, 1, size);
    assert_eq!(a.session.compose.as_ref().unwrap().from, "support@studio.example");
    let from = button(&ctx, &mut a, "support@studio.example");
    click_at(&ctx, &mut a, from, 1, size);
    let other = settled_text_at(&ctx, &mut a, "Other email address…", size).expect("another address offered");
    click_at(&ctx, &mut a, other, 1, size);
    // A plain name is no error: it's checked on Ctrl+K or as the field is left.
    frame_sized(&ctx, &mut a, vec![Event::Text("Dupont".into())], size);
    let field = quatrelle_engine::FieldId { window: a.session.compose.as_ref().map_or(0, |c| c.id), name: "from" };
    assert_eq!(a.ui.people.typing(field), "Dupont", "typed in the field");
    assert!(settled_text_at(&ctx, &mut a, "Not an address: Dupont", size).is_none(), "a name typed isn't said to be no address");
    for _ in 0.."Dupont".len() {
        frame_sized(&ctx, &mut a, vec![key(Key::Backspace, Modifiers::NONE)], size);
    }
    // A piece with `@` that isn't an address is marked by the field itself as it closes.
    frame_sized(&ctx, &mut a, vec![Event::Text("nope@;".into())], size);
    frame_sized(&ctx, &mut a, vec![], size);
    let bad = |p: &crate::look::people_field::Piece| matches!(p, crate::look::people_field::Piece::Text { text, error: Some(crate::look::people_field::PieceError::NotAnAddress) } if text == "nope@");
    assert!(a.ui.people.pieces(field).iter().any(bad), "{:?}", a.ui.people.pieces(field));
    frame_sized(&ctx, &mut a, vec![Event::Text("Bookings <bookings@studio.example>".into())], size);
    frame_sized(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)], size);
    frame_sized(&ctx, &mut a, vec![], size);
    assert_eq!(a.session.compose.as_ref().unwrap().from, "Bookings <bookings@studio.example>");
    assert!(settled_text_at(&ctx, &mut a, "Other email address…", size).is_none(), "the menu closed");
    let _ = std::fs::remove_dir_all(dir);
}

/// Ctrl+K checks the names typed, as Outlook's: one known is written out with its address, one
/// unknown is said; in the text, Ctrl+K is still a link.
#[test]
fn ctrl_k_checks_the_names_typed() {
    let (mut a, dir) = app("ctrl-k");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("Zoe Park; nobody here".into())]);
    frame(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.compose.as_ref().unwrap().to, "Zoe Park <zoe@studio.example>; nobody here; ");
    assert!(a.session.notices.last().is_some_and(|n| n.error && n.text.starts_with("Some names weren't found")), "{:?}", a.session.notices.last());
    // In the text, a link.
    let body = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    ctx.memory_mut(|m| m.request_focus(body));
    frame(&ctx, &mut a, vec![]);
    let notices = a.session.notices.len();
    frame(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.notices.len(), notices, "the link form, not Check Names (\"nobody here\" would be said again)");
    let _ = std::fs::remove_dir_all(dir);
}

/// The sample with an Exchange account beside it (its directory is searched), and its id.
pub(super) fn with_exchange(a: &mut MailApp) -> String {
    let config = quatrelle_engine::model::AccountConfig {
        name: "Riley".into(),
        email: "riley@contoso.example".into(),
        username: "riley".into(),
        ews: Some("https://mail.contoso.example/EWS/Exchange.asmx".into()),
        ..Default::default()
    };
    a.session.add_account(config, Some("pw".into())).unwrap()
}

pub(super) fn directory_answers(a: &mut MailApp, account: &str, query: &str, people: &[(&str, &str)]) {
    let people = people.iter().map(|(n, e)| quatrelle_engine::model::Addressee::from(quatrelle_engine::model::EmailAddress::new(*n, *e))).collect();
    a.session.apply(quatrelle_engine::model::Event::DirectoryPeople {
        account: account.into(),
        query: query.into(),
        people,
        more: false,
        error: None,
    });
}

/// Ctrl+K on a name several people in the directory fit: Check Names asks which one was meant,
/// and the one picked is written out.
#[test]
fn check_names_asks_which_person_a_name_means() {
    let (mut a, dir) = app("check-names-choice");
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("dupont".into())]);
    frame(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.directory_searching(&gal, "dupont"), "asked of the directory");
    directory_answers(&mut a, &gal, "dupont", &[("Anne Dupont", "anne.dupont@contoso.example"), ("Marc Dupont", "marc.dupont@contoso.example")]);
    assert!(settled_text_at(&ctx, &mut a, "More than one person fits “dupont”. Choose who you meant:", size).is_some(), "the choice is asked");
    let marc = settled_text_at(&ctx, &mut a, "Marc Dupont", size).expect("Marc offered");
    click_at(&ctx, &mut a, marc, 2, size);
    frame_sized(&ctx, &mut a, vec![], size);
    assert_eq!(a.session.compose.as_ref().unwrap().to, "Marc Dupont <marc.dupont@contoso.example>; ");
    // (Marc is now a chip in To, drawn by his name: the question is what's gone.)
    assert!(settled_text_at(&ctx, &mut a, "More than one person fits “dupont”. Choose who you meant:", size).is_none(), "the dialog closed");
    let _ = std::fs::remove_dir_all(dir);
}

/// The From button of the message being written, from the own address beside it (the message
/// sends from it, and the menu is closed).
fn from_button(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2) -> egui::Pos2 {
    let own = a.session.senders().first().map(|s| s.label.clone()).expect("the account's own address");
    let row = settled_text_at(ctx, a, &own, size).expect("who it's from");
    frame_shapes(ctx, a, vec![], size)
        .iter()
        .find_map(|s| match s {
            egui::Shape::Text(t) if t.galley.text() == "From" => {
                let at = t.pos + t.galley.rect.center().to_vec2();
                ((at.y - row.y).abs() < 4.0 && at.x < row.x).then_some(at)
            }
            _ => None,
        })
        .expect("the From button")
}

/// The From menu of the message being written opened, and its Other email address (the field
/// has the keyboard); where the From button is.
pub(super) fn open_other_address(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2) -> egui::Pos2 {
    let from = from_button(ctx, a, size);
    click_at(ctx, a, from, 1, size);
    let other = settled_text_at(ctx, a, "Other email address…", size).expect("another address offered");
    click_at(ctx, a, other, 1, size);
    from
}

/// From's other address finds a name when asked, as any contacts field does. This test once
/// checked that the directory was searched as the name was typed; the contacts field's rule
/// (typing never asks the server) took that away: typing asks nothing, Ctrl+K asks the
/// directory, the one person it has becomes the field's chip, and Send from it takes them.
#[test]
fn from_finds_another_address_by_its_name() {
    let (mut a, dir) = app("send-from-name");
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    open_other_address(&ctx, &mut a, size);
    frame_sized(&ctx, &mut a, vec![Event::Text("proginov".into())], size);
    frame_sized(&ctx, &mut a, vec![], size);
    assert!(!a.session.directory_searching(&gal, "proginov"), "typing asks nothing");
    frame_sized(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)], size);
    frame_sized(&ctx, &mut a, vec![], size);
    assert!(a.session.directory_searching(&gal, "proginov"), "Ctrl+K asks the directory");
    directory_answers(&mut a, &gal, "proginov", &[("Proginov Reseau", "reseaux@proginov.example")]);
    frame_sized(&ctx, &mut a, vec![], size);
    frame_sized(&ctx, &mut a, vec![], size);
    let from = quatrelle_engine::FieldId { window: a.session.compose.as_ref().map_or(0, |c| c.id), name: "from" };
    let pieces = a.ui.people.pieces(from).to_vec();
    assert!(
        matches!(pieces.as_slice(), [crate::look::people_field::Piece::Chip { who, refused: None }] if who.address == "reseaux@proginov.example"),
        "one chip: {pieces:?}"
    );
    let send = settled_text_at(&ctx, &mut a, "Send from it", size).expect("Send from it");
    click_at(&ctx, &mut a, send, 1, size);
    assert_eq!(a.session.compose.as_ref().unwrap().from, "Proginov Reseau <reseaux@proginov.example>");
    let _ = std::fs::remove_dir_all(dir);
}

/// A signature written in HTML (a logo beside the name, as Outlook's are made) shows in a new
/// message as it's sent: the name beside the logo, not under it.
#[test]
fn an_html_signature_shows_in_the_message_as_its_sent() {
    use egui::Shape;
    let (mut a, dir) = app("html-signature");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let id = a.session.accounts.first().map(|x| x.config.id.clone()).unwrap();
    let logo = "https://logo.example/logo.png";
    let html = format!(
        r#"<table align="left" border="0" cellpadding="0" cellspacing="0" style="float:left"><tr><td valign="middle" style="vertical-align:middle"><img src="{logo}" width="120px" border="0" style="width:120px;height:48px"></td></tr></table><table border="0" cellpadding="0" cellspacing="0" style="float:left;padding:0px;padding-left:10px;border-left:1px solid #ccc;height:63px"><tr><td style="height:63px;vertical-align:middle"><p>Didier GAUDIN<br>01 02 03 04 05<br>didier@example.org</p></td></tr></table>"#
    );
    a.run("account.setSignature", json!({"account": id, "signature": "", "rawHtml": html}));
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(120, 48, image::Rgba([20, 90, 160, 255])))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    a.session.apply(quatrelle_engine::model::Event::Fetched { url: logo.into(), data: Some(png), error: None });
    // Written in the reading pane (the window drawn here).
    a.session.prefs.new_mail_window = false;
    a.run("mail.new", json!({}));
    let c = a.session.compose.clone().unwrap();
    assert!(c.body.contains(quatrelle_engine::mime::rich::SIGNATURE) && c.signature_html.contains("cid:"), "{:?}", c.body);
    let size = egui::vec2(1280.0, 800.0);
    let mut shapes = Vec::new();
    for _ in 0..6 {
        shapes = frame_shapes(&ctx, &mut a, vec![], size);
    }
    let pictures: Vec<egui::Rect> = shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => Some(m.calc_bounds()),
            Shape::Rect(r) if r.brush.is_some() => Some(r.rect),
            _ => None,
        })
        .collect();
    let logo_at = pictures.iter().find(|r| (r.width() - 120.0).abs() < 1.0 && (r.height() - 48.0).abs() < 1.0).copied();
    let logo_at = logo_at.unwrap_or_else(|| panic!("the logo is drawn at its size: {pictures:?}"));
    let name = shapes.iter().find_map(|s| match s {
        Shape::Text(t) if t.galley.text().contains("Didier GAUDIN") => Some(t.pos),
        _ => None,
    });
    let name = name.unwrap_or_else(|| panic!("the name is drawn"));
    assert!(name.x > logo_at.right(), "beside the logo, not under it: {name:?}, logo {logo_at:?}");
    assert!(name.y < logo_at.bottom(), "at its height: {name:?}, logo {logo_at:?}");
    let _ = std::fs::remove_dir_all(dir);
}
