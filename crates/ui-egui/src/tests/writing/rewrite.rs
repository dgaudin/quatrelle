// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Rewrite where it's seen: beside Discard on a reply written in the reading pane, grey until
//! something is typed above the signature and the message quoted.

use super::*;

type Handed = std::rc::Rc<std::cell::RefCell<Vec<quatrelle_engine::model::Job>>>;

struct Recorder(Handed);

impl crate::Backend for Recorder {
    fn submit(&mut self, job: quatrelle_engine::model::Job) {
        self.0.borrow_mut().push(job);
    }

    fn poll(&mut self) -> Vec<MailEvent> {
        Vec::new()
    }
}

/// The ribbon's Rewrite, below the reading pane's bar when both are drawn.
fn ribbon_rewrite(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2) -> egui::Pos2 {
    for _ in 0..20 {
        let shapes = frame_shapes(ctx, a, vec![], size);
        let mut at: Vec<egui::Pos2> = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) if t.galley.text() == "Rewrite" => Some(t.pos + t.galley.rect.center().to_vec2()),
                _ => None,
            })
            .collect();
        at.sort_by(|a, b| b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal));
        if let Some(p) = at.first() {
            return *p;
        }
    }
    panic!("no Rewrite button");
}

/// Rewrite was under the Review tab only, and on before anything was typed.
#[test]
fn rewrite_is_beside_discard_and_grey_until_something_is_typed() {
    let (mut a, dir) = app("rewrite-bar");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    a.run("assistant.settings", json!({"url": "http://localhost:11434/v1", "model": "llama3"}));
    let k = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    a.run("mail.reply", json!({}));
    let rewrite = settled_text_at(&ctx, &mut a, "Rewrite", size).expect("Rewrite on the reply's bar");
    let discard = settled_text_at(&ctx, &mut a, "Discard", size).expect("Discard");
    assert!((rewrite.y - discard.y).abs() < 4.0 && rewrite.x > discard.x, "beside Discard");
    // Nothing typed yet (the reply holds the signature and the message quoted): no choices.
    click_at(&ctx, &mut a, rewrite, 1, size);
    assert!(settled_text_at(&ctx, &mut a, "Shorter", size).is_none(), "grey until something is typed");
    // Something typed: its choices.
    if let Some(c) = a.session.compose.as_mut() {
        c.body = format!("Thanks, noted\n\n{}", c.body);
        c.styles = quatrelle_engine::mime::rich::Styles::plain(&c.body);
    }
    click_at(&ctx, &mut a, rewrite, 1, size);
    assert!(settled_text_at(&ctx, &mut a, "Shorter", size).is_some(), "its choices");
    let _ = std::fs::remove_dir_all(dir);
}

/// Asking a rewrite takes the keyboard from the menu; the text must get it back, or Ctrl+Z,
/// Ctrl+Y, typing and the right-click menu stop working until it's clicked again.
#[test]
fn asking_a_rewrite_leaves_the_editor_alive() {
    let (mut a, dir) = app("rewrite-alive");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    a.run("assistant.settings", json!({"url": "http://localhost:11434/v1", "model": "llama3"}));
    a.run("list.open", json!({"key": "sample:1:1:INBOX"}));
    a.run("mail.reply", json!({}));
    if let Some(c) = a.session.compose.as_mut() {
        c.body = format!("Thanks, noted\n\n{}", c.body);
        c.styles = quatrelle_engine::mime::rich::Styles::plain(&c.body);
    }
    let id = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![]);
    let rewrite = settled_text_at(&ctx, &mut a, "Rewrite", size).expect("Rewrite");
    click_at(&ctx, &mut a, rewrite, 1, size);
    let shorter = settled_text_at(&ctx, &mut a, "Shorter", size).expect("Shorter");
    click_at(&ctx, &mut a, shorter, 1, size);
    frame(&ctx, &mut a, vec![]);
    assert!(ctx.memory(|m| m.has_focus(id)), "the text has the keyboard after asking Rewrite");
    // Typing goes in, rather than nowhere, and Ctrl+Z/Ctrl+Y still work.
    let before = a.session.compose.as_ref().unwrap().body.clone();
    frame(&ctx, &mut a, vec![Event::Text("!".into())]);
    let typed = a.session.compose.as_ref().unwrap().body.clone();
    assert_ne!(typed, before, "typing still reaches the text");
    frame(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    assert_eq!(a.session.compose.as_ref().unwrap().body, before, "Ctrl+Z still works");
    frame(&ctx, &mut a, vec![key(Key::Y, Modifiers::COMMAND)]);
    assert_eq!(a.session.compose.as_ref().unwrap().body, typed, "Ctrl+Y still works");
    // The right-click menu opens on the text too.
    let at = a.ui.editor.as_ref().and_then(|ed| ed.drawn.clone()).map_or(egui::pos2(300.0, 300.0), |d| d.rect.center());
    frame(&ctx, &mut a, vec![Event::PointerMoved(at)]);
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![button(true)]);
    frame(&ctx, &mut a, vec![button(false)]);
    assert!(settled_text_at(&ctx, &mut a, "Select all", size).is_some(), "the right-click menu still opens");
    let _ = std::fs::remove_dir_all(dir);
}

