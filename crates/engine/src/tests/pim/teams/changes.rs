// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: Teams, changes to messages already there: a reaction, an edit and a deletion
//! shown at once over the window, then as the worker keeps them; what may be edited, deleted and
//! reacted to; mentions sent; the notices when Teams doesn't take a change.

use quatrelle_model::pim::teams::{
    Attachment, Block, BlockKind, ChatText, Member, Mention, MessageKind, Op, OutKind, OutState, Outgoing, Reaction, Run, Sender, Unsent,
};

use super::*;
use crate::TEAMS_NOTHING_LEFT;
use crate::mime::teams_compose;

const CHAT: &str = "19:chat1@thread.v2";
const OTHER: &str = "19:chat2@thread.v2";
const ME: &str = "u-me";
const TENANT_ID: &str = "tenant-1";
const THUMBS: &str = "\u{1F44D}";

fn person(id: &str, name: &str, tenant: &str) -> Member {
    Member { user_id: id.into(), name: name.into(), kind: "aadUserConversationMember".into(), tenant_id: Some(tenant.into()), ..Default::default() }
}

fn from(id: &str) -> Sender {
    Sender::User { id: id.into(), name: String::new(), identity_type: Some("aadUser".into()), tenant_id: Some(TENANT_ID.into()) }
}

/// Ana's message, with her own 👍.
fn m1() -> ChatMessage {
    let reactions = vec![Reaction { emoji: THUMBS.into(), users: vec!["u-ana".into()] }];
    ChatMessage { sender: from("u-ana"), body: teams_compose("hi there", &[], &[]), reactions, ..msg(CHAT, 1) }
}

/// The user's "hello", "hel" in bold.
fn m2() -> ChatMessage {
    ChatMessage { sender: from(ME), body: teams_compose("hello", std::slice::from_ref(&(0..3)), &[]), ..msg(CHAT, 2) }
}

/// Another message of the user's, numbered `i`.
fn mine(i: i64) -> ChatMessage {
    ChatMessage { sender: from(ME), body: teams_compose(&format!("mine {i}"), &[], &[]), ..msg(CHAT, i) }
}

fn window(s: &mut Session, messages: Vec<ChatMessage>) {
    s.apply(teams_event(teams::Event::Messages { chat: CHAT.into(), messages, fresh: false, older: false }));
}

/// Chat 1 open and on screen, the user `u-me` of `tenant-1`; its members Ana (`tenant-1`), Sam
/// (`tenant-9`) and the user; its window `m1` and `m2`; its jobs taken.
fn opened() -> Session {
    let mut s = signed_in(vec![row(1, 10), row(2, 20)]);
    s.prefs.teams.object_id = ME.into();
    s.prefs.teams.tenant_id = TENANT_ID.into();
    s.module = Module::Conversations;
    open(&mut s, CHAT);
    let members = vec![person("u-ana", "Ana Exemple", TENANT_ID), person("u-sam", "Sam Roe", "tenant-9"), person(ME, "Riley", TENANT_ID)];
    s.apply(teams_event(teams::Event::Members { chat: CHAT.into(), members, web_url: None, first: true }));
    window(&mut s, vec![m1(), m2()]);
    s.take_jobs();
    s
}

fn outbox(chat: &str, items: Vec<Outgoing>, took: Option<&str>) -> Event {
    teams_event(teams::Event::Outbox { chat: chat.into(), items, took: took.map(str::to_string) })
}

fn refused() -> OutState {
    OutState::NotSent { reason: Unsent::Refused, code: Some("HTTP 403 Forbidden".into()) }
}

/// A change going out of `chat`, numbered `seq`.
fn change(chat: &str, seq: u64, local: &str, what: OutKind, state: OutState) -> Outgoing {
    Outgoing { seq, local: local.into(), chat: chat.into(), body: ChatText::default(), mentions: Vec::new(), what, state, parts: Vec::new() }
}

fn reaction(message: &str, emoji: &str, on: bool) -> OutKind {
    OutKind::React { message: message.into(), created: 1_000_001, emoji: emoji.into(), on }
}

fn react(s: &mut Session, message: &str, emoji: &str) -> Result<(String, bool), CmdError> {
    let v = s.run("teams.react", &json!({"chat": CHAT, "message": message, "emoji": emoji}))?;
    Ok((v["local"].as_str().unwrap_or_default().to_string(), v["on"].as_bool().unwrap_or_default()))
}

fn edit(s: &mut Session, message: &str, text: &str) -> Result<String, CmdError> {
    s.run("teams.edit", &json!({"chat": CHAT, "message": message, "text": text})).map(|v| v["local"].as_str().unwrap_or_default().to_string())
}

fn delete(s: &mut Session, message: &str) -> Result<String, CmdError> {
    s.run("teams.delete", &json!({"chat": CHAT, "message": message})).map(|v| v["local"].as_str().unwrap_or_default().to_string())
}

fn shown<'a>(s: &'a Session, id: &str) -> &'a ChatMessage {
    s.teams.window.iter().find(|m| m.id == id).unwrap()
}

/// The chips of a message, as (emoji, count, the user's).
fn chips(s: &Session, id: &str) -> Vec<(String, usize, bool)> {
    s.teams.shown_reactions(shown(s, id), ME).iter().map(|r| (r.emoji.to_string(), r.count, r.mine)).collect()
}

