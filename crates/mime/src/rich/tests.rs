//! The rich text: its runs and paragraphs, to HTML and back, as the editor keeps them.

use super::*;

fn bold() -> Style {
    Style { bold: true, ..Default::default() }
}

#[test]
fn typing_follows_the_text() {
    let mut s = Styles::plain("Hello world");
    s.restyle("Hello world", 0..5, |st| Style { bold: true, ..st.clone() });
    // Typed inside the bold word: bold; deleted across: gone.
    s.edit("Hello world", "Helllo world", &bold());
    assert_eq!(s.chars.len(), 12);
    assert!(s.style(3).bold && !s.style(7).bold);
    s.edit("Helllo world", "Hd", &Style::default());
    assert_eq!((s.chars.len(), s.style(0).bold, s.style(1).bold), (2, true, false));
    // A new paragraph takes the layout of the one it split.
    let mut s = Styles::plain("one\ntwo");
    s.relayout("one\ntwo", 0..1, |p| p.list = List::Bullet);
    // Enter at the end of "one", then "new": typed where the cursor is.
    s.edit_at("one\ntwo", "one\nnew\ntwo", &Style::default(), Some(7));
    assert_eq!(s.paras.iter().map(|p| p.list).collect::<Vec<_>>(), [List::Bullet, List::Bullet, List::None]);
    s.edit("one\nnew\ntwo", "one two", &Style::default());
    assert_eq!(s.paras.len(), 1);
    // A cursor past the end of the text (egui's, after Arabic text) is not trusted.
    for cursor in [9, 40, usize::MAX] {
        let mut s = Styles::plain("ab");
        s.edit_at("ab", "abcd", &Style::default(), Some(cursor));
        assert_eq!(s.chars.len(), 4, "cursor {cursor}");
    }
    // Junk lengths are fixed rather than trusted.
    let mut j = Styles { chars: vec![9, 9, 9, 9, 9, 9], palette: vec![], paras: vec![] };
    j.fit("ab");
    assert_eq!((j.chars.as_slice(), j.paras.len()), (&[0u16, 0][..], 1));
}

#[test]
fn html_and_text() {
    let text = "Hi team\nfirst\nsecond\n\nsee site\nx\u{FFFC}";
    let mut s = Styles::plain(text);
    s.restyle(text, 0..2, |st| Style { bold: true, color: Some([200, 0, 0]), ..st.clone() });
    s.relayout(text, 8..10, |p| p.list = List::Number);
    s.relayout(text, 14..16, |p| p.list = List::Number);
    s.restyle(text, 26..30, |st| Style { link: Some("https://a.example/".into()), ..st.clone() });
    s.relayout(text, 22..23, |p| p.align = Align::Center);
    s.restyle(text, 32..33, |st| Style { picture: Some("pic@x".into()), ..st.clone() });
    s.restyle(text, 3..7, |st| Style { size: Some(18), highlight: Some([255, 255, 0]), italic: true, underline: true, ..st.clone() });
    let html = to_html(text, &s);
    assert!(html.contains("<span style=\"color:#c80000\"><b>Hi</b></span>"), "{html}");
    assert!(html.contains("<span style=\"font-size:18pt; background-color:#ffff00\"><i><u>team</u></i></span>"), "{html}");
    assert!(html.contains("<ol>\n<li>first</li>\n<li>second</li>\n</ol>"), "{html}");
    assert!(
        html.contains("<div style=\"text-align:center\">see <a href=\"https://a.example/\" style=\"text-decoration:underline\">site</a></div>"),
        "{html}"
    );
    assert!(html.contains("<div><br></div>") && html.contains("x<img src=\"cid:pic@x\" style=\"max-width:100%\">"), "{html}");
    // A picture given a width says how wide (as Outlook writes one resized); typing after it
    // doesn't carry that on.
    let mut resized = s.clone();
    resized.restyle(text, 32..33, |st| Style { picture_width: Some(240), ..st.clone() });
    assert!(to_html(text, &resized).contains("x<img src=\"cid:pic@x\" width=\"240\" style=\"max-width:100%\">"));
    assert_eq!(resized.style(32).for_typing().picture_width, None);
    let t = to_text(text, &s);
    assert_eq!(t, "Hi team\n1. first\n2. second\n\nsee site <https://a.example/>\nx[picture]");
    // Runs of spaces, tabs and a space at a paragraph's start reach the recipient (HTML
    // collapses them otherwise); one space between words stays a space.
    let spaced = "  Ref  A\tB\nx y";
    let html = to_html(spaced, &Styles::plain(spaced));
    assert!(html.contains("<div>&nbsp;&nbsp;Ref &nbsp;A&nbsp;&nbsp;&nbsp;&nbsp;B</div>") && html.contains("<div>x y</div>"), "{html}");
    let mut st = Styles::plain(spaced);
    st.restyle(spaced, 2..5, |s| Style { bold: true, ..s.clone() });
    let html = to_html(spaced, &st);
    assert!(html.contains("&nbsp;&nbsp;<b>Ref</b> &nbsp;A"), "across runs too: {html}");
    // Nothing typed can become markup.
    let evil = "<script>x</script>";
    assert!(!to_html(evil, &Styles::plain(evil)).contains("<script>"));
    let mut s = Styles::plain("a");
    s.restyle("a", 0..1, |st| Style { link: Some("javascript:alert(1)".into()), ..st.clone() });
    assert!(!to_html("a", &s).contains("javascript"));
}

