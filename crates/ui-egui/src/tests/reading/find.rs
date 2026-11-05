// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: find in the message (R20), the bar at the bottom of the reading pane. `/` opens it
//! (not while typing in a box), the words are looked for as typed, accents and capitals aside, in
//! the text the message shows (plain, formatted, its tables, a long one by chunks), each found
//! one lit, the current one more, and brought into view. With `QUATRELLE_FRAME_PNG` set to a
//! folder, the bar and its marks are painted there, light and dark.

use std::time::{Duration, Instant};

use super::*;

/// A plain-text message of this body, dated `day` (October 2026), put in the Inbox, which is shown.
fn put_text(a: &mut MailApp, uid: u32, day: u32, subject: &str, body: &str) -> MessageKey {
    let raw = format!(
        "From: Ana <ana@posteo.example>\r\nTo: Riley <riley@studio.example>\r\nSubject: {subject}\r\n\
         Date: {day} Oct 2026 09:00:00 +0000\r\nMessage-ID: <find-{uid}@x>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n{body}"
    );
    put_raw(a, uid, raw.as_bytes())
}

fn put_raw(a: &mut MailApp, uid: u32, raw: &[u8]) -> MessageKey {
    let store = a.session.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid };
    let flags = quatrelle_engine::model::Flags { seen: true, ..Default::default() };
    inbox.messages.insert(0, quatrelle_engine::mime::summary_from_full(raw, key.clone(), flags, 0, NOW));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, raw).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    key
}

/// `key` opened, and frames until it has come in.
fn open(a: &mut MailApp, ctx: &egui::Context, key: &MessageKey) {
    a.run("list.open", json!({"key": key.to_id()}));
    for _ in 0..12 {
        frame(ctx, a, vec![]);
    }
}

/// `/` as a keyboard sends it: the key, then the character.
fn slash() -> Vec<Event> {
    vec![key(Key::Slash, Modifiers::NONE), Event::Text("/".into())]
}