/// The notices of `template`, as said.
fn notices(s: &Session, template: &str) -> Vec<String> {
    s.notices.iter().filter(|n| n.template == template).map(|n| n.text.clone()).collect()
}

#[test]
fn a_reaction_shows_at_once_and_stays_once_made() {
    let mut s = opened();
    let (local, on) = react(&mut s, "m1", THUMBS).unwrap();
    assert!(on, "not the user's yet: set");
    let jobs = teams_jobs(&mut s);
    assert!(
        matches!(&jobs[..], [teams::Job::Op { op: Op::React { chat, local: l, message, created: 1_000_001, emoji, on: true } }]
            if chat == CHAT && *l == local && message == "m1" && emoji == THUMBS),
        "{jobs:?}"
    );
    assert_eq!(chips(&s, "m1"), [(THUMBS.to_string(), 2, true)], "shown at once");
    // Taken by the worker, then made: Teams' own state is the same.
    s.apply(outbox(CHAT, vec![change(CHAT, 3, &local, reaction("m1", THUMBS, true), OutState::Waiting)], Some(&local)));
    assert_eq!(chips(&s, "m1"), [(THUMBS.to_string(), 2, true)]);
    let mut made = m1();
    made.reactions[0].users.push(ME.into());
    window(&mut s, vec![made]);
    assert_eq!(chips(&s, "m1"), [(THUMBS.to_string(), 2, true)], "the copy has it, the outbox too: counted once");
    s.apply(outbox(CHAT, vec![], None));
    assert_eq!(chips(&s, "m1"), [(THUMBS.to_string(), 2, true)]);
    // Clicked again: taken back.
    let (_, on) = react(&mut s, "m1", THUMBS).unwrap();
    assert!(!on);
    assert!(matches!(&teams_jobs(&mut s)[..], [teams::Job::Op { op: Op::React { on: false, .. } }]));
    assert_eq!(chips(&s, "m1"), [(THUMBS.to_string(), 1, false)]);
}

/// Review Focus 1: clicked twice before it went, the worker cancels both (neither is in the
/// outbox it says): the chip is Teams' own.
#[test]
fn a_reaction_toggled_back_before_it_went_shows_teams_state() {
    let mut s = opened();
    let (a, on_a) = react(&mut s, "m1", THUMBS).unwrap();
    let (b, on_b) = react(&mut s, "m1", THUMBS).unwrap();
    assert_eq!((on_a, on_b), (true, false));
    assert_eq!(teams_jobs(&mut s).len(), 2);
    assert_eq!(chips(&s, "m1"), [(THUMBS.to_string(), 1, false)], "the last click stands");
    s.apply(outbox(CHAT, vec![], Some(&a)));
    assert_eq!(chips(&s, "m1"), [(THUMBS.to_string(), 1, false)]);
    s.apply(outbox(CHAT, vec![], Some(&b)));
    assert_eq!(chips(&s, "m1"), [(THUMBS.to_string(), 1, false)]);
    assert!(s.teams.outgoing.is_empty(), "{:?}", s.teams.outgoing);
    // Set while another chip of the user's is taken back: each kind as last asked.
    let mut both = m1();
    both.reactions.push(Reaction { emoji: "\u{1F606}".into(), users: vec![ME.into()] });
    window(&mut s, vec![both]);
    react(&mut s, "m1", THUMBS).unwrap();
    react(&mut s, "m1", "\u{1F606}").unwrap();
    assert_eq!(chips(&s, "m1"), [(THUMBS.to_string(), 2, true)], "a kind brought to nobody isn't shown");
}

/// Review Focus 3: the message's ❤ (without the emoji selector) and the user's ❤️ going out are one
/// chip, whichever spelling the user clicks.
#[test]
fn a_reaction_by_name_or_emoji_is_one_chip() {
    let (bare, heart) = ("\u{2764}", "\u{2764}\u{FE0F}");
    let mut s = opened();
    let mut m = m1();
    m.reactions = vec![Reaction { emoji: bare.into(), users: vec!["u-ana".into()] }];
    window(&mut s, vec![m]);
    let (local, on) = react(&mut s, "m1", heart).unwrap();
    assert!(on);
    assert_eq!(chips(&s, "m1"), [(bare.to_string(), 2, true)]);
    // As the worker keeps it: still one.
    s.apply(outbox(CHAT, vec![change(CHAT, 3, &local, reaction("m1", heart, true), OutState::Waiting)], Some(&local)));
    assert_eq!(chips(&s, "m1"), [(bare.to_string(), 2, true)]);
    // Clicked on its chip, by the message's spelling: taken back, cancelling the one waiting.
    let (back, on) = react(&mut s, "m1", bare).unwrap();
    assert!(!on);
    assert_eq!(chips(&s, "m1"), [(bare.to_string(), 1, false)]);
    s.apply(outbox(CHAT, vec![], Some(&back)));
    assert_eq!(chips(&s, "m1"), [(bare.to_string(), 1, false)]);
    // The user's own read back by the other spelling: theirs.
    let mut m = m1();
    m.reactions = vec![Reaction { emoji: bare.into(), users: vec!["u-ana".into(), "U-ME".into()] }];
    window(&mut s, vec![m]);
    assert_eq!(chips(&s, "m1"), [(bare.to_string(), 2, true)]);
    assert_eq!(react(&mut s, "m1", heart).map(|(_, on)| on), Ok(false));
}

