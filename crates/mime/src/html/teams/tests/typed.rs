// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Mentions written as Graph documents them, and a message read back into what the box holds.

#![allow(clippy::single_range_in_vec_init)] // lists of one range

use super::*;
use quatrelle_model::pim::teams::{MAX_ID, MAX_MENTIONS, Mention};

/// A `Typed` of `text` with `mentions` as `(range, user)`.
fn typed(text: &str, mentions: &[(std::ops::Range<usize>, &str)]) -> Typed {
    Typed { text: text.into(), mentions: mentions.iter().map(|(r, u)| (r.clone(), (*u).into())).collect(), ..Typed::default() }
}

/// The texts of the mentions composed from `t`.
fn mentioned(t: &Typed) -> Vec<String> {
    teams_compose_typed(t).1.into_iter().map(|m| m.text).collect()
}

/// A mention entry.
fn entry(id: u32, text: &str, user: Option<&str>) -> Mention {
    Mention { id, text: text.into(), user_id: user.map(Into::into) }
}

#[test]
fn a_mention_is_written_as_graph_documents_it() {
    let typed = Typed { text: "hi Ana Exemple!".into(), mentions: vec![(3..11, "u-ana".into())], ..Typed::default() };
    let (t, m) = teams_compose_typed(&typed);
    assert_eq!(m, vec![Mention { id: 0, text: "Ana Exemple".into(), user_id: Some("u-ana".into()) }]);
    let html = teams_write_mentions(&t, &m).unwrap();
    assert_eq!(html, "<p>hi <at id=\"0\">Ana Exemple</at>!</p>");
    assert_eq!(teams_foreign_markup(&html), None);
    assert_eq!(teams_html(&html, C, "1").blocks[0].runs[1].mention, Some(0));
    assert_eq!(teams_write(&t).unwrap(), "<p>hi Ana Exemple!</p>", "without the mentions, its text alone");
}

#[test]
fn a_name_with_markup_is_escaped_in_its_mention() {
    let name = "Ana <img src=x> & \"Lima\"";
    let (t, m) = teams_compose_typed(&Typed { text: name.into(), mentions: vec![(0..name.len(), "u-ana".into())], ..Typed::default() });
    let html = teams_write_mentions(&t, &m).unwrap();
    assert_eq!(html, "<p><at id=\"0\">Ana &lt;img src=x&gt; &amp; &quot;Lima&quot;</at></p>");
    assert_eq!(teams_foreign_markup(&html), None);
}

#[test]
fn mention_ranges_that_cant_be_are_plain_text() {
    // Off a character's boundary, off the text, empty, holding a line break.
    assert!(mentioned(&typed("éa", &[(0..1, "u")])).is_empty());
    assert!(mentioned(&typed("éa", &[(0..99, "u")])).is_empty());
    assert!(mentioned(&typed("ab", &[(2..2, "u")])).is_empty());
    assert!(mentioned(&typed("a\nb", &[(0..3, "u")])).is_empty());
    assert!(mentioned(&typed("a\u{7}b", &[(0..3, "u")])).is_empty());
    // The second of two that overlap is dropped, the first kept.
    assert_eq!(mentioned(&typed("Ana Exemple", &[(0..3, "u-1"), (2..8, "u-2")])), ["Ana"]);
    assert_eq!(mentioned(&typed("Ana Exemple", &[(4..8, "u-2"), (0..3, "u-1")])), ["Ana", "Lima"], "in the text's order");
    // A user id: empty, too long, with a control character.
    assert!(mentioned(&typed("Ana", &[(0..3, "")])).is_empty());
    assert!(mentioned(&typed("Ana", &[(0..3, &"x".repeat(MAX_ID + 1))])).is_empty());
    assert_eq!(mentioned(&typed("Ana", &[(0..3, &"x".repeat(MAX_ID))])), ["Ana"]);
    assert!(mentioned(&typed("Ana", &[(0..3, "u\u{7}")])).is_empty());
    // A text that isn't one line as it stands: no mention, the text whole.
    let (t, m) = teams_compose_typed(&typed("Ana  Lima", &[(0..9, "u")]));
    assert!(m.is_empty());
    assert_eq!(t.plain(), "Ana  Lima");
    assert!(runs(&t).iter().all(|r| r.mention.is_none()));
    // Past the most.
    let words: Vec<String> = (0..=MAX_MENTIONS).map(|n| format!("w{}", "x".repeat(n % 7))).collect();
    let text = words.join(" ");
    let mut at = 0;
    let all: Vec<(std::ops::Range<usize>, String)> = words
        .iter()
        .map(|w| {
            let r = at..at + w.len();
            at += w.len() + 1;
            (r, "u".to_string())
        })
        .collect();
    assert_eq!(all.len(), MAX_MENTIONS + 1);
    let (t, m) = teams_compose_typed(&Typed { text: text.clone(), mentions: all, ..Typed::default() });
    assert_eq!(m.len(), MAX_MENTIONS);
    assert_eq!(m.last().map(|m| m.id), Some(MAX_MENTIONS as u32 - 1));
    assert_eq!(t.plain(), text, "the last one stays in the text, as plain words");
    assert_eq!(runs(&t).iter().filter(|r| r.mention.is_some()).count(), MAX_MENTIONS);
}