/// Using the rewritten text is one undo step in the editor: Ctrl+Z gives back the text, its
/// styles and its picture; Ctrl+Y puts the rewrite back.
#[test]
fn using_a_rewrite_is_one_undo_step() {
    let (mut a, dir) = app("rewrite-undo");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 1200.0);
    a.run("assistant.settings", json!({"url": "http://localhost:11434/v1", "model": "llama3"}));
    a.run("mail.new", json!({}));
    let (text, styles) = quatrelle_engine::mime::rich::from_markup("**Bonjour**\n\n![logo](cid:logo@studio.example)");
    if let Some(c) = a.session.compose.as_mut() {
        c.signature.clear();
        c.body = text;
        c.styles = styles;
        c.inline = vec![quatrelle_engine::mime::InlinePicture {
            cid: "logo@studio.example".into(),
            name: "logo.png".into(),
            mime: "image/png".into(),
            data: Vec::new(),
        }];
    }
    let id = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![]);
    let original = a.session.compose.as_ref().unwrap().clone();
    a.run("assistant.rewrite", json!({"how": "shorter"}));
    let ask = a.session.take_jobs().into_iter().find_map(|j| match j {
        quatrelle_engine::model::Job::AskLlm { id, .. } => Some(id),
        _ => None,
    });
    let ask = ask.expect("the rewrite was asked");
    a.session.apply(quatrelle_engine::model::Event::LlmAnswered { id: ask, text: "Salut".into(), error: None });
    frame(&ctx, &mut a, vec![]);
    let use_it = settled_text_at(&ctx, &mut a, "Use this text", size).expect("Use this text");
    click_at(&ctx, &mut a, use_it, 1, size);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert!(ctx.memory(|m| m.has_focus(id)), "the text has the keyboard after using the rewrite");
    let after = a.session.compose.as_ref().unwrap().clone();
    assert_ne!(after.body, original.body, "the rewrite went in");
    // Ctrl+Z: exactly the text, styles and picture of before.
    frame(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    let back = a.session.compose.as_ref().unwrap();
    assert_eq!((back.body.clone(), back.styles.clone()), (original.body.clone(), original.styles.clone()), "one step back");
    assert_eq!(back.inline, original.inline, "the picture is the same");
    // Ctrl+Y: the rewrite again.
    frame(&ctx, &mut a, vec![key(Key::Y, Modifiers::COMMAND)]);
    let again = a.session.compose.as_ref().unwrap();
    assert_eq!((again.body.clone(), again.styles.clone()), (after.body.clone(), after.styles.clone()), "one step forward");
    let _ = std::fs::remove_dir_all(dir);
}