#[test]
fn only_a_quick_reaction_or_one_already_there_may_be_added() {
    let mut s = opened();
    assert!(matches!(react(&mut s, "m1", "\u{1F498}"), Err(CmdError::Disabled(_))), "not one of the six");
    let mut m = m1();
    m.reactions.push(Reaction { emoji: "custom".into(), users: vec!["u-ana".into()] });
    m.reactions.push(Reaction { emoji: "\u{1F389}".into(), users: vec!["u-ana".into()] });
    window(&mut s, vec![m]);
    assert!(matches!(react(&mut s, "m1", "custom"), Err(CmdError::Disabled(_))), "a name, not an emoji");
    for junk in ["", " ", "\u{202E}\u{1F44D}", &"\u{1F44D}".repeat(40)] {
        assert!(matches!(react(&mut s, "m1", junk), Err(CmdError::Disabled(_))), "{junk:?}");
    }
    assert!(teams_jobs(&mut s).is_empty());
    assert_eq!(react(&mut s, "m1", "\u{1F62E}").map(|(_, on)| on), Ok(true), "a quick one");
    assert_eq!(react(&mut s, "m1", "\u{1F389}").map(|(_, on)| on), Ok(true), "one the message shows");
    assert_eq!(teams_jobs(&mut s).len(), 2);
    let kinds: Vec<String> = chips(&s, "m1").into_iter().map(|(e, ..)| e).collect();
    assert_eq!(kinds, [THUMBS, "custom", "\u{1F389}", "\u{1F62E}"], "one the message lacks comes last");
    // A message that can't take one: deleted, unshowable, not in the window, in a chat not open.
    let mut deleted = mine(3);
    deleted.deleted = Some(1_000_100);
    let mut unshowable = mine(4);
    unshowable.unshowable = true;
    window(&mut s, vec![deleted, unshowable]);
    for id in ["m3", "m4", "m99"] {
        assert!(matches!(react(&mut s, id, THUMBS), Err(CmdError::Disabled(_))), "{id}");
    }
    let other = s.run("teams.react", &json!({"chat": OTHER, "message": "m1", "emoji": THUMBS}));
    assert!(matches!(other, Err(CmdError::Disabled(_))), "{other:?}");
    assert!(matches!(s.run("teams.react", &json!({"chat": CHAT, "message": "m1"})), Err(CmdError::Params(_))), "no emoji");
    assert!(teams_jobs(&mut s).is_empty());
}

#[test]
fn a_chat_left_or_unreadable_takes_no_change() {
    for unwritable in [true, false] {
        let mut s = opened();
        let mut r = row(1, 10);
        r.left = unwritable;
        r.unreadable = !unwritable;
        s.apply(teams_event(teams::Event::Chats { rows: vec![r] }));
        assert!(matches!(react(&mut s, "m1", THUMBS), Err(CmdError::Disabled(_))));
        assert!(matches!(edit(&mut s, "m2", "changed"), Err(CmdError::Disabled(_))));
        assert!(matches!(delete(&mut s, "m2"), Err(CmdError::Disabled(_))));
        assert!(teams_jobs(&mut s).is_empty() && s.teams.outgoing.is_empty());
    }
}

#[test]
fn an_edit_shows_at_once_and_not_edited_when_refused() {
    let mut s = opened();
    let v = s.run("teams.edit", &json!({"chat": CHAT, "message": "m2", "text": "hello there", "bold": [[0, 3]]})).unwrap();
    let local = v["local"].as_str().unwrap().to_string();
    let body = teams_compose("hello there", std::slice::from_ref(&(0..3)), &[]);
    assert_eq!(
        teams_jobs(&mut s),
        [teams::Job::Op {
            op: Op::Edit {
                chat: CHAT.into(),
                local: local.clone(),
                message: "m2".into(),
                created: 1_000_002,
                body: body.clone(),
                mentions: Vec::new()
            }
        }]
    );
    let provisional = Outgoing {
        seq: 0,
        local: local.clone(),
        chat: CHAT.into(),
        body,
        mentions: Vec::new(),
        what: OutKind::Edit { message: "m2".into(), created: 1_000_002 },
        state: OutState::Waiting,
        parts: Vec::new(),
    };
    assert_eq!(s.teams.change_of("m2"), Some(&provisional), "shown at once");
    assert_eq!(s.teams.change_of("m1"), None);
    // Taken, then refused by Teams: kept "Not edited", with Teams' answer; on screen, no notice.
    let kept = Outgoing { seq: 4, ..provisional };
    s.apply(outbox(CHAT, vec![kept.clone()], Some(&local)));
    assert_eq!(s.teams.change_of("m2"), Some(&kept));
    let not_made = Outgoing { state: refused(), ..kept };
    s.apply(outbox(CHAT, vec![not_made.clone()], None));
    s.apply(outbox(CHAT, vec![not_made.clone()], None));
    assert_eq!(s.teams.change_of("m2"), Some(&not_made));
    assert!(notices(&s, "Your edit in {chat} wasn't made").is_empty(), "seen where it is");
    // Sent again: asked of the worker; refused again, then cancelled.
    s.run("teams.retry", &json!({"seq": 4})).unwrap();
    assert_eq!(teams_jobs(&mut s), [teams::Job::Retry { seq: 4 }]);
    assert_eq!(s.teams.change_of("m2").map(|o| &o.state), Some(&OutState::Waiting));
    s.apply(outbox(CHAT, vec![not_made], None));
    s.run("teams.discard", &json!({"seq": 4})).unwrap();
    assert_eq!(teams_jobs(&mut s), [teams::Job::Discard { seq: 4 }]);
    assert_eq!(s.teams.change_of("m2"), None);
}

