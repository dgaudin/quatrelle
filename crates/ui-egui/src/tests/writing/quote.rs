// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the original under a reply or forward, shown as it came under the text being
//! written and changed where it stands, as Outlook lets it be; and a draft opened again from
//! Drafts. With `QUATRELLE_FRAME_PNG` set to a folder, the replies are painted there (light and
//! dark), and the message sent as it reads back in the reading pane.

use super::super::list::follow_up::{Textures, frame_with_size};
use super::*;

/// A signature laid out in nested tables, as business mail writes one: a column of icons beside
/// the numbers, a coloured warning line, a logo cell, a centred line under it all.
const CARD: &str = r##"<html><body><p>Dear Riley,</p><p>We will share the updated invoice soon.</p>
<table width="640" cellpadding="0" cellspacing="0"><tr>
<td width="40" valign="top"><table><tr><td><img src="https://mill.example/phone.png" width="16" alt="tel"></td></tr><tr><td><img src="https://mill.example/web.png" width="16" alt="web"></td></tr></table></td>
<td width="420"><table><tr><td><b>Morgan Lee</b> | Accounts</td></tr><tr><td>+00 1234 5678</td></tr>
<tr><td><a href="https://mill.example/">mill.example</a> | <b style="color:#d32f2f">Please mind our new bank details</b></td></tr>
<tr><td>1 Example Road, Example Town</td></tr></table></td>
<td width="180" align="right" bgcolor="#1f3a5f" style="color:#ffffff;padding:10px"><b>THE MILL</b></td></tr></table>
<table width="640"><tr><td align="center" style="color:#777777">Printed on recycled paper</td></tr></table>
</body></html>"##;

/// The block of text of the quote being written that holds `words`.
fn cell_with(a: &MailApp, words: &str) -> usize {
    let q = a.session.compose.as_ref().and_then(|c| c.quote.clone()).expect("a quote");
    (0..q.cells()).find(|n| q.cell(*n).is_some_and(|(t, _)| t.contains(words))).unwrap_or_else(|| panic!("{words} in the quote"))
}

