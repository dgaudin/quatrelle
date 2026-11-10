// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a selection that goes across the text of a reply and the blocks of the original
//! under it (the owner's report, 2026-11-10: a drag from the header lines down to under the
//! sender's logo stopped at the end of the header), and what it takes out in one go.

use quatrelle_engine::QuotePoint;

use super::*;

/// An original as business mail comes: two paragraphs, a signature in a table with its logo, a
/// rule over an older message, a table of figures, a last line.
const ORIGINAL: &str = r##"<html><body><p>First paragraph of the original.</p><p>Second paragraph of it.</p>
<table width="400"><tr><td width="300">Morgan Lee<br>Accounts</td><td width="100"><img src="https://mill.example/logo.png" width="80" height="40" alt="logo"></td></tr></table>
<hr><p>An older message under it.</p>
<table width="400"><tr><td>Left cell words</td><td>Right cell words</td></tr><tr><td>Lower left</td><td>Lower right</td></tr></table>
<p>The very end.</p></body></html>"##;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 1500.0);

/// A frame with `events`; what it copied.
fn run(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> Option<String> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    out.platform_output
        .commands
        .into_iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t),
            _ => None,
        })
        .next_back()
}

fn step(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) {
    run(ctx, a, events);
}

fn button(pos: egui::Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers }
}

/// A reply to [`ORIGINAL`], drawn.
fn reply(name: &str) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir) = app_with_html(name, "Quarterly figures", ORIGINAL);
    a.session.prefs.reply_window = false;
    let ctx = egui::Context::default();
    step(&ctx, &mut a, vec![]);
    let key = a.session.list.iter().find(|m| m.subject == "Quarterly figures").map(|m| m.key.to_id()).expect("the message");
    a.run("list.open", json!({"key": key}));
    step(&ctx, &mut a, vec![]);
    a.run("mail.reply", json!({}));
    for _ in 0..4 {
        step(&ctx, &mut a, vec![]);
    }
    (a, dir, ctx)
}

fn compose(a: &MailApp) -> &quatrelle_engine::Compose {
    a.session.compose.as_ref().expect("a message being written")
}

/// The block (0: the text, then the original's) that holds `words`, and where they start.
fn at(a: &MailApp, words: &str) -> QuotePoint {
    let c = compose(a);
    let found = c.selectable_blocks().into_iter().find_map(|block| {
        let (t, _) = c.block_text(block)?;
        let b = t.find(words)?;
        Some(QuotePoint { block, at: t.get(..b)?.chars().count() })
    });
    found.unwrap_or_else(|| panic!("{words:?} in the message"))
}

fn place(a: &MailApp, p: QuotePoint) -> egui::Pos2 {
    a.ui.across.place(p).unwrap_or_else(|| panic!("{p:?} drawn"))
}

/// What goes out as HTML.
fn html(a: &MailApp) -> String {
    compose(a).bodies().1.unwrap_or_default()
}

/// Drag from `from` to `to`, a few frames on the way, and let go.
fn drag(ctx: &egui::Context, a: &mut MailApp, from: egui::Pos2, to: egui::Pos2) {
    step(ctx, a, vec![Event::PointerMoved(from)]);
    step(ctx, a, vec![button(from, true, Modifiers::NONE)]);
    for k in 1..=6 {
        step(ctx, a, vec![Event::PointerMoved(from + (to - from) * (k as f32 / 6.0))]);
    }
    step(ctx, a, vec![Event::PointerMoved(to)]);
    step(ctx, a, vec![button(to, false, Modifiers::NONE)]);
    step(ctx, a, vec![]);
}

/// A click at `pos` with `modifiers` held.
fn click(ctx: &egui::Context, a: &mut MailApp, pos: egui::Pos2, modifiers: Modifiers) {
    step(ctx, a, vec![Event::PointerMoved(pos), Event::ModifiersChanged(modifiers)]);
    step(ctx, a, vec![button(pos, true, modifiers)]);
    step(ctx, a, vec![button(pos, false, modifiers), Event::ModifiersChanged(Modifiers::NONE)]);
    step(ctx, a, vec![]);
}

