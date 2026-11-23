// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Rewrite keeps what Markdown carries and what it doesn't: bold, lists and links come back as
//! styles, pictures come back at their place through markers, and a selection alone is replaced.

use super::*;
use quatrelle_mime::rich::{PICTURE, Styles};

/// A message written from light marks, with a picture at `cid`.
fn written(s: &mut Session, marked: &str, cid: &str) {
    s.run("mail.new", &json!({})).unwrap();
    let (text, styles) = quatrelle_mime::rich::from_markup(marked);
    let c = s.compose.as_mut().unwrap();
    c.signature.clear();
    c.body = text;
    c.styles = styles;
    c.inline = vec![quatrelle_mime::InlinePicture { cid: cid.into(), name: "logo.png".into(), mime: "image/png".into(), data: Vec::new() }];
}

/// Ask a rewrite and return its id and the text sent to the model.
fn ask(s: &mut Session, how: &str, selection: Option<(usize, usize)>) -> (u64, String) {
    let mut params = json!({ "how": how });
    if let Some((from, to)) = selection {
        params["from"] = json!(from);
        params["to"] = json!(to);
    }
    let id = s.run("assistant.rewrite", &params).unwrap()["asked"].as_u64().unwrap();
    let sent = s
        .take_jobs()
        .into_iter()
        .find_map(|j| match j {
            Job::AskLlm { messages, .. } => messages.get(1).map(|m| m.1.clone()),
            _ => None,
        })
        .unwrap_or_default();
    (id, sent)
}

/// The marker sent for a picture.
fn marker(sent: &str) -> String {
    sent.split(|c: char| c.is_whitespace() || matches!(c, ',' | '.' | ';' | ':'))
        .find(|w| w.starts_with("QUATRELLE-IMAGE-") && w.ends_with("-END"))
        .unwrap_or_default()
        .to_string()
}