/// The ribbon's Rewrite while the cursor is in a block of the original shown as it came asks
/// about the whole written text, not about the block's selection applied to it.
#[test]
fn rewrite_in_a_quoted_block_uses_the_whole_written_text() {
    let (mut a, dir) = app("rewrite-quote-selection");
    let handed = Handed::default();
    a.backend = Box::new(Recorder(handed.clone()));
    a.session.prefs.reply_window = false;
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    a.run("assistant.settings", json!({"url": "http://localhost:11434/v1", "model": "llama3"}));
    frame(&ctx, &mut a, vec![]);
    let open = a.session.list.iter().find(|m| m.subject == "Open call: winter group show").map(|m| m.key.to_id()).expect("the sample newsletter");
    a.run("list.open", json!({"key": open}));
    frame(&ctx, &mut a, vec![]);
    a.run("mail.reply", json!({}));
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    a.run("compose.popOut", json!({}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let (compose, n) = {
        let c = a.session.compose.as_ref().expect("a reply");
        let q = c.quote.clone().expect("the original as it came");
        let n = (0..q.cells()).find(|n| q.cell(*n).is_some_and(|(t, _)| t.contains("Dear artists"))).expect("the greeting block");
        (c.id, n)
    };
    if let Some(c) = a.session.compose.as_mut() {
        c.body = format!("WHOLE BODY\n\n{}", c.body);
        c.styles = quatrelle_engine::mime::rich::Styles::plain(&c.body);
    }
    frame(&ctx, &mut a, vec![]);
    let body = a.session.compose.as_ref().map(crate::rich_edit::body_id).expect("a reply");
    ctx.memory_mut(|m| m.surrender_focus(body));
    a.ui.quote_focus = Some((compose, n));
    frame(&ctx, &mut a, vec![]);
    if let Some(ed) = a.ui.quote_editors.get_mut(&(compose, n)) {
        ed.in_quote = true;
        ed.sel = 3..8;
    }
    assert_eq!(a.ui.quote_focus, Some((compose, n)), "the block has the cursor");
    let rewrite = ribbon_rewrite(&ctx, &mut a, size);
    click_at(&ctx, &mut a, rewrite, 1, size);
    let shorter = settled_text_at(&ctx, &mut a, "Shorter", size).expect("Shorter");
    click_at(&ctx, &mut a, shorter, 1, size);
    for _ in 0..4 {
        frame(&ctx, &mut a, vec![]);
    }
    let sent = handed.borrow().iter().find_map(|j| match j {
        quatrelle_engine::model::Job::AskLlm { messages, .. } => messages.get(1).map(|m| m.1.clone()),
        _ => None,
    });
    let sent = sent.unwrap_or_else(|| panic!("the rewrite was asked: {:?}", a.session.compose.as_ref().map(|c| (&c.rewrite, &c.error))));
    assert!(sent.contains("WHOLE BODY"), "the written text, not the quote's selection: {sent:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A selection in a quoted block is not a selection in the written text: the ribbon asks about
/// the whole text sent, never about positions from the quote.
#[test]
fn a_quoted_blocks_selection_is_not_the_written_texts_selection() {
    let mut ed = crate::rich_edit::Editor::new(7);
    ed.sel = 3..8;
    ed.in_quote = true;
    assert_eq!(crate::rich_edit::rewrite_selection(&ed), None);
    ed.in_quote = false;
    assert_eq!(crate::rich_edit::rewrite_selection(&ed), Some(3..8));
}

/// What Use will put is shown as it will read: no Markdown marks, no marker, each picture a
/// word.
#[test]
fn a_rewrite_preview_shows_no_marks_or_tokens() {
    let (mut a, dir) = app("rewrite-preview");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 1200.0);
    a.run("assistant.settings", json!({"url": "http://localhost:11434/v1", "model": "llama3"}));
    a.run("mail.new", json!({}));
    let (text, styles) = quatrelle_engine::mime::rich::from_markup("**Bonjour**\n\n![logo](cid:logo@studio.example)");
    if let Some(c) = a.session.compose.as_mut() {
        c.signature.clear();
        c.body = text;
        c.styles = styles;
        c.inline = vec![quatrelle_engine::mime::InlinePicture {
            cid: "logo@studio.example".into(),
            name: "logo.png".into(),
            mime: "image/png".into(),
            data: Vec::new(),
        }];
    }
    frame(&ctx, &mut a, vec![]);
    a.run("assistant.rewrite", json!({"how": "shorter"}));
    let (id, sent) = a
        .session
        .take_jobs()
        .into_iter()
        .find_map(|j| match j {
            quatrelle_engine::model::Job::AskLlm { id, messages, .. } => messages.get(1).map(|m| (id, m.1.clone())),
            _ => None,
        })
        .expect("the rewrite was asked");
    let marker = sent.split_whitespace().find(|w| w.starts_with("QUATRELLE-IMAGE-") && w.ends_with("-END")).expect("a picture marker");
    a.session.apply(quatrelle_engine::model::Event::LlmAnswered { id, text: format!("**Salut** Zoé,\n\n{marker}"), error: None });
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    let drawn: String = frame_shapes(&ctx, &mut a, vec![], size)
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!drawn.contains("QUATRELLE-IMAGE"), "{drawn}");
    assert!(!drawn.contains("**"), "{drawn}");
    assert!(drawn.contains("Salut Zoé"), "{drawn}");
    assert!(drawn.contains("[picture]") || drawn.contains("[image]"), "the picture as a word: {drawn}");
    let _ = std::fs::remove_dir_all(dir);
}

/// When the part changed, Use is refused where it is seen, the offer stays, and no undo step is
/// made for the refusal: Ctrl+Z goes straight to the step before.
#[test]
fn a_refused_use_says_why_and_leaves_the_undo_history_alone() {
    let (mut a, dir) = app("rewrite-refused");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    a.run("assistant.settings", json!({"url": "http://localhost:11434/v1", "model": "llama3"}));
    a.session.prefs.reply_window = false;
    a.run("list.open", json!({"key": "sample:1:1:INBOX"}));
    a.run("mail.reply", json!({}));
    if let Some(c) = a.session.compose.as_mut() {
        c.body = format!("Hello Ann\n\n{}", c.body);
        c.styles = quatrelle_engine::mime::rich::Styles::plain(&c.body);
    }
    frame(&ctx, &mut a, vec![]);
    let id = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("!".into())]);
    assert!(a.session.compose.as_ref().unwrap().body.contains('!'), "typed");
    a.run("assistant.rewrite", json!({"how": "fix"}));
    let ask = a.session.take_jobs().into_iter().find_map(|j| match j {
        quatrelle_engine::model::Job::AskLlm { id, .. } => Some(id),
        _ => None,
    });
    let ask = ask.expect("the rewrite was asked");
    a.session.apply(quatrelle_engine::model::Event::LlmAnswered { id: ask, text: "Salut Ann".into(), error: None });
    frame(&ctx, &mut a, vec![]);
    let before_edit = a.session.compose.as_ref().unwrap().body.clone();
    // Change the part under the offer: select all and type.
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![Event::Text("Changed".into())]);
    assert_eq!(a.session.compose.as_ref().unwrap().body, "Changed");
    let use_it = settled_text_at(&ctx, &mut a, "Use this text", size).expect("Use this text");
    click_at(&ctx, &mut a, use_it, 1, size);
    assert_eq!(a.session.compose.as_ref().unwrap().body, "Changed", "the offer did not overwrite it");
    assert!(a.session.compose.as_ref().unwrap().rewrite.is_some(), "the offer stays");
    let drawn: String = frame_shapes(&ctx, &mut a, vec![], size)
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(drawn.contains("changed while it was being rewritten"), "{drawn}");
    // Ctrl+Z undoes the typing, not a dead step pushed by the refused Use.
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    assert_eq!(a.session.compose.as_ref().unwrap().body, before_edit, "one step back is the typing");
    let _ = std::fs::remove_dir_all(dir);
}