#[test]
fn only_ones_own_editable_message_may_be_edited() {
    let mut s = opened();
    assert!(matches!(edit(&mut s, "m1", "changed"), Err(CmdError::Disabled(_))), "Ana's");
    let mut file = mine(3);
    file.attachments.push(Attachment::File {
        id: "a1".into(),
        name: "notes.txt".into(),
        url: "https://contoso.example/notes.txt".into(),
        size: Some(5),
    });
    let mut list = mine(4);
    list.body = ChatText {
        blocks: vec![Block { kind: BlockKind::Item { ordered: false, depth: 1 }, runs: vec![Run { text: "one".into(), ..Run::default() }] }],
    };
    let mut system = mine(5);
    system.kind = MessageKind::System;
    let mut deleted = mine(6);
    deleted.deleted = Some(1_000_100);
    deleted.body = ChatText::default();
    let mut unshowable = mine(7);
    unshowable.unshowable = true;
    let mut underlined = mine(8);
    underlined.body.blocks[0].runs[0].underline = true;
    window(&mut s, vec![file, list, system, deleted, unshowable, underlined]);
    for id in ["m3", "m4", "m5", "m6", "m7", "m8"] {
        assert!(matches!(edit(&mut s, id, "changed"), Err(CmdError::Disabled(_))), "{id}");
        assert!(!s.teams.editable(shown(&s, id), ME), "{id}");
    }
    assert!(!s.teams.editable(shown(&s, "m1"), ME));
    assert!(s.teams.editable(shown(&s, "m2"), ME));
    assert!(s.teams.editable(shown(&s, "m2"), "U-ME"), "the user's id whatever its capitals");
    assert!(!s.teams.editable(shown(&s, "m2"), ""), "nobody known signed in");
    // Not in the window, or of a chat not open.
    assert!(matches!(edit(&mut s, "m99", "changed"), Err(CmdError::Disabled(_))));
    let other = s.run("teams.edit", &json!({"chat": OTHER, "message": "m2", "text": "changed"}));
    assert!(matches!(other, Err(CmdError::Disabled(_))), "{other:?}");
    assert!(teams_jobs(&mut s).is_empty());
    // What the box loads: the message's words, or its edit going out's.
    let typed = s.teams.typed_for(shown(&s, "m2")).unwrap();
    assert_eq!((typed.text.as_str(), typed.bold.as_slice()), ("hello", std::slice::from_ref(&(0..3))));
    edit(&mut s, "m2", "hello again").unwrap();
    assert_eq!(s.teams.typed_for(shown(&s, "m2")).map(|t| t.text), Some("hello again".into()));
    assert!(s.teams.typed_for(shown(&s, "m4")).is_none(), "a list the box can't hold");
}

#[test]
fn an_edit_to_nothing_is_refused() {
    let mut s = opened();
    for nothing in ["  \n", "", "\n\n", "\u{1}\u{7}"] {
        assert_eq!(edit(&mut s, "m2", nothing), Err(CmdError::Params(TEAMS_NOTHING_LEFT.into())), "{nothing:?}");
    }
    assert!(teams_jobs(&mut s).is_empty());
    assert!(s.teams.change_of("m2").is_none());
}

#[test]
fn a_deletion_shows_at_once_and_holds_other_changes() {
    let mut s = opened();
    let local = delete(&mut s, "m2").unwrap();
    assert_eq!(
        teams_jobs(&mut s),
        [teams::Job::Op { op: Op::Delete { chat: CHAT.into(), local: local.clone(), message: "m2".into(), created: 1_000_002 } }]
    );
    assert!(matches!(s.teams.change_of("m2").map(|o| &o.what), Some(OutKind::Delete { message, created: 1_000_002 }) if message == "m2"));
    // While it goes, nothing else of that message: no edit, no reaction, no second deletion.
    assert!(matches!(edit(&mut s, "m2", "changed"), Err(CmdError::Disabled(_))));
    assert!(matches!(react(&mut s, "m2", THUMBS), Err(CmdError::Disabled(_))));
    assert!(matches!(delete(&mut s, "m2"), Err(CmdError::Disabled(_))));
    assert!(!s.teams.editable(shown(&s, "m2"), ME));
    // Someone else's message, a deleted one, a system line: never.
    let mut deleted = mine(3);
    deleted.deleted = Some(1_000_100);
    let mut system = mine(4);
    system.kind = MessageKind::System;
    window(&mut s, vec![deleted, system]);
    for id in ["m1", "m3", "m4", "m99"] {
        assert!(matches!(delete(&mut s, id), Err(CmdError::Disabled(_))), "{id}");
    }
    assert!(teams_jobs(&mut s).is_empty());
    // A deletion and an edit of one message going out: the deletion is what shows.
    let deletion = change(CHAT, 3, "100-1", OutKind::Delete { message: "m2".into(), created: 1_000_002 }, OutState::Waiting);
    let later = change(CHAT, 4, "100-2", OutKind::Edit { message: "m2".into(), created: 1_000_002 }, OutState::Waiting);
    s.apply(outbox(CHAT, vec![deletion.clone(), later], Some(&local)));
    assert_eq!(s.teams.change_of("m2"), Some(&deletion));
}

