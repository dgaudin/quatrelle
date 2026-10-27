//! Tests of the message editor: its layout, keys and formats.

use super::*;

/// A right-click on a red-underlined word finds that word (and not its neighbour), so the
/// menu can offer what could take its place, as Outlook's does.
#[test]
fn the_word_under_the_pointer_is_the_one_offered() {
    // The dictionaries are read here, on the test thread, as the app does in the background.
    assert!(crate::spellcheck::load_for_test(), "the dictionaries are built in");
    crate::spellcheck::set_on(true);
    let text = "je vai partir dans les pyrénés";
    let mut c = Compose { body: text.into(), language: "all".into(), ..Default::default() };
    c.styles = Styles::plain(text);
    let found = misspelt_at(&c, 4).map(|(r, w)| (text.chars().skip(r.start).take(r.end - r.start).collect::<String>(), w));
    assert_eq!(found, Some(("vai".to_string(), "vai".to_string())), "the word clicked");
    // The second one, and none on the words that are spelt right or between two of them.
    assert!(misspelt_at(&c, text.chars().count() - 2).is_some_and(|(_, w)| w == "pyrénés"));
    assert_eq!(misspelt_at(&c, 0), None, "je");
    assert_eq!(misspelt_at(&c, 2), None, "the space after it");
    assert_eq!(misspelt_at(&c, 1000), None, "past the end");
    // A word with an accent before it: the range is in characters, not bytes.
    c.body = "déja partir".into();
    c.styles = Styles::plain(&c.body);
    assert!(misspelt_at(&c, 2).is_some_and(|(r, w)| w == "déja" && r == (0..4)), "characters, not bytes");
}

/// Neither the signature nor the original under a reply is checked, nor a word let be in
/// the message; the subject is checked without those stretches.
#[test]
fn the_signature_and_the_original_are_left_alone() {
    assert!(crate::spellcheck::load_for_test(), "the dictionaries are built in");
    let text = "Bonjour servveur\n\n-- \nDidierr\n\n\nOriginall quoted";
    let mut c =
        Compose { body: text.into(), subject: "Objett".into(), signature: "-- \nDidierr".into(), language: "fr".into(), ..Default::default() };
    c.styles = Styles::plain(text);
    c.styles.relayout(text, 24..25, |p| p.rule = true);
    let words = |c: &Compose| misspelt_in(c, false).into_iter().map(|(_, w)| w).collect::<Vec<_>>();
    assert_eq!(words(&c), ["servveur"], "{:?}", check_for(&c));
    assert_eq!(misspelt_in(&c, true).into_iter().map(|(_, w)| w).collect::<Vec<_>>(), ["Objett"]);
    crate::spellcheck::ignore_all(c.id, "servveur");
    assert!(words(&c).is_empty());
    // The message's language: French unless chosen, English words wrong in it alone.
    c.body = "Le backup".into();
    c.styles = Styles::plain(&c.body);
    assert_eq!(effective_language(&c).as_deref(), Some("fr"));
    assert_eq!(words(&c), ["backup"]);
    c.language = "all".into();
    assert!(words(&c).is_empty());
    assert_eq!(language_name("fr"), "Français");
    assert_eq!(language_name("xx"), "XX");
}

/// A horizontal line goes on a paragraph of its own at the cursor, a typeface is set on the
/// selection (and sent with the message), and Undo takes each back.
#[test]
fn rules_and_typefaces_go_in_and_come_out() {
    let mut c = Compose { body: "one two".into(), ..Default::default() };
    c.styles = Styles::plain(&c.body);
    let mut ed = Editor::new(c.id);
    ed.sel = 3..3;
    apply(&mut ed, &mut c, Op::Rule);
    assert_eq!(c.body, "one\n\n two");
    assert!(c.styles.para(1).rule && !c.styles.para(0).rule && !c.styles.para(2).rule, "{:?}", c.styles.paras);
    assert_eq!(ed.sel, 5..5, "the cursor after the line");
    assert!(rich::to_html(&c.body, &c.styles).contains("<hr"));
    ed.sel = 0..3;
    apply(&mut ed, &mut c, Op::Font(Some("Arial".into())));
    assert_eq!(c.styles.style(0).font.as_deref(), Some("Arial"));
    assert!(rich::to_html(&c.body, &c.styles).contains("font-family:'Arial',sans-serif"));
    apply(&mut ed, &mut c, Op::Font(None));
    assert_eq!(c.styles.style(0).font, None);
    let ctx = egui::Context::default();
    for _ in 0..3 {
        undo(&mut ed, &mut c, &ctx, false);
    }
    assert_eq!(c.body, "one two");
}

/// Every row of a centred or right-aligned paragraph is moved over, not the first alone.
#[test]
fn long_aligned_paragraphs_are_aligned_whole() {
    let text = "one two three four five six seven eight nine ten eleven twelve\nleft";
    let mut st = Styles::plain(text);
    st.relayout(text, 0..1, |p| p.align = Align::Center);
    let ctx = egui::Context::default();
    let tokens = theme::Tokens::light();
    let mut out = ctx.run_ui(Default::default(), |ui| {
        let g = layout_aligned(ui, text, &st, false, &tokens, 120.0, &Check::default());
        let centred: Vec<_> = g.rows.iter().take_while(|r| !r.ends_with_newline).collect();
        assert!(centred.len() >= 3, "{} rows", centred.len());
        // Each row sits in the middle of what's left (a row that fills the width stays).
        for r in &centred {
            let room = (120.0 - r.rect().width()).max(0.0);
            assert!((r.pos.x - room / 2.0).abs() < 0.5, "{:?} in {room}", r.rect());
        }
        assert!(centred.iter().any(|r| r.pos.x > 1.0), "the last row, shorter, is moved over");
        assert!(g.rows.last().is_some_and(|r| r.pos.x == 0.0), "the left-aligned one stays");
        // Right-aligned: against the edge, each row.
        st.relayout(text, 0..1, |p| p.align = Align::Right);
        let g = layout_aligned(ui, text, &st, false, &tokens, 120.0, &Check::default());
        for r in g.rows.iter().take_while(|r| !r.ends_with_newline) {
            assert!((r.rect().max.x - 120.0).abs() < 1.0, "{:?}", r.rect());
        }
    });
    out.textures_delta.clear();
}

