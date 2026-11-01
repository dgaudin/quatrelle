//! Autodiscover asks before another domain: a `RedirectAddr` or an `EwsUrl` on a site the account
//! doesn't trust is `Untrusted`, and the account waits for the answer (`NeedsTrust`), its Outbox
//! with it. `127.0.0.1` and `localhost` reach the same fake server as two sites; the address's
//! own domain (`contoso.example`) is stood for by the fake's address (`base`).

use super::*;
use crate::NetError;

fn untrusted(r: crate::Result<String>) -> String {
    match r {
        Err(NetError::Untrusted { site }) => site,
        other => panic!("not Untrusted: {other:?}"),
    }
}

/// The requests the server saw on a host (`localhost`, `127.0.0.1`).
fn seen_on(data: &Arc<Mutex<Data>>, host: &str) -> usize {
    data.lock().unwrap().hosts.iter().filter(|h| h.split(':').next() == Some(host)).count()
}

fn ask(server: &super::server::Server, trust: &quatrelle_model::addr::trust::Trust) -> crate::Result<String> {
    crate::ews::autodiscover("riley@contoso.example", &format!("{DOMAIN}\\{USER}"), PASSWORD, Some(&server.url), trust)
}

/// A `RedirectAddr` to an address on another domain: not followed. Nothing at all goes there,
/// not even a request without the password.
#[test]
fn a_redirect_addr_to_another_domain_is_not_followed() {
    let d = Data { redirect_addr: vec![("riley@contoso.example".into(), "riley@localhost".into())], ..Default::default() };
    let server = serve(d);
    assert_eq!(untrusted(ask(&server, &trusting(&["127.0.0.1"]))), "localhost");
    assert!(seen_on(&server.data, "127.0.0.1") > 0, "the address's own Autodiscover was asked");
    assert_eq!(seen_on(&server.data, "localhost"), 0, "nothing went to the other domain");
}

/// An `EwsUrl` on a site the account doesn't trust is not used.
#[test]
fn an_ews_url_on_another_site_is_untrusted() {
    let server = serve(Data::default());
    assert_eq!(untrusted(ask(&server, &trusting(&[]))), "127.0.0.1");
}

/// Allowed, each is followed or used.
#[test]
fn both_work_once_trusted() {
    let server = serve(Data::default());
    assert_eq!(ask(&server, &trusting(&["127.0.0.1"])).unwrap(), server.url);
    server.data.lock().unwrap().redirect_addr = vec![("riley@contoso.example".into(), "riley@localhost".into())];
    assert_eq!(ask(&server, &trusting(&["localhost", "127.0.0.1"])).unwrap(), server.url);
    assert!(seen_on(&server.data, "localhost") > 0, "the redirect was followed, to the other domain's host");
}

/// Exchange hybrid: a `RedirectAddr` to the cloud's domain, then an `EwsUrl` on a third site.
/// Two questions, one after the other, then it works.
#[test]
fn the_hybrid_chain_asks_twice_then_works() {
    let d = Data { redirect_addr: vec![("riley@contoso.example".into(), "riley@localhost".into())], ..Default::default() };
    let server = serve(d);
    let mut allowed: Vec<&str> = Vec::new();
    let mut asked = Vec::new();
    let found = loop {
        match ask(&server, &trusting(&allowed)) {
            Err(NetError::Untrusted { site }) => {
                assert!(asked.len() < 3, "asked again and again: {asked:?}");
                if site == "localhost" {
                    assert_eq!(seen_on(&server.data, "localhost"), 0, "nothing went there before it was allowed");
                }
                asked.push(site.clone());
                allowed.push(if site == "localhost" { "localhost" } else { "127.0.0.1" });
            }
            other => break other.unwrap(),
        }
    };
    assert_eq!(asked, ["localhost", "127.0.0.1"]);
    assert_eq!(found, server.url);
}

/// The owner's case: an on-premises Exchange that Autodiscover found before, on another domain
/// than his address, kept in `ews.json`. It is in service: the account connects, asking nothing.
#[test]
fn an_exchange_account_found_by_autodiscover_before_keeps_working_unasked() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.deliver(&inbox, &mail("Storyboard", "m1"), false, "2024-05-06T09:00:00Z");
    let server = serve(d);
    let store = crate::fake::tests_store("ews-kept-trusted");
    store.save_account_doc("a1", "ews.json", &server.url).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(crate::ews::AUTODISCOVER.into()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    let seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert!(!seen.iter().any(|e| matches!(e, Event::Status { status: SyncStatus::NeedsTrust { .. }, .. })), "{seen:?}");
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 1);
}

/// A message sent while the account waits for the answer stays in the Outbox: it is neither
/// failed nor dropped, and nothing is sent. Allowed, the account starts again and it goes.
#[test]
fn a_message_waits_in_the_outbox_for_the_answer() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Sent Items", None, Some("sentitems"), "IPF.Note");
    let server = serve(d);
    let email = "waits@contoso.example";
    crate::ews::TEST_BASES.lock().unwrap().push((email.into(), server.url.clone()));
    let store = crate::fake::tests_store("ews-outbox-trust");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: email.into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(crate::ews::AUTODISCOVER.into()),
        ..Default::default()
    };
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret(PASSWORD.into())) });
    until(&pool, "the question", |e| matches!(e, Event::Status { status: SyncStatus::NeedsTrust { site }, .. } if site == "127.0.0.1"));
    let raw = format!("From: {email}\r\nTo: zoe@contoso.example\r\nSubject: waits\r\nMessage-ID: <w@x>\r\n\r\nhi\r\n").into_bytes();
    pool.submit(Job::Send {
        account: "a1".into(),
        id: 7,
        raw,
        from: email.into(),
        recipients: vec!["zoe@contoso.example".into()],
        sent_folder: Some("Sent Items".into()),
    });
    let seen = until(&pool, "queued", |e| matches!(e, Event::Queued { id: 7, .. }));
    assert!(!seen.iter().any(|e| matches!(e, Event::SendFailed { .. })), "{seen:?}");
    assert_eq!(store.outbox("a1").iter().map(|i| i.id).collect::<Vec<_>>(), [7], "it waits in the Outbox");
    assert!(server.data.lock().unwrap().sent.is_empty());
    assert!(store.load_account_doc::<String>("a1", "ews.json").unwrap_or_default().is_empty(), "nothing untrusted kept");

    // Allowed: the account starts again, finds its server and sends what waited.
    pool.submit(Job::Start {
        account: AccountConfig { trusted_domains: vec!["127.0.0.1".into()], ..account },
        password: Some(Secret(PASSWORD.into())),
    });
    until(&pool, "sent", |e| matches!(e, Event::Sent { id: 7, .. }));
    assert!(server.data.lock().unwrap().sent.iter().any(|m| String::from_utf8_lossy(m).contains("Subject: waits")));
    eventually("out of the Outbox", || store.outbox("a1").is_empty());
    assert_eq!(store.load_account_doc::<String>("a1", "ews.json").unwrap(), server.url, "the server found is kept");
}