#[test]
fn a_rewrite_keeps_bold_lists_and_its_picture() {
    let mut s = sample_session("rewrite-format");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    written(
        &mut s,
        "# Titre\n\n**Bonjour** Zoé et [le studio](https://studio.example),\n\n- un\n- deux\n\n![logo](cid:logo@studio.example)",
        "logo@studio.example",
    );
    let (id, sent) = ask(&mut s, "formal", None);
    assert!(sent.contains("# Titre") && sent.contains("**Bonjour**") && sent.contains("- un") && sent.contains("- deux"), "{sent}");
    assert!(sent.contains("[le studio](https://studio.example)"), "{sent}");
    assert!(sent.contains("logo@studio.example") || !marker(&sent).is_empty(), "the picture is a marker: {sent}");
    let m = marker(&sent);
    assert!(!m.is_empty(), "a picture marker reached the model: {sent}");
    s.apply(Event::LlmAnswered {
        id,
        text: format!("# Salut\n\n**Salut** Zoé et [le studio](https://studio.example),\n\n- uno\n- due\n\n{m}"),
        error: None,
    });
    s.run("assistant.endRewrite", &json!({"use": true})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert!(c.body.starts_with("Salut\n\nSalut Zoé"), "{:?}", c.body);
    assert!(c.styles.style(0).bold && c.styles.style(0).size == Some(20), "the heading came back");
    assert!(c.body.contains("Salut") && c.body.contains("le studio"), "{:?}", c.body);
    assert!((0..c.body.chars().count()).any(|i| c.styles.style(i).link.as_deref() == Some("https://studio.example")), "the link came back");
    assert_eq!(c.styles.para(4).list, quatrelle_mime::rich::List::Bullet, "{:?} {:?}", c.body, c.styles.paras);
    assert_eq!(c.styles.para(5).list, quatrelle_mime::rich::List::Bullet, "{:?} {:?}", c.body, c.styles.paras);
    let at = c.body.chars().position(|ch| ch == PICTURE).unwrap_or(usize::MAX);
    assert!(at < c.body.chars().count(), "the picture is still in the text: {:?}", c.body);
    assert_eq!(c.styles.style(at).picture.as_deref(), Some("logo@studio.example"));
    assert!(c.body.chars().filter(|ch| *ch == PICTURE).count() == 1, "and only once");
}

#[test]
fn a_selection_rewrites_only_that_part() {
    let mut s = sample_session("rewrite-selection");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    let body = "Keep this. Change this. Keep that.";
    s.run("mail.new", &json!({})).unwrap();
    if let Some(c) = s.compose.as_mut() {
        c.signature.clear();
        c.body = body.into();
        c.styles = Styles::plain(body);
    }
    let start = body.find("Change this.").unwrap_or(0);
    let end = start + "Change this.".len();
    let (id, sent) = ask(&mut s, "formal", Some((start, end)));
    assert_eq!(sent, "Change this.", "only the selection is sent");
    s.apply(Event::LlmAnswered { id, text: "Change that.".into(), error: None });
    s.run("assistant.endRewrite", &json!({"use": true})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().body, "Keep this. Change that. Keep that.");
}

#[test]
fn an_image_the_model_lost_goes_back_at_its_paragraphs_end() {
    let mut s = sample_session("rewrite-lost-image");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    written(&mut s, "First\n\n![logo](cid:logo@studio.example)\n\nLast", "logo@studio.example");
    let (id, sent) = ask(&mut s, "shorter", None);
    assert!(!marker(&sent).is_empty(), "the picture went as a marker: {sent}");
    s.apply(Event::LlmAnswered { id, text: "First\n\nLast".into(), error: None });
    s.run("assistant.endRewrite", &json!({"use": true})).unwrap();
    let c = s.compose.as_ref().unwrap();
    let count = c.body.chars().filter(|ch| *ch == PICTURE).count();
    assert_eq!(count, 1, "the picture is never dropped: {:?}", c.body);
    // It stood in the paragraph "Last", and comes back at its end.
    assert!(c.body.starts_with("First\n\nLast"), "{:?}", c.body);
    assert!(
        c.body.chars().position(|ch| ch == PICTURE).is_some_and(|at| !c.body[..].chars().skip(at + 1).take(2).any(|ch| ch == '\n')),
        "{:?}",
        c.body
    );
    let at = c.body.chars().position(|ch| ch == PICTURE).unwrap();
    assert_eq!(c.styles.style(at).picture.as_deref(), Some("logo@studio.example"));
}

/// A token of the marker's form already in the text, or one the model made up, is no picture.
#[test]
fn a_marker_that_was_not_sent_is_no_picture() {
    let mut s = sample_session("rewrite-invented-marker");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    let body = "Keep QUATRELLE-IMAGE-7-1-END as words.";
    s.run("mail.new", &json!({})).unwrap();
    if let Some(c) = s.compose.as_mut() {
        c.signature.clear();
        c.body = body.into();
        c.styles = Styles::plain(body);
    }
    let (id, sent) = ask(&mut s, "shorter", None);
    assert!(sent.contains("QUATRELLE-IMAGE-7-1-END"), "{sent}");
    s.apply(Event::LlmAnswered { id, text: format!("{body} QUATRELLE-IMAGE-999-1-END"), error: None });
    s.run("assistant.endRewrite", &json!({"use": true})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.body.chars().filter(|ch| *ch == PICTURE).count(), 0, "{:?}", c.body);
    assert!(c.body.contains("QUATRELLE-IMAGE-7-1-END"), "the user's words stay: {:?}", c.body);
    assert!(!c.body.contains("QUATRELLE-IMAGE-999-1-END"), "the model's marker goes: {:?}", c.body);
}

/// A marker written twice puts one picture back, and the second copy is no text; a marker
/// broken before its end still brings its picture, with no token left over.
#[test]
fn a_duplicate_or_broken_marker_leaves_no_token() {
    let mut s = sample_session("rewrite-marker-leftovers");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    written(
        &mut s,
        "Before

![logo](cid:logo@studio.example)

After",
        "logo@studio.example",
    );
    let (id, sent) = ask(&mut s, "shorter", None);
    let m = marker(&sent);
    assert!(!m.is_empty(), "{sent}");
    s.apply(Event::LlmAnswered { id, text: format!("One {m} two {m}"), error: None });
    s.run("assistant.endRewrite", &json!({"use": true})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.body.chars().filter(|ch| *ch == PICTURE).count(), 1, "{:?}", c.body);
    assert!(!c.body.contains("QUATRELLE-IMAGE"), "{:?}", c.body);

    // The same marker, stopped before its end: the picture goes at the paragraph's end.
    let mut s = sample_session("rewrite-marker-broken");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    written(
        &mut s,
        "First

![logo](cid:logo@studio.example)

Last",
        "logo@studio.example",
    );
    let (id, sent) = ask(&mut s, "shorter", None);
    let m = marker(&sent);
    let broken = m.strip_suffix("-END").unwrap_or(&m).to_string();
    s.apply(Event::LlmAnswered {
        id,
        text: format!(
            "First

{broken}"
        ),
        error: None,
    });
    s.run("assistant.endRewrite", &json!({"use": true})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.body.chars().filter(|ch| *ch == PICTURE).count(), 1, "{:?}", c.body);
    assert!(!c.body.contains("QUATRELLE-IMAGE"), "{:?}", c.body);
}

/// A marker the model invents with the right nonce, and one with another, are no text and no
/// picture.
#[test]
fn an_invented_marker_is_no_picture_and_no_text() {
    let mut s = sample_session("rewrite-marker-invented");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    written(&mut s, "![logo](cid:logo@studio.example)", "logo@studio.example");
    let (id, sent) = ask(&mut s, "shorter", None);
    let m = marker(&sent);
    let nonce = m.split('-').nth(2).unwrap_or("0");
    s.apply(Event::LlmAnswered { id, text: format!("Salut {m} et QUATRELLE-IMAGE-{nonce}-99-END et QUATRELLE-IMAGE-4-2-END"), error: None });
    s.run("assistant.endRewrite", &json!({"use": true})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.body.chars().filter(|ch| *ch == PICTURE).count(), 1, "the real one stays, the invented ones are no pictures: {:?}", c.body);
    assert!(!c.body.contains("QUATRELLE-IMAGE"), "{:?}", c.body);
}

/// Text changed where the rewrite would go is not overwritten: Use refuses, and the rewrite
/// stays on offer. Text added after it is kept.
#[test]
fn text_typed_over_the_part_is_not_overwritten() {
    let mut s = sample_session("rewrite-overwrite");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    if let Some(c) = s.compose.as_mut() {
        c.signature.clear();
        c.body = "Hello".into();
        c.styles = Styles::plain("Hello");
    }
    let (id, _) = ask(&mut s, "fix", None);
    s.apply(Event::LlmAnswered { id, text: "Salut".into(), error: None });
    if let Some(c) = s.compose.as_mut() {
        c.body = "Howdy".into();
        c.styles = Styles::plain("Howdy");
    }
    assert!(s.run("assistant.endRewrite", &json!({"use": true})).is_err(), "the text changed under it");
    assert_eq!(s.compose.as_ref().unwrap().body, "Howdy");
    assert!(s.compose.as_ref().unwrap().rewrite.is_some(), "the offer stays");
    // Ask again and append after the part sent: the rewrite goes in, the addition stays.
    s.run("assistant.endRewrite", &json!({"use": false})).unwrap();
    let (id, _) = ask(&mut s, "fix", None);
    s.apply(Event::LlmAnswered { id, text: "Salut".into(), error: None });
    if let Some(c) = s.compose.as_mut() {
        c.body.push_str(", and more");
        c.styles = Styles::plain(&c.body);
    }
    s.run("assistant.endRewrite", &json!({"use": true})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().body, "Salut, and more");
}

/// Markdown carries no colour, typeface or size: what comes back keeps the look at the start of
/// the part rewritten rather than plain text.
#[test]
fn a_base_style_markdown_cannot_carry_is_kept() {
    let mut s = sample_session("rewrite-base-style");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    if let Some(c) = s.compose.as_mut() {
        c.signature.clear();
        c.body = "Bonjour".into();
        c.styles = Styles::plain("Bonjour");
        c.styles.restyle(&c.body, 0..7, |st| quatrelle_mime::rich::Style {
            color: Some([0x12, 0x34, 0x56]),
            font: Some("Aptos".into()),
            size: Some(14),
            ..st.clone()
        });
    }
    let (id, _) = ask(&mut s, "fix", None);
    s.apply(Event::LlmAnswered { id, text: "Salut".into(), error: None });
    s.run("assistant.endRewrite", &json!({"use": true})).unwrap();
    let st = s.compose.as_ref().unwrap().styles.style(0);
    assert_eq!((st.color, st.font.as_deref(), st.size), (Some([0x12, 0x34, 0x56]), Some("Aptos"), Some(14)));
}

/// The look Markdown cannot carry comes from the part's common text, not from its title: a
/// title's 20 points must not become the size of every line.
#[test]
fn a_base_look_is_the_common_one_not_the_titles() {
    let mut s = sample_session("rewrite-base-heading");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    if let Some(c) = s.compose.as_mut() {
        c.signature.clear();
        let (text, styles) = quatrelle_mime::rich::from_markup("# Titre\n\nBonjour Zoé");
        c.body = text;
        c.styles = styles;
        let at = c.body.find("Bonjour Zoé").unwrap_or(0);
        let start = c.body.get(..at).map_or(0, |t| t.chars().count());
        c.styles.restyle(&c.body, start..start + "Bonjour Zoé".chars().count(), |st| quatrelle_mime::rich::Style {
            color: Some([0x12, 0x34, 0x56]),
            font: Some("Aptos".into()),
            size: Some(14),
            ..st.clone()
        });
    }
    let (id, _) = ask(&mut s, "formal", None);
    s.apply(Event::LlmAnswered { id, text: "# Salut\n\nSalut Zoé".into(), error: None });
    s.run("assistant.endRewrite", &json!({"use": true})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.styles.style(0).size, Some(20), "the title keeps its size: {:?}", c.body);
    let at = c.body.find("Salut Zoé").unwrap_or(0);
    let start = c.body.get(..at).map_or(0, |t| t.chars().count());
    let st = c.styles.style(start);
    assert_eq!((st.color, st.font.as_deref(), st.size), (Some([0x12, 0x34, 0x56]), Some("Aptos"), Some(14)), "{:?}", c.body);
}

/// A red first word does not colour the rest: the common black wins.
#[test]
fn a_base_look_takes_the_common_colour() {
    let mut s = sample_session("rewrite-base-colour");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    if let Some(c) = s.compose.as_mut() {
        c.signature.clear();
        c.body = "Rouge et le reste".into();
        c.styles = Styles::plain(&c.body);
        c.styles.restyle(&c.body, 0..6, |st| quatrelle_mime::rich::Style { color: Some([0xCC, 0, 0]), ..st.clone() });
        c.styles.restyle(&c.body, 6..c.body.chars().count(), |st| quatrelle_mime::rich::Style { color: Some([0, 0, 0]), ..st.clone() });
    }
    let (id, _) = ask(&mut s, "formal", None);
    s.apply(Event::LlmAnswered { id, text: "Salut".into(), error: None });
    s.run("assistant.endRewrite", &json!({"use": true})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().styles.style(0).color, Some([0, 0, 0]));
}

/// A bold word at a title's size is no title when the rest of the line is ordinary: only a
/// whole line in bold at a heading size goes as `#`.
#[test]
fn a_bold_word_at_a_heading_size_is_no_title() {
    let mut s = sample_session("rewrite-heading-whole-line");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    if let Some(c) = s.compose.as_mut() {
        c.signature.clear();
        c.body = "Urgent : la réunion est avancée".into();
        c.styles = Styles::plain(&c.body);
        c.styles.restyle(&c.body, 0..6, |st| quatrelle_mime::rich::Style { bold: true, size: Some(14), ..st.clone() });
    }
    let (id, sent) = ask(&mut s, "formal", None);
    assert!(!sent.contains("###"), "not a title: {sent}");
    s.apply(Event::LlmAnswered { id, text: sent, error: None });
    s.run("assistant.endRewrite", &json!({"use": true})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert!(c.styles.style(0).bold && c.styles.style(0).size == Some(14));
    assert!(!c.styles.style(20).bold && c.styles.style(20).size.is_none(), "the rest is ordinary: {:?}", c.body);
}

/// A part too long for one rewrite is refused before a model is asked, with a way out.
#[test]
fn a_too_long_part_is_not_sent_to_the_model() {
    let mut s = sample_session("rewrite-too-long-part");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    if let Some(c) = s.compose.as_mut() {
        c.signature.clear();
        c.body = "a".repeat(30_000);
        c.styles = Styles::plain(&c.body);
    }
    let r = s.run("assistant.rewrite", &json!({"how": "fix"}));
    assert!(r.is_err(), "a part this long is refused");
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::AskLlm { .. })), "nothing was asked");
}

