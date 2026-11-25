// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the conversation pane. A chat read (days, senders in a row, the user's own on the
//! right, system lines, edited, deleted, unshowable, links, mentions, reactions, pictures, file
//! cards, cards), the chat Graph won't serve, "Open in Teams", the pictures' decoder and its
//! limits, the page before asked for at the top; and the list's photos, its people told as no
//! longer shown, its last hits kept while typing. With `QUATRELLE_FRAME_PNG` set to a folder, the
//! pane is painted there, light and dark.

use quatrelle_engine::model::pim::teams::{Attachment, Block, BlockKind, ChatMessage, ChatText, Member, Mention, MessageKind, Reaction, Run, Sender};

use super::*;
use crate::Action;

pub(super) const ME: &str = "00000000-0000-0000-0000-00000000000a";
pub(super) const TENANT_ID: &str = "11111111-2222-3333-4444-000000000001";
pub(super) const CHAT: &str = "19:pane@thread.v2";
pub(super) const TEAMS_LINK: &str = "https://teams.microsoft.com/l/chat/19%3Apane%40thread.v2/0?tenantId=11111111-2222-3333-4444-000000000001";
/// A tall window: the whole chat fits, nothing scrolled away.
pub(super) const TALL: (usize, usize) = (1400, 2000);

pub(super) fn member(id: &str, name: &str, tenant: &str) -> Member {
    Member { user_id: id.into(), name: name.into(), kind: "aadUserConversationMember".into(), tenant_id: Some(tenant.into()), ..Default::default() }
}

pub(super) fn members() -> Vec<Member> {
    vec![
        member(ME, "Riley Moreno", TENANT_ID),
        member("u-ana", "Ana Exemple", TENANT_ID),
        member("u-bruno", "Bruno Rossi", TENANT_ID),
        member("u-sam", "Sam Okafor", "99999999-0000-0000-0000-000000000009"),
    ]
}

pub(super) fn user(id: &str, name: &str) -> Sender {
    Sender::User { id: id.into(), name: name.into(), identity_type: Some("aadUser".into()), tenant_id: Some(TENANT_ID.into()) }
}

pub(super) fn words(text: &str) -> Run {
    Run { text: text.into(), ..Default::default() }
}

pub(super) fn para(runs: Vec<Run>) -> ChatText {
    ChatText { blocks: vec![Block { kind: BlockKind::Paragraph, runs }] }
}

/// A message `id` from `sender`, `secs` seconds since 1970, saying `body`.
pub(super) fn said(id: &str, sender: Sender, secs: i64, body: ChatText) -> ChatMessage {
    ChatMessage { id: id.into(), chat: CHAT.into(), sender, created: secs * 1000, modified: secs * 1000, body, ..Default::default() }
}

pub(super) fn group_row() -> ChatRow {
    ChatRow { name: "Support team".into(), members: 4, ..row(CHAT, 0) }
}

/// The app in Conversations with `row` open, its members, its link and `messages`; the drive's
/// host `contoso-my.sharepoint.com`.
pub(super) fn chat_app(name: &str, row: ChatRow, web_url: &str, messages: Vec<ChatMessage>) -> (MailApp, std::path::PathBuf, Shared<Job>) {
    let (mut a, dir, jobs) = module_app(name, vec![row.clone()]);
    a.session.teams_apply(teams::Event::Account {
        name: "Riley Moreno".into(),
        address: "riley@contoso.example".into(),
        object_id: ME.into(),
        tenant_id: TENANT_ID.into(),
        drive_host: Some("contoso-my.sharepoint.com".into()),
    });
    a.run("teams.open", json!({"chat": row.id}));
    a.session.teams_apply(teams::Event::Members { chat: row.id.clone(), members: members(), web_url: Some(web_url.into()), first: true });
    for part in messages.chunks(teams::EVENT_ITEMS) {
        a.session.teams_apply(teams::Event::Messages { chat: row.id.clone(), messages: part.to_vec(), fresh: false, older: true });
    }
    (a, dir, jobs)
}

/// A few frames of `size` for the pane to settle, then the texts of the last.
pub(super) fn settled_at(ctx: &egui::Context, a: &mut MailApp, size: (usize, usize)) -> Vec<(String, egui::Rect)> {
    let size = egui::vec2(size.0 as f32, size.1 as f32);
    for _ in 0..6 {
        frame_sized(ctx, a, vec![], size);
    }
    texts_of(&frame_shapes(ctx, a, vec![], size))
}

/// The pane alone, drawn in a frame of `size` with `events`: what it asked for, and what it drew.
/// (The fonts come from a frame of the whole window first.)
pub(super) fn pane(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>, size: (usize, usize)) -> (Vec<Action>, Vec<(String, egui::Rect)>) {
    let input = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0 as f32, size.1 as f32))),
        events,
        ..Default::default()
    };
    let mut acts = Vec::new();
    let mut out = ctx.run_ui(input, |ui| {
        egui::CentralPanel::default().show(ui, |ui| crate::panes::conversations::chat(&a.session, &mut a.ui, ui, &mut acts));
    });
    out.textures_delta.clear();
    let shapes: Vec<egui::Shape> = out.shapes.into_iter().map(|s| s.shape).collect();
    fn flat(s: egui::Shape, out: &mut Vec<egui::Shape>) {
        match s {
            egui::Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
            s => out.push(s),
        }
    }
    let mut all = Vec::new();
    for s in shapes {
        flat(s, &mut all);
    }
    (acts, texts_of(&all))
}

