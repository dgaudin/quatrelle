// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Round trips from the editor's text to Markdown and back, as Rewrite does them.

use super::*;
use crate::rich::{Para, Styles};

/// Send `text` with `styles`, then put the model's answer, the same Markdown, back through the
/// reader.
fn there_and_back(text: &str, styles: &Styles) -> (String, Styles) {
    let (sent, images) = outbound(text, styles, 7);
    inbound(&sent, Style::default(), &images, text, styles)
}

/// A plain text sent and copied back is the same text.
#[test]
fn ordinary_text_comes_back_as_it_was_written() {
    for text in [
        r"Le devis est dans C:\Partage\Devis",
        "2*3*4 = 24",
        "[Rappel] réunion",
        "_brouillon_",
        "Écrire à <zoe@studio.example>",
        "#1 de la liste",
        "Cordialement,\nZoé",
    ] {
        let styles = Styles::plain(text);
        let (back, _) = there_and_back(text, &styles);
        assert_eq!(back, text, "sent {:?}", outbound(text, &styles, 7).0);
    }
}

/// A line in the monospace font alone keeps its look.
#[test]
fn a_monospace_line_keeps_its_font() {
    let text = "let x = 1;";
    let mut styles = Styles::plain(text);
    styles.restyle(text, 0..text.chars().count(), |s| Style { mono: true, ..s.clone() });
    let (back, back_styles) = there_and_back(text, &styles);
    assert_eq!(back, text);
    assert!(back_styles.style(0).mono, "sent {:?}", outbound(text, &styles, 7).0);
}

/// A line break the model writes as a plain newline, without the two spaces the writer put
/// behind it, stays a line break.
#[test]
fn a_line_break_the_model_normalises_stays_a_line() {
    let text = "Cordialement,\nZoé Martin";
    let styles = Styles::plain(text);
    let (sent, images) = outbound(text, &styles, 7);
    let (back, _) = inbound(&sent.replace("  \n", "\n"), Style::default(), &images, text, &styles);
    assert_eq!(back, text, "sent {sent:?}");
}

/// A paragraph wider than the renderer's old 2000 columns is not cut in two.
#[test]
fn a_long_paragraph_comes_back_on_one_line() {
    let text = vec!["mot"; 750].join(" ");
    let styles = Styles::plain(&text);
    let (sent, images) = outbound(&text, &styles, 7);
    let (back, _) = inbound(&sent, Style::default(), &images, &text, &styles);
    assert!(!back.contains('\n'), "one line: {:?}", back.chars().take(80).collect::<String>());
    assert_eq!(back, text);
}

/// A line of ordinary text that would start a list, a quote, a code block or a heading stays
/// ordinary text; `&amp;` written out stays written out.
#[test]
fn text_that_looks_like_markdown_stays_text() {
    for text in ["- pas une liste", "+ plus", "• puce tapée", "3. Troisième point", "3) trois", "> pas une citation", "&amp; reste"] {
        let styles = Styles::plain(text);
        let (back, _) = there_and_back(text, &styles);
        assert_eq!(back, text, "sent {:?}", outbound(text, &styles, 7).0);
    }
    // Indented lines (up to three spaces) are read as their text, without the indent.
    let text = "- pas une liste";
    let styles = Styles::plain(text);
    let (sent, images) = outbound(text, &styles, 7);
    let (back, _) = inbound(&format!("  {sent}"), Style::default(), &images, text, &styles);
    assert_eq!(back, text, "no indent left: {back:?}");
}

/// A rule stays a rule between the paragraph before and the one after, with no line added or
/// lost; a line of box-drawing characters is still text.
#[test]
fn a_rule_stays_a_rule_and_box_lines_stay_text() {
    let text = "Texte\n\nSuite";
    let mut styles = Styles::plain(text);
    if let Some(p) = styles.paras.get_mut(1) {
        p.rule = true;
    }
    let (back, back_styles) = there_and_back(text, &styles);
    assert_eq!(back, text, "sent {:?}, paras {:?}", outbound(text, &styles, 7).0, back_styles.paras);
    assert!(back_styles.para(1).rule, "the line across is still one: {:?}", back_styles.paras);
    let box_line = "────────";
    let styles = Styles::plain(box_line);
    let (back, _) = there_and_back(box_line, &styles);
    assert_eq!(back, box_line, "sent {:?}", outbound(box_line, &styles, 7).0);
}