/// A frame with `events`, then a few more for what they started; what the last one drew.
fn frames(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> Vec<egui::Shape> {
    frame(ctx, a, events);
    for _ in 0..4 {
        frame(ctx, a, vec![]);
    }
    frame_shapes(ctx, a, vec![], egui::vec2(1280.0, 800.0))
}

/// What the bar says: its count ("3 of 12", "No match"), if it says one.
fn count(shapes: &[egui::Shape]) -> Option<String> {
    shapes.iter().rev().find_map(|s| match s {
        egui::Shape::Text(t)
            if t.galley.text() == "No match" || t.galley.text().split_once(" of ").is_some_and(|(i, _)| i.parse::<u32>().is_ok()) =>
        {
            Some(t.galley.text().to_string())
        }
        _ => None,
    })
}

/// The pieces of text drawn on `fill` (lit by the find bar), with where each starts.
fn lit(shapes: &[egui::Shape], fill: egui::Color32) -> Vec<(String, egui::Pos2)> {
    let mut out = Vec::new();
    for s in shapes {
        let egui::Shape::Text(t) = s else { continue };
        if t.opacity_factor <= 0.0 {
            continue;
        }
        for sec in t.galley.job.sections.iter().filter(|sec| sec.format.background == fill) {
            let (start, end): (usize, usize) = (sec.byte_range.start.into(), sec.byte_range.end.into());
            if let Some(text) = t.galley.job.text.get(start..end) {
                let at = t.galley.job.text.get(..start).map_or(0, |b| b.chars().count());
                out.push((text.to_string(), t.pos + t.galley.pos_from_cursor(egui::text::CCursor::new(at)).center().to_vec2()));
            }
        }
    }
    out
}

fn field_focused(ctx: &egui::Context) -> bool {
    ctx.memory(|m| m.has_focus(crate::reading::find_field_id(false, None)))
}

/// `/` in the reading pane opens the bar with the keyboard in its box (the `/` not typed in it);
/// in the search box, or in a reply written in the pane, it's typed as any letter is.
#[test]
fn slash_opens_the_bar_but_not_while_typing() {
    let (mut a, dir) = app("find-slash");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = put_text(&mut a, 9901, 20, "Team", "L'\u{c9}quipe se r\u{e9}unit.\r\n");
    open(&mut a, &ctx, &k);
    assert!(!a.ui.find.pane.open);
    frames(&ctx, &mut a, slash());
    assert!(a.ui.find.pane.open, "opened");
    assert!(field_focused(&ctx), "the keyboard in its box");
    assert_eq!(a.ui.find.pane.query, "", "the slash isn't typed in it");
    let shown = frames(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(!a.ui.find.pane.open, "Escape closes it");
    // A click in the message's text (selected as a box is, never typed in): `/` is the bar's.
    let at = shown.iter().rev().find_map(|s| match s {
        egui::Shape::Text(t) if t.galley.text().starts_with("L'\u{c9}quipe") => Some(t.pos + egui::vec2(20.0, 8.0)),
        _ => None,
    });
    click_at(&ctx, &mut a, at.expect("the message's text"), 1, egui::vec2(1280.0, 800.0));
    assert!(ctx.text_edit_focused(), "the text holds the keyboard");
    frames(&ctx, &mut a, slash());
    assert!(a.ui.find.pane.open && field_focused(&ctx), "opened from the message's text");
    frames(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    // In the search box: typed there.
    frames(&ctx, &mut a, vec![key(Key::E, Modifiers::COMMAND)]);
    assert!(a.ui.search_focused);
    frames(&ctx, &mut a, slash());
    assert!(!a.ui.find.pane.open, "not from the search box");
    assert!(a.ui.search_text.contains('/'), "{:?}", a.ui.search_text);
    let _ = std::fs::remove_dir_all(dir);
    // In a reply written in the pane: typed in it.
    let (mut a, dir) = app("find-slash-reply");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = put_text(&mut a, 9901, 20, "Team", "L'\u{c9}quipe se r\u{e9}unit.\r\n");
    open(&mut a, &ctx, &k);
    a.run("mail.reply", json!({}));
    frame(&ctx, &mut a, vec![]);
    let body = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    ctx.memory_mut(|m| m.request_focus(body));
    frames(&ctx, &mut a, slash());
    assert!(!a.ui.find.pane.open, "not from a reply");
    assert!(a.session.compose.as_ref().unwrap().body.starts_with('/'), "typed in the reply");
    let _ = std::fs::remove_dir_all(dir);
}

/// "equipe" finds "Équipe" and "l'équipe" (accents and capitals aside): 1 of 2; Enter goes to the
/// next, Shift+Enter back; F3 and Shift+F3 too, out of the box; a word that isn't there says so.
#[test]
fn typing_finds_the_words_accents_aside_and_enter_steps() {
    let (mut a, dir) = app("find-equipe");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = put_text(&mut a, 9902, 20, "Team", "\u{c9}quipe du lundi.\r\nToute l'\u{e9}quipe est l\u{e0}.\r\n");
    open(&mut a, &ctx, &k);
    frames(&ctx, &mut a, slash());
    let shown = frames(&ctx, &mut a, vec![Event::Text("equipe".into())]);
    assert_eq!(count(&shown).as_deref(), Some("1 of 2"));
    let (all, now, _) = crate::reading::find_fills(&a.ui.tokens);
    assert_eq!(lit(&shown, now).iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["\u{c9}quipe"], "the current one, as written");
    assert_eq!(lit(&shown, all).iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["\u{e9}quipe"], "the other one");
    let shown = frames(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    assert_eq!(count(&shown).as_deref(), Some("2 of 2"));
    assert!(field_focused(&ctx), "Enter keeps the keyboard in the box");
    let shown = frames(&ctx, &mut a, vec![key(Key::Enter, Modifiers::SHIFT)]);
    assert_eq!(count(&shown).as_deref(), Some("1 of 2"));
    // Out of the box: F3, Shift+F3.
    ctx.memory_mut(|m| m.surrender_focus(crate::reading::find_field_id(false, None)));
    let shown = frames(&ctx, &mut a, vec![key(Key::F3, Modifiers::NONE)]);
    assert_eq!(count(&shown).as_deref(), Some("2 of 2"));
    let shown = frames(&ctx, &mut a, vec![key(Key::F3, Modifiers::SHIFT)]);
    assert_eq!(count(&shown).as_deref(), Some("1 of 2"));
    // Escape out of the box closes it too.
    frames(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(!a.ui.find.pane.open);
    // Opened again, its words kept; one that isn't there.
    frames(&ctx, &mut a, slash());
    assert_eq!(a.ui.find.pane.query, "equipe");
    a.ui.find.pane.query = "zzqx".into();
    let shown = frames(&ctx, &mut a, vec![]);
    assert_eq!(count(&shown).as_deref(), Some("No match"));
    let _ = std::fs::remove_dir_all(dir);
}

/// A 300,000-character message laid out by chunks, a word at its very end: going to it lays out
/// its chunk, and the pane scrolls there.
#[test]
fn a_word_at_the_end_of_a_long_message_is_brought_into_view() {
    let (mut a, dir) = app("find-long");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let mut body: String = (0..4400).map(|i| format!("Line {i:05} of a long message, with words enough to wrap now and then.\r\n")).collect();
    body.push_str("The last word is Zanzibar.\r\n");
    let k = put_text(&mut a, 9903, 20, "Long", &body);
    open(&mut a, &ctx, &k);
    let text = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.text.clone()).unwrap();
    assert!(text.len() > 300_000, "{}", text.len());
    let n = crate::reading::chunks(&text, crate::reading::CHUNK).len();
    frames(&ctx, &mut a, slash());
    frame(&ctx, &mut a, vec![Event::Text("zanzibar".into())]);
    let shown = frames(&ctx, &mut a, vec![]);
    assert_eq!(count(&shown).as_deref(), Some("1 of 1"));
    let v = a.ui.long_text.values().next().unwrap();
    let last = v.shown.iter().find(|(i, _)| *i + 1 == n).map(|(_, r)| *r);
    let last = last.unwrap_or_else(|| panic!("the last chunk laid out: {:?}", v.shown));
    assert!(last.bottom() > 100.0 && last.top() < 800.0, "in view: {last:?}");
    let (_, now, _) = crate::reading::find_fills(&a.ui.tokens);
    let found = lit(&shown, now);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!((0.0..800.0).contains(&found[0].1.y), "{found:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A word in a table's cell of a formatted message is found and lit.
#[test]
fn a_word_in_a_table_cell_is_found_and_lit() {
    let html = "<html><body><p>Quarterly figures</p><table border=\"1\"><tr><td>Budget</td><td>Kangaroo 42</td></tr>\
                <tr><td>Travel</td><td>17</td></tr></table></body></html>";
    let (mut a, dir) = app_with_html("find-table", "Figures", html);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject == "Figures").map(|m| m.key.clone()).unwrap();
    open(&mut a, &ctx, &k);
    assert!(a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).is_some_and(|p| !p.layout.is_empty()), "laid out with its table");
    frames(&ctx, &mut a, slash());
    frame(&ctx, &mut a, vec![Event::Text("KANGAROO".into())]);
    let shown = frames(&ctx, &mut a, vec![]);
    assert_eq!(count(&shown).as_deref(), Some("1 of 1"));
    let (_, now, _) = crate::reading::find_fills(&a.ui.tokens);
    assert_eq!(lit(&shown, now).iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["Kangaroo"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Another message opened (a click, or Down in the list): the bar stays open with its words and
/// counts them in that one; Escape closes it.
#[test]
fn the_bar_stays_open_on_the_next_message_and_counts_again() {
    let (mut a, dir) = app("find-next");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let older = put_text(&mut a, 9904, 29, "Older", "Une \u{e9}quipe.\r\n");
    let newer = put_text(&mut a, 9905, 30, "Newer", "\u{c9}quipe, \u{e9}quipe, EQUIPE.\r\n");
    open(&mut a, &ctx, &newer);
    frames(&ctx, &mut a, slash());
    let shown = frames(&ctx, &mut a, vec![Event::Text("equipe".into())]);
    assert_eq!(count(&shown).as_deref(), Some("1 of 3"));
    // A click on the other one.
    open(&mut a, &ctx, &older);
    let shown = frames(&ctx, &mut a, vec![]);
    assert!(a.ui.find.pane.open && a.ui.find.pane.query == "equipe");
    assert_eq!(count(&shown).as_deref(), Some("1 of 1"));
    // Back, then Down in the list (the keyboard out of the box).
    open(&mut a, &ctx, &newer);
    ctx.memory_mut(|m| m.surrender_focus(crate::reading::find_field_id(false, None)));
    let shown = frames(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    assert_eq!(a.session.reading.as_ref().map(|r| r.key.clone()), Some(older), "Down went to the next one");
    assert_eq!(count(&shown).as_deref(), Some("1 of 1"));
    let shown = frames(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(!a.ui.find.pane.open);
    assert_eq!(count(&shown), None, "the bar is gone");
    let _ = std::fs::remove_dir_all(dir);
}

/// What finding costs in a 300,000-character text: its fold made once (when a message is first
/// looked in), then each search as a key is typed; both well within a frame in a release build
/// (`cargo test --release -p quatrelle-ui-egui find::what_finding_costs -- --nocapture`).
#[test]
fn what_finding_costs_in_a_long_text() {
    let mut text: String =
        (0..4400).map(|i| format!("Ligne {i:05} d'un long message de l'\u{e9}quipe, assez de mots pour aller \u{e0} la ligne.\n")).collect();
    text.push_str("Zanzibar.\n");
    assert!(text.len() > 300_000);
    let t = Instant::now();
    let doc = crate::reading::FindDoc::plain(&text);
    let fold = t.elapsed();
    let t = Instant::now();
    let (one, _) = doc.search("zanzibar");
    let (many, capped) = doc.search("e");
    let search = t.elapsed();
    assert_eq!(one.len(), 1);
    assert_eq!(many.len(), crate::reading::FIND_MOST, "counted up to the cap");
    assert!(capped);
    let (team, _) = doc.search("EQUIPE");
    assert_eq!(team.len(), 4400);
    assert!(text[team[0].clone()].eq("\u{e9}quipe"), "{:?}", &text[team[0].clone()]);
    println!("300,000 characters: folded in {:.2} ms, two searches in {:.2} ms", fold.as_secs_f64() * 1e3, search.as_secs_f64() * 1e3);
    if !cfg!(debug_assertions) {
        assert!(fold < Duration::from_millis(16) && search < Duration::from_millis(16), "{fold:?} {search:?}");
    }
}

/// The bar and its marks, light and dark, a plain message and a formatted one with a table:
/// painted into `QUATRELLE_FRAME_PNG` when set (to look at them), and drawn in any case.
#[test]
fn the_bar_and_its_marks_draw_light_and_dark() {
    use super::super::list::follow_up::{Textures, frame_with};
    let html = "<html><body><p>Hello team, the <b>\u{c9}quipe</b> report:</p><table border=\"1\" cellpadding=\"6\">\
                <tr><td>Name</td><td>Role</td></tr><tr><td>Ana</td><td>l'\u{e9}quipe image</td></tr></table>\
                <p>See you, the equipe.</p></body></html>";
    let (mut a, dir) = app_with_html("find-shots", "Figures", html);
    let ctx = egui::Context::default();
    let mut tex = Textures::default();
    let pictures = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let picture = |name: &str| pictures.as_ref().map(|d| d.join(format!("{name}.png")));
    frame_with(&ctx, &mut a, &mut tex, vec![], None);
    let html_key = a.session.list.iter().find(|m| m.subject == "Figures").map(|m| m.key.clone()).unwrap();
    let plain_key =
        put_text(&mut a, 9906, 20, "Team", "Bonjour,\r\n\r\nL'\u{c9}quipe se r\u{e9}unit lundi. Toute l'\u{e9}quipe est invit\u{e9}e.\r\n");
    for (key, name) in [(plain_key, "plain"), (html_key, "html")] {
        a.run("list.open", json!({"key": key.to_id()}));
        for _ in 0..10 {
            frame_with(&ctx, &mut a, &mut tex, vec![], None);
        }
        if !a.ui.find.pane.open {
            frame_with(&ctx, &mut a, &mut tex, slash(), None);
            frame_with(&ctx, &mut a, &mut tex, vec![Event::Text("equipe".into())], None);
        }
        for (dark, theme) in [(Some(false), "light"), (Some(true), "dark")] {
            a.ui.dark = dark;
            for _ in 0..6 {
                frame_with(&ctx, &mut a, &mut tex, vec![], None);
            }
            let (_, shown) = frame_with(&ctx, &mut a, &mut tex, vec![], picture(&format!("find-{name}-{theme}")));
            let (all, now, _) = crate::reading::find_fills(&a.ui.tokens);
            assert_eq!(lit(&shown, now).len(), 1, "{name} {theme}");
            assert!(!lit(&shown, all).is_empty(), "{name} {theme}");
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}
