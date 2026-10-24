//! OAuth 2.0 sign-in for providers that no longer take passwords over IMAP and SMTP (Gmail,
//! Microsoft 365).
//!
//! The authorization-code flow for native apps (RFC 8252): the system browser opens the
//! provider's sign-in page, which redirects to a one-shot listener on `127.0.0.1`; the code is
//! exchanged for tokens with PKCE (RFC 7636), so a stolen code is useless. The refresh token is
//! what the keychain keeps (in place of a password); access tokens are fetched from it as needed
//! and never stored. IMAP and SMTP then sign in with SASL XOAUTH2.
//!
//! Providers only issue tokens to registered apps. The registration (a client id, and for
//! Google a client "secret" that isn't secret for installed apps) belongs to whoever ships
//! Quatrelle: it is read from `QUATRELLE_GOOGLE_CLIENT_ID` / `QUATRELLE_GOOGLE_CLIENT_SECRET` and
//! `QUATRELLE_MICROSOFT_CLIENT_ID`, at run time or baked in at build time. Without one, a
//! provider isn't offered — except that an account can bring its own Microsoft registration
//! (an organisation's single-tenant one: `AccountConfig::oauth_client_id` and `oauth_tenant`),
//! which then stands in for the built-in one, and for `common`, for that account.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use ring::rand::SecureRandom;
use serde_json::Value;

use crate::http::{Request, form, pct};
use crate::{NetError, Result};

/// An OAuth provider's endpoints and the app's registration with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provider {
    /// `google`, `microsoft`.
    pub id: String,
    pub name: String,
    pub auth_url: String,
    pub token_url: String,
    pub scope: String,
    pub client_id: String,
    pub client_secret: Option<String>,
}

fn setting(name: &str, built_in: Option<&'static str>) -> Option<String> {
    std::env::var(name).ok().or_else(|| built_in.map(str::to_string)).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// A provider by id, when Quatrelle has a registration for it.
pub fn provider(id: &str) -> Option<Provider> {
    match id {
        "google" => Some(Provider {
            id: "google".into(),
            name: "Google".into(),
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            scope: "https://mail.google.com/".into(),
            client_id: setting("QUATRELLE_GOOGLE_CLIENT_ID", option_env!("QUATRELLE_GOOGLE_CLIENT_ID"))?,
            client_secret: setting("QUATRELLE_GOOGLE_CLIENT_SECRET", option_env!("QUATRELLE_GOOGLE_CLIENT_SECRET")),
        }),
        "microsoft" => Some(microsoft(setting("QUATRELLE_MICROSOFT_CLIENT_ID", option_env!("QUATRELLE_MICROSOFT_CLIENT_ID"))?, "common")),
        _ => None,
    }
}

/// Microsoft's sign-in for a registration, at a tenant: `common` (any account, the built-in
/// registration's), or an organisation's own directory.
fn microsoft(client_id: String, tenant: &str) -> Provider {
    Provider {
        id: "microsoft".into(),
        name: "Microsoft".into(),
        auth_url: format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize"),
        token_url: format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"),
        scope: "https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send offline_access".into(),
        client_id,
        client_secret: None,
    }
}

pub use quatrelle_model::oauth_provider_for as provider_for_host;

/// The provider an account signs in with: Graph accounts ask Microsoft for Graph's mail
/// permissions instead of IMAP's and SMTP's (a token is for one service), and an account with
/// its own Microsoft registration (an organisation's) signs in with it, at its tenant, in place
/// of the built-in one (the file is hostile input: a client id or tenant that couldn't be one
/// is ignored).
pub fn provider_for_account(account: &quatrelle_model::AccountConfig) -> Option<Provider> {
    let id = account.oauth.as_deref()?;
    let mut p = if id == "microsoft" {
        let own = account.oauth_client_id.as_deref().map(str::trim).filter(|c| quatrelle_model::valid_oauth_client_id(c));
        let client_id = match own {
            Some(c) => c.to_string(),
            None => setting("QUATRELLE_MICROSOFT_CLIENT_ID", option_env!("QUATRELLE_MICROSOFT_CLIENT_ID"))?,
        };
        let tenant = account.oauth_tenant.as_deref().map(str::trim).filter(|t| quatrelle_model::valid_oauth_tenant(t)).unwrap_or("common");
        microsoft(client_id, tenant)
    } else {
        provider(id)?
    };
    if account.graph.is_some() && p.id == "microsoft" {
        p.scope = GRAPH_SCOPE.into();
    }
    Some(p)
}

/// Graph's permissions: the mailbox, its settings, calendars and contacts — and the shared
/// mailboxes and calendars the person was given (`*.Shared`), so those open too. A permission
/// added here makes Microsoft ask everyone to approve the app again on their next sign-in.
const GRAPH_SCOPE: &str = "https://graph.microsoft.com/Mail.ReadWrite https://graph.microsoft.com/Mail.ReadWrite.Shared https://graph.microsoft.com/Mail.Send https://graph.microsoft.com/Mail.Send.Shared https://graph.microsoft.com/MailboxSettings.ReadWrite https://graph.microsoft.com/Calendars.ReadWrite https://graph.microsoft.com/Calendars.ReadWrite.Shared https://graph.microsoft.com/Contacts.ReadWrite offline_access";

/// The providers with a registration, so sign-in can be offered.
pub fn available() -> Vec<String> {
    ["google", "microsoft"].into_iter().filter(|id| provider(id).is_some()).map(str::to_string).collect()
}

/// Base64url without padding (RFC 4648 §5), as PKCE wants.
pub fn b64url(data: &[u8]) -> String {
    crate::imap::base64(data).replace('+', "-").replace('/', "_").trim_end_matches('=').to_string()
}

fn random(n: usize) -> Result<Vec<u8>> {
    let mut v = vec![0u8; n];
    ring::rand::SystemRandom::new().fill(&mut v).map_err(|_| NetError::Protocol("no randomness available".into()))?;
    Ok(v)
}

/// 32 bytes from the system's secure random source, base64url: a key nobody can guess (the
/// app's single-instance socket asks it of every request).
pub fn secret_token() -> Result<String> {
    Ok(b64url(&random(32)?))
}

/// A PKCE verifier and its S256 challenge.
pub fn pkce() -> Result<(String, String)> {
    let verifier = b64url(&random(32)?);
    let challenge = b64url(ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes()).as_ref());
    Ok((verifier, challenge))
}

