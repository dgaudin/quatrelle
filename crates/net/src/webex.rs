//! Webex's scheduled meetings, through its public REST API: a meeting made for an appointment,
//! changed with it and deleted with it, and who is signed in.
//!
//! Webex is a service of Quatrelle, not of an account. Its integration is the user's own: the
//! client id comes from the settings, and the client secret and the refresh token stay in the
//! keychain, under [`keyring`]. The access token lives in memory alone, and the password an
//! answer carries goes into a [`Secret`]. None of them is ever logged.
//!
//! Answers are hostile input: they are capped, what is kept of them is cleaned, and a meeting
//! with no usable id or link is refused, not half kept.

use std::sync::Arc;

use quatrelle_model::ical::valid_webex_id;
use quatrelle_model::job::Secret;
use quatrelle_model::{AccountConfig, WebexError, WebexFields, WebexMeeting};
use serde_json::{Value, json};

use crate::NetError;
use crate::ews_pim::utc;
use crate::http::{self, Request, Response, pct};
use crate::oauth::{self, Provider, Tokens};
use crate::worker::Credentials;

/// Webex's API, its sign-in page and token form, and the permissions asked of it.
pub const API: &str = "https://webexapis.com/v1";
pub const AUTHORIZE_URL: &str = "https://webexapis.com/v1/authorize";
pub const TOKEN_URL: &str = "https://webexapis.com/v1/access_token";
pub const SCOPE: &str = "meeting:schedules_read meeting:schedules_write spark:people_read";

/// What the keychain entries are called.
pub const REFRESH_KEY: &str = "webex-refresh";
pub const SECRET_KEY: &str = "webex-secret";

/// The longest answer read.
const MAX_ANSWER: usize = 1 << 20;
/// The most kept of a text in an answer.
const MAX_TEXT: usize = 300;
/// The longest meeting link taken.
const MAX_LINK: usize = 2048;

/// Where the keychain keeps Webex's secrets: an entry of their own, no account's.
pub fn keyring() -> AccountConfig {
    AccountConfig { id: "webex".into(), email: "webex".into(), ..Default::default() }
}

/// Webex's sign-in for an integration. `None` when the client id couldn't be one or the secret
/// is empty (they are typed or pasted by a person): nothing is sent anywhere for them.
pub fn provider(client_id: &str, client_secret: &str) -> Option<Provider> {
    let (client_id, client_secret) = (client_id.trim(), client_secret.trim());
    if !quatrelle_model::valid_oauth_client_id(client_id) || client_secret.is_empty() {
        return None;
    }
    Some(Provider {
        id: "webex".into(),
        name: "Webex".into(),
        auth_url: AUTHORIZE_URL.into(),
        token_url: TOKEN_URL.into(),
        scope: SCOPE.into(),
        client_id: client_id.into(),
        client_secret: Some(client_secret.into()),
    })
}

/// A client of Webex's API.
pub struct Webex {
    pub(crate) api: String,
    pub(crate) token_url: String,
    client_id: String,
    creds: Arc<dyn Credentials>,
    /// The access token, and the refresh token it came from.
    access: Option<(Tokens, String)>,
}

impl Webex {
    pub fn new(client_id: &str, creds: Arc<dyn Credentials>) -> Webex {
        Webex { api: API.into(), token_url: TOKEN_URL.into(), client_id: client_id.into(), creds, access: None }
    }

    /// Make a meeting on `site` for an appointment. `quiet`: Webex mails no one.
    pub fn create(&mut self, site: &str, fields: &WebexFields, quiet: bool) -> Result<WebexMeeting, WebexError> {
        let mut body = json!({
            "title": fields.title,
            "start": utc(fields.start),
            "end": utc(fields.end),
            "timezone": fields.timezone,
            "siteUrl": site,
            "sendEmail": !quiet,
        });
        if let Some(rule) = &fields.recurrence {
            body["recurrence"] = json!(rule);
        }
        let req = Request::post(&format!("{}/meetings", self.api), "application/json", body.to_string().into_bytes(), MAX_ANSWER);
        self.meeting(&req)
    }

    /// Send a meeting what changed between `before` and `after` (`PATCH`: the rest, its number
    /// and password among it, stays as it is).
    pub fn update(&mut self, id: &str, before: &WebexFields, after: &WebexFields, quiet: bool) -> Result<WebexMeeting, WebexError> {
        let mut body = json!({"sendEmail": !quiet});
        if after.title != before.title {
            body["title"] = json!(after.title);
        }
        if (after.start, after.end, &after.timezone) != (before.start, before.end, &before.timezone) {
            body["start"] = json!(utc(after.start));
            body["end"] = json!(utc(after.end));
            body["timezone"] = json!(after.timezone);
        }
        if after.recurrence != before.recurrence {
            body["recurrence"] = json!(after.recurrence);
        }
        let mut req = Request::post(&self.meeting_url(id), "application/json", body.to_string().into_bytes(), MAX_ANSWER);
        req.method = "PATCH".into();
        self.meeting(&req)
    }