#[test]
fn a_bold_mention_is_one_run_in_its_first_characters_look() {
    let t = Typed { text: "Ana Exemple".into(), bold: vec![0..3], mentions: vec![(0..8, "u-ana".into())], ..Typed::default() };
    let (c, m) = teams_compose_typed(&t);
    assert_eq!(m.len(), 1);
    assert_eq!(c.blocks.len(), 1);
    assert_eq!(c.blocks[0].runs, [Run { text: "Ana Exemple".into(), bold: true, mention: Some(0), ..Run::default() }]);
    assert_eq!(teams_write_mentions(&c, &m).unwrap(), "<p><b><at id=\"0\">Ana Exemple</at></b></p>");
    // The first character not bold: the whole mention isn't, whatever follows.
    let t = Typed { text: "Ana Exemple".into(), bold: vec![1..8], italic: vec![0..1], mentions: vec![(0..8, "u-ana".into())] };
    let (c, _) = teams_compose_typed(&t);
    assert_eq!(c.blocks[0].runs, [Run { text: "Ana Exemple".into(), italic: true, mention: Some(0), ..Run::default() }]);
}

#[test]
fn a_mention_without_its_entry_is_written_as_its_text() {
    let t = message(vec![Run { text: "Ana".into(), mention: Some(3), ..Run::default() }]);
    assert_eq!(teams_write_mentions(&t, &[]).unwrap(), "<p>Ana</p>");
    assert_eq!(teams_write_mentions(&t, &[entry(0, "Ana", Some("u"))]).unwrap(), "<p>Ana</p>");
    assert_eq!(teams_write_mentions(&t, &[entry(3, "Ana", None)]).unwrap(), "<p>Ana</p>");
    assert_eq!(teams_write_mentions(&t, &[entry(3, "Ana", Some("u"))]).unwrap(), "<p><at id=\"3\">Ana</at></p>");
    // Never in a link.
    let linked = message(vec![Run { text: "Ana".into(), mention: Some(3), link: Some("https://x.example/".into()), ..Run::default() }]);
    assert_eq!(teams_write_mentions(&linked, &[entry(3, "Ana", Some("u"))]).unwrap(), "<p><at id=\"3\">Ana</at></p>");
}

#[test]
fn a_message_the_box_can_hold_comes_back_typed() {
    let box_holds = Typed { text: "one two\n\nAna".into(), bold: vec![4..7], italic: vec![0..3], mentions: vec![(9..12, "u-ana".into())] };
    let c = teams_compose_typed(&box_holds);
    assert_eq!(c.1.len(), 1);
    assert_eq!(teams_typed(&c.0, &c.1), Some(box_holds));
}

