// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a sender's links in a conversation (security review 1.5, content n° 3). What the
//! pointer shows over a link, or over a file card's "Open in browser", is where it goes: with
//! links cleaned (File › Options › Safety), out of the wrapper that carries it, as the mail shows;
//! a link whose address out of its wrapper couldn't be handed to a browser is plain text, and its
//! card offers no "Open in browser".

use quatrelle_engine::model::pim::teams::{Attachment, ChatMessage, Run};

use super::chat::{TALL, TEAMS_LINK, chat_app, click_pane, group_row, pane, para, said, settled_at, texts_with, user, words};
use super::*;
use crate::Action;

/// A wrapper that carries `https://piege.example/pay`, which a browser may be handed.
const WRAPPED: &str = "https://www.google.com/url?q=https%3A%2F%2Fpiege.example%2Fpay";
/// A wrapper that carries an address with user information (`banque.example@`), which can't be.
const WRAPPED_AT: &str = "https://www.google.com/url?q=https%3A%2F%2Fbanque.example%40piege.example%2F";
/// The same with a `\` written in the wrapper and carried: neither host can be read.
const WRAPPED_AT_BACKSLASH: &str = "https://www.google.com/url?q=https%3A%2F%2Fbanque.example%40piege.example%2F%5C&z=\\";

/// A message of Ana's saying "Read the plan", "the plan" a link to `url`.
fn linked(url: &str) -> ChatMessage {
    said(
        "m1",
        user("u-ana", "Ana Exemple"),
        NOW,
        para(vec![words("Read "), Run { text: "the plan".into(), link: Some(url.into()), ..Default::default() }]),
    )
}

/// The pane with the pointer at `at` for a second (a tooltip's wait): what it asked for, and what
/// the last frame drew.
fn pointed(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) -> (Vec<Action>, Vec<(String, egui::Rect)>) {
    let mut acts = pane(ctx, a, vec![Event::PointerMoved(at)], TALL).0;
    for _ in 0..60 {
        acts.extend(pane(ctx, a, vec![], TALL).0);
    }
    let (more, texts) = pane(ctx, a, vec![], TALL);
    acts.extend(more);
    (acts, texts)
}

/// The pane alone settled: what it draws, where (as it's pointed at and clicked).
fn settled_pane(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, egui::Rect)> {
    settled_at(ctx, a, TALL);
    for _ in 0..3 {
        pane(ctx, a, vec![], TALL);
    }
    pane(ctx, a, vec![], TALL).1
}

/// Where the link "the plan" is drawn: the end of "Read the plan".
fn the_link(texts: &[(String, egui::Rect)]) -> egui::Pos2 {
    let r = rect_of(texts, |t| t == "Read the plan").expect("the message's text");
    egui::pos2(r.right() - 12.0, r.center().y)
}

/// Links cleaned (the default): the pointer on a wrapped link shows the address it carries, never
/// the wrapper's; links not cleaned, the address as written.
#[test]
fn a_wrapped_link_shows_where_it_goes() {
    let (mut a, dir, _) = chat_app("teams-link-wrapped", group_row(), TEAMS_LINK, vec![linked(WRAPPED)]);
    assert!(a.session.prefs.clean_links, "cleaned by default");
    let ctx = egui::Context::default();
    let texts = settled_pane(&ctx, &mut a);
    let at = the_link(&texts);
    let (_, texts) = pointed(&ctx, &mut a, at);
    assert!(has_text(&texts, "https://piege.example/pay"), "where it goes: {texts:?}");
    assert!(texts_with(&texts, "google.com").is_empty(), "not the wrapper: {texts:?}");
    // A click asks for it as written: the window takes its wrapper off as it opens it.
    let acts = click_pane(&ctx, &mut a, at, TALL);
    assert!(acts.iter().any(|x| matches!(x, Action::OpenUrl(u) if u == WRAPPED)), "a click opens it");
    let (mut a2, dir2, _) = chat_app("teams-link-written", group_row(), TEAMS_LINK, vec![linked(WRAPPED)]);
    a2.session.prefs.clean_links = false;
    let texts = settled_pane(&ctx, &mut a2);
    let (_, texts) = pointed(&ctx, &mut a2, the_link(&texts));
    assert!(has_text(&texts, WRAPPED), "as written, links not cleaned: {texts:?}");
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
}

/// A wrapped link whose address couldn't be handed to a browser (`banque.example@piege.example`:
/// another host than it reads as) is plain text: no tooltip, and a click asks for nothing.
#[test]
fn a_wrapped_link_that_cant_be_opened_is_plain_text() {
    let (mut a, dir, _) = chat_app("teams-link-at", group_row(), TEAMS_LINK, vec![linked(WRAPPED_AT)]);
    let ctx = egui::Context::default();
    let texts = settled_pane(&ctx, &mut a);
    let at = the_link(&texts);
    let (_, texts) = pointed(&ctx, &mut a, at);
    assert!(texts_with(&texts, "piege.example").is_empty(), "no tooltip: {texts:?}");
    let acts = click_pane(&ctx, &mut a, at, TALL);
    assert!(!acts.iter().any(|x| matches!(x, Action::OpenUrl(_))), "a click asked for a link");
    let _ = std::fs::remove_dir_all(dir);
}