#[test]
fn the_last_own_message_is_what_up_edits() {
    let mut s = opened();
    window(&mut s, vec![ChatMessage { sender: from("u-ana"), ..msg(CHAT, 3) }]);
    assert_eq!(s.teams.last_own(ME).map(|m| m.id.as_str()), Some("m2"), "Ana's after it doesn't count");
    let mut deleted = mine(4);
    deleted.deleted = Some(1_000_100);
    window(&mut s, vec![deleted]);
    assert_eq!(s.teams.last_own(ME).map(|m| m.id.as_str()), Some("m2"), "nor a deleted one");
    assert_eq!(s.teams.last_own("u-nobody"), None);
    s.teams.newer_left_out = true;
    assert_eq!(s.teams.last_own(ME), None, "newer ones left out: the last isn't known");
}

/// ↑ right after a send: while a message of the user's to the open chat is going out (not "Not
/// sent"), the last isn't in the window yet, so there's none to edit (not the one before it).
#[test]
fn no_last_own_message_while_one_is_going_out() {
    let mut s = opened();
    s.run("teams.send", &json!({"chat": CHAT, "text": "fixed typo", "mentions": []})).unwrap();
    let local = s.teams.outgoing.last().map(|o| o.local.clone()).unwrap();
    assert_eq!(s.teams.last_own(ME), None, "just sent, not answered for yet");
    let send = |state| Outgoing {
        seq: 5,
        local: local.clone(),
        chat: CHAT.into(),
        body: ChatText::default(),
        mentions: Vec::new(),
        what: OutKind::Send,
        state,
        parts: Vec::new(),
    };
    for state in [OutState::Waiting, OutState::Sending] {
        s.apply(outbox(CHAT, vec![send(state.clone())], Some(&local)));
        assert_eq!(s.teams.last_own(ME), None, "{state:?}");
    }
    s.apply(outbox(CHAT, vec![send(refused())], None));
    assert_eq!(s.teams.last_own(ME).map(|m| m.id.as_str()), Some("m2"), "one not sent isn't going out");
    // Sent: it is the last.
    s.apply(outbox(CHAT, Vec::new(), None));
    window(&mut s, vec![mine(3)]);
    assert_eq!(s.teams.last_own(ME).map(|m| m.id.as_str()), Some("m3"));
}

#[test]
fn mentions_name_members_of_the_organisation() {
    let mut s = opened();
    let send = |s: &mut Session, text: &str, mentions: Value| s.run("teams.send", &json!({"chat": CHAT, "text": text, "mentions": mentions}));
    send(&mut s, "hi Ana Exemple", json!([[3, 14, "u-ana"]])).unwrap();
    let jobs = teams_jobs(&mut s);
    let [teams::Job::Op { op: Op::Send { mentions, body, .. } }] = &jobs[..] else { panic!("{jobs:?}") };
    assert_eq!(mentions, &[Mention { id: 0, text: "Ana Exemple".into(), user_id: Some("u-ana".into()) }]);
    assert!(body.blocks[0].runs.iter().any(|r| r.text == "Ana Exemple" && r.mention == Some(0)), "{body:?}");
    assert_eq!(s.teams.outgoing.last().map(|o| &o.mentions), Some(mentions), "shown with its mentions");
    // A member's id in other capitals, a guest of the organisation: taken.
    let mut guest = person("u-gus", "Gus", TENANT_ID);
    guest.roles = vec!["guest".into()];
    s.apply(teams_event(teams::Event::Members { chat: CHAT.into(), members: vec![guest], web_url: None, first: false }));
    send(&mut s, "Gus and Ana", json!([[0, 3, "u-gus"], [8, 11, "U-ANA"]])).unwrap();
    assert_eq!(teams_jobs(&mut s).len(), 1);
    // Someone of another organisation, a stranger: refused.
    let stranger = Err(CmdError::Params("a mention names a member of the conversation".into()));
    assert_eq!(send(&mut s, "hi Sam Roe", json!([[3, 10, "u-sam"]])), stranger);
    assert_eq!(send(&mut s, "hi Ana Exemple", json!([[3, 14, "u-nobody"]])), stranger);
    // Ranges that overlap, hold a line break, aren't one line as they stand, are off the text or
    // inside a character, or a mention that isn't [from, to, id]: refused.
    for (text, bad) in [
        ("hi Ana Exemple", json!([[3, 14, "u-ana"], [7, 14, "u-ana"]])),
        ("hi Ana Exemple", json!([[7, 14, "u-ana"], [3, 8, "u-ana"]])),
        ("hi\nAna Exemple", json!([[0, 6, "u-ana"]])),
        ("hi Ana  Exemple", json!([[3, 12, "u-ana"]])),
        ("hi Ana Exemple", json!([[2, 11, "u-ana"]])),
        ("hi Ana Exemple", json!([[3, 3, "u-ana"]])),
        ("hi Ana Exemple", json!([[3, 99, "u-ana"]])),
        ("hé Ana", json!([[2, 6, "u-ana"]])),
        ("hi Ana Exemple", json!([[3, 14]])),
        ("hi Ana Exemple", json!([[3, 14, ""]])),
        ("hi Ana Exemple", json!([[3, 14, 7]])),
        ("hi Ana Exemple", json!([3, 14, "u-ana"])),
        ("hi Ana Exemple", json!("u-ana")),
    ] {
        assert!(matches!(send(&mut s, text, bad.clone()), Err(CmdError::Params(_))), "{text:?} {bad}");
    }
    // Fifty at most.
    let text = "a ".repeat(51);
    let ranges = |n: usize| json!((0..n).map(|i| json!([2 * i, 2 * i + 1, "u-ana"])).collect::<Vec<_>>());
    assert!(matches!(send(&mut s, &text, ranges(51)), Err(CmdError::Params(_))));
    assert!(teams_jobs(&mut s).is_empty());
    send(&mut s, &text, ranges(50)).unwrap();
    assert!(matches!(&teams_jobs(&mut s)[..], [teams::Job::Op { op: Op::Send { mentions, .. } }] if mentions.len() == 50));
    // To a chat not open: its members aren't known here, so no mention.
    let elsewhere = s.run("teams.send", &json!({"chat": OTHER, "text": "hi Ana Exemple", "mentions": [[3, 14, "u-ana"]]}));
    assert_eq!(elsewhere, stranger);
    s.run("teams.send", &json!({"chat": OTHER, "text": "hi Ana Exemple", "mentions": []})).unwrap();
    assert_eq!(teams_jobs(&mut s).len(), 1);
}

