// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The OAuth sign-in's tests.

use super::*;

/// A provider whose endpoints are local test servers.
pub(crate) fn local(token_url: &str) -> Provider {
    Provider {
        id: "test".into(),
        name: "Test".into(),
        auth_url: "http://127.0.0.1:9/auth".into(),
        token_url: token_url.into(),
        scope: "mail".into(),
        client_id: "quatrelle-test".into(),
        client_secret: None,
    }
}

#[test]
fn pkce_and_helpers() {
    let (v, c) = pkce().unwrap();
    assert!(v.len() >= 43 && !v.contains(['+', '/', '=']));
    assert_eq!(c, b64url(ring::digest::digest(&ring::digest::SHA256, v.as_bytes()).as_ref()));
    // RFC 7636 appendix B.
    assert_eq!(
        b64url(ring::digest::digest(&ring::digest::SHA256, b"dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk").as_ref()),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
    assert_eq!(xoauth2("a@b.c", "tok"), crate::imap::base64(b"user=a@b.c\x01auth=Bearer tok\x01\x01"));
    assert_eq!(query_params("/?code=4%2F0A&state=s+1"), vec![("code".to_string(), "4/0A".to_string()), ("state".into(), "s 1".into())]);
    assert_eq!(provider_for_host("IMAP.gmail.com"), Some("google"));
    assert_eq!(provider_for_host("outlook.office365.com"), Some("microsoft"));
    assert_eq!(provider_for_host("mail.example.org"), None);
    assert!(provider("nope").is_none());
    // Who signed in, from an ID token's claims.
    let token = |claims: &str| format!("e30.{}.sig", b64url(claims.as_bytes()));
    assert_eq!(
        id_token_user(&token(r#"{"preferred_username":"Zoe@Live.example","email":"zoe@outlook.example"}"#)).as_deref(),
        Some("zoe@outlook.example")
    );
    assert_eq!(id_token_user(&token(r#"{"preferred_username":"zoe@live.example"}"#)).as_deref(), Some("zoe@live.example"));
    assert_eq!(id_token_user(&token(r#"{"name":"Zoe"}"#)), None);
    for junk in ["", "a", "a.b.c", "a.!!!.c", "a.e30"] {
        assert_eq!(id_token_user(junk), None, "{junk}");
    }
}

/// An organisation's own Microsoft registration goes with the account: its client id, at
/// its tenant, in place of the built-in one and `common`; Graph asks for the shared
/// mailboxes' permissions too. A tenant or client id that couldn't be one is ignored.
#[test]
fn an_account_brings_its_own_microsoft_registration() {
    let mut a = quatrelle_model::AccountConfig::guess("zoe@contoso.example");
    a.oauth = Some("microsoft".into());
    a.oauth_client_id = Some(" 11111111-2222-3333-4444-555555555555 ".into());
    a.oauth_tenant = Some("contoso.onmicrosoft.com".into());
    let p = provider_for_account(&a).unwrap();
    assert_eq!(p.client_id, "11111111-2222-3333-4444-555555555555");
    assert_eq!(p.auth_url, "https://login.microsoftonline.com/contoso.onmicrosoft.com/oauth2/v2.0/authorize");
    assert_eq!(p.token_url, "https://login.microsoftonline.com/contoso.onmicrosoft.com/oauth2/v2.0/token");
    assert!(p.scope.contains("IMAP.AccessAsUser.All") && p.client_secret.is_none());
    a.graph = Some(quatrelle_model::GRAPH_URL.into());
    let p = provider_for_account(&a).unwrap();
    for s in ["Mail.ReadWrite ", "Mail.ReadWrite.Shared", "Mail.Send ", "Mail.Send.Shared", "Calendars.ReadWrite.Shared", "offline_access"] {
        assert!(p.scope.contains(s), "{s}: {}", p.scope);
    }
    a.oauth_tenant = Some("not a tenant".into());
    assert!(provider_for_account(&a).unwrap().auth_url.contains("/common/"), "a tenant that couldn't be one: common");
    a.oauth_client_id = Some("not a client id!".into());
    assert_eq!(provider_for_account(&a).is_some(), provider("microsoft").is_some(), "a client id that couldn't be one: the built-in");
    a.oauth = Some("google".into());
    assert_eq!(provider_for_account(&a).is_some(), provider("google").is_some(), "Google takes no registration from the account");
}

/// Teams signs in with the organisation's own registration, at its tenant, for Teams'
/// Graph permissions (the id stays `microsoft`: the ID token is asked for too).
#[test]
fn teams_provider_signs_in_at_the_tenant_with_teams_scopes() {
    let p = teams_provider("11111111-2222-3333-4444-555555555555", "contoso.onmicrosoft.com", false).unwrap();
    assert_eq!(p.auth_url, "https://login.microsoftonline.com/contoso.onmicrosoft.com/oauth2/v2.0/authorize");
    assert_eq!(p.token_url, "https://login.microsoftonline.com/contoso.onmicrosoft.com/oauth2/v2.0/token");
    assert_eq!(p.scope, TEAMS_SCOPE);
    assert_eq!(p.client_secret, None);
    assert_eq!(p.id, "microsoft");
    assert_eq!(p.name, "Microsoft Teams");
    assert_eq!(p.client_id, "11111111-2222-3333-4444-555555555555");
    assert_eq!(identity_scope(&p), " openid email");
    // Surrounding blanks (a pasted value) are not part of the id.
    assert!(teams_provider(" 11111111-2222-3333-4444-555555555555 ", " contoso.com ", false).unwrap().auth_url.contains("/contoso.com/"));
}

#[test]
fn teams_provider_refuses_what_cant_be_an_id() {
    let valid = "11111111-2222-3333-4444-555555555555";
    assert_eq!(teams_provider("not a guid", "contoso.com", false), None);
    assert_eq!(teams_provider(valid, "a/b", false), None);
    assert_eq!(teams_provider(valid, "", false), None);
}

/// The permission to make Teams meetings is asked for apart, only when the user wants meetings:
/// never part of the chats' own permissions, which a refresh token without it would otherwise
/// fail every request of until a new sign-in.
#[test]
fn the_meetings_permission_is_asked_only_when_wanted() {
    let (id, tenant) = ("11111111-2222-3333-4444-555555555555", "contoso.onmicrosoft.com");
    assert!(!TEAMS_SCOPE.contains("OnlineMeetings"), "{TEAMS_SCOPE}");
    assert_eq!(teams_provider(id, tenant, false).unwrap().scope, TEAMS_SCOPE);
    let wide = teams_provider(id, tenant, true).unwrap();
    assert_eq!(wide.scope, format!("{TEAMS_SCOPE} https://graph.microsoft.com/OnlineMeetings.ReadWrite"));
    assert_eq!(TEAMS_MEETINGS_SCOPE, "https://graph.microsoft.com/OnlineMeetings.ReadWrite");
    // Everything else is the chats' sign-in, unchanged.
    assert_eq!(Provider { scope: TEAMS_SCOPE.into(), ..wide }, teams_provider(id, tenant, false).unwrap());
    assert_eq!(teams_provider("not a guid", tenant, true), None, "ids that can't be ones: refused all the same");
}

#[test]
fn id_token_claim_reads_tid_and_survives_junk() {
    let token = |claims: &str| format!("e30.{}.sig", b64url(claims.as_bytes()));
    let t = token(r#"{"tid":"t-1","email":"a@b.example"}"#);
    assert_eq!(id_token_claim(&t, "tid").as_deref(), Some("t-1"));
    assert_eq!(id_token_claim(&t, "email").as_deref(), Some("a@b.example"));
    assert_eq!(id_token_claim(&t, "oid"), None);
    assert_eq!(id_token_claim(&token(r#"{"tid":42}"#), "tid"), None, "a number is not a string claim");
    assert_eq!(id_token_claim(&token(r#"[1,2]"#), "tid"), None, "a payload that isn't an object");
    for junk in ["", "x", "a.b.c", "a.!!!.c", "a.e30"] {
        assert_eq!(id_token_claim(junk, "tid"), None, "{junk}");
    }
}

#[test]
fn browser_sign_in_then_refresh() {
    let json = |status: &str, body: &str| {
        format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    };
    let (base, seen) = crate::http::tests::serve(vec![
        json(
            "200 OK",
            &format!(
                r#"{{"access_token":"good-token","refresh_token":"r1","expires_in":3600,"id_token":"e30.{}.sig"}}"#,
                b64url(br#"{"tid":"tenant-1","email":"zoe@x.example"}"#)
            ),
        ),
        json("200 OK", r#"{"access_token":"fresh","expires_in":3600}"#),
        json("400 Bad Request", r#"{"error":"invalid_grant"}"#),
    ]);
    let p = local(&format!("{base}/token"));
    // The "browser": follow the redirect with a code, after a stray request.
    let mut opened = String::new();
    let mut browser = |url: &str| -> std::result::Result<(), String> {
        opened = url.to_string();
        let params = query_params(url);
        let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
        let redirect = get("redirect_uri");
        let state = get("state");
        std::thread::spawn(move || {
            let _ = crate::http::get(&format!("{redirect}favicon.ico"), 100);
            let _ = crate::http::get(&format!("{redirect}?code=the-code&state={state}"), 10_000);
        });
        Ok(())
    };
    let t = authorize(&p, "zoe@x.example", &mut browser, Duration::from_secs(10)).unwrap();
    assert_eq!((t.access.as_str(), t.refresh.as_deref()), ("good-token", Some("r1")));
    // The tenant and the address come from the ID token (the refresh below carries none).
    assert_eq!((t.tenant.as_deref(), t.user.as_deref()), (Some("tenant-1"), Some("zoe@x.example")));
    assert!(opened.contains("code_challenge_method=S256") && opened.contains("login_hint=zoe%40x.example"), "{opened}");
    let t = refresh(&p, "r1").unwrap();
    // Expiry is wall-clock time (it goes on while the computer sleeps), a minute early.
    assert!((now_unix() + 3500..=now_unix() + 3540).contains(&t.expires_at), "{}", t.expires_at);
    assert!(!t.expired() && Tokens { expires_at: now_unix() - 1, ..t.clone() }.expired());
    assert_eq!((t.access.as_str(), t.refresh, t.tenant), ("fresh", None, None));
    assert!(matches!(refresh(&p, "revoked"), Err(NetError::Auth(m)) if m.contains("invalid_grant")));
    let seen = seen.join().unwrap();
    assert!(
        seen[0].contains("grant_type=authorization_code") && seen[0].contains("code=the-code") && seen[0].contains("code_verifier="),
        "{}",
        seen[0]
    );
    assert!(seen[1].contains("grant_type=refresh_token&refresh_token=r1"));
    // Both ask for the permissions again (a Microsoft token for IMAP alone can't send).
    assert!(seen[0].contains("scope=mail") && seen[1].contains("scope=mail"), "{}", seen[1]);
    // A denied sign-in is an error, not a hang.
    let mut deny = |url: &str| -> std::result::Result<(), String> {
        let params = query_params(url);
        let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
        let (redirect, state) = (get("redirect_uri"), get("state"));
        std::thread::spawn(move || {
            let _ = crate::http::get(&format!("{redirect}?error=access_denied&state={state}"), 10_000);
        });
        Ok(())
    };
    assert!(matches!(authorize(&p, "", &mut deny, Duration::from_secs(10)), Err(NetError::Auth(m)) if m.contains("access_denied")));
}

/// Final review I3: the sign-in service busy or failing (429, 5xx, `temporarily_unavailable`,
/// `server_error`) is the connection's failure, tried again: never a refusal that asks to sign
/// in again.
#[test]
fn a_sign_in_service_busy_or_failing_is_not_a_refusal() {
    let json = |status: &str, kind: &str, body: &str| {
        format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    };
    let (base, seen) = crate::http::tests::serve(vec![
        json("503 Service Unavailable", "application/json", r#"{"error":"temporarily_unavailable","error_description":"AADSTS90033: busy"}"#),
        json("500 Internal Server Error", "application/json", r#"{"error":"server_error"}"#),
        json("429 Too Many Requests", "application/json", r#"{"error":"invalid_request"}"#),
        json("400 Bad Request", "application/json", r#"{"error":"temporarily_unavailable"}"#),
        json("502 Bad Gateway", "text/html", "<html>down</html>"),
        json("200 OK", "application/json", r#"{"access_token":"fresh","expires_in":3600}"#),
    ]);
    let p = local(&format!("{base}/token"));
    for want in ["HTTP 503", "HTTP 500", "HTTP 429", "HTTP 400", "HTTP 502"] {
        match refresh(&p, "r1") {
            Err(NetError::Closed(m)) => assert_eq!(m, want),
            other => panic!("{want}: {other:?}"),
        }
    }
    assert_eq!(refresh(&p, "r1").unwrap().access, "fresh", "then renewed");
    seen.join().unwrap();
}

/// The address a sign-in's redirect listens on, from the URL the browser is sent to.
fn listening_on(url: &str) -> String {
    let params = query_params(url);
    let redirect = params.iter().find(|(n, _)| n == "redirect_uri").map(|(_, v)| v.clone()).unwrap_or_default();
    redirect.trim_start_matches("http://").split('/').next().unwrap_or("").to_string()
}

/// A connection that sends nothing readable (a browser's early one) doesn't end the sign-in:
/// the redirect behind it still counts.
#[test]
fn a_connection_that_says_nothing_readable_is_skipped() {
    let (base, _seen) = crate::http::tests::serve(vec![
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 45\r\n\r\n{\"access_token\":\"good\",\"expires_in\":3600}      "
            .to_vec(),
    ]);
    let p = local(&format!("{base}/token"));
    let mut browser = |url: &str| -> std::result::Result<(), String> {
        let params = query_params(url);
        let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
        let (redirect, state, at) = (get("redirect_uri"), get("state"), listening_on(url));
        std::thread::spawn(move || {
            if let Ok(mut junk) = std::net::TcpStream::connect(&at) {
                let _ = junk.write_all(b"\xff\xfe\n");
            }
            let _ = crate::http::get(&format!("{redirect}?code=the-code&state={state}"), 10_000);
        });
        Ok(())
    };
    assert!(authorize(&p, "", &mut browser, Duration::from_secs(10)).is_ok());
}

/// Connections that stay silent can't hold the wait past its timeout.
#[test]
fn silent_connections_cannot_outlast_the_timeout() {
    let p = local("http://127.0.0.1:9/token");
    let held = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let keep = held.clone();
    let mut browser = move |url: &str| -> std::result::Result<(), String> {
        let at = listening_on(url);
        for _ in 0..3 {
            if let Ok(c) = std::net::TcpStream::connect(&at)
                && let Ok(mut h) = keep.lock()
            {
                h.push(c);
            }
        }
        Ok(())
    };
    let started = Instant::now();
    assert!(matches!(authorize(&p, "", &mut browser, Duration::from_secs(1)), Err(NetError::Auth(m)) if m.contains("in time")));
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    drop(held);
}

#[test]
fn a_refused_redirect_keeps_its_aadsts_code_only() {
    // A tenant that forbids user consent: `access_denied`, the reason in the description,
    // which also names the account. The code is kept; Microsoft's words aren't.
    let p = local("http://127.0.0.1:9/token");
    let mut deny = |url: &str| -> std::result::Result<(), String> {
        let params = query_params(url);
        let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
        let (redirect, state) = (get("redirect_uri"), get("state"));
        std::thread::spawn(move || {
            let description = "AADSTS65004%3A+User+ana%40contoso.example+declined+to+consent+to+access+the+app.+Trace+ID%3A+x";
            let _ = crate::http::get(&format!("{redirect}?error=access_denied&error_description={description}&state={state}"), 10_000);
        });
        Ok(())
    };
    match authorize(&p, "", &mut deny, Duration::from_secs(10)) {
        Err(NetError::Auth(m)) => assert_eq!(m, format!("{} sign-in was cancelled (access_denied, AADSTS65004)", p.name)),
        other => panic!("{:?}", other.map(|t| t.access)),
    }
}

#[test]
fn webexs_token_request_carries_the_secret_and_no_scope() {
    let body = r#"{"access_token":"fresh","expires_in":3600}"#;
    let answer = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    let (base, seen) = crate::http::tests::serve(vec![answer.into_bytes()]);
    let mut p = crate::webex::provider("C1", "s3").unwrap();
    p.token_url = format!("{base}/token");
    assert_eq!(refresh(&p, "r1").unwrap().access, "fresh");
    let seen = seen.join().unwrap();
    assert!(seen[0].contains("client_secret=s3") && seen[0].contains("client_id=C1"), "{}", seen[0]);
    assert!(!seen[0].contains("scope"), "{}", seen[0]);
}

/// Webex's client secret is a real one: a provider in a log or a panic doesn't show it.
#[test]
fn a_providers_debug_hides_its_secret() {
    let mut p = crate::webex::provider("C1", "s3").unwrap();
    let shown = format!("{p:?}");
    assert!(!shown.contains("s3"), "{shown}");
    assert!(shown.contains("webex") && shown.contains("C1") && shown.contains("client_secret: Some(\"<redacted>\")"), "{shown}");
    p.client_secret = None;
    assert!(format!("{p:?}").contains("client_secret: None"));
}

#[test]
fn sign_in_listens_on_the_port_asked_for() {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut opened = String::new();
    // The address it hands the browser, and something there to answer: then no further.
    let mut browser = |url: &str| -> std::result::Result<(), String> {
        opened = url.to_string();
        std::net::TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
        Err("no browser here".into())
    };
    let r = authorize_on(&local("http://127.0.0.1:9/token"), port, "", &mut browser, Duration::from_secs(1));
    assert!(matches!(r, Err(NetError::Protocol(m)) if m.contains("no browser here")), "the listener was there");
    assert!(opened.contains(&format!("redirect_uri=http%3A%2F%2F127.0.0.1%3A{port}%2F")), "{opened}");
}

#[test]
fn a_port_in_use_says_so() {
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let mut browser = |_: &str| -> std::result::Result<(), String> { panic!("nothing to open a browser for") };
    match authorize_on(&local("http://127.0.0.1:9/token"), port, "", &mut browser, Duration::from_secs(1)) {
        Err(NetError::Auth(m)) => assert_eq!(m, format!("port {port} is in use: a sign-in may still be waiting in the browser")),
        other => panic!("{:?}", other.map(|t| t.access)),
    }
}
