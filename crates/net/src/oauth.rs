// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
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

use std::io::{ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ring::rand::SecureRandom;
use serde_json::Value;

use crate::http::{Request, form, pct};
use crate::{NetError, Result};

/// The longest request line read from the redirect listener.
const REQUEST_LINE_MAX: usize = 8192;

/// The most read of one request to the redirect listener: its line and its headers.
const REQUEST_MAX: usize = 64 * 1024;

/// An OAuth provider's endpoints and the app's registration with it.
#[derive(Clone, PartialEq, Eq)]
pub struct Provider {
    /// `google`, `microsoft`, `webex`.
    pub id: String,
    pub name: String,
    pub auth_url: String,
    pub token_url: String,
    pub scope: String,
    pub client_id: String,
    /// Google's isn't secret, for an installed app; Webex's is, and stays out of `Debug`.
    pub client_secret: Option<String>,
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("auth_url", &self.auth_url)
            .field("token_url", &self.token_url)
            .field("scope", &self.scope)
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.as_ref().map(|_| "<redacted>"))
            .finish()
    }
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
        other => test_provider(other),
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

#[cfg(not(test))]
fn test_provider(_id: &str) -> Option<Provider> {
    None
}

/// Providers the tests register, with local token endpoints.
#[cfg(test)]
pub(crate) static TEST_PROVIDERS: std::sync::Mutex<Vec<Provider>> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
fn test_provider(id: &str) -> Option<Provider> {
    TEST_PROVIDERS.lock().ok()?.iter().find(|p| p.id == id).cloned()
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

/// Teams' permissions, through Graph: who the person is and the people around them, their chats
/// (read and write), presence, and the files shared in conversations. They are the
/// organisation's to approve (its administrator may have to): see `docs/teams.md`.
pub const TEAMS_SCOPE: &str = "https://graph.microsoft.com/User.Read https://graph.microsoft.com/User.ReadBasic.All https://graph.microsoft.com/Chat.ReadWrite https://graph.microsoft.com/Presence.Read.All https://graph.microsoft.com/Files.ReadWrite https://graph.microsoft.com/Files.Read.All offline_access";

/// The permission to make Teams meetings (online meetings of the user's own). Never part of
/// [`TEAMS_SCOPE`]: asked for apart, only when the user wants meetings, as a refresh token
/// without it would otherwise fail every request of the chats until a new sign-in.
pub const TEAMS_MEETINGS_SCOPE: &str = "https://graph.microsoft.com/OnlineMeetings.ReadWrite";

/// The permissions to read Teams' teams and channels: the teams joined, their channels, the
/// channels' messages and their members (the last two the administrator's to grant). Never part
/// of [`TEAMS_SCOPE`], for the reason [`TEAMS_MEETINGS_SCOPE`] isn't.
pub const TEAMS_CHANNELS_SCOPE: &str = "https://graph.microsoft.com/Team.ReadBasic.All https://graph.microsoft.com/Channel.ReadBasic.All https://graph.microsoft.com/ChannelMessage.Read.All https://graph.microsoft.com/ChannelMember.Read.All";

/// The permissions to write in Teams' channels: post, reply and react; edit and delete one's own
/// messages (the administrator's to grant). Asked for apart from [`TEAMS_CHANNELS_SCOPE`].
pub const TEAMS_CHANNELS_WRITE_SCOPE: &str = "https://graph.microsoft.com/ChannelMessage.Send https://graph.microsoft.com/ChannelMessage.ReadWrite";

/// Teams' sign-in with the organisation's own registration, at its tenant; `meetings`: the
/// permission to make Teams meetings asked for too. `None` when the client id or the tenant
/// couldn't be one (they are typed or pasted by a person): nothing is sent anywhere for them.
/// The id stays `microsoft`, so the ID token is asked for too.
pub fn teams_provider(client_id: &str, tenant: &str, meetings: bool) -> Option<Provider> {
    let (client_id, tenant) = (client_id.trim(), tenant.trim());
    if !quatrelle_model::valid_oauth_client_id(client_id) || !quatrelle_model::valid_oauth_tenant(tenant) {
        return None;
    }
    let mut p = microsoft(client_id.to_string(), tenant);
    p.scope = if meetings { format!("{TEAMS_SCOPE} {TEAMS_MEETINGS_SCOPE}") } else { TEAMS_SCOPE.into() };
    p.name = "Microsoft Teams".into();
    Some(p)
}

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
    /// When the access token runs out, in Unix seconds: wall-clock time, which goes on while
    /// the computer sleeps (an `Instant` doesn't on Linux and macOS, so a token taken before a
    /// night's sleep still looked fresh in the morning).
    pub expires_at: i64,
    /// The permissions the provider says it granted (Microsoft lists them; empty when not said).
    pub granted: String,
    /// Who signed in, from the ID token (its `email`, else `preferred_username`): only after a
    /// browser sign-in that asked for it.
    pub user: Option<String>,
    /// The directory the account signed in at, from the ID token's `tid`, as `user` is.
    pub tenant: Option<String>,
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
    ["email", "preferred_username"]
        .iter()
        .filter_map(|k| id_token_claim(id_token, k))
        .find(|a| a.trim().contains('@') && a.trim().len() < 320)
        .map(|a| a.trim().to_string())
}

