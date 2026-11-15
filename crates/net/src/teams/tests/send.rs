// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Posting a message's HTML: what goes and what Graph makes of it, the chat's id and the size
//! checked before any request, a 503 never waited out, and a connection cut after the post.

use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::{SEND_MAX, Sender};

use super::*;

/// What the fake holds of `CHAT`'s messages, by HTML.
fn held(server: &Server) -> Vec<String> {
    let d = server.data.lock().unwrap();
    d.teams.as_ref().unwrap().chat(CHAT).unwrap().messages.iter().map(|m| m.body_html.clone()).collect()
}

fn posts(server: &Server) -> Vec<(String, serde_json::Value)> {
    server.data.lock().unwrap().teams.as_ref().unwrap().posts.clone()
}

#[test]
fn a_message_goes_as_html_and_comes_back_as_graph_made_it() {
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 2)]));
    let c = client(&server);
    let v = c.send_html(CHAT, "<p>hi &amp; bye</p>").unwrap();
    let m = message_of(CHAT, &v);
    assert!(matches!(&m.sender, Sender::User { id, .. } if id == ME), "{:?}", m.sender);
    assert_eq!(m.body.plain(), "hi & bye");
    assert!(!m.unshowable && !m.id.is_empty());
    let posts = posts(&server);
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].0, CHAT);
    assert_eq!(posts[0].1["body"]["contentType"], "html");
    assert_eq!(posts[0].1["body"]["content"], "<p>hi &amp; bye</p>");
    assert_eq!(held(&server).last().map(String::as_str), Some("<p>hi &amp; bye</p>"));
}

#[test]
fn an_id_that_cant_be_one_is_refused_before_any_request() {
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 2)]));
    let c = client(&server);
    for id in ["19:a/b", "x?y", "x#y", "x\\y", "", "..", "x\ny"] {
        assert!(matches!(c.send_html(id, "<p>hi</p>"), Err(NetError::Refused(_))), "{id:?}");
        assert!(matches!(c.mark_read(id, ME, TENANT), Err(NetError::Refused(_))), "{id:?}");
    }
    assert_eq!(hits(&server, "post") + hits(&server, "markChatReadForUser"), 0);
    assert_eq!(c.stats().requests, 1, "/me only");
    // A chat's own id goes.
    c.mark_read(CHAT, ME, TENANT).unwrap();
    assert_eq!(hits(&server, "markChatReadForUser"), 1);
}

#[test]
fn a_body_past_the_cap_is_refused_before_any_request() {
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 2)]));
    let c = client(&server);
    assert!(matches!(c.send_html(CHAT, &"a".repeat(SEND_MAX + 1)), Err(NetError::Refused(_))));
    assert_eq!((hits(&server, "post"), c.stats().requests), (0, 1));
    // The cap itself goes.
    c.send_html(CHAT, &"a".repeat(SEND_MAX)).unwrap();
    assert_eq!(hits(&server, "post"), 1);
    assert_eq!(held(&server).last().map(String::len), Some(SEND_MAX));
}

#[test]
fn a_503_after_a_post_is_not_waited_out() {
    let mut t = teams_with(vec![chat(CHAT, T0, 2)]);
    t.answer_lost.insert("post".into(), 1);
    t.lost_retry_after = Some(1);
    let server = serve_teams(t);
    let c = client(&server);
    c.set_waiting(true);
    let start = Instant::now();
    let sent = c.send_html(CHAT, "<p>once</p>");
    assert!(start.elapsed() < Duration::from_millis(500), "{:?}", start.elapsed());
    assert!(matches!(&sent, Err(NetError::Closed(m)) if m.starts_with("HTTP 503")), "{sent:?}");
    assert_eq!(c.retry_after(), Some(1), "its Retry-After heard, for the worker to honour");
    assert_eq!(posts(&server).len(), 1);
    assert_eq!(held(&server), ["<p>Message 0</p>", "<p>Message 1</p>", "<p>once</p>"], "made, though the answer was lost");
}

#[test]
fn a_cut_connection_after_a_post_is_an_error_and_the_message_is_there() {
    let mut t = teams_with(vec![chat(CHAT, T0, 2)]);
    t.cut_after.insert("post".into(), 1);
    let server = serve_teams(t);
    let c = client(&server);
    let sent = c.send_html(CHAT, "<p>cut</p>");
    assert!(sent.is_err() && !matches!(sent, Err(NetError::Auth(_))), "{sent:?}");
    assert_eq!((hits(&server, "post"), c.stats().requests), (1, 2), "posted once, not again");
    assert_eq!(held(&server).last().map(String::as_str), Some("<p>cut</p>"));
    // Only the next one was cut.
    c.send_html(CHAT, "<p>next</p>").unwrap();
    assert_eq!(held(&server).len(), 4);
}