/// A click on the pane alone at `at`: what it asked for.
pub(super) fn click_pane(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2, size: (usize, usize)) -> Vec<Action> {
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    let mut acts = Vec::new();
    for events in [vec![Event::PointerMoved(at)], vec![press(true)], vec![press(false)], vec![]] {
        acts.extend(pane(ctx, a, events, size).0);
    }
    acts
}

pub(super) fn texts_with(texts: &[(String, egui::Rect)], part: &str) -> Vec<egui::Rect> {
    texts.iter().filter(|(t, _)| t.contains(part)).map(|(_, r)| *r).collect()
}

pub(super) fn picture_jobs(jobs: &Shared<Job>, f: impl Fn(&Picture) -> bool) -> usize {
    jobs.borrow().iter().filter(|j| matches!(j, Job::Teams { job: teams::Job::Picture { picture } } if f(picture))).count()
}

/// Every kind of content, light and dark: days, a sender's messages in a row under one name,
/// the user's own on the right, a system line, edited, deleted, unshowable, a link, a mention
/// of the user, a reaction, a picture kept with the message and one from elsewhere, a file
/// from another site, a card, an application's and an outsider's messages.
#[test]
fn a_conversation_with_every_kind_of_content() {
    let ana = || user("u-ana", "Ana Exemple");
    let (today, yesterday, before) = (NOW, NOW - 86_400, NOW - 3 * 86_400);
    let mut edited = said("m2", ana(), yesterday, para(vec![words("We moved the review")]));
    edited.edited = Some(yesterday * 1000 + 5);
    let mut link = said(
        "m3",
        ana(),
        yesterday + 60,
        para(vec![words("Read "), Run { text: "the plan".into(), link: Some("https://contoso.example/plan".into()), ..Default::default() }]),
    );
    link.reactions = vec![Reaction { emoji: "\u{1F44D}".into(), users: vec!["u-ana".into(), "u-bruno".into()] }];
    let mut system = said("m4", Sender::Nobody, today - 3600, para(vec![words("Riley renamed the conversation to \"Q3 planning\"")]));
    system.kind = MessageKind::System;
    let mut mention = said(
        "m5",
        ana(),
        today - 3000,
        para(vec![words("Thanks "), Run { text: "Riley".into(), mention: Some(0), ..Default::default() }, words("!")]),
    );
    mention.mentions = vec![Mention { id: 0, text: "Riley".into(), user_id: Some(ME.into()) }];
    let mut deleted = said("m6", ana(), today - 2900, para(vec![words("gone")]));
    deleted.deleted = Some(today * 1000);
    let mut unshowable = said("m7", ana(), today - 2800, ChatText::default());
    unshowable.unshowable = true;
    let mine = said("m8", user(ME, "Riley Moreno"), today - 2700, para(vec![words("I'll take it")]));
    let pictures = said(
        "m9",
        ana(),
        today - 2600,
        para(vec![
            words("Two pictures"),
            Run { hosted: Some("h1".into()), ..Default::default() },
            Run { outside: Some("https://images.example/cat.gif".into()), ..Default::default() },
        ]),
    );
    let mut file = said("m10", ana(), today - 2500, para(vec![words("The report")]));
    file.attachments =
        vec![Attachment::File { id: "f1".into(), name: "report.pdf".into(), url: "https://files.evil.example/report.pdf".into(), size: Some(2048) }];
    let mut card = said("m11", ana(), today - 2400, ChatText::default());
    card.attachments = vec![Attachment::Card];
    let app = said("m12", Sender::App { id: "app-1".into(), name: "Build bot".into() }, today - 2300, para(vec![words("Build passed")]));
    let blocks = ChatText {
        blocks: vec![
            Block { kind: BlockKind::Quote, runs: vec![words("Quoted words")] },
            Block { kind: BlockKind::Code, runs: vec![words("let x = 1;")] },
            Block { kind: BlockKind::Item { ordered: true, depth: 1 }, runs: vec![words("First step")] },
        ],
    };
    let blocks = said("m14", ana(), today - 2100, blocks);
    let outsider = said(
        "m13",
        Sender::User { id: "u-zoe".into(), name: "Zoe Stranger".into(), identity_type: None, tenant_id: None },
        today - 2200,
        para(vec![words("Hello from outside")]),
    );
    let messages = vec![
        said("m1", ana(), before, para(vec![words("Plans for the week")])),
        edited,
        link,
        system,
        mention,
        deleted,
        unshowable,
        mine,
        pictures,
        file,
        card,
        app,
        outsider,
        blocks,
    ];
    let (mut a, dir, jobs) = chat_app("teams-chat-every", group_row(), TEAMS_LINK, messages);
    let ctx = egui::Context::default();
    let pictures_dir = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let mut tex = Textures::default();
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        a.ui.dark = Some(dark);
        for _ in 0..6 {
            frame_with_size(&ctx, &mut a, &mut tex, vec![], None, TALL);
        }
        let png = pictures_dir.as_ref().map(|d| d.join(format!("teams-chat-every-{theme}.png")));
        let (_, shapes) = frame_with_size(&ctx, &mut a, &mut tex, vec![], png, TALL);
        let texts = texts_of(&shapes);
        let day = quatrelle_engine::model::dates::day_label(a.session.clock.local_day(before));
        for w in [
            "Support team",
            "Today",
            "Yesterday",
            day.as_str(),
            "(edited)",
            "Riley renamed the conversation to \"Q3 planning\"",
            "This message was deleted",
            "This message can't be shown",
            "\u{1F44D} 2",
            "Picture",
            "Show picture",
            "Link to another site",
            "This content can be seen in Teams",
            "Open in Teams",
            "Build bot",
            "App",
            "Zoe Stranger",
            "Members: 4",
            "Quoted words",
            "let x = 1;",
            "1. First step",
        ] {
            assert!(has_text(&texts, w), "{theme}: {w}: {texts:?}");
        }
        assert!(!texts_with(&texts, "Read the plan").is_empty(), "{theme}: the link's text");
        assert!(!texts_with(&texts, "Thanks Riley Moreno!").is_empty(), "{theme}: the mention by the member's name: {texts:?}");
        assert!(!texts_with(&texts, "files.evil.example").is_empty(), "{theme}: the file's host on its card");
        // Ana's messages in five rows (a day, the next, then around the user's own, the app's
        // and the outsider's): her name once for each, not for each of her 11 messages.
        assert_eq!(texts.iter().filter(|(t, _)| t == "Ana Exemple").count(), 5, "{theme}: one name per row of messages: {texts:?}");
        // The user's own on the right, others' on the left.
        let own = rect_of(&texts, |t| t == "I'll take it").expect("own message");
        let theirs = rect_of(&texts, |t| t == "Plans for the week").expect("Ana's message");
        assert!(own.left() > theirs.right() && own.right() > 1000.0, "{theme}: own {own:?}, theirs {theirs:?}");
        // "External" before the outsider's name, on its line.
        let zoe = rect_of(&texts, |t| t == "Zoe Stranger").expect("the outsider's name");
        let external = texts.iter().filter(|(t, _)| t == "External").map(|(_, r)| *r).find(|r| (r.center().y - zoe.center().y).abs() < 8.0);
        assert!(external.is_some_and(|r| r.right() <= zoe.left()), "{theme}: External first: {external:?} {zoe:?}");
    }
    // The picture kept with the message was asked for once; the one from elsewhere wasn't.
    assert_eq!(picture_jobs(&jobs, |p| matches!(p, Picture::Hosted { hosted, .. } if hosted == "h1")), 1);
    assert_eq!(picture_jobs(&jobs, |p| matches!(p, Picture::Outside { .. })), 0);
    let _ = std::fs::remove_dir_all(dir);
}