use proptest::prelude::*;

/// A character a message may hold, without white space at a line's edge or a newline.
fn any_char() -> impl Strategy<Value = char> {
    let mut all: Vec<char> = ('a'..='z').chain('A'..='Z').chain('0'..='9').collect();
    all.extend(['é', '😀', '\\', '*', '_', '[', ']', '(', ')', '<', '>', '#', '-', '+', '.', '|', '~', '&', ' ', '•', '─']);
    proptest::sample::select(all)
}

/// The look Markdown carries, per character.
fn markdown_look(s: &Style) -> (bool, bool, bool, bool, Option<String>, Option<String>) {
    (s.bold, s.italic, s.strike, s.mono, s.link.clone(), s.picture.clone())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Text and the styles Markdown can carry come back character by character, with the same
    /// lists, titles *[sic]* and pictures.
    #[test]
    fn every_text_markdown_can_carry_comes_back(lines in prop::collection::vec((0u8..5, prop::collection::vec(any_char(), 0..24)), 1..5)) {
        // A block (a list or a title) is written with the blank line the editor puts around it;
        // text without blank lines around blocks is covered by the exact tests above.
        let mut prepared: Vec<(u8, Vec<char>)> = Vec::new();
        let (mut prev_nonblank, mut after_block) = (false, false);
        for (kind, chars) in &lines {
            let blank_line = *kind == 5;
            let block = matches!(kind, 1 | 2 | 4);
            if !blank_line && ((block && prev_nonblank) || after_block) {
                prepared.push((5, Vec::new()));
            }
            prepared.push((*kind, chars.clone()));
            prev_nonblank = !blank_line && *kind != 3;
            after_block = block;
        }
        let mut text = String::new();
        let mut styles = Styles::plain("");
        styles.chars.clear();
        styles.paras.clear();
        for (n, (kind, chars)) in prepared.iter().enumerate() {
            if n > 0 {
                text.push('\n');
                styles.chars.push(0);
            }
            let mut chars = chars.clone();
            while chars.first() == Some(&' ') { chars.remove(0); }
            while chars.last() == Some(&' ') { chars.pop(); }
            // A run of spaces collapses where HTML is read; one space is exact.
            chars.dedup_by(|a, b| *a == ' ' && *b == ' ');
            // A block line that is empty would be a case of its own; the exact test above
            // covers an empty bullet, and the round trip here keeps to text.
            if !matches!(kind, 3 | 5) && chars.is_empty() {
                chars.push('a');
            }
            let heading = match kind {
                4 => match n % 3 { 0 => Some(20), 1 => Some(16), _ => Some(14) },
                _ => None,
            };
            let mut para = Para::default();
            if *kind == 5 {
                styles.paras.push(para);
                continue;
            }
            match kind {
                1 | 2 if heading.is_none() => {
                    para.list = if *kind == 1 { List::Bullet } else { List::Number };
                }
                3 => para.rule = true,
                _ => {}
            }
            for (i, ch) in chars.iter().enumerate() {
                if *kind == 3 { break; }
                let link = if heading.is_none() && (i + n) % 7 == 0 { Some(format!("https://studio.example/{n}/{i}")) } else { None };
                let mono = heading.is_none() && (i + n) % 5 == 0;
                let bold = heading.is_some() || (heading.is_none() && (i + n) % 3 == 0);
                let italic = heading.is_none() && (i + n) % 4 == 0;
                let strike = heading.is_none() && (i + n) % 6 == 0;
                let picture = if heading.is_none() && (i + n) % 11 == 0 && !mono {
                    Some(if n % 2 == 0 { "logo@studio.example".to_string() } else { "plan@studio.example".to_string() })
                } else {
                    None
                };
                let st = if let Some(cid) = &picture {
                    Style { picture: Some(cid.clone()), ..Style::default() }
                } else {
                    Style { bold, italic, strike, mono, link, size: heading, ..Style::default() }
                };
                text.push(if picture.is_some() { PICTURE } else { *ch });
                let id = styles.intern(st);
                styles.chars.push(id);
            }
            styles.paras.push(para);
        }
        styles.fit(&text);
        let (back, back_styles) = there_and_back(&text, &styles);
        prop_assert_eq!(&back, &text, "sent {:?}", outbound(&text, &styles, 7).0);
        for (i, ch) in text.chars().enumerate() {
            // Markdown can't mark only a white space: its own look isn't carried.
            if ch.is_whitespace() {
                continue;
            }
            let a = markdown_look(&styles.style(i));
            let b = markdown_look(&back_styles.style(i));
            prop_assert_eq!((ch, b), (ch, a), "character {} at {}", ch, i);
        }
        for n in 0..text.split('\n').count() {
            let a = styles.para(n);
            let b = back_styles.para(n);
            prop_assert_eq!((b.list, b.indent, b.rule), (a.list, a.indent, a.rule), "paragraph {}", n);
        }
    }
}