/// An answer far longer than the part sent is not cut: Use refuses and the text stays intact.
#[test]
fn a_too_long_answer_is_refused_rather_than_cut() {
    let mut s = sample_session("rewrite-too-long-answer");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    if let Some(c) = s.compose.as_mut() {
        c.signature.clear();
        c.body = "Bonjour".into();
        c.styles = Styles::plain("Bonjour");
    }
    let (id, _) = ask(&mut s, "fix", None);
    s.apply(Event::LlmAnswered { id, text: "a".repeat(30_000), error: None });
    let r = s.run("assistant.endRewrite", &json!({"use": true}));
    assert!(r.is_err(), "the answer is too long");
    assert_eq!(s.compose.as_ref().unwrap().body, "Bonjour");
}

/// A word made bold in the part sent is a change too: Use refuses rather than overwrite it.
#[test]
fn a_style_changed_in_the_part_is_not_overwritten() {
    let mut s = sample_session("rewrite-style-changed");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    if let Some(c) = s.compose.as_mut() {
        c.signature.clear();
        c.body = "Hello Ann".into();
        c.styles = Styles::plain(&c.body);
    }
    let (id, _) = ask(&mut s, "fix", None);
    s.apply(Event::LlmAnswered { id, text: "Salut Ann".into(), error: None });
    if let Some(c) = s.compose.as_mut() {
        c.styles.fit(&c.body);
        let text = c.body.clone();
        c.styles.restyle(&text, 0..5, |st| quatrelle_mime::rich::Style { bold: true, ..st.clone() });
    }
    assert!(s.run("assistant.endRewrite", &json!({"use": true})).is_err(), "the style changed under it");
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.body, "Hello Ann");
    assert!(c.styles.style(0).bold, "the user's change stays");
    assert!(c.rewrite.is_some(), "the offer stays");
}

