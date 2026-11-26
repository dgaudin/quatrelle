// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: calling from a conversation. "Call" in the header with "Audio call" and "Video call"
//! in a one-to-one chat and a group whose members all have an address (and not in a meeting chat,
//! a chat the user left, or a chat with no address), the engine's link opened at once in the
//! meetings browser's window (else the default browser), a page that isn't a Teams call link
//! opened nowhere, and the log's line for a call naming no one; the header with Call, light and
//! dark.

use quatrelle_engine::TeamsPage;
use quatrelle_engine::model::pim::teams::Member;

use super::chat::{CHAT, ME, TALL, TEAMS_LINK, TENANT_ID, chat_app, click_pane, member, pane, settled_at};
use super::*;
use crate::Action;
use crate::tests::meetings::{launching_with, opened};

const SIZE: (usize, usize) = (1280, 800);
const AUDIO: &str = "https://teams.microsoft.com/l/call/0/0?users=ana%40contoso.example";

fn with_address(id: &str, name: &str, address: &str) -> Member {
    Member { address: address.into(), ..member(id, name, TENANT_ID) }
}

fn ana_and_me() -> Vec<Member> {
    vec![with_address(ME, "Riley Moreno", "riley@contoso.example"), with_address("u-ana", "Ana Exemple", "ana@contoso.example")]
}

fn with_ana(row: ChatRow) -> ChatRow {
    ChatRow { name: "Ana Exemple".into(), kind: ChatKind::OneOnOne, members: 2, person: Some("u-ana".into()), ..row }
}

/// The app in a chat `row` whose members are `members`, the pane settled.
fn called(name: &str, row: ChatRow, members: Vec<Member>) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir, _) = chat_app(name, row, TEAMS_LINK, vec![]);
    a.session.teams_apply(teams::Event::Members { chat: CHAT.into(), members, web_url: Some(TEAMS_LINK.into()), first: true });
    let ctx = egui::Context::default();
    settled_at(&ctx, &mut a, SIZE);
    (a, dir, ctx)
}

