// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: the original under a reply or forward written in HTML, kept as it came and
//! changed where it stands, as Outlook keeps it.

use super::drafts::{MILL, open_mill};
use super::*;

/// The block of text of the quote that holds `words`.
fn cell_with(c: &crate::Compose, words: &str) -> usize {
    let q = c.quote.clone().expect("a quote");
    (0..q.cells()).find(|n| q.cell(*n).is_some_and(|(t, _)| t.contains(words))).unwrap_or_else(|| panic!("{words} in the quote"))
}

/// The owner's report upstream: a reply turned a message laid out in tables into lines of text,
/// its columns gone. Mail written in HTML is quoted as it came: shown from the message, sent as
/// its own HTML (without what runs, nor its tracking picture), its picture with it, its text in
/// the plain twin. Forwarded as an attachment nothing is quoted; a plain message is quoted in
/// the text as before.
#[test]
fn a_reply_keeps_the_original_as_it_came() {
    let mut s = sample_session("quote-as-it-came");
    let key = open_mill(&mut s);
    for cmd in ["mail.reply", "mail.replyAll", "mail.forward"] {
        s.run(cmd, &json!({})).unwrap();
        let c = s.compose.clone().unwrap();
        let q = c.quote.clone().unwrap_or_else(|| panic!("{cmd}: kept as it came"));
        assert_eq!(q.key, key);
        assert!(!q.parsed.layout.is_empty() && q.cells() >= 4, "{cmd}: shown with its columns, each block of text its own");
        // The text holds who sent it to whom and when; the original is after it, not in it.
        assert!(c.body.contains("Subject: Invoice") && !c.body.contains("A warning in red"), "{cmd}: {}", c.body);
        let (text, html, pictures) = c.bodies();
        let html = html.unwrap_or_default();
        assert!(html.contains("<table width=\"640\"") && html.contains("A warning in red"), "{cmd}: {html}");
        assert!(html.contains("color:#ff0000") || html.contains("color: #ff0000"), "its style sheet written into its tags: {html}");
        for gone in ["<script", "alert(1)", "onclick", "steal()", "<style", "tracker.example", "<head"] {
            assert!(!html.contains(gone), "{cmd}: {gone} in {html}");
        }
        assert!(html.contains("src=\"cid:phone@mill.example\"") && html.contains("class=\"quatrelle-quote\""), "{cmd}: {html}");
        assert_eq!(pictures.iter().map(|p| p.cid.as_str()).collect::<Vec<_>>(), ["phone@mill.example"], "{cmd}");
        assert!(text.contains("Subject: Invoice") && text.contains("A warning in red"), "{cmd}: {text}");
        assert!(!text.contains('│') && !text.contains('─'), "{cmd}: {text}");
        // As plain text: the text alone.
        s.run("compose.format", &json!({"plain": true})).unwrap();
        let (text, html, _) = s.compose.as_ref().unwrap().bodies();
        assert!(html.is_none() && text.contains("+00 1234 5678"), "{cmd}: {text}");
        s.run("compose.discard", &json!({})).unwrap();
    }
    // Forwarded as an attachment, nothing is quoted.
    s.run("mail.forwardAsAttachment", &json!({})).unwrap();
    assert!(s.compose.as_ref().unwrap().quote.is_none());
    s.run("compose.discard", &json!({})).unwrap();
    // A plain text message has nothing to keep: quoted in the text, as before.
    let k = key_of(&s, "Sound mix, version 3");
    s.run("list.open", &k).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert!(c.quote.is_none() && c.body.contains("Version 3 of the mix"), "{}", c.body);
    done(s);
}

