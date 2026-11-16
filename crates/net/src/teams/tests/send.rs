// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Posting a message's HTML: what goes and what Graph makes of it, the chat's id and the size
//! checked before any request, a 503 never waited out, and a connection cut after the post.

use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::{Mention, SEND_MAX, Sender};
use serde_json::json;

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
    let v = c.send_html(CHAT, "<p>hi &amp; bye</p>", &[]).unwrap();
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
        assert!(matches!(c.send_html(id, "<p>hi</p>", &[]), Err(NetError::Refused(_))), "{id:?}");
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
    assert!(matches!(c.send_html(CHAT, &"a".repeat(SEND_MAX + 1), &[]), Err(NetError::Refused(_))));
    assert_eq!((hits(&server, "post"), c.stats().requests), (0, 1));
    // The cap itself goes.
    c.send_html(CHAT, &"a".repeat(SEND_MAX), &[]).unwrap();
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
    let sent = c.send_html(CHAT, "<p>once</p>", &[]);
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
    let sent = c.send_html(CHAT, "<p>cut</p>", &[]);
    assert!(sent.is_err() && !matches!(sent, Err(NetError::Auth(_))), "{sent:?}");
    assert_eq!((hits(&server, "post"), c.stats().requests), (1, 2), "posted once, not again");
    assert_eq!(held(&server).last().map(String::as_str), Some("<p>cut</p>"));
    // Only the next one was cut.
    c.send_html(CHAT, "<p>next</p>", &[]).unwrap();
    assert_eq!(held(&server).len(), 4);
}

/// The own message and Ana's of `chat(CHAT, T0, 2)`: ids.
const ANA_MSG: &str = "1790812800000";
const OWN_MSG: &str = "1790812860000";

fn writes(server: &Server) -> Vec<(String, String, String, serde_json::Value)> {
    server.data.lock().unwrap().teams.as_ref().unwrap().writes.clone()
}

fn message(server: &Server, id: &str) -> crate::fake_graph::FakeMessage {
    let d = server.data.lock().unwrap();
    d.teams.as_ref().unwrap().chat(CHAT).unwrap().messages.iter().find(|m| m.id == id).unwrap().clone()
}

fn ana(id: u32) -> Mention {
    Mention { id, text: "Ana Lima".into(), user_id: Some("u-ana".into()) }
}

fn all_hits(server: &Server) -> u32 {
    server.data.lock().unwrap().teams.as_ref().unwrap().hits.iter().filter(|(k, _)| k.as_str() != "me").map(|(_, n)| *n).sum()
}

#[test]
fn a_message_with_mentions_goes_with_its_array() {
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 2)]));
    let c = client(&server);
    let v = c.send_html(CHAT, "<p>hi <at id=\"0\">Ana Lima</at></p>", &[ana(0)]).unwrap();
    let sent = posts(&server);
    assert_eq!(
        sent[0].1["mentions"],
        json!([{"id":0,"mentionText":"Ana Lima","mentioned":{"user":{"id":"u-ana","displayName":"Ana Lima","userIdentityType":"aadUser"}}}])
    );
    assert_eq!(message_of(CHAT, &v).mentions[0].user_id.as_deref(), Some("u-ana"));
    c.send_html(CHAT, "<p>plain</p>", &[]).unwrap();
    assert!(posts(&server)[1].1.get("mentions").is_none());
}

#[test]
fn at_and_mentions_must_agree() {
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 2)]));
    let c = client(&server);
    let a = c.send_html(CHAT, "<p><at id=\"1\">Ana Lima</at></p>", &[ana(0)]);
    assert!(matches!(&a, Err(NetError::Protocol(m)) if m.starts_with("HTTP 400")), "{a:?}");
    let b = c.send_html(CHAT, "<p>Ana Lima</p>", &[ana(0)]);
    assert!(matches!(&b, Err(NetError::Protocol(m)) if m.starts_with("HTTP 400")), "{b:?}");
    let d = c.send_html(CHAT, "<p><at id=\"0\">Ana Lima</at></p>", &[]);
    assert!(matches!(&d, Err(NetError::Protocol(m)) if m.starts_with("HTTP 400")), "{d:?}");
    let e = c.edit_html(CHAT, OWN_MSG, "<p><at id=\"0\">Ana Lima</at></p>", &[]);
    assert!(matches!(&e, Err(NetError::Protocol(m)) if m.starts_with("HTTP 400")), "{e:?}");
    assert_eq!(held(&server).len(), 2, "nothing was made");
    c.edit_html(CHAT, OWN_MSG, "<p><at id=\"0\">Ana Lima</at></p>", &[ana(0)]).unwrap();
    assert_eq!(message(&server, OWN_MSG).mentions, ["u-ana"]);
}

