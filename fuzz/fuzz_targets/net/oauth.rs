// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! OAuth 2.0 sign-in: a token endpoint's answer (its JSON, error codes, `expires_in`, the ID
//! token's claims) read as hostile, and the request line a browser sends back to the loopback
//! redirect (its query string: `code`, `state`, `error`, and Microsoft's `AADSTS` code alone
//! kept from its words). A busy answer stays a connection failure, a refusal an `Err`; the
//! access token is never empty and its expiry stays within a day.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_net::oauth::{id_token_claim, id_token_user, now_unix, redirect_read, tokens_of_answer};

/// The first two bytes as an HTTP status (`200` when there are none).
fn status_of(data: &[u8]) -> u16 {
    match data {
        [a, b, ..] => u16::from_be_bytes([*a, *b]),
        [a] => u16::from(*a),
        [] => 200,
    }
}

fuzz_target!(|data: &[u8]| {
    // The redirect's request line: the parameters read, the `AADSTS` code in a refusal.
    let line = String::from_utf8_lossy(data);
    let (params, code) = redirect_read(&line);
    assert!(params.len() <= line.len() + 1);
    for (k, v) in &params {
        assert!(k.len() <= line.len() && v.len() <= line.len().saturating_mul(3), "{k:?} {v:?}");
    }
    assert!(code.as_ref().is_none_or(|c| c.len() <= 12 && c.starts_with("AADSTS")));
    // The whole line as a target too (a server's own answer read the same way).
    let _ = redirect_read(&format!("GET {line} HTTP/1.1"));

    // The token endpoint's answer, for every kind of status it may give.
    let body = data.get(2..).unwrap_or_default();
    for status in [status_of(data), 200, 201, 400, 401, 429, 500, 503] {
        match tokens_of_answer(status, body, "Test") {
            Ok(t) => {
                assert!(!t.access.is_empty());
                let now = now_unix();
                assert!(t.expires_at >= now.saturating_sub(2) && t.expires_at <= now + 86_400, "{}", t.expires_at);
                assert!(t.granted.len() <= body.len() && t.user.as_ref().is_none_or(|u| u.len() < 320));
                assert!(t.tenant.as_ref().is_none_or(|tt| tt.len() <= body.len()));
                assert!(t.refresh.as_ref().is_none_or(|r| r.len() <= body.len()));
            }
            // Busy or failing is the connection's, never a refusal; a refusal names the provider.
            Err(quatrelle_net::NetError::Closed(m)) => assert_eq!(m, format!("HTTP {status}"), "{status}"),
            Err(quatrelle_net::NetError::Auth(m)) => assert!(m.starts_with("Test: "), "{m:?}"),
            Err(quatrelle_net::NetError::Protocol(_)) => {}
            Err(e) => panic!("{e:?}"),
        }
    }

    // An ID token is a JWT whose middle part is base64url JSON; anything else is no claim.
    let rest = String::from_utf8_lossy(data.get(1..).unwrap_or_default());
    let (line_text, rest_text): (&str, &str) = (&line, &rest);
    for s in [line_text, rest_text, "e30.e30.sig"] {
        for claim in ["tid", "email", "preferred_username"] {
            if let Some(v) = id_token_claim(s, claim) {
                assert!(v.len() <= s.len().saturating_mul(3));
            }
        }
        if let Some(u) = id_token_user(s) {
            assert!(u.len() < 320 && u.contains('@'), "{u:?}");
        }
    }
});
