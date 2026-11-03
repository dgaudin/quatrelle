// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The session's URLs are the server's to give: the account's credentials go only to the sites it
//! trusts. `127.0.0.1` and `localhost` reach the same fake server as two sites.

use super::*;
use crate::NetError;

/// The requests the server saw on a host.
fn seen_on(server: &Server, host: &str) -> Vec<String> {
    server.data.lock().unwrap().requests.iter().filter(|r| r.split(' ').next() == Some(host)).cloned().collect()
}

fn untrusted(r: crate::Result<crate::jmap::Client>) -> String {
    match r {
        Err(NetError::Untrusted { site }) => site,
        Err(e) => panic!("not Untrusted: {e:?}"),
        Ok(_) => panic!("connected"),
    }
}

/// A well-known address that redirects within its site, as a provider's does: the credentials go
/// along and the session's relative URLs resolve against where it ended.
#[test]
fn a_session_within_the_site_connects() {
    let mut d = Data::default();
    d.mailbox("Inbox", None, Some("inbox"));
    d.redirect_session = Some("127.0.0.1".into());
    let server = serve(d);
    let c = crate::jmap::Client::connect(&server.base, USER, PASSWORD, false, &[]).unwrap();
    let mut st = crate::jmap::State::default();
    assert_eq!(crate::jmap::list_folders(&c, &mut st, "a1").unwrap().len(), 1);
    let seen = seen_on(&server, "127.0.0.1");
    assert!(seen.iter().any(|r| r.ends_with(" /session")), "{seen:?}");
    assert!(seen_on(&server, "localhost").is_empty());
}

/// An `apiUrl` on a site the account doesn't trust: refused at connect, before the token or the
/// password goes there (the server saw nothing on that site, and no method call).
#[test]
fn a_session_url_on_another_site_is_refused_before_anything_is_sent() {
    let mut d = Data::default();
    d.mailbox("Inbox", None, Some("inbox"));
    d.elsewhere = vec!["apiUrl"];
    let server = serve(d);
    assert_eq!(untrusted(crate::jmap::Client::connect(&server.base, USER, PASSWORD, false, &[])), "localhost");
    assert!(seen_on(&server, "localhost").is_empty());
    assert!(server.data.lock().unwrap().calls.is_empty());
    // The same for the push address.
    server.data.lock().unwrap().elsewhere = vec!["eventSourceUrl"];
    assert_eq!(untrusted(crate::jmap::Client::connect(&server.base, USER, PASSWORD, false, &[])), "localhost");
    // And a well-known address redirected to another site: the credentials stay behind, and the
    // refusal it gets there says which site, not "wrong password".
    let server = serve(Data { redirect_session: Some("localhost".into()), ..Default::default() });
    assert_eq!(untrusted(crate::jmap::Client::connect(&server.base, USER, PASSWORD, false, &[])), "localhost");
    // Trusted, it goes on with them.
    assert!(crate::jmap::Client::connect(&server.base, USER, PASSWORD, false, &["localhost".into()]).is_ok());
}

/// Fastmail's case: the downloads are on another site (`fastmailusercontent.com`). Untrusted
/// until the account allows it, then the message sources come from there, signed in.
#[test]
fn a_download_url_on_another_site_is_untrusted_then_works_once_trusted() {
    let mut d = Data::default();
    let inbox = d.mailbox("Inbox", None, Some("inbox"));
    d.deliver(&inbox, &mail("Hello", "h1"), &[], "2024-05-01T09:00:00Z");
    d.elsewhere = vec!["downloadUrl"];
    let server = serve(d);
    assert_eq!(untrusted(crate::jmap::Client::connect(&server.base, USER, PASSWORD, false, &[])), "localhost");
    assert!(seen_on(&server, "localhost").is_empty());

    let c = crate::jmap::Client::connect(&server.base, USER, PASSWORD, false, &["localhost".into()]).unwrap();
    let dir = std::env::temp_dir().join(format!("quatrelle-jmap-trust-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut st = crate::jmap::State::default();
    crate::jmap::list_folders(&c, &mut st, "a1").unwrap();
    let opts = crate::sync::SyncOptions { window: 100, prefetch: 0, ..Default::default() };
    crate::jmap::sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let key = store.load_folder("a1", "INBOX").unwrap().messages.first().unwrap().key.clone();
    let raw = crate::jmap::fetch_body(&c, &st, &store, &key).unwrap();
    assert!(String::from_utf8_lossy(&raw).contains("Subject: Hello"));
    let seen = seen_on(&server, "localhost");
    assert!(seen.iter().any(|r| r.contains(" /download/")), "{seen:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The desktop's HTTP client says where an answer came from once the redirects were followed.
#[test]
fn the_final_url_is_reported_natively() {
    use quatrelle_jmap::Http as _;
    let d = Data { redirect_session: Some("127.0.0.1".into()), ..Default::default() };
    let server = serve(d);
    let req = quatrelle_jmap::Request::get(&format!("{}/.well-known/jmap", server.base), 1 << 20);
    let resp = crate::jmap::block_on(crate::jmap::Blocking.send(req)).unwrap();
    assert_eq!(resp.status, 401);
    assert_eq!(resp.url.as_deref(), Some(format!("{}/session", server.base).as_str()));
}