/// The original is changed where it stands, a block of its text at a time (its columns stay),
/// as Outlook lets it be. Changed, it goes out as its layout written again around the new text,
/// its pictures with it (one put into it too); set back as it was, the block isn't counted.
#[test]
fn the_original_is_changed_where_it_stands() {
    let mut s = sample_session("quote-changed");
    open_mill(&mut s);
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.as_mut().unwrap();
    let n = cell_with(c, "A warning");
    let before = c.fingerprint();
    // Typed into, as the editor does (the block stands in for the text).
    let typed = c.in_quote_cell(n, |c| {
        c.body = c.body.replace("A warning in red", "A warning in red, paid on Friday");
        c.styles.fit(&c.body);
        c.body.clone()
    });
    assert!(typed.is_some_and(|t| t.contains("paid on Friday")));
    assert!(c.body.contains("Subject: Invoice") && !c.body.contains("Friday"), "the text is the message's again: {}", c.body);
    assert_ne!(c.fingerprint(), before, "changed: kept as a draft when another message is read");
    let q = c.quote.clone().unwrap();
    assert_eq!(q.edits.len(), 1, "only the block changed");
    let (t, st) = q.cell(n).unwrap();
    c.set_quote_cell(n, t, st);
    assert_eq!(c.quote.as_ref().unwrap().edits.len(), 1);
    let (text, html, pictures) = c.bodies();
    let html = html.unwrap_or_default();
    assert!(html.contains("<table") && html.contains("width=\"640\"") && html.contains("paid on Friday"), "{html}");
    assert!(html.contains("+00 1234 5678") && html.contains("src=\"cid:phone@mill.example\""), "{html}");
    assert!(!html.contains("<script") && !html.contains("tracker.example"), "{html}");
    assert_eq!(pictures.len(), 1);
    assert!(text.contains("paid on Friday"), "{text}");
    // A picture put into the block where its cursor is.
    let r = s.run("compose.insertPicture", &json!({"name": "stamp.png", "data": crate::sample::sample_png(), "at": 0, "quoteCell": n})).unwrap();
    let cid = r["cid"].as_str().unwrap().to_string();
    let c = s.compose.as_ref().unwrap();
    assert!(!c.body.contains(quatrelle_mime::rich::PICTURE), "not into the text: {}", c.body);
    let (_, html, pictures) = c.bodies();
    assert!(html.unwrap_or_default().contains(&format!("cid:{cid}")));
    assert!(pictures.iter().any(|p| p.cid == cid));
    assert!(s.run("compose.insertPicture", &json!({"name": "x.png", "data": crate::sample::sample_png(), "quoteCell": 999})).is_err());
    done(s);
}

