// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Round trips from the editor's text to Markdown and back, as Rewrite does them.

use super::*;
use crate::rich::Styles;

/// Send `text` with `styles`, then put the model's answer, the same Markdown, back through the
/// reader.
fn there_and_back(text: &str, styles: &Styles) -> (String, Styles) {
    let (sent, images) = outbound(text, styles, 7);
    inbound(&sent, Style::default(), &images, 24_000)
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
    assert!(sent.contains("  \n"), "the writer says where the line ends: {sent:?}");
    let (back, _) = inbound(&sent.replace("  \n", "\n"), Style::default(), &images, 24_000);
    assert_eq!(back, text);
}

/// A paragraph wider than the renderer's old 2000 columns is not cut in two.
#[test]
fn a_long_paragraph_comes_back_on_one_line() {
    let text = vec!["mot"; 750].join(" ");
    let styles = Styles::plain(&text);
    let (sent, images) = outbound(&text, &styles, 7);
    let (back, _) = inbound(&sent, Style::default(), &images, 24_000);
    assert!(!back.contains('\n'), "one line: {:?}", back.chars().take(80).collect::<String>());
    assert_eq!(back, text);
}