/// An edit keeps the people the message mentions already, members or not any more; it adds only
/// members of the organisation.
#[test]
fn an_edit_keeps_who_the_message_mentions() {
    let mut s = opened();
    let mut m = mine(3);
    let lee = Run { text: "Lee Chu".into(), mention: Some(0), ..Run::default() };
    m.body = ChatText { blocks: vec![Block { kind: BlockKind::Paragraph, runs: vec![Run { text: "ask ".into(), ..Run::default() }, lee] }] };
    m.mentions = vec![Mention { id: 0, text: "Lee Chu".into(), user_id: Some("u-lee".into()) }];
    window(&mut s, vec![m]);
    let typed = s.teams.typed_for(shown(&s, "m3")).unwrap();
    assert_eq!(typed.mentions, [(4..11, "u-lee".to_string())]);
    let edit = |s: &mut Session, text: &str, mentions: Value| {
        s.run("teams.edit", &json!({"chat": CHAT, "message": "m3", "text": text, "mentions": mentions}))
    };
    edit(&mut s, "ask Lee Chu now", json!([[4, 11, "u-lee"]])).unwrap();
    let jobs = teams_jobs(&mut s);
    assert!(
        matches!(&jobs[..], [teams::Job::Op { op: Op::Edit { mentions, .. } }] if *mentions == [Mention { id: 0, text: "Lee Chu".into(), user_id: Some("u-lee".into()) }]),
        "{jobs:?}"
    );
    assert_eq!(edit(&mut s, "ask Sam Roe", json!([[4, 11, "u-sam"]])), Err(CmdError::Params("a mention names a member of the conversation".into())));
    assert!(teams_jobs(&mut s).is_empty());
}

#[test]
fn a_refused_reaction_is_said_once_and_not_kept() {
    const TEMPLATE: &str = "Teams didn't take your reaction in {chat}";
    let mut s = opened();
    let (local, _) = react(&mut s, "m1", THUMBS).unwrap();
    let not_taken = Outgoing { state: refused(), ..change(CHAT, 0, &local, reaction("m1", THUMBS, true), OutState::Waiting) };
    s.apply(outbox(CHAT, vec![not_taken.clone()], Some(&local)));
    s.apply(outbox(CHAT, vec![not_taken.clone()], None));
    assert_eq!(notices(&s, TEMPLATE), ["Teams didn't take your reaction in Chat 1"], "said once, on screen too");
    assert!(s.teams.outgoing.is_empty(), "never kept: {:?}", s.teams.outgoing);
    assert_eq!(chips(&s, "m1"), [(THUMBS.to_string(), 1, false)], "Teams' own state");
    // Let go after it was taken (refused when made): the same.
    let (local, _) = react(&mut s, "m1", THUMBS).unwrap();
    s.apply(outbox(CHAT, vec![change(CHAT, 3, &local, reaction("m1", THUMBS, true), OutState::Waiting)], Some(&local)));
    let let_go = Outgoing { state: OutState::NotSent { reason: Unsent::Unconfirmed, code: None }, ..not_taken.clone() };
    s.apply(outbox(CHAT, vec![Outgoing { local: local.clone(), ..let_go }], None));
    assert_eq!(notices(&s, TEMPLATE).len(), 2);
    assert!(s.teams.outgoing.is_empty() && chips(&s, "m1") == [(THUMBS.to_string(), 1, false)]);
    // Of a chat not open: said, never kept for it, and not said as a message not sent.
    let elsewhere = Outgoing { chat: OTHER.into(), local: "100-7".into(), ..not_taken };
    s.apply(outbox(OTHER, vec![elsewhere.clone()], None));
    s.apply(outbox(OTHER, vec![elsewhere], None));
    assert_eq!(
        notices(&s, TEMPLATE),
        ["Teams didn't take your reaction in Chat 1", "Teams didn't take your reaction in Chat 1", "Teams didn't take your reaction in Chat 2"]
    );
    assert!(notices(&s, "A message to {chat} wasn't sent").is_empty());
    open(&mut s, OTHER);
    assert!(s.teams.outgoing.is_empty());
}

