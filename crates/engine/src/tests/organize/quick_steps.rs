// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Quick Steps that finish follow-up, write a message, or wait to be set up.

use super::*;

fn open_and_select(s: &mut Session, subject: &str) -> Value {
    let k = key_of(s, subject);
    s.run("list.open", &k).unwrap();
    k
}

/// "Mark complete": the follow-up is done, as Mark Complete does it.
#[test]
fn a_quick_step_can_complete_the_follow_up() {
    let mut s = sample_session("qs-complete");
    s.run("quickStep.create", &json!({"name": "Done", "complete": true})).unwrap();
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("mail.flag", &json!({"due": "tomorrow"})).unwrap();
    s.run("quickStep.run", &json!({"name": "Done"})).unwrap();
    let m = s.list.iter().find(|m| m.subject == "Character turnarounds").cloned().unwrap();
    assert!(!m.flags.flagged && m.flags.keywords.iter().any(|k| k.eq_ignore_ascii_case(quatrelle_model::FOLLOW_UP_COMPLETE)), "{:?}", m.flags);
}

/// A step that writes opens the message filled in, and sends nothing.
#[test]
fn a_quick_step_that_writes_opens_a_window_and_sends_nothing() {
    let mut s = sample_session("qs-write");
    s.run("quickStep.create", &json!({"name": "Boss", "write": {"forward": {"to": "boss@studio.example"}}})).unwrap();
    s.run("quickStep.create", &json!({"name": "Team", "write": {"newMessage": {"to": "a@studio.example, b@studio.example"}}})).unwrap();
    s.run("quickStep.create", &json!({"name": "Answer", "actions": [{"do": "delete"}], "write": "reply"})).unwrap();
    open_and_select(&mut s, "Character turnarounds");
    s.run("quickStep.run", &json!({"name": "Boss"})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.subject, "Fwd: Character turnarounds");
    assert!(c.to.starts_with("boss@studio.example"), "{}", c.to);
    assert!(s.outbox.is_empty());
    s.run("compose.discard", &json!({})).unwrap();
    s.run("quickStep.run", &json!({"name": "Team"})).unwrap();
    let c = s.compose.clone().unwrap();
    assert!(c.source.is_none() && c.to.contains("a@studio.example") && c.to.contains("b@studio.example"), "{}", c.to);
    s.run("compose.discard", &json!({})).unwrap();
    // Reply, then the original goes to Deleted Items.
    let trash = s.account("sample").and_then(|a| a.folder(FolderRole::Trash)).map(|f| f.path.clone()).unwrap();
    let id = s.list.iter().find(|m| m.subject == "Character turnarounds").map(|m| m.message_id.clone()).unwrap();
    s.run("quickStep.run", &json!({"name": "Answer"})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.subject, "Re: Character turnarounds");
    assert!(c.source.is_some(), "the window keeps its original");
    assert!(s.folder_state("sample", &trash).messages.iter().any(|m| m.message_id == id));
    assert!(s.outbox.is_empty());
}

/// Writing is for one message: with two, nothing is marked either.
#[test]
fn a_quick_step_that_writes_refuses_two_messages_before_any_mark() {
    let mut s = sample_session("qs-two");
    s.run("quickStep.create", &json!({"name": "Answer", "actions": [{"do": "markRead"}, {"do": "flag"}], "write": "reply"})).unwrap();
    let (a, b) = ("Sound mix, version 3", "Character turnarounds");
    s.run("list.select", &key_of(&s, a)).unwrap();
    s.run("list.select", &json!({"key": key_of(&s, b)["key"], "add": true})).unwrap();
    assert_eq!(s.selection.len(), 2);
    assert!(matches!(s.run("quickStep.run", &json!({"name": "Answer"})), Err(CmdError::Disabled(_))));
    assert!(s.list.iter().filter(|m| m.subject == a || m.subject == b).all(|m| !m.flags.flagged));
    assert!(s.list.iter().find(|m| m.subject == a).is_some_and(|m| !m.flags.seen));
    assert!(s.compose.is_none());
}