#[test]
fn link_addresses() {
    let web = |a: &str| LinkForm { address: a.into(), ..Default::default() }.url();
    assert_eq!(web("example.com").as_deref(), Some("https://example.com"));
    assert_eq!(web("http://x.example/a").as_deref(), Some("http://x.example/a"));
    assert_eq!(web("C:/Users/me/plan.docx").as_deref(), Some("file:///C:/Users/me/plan.docx"));
    assert_eq!(web("https://"), None);
    assert_eq!(web("  "), None);
    let mail = LinkForm { email: true, to: "zoe@studio.example".into(), subject: "Poster & sizes".into(), ..Default::default() };
    assert_eq!(mail.url().as_deref(), Some("mailto:zoe@studio.example?subject=Poster%20%26%20sizes"));
    assert_eq!(LinkForm { email: true, ..Default::default() }.url(), None);
    // An existing mailto link opens as an email address with its subject.
    let mut c = Compose { body: "write".into(), ..Default::default() };
    c.styles = Styles::plain(&c.body);
    c.styles.restyle(&c.body, 0..5, |s| Style { link: Some("mailto:ann@x.example?subject=Hi%20there".into()), ..s.clone() });
    let f = LinkForm::open(&c, 0..5);
    assert!(f.email && f.editing && !f.multi);
    assert_eq!((f.to.as_str(), f.subject.as_str(), f.text.as_str()), ("ann@x.example", "Hi there", "write"));
}

/// Right-to-left paragraphs in a message start against the editor's right edge, a short one
/// and every row of a wrapped one, in rich and plain text; a left-to-right one stays left. (egui
/// put them against the widest line, so a short Hebrew line sat in the middle.)
#[test]
fn right_to_left_paragraphs_start_on_the_right() {
    let text = "שלום עולם
English stays left.
זוהי פסקה ארוכה מאוד בעברית שתיכנס לכמה שורות כשהעורך צר, וכל שורה שלה צריכה להתחיל בצד ימין של העורך.";
    let wrap = 300.0;
    let ctx = egui::Context::default();
    let t = Tokens::light();
    for plain in [false, true] {
        let mut rows = Vec::new();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let g = layout::layout(ui, text, &Styles::plain(text), plain, &t, wrap, &Check::default());
            rows = g
                .rows
                .iter()
                .map(|r| {
                    // Where its letters are (a row's rect takes in its leading space).
                    let ink =
                        |f: fn(f32, f32) -> f32, init: f32| r.row.glyphs.iter().filter(|g| !g.chr.is_whitespace()).map(|g| g.pos.x).fold(init, f);
                    let right = r.row.glyphs.iter().map(|g| g.max_x()).fold(0.0, f32::max);
                    (r.pos.x + ink(f32::min, f32::INFINITY), r.pos.x + right, r.ends_with_newline)
                })
                .collect();
        });
        out.textures_delta.clear();
        assert!(rows.len() > 3, "the long paragraph wraps: {rows:?}");
        // Row 0: the short Hebrew line; row 1: English; then the wrapped Hebrew paragraph.
        let (min0, max0, _) = rows[0];
        assert!((wrap - max0).abs() < 2.0 && min0 > 100.0, "plain={plain}: the short line against the right edge: {rows:?}");
        assert!(rows[1].0 < 2.0, "plain={plain}: English on the left: {rows:?}");
        for r in &rows[2..] {
            assert!((wrap - r.1).abs() < 2.0, "plain={plain}: each wrapped row against the right edge: {rows:?}");
        }
    }
}

/// A picture's room starts where its place in the text does, and is as wide at the start of
/// a line as after text (letter spacing put it some 60 points right of its place after text,
/// and none at a line's start, squeezing the picture into 36).
#[test]
fn a_picture_sits_where_it_is_in_the_text() {
    let ctx = egui::Context::default();
    let t = Tokens::light();
    for (body, at) in [("\u{FFFC}", 0), ("ab\u{FFFC}cd", 2), ("x\n\u{FFFC}\u{FFFC}", 2), ("x\n\u{FFFC}\u{FFFC}", 3)] {
        let mut st = Styles::plain(body);
        for (i, ch) in body.chars().enumerate() {
            if ch == PICTURE {
                st.restyle(body, i..i + 1, |s| Style { picture: Some("cid".into()), ..s.clone() });
            }
        }
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let g = layout::layout(ui, body, &st, false, &t, 600.0, &Check::default());
            let (a, b) = (g.pos_from_cursor(CCursor::new(at)), g.pos_from_cursor(CCursor::new(at + 1)));
            let mut glyphs = g.rows.iter().flat_map(|r| r.row.glyphs.iter().map(move |gl| (r.pos.x + gl.pos.x, r.rect().center().y, gl.chr)));
            let here = glyphs.any(|(x, y, c)| c == PICTURE && (x - a.min.x).abs() < 1.0 && (y - a.center().y).abs() < 40.0);
            assert!(here, "{body:?}: the picture's character where its room starts");
            assert!(b.min.x - a.min.x >= PICTURE_PX * 1.2, "{body:?} at {at}: a room {} wide", b.min.x - a.min.x);
        });
        out.textures_delta.clear();
    }
}