#[test]
fn an_edit_not_made_elsewhere_is_said() {
    let mut s = opened();
    let edit = Outgoing {
        body: teams_compose("new words", &[], &[]),
        ..change(OTHER, 3, "100-1", OutKind::Edit { message: "m9".into(), created: 9 }, refused())
    };
    s.apply(outbox(OTHER, vec![edit.clone()], None));
    s.apply(outbox(OTHER, vec![edit.clone()], None));
    assert_eq!(notices(&s, "Your edit in {chat} wasn't made"), ["Your edit in Chat 2 wasn't made"]);
    // A deletion and a message not made there: each said in its own words.
    let deletion = change(OTHER, 4, "100-2", OutKind::Delete { message: "m8".into(), created: 8 }, refused());
    let send = Outgoing { body: teams_compose("hi", &[], &[]), ..change(OTHER, 5, "100-3", OutKind::Send, refused()) };
    s.apply(outbox(OTHER, vec![edit, deletion, send], None));
    assert_eq!(notices(&s, "Your edit in {chat} wasn't made").len(), 1);
    assert_eq!(notices(&s, "Your deletion in {chat} wasn't made"), ["Your deletion in Chat 2 wasn't made"]);
    assert_eq!(notices(&s, "A message to {chat} wasn't sent"), ["A message to Chat 2 wasn't sent"]);
    // The open chat in the background: said too.
    s.teams_focus(false);
    let here = change(CHAT, 6, "100-4", OutKind::Edit { message: "m2".into(), created: 1_000_002 }, refused());
    s.apply(outbox(CHAT, vec![here], None));
    assert_eq!(notices(&s, "Your edit in {chat} wasn't made"), ["Your edit in Chat 2 wasn't made", "Your edit in Chat 1 wasn't made"]);
}

#[test]
fn a_refused_edit_sent_again_keeps_its_kind() {
    let mut s = opened();
    let local = edit(&mut s, "m2", "hello there").unwrap();
    let Some(teams::Job::Op { op: Op::Edit { body, .. } }) = teams_jobs(&mut s).pop() else { panic!("no edit") };
    let what = OutKind::Edit { message: "m2".into(), created: 1_000_002 };
    let too_many = OutState::NotSent { reason: Unsent::TooMany, code: None };
    let refusal = Outgoing { body: body.clone(), ..change(CHAT, 0, &local, what.clone(), too_many) };
    s.apply(outbox(CHAT, vec![refusal.clone()], Some(&local)));
    s.apply(outbox(CHAT, vec![], None));
    assert_eq!(s.teams.change_of("m2"), Some(&refusal), "kept here: the worker holds nothing of it");
    let again = s.run("teams.retry", &json!({"seq": 0, "local": local})).unwrap()["local"].as_str().unwrap().to_string();
    assert_ne!(again, local);
    assert_eq!(
        teams_jobs(&mut s),
        [teams::Job::Op {
            op: Op::Edit { chat: CHAT.into(), local: again.clone(), message: "m2".into(), created: 1_000_002, body, mentions: Vec::new() }
        }]
    );
    assert_eq!(s.teams.change_of("m2").map(|o| (o.local.as_str(), &o.what, &o.state)), Some((again.as_str(), &what, &OutState::Waiting)));
    // A deletion likewise; and an edit refused, sent again while its message is being deleted:
    // refused, it would come after the deletion.
    let deletion = OutKind::Delete { message: "m2".into(), created: 1_000_002 };
    s.apply(outbox(CHAT, vec![change(CHAT, 0, "100-9", deletion, refused())], Some("100-9")));
    s.run("teams.retry", &json!({"seq": 0, "local": "100-9"})).unwrap();
    assert!(matches!(&teams_jobs(&mut s)[..], [teams::Job::Op { op: Op::Delete { message, .. } }] if message == "m2"));
    let edit_refused = Outgoing { body: teams_compose("later", &[], &[]), ..change(CHAT, 0, "100-8", what, refused()) };
    s.apply(outbox(CHAT, vec![edit_refused], Some("100-8")));
    assert!(matches!(s.run("teams.retry", &json!({"seq": 0, "local": "100-8"})), Err(CmdError::Disabled(_))));
    assert!(teams_jobs(&mut s).is_empty());
}

#[test]
fn chips_are_bounded_and_a_reaction_not_made_changes_none() {
    let mut s = opened();
    let mut m = m1();
    m.reactions = (0..teams::MAX_REACTIONS + 20).map(|i| Reaction { emoji: format!("{i}\u{20E3}"), users: vec!["u-ana".into()] }).collect();
    window(&mut s, vec![m]);
    assert_eq!(chips(&s, "m1").len(), teams::MAX_REACTIONS);
    // A reaction the worker says wasn't made, whatever its number, is no wish.
    let mut s = opened();
    s.apply(outbox(CHAT, vec![change(CHAT, 3, "100-1", reaction("m1", "\u{2764}\u{FE0F}", true), refused())], None));
    assert_eq!(chips(&s, "m1"), [(THUMBS.to_string(), 1, false)]);
    assert!(s.teams.outgoing.is_empty());
}

