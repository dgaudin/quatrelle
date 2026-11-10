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

/// The selection across the blocks of the message being written.
fn ac(a: &MailApp) -> &crate::reading::AcrossState {
    a.ui.across.get(compose(a).id).expect("the message was drawn")
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
    ac(a).place(p).unwrap_or_else(|| panic!("{p:?} drawn"))
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
        ac(&a).drawn.iter().find(|(k, _)| *k == logo.block).map(|(_, d)| d.rect.right_bottom() - egui::vec2(3.0, 2.0)).expect("the logo's cell");
    let start = place(&a, from);
    drag(&ctx, &mut a, start, under);
    let sel = ac(&a).sel.clone().expect("a selection across the blocks (it stopped at the end of the text's own)");
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
    assert!(ac(&a).sel.is_none());
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
    assert!(ac(&a).sel.as_ref().is_some_and(|s| s.spans()), "selected as it was");
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
    let sel = ac(&a).sel.clone().expect("all of it selected");
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
    let sel = ac(&a).sel.clone().expect("Shift with a click goes on into the other cell");
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
    assert!(ac(&a).sel.as_ref().is_some_and(|s| s.spans()));
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
    assert!(ac(&a).sel.is_none(), "one block: its own selection");
    let len = compose(&a).body.chars().count();
    assert_eq!(crate::rich_edit::caret(&ctx, body), Some((len, len - 1)));
    // Past its end: into the original's first block, a letter at a time, then a row.
    step(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::COMMAND), key(Key::ArrowLeft, Modifiers::NONE), key(Key::ArrowLeft, Modifiers::NONE)]);
    step(&ctx, &mut a, vec![key(Key::ArrowRight, Modifiers::SHIFT), key(Key::ArrowRight, Modifiers::SHIFT)]);
    assert!(ac(&a).sel.is_none(), "up to the text's end: still its own");
    step(&ctx, &mut a, vec![key(Key::ArrowRight, Modifiers::SHIFT), key(Key::ArrowRight, Modifiers::SHIFT)]);
    step(&ctx, &mut a, vec![]);
    let sel = ac(&a).sel.clone().expect("on into the original");
    assert_eq!((sel.anchor, sel.head), (QuotePoint { block: 0, at: len - 2 }, QuotePoint { block: 1, at: 1 }));
    step(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::SHIFT)]);
    step(&ctx, &mut a, vec![]);
    let head = ac(&a).sel.as_ref().map(|s| s.head).unwrap();
    assert!(head.block == 1 && head.at > 1, "{head:?}");
    let copied = run(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert!(copied.contains("First paragraph"), "{copied}");
    // Back into the text: its own selection again.
    step(&ctx, &mut a, vec![key(Key::Home, Modifiers::SHIFT | Modifiers::COMMAND)]);
    step(&ctx, &mut a, vec![]);
    assert!(ac(&a).sel.is_none());
    assert_eq!(crate::rich_edit::caret(&ctx, body), Some((len - 2, 0)));
    let _ = std::fs::remove_dir_all(dir);
}