/// Tokens from the provider.
#[derive(Clone, PartialEq, Eq)]
pub struct Tokens {
    pub access: String,
    pub refresh: Option<String>,
    pub expires_at: Instant,
    /// The permissions the provider says it granted (Microsoft lists them; empty when not said).
    pub granted: String,
    /// Who signed in, from the ID token (its `email`, else `preferred_username`): only after a
    /// browser sign-in that asked for it.
    pub user: Option<String>,
}

/// What the browser sign-in asks for besides the mail permissions: who signed in (an ID token),
/// so a mailbox that won't open can be told apart from another account picked on the page.
/// Microsoft only: refreshing asks for the mail permissions alone, as tokens granted before
/// this was asked for still have them.
fn identity_scope(p: &Provider) -> &'static str {
    if p.id == "microsoft" { " openid email" } else { "" }
}

/// The address in an ID token (a JWT: its middle part is base64url JSON). It came straight from
/// the provider over TLS, so it isn't checked further; it's only ever compared with the account.
pub fn id_token_user(id_token: &str) -> Option<String> {
    let claims = crate::ews::unb64(id_token.split('.').nth(1)?)?;
    let v: Value = serde_json::from_slice(&claims).ok()?;
    ["email", "preferred_username"]
        .iter()
        .filter_map(|k| v.get(*k).and_then(Value::as_str))
        .map(str::trim)
        .find(|a| a.contains('@') && a.len() < 320)
        .map(str::to_string)
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Tokens(<redacted>)")
    }
}

fn token_request(p: &Provider, mut fields: Vec<(&str, String)>, extra_scope: &str) -> Result<Tokens> {
    fields.push(("client_id", p.client_id.clone()));
    // The permissions wanted, each time: without them a personal Microsoft account's token
    // covered reading (IMAP) but not sending (SMTP refused it, 535 5.7.3). Google takes none here.
    if p.id != "google" {
        fields.push(("scope", format!("{}{extra_scope}", p.scope)));
    }
    if let Some(s) = &p.client_secret {
        fields.push(("client_secret", s.clone()));
    }
    let pairs: Vec<(&str, &str)> = fields.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let r = crate::http::send(
        &Request::post(&p.token_url, "application/x-www-form-urlencoded", form(&pairs), 1 << 20).header("Accept", "application/json"),
    )?;
    let v: Value =
        serde_json::from_slice(&r.body).map_err(|_| NetError::Protocol(format!("the sign-in service answered HTTP {} without JSON", r.status)))?;
    if !r.ok() || v.get("error").is_some() {
        let what = v.get("error_description").or_else(|| v.get("error")).and_then(Value::as_str).unwrap_or("refused");
        // A revoked or expired grant means signing in again.
        return Err(NetError::Auth(format!("{}: {what}", p.name)));
    }
    let access = v
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| NetError::Protocol("no access token in the answer".into()))?;
    let expires = v.get("expires_in").and_then(Value::as_u64).unwrap_or(3600).clamp(60, 86_400);
    Ok(Tokens {
        access: access.to_string(),
        refresh: v.get("refresh_token").and_then(Value::as_str).map(str::to_string),
        granted: v.get("scope").and_then(Value::as_str).unwrap_or_default().to_string(),
        user: v.get("id_token").and_then(Value::as_str).and_then(id_token_user),
        // A minute early, so a token never runs out mid-command.
        expires_at: Instant::now() + Duration::from_secs(expires - 60),
    })
}