/// The owner's report, replayed: a drag from the middle of the header lines (in the text) to
/// under the logo of the sender's signature selects all of it, lit in every block on the way;
/// Delete takes it out, the signature's table with it, and the cursor stays where the two ends
/// meet. What goes out has neither the paragraphs nor the logo, and the rest untouched. Ctrl+Z
/// puts it all back in one step, Ctrl+Y takes it out again; saved and opened again, the draft is
/// as it was left.
#[test]
fn a_drag_from_the_header_to_under_the_logo_selects_it_all_and_delete_takes_it_out() {
    let (mut a, dir, ctx) = reply("across-drag");
    let came = html(&a);
    let from = at(&a, "To: ");
    let logo = at(&a, "\u{FFFC}");
    let under =
        a.ui.across.drawn.iter().find(|(k, _)| *k == logo.block).map(|(_, d)| d.rect.right_bottom() - egui::vec2(3.0, 2.0)).expect("the logo's cell");
    let start = place(&a, from);
    drag(&ctx, &mut a, start, under);
    let sel = a.ui.across.sel.clone().expect("a selection across the blocks (it stopped at the end of the text's own)");
    assert_eq!((sel.anchor, sel.head), (from, QuotePoint { block: logo.block, at: 1 }));
    // Copied, it reads in order.
    let copied = run(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    let order: Vec<usize> = ["To: ", "First paragraph", "Second paragraph", "Morgan Lee"].iter().map(|w| copied.find(w).expect(w)).collect();
    assert!(order.is_sorted() && !copied.contains("older message"), "{copied}");
    let kept: String = compose(&a).body.chars().take(from.at).collect();
    step(&ctx, &mut a, vec![key(Key::Delete, Modifiers::NONE)]);
    step(&ctx, &mut a, vec![]);
    let c = compose(&a);
    assert_eq!(c.body, kept, "the text is cut where the drag started");
    let out = html(&a);
    for gone in ["First paragraph", "Second paragraph", "Morgan Lee", "logo.png"] {
        assert!(!out.contains(gone), "{gone} still goes out: {out}");
    }
    for stays in ["An older message under it.", "Left cell words", "Lower right", "The very end.", "<table"] {
        assert!(out.contains(stays), "{stays} no longer goes out: {out}");
    }
    assert_eq!(out.matches("<table").count(), 1, "the signature's table went, the other stays: {out}");
    assert!(a.ui.across.sel.is_none());
    assert!(ctx.memory(|m| m.has_focus(crate::rich_edit::body_id(c))), "the cursor is where the two ends meet");
    // Typed now, it goes there.
    step(&ctx, &mut a, vec![Event::Text("Z".into())]);
    assert_eq!(compose(&a).body, format!("{kept}Z"));
    // Ctrl+Z: the letter, then all that was taken out, in one step.
    step(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    assert_eq!(compose(&a).body, kept);
    step(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    step(&ctx, &mut a, vec![]);
    assert_eq!(html(&a), came, "Ctrl+Z puts it all back");
    assert!(a.ui.across.sel.as_ref().is_some_and(|s| s.spans()), "selected as it was");
    // Ctrl+Y takes it out again.
    step(&ctx, &mut a, vec![key(Key::Y, Modifiers::COMMAND)]);
    step(&ctx, &mut a, vec![]);
    assert_eq!(html(&a), out);
    // Saved and opened again from Drafts: as it was left.
    a.run("compose.saveDraft", json!({}));
    a.session.compose = None;
    a.run("folder.open", json!({"account": "sample", "folder": "Drafts"}));
    let key = a.session.list.iter().find(|m| m.subject == "Re: Quarterly figures").map(|m| m.key.clone()).expect("in Drafts");
    a.run("list.open", json!({"key": key.to_id()}));
    step(&ctx, &mut a, vec![]);
    a.act(crate::Action::Run("draft.edit", json!({"key": key.to_id()})));
    step(&ctx, &mut a, vec![]);
    step(&ctx, &mut a, vec![]);
    let text = compose(&a).quote.as_ref().map(|q| q.text()).expect("its original");
    assert!(
        text.contains("An older message") && text.contains("Lower left") && !text.contains("First paragraph") && !text.contains("Morgan Lee"),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Ctrl+A takes the text and the whole original; Delete then empties both, and Ctrl+Z brings
/// them back. Typing over such a selection puts what's typed in its place.
#[test]
fn ctrl_a_selects_the_text_and_the_original_and_delete_empties_both() {
    let (mut a, dir, ctx) = reply("across-all");
    let came = html(&a);
    let body = crate::rich_edit::body_id(compose(&a));
    ctx.memory_mut(|m| m.request_focus(body));
    step(&ctx, &mut a, vec![]);
    step(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    step(&ctx, &mut a, vec![]);
    let sel = a.ui.across.sel.clone().expect("all of it selected");
    let last = compose(&a).selectable_blocks().last().copied().unwrap();
    assert_eq!((sel.anchor, sel.head.block), (QuotePoint::default(), last));
    let copied = run(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert!(copied.contains("Subject: Quarterly figures") && copied.contains("The very end."), "{copied}");
    step(&ctx, &mut a, vec![key(Key::Delete, Modifiers::NONE)]);
    step(&ctx, &mut a, vec![]);
    let c = compose(&a);
    assert!(c.body.is_empty(), "{}", c.body);
    let out = html(&a);
    assert!(!out.contains("First paragraph") && !out.contains("<table") && !out.contains("The very end"), "{out}");
    assert_eq!(c.selectable_blocks(), [0]);
    step(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    step(&ctx, &mut a, vec![]);
    assert_eq!(html(&a), came);
    // Typed over it: in its place.
    step(&ctx, &mut a, vec![Event::Text("Fine.".into())]);
    step(&ctx, &mut a, vec![]);
    assert_eq!(compose(&a).body, "Fine.");
    assert!(!html(&a).contains("First paragraph"));
    let _ = std::fs::remove_dir_all(dir);
}

/// A table wholly inside the selection goes; one taken in part keeps its cells, those touched
/// emptied of what was selected. Shift with a click carries the selection on into another block.
#[test]
fn a_table_taken_whole_goes_and_one_taken_in_part_is_emptied() {
    let (mut a, dir, ctx) = reply("across-tables");
    // From inside the first cell to inside the last: the table stays.
    let (from, to) = (at(&a, "cell words"), at(&a, "right"));
    let (p, q) = (place(&a, from), place(&a, to));
    click(&ctx, &mut a, p, Modifiers::NONE);
    click(&ctx, &mut a, q, Modifiers::SHIFT);
    let sel = a.ui.across.sel.clone().expect("Shift with a click goes on into the other cell");
    assert_eq!((sel.anchor.block, sel.head.block), (from.block, to.block));
    assert!((sel.anchor.at.abs_diff(from.at) <= 1) && sel.head.at.abs_diff(to.at) <= 1, "{sel:?}");
    step(&ctx, &mut a, vec![Event::Text("X".into())]);
    step(&ctx, &mut a, vec![]);
    let out = html(&a);
    assert_eq!(out.matches("<table").count(), 2, "both tables stay: {out}");
    assert!(!out.contains("Right cell words") && !out.contains("Lower left"), "the cells between are emptied: {out}");
    let c = compose(&a);
    let texts: Vec<String> = c.selectable_blocks().into_iter().filter_map(|k| c.block_text(k)).map(|(t, _)| t).collect();
    assert!(
        texts.iter().any(|t| t.starts_with("Left") && t.ends_with('X')) && texts.iter().any(|t| t.ends_with("right") && !t.contains("Lower")),
        "{texts:?}"
    );
    step(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    step(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    step(&ctx, &mut a, vec![]);
    assert!(html(&a).contains("Right cell words"), "put back");
    // From the paragraph over it to the line under it: the table goes, and what is left of
    // the two paragraphs is one.
    let (from, to) = (at(&a, "under it."), at(&a, "very end."));
    let (p, q) = (place(&a, from), place(&a, to));
    click(&ctx, &mut a, p, Modifiers::NONE);
    click(&ctx, &mut a, q, Modifiers::SHIFT);
    assert!(a.ui.across.sel.as_ref().is_some_and(|s| s.spans()));
    step(&ctx, &mut a, vec![key(Key::Backspace, Modifiers::NONE)]);
    step(&ctx, &mut a, vec![]);
    let out = html(&a);
    assert_eq!(out.matches("<table").count(), 1, "the table between went: {out}");
    assert!(!out.contains("Left cell words") && out.contains("Morgan Lee"), "{out}");
    let joined = compose(&a).quote.as_ref().map(|q| q.text()).unwrap_or_default();
    assert!(joined.contains("An older message") && joined.contains("very end.") && !joined.contains("under it."), "{joined}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Shift with the arrows goes on from a block's end into the next, and back; a selection inside
/// one block is that block's own, as before.
#[test]
fn shift_with_the_arrows_leaves_a_block_for_the_next() {
    let (mut a, dir, ctx) = reply("across-keys");
    let body = crate::rich_edit::body_id(compose(&a));
    ctx.memory_mut(|m| m.request_focus(body));
    step(&ctx, &mut a, vec![]);
    // Inside the text: the text box's own selection.
    step(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::COMMAND)]);
    step(&ctx, &mut a, vec![key(Key::ArrowLeft, Modifiers::SHIFT)]);
    step(&ctx, &mut a, vec![]);
    assert!(a.ui.across.sel.is_none(), "one block: its own selection");
    let len = compose(&a).body.chars().count();
    assert_eq!(crate::rich_edit::caret(&ctx, body), Some((len, len - 1)));
    // Past its end: into the original's first block, a letter at a time, then a row.
    step(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::COMMAND), key(Key::ArrowLeft, Modifiers::NONE), key(Key::ArrowLeft, Modifiers::NONE)]);
    step(&ctx, &mut a, vec![key(Key::ArrowRight, Modifiers::SHIFT), key(Key::ArrowRight, Modifiers::SHIFT)]);
    assert!(a.ui.across.sel.is_none(), "up to the text's end: still its own");
    step(&ctx, &mut a, vec![key(Key::ArrowRight, Modifiers::SHIFT), key(Key::ArrowRight, Modifiers::SHIFT)]);
    step(&ctx, &mut a, vec![]);
    let sel = a.ui.across.sel.clone().expect("on into the original");
    assert_eq!((sel.anchor, sel.head), (QuotePoint { block: 0, at: len - 2 }, QuotePoint { block: 1, at: 1 }));
    step(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::SHIFT)]);
    step(&ctx, &mut a, vec![]);
    let head = a.ui.across.sel.as_ref().map(|s| s.head).unwrap();
    assert!(head.block == 1 && head.at > 1, "{head:?}");
    let copied = run(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert!(copied.contains("First paragraph"), "{copied}");
    // Back into the text: its own selection again.
    step(&ctx, &mut a, vec![key(Key::Home, Modifiers::SHIFT | Modifiers::COMMAND)]);
    step(&ctx, &mut a, vec![]);
    assert!(a.ui.across.sel.is_none());
    assert_eq!(crate::rich_edit::caret(&ctx, body), Some((len - 2, 0)));
    let _ = std::fs::remove_dir_all(dir);
}