/// A string claim of an ID token's payload (`tid`, `email`…): `None` when the token isn't a
/// JWT, its payload isn't base64url JSON, or the claim isn't there or isn't a string. Same
/// trust as `id_token_user`.
pub fn id_token_claim(id_token: &str, claim: &str) -> Option<String> {
    let payload = crate::ews::unb64(id_token.split('.').nth(1)?)?;
    let v: Value = serde_json::from_slice(&payload).ok()?;
    v.get(claim).and_then(Value::as_str).map(str::to_string)
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Tokens(<redacted>)")
    }
}

/// How long a token request may take in all, its connection to the last byte of its answer: the
/// thread that renews (the Teams worker's, an account's) hears nothing else meanwhile.
const TOKEN_DEADLINE: Duration = Duration::from_secs(120);

fn token_request(p: &Provider, fields: Vec<(&str, String)>, extra_scope: &str) -> Result<Tokens> {
    token_request_within(p, fields, extra_scope, TOKEN_DEADLINE)
}

/// A token request answered within `within` ([`crate::http::send_within`]): a sign-in service
/// that trickles its answer fails it as a connection cut does, never a refusal.
pub(crate) fn token_request_within(p: &Provider, mut fields: Vec<(&str, String)>, extra_scope: &str, within: Duration) -> Result<Tokens> {
    fields.push(("client_id", p.client_id.clone()));
    // The permissions wanted, each time: without them a personal Microsoft account's token
    // covered reading (IMAP) but not sending (SMTP refused it, 535 5.7.3). Google and Webex take
    // none here.
    if !matches!(p.id.as_str(), "google" | "webex") {
        fields.push(("scope", format!("{}{extra_scope}", p.scope)));
    }
    if let Some(s) = &p.client_secret {
        fields.push(("client_secret", s.clone()));
    }
    let pairs: Vec<(&str, &str)> = fields.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let r = crate::http::send_within(
        &Request::post(&p.token_url, "application/x-www-form-urlencoded", form(&pairs), 1 << 20).header("Accept", "application/json"),
        within,
    )?;
    tokens_of_answer(r.status, &r.body, &p.name)
}

/// The tokens a sign-in service's answer carries, read as hostile JSON, or why it refused. A
/// service busy or failing (429, 5xx, said in its JSON or not) is the connection's failure, tried
/// again; never taken for a refusal that asks to sign in again. For the fuzz target.
#[doc(hidden)]
pub fn tokens_of_answer(status: u16, body: &[u8], name: &str) -> Result<Tokens> {
    // The service busy or failing (429, 5xx, said in its JSON or not): the connection's failure,
    // tried again; never taken for a refusal that asks to sign in again.
    let busy = |status: u16| NetError::Closed(format!("HTTP {status}"));
    if status == 429 || (500..600).contains(&status) {
        return Err(busy(status));
    }
    let v: Value =
        serde_json::from_slice(body).map_err(|_| NetError::Protocol(format!("the sign-in service answered HTTP {status} without JSON")))?;
    if matches!(v.get("error").and_then(Value::as_str), Some("temporarily_unavailable" | "server_error")) {
        return Err(busy(status));
    }
    if !(200..300).contains(&status) || v.get("error").is_some() {
        let what = v.get("error_description").or_else(|| v.get("error")).and_then(Value::as_str).unwrap_or("refused");
        // A revoked or expired grant means signing in again.
        return Err(NetError::Auth(format!("{name}: {what}")));
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
        tenant: v.get("id_token").and_then(Value::as_str).and_then(|t| id_token_claim(t, "tid")),
        // A minute early, so a token never runs out mid-command.
        expires_at: now_unix().saturating_add(i64::try_from(expires - 60).unwrap_or(0)),
    })
}