/// A reply's draft opens again with its original as it came, what was changed in it with it, its
/// picture in place; and a reply taken back out of the Outbox the same way (it came back as plain
/// text, its picture as a file).
#[test]
fn the_original_comes_back_with_a_draft_and_from_the_outbox() {
    let mut s = sample_session("quote-again");
    open_mill(&mut s);
    s.run("mail.reply", &json!({})).unwrap();
    s.run("compose.update", &json!({"body": format!("Thanks.{}", s.compose.as_ref().unwrap().body)})).unwrap();
    let c = s.compose.as_mut().unwrap();
    let n = cell_with(c, "A warning");
    c.in_quote_cell(n, |c| {
        c.body = c.body.replace("A warning in red", "Settled");
        c.styles.fit(&c.body);
    });
    let sent_as = c.bodies();
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.compose = None;
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    let key = s.list.iter().find(|m| m.subject == "Re: Invoice").map(|m| m.key.clone()).expect("in Drafts");
    s.run("draft.edit", &json!({"key": key.to_id()})).unwrap();
    let c = s.compose.clone().unwrap();
    let q = c.quote.clone().expect("its original as it came");
    assert!(!q.parsed.layout.is_empty() && (0..q.cells()).any(|n| q.cell(n).is_some_and(|(t, _)| t.contains("Settled"))), "{q:?}");
    assert!(c.body.starts_with("Thanks.") && !c.body.contains("Settled") && c.body.contains("Subject: Invoice"), "{}", c.body);
    assert!(c.attachments.is_empty() && c.inline.iter().any(|p| p.cid == "phone@mill.example"));
    let (_, html, pictures) = c.bodies();
    let html = html.unwrap_or_default();
    assert_eq!(html.matches("class=\"quatrelle-quote\"").count(), 1, "one quote, not one in another: {html}");
    assert!(html.contains("Settled") && html.contains("<table") && pictures.len() == 1, "{html}");
    assert_eq!(sent_as.2.len(), pictures.len());
    s.run("compose.discard", &json!({})).unwrap();
    // Through the Outbox.
    let store = s.store.clone().unwrap();
    let (text, html, inline) = sent_as;
    let d = quatrelle_mime::Draft {
        to: vec![quatrelle_model::EmailAddress::new("", "morgan@mill.example")],
        subject: "Re: Invoice".into(),
        body: text,
        html,
        inline,
        ..Default::default()
    };
    let raw = quatrelle_mime::build(&d, &quatrelle_model::EmailAddress::new("Riley", "riley@studio.example"), NOW, "out@x").unwrap();
    let item = quatrelle_store::OutboxItem {
        id: 77,
        recipients: vec!["morgan@mill.example".into()],
        queued: NOW,
        error: Some("offline".into()),
        ..Default::default()
    };
    store.outbox_add("sample", &item, &raw).unwrap();
    s.apply(Event::Queued { account: "sample".into(), id: 77, error: "offline".into() });
    s.run("folder.outbox", &json!({})).unwrap();
    let key = s.list.iter().find(|m| m.subject == "Re: Invoice").map(|m| m.key.clone()).expect("in the Outbox");
    s.run("outbox.edit", &json!({"key": key.to_id()})).unwrap();
    let c = s.compose.clone().unwrap();
    assert!(c.attachments.is_empty(), "its picture is in its original, not a file: {:?}", c.attachments.iter().map(|a| &a.name).collect::<Vec<_>>());
    let q = c.quote.clone().expect("its original as it was");
    assert!(!q.parsed.layout.is_empty() && (0..q.cells()).any(|n| q.cell(n).is_some_and(|(t, _)| t.contains("Settled"))));
    let (_, html, pictures) = c.bodies();
    assert!(html.unwrap_or_default().contains("cid:phone@mill.example") && pictures.len() == 1);
    let _ = MILL;
    done(s);
}