/// Quatrelle's own marks and a bare picture in the model's answer are not read as ours.
#[test]
fn an_answer_cannot_forge_an_internal_mark_or_a_picture() {
    let answer = "Salut\u{E002}\u{E041}\u{E06F}\u{FFFC}";
    let (back, _) = inbound(answer, Style::default(), &[], "Salut", &Styles::plain("Salut"));
    assert_eq!(back, "Salut");
    assert!(!back.chars().any(|c| c == PICTURE || ('\u{E000}'..='\u{E06F}').contains(&c)), "{back:?}");
}

/// An answer wrapped whole in a fenced code block is the text inside it.
#[test]
fn a_whole_answer_in_one_fence_loses_the_fence() {
    let (back, styles) = inbound("```markdown\n**Salut** Zoé\n```", Style::default(), &[], "Salut Zoé", &Styles::plain("Salut Zoé"));
    assert_eq!(back, "Salut Zoé");
    assert!(styles.style(0).bold, "the text inside keeps its marks");
}

/// A bare web address in ordinary text comes back as a link, as pasted text would.
#[test]
fn a_bare_web_address_comes_back_as_a_link() {
    let text = "Voir https://studio.example/plan pour le détail";
    let styles = Styles::plain(text);
    let (back, back_styles) = there_and_back(text, &styles);
    assert_eq!(back, text);
    let at = back.find("https://studio.example/plan").unwrap_or(0);
    let at = back.get(..at).map_or(0, |t| t.chars().count());
    assert_eq!(back_styles.style(at).link.as_deref(), Some("https://studio.example/plan"));
}

/// A title whose text is a single hash comes back as that hash, still a title.
#[test]
fn a_title_whose_text_is_a_hash_comes_back() {
    let text = "#";
    let mut styles = Styles::plain(text);
    styles.restyle(text, 0..1, |s| Style { bold: true, size: Some(20), ..s.clone() });
    let (back, back_styles) = there_and_back(text, &styles);
    assert_eq!(back, text, "sent {:?}", outbound(text, &styles, 7).0);
    assert!(back_styles.style(0).bold && back_styles.style(0).size == Some(20), "still a title");
}

/// A line made of one Markdown sign, as a paragraph, a list item or a title, comes back as that
/// sign with its layout.
#[test]
fn a_line_that_is_one_markdown_sign_comes_back() {
    fn line(text: &str, heading: bool, list: List) -> (Styles, Para) {
        let mut styles = Styles::plain(text);
        let para = Para { list, ..Para::default() };
        styles.paras[0] = para;
        if heading {
            styles.restyle(text, 0..text.chars().count(), |s| Style { bold: true, size: Some(20), ..s.clone() });
        }
        (styles, para)
    }
    for sign in ["#", "-", "+", ">", "1.", "*", "_"] {
        for (heading, list) in [(false, List::None), (false, List::Bullet), (false, List::Number), (true, List::None)] {
            let (styles, para) = line(sign, heading, list);
            let (back, back_styles) = there_and_back(sign, &styles);
            assert_eq!(back, sign, "sign {sign:?}, heading {heading}, list {list:?}: sent {:?}", outbound(sign, &styles, 7).0);
            let now = back_styles.para(0);
            assert_eq!((now.list, now.rule), (para.list, para.rule), "layout of {sign:?}");
            if heading {
                assert!(back_styles.style(0).bold && back_styles.style(0).size == Some(20), "title {sign:?}");
            }
        }
    }
}

