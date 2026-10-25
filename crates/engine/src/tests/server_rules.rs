//! Engine tests: inbox rules the servers run (Exchange's, and Sieve's for an IMAP account).

use super::*;

/// A server's rule Quatrelle understands can be changed from here: only that rule goes, with
/// the place it has there; one it doesn't understand is refused rather than truncated.
#[test]
fn a_server_rule_is_changed_where_it_lives() {
    use quatrelle_model::organize::{Condition, ForeignRule, Rule, RuleAction};

    let mut s = Session::open(Some(temp("server-rule-save")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let known = Rule {
        name: "Clients".into(),
        enabled: true,
        account: Some(id.clone()),
        conditions: vec![Condition::From("@client.example".into())],
        match_all: true,
        actions: vec![RuleAction::MoveTo("INBOX/Clients".into())],
        stop: false,
        server: true,
        remote_id: Some("R3".into()),
        priority: Some(7),
        refused: None,
    };
    s.foreign_rules.push(ForeignRule {
        account: id.clone(),
        id: "R3".into(),
        name: "Clients".into(),
        enabled: true,
        runnable: true,
        rule: Some(known.clone()),
        summary: String::new(),
    });
    s.foreign_rules.push(ForeignRule {
        account: id.clone(),
        id: "R9".into(),
        name: "Body rule".into(),
        enabled: true,
        runnable: false,
        rule: None,
        summary: String::new(),
    });
    // Changed: its name and what it does, its place kept.
    let edited = Rule {
        name: "Clients (prospects)".into(),
        conditions: vec![Condition::From("@client.example".into()), Condition::HasAttachment],
        actions: vec![RuleAction::MoveTo("INBOX/Clients".into()), RuleAction::MarkRead],
        ..known.clone()
    };
    s.run("rules.server.save", &json!({"account": id, "id": "R3", "rule": edited})).unwrap();
    let job = s.take_jobs().into_iter().find(|j| matches!(j, Job::SetServerRule { .. })).expect("the rule goes to the server");
    assert!(matches!(&job, Job::SetServerRule { .. }), "the rule goes to the server");
    if let Job::SetServerRule { id: rule_id, rule, .. } = job {
        assert_eq!(rule_id, "R3");
        assert_eq!(rule.conditions.len(), 2);
        assert_eq!(rule.actions.len(), 2);
        assert_eq!(rule.priority, Some(7), "the place it has there");
        assert_eq!(rule.remote_id.as_deref(), Some("R3"));
    }
    // A rule Quatrelle can't say entirely isn't half-written.
    assert!(s.run("rules.server.save", &json!({"account": id, "id": "R9", "rule": known})).is_err());
    done(s);
}

/// A rule marked "on the server" goes to Exchange as a job, only when what the server would be
/// told changes; the id Exchange gives it comes back and is kept, so the next change finds the
/// same rule. A rule Exchange can't run stays home, and says so.
#[test]
fn server_rules_go_to_the_server_and_keep_their_id() {
    use quatrelle_model::organize::{Condition, Rule, RuleAction};
    let mut s = sample_session("server-rules");
    let account = s.accounts[0].config.id.clone();
    let rule = Rule {
        name: "Invoices".into(),
        enabled: true,
        account: Some(account.clone()),
        conditions: vec![Condition::Subject("invoice".into())],
        match_all: true,
        actions: vec![RuleAction::MoveTo("Inbox".into())],
        stop: false,
        server: true,
        remote_id: None,
        priority: None,
        refused: None,
    };
    s.run("rules.set", &json!({ "rules": [rule] })).unwrap();
    let sent: Vec<(String, Vec<Rule>, Vec<String>)> = s
        .take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            quatrelle_model::Job::SetServerRules { account, rules, delete, .. } => Some((account, rules, delete)),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1, "one job for the account");
    assert_eq!(sent[0].1.len(), 1, "the rule marked for the server");
    assert!(sent[0].2.is_empty(), "nothing to remove");
    // The server's answer brings its id back: it's kept, and nothing is sent again.
    s.apply(quatrelle_model::Event::ServerRules {
        account: account.clone(),
        ids: vec![("Invoices".into(), "R1".into())],
        others: Vec::new(),
        cleared: Vec::new(),
        error: None,
    });
    assert_eq!(s.rules[0].remote_id.as_deref(), Some("R1"));
    assert!(s.take_jobs().is_empty(), "the same rules aren't sent twice");
    // The rules the server runs that Quatrelle didn't make are listed as they are (Outlook's
    // own, say), and never changed here.
    s.apply(quatrelle_model::Event::ServerRules {
        account: account.clone(),
        ids: Vec::new(),
        others: vec![quatrelle_model::organize::ForeignRule {
            account: String::new(),
            id: "R9".into(),
            name: "Outlook rule".into(),
            enabled: true,
            runnable: false,
            rule: None,
            summary: "subject \"x\" → delete".into(),
        }],
        cleared: Vec::new(),
        error: None,
    });
    assert_eq!(s.foreign_rules.len(), 1, "{:?}", s.foreign_rules);
    assert_eq!(s.foreign_rules[0].name, "Outlook rule");
    assert_eq!(s.foreign_rules[0].account, account);
    assert_eq!(s.rules[0].remote_id.as_deref(), Some("R1"), "a read leaves our own ids alone");
    // A server that has started syncing is readable: its rules are asked for then, not once
    // the whole mailbox has been walked — and once is enough, however many folders follow.
    s.apply(quatrelle_model::Event::Status { account: account.clone(), status: SyncStatus::Syncing { folder: "INBOX".into() } });
    let asked = |s: &mut Session| s.take_jobs().into_iter().filter(|j| matches!(j, quatrelle_model::Job::GetServerRules { .. })).count();
    assert_eq!(asked(&mut s), 1, "asked as soon as the server answers");
    s.apply(quatrelle_model::Event::Status { account: account.clone(), status: SyncStatus::Syncing { folder: "Sent".into() } });
    assert_eq!(asked(&mut s), 0, "once a session is enough");
    // Connecting asks the server what it keeps: ours (to keep their ids right) and the rest.
    s.apply(quatrelle_model::Event::Status { account: account.clone(), status: SyncStatus::Online { at: NOW } });
    let reads: Vec<(String, Vec<String>)> = s
        .take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            quatrelle_model::Job::GetServerRules { account, ours } => Some((account, ours)),
            _ => None,
        })
        .collect();
    assert_eq!(reads, vec![(account.clone(), vec!["R1".to_string()])]);
    // Online again (after each folder, each look for new mail): not read again — that was over
    // a hundred reads a pass on a big mailbox.
    s.apply(quatrelle_model::Event::Status { account: account.clone(), status: SyncStatus::Online { at: NOW + 60 } });
    assert_eq!(asked(&mut s), 0, "read on coming online, not on every Online");
    // Each look for new mail goes through Syncing to Online again, every half minute: the
    // rules were read on each, all day long.
    for i in 1..4 {
        s.apply(quatrelle_model::Event::Status { account: account.clone(), status: SyncStatus::Syncing { folder: "INBOX".into() } });
        s.apply(quatrelle_model::Event::Status { account: account.clone(), status: SyncStatus::Online { at: NOW + 60 + i * 30 } });
    }
    assert_eq!(asked(&mut s), 0, "a look for new mail doesn't read the rules");
    // Losing the server starts it over: read when it answers again, and once it's online.
    s.apply(quatrelle_model::Event::Status { account: account.clone(), status: SyncStatus::Error { error: "timed out".into() } });
    s.apply(quatrelle_model::Event::Status { account: account.clone(), status: SyncStatus::Syncing { folder: "INBOX".into() } });
    assert_eq!(asked(&mut s), 1, "read again once the server answers");
    s.apply(quatrelle_model::Event::Status { account: account.clone(), status: SyncStatus::Online { at: NOW + 300 } });
    assert_eq!(asked(&mut s), 1, "and on coming online");
    // A refusal says nothing of what the server keeps: the list stays.
    s.apply(quatrelle_model::Event::ServerRules {
        account: account.clone(),
        ids: Vec::new(),
        others: Vec::new(),
        cleared: Vec::new(),
        error: Some("the server is busy".into()),
    });
    assert_eq!(s.foreign_rules.len(), 1, "Outlook's rules stay listed after a refusal");
    // Turned off: its server copy is removed, and the id with it.
    let mut off = s.rules[0].clone();
    off.server = false;
    s.run("rules.set", &json!({ "rules": [off] })).unwrap();
    let sent: Vec<(String, Vec<Rule>, Vec<String>)> = s
        .take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            quatrelle_model::Job::SetServerRules { account, rules, delete, .. } => Some((account, rules, delete)),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].1.is_empty());
    assert_eq!(sent[0].2, vec!["R1".to_string()], "the server copy goes");
    assert_eq!(s.rules[0].remote_id, None, "and the id is forgotten");
    // A rule Exchange can't run stays on this computer, and the user is told why.
    let mut good = s.rules[0].clone();
    good.server = true;
    let mut bad = good.clone();
    bad.name = "Newsletters".into();
    bad.conditions = vec![Condition::Bulk];
    s.run("rules.set", &json!({ "rules": [good, bad] })).unwrap();
    assert!(!s.rules[1].server, "Exchange knows no newsletters");
    assert!(s.notices.iter().any(|n| n.text.contains("Newsletters")), "{:?}", s.notices);
}