#[test]
fn a_message_the_box_cant_hold_whole_isnt_typed() {
    let one = |run: Run| teams_typed(&message(vec![run]), &[entry(0, "Ana", Some("u-ana")), entry(1, "Bo", None)]);
    assert!(one(plain("fine")).is_some());
    let quote = ChatText { blocks: vec![Block { kind: BlockKind::Quote, runs: vec![plain("q")] }] };
    assert_eq!(teams_typed(&quote, &[]), None);
    let item = ChatText { blocks: vec![Block { kind: BlockKind::Item { ordered: false, depth: 1 }, runs: vec![plain("i")] }] };
    assert_eq!(teams_typed(&item, &[]), None);
    let code_block = ChatText { blocks: vec![Block { kind: BlockKind::Code, runs: vec![plain("c")] }] };
    assert_eq!(teams_typed(&code_block, &[]), None);
    assert_eq!(one(Run { code: true, ..plain("c") }), None);
    assert_eq!(one(Run { underline: true, ..plain("u") }), None);
    assert_eq!(one(Run { strike: true, ..plain("s") }), None);
    assert_eq!(one(Run { hosted: Some("aGk=".into()), ..Run::default() }), None);
    assert_eq!(one(Run { outside: Some("https://e.example/a.png".into()), ..Run::default() }), None);
    assert_eq!(one(Run { attachment: Some("a1".into()), ..plain("f") }), None);
    assert_eq!(one(Run { link: Some("https://x.example/p".into()), ..plain("the plan") }), None);
    // A mention not in the list, or without a user.
    assert_eq!(one(Run { mention: Some(7), ..plain("Ana") }), None);
    assert_eq!(one(Run { mention: Some(1), ..plain("Bo") }), None);
    assert!(one(Run { mention: Some(0), ..plain("Ana") }).is_some());
    // A link whose text is its address is its text.
    let t = one(Run { link: Some("https://x.example/p".into()), ..plain("https://x.example/p") });
    assert_eq!(t.map(|t| t.text), Some("https://x.example/p".to_string()));
}

#[test]
fn blocks_are_lines_and_runs_keep_their_breaks_and_looks() {
    let t = ChatText {
        blocks: vec![
            Block {
                kind: BlockKind::Paragraph,
                runs: vec![plain("a"), Run { bold: true, ..plain("b\nc") }, Run { bold: true, italic: true, ..plain("d") }],
            },
            Block::default(),
            Block { kind: BlockKind::Paragraph, runs: vec![Run { italic: true, ..plain("é") }] },
        ],
    };
    let typed = teams_typed(&t, &[]).unwrap();
    assert_eq!(typed.text, "ab\ncd\n\né");
    assert_eq!(typed.bold, [1..5]);
    assert_eq!(typed.italic, [4..5, 7..9]);
    assert_eq!(teams_typed(&ChatText::default(), &[]), Some(Typed::default()));
}

/// A word: letters, the marks HTML escapes, an accented letter, an emoji.
fn word() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[a-z<>&\"'é😀]{1,6}").unwrap()
}

/// A text of 1 to 5 lines of 1 to 8 words, with bold and italic over whole words, and up to three
/// mentions each one whole word.
fn any_typed() -> impl Strategy<Value = Typed> {
    let line = proptest::collection::vec((word(), 0..4u8, 0..3u8), 1..=8);
    (proptest::collection::vec(line, 1..=5), proptest::collection::vec(0..40usize, 0..=3)).prop_map(|(lines, picks)| {
        let mut t = Typed::default();
        let mut words = Vec::new();
        for (n, line) in lines.iter().enumerate() {
            if n > 0 {
                t.text.push('\n');
            }
            for (i, (w, look, _)) in line.iter().enumerate() {
                if i > 0 {
                    t.text.push(' ');
                }
                let r = t.text.len()..t.text.len() + w.len();
                t.text.push_str(w);
                if look & 1 == 1 {
                    t.bold.push(r.clone());
                }
                if look & 2 == 2 {
                    t.italic.push(r.clone());
                }
                words.push(r);
            }
        }
        let mut chosen: Vec<usize> = picks.iter().map(|p| p % words.len()).collect();
        chosen.sort_unstable();
        chosen.dedup();
        t.mentions = chosen.iter().enumerate().map(|(n, w)| (words[*w].clone(), format!("u-{}", n + 1))).collect();
        t
    })
}

proptest::proptest! {
    #[test]
    fn typed_text_composes_back_the_same(typed in any_typed()) {
        let c = teams_compose_typed(&typed);
        let back = teams_typed(&c.0, &c.1);
        proptest::prop_assert!(back.is_some(), "{:?}", c);
        proptest::prop_assert_eq!(back.map(|t| teams_compose_typed(&t)), Some(c.clone()));
        let html = teams_write_mentions(&c.0, &c.1).map_err(|e| proptest::test_runner::TestCaseError::fail(format!("{e:?}")))?;
        proptest::prop_assert_eq!(teams_foreign_markup(&html), None, "{}", html);
        let read = teams_html(&html, C, "1");
        let got: Vec<(Option<u32>, &str)> = runs(&read).into_iter().filter(|r| r.mention.is_some()).map(|r| (r.mention, r.text.as_str())).collect();
        let want: Vec<(Option<u32>, &str)> = c.1.iter().map(|m| (Some(m.id), m.text.as_str())).collect();
        proptest::prop_assert_eq!(got, want, "{}", html);
    }
}
