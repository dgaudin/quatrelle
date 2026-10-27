//! UI tests: the list, the reading pane, attachments and notices.

use super::*;

#[test]
fn renders_every_state_without_panicking() {
    tour();
}

/// A newsletter with a picture behind its heading and a label placed over its poster draws in
/// both themes and in a narrow reading pane (where its columns stack).
#[test]
fn a_banner_and_a_placed_label_draw() {
    let (mut a, dir) = app("banner");
    let ctx = egui::Context::default();
    let k = a.session.list.iter().find(|m| m.subject.contains("winter group show")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let p = a.session.reading.as_ref().and_then(|r| r.parsed.clone()).unwrap();
    assert!(p.images.iter().any(|i| i == "cid:banner@gallery.example"), "the banner is one of its pictures: {:?}", p.images);
    for dark in [Some(false), Some(true)] {
        a.ui.dark = dark;
        for pane in [ReadingPane::Right, ReadingPane::Bottom] {
            a.session.prefs.reading_pane = pane;
            for _ in 0..3 {
                frame(&ctx, &mut a, vec![]);
            }
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// A shop's receipt as HTML mail: a framed, rounded card, a receipt table with `border="1"` and
/// `cellpadding`, a row whose short cell sits in the middle beside a tall one, a rounded
/// button, underlined words.
pub(crate) const RECEIPT_HTML: &str = r##"<table width="560" align="center" cellpadding="0" cellspacing="0" style="border:1px solid #dddddd; border-radius:8px">
<tr><td style="padding:16px 20px; border-bottom:1px solid #dddddd"><h2>Your order is on its way</h2></td></tr>
<tr><td style="padding:16px 20px">
<p>Thanks for your order, <u>Riley</u>. Here is what we <span style="text-decoration:underline">packed</span>:</p>
<table border="1" cellpadding="6" cellspacing="0" width="100%" style="border-collapse:collapse">
<tr bgcolor="#f4f4f4"><th align="left">Item</th><th align="right">Qty</th><th align="right">Price</th></tr>
<tr><td>Screen print, <i>Lantern</i> (A2)</td><td align="right">1</td><td align="right">40.00</td></tr>
<tr><td>Postcards, set of 8</td><td align="right">2</td><td align="right">12.00</td></tr>
<tr><td colspan="2" align="right"><b>Total</b></td><td align="right"><b>64.00</b></td></tr>
</table>
<table width="100%" cellpadding="8" cellspacing="0" style="margin-top:12px">
<tr><td width="45%" bgcolor="#eef3ff" style="border-left:4px solid #1a73e8">Tall line 1<br>Tall line 2<br>Tall line 3<br>Tall line 4</td>
<td bgcolor="#fff7e6">Short</td></tr>
</table>
</td></tr>
<tr><td align="center" style="padding:0 20px 20px">
<table cellpadding="0" cellspacing="0"><tr><td bgcolor="#1a73e8" style="border-radius:4px; padding:12px 24px"><a href="https://example.com/track" style="color:#ffffff; text-decoration:none"><b>Track your parcel</b></a></td></tr></table>
</td></tr>
</table>"##;

/// Borders, round corners, cell padding and row alignment draw: the receipt's frame and grid
/// are painted in their colours, its button is a rounded box, the short cell's text sits in
/// the middle of its row from the second frame (the first measures), underlined words are
/// underlined; in both themes, and in a narrow pane (its columns stacked) nothing breaks.
#[test]
fn bordered_tables_and_aligned_cells_draw() {
    use egui::Shape;
    let (mut a, dir) = app_with_html("borders", "Your order is on its way", RECEIPT_HTML);
    let ctx = egui::Context::default();
    // (The first frame sets the app up; a command run before it would be undone.)
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.contains("Your order")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    a.session.prefs.reading_pane = ReadingPane::Right;
    let p = a.session.reading.as_ref().and_then(|r| r.parsed.clone()).unwrap();
    assert!(!p.layout.is_empty(), "laid out as tables");
    let rgb = |c: [u8; 3]| egui::Color32::from_rgb(c[0], c[1], c[2]);
    // Every box drawn: its fill, its stroke's colour (a rounded frame draws its border so) and
    // how round its corners are.
    let boxes = |shapes: &[Shape]| {
        shapes
            .iter()
            .filter_map(|s| if let Shape::Rect(r) = s { Some((r.fill, r.stroke.color, r.corner_radius.nw)) } else { None })
            .collect::<Vec<_>>()
    };
    // Where a piece of the message's text is drawn (a whole label: the list's preview holds
    // the same words).
    let text_y = |shapes: &[Shape], text: &str| {
        shapes.iter().find_map(|s| match s {
            Shape::Text(t) if t.galley.text() == text => Some(t.pos.y),
            _ => None,
        })
    };
    // The first frame after opening draws the message out of sight (it fades in) and measures
    // its cells; from the next, the short cell's content sits in the middle of its row.
    frame(&ctx, &mut a, vec![]);
    assert!((0..12).any(|n| a.ui.cell_heights.get(&k, n).is_some()), "the cells' heights are measured on the first frame");
    // Faded in, its colours are whole.
    let card = |shapes: &[Shape]| boxes(shapes).iter().any(|(_, stroke, r)| *stroke == rgb([0xdd, 0xdd, 0xdd]) && *r == 8);
    let mut shown = Vec::new();
    for _ in 0..40 {
        shown = frame_shapes(&ctx, &mut a, vec![], egui::vec2(1280.0, 800.0));
        if card(&shown) {
            break;
        }
    }
    let texts: Vec<String> =
        shown.iter().filter_map(|s| if let Shape::Text(t) = s { Some(t.galley.text().chars().take(30).collect()) } else { None }).collect();
    assert!(text_y(&shown, "Tall line 1").is_some(), "the message is drawn: {texts:?}");
    let strokes: Vec<_> = boxes(&shown).iter().filter(|(_, s, _)| s.a() > 0).map(|(_, s, r)| (*s, *r)).collect();
    assert!(card(&shown), "the card's rounded frame in its colour: {strokes:?}");
    let drawn = |c: [u8; 3]| boxes(&shown).iter().any(|(fill, stroke, _)| *fill == rgb(c) || *stroke == rgb(c));
    assert!(drawn([0x99, 0x99, 0x99]), "the receipt's grid (border=\"1\") in grey");
    assert!(drawn([0x1a, 0x73, 0xe8]), "the rule down the tall cell, and the button");
    assert!(
        boxes(&shown).iter().any(|(fill, _, r)| *fill == rgb([0x1a, 0x73, 0xe8]) && *r == 4),
        "the button's corners rounded: {:?}",
        boxes(&shown)
    );
    let underlined = shown.iter().any(
        |s| matches!(s, Shape::Text(t) if t.galley.text() == "Riley" && t.galley.job.sections.iter().any(|sec| sec.format.underline.width > 0.0)),
    );
    assert!(underlined, "the <u> word is underlined");
    let (tall, short) = (text_y(&shown, "Tall line 1").unwrap(), text_y(&shown, "Short").unwrap());
    assert!(short > tall + 15.0, "the short cell's text sits in the middle of its row: {short} vs {tall}");
    let again = frame_shapes(&ctx, &mut a, vec![], egui::vec2(1280.0, 800.0));
    assert_eq!(text_y(&again, "Short"), Some(short), "steady from then on");
    for dark in [Some(true), Some(false)] {
        a.ui.dark = dark;
        for pane in [ReadingPane::Bottom, ReadingPane::Right] {
            a.session.prefs.reading_pane = pane;
            for _ in 0..2 {
                frame(&ctx, &mut a, vec![]);
            }
        }
        // Narrow (the pane under the list, the window 480 wide and tall enough to show the
        // message, since what's scrolled out of sight isn't painted): the receipt's three
        // columns stack, each cell's content at its own top, and the card still draws.
        a.session.prefs.reading_pane = ReadingPane::Bottom;
        let mut narrow = Vec::new();
        for _ in 0..2 {
            narrow = frame_shapes(&ctx, &mut a, vec![], egui::vec2(480.0, 1800.0));
        }
        let texts: Vec<String> =
            narrow.iter().filter_map(|s| if let Shape::Text(t) = s { Some(t.galley.text().chars().take(30).collect()) } else { None }).collect();
        assert!(card(&narrow) && text_y(&narrow, "Item").is_some(), "the card ({}) and the receipt's first cell, narrow: {texts:?}", card(&narrow));
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Dialogs kept the size of the last big one (their buttons' row took all the room left): Add
/// account stayed tall after Server settings closed, Remove account opened as tall as Options.
#[test]
fn dialogs_fit_what_they_show() {
    let (mut a, dir) = app("dialog-size");
    let ctx = egui::Context::default();
    let height = |ctx: &egui::Context, d: &Dialog| {
        // Its own window (or, before, the one all dialogs shared).
        let ids = [egui::Id::unique(("dialog", std::mem::discriminant(d))), egui::Id::unique("dialog")];
        ids.iter().find_map(|id| ctx.memory(|m| m.area_rect(*id))).map_or(0.0, |r| r.height())
    };
    let shown = |a: &mut MailApp, d: Dialog| {
        a.ui.dialog = Some(d);
        for _ in 0..4 {
            frame(&ctx, a, vec![]);
        }
        a.ui.dialog.clone().unwrap()
    };
    let mut d = Dialog::add_account_with("zoe@gmail.com", true);
    let open = height(&ctx, &shown(&mut a, d.clone()));
    if let Dialog::AddAccount(f) = &mut d {
        f.advanced = false;
    }
    let closed = height(&ctx, &shown(&mut a, d));
    assert!(open > 0.0 && closed < open - 60.0, "Server settings closed: {closed} (open: {open})");
    let edit = Dialog::edit_account(&a.session.accounts[0].config);
    shown(&mut a, edit);
    let remove = Dialog::RemoveAccount { account: "sample".into(), email: "x".into() };
    let small = height(&ctx, &shown(&mut a, remove));
    assert!(small > 0.0 && small < 220.0, "Remove account: {small}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn keyboard_shortcuts_run_commands() {
    let (mut a, dir) = app("keys");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    let first = a.session.focus.clone().unwrap();
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    assert_ne!(a.session.focus.clone().unwrap(), first);
    let was = a.session.selected()[0].flags.flagged;
    frame(&ctx, &mut a, vec![key(Key::Insert, Modifiers::NONE)]);
    assert_eq!(a.session.selected()[0].flags.flagged, !was, "Insert toggles the flag");
    let before = a.session.list.len();
    frame(&ctx, &mut a, vec![key(Key::Delete, Modifiers::NONE)]);
    assert_eq!(a.session.list.len(), before - 1);
    frame(&ctx, &mut a, vec![key(Key::N, Modifiers::COMMAND)]);
    assert!(a.session.compose.is_some());
    // The new message's To box takes focus, so Delete edits text instead of deleting mail.
    frame(&ctx, &mut a, vec![]);
    let n = a.session.list.len();
    frame(&ctx, &mut a, vec![key(Key::Delete, Modifiers::NONE)]);
    assert_eq!(a.session.list.len(), n);
    frame(&ctx, &mut a, vec![key(Key::M, Modifiers::COMMAND | Modifiers::SHIFT)]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn folder_dialogs_create_rename_and_report_errors() {
    let (mut a, dir) = app("folders");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Some(Dialog::new_folder("sample", None));
    frame(&ctx, &mut a, vec![]);
    if let Some(Dialog::FolderName { name, .. }) = a.ui.dialog.as_mut() {
        *name = "Clients".into();
    }
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    assert!(a.session.folder("sample", "Clients").is_some());
    assert!(a.ui.dialog.is_none(), "the dialog closes once the folder exists");
    // A duplicate keeps the dialog open with the error.
    a.act(crate::Action::Dialog(Dialog::new_folder("sample", None)));
    a.act(crate::Action::Submit("folder.create", json!({"account": "sample", "name": "Clients"})));
    assert!(matches!(&a.ui.dialog, Some(Dialog::FolderName { error: Some(e), .. }) if e.contains("already")));
    frame(&ctx, &mut a, vec![]);
    // Favorites show the new folder once pinned.
    a.run("folder.favorite", json!({"account": "sample", "folder": "Clients"}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.favorites().iter().any(|f| f.folder == "Clients"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn attachments_drag_out_as_files() {
    let (mut a, dir) = app("drag-out");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.has_attachments).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    // Not offered where the system can't take the file.
    a.act(crate::Action::DragOut(0));
    assert!(a.take_drag_out().is_none());
    a.services = Box::new(Recorder(Default::default()));
    a.ui.can_drag_out = true;
    let index = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).and_then(|p| p.attachments.first()).map(|x| x.index).unwrap();
    a.act(crate::Action::DragOut(index));
    let path = a.take_drag_out().expect("a file to drag");
    assert!(path.is_file() && path.metadata().unwrap().len() > 0, "{path:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn new_mail_is_announced_and_counted_in_the_title() {
    let (mut a, dir) = app("announce");
    let shown = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(Recorder(shown.clone()));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.title_shown, "(3) Inbox – Quatrelle");
    assert!(shown.lock().unwrap().is_empty(), "the mail already there isn't news");
    let m = |uid: u32, who: &str, subject: &str| Summary {
        key: MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: 1, uid },
        from: quatrelle_engine::model::EmailAddress::new(who, "x@y.example"),
        subject: subject.into(),
        date: NOW - 5,
        ..Default::default()
    };
    a.session.new_mail = vec![m(90, "Zoe Park", "Lunch?")];
    frame(&ctx, &mut a, vec![]);
    a.session.new_mail = vec![m(91, "Ann", "a"), m(92, "Bo", "b"), m(93, "Ann", "c")];
    frame(&ctx, &mut a, vec![]);
    let got = shown.lock().unwrap().clone();
    assert_eq!(got[0], ("Zoe Park".to_string(), "Lunch?".to_string()));
    assert_eq!(got[1], ("3 new messages".to_string(), "From Ann and Bo".to_string()));
    let _ = std::fs::remove_dir_all(dir);
}

/// Minimized or hidden, the window draws nothing (eframe calls only `logic`): new mail is told on
/// the desktop all the same, not when the window comes back.
#[test]
fn new_mail_is_announced_while_the_window_is_hidden() {
    let (mut a, dir) = app("announce-hidden");
    let shown = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(Recorder(shown.clone()));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    a.session.new_mail = vec![Summary {
        key: MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: 1, uid: 90 },
        from: quatrelle_engine::model::EmailAddress::new("Zoe Park", "zoe@studio.example"),
        subject: "Lunch?".into(),
        date: NOW - 5,
        ..Default::default()
    }];
    // No `ui`: what eframe does for a minimized window.
    a.logic(&ctx);
    assert_eq!(shown.lock().unwrap().clone(), [("Zoe Park".to_string(), "Lunch?".to_string())]);
    // The next frame drawn (logic first, as eframe does) doesn't tell it again.
    a.logic(&ctx);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(shown.lock().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn long_folders_render_quickly() {
    let (mut a, dir) = app("long");
    let store = a.session.store.clone().unwrap();
    let messages: Vec<Summary> = (1..=2000u32)
        .rev()
        .map(|uid| Summary {
            key: MessageKey { account: "sample".into(), folder: "Archive".into(), validity: 1, uid },
            subject: format!("Message {uid}"),
            date: NOW - i64::from(2000 - uid) * 3600,
            preview: "Lorem".into(),
            ..Default::default()
        })
        .collect();
    store.save_folder("sample", "Archive", &FolderState { validity: 1, uid_next: 2001, highest_modseq: 0, messages, window: 0 }).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "Archive".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "Archive"}));
    assert_eq!(a.session.list.len(), 2000);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let t = std::time::Instant::now();
    for _ in 0..10 {
        frame(&ctx, &mut a, vec![]);
    }
    let per = t.elapsed() / 10;
    // Virtualised: only visible rows are laid out (debug builds are slow; this is generous).
    assert!(per.as_millis() < 400, "frame took {per:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn every_kind_of_attachment_previews_in_the_reading_pane() {
    let (mut a, dir) = app("attachment-preview");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.starts_with("Your proof")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    let n = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map_or(0, |p| p.attachments.len());
    assert_eq!(n, 4, "notes, a PDF, a document and a workbook");
    for i in 0..n {
        a.ui.attachment_view = Some((k.clone(), i));
        // A PDF draws a page a frame.
        for _ in 0..3 {
            frame(&ctx, &mut a, vec![]);
        }
        assert_eq!(a.ui.attachment_view, Some((k.clone(), i)));
    }
    // Escape: back to the message.
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.attachment_view.is_none());
    // One that isn't there any more (another message's index) is just the message.
    a.ui.attachment_view = Some((k.clone(), 99));
    frame(&ctx, &mut a, vec![]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Ctrl+A goes to what was last clicked: the attachments when they were, else the mail list.
#[test]
fn ctrl_a_selects_attachments_when_they_have_the_keyboard() {
    let (mut a, dir) = app("attach-all");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.contains("festival poster")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    a.ui.attachments_active = true;
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    let picked = a.ui.attachment_selected.as_ref().map(|(_, v)| v.len());
    assert_eq!(picked, Some(4), "all four attachments");
    assert_eq!(a.session.selection.len(), 1, "the mail selection is left alone");
    // Escape lets go of them.
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.attachment_selected.is_none() && !a.ui.attachments_active);
    a.ui.attachments_active = true;
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    assert_eq!(a.ui.attachment_selected.as_ref().map(|(_, v)| v.len()), Some(4));
    // Elsewhere (a press off the attachments), Ctrl+A selects the mail again.
    let off = egui::pos2(5.0, 400.0);
    frame(&ctx, &mut a, vec![Event::PointerMoved(off)]);
    frame(&ctx, &mut a, vec![Event::PointerButton { pos: off, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
    frame(&ctx, &mut a, vec![Event::PointerButton { pos: off, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    assert!(!a.ui.attachments_active);
    assert!(a.ui.attachment_selected.is_none(), "a click elsewhere lets go of them");
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    assert_eq!(a.session.selection.len(), a.session.list.len());
    let _ = std::fs::remove_dir_all(dir);
}

/// Back from a previewed attachment, its chip isn't left selected.
#[test]
fn leaving_an_attachment_preview_lets_go_of_its_chip() {
    let (mut a, dir) = app("attach-back");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.contains("festival poster")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    let index = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.attachments[0].index).unwrap();
    a.ui.attachment_view = Some((k.clone(), index));
    a.ui.attachment_selected = Some((k.clone(), vec![index]));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.attachment_view.is_none() && a.ui.attachment_selected.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// Enter in a dialog's one field submits it: the dialog gave the field its focus back before
/// looking for the Enter that had just ended input, so only the button worked.
#[test]
fn enter_submits_a_one_field_dialog() {
    let (mut a, dir) = app("enter-submits");
    let ctx = egui::Context::default();
    // The sample account isn't an Exchange one, so the submit comes back as the dialog's error.
    a.ui.dialog = Some(Dialog::OpenSharedCalendar { email: "ana@contoso.example".into(), error: None });
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(&a.ui.dialog, Some(Dialog::OpenSharedCalendar { error: Some(_), .. })), "submitted: {:?}", a.ui.dialog.is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// Motion: the folder pane slides between its two widths and the rest draws through it; Play
/// animations off makes egui's own motion instant too; a message just opened fades in.
#[test]
fn motion_plays_and_stops() {
    let (mut a, dir) = app("motion");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    assert!(crate::motion::enabled(&ctx));
    for minimized in [true, false, true, false] {
        a.run("view.folderPane", json!({"minimized": minimized}));
        for _ in 0..4 {
            frame(&ctx, &mut a, vec![]);
        }
    }
    a.run("list.open", json!({"key": "sample:1:1:INBOX"}));
    frame(&ctx, &mut a, vec![]);
    a.run("view.animations", json!({"on": false}));
    frame(&ctx, &mut a, vec![]);
    assert!(!crate::motion::enabled(&ctx));
    assert_eq!(ctx.global_style().animation_time, 0.0);
    a.run("view.folderPane", json!({"minimized": true}));
    frame(&ctx, &mut a, vec![]);
    a.run("view.animations", json!({}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.prefs.animations && crate::motion::enabled(&ctx));
    let _ = std::fs::remove_dir_all(dir);
}

/// A plain-text message's addresses written out are links: a click on one opens it (a mail
/// address: a new message to it), as Outlook makes them links.
#[test]
fn addresses_in_a_plain_text_message_open_with_a_click() {
    let (mut a, dir) = app("plain-links");
    let raw = b"From: Zoe Park <zoe@studio.example>\r\nTo: riley@studio.example\r\nSubject: Links\r\nMessage-ID: <plain-links@x>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe notes are on http://notes.example.fr/today.\r\nQuestions: support@example.com.\r\n";
    let store = a.session.store.clone().unwrap();
    let mut st = store.load_folder("sample", "INBOX").unwrap();
    let uid = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1;
    let key = quatrelle_engine::model::MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: st.validity.max(1), uid };
    st.messages.insert(0, quatrelle_engine::mime::summary_from_full(raw, key.clone(), Default::default(), raw.len() as u32, NOW));
    store.write_body(&key, raw).unwrap();
    store.save_folder("sample", "INBOX", &st).unwrap();
    a.session.apply(quatrelle_engine::model::Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    a.run("view.html", json!({"on": false}));
    a.run("list.open", json!({"key": key.to_id()}));
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let at: Vec<egui::Rect> = ctx.data(|d| d.get_temp(egui::Id::unique("plain-links"))).unwrap_or_default();
    assert_eq!(at.len(), 2, "the web address and the mail address");
    let p = at[1].left_center() + egui::vec2(4.0, 0.0);
    let button = |pressed| Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![Event::PointerMoved(p)]);
    frame(&ctx, &mut a, vec![button(true)]);
    frame(&ctx, &mut a, vec![button(false)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.compose.as_ref().map(|c| c.to.as_str()), Some("support@example.com"), "a new message to it");
    let _ = std::fs::remove_dir_all(dir);
}

/// A network share written in a message (`\\server\share\folder`) is a link too; clicked, it
/// asks first, naming the server (a share on a stranger's server can ask for the user's
/// sign-in), and once trusted, that server's shares open without asking.
#[test]
fn a_network_share_asks_before_it_opens() {
    let (mut a, dir) = app("share-links");
    let raw = b"From: Zoe Park <zoe@studio.example>\r\nTo: riley@studio.example\r\nSubject: Share\r\nMessage-ID: <share@x>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe files: \\\\files-01\\Common\\Projects\\2026 (and the rest).\r\n";
    let store = a.session.store.clone().unwrap();
    let mut st = store.load_folder("sample", "INBOX").unwrap();
    let uid = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1;
    let key = quatrelle_engine::model::MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: st.validity.max(1), uid };
    st.messages.insert(0, quatrelle_engine::mime::summary_from_full(raw, key.clone(), Default::default(), raw.len() as u32, NOW));
    store.write_body(&key, raw).unwrap();
    store.save_folder("sample", "INBOX", &st).unwrap();
    a.session.apply(quatrelle_engine::model::Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    a.run("view.html", json!({"on": false}));
    a.run("list.open", json!({"key": key.to_id()}));
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let click = |a: &mut MailApp| {
        let at: Vec<egui::Rect> = ctx.data(|d| d.get_temp(egui::Id::unique("plain-links"))).unwrap_or_default();
        let p = at.first().expect("the share is a link").left_center() + egui::vec2(6.0, 0.0);
        let button = |pressed| Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        frame(&ctx, a, vec![Event::PointerMoved(p)]);
        frame(&ctx, a, vec![button(true)]);
        frame(&ctx, a, vec![button(false)]);
        frame(&ctx, a, vec![]);
    };
    click(&mut a);
    match a.ui.dialog.as_mut() {
        Some(Dialog::OpenShare { link, server, path, always }) => {
            assert_eq!(
                (link.as_str(), server.as_str(), path.as_str()),
                ("file://files-01/Common/Projects/2026", "files-01", r"\\files-01\Common\Projects\2026")
            );
            *always = true;
        }
        other => panic!("asked nothing: {other:?}"),
    }
    frame(&ctx, &mut a, vec![]);
    // "Open", with "Don't ask again" ticked: the server is trusted from now on.
    a.act(crate::Action::Run("link.trustShareServer", json!({"server": "files-01"})));
    a.act(crate::Action::OpenShare("file://files-01/Common/Projects/2026".into()));
    a.ui.dialog = None;
    assert!(a.session.prefs.trusted_share_servers.contains("files-01"));
    click(&mut a);
    assert!(a.ui.dialog.is_none(), "a trusted server's share opens without asking");
    assert!(a.session.run("link.trustShareServer", &json!({"server": "bad name/x"})).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

/// A message downloaded ahead is listed without a preview: drawn, its row asks for one, and the
/// frames after show the words from its source under the subject.
#[test]
fn a_row_listed_without_a_preview_shows_one_from_its_source() {
    let (mut a, dir) = app_with_html("row-preview", "Quarterly figures", "<p>The new figures are in the shared folder.</p>");
    let account = quatrelle_engine::sample::SAMPLE_ACCOUNT;
    let store = a.session.store.clone().unwrap();
    let mut fs = store.load_folder(account, "INBOX").unwrap();
    let m = fs.messages.iter_mut().find(|m| m.subject == "Quarterly figures").unwrap();
    m.preview.clear();
    store.save_folder(account, "INBOX", &fs).unwrap();
    a.session = Session::open(Some(store), Clock::new(NOW, 0));
    let ctx = egui::Context::default();
    let shows = |shapes: &[egui::Shape]| {
        shapes.iter().any(|s| matches!(s, egui::Shape::Text(t) if t.galley.text() == "The new figures are in the shared folder."))
    };
    assert!(!shows(&frame_shapes(&ctx, &mut a, vec![], egui::vec2(1280.0, 800.0))), "listed without one");
    let mut shown = false;
    for _ in 0..6 {
        if shows(&frame_shapes(&ctx, &mut a, vec![], egui::vec2(1280.0, 800.0))) {
            shown = true;
            break;
        }
    }
    assert!(shown, "the row's preview comes from its source");
    let _ = std::fs::remove_dir_all(dir);
}

/// The More menu's Message headers: a dialog of the header lines, which can be copied whole.
#[test]
fn the_headers_open_in_a_dialog_with_copy() {
    let (mut a, dir) = app("headers");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let key = a.session.list.iter().find(|m| m.subject == "Storyboard review on Thursday").map(|m| m.key.clone()).unwrap();
    a.session.open_message(&key);
    a.act(crate::Action::Run("message.headers", json!({})));
    let Some(Dialog::Preview { title, text, .. }) = &a.ui.dialog else { panic!("no dialog: {:?}", a.ui.dialog.is_some()) };
    assert_eq!(title, "Message headers");
    assert!(text.lines().any(|l| l == "Subject: Storyboard review on Thursday"), "{text}");
    // Drawn once it has faded in.
    let buttons = |drawn: &[egui::Shape]| {
        let labels: Vec<&str> = drawn.iter().filter_map(|s| if let egui::Shape::Text(t) = s { Some(t.galley.text()) } else { None }).collect();
        labels.contains(&"Copy") && labels.contains(&"Close")
    };
    assert!((0..10).any(|_| buttons(&frame_shapes(&ctx, &mut a, vec![], egui::vec2(1280.0, 800.0)))), "Copy beside Close");
    assert!(a.ui.dialog.is_some(), "still open");
    let _ = std::fs::remove_dir_all(dir);
}

/// A report's table of figures (twelve columns, a dark heading row) stays a table, as Outlook
/// shows it: its cells side by side, its columns sized to their content, scrolling sideways in a
/// pane too narrow for its words. Each cell went on a line of its own (twelve columns can't each
/// be 140 wide).
#[test]
fn a_table_of_figures_stays_a_table() {
    use egui::Shape;
    let heads = [
        "Serveur",
        "Statut",
        "vCPU",
        "RAM (Go)",
        "CPU moyen (%)",
        "CPU p95 (%)",
        "CPU max (%)",
        "Heures CPU > 90%",
        "RAM moyenne (%)",
        "RAM p95 (%)",
        "Heures RAM > 90%",
        "File CPU p95 / cœur",
    ];
    let row = ["D4BATCHVN001", "Saturé", "10", "36", "34,6", "99", "99,7", "50,9", "14,2", "19,2", "0", "2,7"];
    let cells = |tag: &str, texts: &[&str]| {
        texts.iter().map(|t| format!("<{tag} style=\"padding:2px 6px\">{}</{tag}>", t.replace('>', "&gt;"))).collect::<String>()
    };
    let html = format!(
        "<p>Pour les Batch voici une première analyse :</p><table style=\"border-collapse:collapse\"><tr style=\"background:#1f5f8b;color:#ffffff\">{}</tr>{}</table>",
        cells("th", &heads),
        (0..20).map(|_| format!("<tr>{}</tr>", cells("td", &row))).collect::<String>()
    );
    let (mut a, dir) = app_with_html("figures", "CR Astreinte : Semaine 43", &html);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.starts_with("CR Astreinte")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let text_y = |shapes: &[Shape], text: &str| {
        shapes.iter().find_map(|s| match s {
            Shape::Text(t) if t.galley.text() == text => Some(t.pos.y),
            _ => None,
        })
    };
    // Wide, the whole table in the pane (some headings over two lines); narrow, its words can't
    // all fit: it scrolls sideways, its first columns in sight.
    for (size, last, fits) in [(egui::vec2(2400.0, 900.0), "File CPU p95 / cœur", true), (egui::vec2(900.0, 900.0), "Statut", false)] {
        let mut shown = Vec::new();
        for _ in 0..30 {
            shown = frame_shapes(&ctx, &mut a, vec![], size);
        }
        let texts: Vec<String> =
            shown.iter().filter_map(|s| if let Shape::Text(t) = s { Some(t.galley.text().chars().take(20).collect()) } else { None }).collect();
        // On one row (a heading over two lines sits in the middle of it); stacked, they were a
        // row apart each.
        let (serveur, other) = (text_y(&shown, "Serveur"), text_y(&shown, last));
        let side_by_side = serveur.zip(other).is_some_and(|(a, b)| (a - b).abs() < 30.0);
        assert!(side_by_side, "the headings side by side at {size:?}: {serveur:?}, {other:?} in {texts:?}");
        assert_eq!(text_y(&shown, "D4BATCHVN001"), text_y(&shown, "Saturé"), "a row's cells side by side at {size:?}");
        assert_eq!(text_y(&shown, "File CPU p95 / cœur").is_some(), fits, "the last column in sight at {size:?}: {texts:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// As Outlook's, the header stays put while the message scrolls under it: the subject, who it's
/// from and to, and Reply… (they went up and away with the text).
#[test]
fn the_header_stays_while_the_message_scrolls() {
    use egui::{Event, Shape};
    let html: String = (1..=80).map(|n| format!("<p>Paragraph {n} of a long message.</p>")).collect();
    let (mut a, dir) = app_with_html("header-stays", "A long message", &html);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject == "A long message").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let size = egui::vec2(1280.0, 800.0);
    let text_y = |shapes: &[Shape], text: &str| {
        shapes.iter().rev().find_map(|s| match s {
            Shape::Text(t) if t.galley.text() == text => Some(t.pos.y),
            _ => None,
        })
    };
    let mut before = Vec::new();
    for _ in 0..30 {
        before = frame_shapes(&ctx, &mut a, vec![], size);
    }
    let (subject, first) = (text_y(&before, "A long message").unwrap(), text_y(&before, "Paragraph 1 of a long message.").unwrap());
    // The wheel over the message's text.
    let over = egui::pos2(1000.0, 600.0);
    let wheel = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -400.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Trackpad,
        modifiers: egui::Modifiers::NONE,
    };
    frame_shapes(&ctx, &mut a, vec![Event::PointerMoved(over)], size);
    let mut after = frame_shapes(&ctx, &mut a, vec![Event::PointerMoved(over), wheel], size);
    for _ in 0..30 {
        after = frame_shapes(&ctx, &mut a, vec![Event::PointerMoved(over)], size);
    }
    assert_eq!(text_y(&after, "A long message"), Some(subject), "the subject stays put");
    assert!(text_y(&after, "Reply").is_some(), "Reply too");
    let moved = text_y(&after, "Paragraph 1 of a long message.").is_none_or(|y| y < first - 100.0);
    assert!(moved, "the text scrolled up: {first} → {:?}", text_y(&after, "Paragraph 1 of a long message."));
    let _ = std::fs::remove_dir_all(dir);
}

/// Ctrl+wheel over the message zooms it, as Outlook's: a notch is ten percent, the text grows
/// and stays where it was (no scrolling), and the status bar says how far; over the list it
/// doesn't.
#[test]
fn ctrl_wheel_zooms_the_message_read() {
    use egui::{Event, Shape};
    let (mut a, dir) = app_with_html("reading-zoom", "A zoomed message", "<p>Some words to read.</p>");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject == "A zoomed message").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let size = egui::vec2(1280.0, 800.0);
    let text = |shapes: &[Shape], text: &str| {
        shapes.iter().rev().find_map(|s| match s {
            Shape::Text(t) if t.galley.text() == text => Some((t.pos.y, t.galley.job.sections.first().map_or(0.0, |s| s.format.font_id.size))),
            _ => None,
        })
    };
    let wheel = |at: egui::Pos2, lines: f32| {
        let notch = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, lines),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: egui::Modifiers::CTRL,
        };
        vec![Event::PointerMoved(at), notch]
    };
    let mut before = Vec::new();
    for _ in 0..10 {
        before = frame_shapes(&ctx, &mut a, vec![], size);
    }
    let (y, written) = text(&before, "Some words to read.").unwrap();
    // Over the list: nothing.
    frame_shapes(&ctx, &mut a, wheel(egui::pos2(300.0, 400.0), 1.0), size);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.prefs.reading_zoom, 100, "the list isn't the message");
    // Over the message: one notch, ten percent.
    let over = egui::pos2(1000.0, 500.0);
    frame_shapes(&ctx, &mut a, vec![Event::PointerMoved(over)], size);
    frame_shapes(&ctx, &mut a, wheel(over, 1.0), size);
    let mut after = Vec::new();
    for _ in 0..10 {
        after = frame_shapes(&ctx, &mut a, vec![Event::PointerMoved(over)], size);
    }
    assert_eq!(a.session.prefs.reading_zoom, 110);
    let (y2, zoomed) = text(&after, "Some words to read.").unwrap();
    assert!((zoomed - written * 1.1).abs() < 0.01, "the text is 10 % bigger: {written} → {zoomed}");
    assert!((y2 - y).abs() < 1.0, "and wasn't scrolled: {y} → {y2}");
    assert!(after.iter().any(|s| matches!(s, Shape::Text(t) if t.galley.text() == "110%")), "the status bar says how far");
    // A notch the other way once the first has died away (egui spreads one over frames): back.
    for _ in 0..20 {
        frame_shapes(&ctx, &mut a, vec![Event::PointerMoved(over)], size);
    }
    frame_shapes(&ctx, &mut a, wheel(over, -1.0), size);
    for _ in 0..30 {
        frame_shapes(&ctx, &mut a, vec![Event::PointerMoved(over)], size);
    }
    assert_eq!(a.session.prefs.reading_zoom, 100, "one notch, one step");
    let _ = std::fs::remove_dir_all(dir);
}

/// The language model's answer above the message closes with the cross at its top right,
/// unused.
#[test]
fn the_assistants_answer_closes_with_its_cross() {
    let (mut a, dir) = app("assistant-close");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("assistant.settings", json!({"url": "http://localhost:11434/v1", "model": "llama3.1"}));
    a.run("list.open", json!({"key": "sample:1:1:INBOX"}));
    a.run("assistant.suggestReply", json!({}));
    let id = a.session.assistant.as_ref().map(|x| x.id).unwrap();
    a.session.apply(quatrelle_engine::model::Event::LlmAnswered { id, text: "Bonjour, c'est noté.".into(), error: None });
    frame(&ctx, &mut a, vec![]);
    let shapes = frame_shapes(&ctx, &mut a, vec![], egui::vec2(1280.0, 800.0));
    let title = shapes
        .iter()
        .find_map(|s| match s {
            egui::Shape::Text(t) if t.galley.text() == "Suggested reply" => Some(t.pos),
            _ => None,
        })
        .unwrap();
    // Its background: the least painted box around its title (still fading in, so not by colour).
    let panel = shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Rect(r) if r.fill.a() > 0 && r.rect.contains(title) => Some(r.rect),
            _ => None,
        })
        .min_by(|x, y| x.area().total_cmp(&y.area()))
        .unwrap();
    let cross = panel.right_top() + egui::vec2(-21.0, 21.0);
    let button = |pressed| Event::PointerButton { pos: cross, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![Event::PointerMoved(cross)]);
    frame(&ctx, &mut a, vec![button(true)]);
    frame(&ctx, &mut a, vec![button(false)]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.assistant.is_none(), "closed");
    assert!(a.session.compose.is_none(), "no reply started");
    let _ = std::fs::remove_dir_all(dir);
}