/// A rule the server refuses outright (Exchange won't change its rules while Outlook keeps its
/// own copy of them) is marked with why, isn't sent again on every connection, and the reason
/// is told once per account a session, not on every try. Changed here, or turned off and on, it
/// goes again; a refusal that may pass (the network) marks nothing.
#[test]
fn a_refused_rule_waits_to_be_changed() {
    use quatrelle_model::organize::{Condition, Rule, RuleAction};
    let mut s = sample_session("refused-rule");
    let account = s.accounts[0].config.id.clone();
    let rule = Rule {
        name: "Invoices".into(),
        enabled: true,
        account: Some(account.clone()),
        conditions: vec![Condition::Subject("invoice".into())],
        match_all: true,
        actions: vec![RuleAction::MoveTo("Inbox".into())],
        stop: false,
        server: true,
        remote_id: None,
        priority: None,
        refused: None,
    };
    s.run("rules.set", &json!({ "rules": [rule] })).unwrap();
    let sets = |s: &mut Session| s.take_jobs().into_iter().filter(|j| matches!(j, quatrelle_model::Job::SetServerRules { .. })).count();
    assert_eq!(sets(&mut s), 1);
    let refusal = || quatrelle_model::Event::ServerRules {
        account: account.clone(),
        ids: Vec::new(),
        others: Vec::new(),
        cleared: Vec::new(),
        error: Some("the server refused: Outlook keeps its own copy of this mailbox's rules".into()),
    };
    let errors = |s: &Session| s.notices.iter().filter(|n| n.error).count();
    s.apply(refusal());
    assert!(s.rules[0].refused.as_deref().is_some_and(|w| w.contains("Outlook")), "{:?}", s.rules[0]);
    assert_eq!(errors(&s), 1, "told");
    // Kept: a restart finds the marker.
    let again = Session::open(Some(Store::open(s.store.clone().unwrap().root()).unwrap()), quatrelle_model::dates::Clock::new(NOW, 0));
    assert_eq!(again.rules[0].refused, s.rules[0].refused);
    // Connected again: not sent again, and the same refusal isn't told again.
    s.apply(quatrelle_model::Event::Status { account: account.clone(), status: SyncStatus::Online { at: NOW } });
    assert_eq!(sets(&mut s), 0, "a refused rule waits");
    s.apply(refusal());
    assert_eq!(errors(&s), 1, "the same reason, told once a session");
    s.apply(quatrelle_model::Event::ServerRules {
        account: account.clone(),
        ids: Vec::new(),
        others: Vec::new(),
        cleared: Vec::new(),
        error: Some("the server refused: access denied".into()),
    });
    assert_eq!(errors(&s), 2, "another reason is told");
    // Changed here (the editor clears the marker): sent again.
    let mut edited = s.rules[0].clone();
    edited.conditions = vec![Condition::Subject("invoice ".into())];
    edited.refused = None;
    s.run("rules.set", &json!({ "rules": [edited] })).unwrap();
    assert_eq!(sets(&mut s), 1, "sent again once changed");
    assert_eq!(s.rules[0].refused, None);
    // A refusal that may pass (the network, a busy server) marks nothing: tried again next time.
    s.apply(quatrelle_model::Event::ServerRules {
        account: account.clone(),
        ids: Vec::new(),
        others: Vec::new(),
        cleared: Vec::new(),
        error: Some("network: unreachable".into()),
    });
    assert_eq!(s.rules[0].refused, None);
    s.apply(quatrelle_model::Event::Status { account: account.clone(), status: SyncStatus::Offline });
    s.apply(quatrelle_model::Event::Status { account: account.clone(), status: SyncStatus::Online { at: NOW + 60 } });
    assert_eq!(sets(&mut s), 1, "sent again on the next connection");
    // Turned off, the marker goes with the server copy; on again, it's tried afresh.
    s.apply(refusal());
    assert!(s.rules[0].refused.is_some());
    let mut off = s.rules[0].clone();
    off.server = false;
    s.run("rules.set", &json!({ "rules": [off] })).unwrap();
    assert_eq!(s.rules[0].refused, None);
    let _ = sets(&mut s);
    let mut on = s.rules[0].clone();
    on.server = true;
    s.run("rules.set", &json!({ "rules": [on] })).unwrap();
    assert_eq!(sets(&mut s), 1);
}