/// Wrong addresses and empty steps are refused; saving a ready-made step sets it up.
#[test]
fn quick_steps_that_write_are_checked() {
    let mut s = sample_session("qs-checked");
    for bad in [
        json!({"name": "A", "write": {"forward": {"to": "not an address"}}}),
        json!({"name": "B", "write": {"forward": {"to": ""}}}),
        json!({"name": "C", "write": {"newMessage": {"to": "  "}}}),
        json!({"name": "D", "write": "sideways"}),
        json!({"name": "E", "complete": "yes"}),
        json!({"name": "F"}),
        json!({"name": "G", "actions": [], "complete": false}),
    ] {
        assert!(s.run("quickStep.create", &bad).is_err(), "{bad}");
    }
    assert!(s.prefs.quick_steps.is_empty());
    s.run("quickStep.create", &json!({"name": "Ok", "write": "replyAll"})).unwrap();
    assert!(s.run("quickStep.edit", &json!({"name": "Ok", "write": {"forward": {"to": "x"}}})).is_err());
    s.run("quickStep.edit", &json!({"name": "Ok", "write": null, "complete": true})).unwrap();
    assert!(s.prefs.quick_steps[0].complete && s.prefs.quick_steps[0].write.is_none());
    assert!(s.run("quickStep.edit", &json!({"name": "Ok", "complete": false})).is_err(), "nothing left to do");
}

/// The ready-made steps: four, waiting to be set up, once; a name taken is passed over.
#[test]
fn the_ready_made_quick_steps_come_once_and_wait_to_be_set_up() {
    let names = json!({"names": ["Forward to my manager", "Write to the team", "Handled", "Reply then delete"]});
    let mut s = sample_session("qs-seed");
    s.run("quickStep.create", &json!({"name": "write to the TEAM", "actions": [{"do": "flag"}]})).unwrap();
    s.run("quickStep.seed", &names).unwrap();
    let got: Vec<(&str, bool)> = s.prefs.quick_steps.iter().map(|q| (q.name.as_str(), q.unset)).collect();
    assert_eq!(got, [("write to the TEAM", false), ("Forward to my manager", true), ("Handled", true), ("Reply then delete", true)]);
    assert!(s.prefs.quick_steps_seeded);
    // Twice: nothing. Deleted: they stay gone.
    s.run("quickStep.seed", &names).unwrap();
    assert_eq!(s.prefs.quick_steps.len(), 4);
    s.run("quickStep.delete", &json!({"name": "Handled"})).unwrap();
    s.run("quickStep.seed", &names).unwrap();
    assert_eq!(s.prefs.quick_steps.len(), 3);
    // Not set up: refused.
    open_and_select(&mut s, "Character turnarounds");
    assert!(matches!(s.run("quickStep.run", &json!({"name": "Forward to my manager"})), Err(CmdError::Params(m)) if m.contains("set up")));
    assert!(s.compose.is_none());
    // Saved with an address: it is set up.
    s.run("quickStep.edit", &json!({"name": "Forward to my manager", "write": {"forward": {"to": "boss@studio.example"}}})).unwrap();
    assert!(!s.prefs.quick_steps[1].unset);
    s.run("quickStep.run", &json!({"name": "Forward to my manager"})).unwrap();
    assert!(s.compose.as_ref().is_some_and(|c| c.to.starts_with("boss@studio.example")));
    // Bad names.
    for bad in [json!({}), json!({"names": ["a"]}), json!({"names": [1, 2, 3, 4]}), json!({"names": "x"})] {
        let mut t = sample_session("qs-seed-bad");
        assert!(t.run("quickStep.seed", &bad).is_err(), "{bad}");
        assert!(!t.prefs.quick_steps_seeded);
    }
}

/// Settings saved before these fields load as they were.
#[test]
fn old_quick_steps_load() {
    let old = json!({"quickSteps": [{"name": "Flag it", "actions": [{"do": "flag"}]}]});
    let mut prefs = serde_json::to_value(crate::session::Prefs::default()).unwrap();
    if let (Some(p), Some(o)) = (prefs.as_object_mut(), old.as_object()) {
        p.extend(o.clone());
        p.remove("quickStepsSeeded");
    }
    let back: crate::session::Prefs = serde_json::from_value(prefs).unwrap();
    assert_eq!(back.quick_steps.len(), 1);
    let q = &back.quick_steps[0];
    assert!(!q.complete && q.write.is_none() && !q.unset && !back.quick_steps_seeded);
    let again = serde_json::to_value(q).unwrap();
    assert!(again.get("write").is_none(), "{again}");
}