/// A file card whose link is wrapped names the host it goes to, and "Open in browser" shows the
/// address it carries; one carrying an address that couldn't be handed to a browser has no "Open
/// in browser".
#[test]
fn a_file_cards_wrapped_link_names_where_it_goes() {
    let file = |url: &str| {
        let mut m = said("m1", user("u-ana", "Ana Exemple"), NOW, para(vec![words("A file")]));
        m.attachments = vec![Attachment::File { id: "f-1".into(), name: "report.pdf".into(), url: url.into(), size: Some(2048) }];
        m
    };
    let ctx = egui::Context::default();
    let (mut a, dir, _) = chat_app("teams-link-card", group_row(), TEAMS_LINK, vec![file(WRAPPED)]);
    let texts = settled_pane(&ctx, &mut a);
    assert!(!texts_with(&texts, "piege.example").is_empty() && texts_with(&texts, "google.com").is_empty(), "the host it goes to: {texts:?}");
    assert!(has_text(&texts, "Link to another site"));
    let button = rect_of(&texts, |t| t == "Open in browser").expect("Open in browser");
    let (_, texts) = pointed(&ctx, &mut a, button.center());
    assert!(has_text(&texts, "https://piege.example/pay"), "where it goes: {texts:?}");
    let (mut a2, dir2, _) = chat_app("teams-link-card-at", group_row(), TEAMS_LINK, vec![file(WRAPPED_AT)]);
    let texts = settled_pane(&ctx, &mut a2);
    assert!(!has_text(&texts, "Open in browser"), "{texts:?}");
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
}

/// Re-review n° 3: a file card whose link is the organisation's link check (Safe Links) around the
/// user's own SharePoint names where it goes, but isn't the user's own: a click opens the check's
/// address, which a sender can forge. Out of a plain redirect, what opens is the SharePoint, which
/// is the user's.
#[test]
fn a_file_card_is_the_users_own_only_by_what_its_click_opens() {
    let file = |url: &str| {
        let mut m = said("m1", user("u-ana", "Ana Exemple"), NOW, para(vec![words("A file")]));
        m.attachments = vec![Attachment::File { id: "f-1".into(), name: "notes.docx".into(), url: url.into(), size: Some(2048) }];
        m
    };
    let ctx = egui::Context::default();
    let gate = "https://eur01.safelinks.protection.outlook.com/?url=https%3A%2F%2Fcontoso.sharepoint.com%2Fsites%2Fteam%2Fnotes.docx&data=05%7C01&reserved=0";
    let (mut a, dir, _) = chat_app("teams-link-card-gate", group_row(), TEAMS_LINK, vec![file(gate)]);
    let texts = settled_pane(&ctx, &mut a);
    assert!(!texts_with(&texts, "contoso.sharepoint.com").is_empty(), "where it goes: {texts:?}");
    assert!(has_text(&texts, "Link to another site"), "not the user's own: {texts:?}");
    let plain = "https://www.google.com/url?q=https%3A%2F%2Fcontoso.sharepoint.com%2Fsites%2Fteam%2Fnotes.docx";
    let (mut a2, dir2, _) = chat_app("teams-link-card-redirect", group_row(), TEAMS_LINK, vec![file(plain)]);
    let texts = settled_pane(&ctx, &mut a2);
    assert!(!has_text(&texts, "Link to another site"), "the SharePoint opens: {texts:?}");
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
}

/// Re-review, rounds 1 and 2, n° 1: a wrapper written with a `\`, and carrying one, has no host
/// that can be read on either side, which isn't the same host: what it carries is checked, and the
/// link is plain text (no tooltip, a click asks for nothing).
#[test]
fn a_wrapper_whose_hosts_cant_be_read_is_plain_text() {
    let (mut a, dir, _) = chat_app("teams-link-backslash", group_row(), TEAMS_LINK, vec![linked(WRAPPED_AT_BACKSLASH)]);
    let ctx = egui::Context::default();
    let texts = settled_pane(&ctx, &mut a);
    let at = the_link(&texts);
    let (_, texts) = pointed(&ctx, &mut a, at);
    assert!(texts_with(&texts, "piege.example").is_empty(), "no tooltip: {texts:?}");
    let acts = click_pane(&ctx, &mut a, at, TALL);
    assert!(!acts.iter().any(|x| matches!(x, Action::OpenUrl(_))), "a click asked for a link");
    let _ = std::fs::remove_dir_all(dir);
}