/// An IMAP account's rules run in its Sieve script, so a rule Exchange couldn't take (looking in
/// every header at once) still goes to the server; what neither can run stays home.
#[test]
fn sieve_rules_go_to_an_imap_account() {
    use quatrelle_model::organize::{Condition, Rule, RuleAction, ServerKind};
    let mut s = sample_session("sieve-rules");
    let account = s.accounts[0].config.id.clone();
    assert_eq!(s.server_kind(&account), Some(ServerKind::Sieve), "the sample account is IMAP");
    let rule = Rule {
        name: "Urgent".into(),
        enabled: true,
        account: Some(account.clone()),
        conditions: vec![Condition::AnyText("urgent".into())],
        match_all: true,
        actions: vec![RuleAction::MarkRead, RuleAction::MoveToRole(FolderRole::Archive)],
        stop: false,
        server: true,
        remote_id: None,
        priority: None,
        refused: None,
    };
    assert!(!rule.server_ready() && rule.sieve_ready(), "Exchange can't, Sieve can");
    s.run("rules.set", &json!({ "rules": [rule] })).unwrap();
    let sent: Vec<Vec<Rule>> = s
        .take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            quatrelle_model::Job::SetServerRules { rules, .. } => Some(rules),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1, "the rule marked for the server is sent");
    assert_eq!(sent[0].len(), 1);
    assert!(s.rules[0].server, "it stays marked for the server");
    assert!(!s.notices.iter().any(|n| n.text.contains("Urgent")), "nothing to keep home: {:?}", s.notices);
    // What Sieve can't look at either (attachments) stays on this computer, and says so.
    let mut bad = s.rules[0].clone();
    bad.name = "Attachments".into();
    bad.conditions = vec![Condition::HasAttachment];
    let mut good = s.rules[0].clone();
    good.name = "Urgent".into();
    s.run("rules.set", &json!({ "rules": [good, bad] })).unwrap();
    assert!(!s.rules[1].server, "Sieve can't see attachments");
    assert!(s.notices.iter().any(|n| n.text.contains("Attachments")), "{:?}", s.notices);
}