/// The owner's report (2026-11-10): what's selected from the text into the original, or across
/// its blocks, couldn't be taken out in one go. From the middle of the text to the end of the
/// original's last block: the text is cut there, every block of the original goes (its tables,
/// its picture), nothing of it is sent and its picture isn't attached any more; put back, the
/// message goes as it came. A table taken in part keeps its cells, emptied of what was selected.
#[test]
fn a_selection_across_the_text_and_the_original_is_taken_out() {
    use crate::QuotePoint;
    let mut s = sample_session("quote-across");
    open_mill(&mut s);
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.as_mut().unwrap();
    let before = c.written();
    let came = c.bodies().1.unwrap_or_default();
    let cut = c.body.chars().count() / 2;
    let kept: String = c.body.chars().take(cut).collect();
    let last = c.quote.as_ref().unwrap().cells();
    let end = c.block_text(last).map(|(t, _)| t.chars().count()).unwrap();
    let (from, to) = (QuotePoint { block: 0, at: cut }, QuotePoint { block: last, at: end });
    // Copied, it reads in order.
    let text = c.text_across(from, to);
    let places: Vec<usize> = ["updated invoice", "+00 1234 5678", "A warning in red", "Mill"].iter().map(|w| text.find(w).expect(w)).collect();
    assert!(places.is_sorted(), "{text}");
    // Taken out, the far end first as a drag upward gives it.
    assert_eq!(c.delete_across(to, from), Some(from), "the cursor where the two ends meet");
    assert_eq!(c.body, kept);
    assert_eq!(c.selectable_blocks(), [0], "no block of the original is left to select in");
    let (text, html, pictures) = c.bodies();
    let html = html.unwrap_or_default();
    for gone in ["updated invoice", "<table", "1234", "A warning in red", "cid:phone@mill.example"] {
        assert!(!html.contains(gone), "{gone} in {html}");
    }
    assert!(!text.contains("A warning in red"), "{text}");
    assert!(pictures.is_empty(), "a picture nothing shows any more isn't attached");
    assert!(html.contains("class=\"quatrelle-quote\""), "{html}");
    let after = c.written();
    assert!(c.is_written(&after) && !c.is_written(&before));
    // Put back: as it came.
    c.set_written(before.clone());
    assert!(c.is_written(&before));
    assert_eq!(c.bodies().1.unwrap_or_default(), came);
    assert_eq!(c.bodies().2.len(), 1);
    // In part: from inside the first paragraph to inside the warning. The paragraph is cut, the
    // table stays with the cells between emptied and the warning's cut, the picture's cell (it
    // was whole between the two) empty, so the picture goes.
    let (first, warn) = (cell_with(c, "updated invoice"), cell_with(c, "A warning in red"));
    assert_eq!(
        c.delete_across(QuotePoint { block: first + 1, at: 7 }, QuotePoint { block: warn + 1, at: 2 }),
        Some(QuotePoint { block: first + 1, at: 7 })
    );
    let q = c.quote.clone().unwrap();
    assert_eq!(q.cell(first).unwrap().0, "We will");
    assert_eq!(q.cell(warn).unwrap().0, "warning in red");
    assert!(q.removed.is_empty(), "no block is wholly taken: {:?}", q.removed);
    let (_, html, pictures) = c.bodies();
    let html = html.unwrap_or_default();
    assert!(html.contains("We will<") && html.contains("warning in red") && html.contains("<table") && html.contains(">Mill<"), "{html}");
    assert!(!html.contains("1234") && !html.contains("cid:phone@mill.example") && pictures.is_empty(), "{html}");
    done(s);
}

/// A message being written is kept as before when nothing was taken out of its original (the
/// messages already saved read back the same), and what was taken out is kept with it.
#[test]
fn what_was_taken_out_of_the_original_is_kept_with_the_message() {
    use crate::QuotePoint;
    let mut s = sample_session("quote-across-kept");
    open_mill(&mut s);
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.as_mut().unwrap();
    let saved = serde_json::to_value(&*c).unwrap();
    assert!(saved["quote"].get("removed").is_none(), "nothing new in a message nothing was taken out of");
    let again: crate::Compose = serde_json::from_value(saved).unwrap();
    assert!(again.quote == c.quote && again.bodies() == c.bodies(), "one saved before reads back as it was");
    let first = cell_with(c, "updated invoice");
    let end = c.block_text(c.quote.as_ref().unwrap().cells()).map(|(t, _)| t.chars().count()).unwrap();
    c.delete_across(QuotePoint { block: first + 1, at: 7 }, QuotePoint { block: c.quote.as_ref().unwrap().cells(), at: end });
    let saved = serde_json::to_value(&*c).unwrap();
    let again: crate::Compose = serde_json::from_value(saved).unwrap();
    assert!(again.bodies() == c.bodies(), "read back, it goes out the same");
    assert!(!again.quote.as_ref().unwrap().removed.is_empty());
    // Saved to Drafts and opened again: its original is as it was left.
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.compose = None;
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    let key = s.list.iter().find(|m| m.subject == "Re: Invoice").map(|m| m.key.clone()).expect("in Drafts");
    s.run("draft.edit", &json!({"key": key.to_id()})).unwrap();
    let c = s.compose.as_ref().expect("being written again");
    let q = c.quote.clone().expect("its original");
    assert!(q.text().contains("We will") && !q.text().contains("1234") && !q.text().contains("A warning in red"), "{}", q.text());
    assert!(!c.bodies().1.unwrap_or_default().contains("<table"));
    done(s);
}