/// A picture from elsewhere is asked for only once the user clicks "Show picture": drawing and
/// scrolling never do.
#[test]
fn an_outside_image_is_asked_only_on_a_click() {
    let url = "https://images.example/cat.gif";
    let m = said("m1", user("u-ana", "Ana Exemple"), NOW, para(vec![words("Look"), Run { outside: Some(url.into()), ..Default::default() }]));
    let (mut a, dir, jobs) = chat_app("teams-chat-outside", group_row(), TEAMS_LINK, vec![m]);
    let ctx = egui::Context::default();
    let texts = settled_at(&ctx, &mut a, (1280, 800));
    let wheel = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 300.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Trackpad,
        modifiers: Modifiers::NONE,
    };
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(egui::pos2(900.0, 400.0)), wheel], egui::vec2(1280.0, 800.0));
    let outside = |jobs: &Shared<Job>| picture_jobs(jobs, |p| matches!(p, Picture::Outside { url: u } if u == url));
    assert_eq!(outside(&jobs), 0, "not by drawing or scrolling");
    let button = rect_of(&texts, |t| t == "Show picture").expect("the button");
    click_at(&ctx, &mut a, button.center(), 1, egui::vec2(1280.0, 800.0));
    assert_eq!(outside(&jobs), 1, "once clicked");
    let _ = std::fs::remove_dir_all(dir);
}

/// "Open in Teams" is drawn only for a link to a Teams chat, and asks for it through
/// `teams.openInTeams` (which opens it in the meetings browser's window), never as a link.
#[test]
fn open_in_teams_needs_a_teams_chat_link() {
    let mut card = said("m1", user("u-ana", "Ana Exemple"), NOW, ChatText::default());
    card.attachments = vec![Attachment::Card];
    let ctx = egui::Context::default();
    let size = (1280, 800);
    let (mut a, dir, _) = chat_app("teams-chat-link-evil", group_row(), "https://evil.example/l/chat/x", vec![card.clone()]);
    settled_at(&ctx, &mut a, size);
    let (_, texts) = pane(&ctx, &mut a, vec![], size);
    assert!(has_text(&texts, "This content can be seen in Teams"), "{texts:?}");
    assert!(!has_text(&texts, "Open in Teams"), "not for another site: {texts:?}");
    let _ = std::fs::remove_dir_all(dir);

    let (mut a, dir, _) = chat_app("teams-chat-link-teams", group_row(), TEAMS_LINK, vec![card]);
    settled_at(&ctx, &mut a, size);
    let (_, texts) = pane(&ctx, &mut a, vec![], size);
    // The card's button, below the header's.
    let button = texts.iter().filter(|(t, _)| t == "Open in Teams").map(|(_, r)| *r).max_by(|a, b| a.top().total_cmp(&b.top()));
    let acts = click_pane(&ctx, &mut a, button.expect("the button for a Teams chat").center(), size);
    assert!(acts.iter().any(|x| matches!(x, Action::Run("teams.openInTeams", _))), "teams.openInTeams asked for");
    assert!(!acts.iter().any(|x| matches!(x, Action::OpenUrl(_))), "never opened as a link");
    let _ = std::fs::remove_dir_all(dir);
}

