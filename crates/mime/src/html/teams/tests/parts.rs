// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The places of pictures and files in a message written: after the text, in the writer's own
//! markup, a file's id escaped, the whole within the cap.

use super::*;
use quatrelle_model::pim::teams::Mention;

const G: &str = "00000000-0000-0000-0000-00000000000a";

#[test]
fn pictures_and_files_are_written_after_the_text() {
    let t = teams_compose("Here", &[], &[]);
    let html = teams_write_parts(&t, &[], 2, &[G]).unwrap();
    assert_eq!(
        html,
        "<p>Here</p><p><img src=\"../hostedContents/1/$value\"></p><p><img src=\"../hostedContents/2/$value\"></p>\
         <attachment id=\"00000000-0000-0000-0000-00000000000a\"></attachment>"
    );
    assert_eq!(teams_foreign_markup(&html), None);
}

#[test]
fn a_message_of_files_alone_is_only_their_places() {
    let html = teams_write_parts(&ChatText::default(), &[], 0, &[G]).unwrap();
    assert_eq!(html, "<attachment id=\"00000000-0000-0000-0000-00000000000a\"></attachment>");
    assert_eq!(teams_foreign_markup(&html), None);
}

#[test]
fn without_parts_nothing_changes() {
    let t = teams_compose("Hi Ann", &[], &[]);
    let mentions = [Mention { id: 0, user_id: Some("u-1".into()), text: "Ann".into() }];
    assert_eq!(teams_write_parts(&t, &mentions, 0, &[]), teams_write_mentions(&t, &mentions));
}

#[test]
fn an_id_with_markup_is_escaped() {
    let html = teams_write_parts(&ChatText::default(), &[], 0, &["\"><script>"]).unwrap();
    assert!(html.contains("&quot;&gt;&lt;script&gt;"), "{html}");
    assert!(!html.contains("<script"), "{html}");
}

#[test]
fn the_cap_counts_the_places() {
    // `<p>` and `</p>` are 7 bytes: the text's HTML is SEND_MAX - 10 bytes.
    let t = teams_compose(&"a".repeat(SEND_MAX - 17), &[], &[]);
    assert_eq!(teams_write(&t).unwrap().len(), SEND_MAX - 10);
    assert!(matches!(teams_write_parts(&t, &[], 0, &[G]), Err(TooLong { .. })));
    assert!(teams_write_parts(&t, &[], 0, &[]).is_ok());
}

#[test]
fn places_arent_foreign_but_their_look_alikes_are() {
    for html in [
        "<img src=\"https://evil.example/x\">",
        "<img src=\"../hostedContents/1/$value\" onerror=\"x\">",
        "<img src=\"../hostedContents/123/$value\">",
        "<attachment id=\"x\"></attachment>",
        "<attachment id='00000000-0000-0000-0000-00000000000a'></attachment>",
    ] {
        assert!(teams_foreign_markup(html).is_some(), "{html}");
    }
}

#[test]
fn at_most_the_cap_of_places_is_written() {
    use quatrelle_model::pim::teams::PARTS_MAX;
    let ids = vec![G; PARTS_MAX + 3];
    let html = teams_write_parts(&ChatText::default(), &[], PARTS_MAX + 5, &ids).unwrap();
    assert_eq!(html.matches("<img").count(), PARTS_MAX);
    assert_eq!(html.matches("<attachment").count(), PARTS_MAX);
    assert_eq!(teams_foreign_markup(&html), None);
}
