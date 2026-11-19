// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! MailTips through the fake Graph server: what the server says about a message's recipients
//! before it goes.

use quatrelle_model::job::Secret;
use quatrelle_model::scheduling::MailTip;
use quatrelle_model::{AccountConfig, Event, Job, SyncStatus};

use super::*;
use crate::worker::{MemoryCredentials, Pool};

/// A tip is read for the recipients that have one, as text (an automatic reply's HTML turned
/// into words, the scope as Graph spells it); an ordinary recipient gets none.
#[test]
fn mail_tips_are_read_for_the_recipients_with_something_to_tell() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.mail_tips = vec![
        MailTip {
            email: "ana@contoso.example".into(),
            automatic_replies: "<p>Away until <b>Monday</b></p>".into(),
            custom: "Ring first".into(),
            scope: "externalPartner".into(),
            ..Default::default()
        },
        MailTip {
            email: "full@contoso.example".into(),
            mailbox_full: true,
            external_members: 12,
            max_message_size: 10_485_760,
            restricted: true,
            moderated: true,
            invalid: true,
            ..Default::default()
        },
    ];
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let emails = ["ana@contoso.example".to_string(), "bo@contoso.example".into(), "full@contoso.example".into(), "nobody@contoso.example".into()];
    let tips = crate::graph_pim::mailtips::mail_tips(&c, &emails).unwrap();
    assert_eq!(tips.len(), 2, "{tips:?}");
    assert_eq!((tips[0].email.as_str(), tips[0].automatic_replies.as_str()), ("ana@contoso.example", "Away until Monday"));
    assert_eq!((tips[0].custom.as_str(), tips[0].scope.as_str()), ("Ring first", "externalPartner"));
    assert!(tips[1].mailbox_full && tips[1].restricted && tips[1].moderated && tips[1].invalid);
    assert_eq!((tips[1].external_members, tips[1].max_message_size), (12, 10_485_760));
    let asked = server.data.lock().unwrap().mail_tips_asked.clone();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0]["EmailAddresses"], json!(["ana@contoso.example", "bo@contoso.example", "full@contoso.example", "nobody@contoso.example"]));
    assert!(asked[0]["MailTipsOptions"].as_str().unwrap_or_default().contains("automaticReplies"));
}

/// Through the pool: the engine asks, the server's tips come back with the question's `ask`.
#[test]
fn mail_tips_are_answered_through_the_pool() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.mail_tips = vec![MailTip { email: "ana@contoso.example".into(), custom: "On leave".into(), ..Default::default() }];
    let server = serve(d);
    let store = crate::fake::tests_store("graph-mail-tips");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let account = AccountConfig { id: "a1".into(), email: "me@contoso.example".into(), graph: Some(server.base.clone()), ..Default::default() };
    pool.submit(Job::Start { account, password: Some(Secret(TOKEN.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    pool.submit(Job::GetMailTips { account: "a1".into(), from: "me@contoso.example".into(), recipients: vec!["ana@contoso.example".into()], ask: 9 });
    let seen = until(&pool, "mail tips", |e| matches!(e, Event::MailTips { .. }));
    match seen.last() {
        Some(Event::MailTips { account, ask, tips, error }) => {
            assert_eq!((account.as_str(), *ask, error.as_deref()), ("a1", 9, None));
            assert_eq!(tips.len(), 1);
            assert_eq!(tips[0].custom, "On leave");
        }
        other => panic!("{other:?}"),
    }
    drop(pool);
}