#[test]
fn an_edit_patches_the_body_and_clears_mentions() {
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 2)]));
    let c = client(&server);
    c.edit_html(CHAT, OWN_MSG, "<p>b</p>", &[]).unwrap();
    assert_eq!(
        writes(&server)[0],
        ("edit".to_string(), CHAT.to_string(), OWN_MSG.to_string(), json!({"body":{"contentType":"html","content":"<p>b</p>"},"mentions":[]}))
    );
    let m = message(&server, OWN_MSG);
    assert_eq!(m.body_html, "<p>b</p>");
    assert!(m.edited.is_some());
    let other = c.edit_html(CHAT, ANA_MSG, "<p>b</p>", &[]);
    assert!(matches!(&other, Err(NetError::Protocol(m)) if m.starts_with("HTTP 403")), "{other:?}");
    assert_eq!(message(&server, ANA_MSG).body_html, "<p>Message 0</p>");
    assert_eq!(writes(&server).len(), 2, "received, whatever the answer");
}

#[test]
fn a_reaction_goes_as_its_emoji_and_a_name_isnt_one() {
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 2)]));
    let c = client(&server);
    assert!(matches!(c.set_reaction(CHAT, ANA_MSG, "like"), Err(NetError::Refused(_))));
    assert_eq!(hits(&server, "setReaction"), 0);
    c.set_reaction(CHAT, ANA_MSG, "\u{1F44D}").unwrap();
    assert_eq!(writes(&server)[0].3, json!({"reactionType": "\u{1F44D}"}));
    assert_eq!(message(&server, ANA_MSG).reactions, [("\u{1F44D}".to_string(), ME.to_string())]);
}

#[test]
fn a_reaction_listed_by_name_is_taken_back_by_its_emoji() {
    let mut t = teams_with(vec![chat(CHAT, T0, 2)]);
    t.reactions_by_name = true;
    t.chat_mut(CHAT).unwrap().messages[0].reactions.push(("\u{1F44D}".into(), ME.into()));
    let server = serve_teams(t);
    let c = client(&server);
    let page = c.messages(CHAT, Range::After("2026-01-01T00:00:00Z"), None).unwrap();
    let listed = page.items.iter().find(|m| m["id"] == ANA_MSG).unwrap();
    assert_eq!(listed["reactions"][0]["reactionType"], "like");
    c.unset_reaction(CHAT, ANA_MSG, "\u{1F44D}").unwrap();
    assert!(message(&server, ANA_MSG).reactions.is_empty());
}

#[test]
fn only_ones_own_message_is_deleted() {
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 2)]));
    let c = client(&server);
    let other = c.soft_delete(CHAT, ANA_MSG, Some(ME));
    assert!(matches!(&other, Err(NetError::Protocol(m)) if m.starts_with("HTTP 403")), "{other:?}");
    assert!(message(&server, ANA_MSG).deleted.is_none());
    c.soft_delete(CHAT, OWN_MSG, Some(ME)).unwrap();
    let at = message(&server, OWN_MSG).deleted;
    assert!(at.is_some());
    c.soft_delete(CHAT, OWN_MSG, None).unwrap();
    assert_eq!(message(&server, OWN_MSG).deleted, at, "deleted already: 204, nothing moved");
}

#[test]
fn ids_that_arent_one_segment_are_refused_before_any_request() {
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 2)]));
    let c = client(&server);
    assert!(matches!(c.edit_html("19:a/b", OWN_MSG, "<p>x</p>", &[]), Err(NetError::Refused(_))));
    assert!(matches!(c.edit_html(CHAT, "1/2", "<p>x</p>", &[]), Err(NetError::Refused(_))));
    assert!(matches!(c.edit_text("19:a/b", OWN_MSG, "x"), Err(NetError::Refused(_))));
    assert!(matches!(c.set_reaction(CHAT, "1/2", "\u{1F44D}"), Err(NetError::Refused(_))));
    assert!(matches!(c.unset_reaction("a?b", OWN_MSG, "\u{1F44D}"), Err(NetError::Refused(_))));
    assert!(matches!(c.soft_delete(CHAT, "..", Some(ME)), Err(NetError::Refused(_))));
    assert!(matches!(c.soft_delete(CHAT, "1", Some("u/x")), Err(NetError::Refused(_))));
    assert!(matches!(c.soft_delete("19:a#b", "1", None), Err(NetError::Refused(_))));
    assert!(matches!(c.send_text("x?y", "hi"), Err(NetError::Refused(_))));
    assert_eq!(all_hits(&server), 0);
    assert!(writes(&server).is_empty());
}