/// A title whose text begins like a bullet stays its text, not a bullet.
#[test]
fn a_title_that_begins_like_a_bullet_stays_text() {
    let text = "* a";
    let mut styles = Styles::plain(text);
    styles.restyle(text, 0..text.chars().count(), |s| Style { bold: true, size: Some(20), ..s.clone() });
    let (back, back_styles) = there_and_back(text, &styles);
    assert_eq!(back, text, "sent {:?}", outbound(text, &styles, 7).0);
    assert!(back_styles.style(0).bold && back_styles.style(0).size == Some(20), "still a title");
}

/// A title with a hash at its end keeps it: the hash would close the title otherwise.
#[test]
fn a_title_ending_with_a_hash_keeps_it() {
    let text = "a #";
    let mut styles = Styles::plain(text);
    styles.restyle(text, 0..text.chars().count(), |s| Style { bold: true, size: Some(20), ..s.clone() });
    let (back, back_styles) = there_and_back(text, &styles);
    assert_eq!(back, text, "sent {:?}", outbound(text, &styles, 7).0);
    assert!(back_styles.style(0).bold && back_styles.style(0).size == Some(20), "still a title");
}

/// An empty list item is a list item, not a lost paragraph.
#[test]
fn an_empty_bullet_line_stays_a_bullet() {
    let text = "";
    let mut styles = Styles::plain(text);
    styles.paras[0].list = List::Bullet;
    let sent = outbound(text, &styles, 7).0;
    let (back, back_styles) = there_and_back(text, &styles);
    assert_eq!(back, text, "sent {sent:?} read {:?}", crate::read_markdown(&sent));
    assert_eq!(back_styles.para(0).list, List::Bullet, "paras {:?} read {:?}", back_styles.paras, crate::read_markdown(&sent));
}

/// An ordinary paragraph whose text begins like a bullet (`* a`) stays its text; the reader
/// trusts the line's own mark, not the characters.
#[test]
fn a_paragraph_beginning_like_a_bullet_stays_text() {
    let text = "a\n* a";
    let styles = Styles::plain(text);
    let (back, back_styles) = there_and_back(text, &styles);
    assert_eq!(back, text, "sent {:?}", outbound(text, &styles, 7).0);
    assert_eq!(back_styles.para(1).list, List::None);
}

/// An answer with no text the reader keeps (only blanks, or only an HTML comment) still gives
/// every picture back: the caller decides it is empty, a picture is never lost on the way.
#[test]
fn an_answer_without_readable_text_keeps_its_picture() {
    let text = "Bonjour \u{FFFC} Zoé";
    let mut styles = Styles::plain(text);
    styles.restyle(text, 8..9, |s| Style { picture: Some("logo@studio.example".into()), ..s.clone() });
    let (_, images) = outbound(text, &styles, 7);
    assert_eq!(images.len(), 1);
    for answer in ["<!-- -->tps://studio.example/AgE-7-1-END\n", "   \n", ""] {
        let (back, _) = inbound(answer, Style::default(), &images, text, &styles);
        assert_eq!(back.chars().filter(|c| *c == PICTURE).count(), 1, "answer {answer:?} gave {back:?}");
    }
}

/// A marker taken out of the answer can leave the pieces around it joined into another
/// (`QUATRELLE-I` + a marker + `MAGE-7-…`): that one goes as well, however deep they nest, and
/// the picture still comes back once.
#[test]
fn a_marker_joined_by_taking_another_out_goes_too() {
    let text = "Bonjour \u{FFFC} Zoé";
    let mut styles = Styles::plain(text);
    styles.restyle(text, 8..9, |s| Style { picture: Some("logo@studio.example".into()), ..s.clone() });
    let (_, images) = outbound(text, &styles, 7);
    let sent = images.first().map(|im| im.marker.clone()).unwrap_or_default();
    let mut nested = String::from("QUATRELLE-IMAGE-7-4-END");
    for _ in 0..40 {
        nested = format!("QUATRELLE-I{nested}MAGE-7-");
    }
    for answer in [format!("Hello {sent} 4QUATRELLE-IQUATRELLE-IMAGE-7-4-ENDMAGE-7-\u{3a7}T"), format!("Hello {sent} x{nested}y")] {
        let (back, _) = inbound(&answer, Style::default(), &images, text, &styles);
        assert_eq!(back.chars().filter(|c| *c == PICTURE).count(), 1, "{back:?}");
        assert!(!back.contains("QUATRELLE-IMAGE-7-"), "{back:?}");
    }
}