    /// Delete a meeting; one already gone is deleted.
    pub fn delete(&mut self, id: &str, quiet: bool) -> Result<(), WebexError> {
        let query = if quiet { "?sendEmail=false" } else { "" };
        let mut req = Request::get(&format!("{}{query}", self.meeting_url(id)), MAX_ANSWER);
        req.method = "DELETE".into();
        let r = self.call(&req)?;
        if r.ok() || r.status == 404 { Ok(()) } else { Err(refusal(&r)) }
    }

    /// The address of whoever is signed in; any failure is none.
    pub fn me(&mut self) -> Option<String> {
        let r = self.call(&Request::get(&format!("{}/people/me", self.api), MAX_ANSWER)).ok()?;
        if !r.ok() {
            return None;
        }
        let v: Value = serde_json::from_slice(&r.body).ok()?;
        Some(clean(v.pointer("/emails/0")?.as_str()?)).filter(|address| !address.is_empty())
    }

    fn meeting_url(&self, id: &str) -> String {
        format!("{}/meetings/{}", self.api, pct(id))
    }

    /// Send `req` and read the meeting it answers with.
    fn meeting(&mut self, req: &Request) -> Result<WebexMeeting, WebexError> {
        let r = self.call(req)?;
        if !r.ok() {
            return Err(refusal(&r));
        }
        meeting(&r.body)
    }

    /// Send `req` with the access token. A 401 drops the token, and `req` goes once more with a
    /// fresh one; a second 401 means signing in again.
    fn call(&mut self, req: &Request) -> Result<Response, WebexError> {
        for _ in 0..2 {
            let token = self.token()?;
            let r = http::send(&req.clone().header("Authorization", &format!("Bearer {token}"))).map_err(failed)?;
            if r.status != 401 {
                return Ok(r);
            }
            self.access = None;
        }
        Err(WebexError::SignIn)
    }

    /// The access token: the one held, while it hasn't run out and the keychain's refresh token
    /// is still the one it came from (else the person signed out, or in as someone else), or a
    /// fresh one.
    fn token(&mut self) -> Result<String, WebexError> {
        let keyring = keyring();
        let refresh = self.creds.get_secret(&keyring, REFRESH_KEY).ok_or(WebexError::SignIn)?;
        let secret = self.creds.get_secret(&keyring, SECRET_KEY).ok_or(WebexError::SignIn)?;
        if let Some((t, from)) = &self.access
            && *from == refresh
            && !t.expired()
        {
            return Ok(t.access.clone());
        }
        let mut p = provider(&self.client_id, &secret).ok_or(WebexError::SignIn)?;
        p.token_url.clone_from(&self.token_url);
        let t = oauth::refresh(&p, &refresh).map_err(|e| if matches!(e, NetError::Auth(_)) { WebexError::SignIn } else { failed(e) })?;
        // A refresh may hand out a new refresh token, which replaces the old one. Unsaved, the
        // old one is still the keychain's: the token in hand keeps working until it runs out.
        let mut from = refresh;
        if let Some(new) = t.refresh.clone().filter(|r| !r.is_empty() && *r != from) {
            match self.creds.set_secret(&keyring, REFRESH_KEY, &new) {
                Ok(()) => from = new,
                Err(e) => log::warn!("the new Webex sign-in couldn't be saved in the keychain: {e}"),
            }
        }
        let access = t.access.clone();
        self.access = Some((t, from));
        Ok(access)
    }
}

fn failed(e: NetError) -> WebexError {
    WebexError::Failed(e.to_string())
}

/// Why Webex said no: 403 on its own, else its words on one line, or the status alone.
fn refusal(r: &Response) -> WebexError {
    if r.status == 403 {
        return WebexError::Forbidden;
    }
    let v: Value = serde_json::from_slice(&r.body).unwrap_or_default();
    let said = [v.pointer("/errors/0/description"), v.get("message")]
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|words| clean(&words.split_whitespace().collect::<Vec<_>>().join(" ")))
        .find(|words| !words.is_empty());
    WebexError::Refused { status: r.status, message: said.unwrap_or_else(|| r.status.to_string()) }
}

/// The meeting in an answer.
fn meeting(body: &[u8]) -> Result<WebexMeeting, WebexError> {
    let v: Value = serde_json::from_slice(body).unwrap_or_default();
    let text = |name: &str| v.get(name).and_then(Value::as_str).unwrap_or_default();
    let (id, web_link) = (text("id"), text("webLink"));
    if !valid_webex_id(id) {
        return Err(WebexError::Failed("Webex's answer has no usable meeting id".into()));
    }
    if !usable_link(web_link) {
        return Err(WebexError::Failed("Webex's answer has no usable meeting link".into()));
    }
    Ok(WebexMeeting {
        id: id.into(),
        web_link: web_link.into(),
        meeting_number: clean(text("meetingNumber")),
        password: Secret(clean(text("password"))),
        sip_address: clean(text("sipAddress")),
    })
}

/// Whether a link from an answer may go into an invitation.
fn usable_link(link: &str) -> bool {
    link.starts_with("https://") && link.chars().count() <= MAX_LINK && !link.chars().any(char::is_control)
}

/// A text from an answer, with no control character, trimmed and at most [`MAX_TEXT`] long.
fn clean(text: &str) -> String {
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    text.trim().chars().take(MAX_TEXT).collect()
}

#[cfg(test)]
mod tests;