#[test]
fn an_edit_past_the_cap_is_refused_before_any_request() {
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 2)]));
    let c = client(&server);
    assert!(matches!(c.edit_html(CHAT, OWN_MSG, &"a".repeat(SEND_MAX + 1), &[]), Err(NetError::Refused(_))));
    assert_eq!(hits(&server, "edit"), 0);
    c.edit_html(CHAT, OWN_MSG, &"a".repeat(SEND_MAX), &[]).unwrap();
    assert_eq!(hits(&server, "edit"), 1);
}

#[test]
fn every_write_route_honours_the_failures_asked_for() {
    let mut t = teams_with(vec![chat(CHAT, T0, 2)]);
    t.fail_next.insert("edit".into(), 1);
    t.answer_lost.insert("setReaction".into(), 1);
    t.cut_after.insert("softDelete".into(), 1);
    let server = serve_teams(t);
    let c = client(&server);
    c.set_waiting(false);
    let a = c.edit_html(CHAT, OWN_MSG, "<p>x</p>", &[]);
    assert!(matches!(&a, Err(NetError::Closed(m)) if m.starts_with("HTTP 500")), "{a:?}");
    assert_eq!(message(&server, OWN_MSG).body_html, "<p>Message 1</p>", "failed before it was made");
    let b = c.set_reaction(CHAT, ANA_MSG, "\u{1F44D}");
    assert!(matches!(&b, Err(NetError::Closed(m)) if m.starts_with("HTTP 503")), "{b:?}");
    assert_eq!(message(&server, ANA_MSG).reactions.len(), 1, "made, though the answer was lost");
    assert!(c.soft_delete(CHAT, OWN_MSG, Some(ME)).is_err());
    assert!(message(&server, OWN_MSG).deleted.is_some(), "made, then the connection closed");
    let routes: Vec<String> = writes(&server).into_iter().map(|w| w.0).collect();
    assert_eq!(routes, ["edit", "setReaction", "softDelete"]);
}

#[test]
fn a_forbidden_chat_refuses_edits_and_reactions() {
    let mut t = teams_with(vec![chat(CHAT, T0, 2)]);
    t.forbid_edit.push(CHAT.into());
    t.forbid_react.push(CHAT.into());
    let server = serve_teams(t);
    let c = client(&server);
    for r in [c.edit_html(CHAT, OWN_MSG, "<p>x</p>", &[]), c.set_reaction(CHAT, OWN_MSG, "\u{1F44D}"), c.unset_reaction(CHAT, OWN_MSG, "\u{1F44D}")] {
        assert!(matches!(&r, Err(NetError::Protocol(m)) if m.starts_with("HTTP 403")), "{r:?}");
    }
    c.soft_delete(CHAT, OWN_MSG, Some(ME)).unwrap();
}

#[test]
fn an_edit_needs_its_content_type_and_fits_the_limit() {
    let mut t = teams_with(vec![chat(CHAT, T0, 2)]);
    t.post_limit = Some(10);
    let server = serve_teams(t);
    let c = client(&server);
    let long = c.edit_html(CHAT, OWN_MSG, "<p>0123456789</p>", &[]);
    assert!(matches!(&long, Err(NetError::Protocol(m)) if m.starts_with("HTTP 413")), "{long:?}");
    let by_text = c.edit_text(CHAT, OWN_MSG, "ok");
    assert!(by_text.is_ok(), "{by_text:?}");
}

/// Seeds `ANA_MSG` of a fresh fake with the reaction `kind` of ME.
fn seeded(kind: &str) -> Server {
    let mut t = teams_with(vec![chat(CHAT, T0, 2)]);
    t.chat_mut(CHAT).unwrap().messages[0].reactions.push((kind.into(), ME.into()));
    serve_teams(t)
}

#[test]
fn a_reaction_kept_by_name_is_taken_back_by_its_emoji() {
    let server = seeded("like");
    client(&server).unset_reaction(CHAT, ANA_MSG, "\u{1F44D}").unwrap();
    assert!(message(&server, ANA_MSG).reactions.is_empty());
}

#[test]
fn a_reaction_set_by_emoji_is_the_one_kept_by_name() {
    let server = seeded("like");
    client(&server).set_reaction(CHAT, ANA_MSG, "\u{1F44D}").unwrap();
    let mine: Vec<_> = message(&server, ANA_MSG).reactions.into_iter().filter(|(_, u)| u == ME).collect();
    assert_eq!(mine.len(), 1, "{mine:?}");
}

#[test]
fn a_heart_with_or_without_its_selector_is_one_kind() {
    let server = seeded("\u{2764}");
    client(&server).unset_reaction(CHAT, ANA_MSG, "\u{2764}\u{FE0F}").unwrap();
    assert!(message(&server, ANA_MSG).reactions.is_empty());
}