/// The wall-clock time in Unix seconds.
pub fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl Tokens {
    /// Whether the access token has run out (or is about to: see `expires_at`).
    pub fn expired(&self) -> bool {
        now_unix() >= self.expires_at
    }
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

/// The parameters of the browser's request line to the loopback redirect (`GET /?code=…&state=…
/// HTTP/1.1`), as the sign-in's listener reads them, and the `AADSTS` code in a refusal's words.
/// For the fuzz target.
#[doc(hidden)]
pub fn redirect_read(line: &str) -> (Vec<(String, String)>, Option<String>) {
    let target = line.split_whitespace().nth(1).unwrap_or("");
    let params = query_params(target);
    let code = params.iter().find(|(k, _)| k == "error_description").and_then(|(_, d)| aadsts_code(d));
    (params, code)
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
    authorize_on(p, 0, login_hint, open, timeout)
}

/// The first line of a request on `sock`, or `None` when the connection doesn't send a whole one
/// (at most [`REQUEST_LINE_MAX`] bytes) within `wait`, or fails.
///
/// The request's headers are read to their end too (within `wait`, at most [`REQUEST_MAX`]
/// bytes in all): a connection closed with part of the request unread can be reset, and the
/// browser then shows an error instead of the page written to it.
fn request_line(sock: &TcpStream, wait: Duration) -> Option<String> {
    sock.set_nonblocking(false).ok()?;
    let until = Instant::now().checked_add(wait)?;
    let mut got: Vec<u8> = Vec::new();
    let mut line_end: Option<usize> = None;
    let mut buf = [0u8; 1024];
    loop {
        let chunk = until
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .and_then(|left| sock.set_read_timeout(Some(left)).ok())
            .and_then(|()| (&*sock).read(&mut buf).ok())
            .and_then(|n| buf.get(..n))
            .filter(|c| !c.is_empty());
        let Some(chunk) = chunk else { break };
        let from = got.len().saturating_sub(2);
        got.extend_from_slice(chunk);
        if line_end.is_none() {
            line_end = got.iter().position(|&b| b == b'\n');
            if line_end.unwrap_or(got.len()) > REQUEST_LINE_MAX {
                return None;
            }
        }
        let tail = got.get(from..).unwrap_or_default();
        let ended = tail.windows(2).any(|w| w == b"\n\n") || tail.windows(3).any(|w| w == b"\n\r\n");
        if line_end.is_some() && (ended || got.len() >= REQUEST_MAX) {
            break;
        }
    }
    let line = got.get(..line_end?)?;
    Some(String::from_utf8_lossy(line).into_owned())
}

/// [`authorize`] with the listener on `port` (0: any free one), for a provider that takes only
/// the redirect address registered with it, to the letter.
pub fn authorize_on(
    p: &Provider,
    port: u16,
    login_hint: &str,
    open: &mut dyn FnMut(&str) -> std::result::Result<(), String>,
    timeout: Duration,
) -> Result<Tokens> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| match e.kind() {
        ErrorKind::AddrInUse if port != 0 => NetError::Auth(format!("port {port} is in use: a sign-in may still be waiting in the browser")),
        _ => e.into(),
    })?;
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
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        // Never past the sign-in's own timeout, whatever is connected.
        let left = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
        // A connection that says nothing readable (a browser's early one) is skipped: the real
        // redirect may be waiting behind it.
        let Some(line) = request_line(&sock, left.min(Duration::from_secs(10))) else { continue };
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
            (None, e) => {
                // Microsoft's reason is an AADSTS code at the start of the description, whose
                // words can name the account: the code alone is kept.
                let code = get("error_description").and_then(|d| aadsts_code(&d));
                let said = [e, code].into_iter().flatten().collect::<Vec<_>>().join(", ");
                return Err(NetError::Auth(format!("{} sign-in was cancelled ({said})", p.name)));
            }
        }
    };
    token_request(
        p,
        vec![("grant_type", "authorization_code".into()), ("code", code), ("redirect_uri", redirect), ("code_verifier", verifier)],
        identity_scope(p),
    )
}

/// The first `AADSTS<digits>` code in Microsoft's words, if any.
pub(crate) fn aadsts_code(text: &str) -> Option<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .find(|w| w.len() > 6 && w.len() <= 12 && w.strip_prefix("AADSTS").is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit())))
        .map(str::to_string)
}

/// The SASL XOAUTH2 initial response (base64 of `user=…^Aauth=Bearer …^A^A`).
pub fn xoauth2(user: &str, token: &str) -> String {
    crate::imap::base64(format!("user={user}\x01auth=Bearer {token}\x01\x01").as_bytes())
}

use std::io::Read as _;

#[cfg(test)]
pub(crate) mod tests;