/// Exchange refuses every change while Outlook keeps its own copy of the rules: once the user
/// says so, that copy is replaced and every rule of the account goes again.
#[test]
fn outlooks_copy_of_the_rules_is_replaced_when_the_user_says_so() {
    use quatrelle_model::organize::{Condition, Rule, RuleAction};
    let mut s = sample_session("outlook-rules");
    let account = s.accounts[0].config.id.clone();
    assert!(s.run("rules.replaceOutlookCopy", &json!({"account": account})).is_err(), "not an Exchange account");
    s.accounts[0].config.ews = Some("https://mail.studio.example/EWS/Exchange.asmx".into());
    let rule = Rule {
        name: "Invoices".into(),
        enabled: true,
        account: Some(account.clone()),
        conditions: vec![Condition::Subject("invoice".into())],
        match_all: true,
        actions: vec![RuleAction::MarkRead],
        stop: false,
        server: true,
        remote_id: None,
        priority: None,
        refused: None,
    };
    s.run("rules.set", &json!({ "rules": [rule] })).unwrap();
    s.take_jobs();
    assert!(!s.outlook_keeps_rules(&account));
    s.apply(quatrelle_model::Event::ServerRules {
        account: account.clone(),
        ids: Vec::new(),
        others: Vec::new(),
        cleared: Vec::new(),
        error: Some("the server refused: Outlook keeps its own copy of this mailbox's rules".into()),
    });
    assert!(s.outlook_keeps_rules(&account) && s.rules[0].refused.is_some());
    let out = s.run("rules.replaceOutlookCopy", &json!({"account": account})).unwrap();
    assert_eq!(out["rules"], 1);
    let jobs = s.take_jobs();
    assert!(matches!(jobs.first(), Some(Job::ReplaceOutlookRules { account: a }) if *a == account), "the copy first: {jobs:?}");
    assert!(matches!(jobs.get(1), Some(Job::SetServerRules { rules, .. }) if rules.len() == 1), "then the rules: {jobs:?}");
    assert!(!s.outlook_keeps_rules(&account) && s.rules[0].refused.is_none());
}