/// Every header has "Open in Teams" when the chat's link is a Teams chat link, a readable group
/// chat's too, beside the name; a click asks for it through `teams.openInTeams`. A link elsewhere
/// has none. A chat Graph won't serve whose link the worker built (decision 21) has it in its
/// header and its pane, and the engine opens that link.
#[test]
fn every_header_has_open_in_teams_when_its_link_is_one() {
    let ctx = egui::Context::default();
    let size = (1280, 800);
    let hello = said("m1", user("u-ana", "Ana Exemple"), NOW, para(vec![words("Hello")]));
    let (mut a, dir, _) = chat_app("teams-header-link", group_row(), TEAMS_LINK, vec![hello.clone()]);
    settled_at(&ctx, &mut a, size);
    let (_, texts) = pane(&ctx, &mut a, vec![], size);
    let buttons: Vec<egui::Rect> = texts.iter().filter(|(t, _)| t == "Open in Teams").map(|(_, r)| *r).collect();
    let name = rect_of(&texts, |t| t == "Support team").expect("the name");
    assert_eq!(buttons.len(), 1, "{texts:?}");
    assert!((buttons[0].center().y - name.center().y).abs() < 12.0, "beside the name: {buttons:?} {name:?}");
    let acts = click_pane(&ctx, &mut a, buttons[0].center(), size);
    assert!(acts.iter().any(|x| matches!(x, Action::Run("teams.openInTeams", _))), "teams.openInTeams asked for");
    assert!(!acts.iter().any(|x| matches!(x, Action::OpenUrl(_))), "never opened as a link");
    let _ = std::fs::remove_dir_all(dir);
    // A link elsewhere: none.
    let (mut a, dir, _) = chat_app("teams-header-evil", group_row(), "https://evil.example/l/chat/x", vec![hello]);
    settled_at(&ctx, &mut a, size);
    let (_, texts) = pane(&ctx, &mut a, vec![], size);
    assert!(!has_text(&texts, "Open in Teams"), "{texts:?}");
    let _ = std::fs::remove_dir_all(dir);
    // A chat Graph won't serve, its link built by the worker.
    let built = "https://teams.microsoft.com/l/chat/19%3Apane%40thread.v2/conversations?context=%7B%22contextType%22%3A%22chat%22%7D&tenantId=11111111-2222-3333-4444-000000000001";
    let unknown = ChatRow { kind: ChatKind::Other { name: "unknownFutureValue".into() }, unreadable: true, ..group_row() };
    let (mut a, dir, _) = chat_app("teams-header-built", unknown, built, vec![]);
    settled_at(&ctx, &mut a, size);
    let (_, texts) = pane(&ctx, &mut a, vec![], size);
    let buttons: Vec<egui::Rect> = texts.iter().filter(|(t, _)| t == "Open in Teams").map(|(_, r)| *r).collect();
    assert_eq!(buttons.len(), 2, "in the header and the pane: {texts:?}");
    let acts = click_pane(&ctx, &mut a, buttons[1].center(), size);
    let asked: Vec<(&str, serde_json::Value)> = acts
        .into_iter()
        .filter_map(|x| match x {
            Action::Run(id @ "teams.openInTeams", p) => Some((id, p)),
            _ => None,
        })
        .collect();
    assert_eq!(asked.len(), 1);
    for (id, p) in asked {
        a.session.run(id, &p).unwrap();
    }
    assert_eq!(a.session.take_teams_page(), Some(quatrelle_engine::TeamsPage::Chat(built.into())));
    let _ = std::fs::remove_dir_all(dir);
}