/// A fresh access token from a refresh token.
pub fn refresh(p: &Provider, refresh_token: &str) -> Result<Tokens> {
    token_request(p, vec![("grant_type", "refresh_token".into()), ("refresh_token", refresh_token.into())], "")
}

/// The query parameters of a request target (`/?code=…&state=…`).
fn query_params(target: &str) -> Vec<(String, String)> {
    let q = target.split_once('?').map_or("", |(_, q)| q);
    q.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (unpct(k), unpct(&v.replace('+', " ")))).collect()
}

pub(crate) fn unpct(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match (b.get(i), b.get(i + 1..i + 3)) {
            (Some(b'%'), Some(h)) if std::str::from_utf8(h).ok().and_then(|h| u8::from_str_radix(h, 16).ok()).is_some() => {
                out.push(std::str::from_utf8(h).ok().and_then(|h| u8::from_str_radix(h, 16).ok()).unwrap_or(b'?'));
                i += 3;
            }
            (Some(c), _) => {
                out.push(*c);
                i += 1;
            }
            (None, _) => break,
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Run the browser sign-in: `open` shows the provider's page (the system browser), the
/// redirect comes back to a listener on 127.0.0.1, and the code becomes tokens.
pub fn authorize(p: &Provider, login_hint: &str, open: &mut dyn FnMut(&str) -> std::result::Result<(), String>, timeout: Duration) -> Result<Tokens> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let redirect = format!("http://127.0.0.1:{}/", listener.local_addr()?.port());
    let state = b64url(&random(16)?);
    let (verifier, challenge) = pkce()?;
    let mut url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={state}&code_challenge={challenge}&code_challenge_method=S256",
        p.auth_url,
        pct(&p.client_id),
        pct(&redirect),
        pct(&format!("{}{}", p.scope, identity_scope(p)))
    );
    if !login_hint.is_empty() {
        url.push_str(&format!("&login_hint={}", pct(login_hint)));
    }
    if p.id == "google" {
        // A refresh token, every time.
        url.push_str("&access_type=offline&prompt=consent");
    }
    open(&url).map_err(|e| NetError::Protocol(format!("the browser couldn't be opened: {e}")))?;
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + timeout;
    let code = loop {
        if Instant::now() > deadline {
            return Err(NetError::Auth("the sign-in wasn't finished in time".into()));
        }
        let (sock, _) = match listener.accept() {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        sock.set_nonblocking(false)?;
        sock.set_read_timeout(Some(Duration::from_secs(10)))?;
        let mut line = String::new();
        BufReader::new(sock.try_clone()?).take(8192).read_line(&mut line)?;
        let target = line.split_whitespace().nth(1).unwrap_or("");
        let params = query_params(target);
        let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        // Stray requests (a favicon) are ignored; only the redirect with our state counts.
        if get("state").as_deref() != Some(state.as_str()) {
            let _ = (&sock).write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            continue;
        }
        let (status, page) = match (get("code"), get("error")) {
            (Some(_), _) => ("200 OK", "You're signed in to Quatrelle. You can close this window."),
            _ => ("400 Bad Request", "Quatrelle wasn't allowed to sign in. You can close this window and try again."),
        };
        let body = format!("<!doctype html><meta charset=utf-8><title>Quatrelle</title><p style=\"font:16px sans-serif;margin:3em\">{page}</p>");
        let _ = (&sock).write_all(
            format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        );
        match (get("code"), get("error")) {
            (Some(c), _) => break c,
            (None, e) => return Err(NetError::Auth(format!("{} sign-in was cancelled ({})", p.name, e.unwrap_or_default()))),
        }
    };
    token_request(
        p,
        vec![("grant_type", "authorization_code".into()), ("code", code), ("redirect_uri", redirect), ("code_verifier", verifier)],
        identity_scope(p),
    )
}

/// The SASL XOAUTH2 initial response (base64 of `user=…^Aauth=Bearer …^A^A`).
pub fn xoauth2(user: &str, token: &str) -> String {
    crate::imap::base64(format!("user={user}\x01auth=Bearer {token}\x01\x01").as_bytes())
}

use std::io::Read as _;

#[cfg(test)]
pub(crate) mod tests {
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

    #[test]
    fn browser_sign_in_then_refresh() {
        let json = |status: &str, body: &str| {
            format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
        };
        let (base, seen) = crate::http::tests::serve(vec![
            json("200 OK", r#"{"access_token":"good-token","refresh_token":"r1","expires_in":3600}"#),
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
        assert!(opened.contains("code_challenge_method=S256") && opened.contains("login_hint=zoe%40x.example"), "{opened}");
        let t = refresh(&p, "r1").unwrap();
        assert_eq!((t.access.as_str(), t.refresh), ("fresh", None));
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
}