/// The owner's report upstream: a reply showed the original's columns as broken lines of text.
/// The sample newsletter (a band, two columns, a label over its poster) is quoted as it came under
/// the text, in both themes, and typed into where it stands: its blocks of text are text boxes,
/// the main text keeps the keyboard's typing when it has the cursor, and the message stays one
/// being written while it's drawn (it's out of the session meanwhile).
#[test]
fn a_reply_shows_the_original_as_it_came_and_changes_it_in_place() {
    let (mut a, dir) = app("quote-shown");
    a.session.prefs.reply_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let open = a.session.list.iter().find(|m| m.subject == "Open call: winter group show").map(|m| m.key.to_id()).expect("the sample newsletter");
    a.run("list.open", json!({"key": open}));
    frame(&ctx, &mut a, vec![]);
    a.run("mail.reply", json!({}));
    for dark in [Some(false), Some(true)] {
        a.ui.dark = dark;
        frame(&ctx, &mut a, vec![]);
        frame(&ctx, &mut a, vec![]);
    }
    let c = a.session.compose.as_ref().expect("still being written after it was drawn");
    let q = c.quote.clone().expect("kept as it came");
    assert!(!q.parsed.layout.is_empty());
    assert!(q.edits.is_empty(), "drawing it changes nothing in it");
    // Typed at the top, the text takes it; the quote stays as it came.
    let id = crate::rich_edit::body_id(c);
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("Thanks".into())]);
    let c = a.session.compose.as_ref().unwrap();
    assert!(c.body.contains("Thanks") && c.quote.as_ref().is_some_and(|q| q.edits.is_empty()), "{}", c.body);
    // Typed into a block of the original: that block changes, where it stands.
    let n = cell_with(&a, "Dear artists");
    let compose = c.id;
    ctx.memory_mut(|m| m.request_focus(crate::reading::cell_box(compose, n)));
    frame(&ctx, &mut a, vec![]);
    // Command+Up is the start of the text on every system (Ctrl+Home isn't on macOS).
    frame(&ctx, &mut a, vec![key(Key::ArrowUp, Modifiers::COMMAND), Event::Text("Hello! ".into())]);
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.as_ref().unwrap();
    let q = c.quote.clone().unwrap();
    let (text, _) = q.cell(n).unwrap();
    assert!(text.starts_with("Hello! Dear artists"), "{text}");
    assert_eq!(q.edits.len(), 1, "that block alone");
    assert!(!c.body.contains("Hello!"), "not in the text: {}", c.body);
    assert_eq!(a.ui.quote_focus, Some((c.id, n)), "the ribbon works on it");
    let html = c.bodies().1.unwrap_or_default();
    assert!(html.contains("Hello! Dear artists") && html.contains("<table"), "{html}");
    // Bold from the keyboard, in that block (its words selected).
    frame(&ctx, &mut a, vec![key(Key::ArrowUp, Modifiers::COMMAND), key(Key::ArrowRight, Modifiers::SHIFT)]);
    frame(&ctx, &mut a, vec![key(Key::B, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    let q = a.session.compose.as_ref().and_then(|c| c.quote.clone()).unwrap();
    assert!(q.cell(n).is_some_and(|(_, s)| s.style(0).bold), "Ctrl+B in the block");
    // In a window of its own too.
    a.run("compose.popOut", json!({}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.compose.as_ref().is_some_and(|c| c.quote.is_some()));
    let _ = std::fs::remove_dir_all(dir);
}

/// A draft in Drafts opens again to carry on writing it, as it was (its original as it came
/// under it): from a double-click in the list, as Outlook opens one.
#[test]
fn a_draft_opens_again_from_drafts() {
    let (mut a, dir) = app("draft-again");
    a.session.prefs.reply_window = false;
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let open = a.session.list.iter().find(|m| m.subject == "Open call: winter group show").map(|m| m.key.to_id()).expect("the sample newsletter");
    a.run("list.open", json!({"key": open}));
    a.run("mail.reply", json!({}));
    frame(&ctx, &mut a, vec![]);
    a.run("compose.saveDraft", json!({}));
    a.session.compose = None;
    a.run("folder.open", json!({"account": "sample", "folder": "Drafts"}));
    let key = a.session.list.iter().find(|m| m.subject == "Re: Open call: winter group show").map(|m| m.key.clone()).expect("in Drafts");
    a.run("list.open", json!({"key": key.to_id()}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Run("draft.edit", json!({"key": key.to_id()})));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.as_ref().expect("being written again");
    assert_eq!(c.draft.as_ref(), Some(&key));
    assert!(c.quote.as_ref().is_some_and(|q| !q.parsed.layout.is_empty()), "its original as it came");
    let _ = std::fs::remove_dir_all(dir);
}

/// A reply to the sample newsletter and to a message with a signature in tables, light and dark
/// (painted into `QUATRELLE_FRAME_PNG` when set), then sent and read back in the reading pane: its
/// original keeps its columns and colours there too.
#[test]
fn replies_to_layouts_draw_light_and_dark_and_read_back_so() {
    let (mut a, dir) = app_with_html("quote-shots", "Invoice", CARD);
    a.session.prefs.reply_window = false;
    let ctx = egui::Context::default();
    let mut tex = Textures::default();
    let pictures = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let picture = |name: &str| pictures.as_ref().map(|d| d.join(format!("{name}.png")));
    frame_with_size(&ctx, &mut a, &mut tex, vec![], None, (1280, 1300));
    for (subject, name) in [("Open call: winter group show", "newsletter"), ("Invoice", "card")] {
        a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
        let key = a.session.list.iter().find(|m| m.subject == subject).map(|m| m.key.to_id()).expect(subject);
        a.run("list.open", json!({"key": key}));
        for _ in 0..4 {
            frame_with_size(&ctx, &mut a, &mut tex, vec![], None, (1280, 1300));
        }
        a.run("mail.reply", json!({}));
        // Typed above the original, its look (the line over it) kept.
        if let Some(c) = a.session.compose.as_mut() {
            c.styles.fit(&c.body);
            let typed = quatrelle_engine::mime::rich::Styles::plain("Thank you, noted.");
            quatrelle_engine::mime::rich::splice(&mut c.body, &mut c.styles, 0..0, "Thank you, noted.", &typed);
        }
        for (dark, theme) in [(Some(false), "light"), (Some(true), "dark")] {
            a.ui.dark = dark;
            for _ in 0..6 {
                frame_with_size(&ctx, &mut a, &mut tex, vec![], None, (1280, 1300));
            }
            let (_, shapes) = frame_with_size(&ctx, &mut a, &mut tex, vec![], picture(&format!("s6-reply-{name}-{theme}")), (1280, 1300));
            assert!(!shapes.is_empty());
            assert!(a.session.compose.as_ref().is_some_and(|c| c.quote.is_some()), "{name} {theme}: still being written");
        }
        let c = a.session.compose.clone().unwrap();
        let html = c.bodies().1.unwrap_or_default();
        assert!(html.contains("<table") && html.contains("quatrelle-quote"), "{name}: {html}");
        a.run("compose.send", json!({"checked": true}));
        assert!(a.session.compose.is_none(), "{name}: sent");
        // Read back from Sent, as it went.
        a.run("folder.open", json!({"account": "sample", "folder": "Sent"}));
        let sent = a.session.list.iter().find(|m| m.subject == format!("Re: {subject}")).map(|m| m.key.to_id()).expect("in Sent");
        a.run("list.open", json!({"key": sent}));
        for (dark, theme) in [(Some(false), "light"), (Some(true), "dark")] {
            a.ui.dark = dark;
            for _ in 0..6 {
                frame_with_size(&ctx, &mut a, &mut tex, vec![], None, (1280, 1300));
            }
            frame_with_size(&ctx, &mut a, &mut tex, vec![], picture(&format!("s6-sent-{name}-{theme}")), (1280, 1300));
        }
        let p = a.session.reading.as_ref().and_then(|r| r.parsed.clone()).expect("read back");
        assert!(!p.layout.is_empty(), "{name}: its columns come back");
        let words = if name == "card" { "Please mind our new bank details" } else { "Dear artists" };
        assert!(p.text.contains("Thank you, noted.") && p.text.contains(words), "{name}: {}", p.text);
    }
    let _ = std::fs::remove_dir_all(dir);
}