/// Seen again in the real window (2026-11-10): an original holding two messages, the first
/// ending in a signature laid out in a table (logo, name), then a rule and the message it
/// answered, its header full of links and its text long enough to scroll.
const TWO_MESSAGES: &str = r##"<html><body lang="FR"><div class="WordSection1">
<p class="MsoNormal">The client would like to move on, could you answer?<o:p></o:p></p>
<p class="MsoNormal"><o:p>&nbsp;</o:p></p>
<p class="MsoNormal">Thanks<o:p></o:p></p>
<table class="MsoNormalTable" border="0" cellspacing="0" cellpadding="0"><tr>
<td style="padding:0cm 5.4pt 0cm 5.4pt"><p class="MsoNormal"><img width="180" height="70" src="https://mill.example/logo.png" alt="logo"><o:p></o:p></p></td>
<td style="border:none;border-left:solid #7F7F7F 1.0pt;padding:0cm 5.4pt 0cm 5.4pt"><p class="MsoNormal"><b>Morgan Lee</b><o:p></o:p></p>
<p class="MsoNormal">Tel.: 00 00 00 00<o:p></o:p></p><p class="MsoNormal"><a href="mailto:morgan@mill.example">morgan@mill.example</a><o:p></o:p></p></td></tr></table>
<p class="MsoNormal"><o:p>&nbsp;</o:p></p>
<div><div style="border:none;border-top:solid #E1E1E1 1.0pt;padding:3.0pt 0cm 0cm 0cm">
<p class="MsoNormal"><b>From:</b> Sam Hill &lt;<a href="mailto:sam@studio.example">sam@studio.example</a>&gt; <br>
<b>Sent:</b> Monday, 9 November 2026 09:36<br>
<b>To:</b> Robin Field &lt;<a href="mailto:robin@mill.example">robin@mill.example</a>&gt;<br>
<b>Cc:</b> <a href="mailto:a@studio.example">a@studio.example</a>; Kim Ward &lt;<a href="mailto:kim@studio.example">kim@studio.example</a>&gt;<br>
<b>Subject:</b> RE: Tunnel figures<o:p></o:p></p></div></div>
<p class="MsoNormal"><o:p>&nbsp;</o:p></p>
<p class="MsoNormal">Hello Robin,<o:p></o:p></p><p class="MsoNormal">Can you confirm a few points:<o:p></o:p></p>
<ol><li class="MsoListParagraph"><b>Routes received</b>: do you see both ranges on your two sessions?</li>
<li class="MsoListParagraph"><b>The whole list of prefixes</b> you will announce in the tunnel.</li>
<li class="MsoListParagraph"><b>Which tunnel is preferred</b>: we see a longer path on your side.</li></ol>
<p class="MsoNormal"><span style="color:#C00000">Yes, quite. It is active and passive, the first one active by default.</span><o:p></o:p></p>
<p class="MsoNormal">Line 01 of the long text.</p><p class="MsoNormal">Line 02 of the long text.</p><p class="MsoNormal">Line 03 of the long text.</p>
<p class="MsoNormal">Line 04 of the long text.</p><p class="MsoNormal">Line 05 of the long text.</p><p class="MsoNormal">Line 06 of the long text.</p>
<p class="MsoNormal">Line 07 of the long text.</p><p class="MsoNormal">Line 08 of the long text.</p><p class="MsoNormal">Line 09 of the long text.</p>
<p class="MsoNormal">Line 10 of the long text.</p><p class="MsoNormal">Line 11 of the long text.</p><p class="MsoNormal">Line 12 of the long text.</p>
<p class="MsoNormal">Regards,<o:p></o:p></p><p class="MsoNormal"><b><span style="color:#0070C0">Sam Hill</span></b><o:p></o:p></p>
</div></body></html>"##;

/// A frame in a window of `size`.
fn step_in(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>, size: egui::Vec2) {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
}

/// A reply to [`TWO_MESSAGES`] in a window of `size`, drawn; `before` runs once the app is up.
fn reply_to_two(name: &str, size: egui::Vec2, before: impl FnOnce(&mut MailApp)) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir) = app_with_html(name, "Tunnel figures", TWO_MESSAGES);
    a.session.prefs.reply_window = false;
    let ctx = egui::Context::default();
    step_in(&ctx, &mut a, vec![], size);
    before(&mut a);
    step_in(&ctx, &mut a, vec![], size);
    let key = a.session.list.iter().find(|m| m.subject == "Tunnel figures").map(|m| m.key.to_id()).expect("the message");
    a.run("list.open", json!({"key": key}));
    step_in(&ctx, &mut a, vec![], size);
    a.run("mail.reply", json!({}));
    for _ in 0..6 {
        step_in(&ctx, &mut a, vec![], size);
    }
    (a, dir, ctx)
}

/// The sender's signature written in HTML, its logo loaded: it is drawn over its room in the text.
fn html_signature(a: &mut MailApp) {
    let id = a.session.accounts.first().map(|x| x.config.id.clone()).unwrap();
    let logo = "https://logo.example/logo.png";
    let html = format!(
        r#"<table align="left" border="0" cellpadding="0" cellspacing="0" style="float:left"><tr><td valign="middle"><img src="{logo}" width="120px" border="0" style="width:120px;height:48px"></td><td style="padding-left:8px"><b>Riley MORENO</b><br>Tel.: 00 00<br>riley@studio.example</td></tr></table>"#
    );
    a.run("account.setSignature", json!({"account": id, "signature": "", "rawHtml": html}));
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(120, 48, image::Rgba([20, 90, 160, 255])))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    a.session.apply(quatrelle_engine::model::Event::Fetched { url: logo.into(), data: Some(png), error: None });
}