/// Gone back through the history, a change is to a message on screen: the window stays (a message
/// sent opens the chat at its newest again; a change doesn't).
#[test]
fn a_change_from_an_older_view_keeps_the_window() {
    let mut s = opened();
    s.teams.newer_left_out = true;
    react(&mut s, "m1", THUMBS).unwrap();
    edit(&mut s, "m2", "hello there").unwrap();
    delete(&mut s, "m2").unwrap();
    let jobs = teams_jobs(&mut s);
    assert!(jobs.iter().all(|j| matches!(j, teams::Job::Op { .. })) && jobs.len() == 3, "{jobs:?}");
    assert!(s.teams.newer_left_out && s.teams.window.len() == 2);
}

/// The message's edit `local` that Teams refused (`seq` 0: refused at the door, known here alone).
fn edit_not_made(s: &mut Session, seq: u64, local: &str, text: &str) {
    let what = OutKind::Edit { message: "m2".into(), created: 1_000_002 };
    let state = if seq == 0 { OutState::NotSent { reason: Unsent::TooMany, code: None } } else { refused() };
    let not_made = Outgoing { body: teams_compose(text, &[], &[]), ..change(CHAT, seq, local, what, state) };
    s.apply(outbox(CHAT, vec![not_made], Some(local)));
}

fn shown_locals(s: &Session) -> Vec<String> {
    s.teams.outgoing.iter().map(|o| o.local.clone()).collect()
}

/// An edit Teams refused is let go by a newer edit of its message (made later, it would put older
/// words back): the worker is told to let go of the one it keeps, and once the newer one is made
/// the message reads as Teams has it.
#[test]
fn a_newer_edit_takes_the_place_of_one_not_made() {
    let mut s = opened();
    let a = edit(&mut s, "m2", "hello A").unwrap();
    edit_not_made(&mut s, 4, &a, "hello A");
    teams_jobs(&mut s);
    // A newer edit refused here lets nothing go.
    assert!(matches!(edit(&mut s, "m2", &"a".repeat(90 << 10)), Err(CmdError::Params(_))));
    assert!(teams_jobs(&mut s).is_empty());
    assert_eq!(shown_locals(&s), std::slice::from_ref(&a));
    let b = edit(&mut s, "m2", "hello B").unwrap();
    let jobs = teams_jobs(&mut s);
    assert!(matches!(&jobs[..], [teams::Job::Discard { seq: 4 }, teams::Job::Op { op: Op::Edit { local, .. } }] if *local == b), "{jobs:?}");
    assert_eq!(shown_locals(&s), std::slice::from_ref(&b));
    // The worker took it in the place of the one let go, then made it: it holds neither.
    s.apply(outbox(CHAT, vec![], Some(&b)));
    assert_eq!(s.teams.change_of("m2"), None);
    assert_eq!(s.teams.typed_for(shown(&s, "m2")).map(|t| t.text), Some("hello".into()), "the message's own words");
}

/// The same with an edit refused at the door (`seq` 0, nothing kept by the worker): it leaves the
/// conversation, and nothing is asked of the worker.
#[test]
fn a_newer_edit_takes_the_place_of_one_refused_at_the_door() {
    let mut s = opened();
    let a = edit(&mut s, "m2", "hello A").unwrap();
    edit_not_made(&mut s, 0, &a, "hello A");
    s.apply(outbox(CHAT, vec![], None));
    teams_jobs(&mut s);
    assert_eq!(shown_locals(&s), std::slice::from_ref(&a), "kept here");
    let b = edit(&mut s, "m2", "hello B").unwrap();
    assert!(matches!(&teams_jobs(&mut s)[..], [teams::Job::Op { op: Op::Edit { local, .. } }] if *local == b));
    assert_eq!(shown_locals(&s), std::slice::from_ref(&b));
    s.apply(outbox(CHAT, vec![], Some(&b)));
    assert_eq!(s.teams.change_of("m2"), None);
    assert_eq!(s.teams.typed_for(shown(&s, "m2")).map(|t| t.text), Some("hello".into()));
}

/// A deletion lets go of its message's edits not made: nothing "Not edited" stays under a message
/// deleted.
#[test]
fn a_deletion_takes_the_place_of_an_edit_not_made() {
    let mut s = opened();
    let a = edit(&mut s, "m2", "hello A").unwrap();
    edit_not_made(&mut s, 4, &a, "hello A");
    teams_jobs(&mut s);
    let d = delete(&mut s, "m2").unwrap();
    let jobs = teams_jobs(&mut s);
    assert!(matches!(&jobs[..], [teams::Job::Discard { seq: 4 }, teams::Job::Op { op: Op::Delete { local, .. } }] if *local == d), "{jobs:?}");
    assert!(matches!(s.teams.change_of("m2").map(|o| &o.what), Some(OutKind::Delete { .. })));
    assert_eq!(shown_locals(&s), [d]);
}