/// A file card writes its link's host: the user's OneDrive's or their SharePoint's is theirs;
/// any other says "Link to another site".
#[test]
fn a_file_card_names_its_host_and_says_another_site() {
    let file = |id: &str, url: &str| {
        let mut m = said(id, user("u-ana", "Ana Exemple"), NOW, para(vec![words("A file")]));
        m.attachments = vec![Attachment::File { id: format!("f-{id}"), name: "notes.docx".into(), url: url.into(), size: Some(12 * 1024) }];
        m
    };
    let ctx = egui::Context::default();
    for (url, host, another) in [
        ("https://contoso-my.sharepoint.com/personal/riley/notes.docx", "contoso-my.sharepoint.com", false),
        ("https://contoso.sharepoint.com/sites/team/notes.docx", "contoso.sharepoint.com", false),
        ("https://files.evil.example/notes.docx", "files.evil.example", true),
        // A host that would read as the user's once its bidi character is set aside isn't theirs.
        ("https://contoso-my\u{202E}.sharepoint.com/notes.docx", "contoso-my.sharepoint.com", true),
    ] {
        let (mut a, dir, _) = chat_app("teams-chat-file", group_row(), TEAMS_LINK, vec![file("m1", url)]);
        let texts = settled_at(&ctx, &mut a, (1280, 800));
        assert!(!texts_with(&texts, host).is_empty(), "{host} on the card: {texts:?}");
        assert_eq!(has_text(&texts, "Link to another site"), another, "{host}: {texts:?}");
        // The bidi one isn't handed to a browser either.
        assert_eq!(has_text(&texts, "Open in browser"), !url.contains('\u{202E}'), "{texts:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// A chat Graph won't serve: the sentence, the last line the list gave (its sender External),
/// and "Open in Teams", light and dark.
#[test]
fn the_unreadable_chat_pane() {
    let mut r = group_row();
    r.unreadable = true;
    r.last_external = true;
    r.last_line = "Sam: the files are ready".into();
    let (mut a, dir, _) = chat_app("teams-chat-unreadable", r, TEAMS_LINK, vec![]);
    let ctx = egui::Context::default();
    let pictures_dir = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let mut tex = Textures::default();
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        a.ui.dark = Some(dark);
        for _ in 0..6 {
            frame_with_size(&ctx, &mut a, &mut tex, vec![], None, (1280, 800));
        }
        let png = pictures_dir.as_ref().map(|d| d.join(format!("teams-chat-unreadable-{theme}.png")));
        let (_, shapes) = frame_with_size(&ctx, &mut a, &mut tex, vec![], png, (1280, 800));
        let texts = texts_of(&shapes);
        for w in ["Teams doesn't let this conversation be read here", "Sam: the files are ready", "Open in Teams", "Support team"] {
            assert!(has_text(&texts, w), "{theme}: {w}: {texts:?}");
        }
        let line = rect_of(&texts, |t| t == "Sam: the files are ready").unwrap();
        assert!(
            texts.iter().any(|(t, r)| t == "External" && (r.center().y - line.center().y).abs() < 8.0 && r.right() <= line.left()),
            "{theme}: External before the line"
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Scrolled to the top, the page before is asked for once; when it comes, the view stays where it
/// was, and the next page is asked for only at the top again.
#[test]
fn scrolling_up_asks_for_the_older_page_once() {
    let page = |from: i64, to: i64| -> Vec<ChatMessage> {
        (from..to)
            .map(|i| {
                said(
                    &format!("m{i:03}"),
                    user(if i % 2 == 0 { "u-ana" } else { ME }, "Ana Exemple"),
                    NOW - 7200 + i * 30,
                    para(vec![words(&format!("Line {i}"))]),
                )
            })
            .collect()
    };
    let (mut a, dir, jobs) = chat_app("teams-chat-older", group_row(), TEAMS_LINK, page(100, 180));
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    let texts = settled_at(&ctx, &mut a, (1280, 800));
    assert!(rect_of(&texts, |t| t == "Line 179").is_some_and(|r| r.bottom() < 800.0 && r.top() > 0.0), "the newest shown first");
    let pages = |jobs: &Shared<Job>| jobs.borrow().iter().filter(|j| matches!(j, Job::Teams { job: teams::Job::Page { .. } })).count();
    assert_eq!(pages(&jobs), 0, "not at opening");
    let over = egui::pos2(900.0, 400.0);
    // The wheel turned towards the top, a notch a frame, until it's there.
    let to_the_top = |a: &mut MailApp| {
        let notch = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 1500.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: Modifiers::NONE,
        };
        frame_sized(&ctx, a, vec![Event::PointerMoved(over)], size);
        for _ in 0..12 {
            frame_sized(&ctx, a, vec![Event::PointerMoved(over), notch.clone()], size);
        }
        for _ in 0..20 {
            frame_sized(&ctx, a, vec![Event::PointerMoved(over)], size);
        }
        texts_of(&frame_shapes(&ctx, a, vec![], size))
    };
    let on_screen = |texts: &[(String, egui::Rect)], line: &str| rect_of(texts, |t| t == line).filter(|r| r.top() > 0.0 && r.bottom() < 800.0);
    let texts = to_the_top(&mut a);
    assert!(on_screen(&texts, "Line 100").is_some(), "at the top: {texts:?}");
    assert_eq!(pages(&jobs), 1, "the page before, once");
    let texts = to_the_top(&mut a);
    assert_eq!(pages(&jobs), 1, "still once at the top");
    // It comes in above: what was on screen stays there, and nothing more is asked.
    let first = on_screen(&texts, "Line 101").expect("on screen");
    let older = page(40, 100);
    for part in older.chunks(teams::EVENT_ITEMS) {
        a.session.teams_apply(teams::Event::Messages { chat: CHAT.into(), messages: part.to_vec(), fresh: false, older: true });
    }
    for _ in 0..4 {
        frame_sized(&ctx, &mut a, vec![], size);
    }
    let texts = texts_of(&frame_shapes(&ctx, &mut a, vec![], size));
    let still = rect_of(&texts, |t| t == "Line 101").expect("still drawn");
    assert!((still.top() - first.top()).abs() < 4.0, "the view stays: {first:?} → {still:?}");
    assert_eq!(pages(&jobs), 1, "not again until the top");
    let texts = to_the_top(&mut a);
    assert!(on_screen(&texts, "Line 40").is_some(), "at the new top");
    assert_eq!(pages(&jobs), 2, "the next page, at the top again");
    let _ = std::fs::remove_dir_all(dir);
}

/// The list not drawn (another module, the pane minimized): the worker is told no one is shown,
/// once; drawn again, the people on it are told again.
#[test]
fn the_list_tells_no_one_is_shown_when_it_is_not_drawn() {
    let rows = vec![one_to_one("19:p@thread.v2", "Paul Durand", "u-paul", NOW)];
    let (mut a, dir, jobs) = module_app("teams-list-unseen", rows);
    let ctx = egui::Context::default();
    let visible = |jobs: &Shared<Job>| -> Vec<Vec<String>> {
        jobs.borrow()
            .iter()
            .filter_map(|j| match j {
                Job::Teams { job: teams::Job::Visible { users } } => Some(users.clone()),
                _ => None,
            })
            .collect()
    };
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(visible(&jobs), vec![vec!["u-paul".to_string()]]);
    a.session.module = quatrelle_engine::Module::Mail;
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(visible(&jobs), vec![vec!["u-paul".to_string()], vec![]], "no one, once");
    a.session.module = quatrelle_engine::Module::Conversations;
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(visible(&jobs).last(), Some(&vec!["u-paul".to_string()]), "shown again");
    a.run("view.folderPane", json!({"minimized": true}));
    for _ in 0..20 {
        frame(&ctx, &mut a, vec![]);
    }
    let told = visible(&jobs).len();
    assert_eq!(visible(&jobs).last(), Some(&vec![]), "minimized: no one");
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(visible(&jobs).len(), told, "once");
    a.run("view.folderPane", json!({"minimized": false}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(visible(&jobs).last(), Some(&vec!["u-paul".to_string()]), "shown again");
    let _ = std::fs::remove_dir_all(dir);
}

/// While the next query's answer is on its way, the last hits stay: no empty list and no
/// "Searching…" at each key.
#[test]
fn the_last_hits_stay_while_typing() {
    let rows = vec![row("19:a@thread.v2", 0), ChatRow { at: Some(NOW - 5), ..row("19:b@thread.v2", 0) }];
    let (mut a, dir, _) = module_app("teams-list-typing", rows);
    let ctx = egui::Context::default();
    a.ui.conversations.query = "b".into();
    a.run("teams.search", json!({"query": "b"}));
    a.session.teams_apply(teams::Event::Found {
        query: "b".into(),
        hits: vec![("19:b@thread.v2".into(), String::new())],
        indexing: false,
        partial: false,
    });
    let texts = settled(&ctx, &mut a);
    assert!(has_text(&texts, "Chat 19:b@thread.v2") && !has_text(&texts, "Chat 19:a@thread.v2"), "{texts:?}");
    a.ui.conversations.query = "bo".into();
    a.run("teams.search", json!({"query": "bo"}));
    let texts = settled(&ctx, &mut a);
    assert!(has_text(&texts, "Chat 19:b@thread.v2"), "the last hits stay: {texts:?}");
    assert!(!has_text(&texts, "Searching…"), "{texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A person's presence is said in every language, as a presence: not as the calendar's "Busy"
/// or the connection's "Offline".
#[test]
fn a_presence_is_said_in_every_language() {
    for lang in crate::i18n::Lang::all().skip(1) {
        crate::i18n::set_current(lang);
        // These have no row of their own outside presence: untranslated, they'd be English.
        for word in ["Available", "Away", "Do not disturb"] {
            assert_ne!(crate::i18n::tc("presence", word), word, "{}", lang.code());
        }
    }
    let ja = crate::i18n::Lang::from_code("ja").unwrap();
    crate::i18n::set_current(ja);
    assert_ne!(crate::i18n::tc("presence", "Busy"), crate::i18n::tr(ja, "Busy"), "ja: not the calendar's busy");
    let fr = crate::i18n::Lang::from_code("fr").unwrap();
    crate::i18n::set_current(fr);
    assert_eq!(crate::i18n::tc("presence", "Offline"), "Hors ligne", "fr: a person, not the connection");
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

/// A one-to-one chat's header says the other person's presence.
#[test]
fn the_header_says_the_other_persons_presence() {
    let r = one_to_one(CHAT, "Ana Exemple", "u-ana", NOW);
    let (mut a, dir, _) = chat_app("teams-chat-presence", r, TEAMS_LINK, vec![]);
    a.session.teams_apply(teams::Event::Presence { presences: vec![("u-ana".into(), teams::Presence::DoNotDisturb)] });
    let ctx = egui::Context::default();
    let texts = settled_at(&ctx, &mut a, (1280, 800));
    assert!(has_text(&texts, "Do not disturb"), "{texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A chat of 5,000 members whose messages each carry 100 reactions of 200 people (none of them
/// a member: each looked for in vain) and 50 mentions: a frame of the pane never goes through
/// the whole list of members for a person. It takes no more than four times a frame of the same
/// messages in a chat of four, and 2 ms (each in a window of its own, their frames in turn, the
/// quickest of five each: a pause of the system only adds to a frame, a slower pane slows them
/// all), and under 2 s.
#[test]
fn a_chat_of_5000_members_and_full_reactions_draws_in_time() {
    let id = |i: usize| format!("u-{i:04}");
    let busy = |n: usize| {
        let runs: Vec<Run> = (0..50).map(|k| Run { text: "someone".into(), mention: Some(k), ..Default::default() }).collect();
        let mut m = said(&format!("m{n}"), user(&id(4999 - n), "Sender"), NOW - 600 + n as i64, para(runs));
        m.mentions = (0..50).map(|k| Mention { id: k, text: "someone".into(), user_id: Some(id(4950 + k as usize)) }).collect();
        m.reactions = (0..teams::MAX_REACTIONS)
            .map(|r| Reaction {
                emoji: char::from_u32(0x1F600 + r as u32).unwrap_or('x').to_string(),
                users: (0..200).map(|k| format!("x-{k:04}")).collect(),
            })
            .collect();
        m
    };
    let (mut few, few_dir, _) = chat_app("teams-chat-few", group_row(), TEAMS_LINK, (0..4).map(busy).collect());
    let (mut a, dir, _) = chat_app("teams-chat-crowd", group_row(), TEAMS_LINK, (0..4).map(busy).collect());
    // The 5,000 members as the worker says them, through the window's queue.
    let mut events: Vec<MailEvent> = (0..teams::MAX_MEMBERS)
        .step_by(teams::EVENT_ITEMS)
        .map(|start| {
            let part: Vec<Member> = (start..start + teams::EVENT_ITEMS).map(|i| member(&id(i), &format!("Person {i}"), TENANT_ID)).collect();
            MailEvent::Teams {
                event: teams::Event::Members { chat: CHAT.into(), members: part, web_url: Some(TEAMS_LINK.into()), first: start == 0 },
            }
        })
        .collect();
    a.ui.teams_queue.take(&mut events);
    while a.ui.teams_queue.apply(&mut a.session, crate::conversations::TEAMS_EVENTS_A_FRAME) {}
    assert_eq!(a.session.teams.members().len(), teams::MAX_MEMBERS);
    let (few_ctx, ctx) = (egui::Context::default(), egui::Context::default());
    settled_at(&few_ctx, &mut few, TALL);
    settled_at(&ctx, &mut a, TALL);
    let timed = |ctx: &egui::Context, a: &mut MailApp| {
        let started = std::time::Instant::now();
        pane(ctx, a, vec![], TALL);
        started.elapsed()
    };
    let (mut small, mut crowd) = (std::time::Duration::MAX, std::time::Duration::MAX);
    for _ in 0..5 {
        small = small.min(timed(&few_ctx, &mut few));
        crowd = crowd.min(timed(&ctx, &mut a));
    }
    let (_, texts) = pane(&ctx, &mut a, vec![], TALL);
    assert!(has_text(&texts, "Person 4996"), "the sender by the member's name: {texts:?}");
    assert!(texts.iter().any(|(t, _)| t.contains("Person 4950")), "a mention by the member's name");
    eprintln!("teams crowd: a frame of 5,000 members {crowd:?}, of four {small:?}");
    let margin = std::time::Duration::from_millis(2);
    assert!(crowd <= 4 * small + margin, "a frame of 5,000 members took {crowd:?}, of four {small:?}");
    assert!(crowd < std::time::Duration::from_secs(2), "a frame took {crowd:?}");
    let _ = std::fs::remove_dir_all(few_dir);
    let _ = std::fs::remove_dir_all(dir);
}

/// A member is known by their id whatever its capitals: a sender whose id Graph wrote in other
/// capitals is shown by the member's name, and as of the organisation, not as an outsider.
#[test]
fn a_sender_is_the_member_whatever_the_ids_capitals() {
    let m = said("m1", user("U-ANA", "Someone Else"), NOW, para(vec![words("Hello")]));
    let m = ChatMessage { sender: Sender::User { id: "U-ANA".into(), name: "Someone Else".into(), identity_type: None, tenant_id: None }, ..m };
    let (mut a, dir, _) = chat_app("teams-chat-capitals", group_row(), TEAMS_LINK, vec![m]);
    let ctx = egui::Context::default();
    let texts = settled_at(&ctx, &mut a, TALL);
    assert!(has_text(&texts, "Ana Exemple"), "{texts:?}");
    assert!(!has_text(&texts, "Someone Else"), "{texts:?}");
    assert!(!has_text(&texts, "External"), "a member of the organisation: {texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Security review 1.5 (content n° 1): a sender the chat's members don't have, whose message says
/// they're an `aadUser` of the organisation (a guest who left says so too), is External before
/// their name.
#[test]
fn a_sender_the_members_dont_have_is_external_on_their_message() {
    let m = said("m1", user("u-gale", "Gale Stone"), NOW, para(vec![words("Hello again")]));
    let (mut a, dir, _) = chat_app("teams-chat-gone", group_row(), TEAMS_LINK, vec![m]);
    let ctx = egui::Context::default();
    let texts = settled_at(&ctx, &mut a, TALL);
    let gale = rect_of(&texts, |t| t == "Gale Stone").expect("the sender's name");
    let external = texts.iter().filter(|(t, _)| t == "External").map(|(_, r)| *r).find(|r| (r.center().y - gale.center().y).abs() < 8.0);
    assert!(external.is_some_and(|r| r.right() <= gale.left()), "External before the name: {external:?} {gale:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Security review 1.5 (content n° 6): thirty messages in a row from a member of another
/// organisation are each tagged External, not only the first under the name: one scrolled away,
/// the others still say who is outside.
#[test]
fn external_is_on_each_message_of_a_row() {
    let sam = || user("u-sam", "Sam Okafor");
    let row: Vec<ChatMessage> =
        (0..30).map(|k| said(&format!("m{k:02}"), sam(), NOW - 1800 + k * 30, para(vec![words(&format!("Note {k}"))]))).collect();
    let (mut a, dir, _) = chat_app("teams-chat-external-row", group_row(), TEAMS_LINK, row);
    let ctx = egui::Context::default();
    let texts = settled_at(&ctx, &mut a, TALL);
    assert_eq!(texts.iter().filter(|(t, _)| t == "Sam Okafor").count(), 1, "one name for the row: {texts:?}");
    let external: Vec<egui::Rect> = texts.iter().filter(|(t, _)| t == "External").map(|(_, r)| *r).collect();
    for k in 1..30 {
        let note = rect_of(&texts, |t| t == format!("Note {k}")).expect("the message");
        assert!(external.iter().any(|r| r.center().y > note.top() - 6.0 && r.center().y < note.bottom() + 6.0), "Note {k} tagged: {external:?}");
    }
    assert_eq!(external.len(), 30, "the first under the name, the others beside their time");
    if let Some(d) = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from) {
        let mut tex = Textures::default();
        for (dark, theme) in [(false, "light"), (true, "dark")] {
            a.ui.dark = Some(dark);
            for _ in 0..6 {
                frame_with_size(&ctx, &mut a, &mut tex, vec![], None, (1280, 800));
            }
            frame_with_size(&ctx, &mut a, &mut tex, vec![], Some(d.join(format!("teams-chat-external-row-{theme}.png"))), (1280, 800));
        }
        a.ui.dark = Some(false);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// A chat the user is no longer in says so under its header, not only in the list's tooltip.
#[test]
fn a_left_chat_says_so_under_its_header() {
    let left = ChatRow { left: true, ..group_row() };
    let (mut a, dir, _) =
        chat_app("teams-chat-left", left, TEAMS_LINK, vec![said("m1", user("u-ana", "Ana Exemple"), NOW, para(vec![words("Bye")]))]);
    let ctx = egui::Context::default();
    let (_, texts) = {
        settled_at(&ctx, &mut a, TALL);
        pane(&ctx, &mut a, vec![], TALL)
    };
    let header = rect_of(&texts, |t| t == "Support team").expect("the header");
    let said = rect_of(&texts, |t| t == "You're no longer in this conversation").expect("said under the header");
    assert!(said.top() >= header.bottom(), "{said:?} under {header:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Re-review n° 4: in the narrowest pane (332 points: the window at its narrowest, 760, and the
/// list at its widest, 380) and at the default width, an outsider's long messages leave room for
/// "External" and the time beside them: both stay whole in the pane, beside a picture's place and
/// beside its thumbnail too (round 3: a thumbnail is as wide as the bubble at most). With
/// `QUATRELLE_FRAME_PNG` set, both widths are painted, the picture waited for and come.
#[test]
fn external_and_the_time_stay_in_the_narrowest_pane() {
    let sam = || user("u-sam", "Sam Okafor");
    let long = "We moved the review to Thursday, and the figures for the third quarter are in the shared folder now";
    let mut row: Vec<ChatMessage> = (0..6)
        .map(|k| said(&format!("m{k:02}"), sam(), NOW - 1800 + k * 30, para(vec![words(if k % 2 == 0 { long } else { "Short note" })])))
        .collect();
    row.push(said("m06", sam(), NOW - 1600, para(vec![words("The plan"), Run { hosted: Some("h1".into()), ..Default::default() }])));
    let (mut a, dir, _) = chat_app("teams-chat-narrowest", group_row(), TEAMS_LINK, row);
    let ctx = egui::Context::default();
    let pictures = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let mut tex = Textures::default();
    // The list at its default 260 points: a window of 640 leaves the pane the narrowest's 332. The
    // picture's place first, then its thumbnail (a wide picture: 240 points if it may).
    let sizes = [(640usize, 332.0, "waiting"), (760, 452.0, "waiting"), (640, 332.0, "come"), (760, 452.0, "come")];
    // The picture's message's time: drawn (one pushed out of the pane wouldn't be) and in the pane.
    let at = quatrelle_engine::model::dates::civil(NOW - 1600, &a.session.clock);
    let its_time = format!("{:02}:{:02}", at.hour, at.minute);
    for (i, (w, pane, picture)) in sizes.into_iter().enumerate() {
        if i == 2 {
            let hosted = Picture::Hosted { chat: CHAT.into(), message: "m06".into(), hosted: "h1".into() };
            a.session.teams_apply(teams::Event::Picture { picture: hosted.clone(), bytes: super::pictures::png(600, 300) });
            super::pictures::decoded(&ctx, &mut a);
            assert!(matches!(a.ui.teams_queue.pictures.image(&hosted), Some(crate::panes::conversations::Shown::Ready(_))), "decoded");
        }
        for _ in 0..8 {
            frame_with_size(&ctx, &mut a, &mut tex, vec![], None, (w, 480));
        }
        let png = pictures.as_ref().map(|d| d.join(format!("teams-chat-narrowest-{pane}-{picture}.png")));
        let (_, shapes) = frame_with_size(&ctx, &mut a, &mut tex, vec![], png, (w, 480));
        let texts = texts_of(&shapes);
        let is_time = |t: &str| t.len() == 5 && t.as_bytes().get(2) == Some(&b':');
        let beside: Vec<&(String, egui::Rect)> = texts.iter().filter(|(t, r)| (t == "External" || is_time(t)) && r.left() > 300.0).collect();
        assert!(beside.iter().any(|(t, _)| t == "External") && beside.iter().any(|(t, _)| *t == its_time), "{pane}, picture {picture}: {texts:?}");
        for (t, r) in beside {
            assert!(r.right() <= w as f32 - 1.0, "{pane}-point pane, picture {picture}: {t:?} out of it at {r:?}");
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}