/// The case seen in the real window, as far as it can be replayed without a pointer: in a low window, a
/// drag from the rule over the header lines (under a signature written in HTML) held at the
/// view's bottom goes on through the first message's signature table into the message quoted
/// under it, to its end as the view scrolls, and stays so once let go. (It passed before the
/// second report's changes too: what the video shows wasn't reproduced this way.)
#[test]
fn a_drag_held_at_the_views_edge_goes_on_into_the_message_quoted_under_the_first() {
    let low = egui::vec2(1280.0, 800.0);
    let (mut a, dir, ctx) = reply_to_two("across-two", low, html_signature);
    let from = at(&a, "\nFrom: ");
    let start = place(&a, from);
    let last = compose(&a).selectable_blocks().last().copied().unwrap();
    let end = compose(&a).block_text(last).map(|(t, _)| t.chars().count()).unwrap();
    step_in(&ctx, &mut a, vec![Event::PointerMoved(start)], low);
    step_in(&ctx, &mut a, vec![button(start, true, Modifiers::NONE)], low);
    let bottom = egui::pos2(start.x + 40.0, 796.0);
    for k in 1..=10 {
        step_in(&ctx, &mut a, vec![Event::PointerMoved(start + (bottom - start) * (k as f32 / 10.0))], low);
    }
    let seen = ac(&a).sel.clone().expect("out of the text into the original");
    assert!(seen.anchor == from && seen.head.block >= 1, "{seen:?}");
    for _ in 0..240 {
        step_in(&ctx, &mut a, vec![Event::PointerMoved(bottom)], low);
    }
    let held = ac(&a).sel.clone().expect("still selected");
    assert_eq!((held.anchor, held.head), (from, QuotePoint { block: last, at: end }), "to the end of the message quoted under the first");
    step_in(&ctx, &mut a, vec![button(bottom, false, Modifiers::NONE)], low);
    for _ in 0..3 {
        step_in(&ctx, &mut a, vec![], low);
    }
    let after = ac(&a).sel.clone().expect("kept once let go");
    assert_eq!((after.anchor, after.head), (held.anchor, held.head));
    let top = ac(&a).drawn.iter().find(|(k, _)| *k == 0).map(|(_, d)| d.rect.top()).unwrap();
    assert!(top < 0.0, "the view stays where the drag scrolled it: {top}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Several messages are drawn in one frame when one has a window of its own. One state for
/// them all was emptied by each in turn: with another message open, no drag ever left its block.
/// Here the other message is drawn between the frames of the drag, as its window would be.
#[test]
fn another_message_being_written_does_not_stop_the_selection() {
    let (mut a, dir, ctx) = reply_to_two("across-other", SIZE, |a| {
        a.run("mail.new", json!({}));
        a.run("compose.update", json!({"subject": "Another one"}));
    });
    let reply = compose(&a).id;
    let other = a.session.all_composes().map(|c| c.id).find(|id| *id != reply).expect("another message");
    let (from, to) = (at(&a, "To: "), at(&a, "Hello Robin"));
    let (p, q) = (place(&a, from), place(&a, to));
    let turn = |a: &mut MailApp, events: Vec<Event>| {
        step(&ctx, a, events);
        assert!(a.use_compose(other));
        step(&ctx, a, vec![]);
        assert!(a.use_compose(reply));
    };
    turn(&mut a, vec![Event::PointerMoved(p)]);
    turn(&mut a, vec![button(p, true, Modifiers::NONE)]);
    for k in 1..=6 {
        turn(&mut a, vec![Event::PointerMoved(p + (q - p) * (k as f32 / 6.0))]);
    }
    turn(&mut a, vec![button(q, false, Modifiers::NONE)]);
    let sel = ac(&a).sel.clone().expect("a selection across the blocks, with another message drawn in between");
    assert_eq!((sel.anchor.block, sel.head.block), (from.block, to.block), "{sel:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A precaution, not a defect reproduced: the block a drag started in is also taken from the
/// text box egui itself is dragging a selection in, should the press not have been seen here.
#[test]
fn a_drag_whose_press_was_not_seen_still_leaves_its_block() {
    let (mut a, dir, ctx) = reply_to_two("across-unseen", SIZE, |_| {});
    let id = compose(&a).id;
    let (from, to) = (at(&a, "To: "), at(&a, "Hello Robin"));
    let (p, q) = (place(&a, from), place(&a, to));
    step(&ctx, &mut a, vec![Event::PointerMoved(p)]);
    step(&ctx, &mut a, vec![button(p, true, Modifiers::NONE)]);
    step(&ctx, &mut a, vec![Event::PointerMoved(p + egui::vec2(12.0, 0.0))]);
    a.ui.across.of(id).pressed = None;
    for k in 1..=6 {
        step(&ctx, &mut a, vec![Event::PointerMoved(p + (q - p) * (k as f32 / 6.0))]);
    }
    let sel = ac(&a).sel.clone().expect("found from the text box being dragged");
    assert_eq!((sel.anchor.block, sel.head.block), (from.block, to.block), "{sel:?}");
    let _ = std::fs::remove_dir_all(dir);
}