#[test]
fn call_offers_audio_and_video_and_asks_the_engine() {
    let row = with_ana(super::chat::group_row());
    let (mut a, dir, ctx) = called("teams-call-offered", row, ana_and_me());
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    let name = rect_of(&texts, |t| t == "Ana Exemple").expect("the name");
    let call = rect_of(&texts, |t| t == "Call").expect("Call drawn");
    assert!((call.center().y - name.center().y).abs() < 14.0, "beside the name: {call:?} {name:?}");
    assert!(has_text(&texts, "Open in Teams") && has_text(&texts, "Members: 2"), "{texts:?}");
    // Left of the members' button.
    let members = rect_of(&texts, |t| t.starts_with("Members")).expect("members");
    assert!(call.right() < members.left(), "{call:?} {members:?}");
    for (item, video) in [("Audio call", false), ("Video call", true)] {
        click_pane(&ctx, &mut a, call.center(), SIZE);
        for _ in 0..3 {
            pane(&ctx, &mut a, vec![], SIZE);
        }
        let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
        let entry = rect_of(&texts, |t| t == item).unwrap_or_else(|| panic!("{item} in the menu: {texts:?}"));
        let acts = click_pane(&ctx, &mut a, entry.center(), SIZE);
        let asked: Vec<_> = acts.iter().filter_map(|x| if let Action::Run("teams.call", p) = x { Some(p.clone()) } else { None }).collect();
        assert_eq!(asked, [json!({"video": video})], "{item}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn no_call_in_a_meeting_chat_a_chat_left_or_without_addresses() {
    let one = || with_ana(super::chat::group_row());
    let no_address = vec![with_address(ME, "Riley Moreno", "riley@contoso.example"), with_address("u-ana", "Ana Exemple", "")];
    let cases = [
        ("meeting", ChatRow { kind: ChatKind::Meeting, ..one() }, ana_and_me()),
        ("left", ChatRow { left: true, ..one() }, ana_and_me()),
        ("no-address", one(), no_address),
        ("alone", one(), vec![with_address(ME, "Riley Moreno", "riley@contoso.example")]),
    ];
    for (name, row, members) in cases {
        let (mut a, dir, ctx) = called(&format!("teams-call-none-{name}"), row, members);
        let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
        assert!(!has_text(&texts, "Call"), "{name}: {texts:?}");
        assert!(has_text(&texts, "Open in Teams"), "{name}: the header is drawn: {texts:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The engine's call, opened at once (no frame between) in the meetings browser's window.
fn calling(name: &str, browsers: &[(&str, &str)]) -> (MailApp, std::path::PathBuf, std::rc::Rc<std::cell::RefCell<Vec<String>>>) {
    let (mut a, dir, o) = launching_with(name, browsers, &[], false);
    let t = &mut a.session.prefs.teams;
    t.on = true;
    t.signed_in = true;
    t.object_id = ME.into();
    let row = with_ana(super::chat::group_row());
    a.session.teams_apply(teams::Event::Chats { rows: vec![row] });
    a.run("teams.open", json!({"chat": CHAT}));
    a.session.teams_apply(teams::Event::Members { chat: CHAT.into(), members: ana_and_me(), web_url: None, first: true });
    (a, dir, o)
}

#[test]
fn a_call_opens_in_the_meetings_browsers_window() {
    let (mut a, dir, o) = calling("teams-call-app", &[("Google Chrome", "/opt/chrome")]);
    a.act(Action::Run("teams.call", json!({"video": false})));
    assert_eq!(opened(&o), [format!("app /opt/chrome {AUDIO}")]);
    let _ = std::fs::remove_dir_all(dir);
    let (mut a, dir, o) = calling("teams-call-default", &[]);
    a.act(Action::Run("teams.call", json!({"video": true})));
    assert_eq!(opened(&o), [format!("url {AUDIO}&withVideo=true")]);
    let _ = std::fs::remove_dir_all(dir);
}

/// H14: a page handed over as a call that isn't Teams' call link (another site, a meeting) opens
/// nowhere.
#[test]
fn a_page_that_isnt_a_call_link_opens_nowhere() {
    let (mut a, dir, o) = calling("teams-call-evil", &[("Google Chrome", "/opt/chrome")]);
    let meeting = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_x%40thread.v2/0?context=%7b%7d";
    for bad in ["https://evil.example/l/call/0/0?users=a%40b.example", meeting, "https://teams.microsoft.com/l/call/0/0?users=a%40b.example&x=1"] {
        a.open_teams_page(&TeamsPage::Call(bad.into()));
        assert!(opened(&o).is_empty(), "{bad}: {:?}", opened(&o));
    }
    let _ = std::fs::remove_dir_all(dir);
    let line =
        crate::app::page_line(&TeamsPage::Call("https://evil.example/l/call/0/0?users=a%40b.example".into()), &crate::app::PageOutcome::Refused);
    assert!(line.starts_with("teams: a page that isn't Teams' own was not opened (a call)"), "{line}");
}

#[test]
fn the_calls_line_in_the_log_names_no_one() {
    use crate::app::{PageOutcome, ProfileKind, page_line};
    let page = TeamsPage::Call("https://teams.microsoft.com/l/call/0/0?users=ana%40contoso.example,bruno%40contoso.example&withVideo=true".into());
    let shape = "teams.microsoft.com/l/call/0/0?users&withVideo";
    let ours = ProfileKind::of(Some(quatrelle_engine::MEETINGS_PROFILE));
    let lines = [
        page_line(&page, &PageOutcome::AppWindow { browser: "Google Chrome", profile: ours }),
        page_line(&page, &PageOutcome::DefaultBrowser),
        page_line(&page, &PageOutcome::Failed { error: "boom" }),
        page_line(&page, &PageOutcome::Refused),
    ];
    assert_eq!(lines[0], format!("teams: a call opened in Google Chrome, in Quatrelle's own profile: {shape}"));
    for line in &lines {
        for secret in ["%40", "@", "contoso", "ana", "bruno", "true"] {
            assert!(!line.contains(secret), "{secret} in {line}");
        }
        assert!(line.ends_with(shape), "{line}");
    }
}

/// The header with Call, light and dark, at the window's size and at a tall one.
#[test]
fn the_header_with_call_in_light_and_dark() {
    let row = with_ana(super::chat::group_row());
    let (mut a, dir, ctx) = called("teams-call-frames", row, ana_and_me());
    for dark in [false, true] {
        a.ui.dark = Some(dark);
        for size in [SIZE, TALL] {
            let texts = settled_at(&ctx, &mut a, size);
            assert!(has_text(&texts, "Call") && has_text(&texts, "Ana Exemple"), "{dark} {size:?}: {texts:?}");
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}