/// A link pasted without its underline (Outlook's signatures often have one) keeps its
/// colour and goes out underlined, as the editor shows it.
#[test]
fn a_pasted_link_goes_out_underlined_in_its_colour() {
    let pasted = r#"<p><a href="mailto:dgaudin@proginov.com" style="color:#4B5763;text-decoration:none">dgaudin@proginov.com</a></p>"#;
    let (text, styles) = from_rich(&crate::html::html_to_rich(pasted));
    let html = to_html(&text, &styles);
    assert!(html.contains("<a href=\"mailto:dgaudin@proginov.com\" style=\"text-decoration:underline\">"), "{html}");
    assert!(html.contains("color:#4b5763"), "{html}");
}

#[test]
fn rules_received_formatting_and_appending() {
    let mut text = "Thanks".to_string();
    let mut styles = Styles::plain(&text);
    let (orig, ost) = from_rich(&vec![
        vec![Span { text: "Big".into(), heading: 1, ..Default::default() }],
        vec![Span { text: "plain ".into(), ..Default::default() }, Span { text: "red".into(), color: Some([255, 0, 0]), ..Default::default() }],
    ]);
    assert_eq!(orig, "Big\nplain red");
    assert!(ost.style(0).bold && ost.style(0).size == Some(20) && ost.style(12).color == Some([255, 0, 0]));
    let mut rule = Styles::plain("\n\n");
    rule.paras[1].rule = true;
    append(&mut text, &mut styles, "\n\n", &rule);
    append(&mut text, &mut styles, &orig, &ost);
    assert_eq!(text, "Thanks\n\nBig\nplain red");
    assert!(styles.paras[1].rule && !styles.paras[2].rule);
    assert!(styles.style(8).bold);
    assert!(to_html(&text, &styles).contains("<hr"));
    assert_eq!(to_text(&text, &styles).lines().nth(1), Some(crate::markup::RULE));
    assert_eq!((step_size(11, true), step_size(11, false), step_size(36, true)), (12, 10, 36));
    assert_eq!(lines_of("ab\ncd\nef", 1..4), (0, 1));
    assert!(all("abc", &styles_bold("abc"), 0..3, |s| s.bold) && !all("abc", &styles_bold("abc"), 0..0, |s| s.bold));
}

fn styles_bold(t: &str) -> Styles {
    let mut s = Styles::plain(t);
    s.restyle(t, 0..t.len(), |_| bold());
    s
}