/// A letter typed before the part moves it: Use replaces the part where it is now and keeps the
/// letter.
#[test]
fn a_letter_typed_before_the_part_is_kept() {
    let mut s = sample_session("rewrite-before-letter");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    if let Some(c) = s.compose.as_mut() {
        c.signature.clear();
        c.body = "Bonjour".into();
        c.styles = Styles::plain("Bonjour");
    }
    let (id, _) = ask(&mut s, "fix", None);
    s.apply(Event::LlmAnswered { id, text: "Salut".into(), error: None });
    if let Some(c) = s.compose.as_mut() {
        c.body.insert(0, 'X');
        c.styles = Styles::plain(&c.body);
    }
    s.run("assistant.endRewrite", &json!({"use": true})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().body, "XSalut");
}

/// An empty answer (or only blank spaces) is refused rather than wiping the part.
#[test]
fn an_empty_answer_is_refused() {
    let mut s = sample_session("rewrite-empty-answer");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    if let Some(c) = s.compose.as_mut() {
        c.signature.clear();
        c.body = "Bonjour".into();
        c.styles = Styles::plain("Bonjour");
    }
    let (id, _) = ask(&mut s, "fix", None);
    s.apply(Event::LlmAnswered { id, text: "   \n".into(), error: None });
    assert!(s.run("assistant.endRewrite", &json!({"use": true})).is_err(), "nothing to put in");
    assert_eq!(s.compose.as_ref().unwrap().body, "Bonjour");
}

/// A picture without a cid still goes to the model as a marker and comes back at its place.
#[test]
fn a_picture_without_a_cid_is_kept() {
    let mut s = sample_session("rewrite-no-cid");
    s.run("assistant.settings", &json!({"url": "http://localhost:11434/v1", "model": "llama3"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    if let Some(c) = s.compose.as_mut() {
        c.signature.clear();
        c.body = format!("Before {} After", PICTURE);
        c.styles = Styles::plain(&c.body);
    }
    let (id, sent) = ask(&mut s, "formal", None);
    let m = marker(&sent);
    assert!(!m.is_empty(), "the picture went as a marker: {sent}");
    s.apply(Event::LlmAnswered { id, text: format!("One {m} two"), error: None });
    s.run("assistant.endRewrite", &json!({"use": true})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.body.chars().filter(|ch| *ch == PICTURE).count(), 1, "{:?}", c.body);
    assert!(!c.body.contains("QUATRELLE-IMAGE"), "{:?}", c.body);
}
