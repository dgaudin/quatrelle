//! The session: the HTTP interface the transport runs over, and signing in (fetching the JMAP
//! session, RFC 8620 §2, and resolving its URLs).

use super::*;

/// An HTTP request.
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Largest body accepted.
    pub max: usize,
}

impl Request {
    pub fn get(url: &str, max: usize) -> Request {
        Request { method: "GET".into(), url: url.into(), headers: Vec::new(), body: Vec::new(), max }
    }

    pub fn post(url: &str, content_type: &str, body: Vec<u8>, max: usize) -> Request {
        Request { method: "POST".into(), url: url.into(), headers: vec![("Content-Type".into(), content_type.into())], body, max }
    }

    pub fn header(mut self, name: &str, value: &str) -> Request {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// An HTTP response.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

/// The HTTP client JMAP runs over. Implementations follow redirects for GET, never from https
/// to http, and cap the body at [`Request::max`].
#[allow(async_fn_in_trait)]
pub trait Http {
    async fn send(&self, req: Request) -> Result<Response>;
}

/// The session URL for what the user typed: a bare host or origin gets the well-known path
/// (RFC 8620 §2.2).
pub fn session_url(input: &str) -> String {
    let s = input.trim().trim_end_matches('/');
    let s = if s.contains("://") { s.to_string() } else { format!("https://{s}") };
    let after_scheme = s.split_once("://").map_or("", |(_, r)| r);
    if after_scheme.contains('/') { s } else { format!("{s}/.well-known/jmap") }
}

/// A signed-in JMAP session.
pub struct Client<H> {
    pub(super) http: H,
    pub(super) auth: String,
    pub(super) api: String,
    pub(super) download: String,
    pub(super) upload: String,
    /// The push template (RFC 8620 §7.3), when the server offers one.
    pub(super) events: Option<String>,
    pub(super) account: String,
    pub(super) submission: bool,
    pub(super) quota: bool,
    pub(super) vacation: bool,
    pub(super) max_get: usize,
}

pub(super) fn str_at<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key).and_then(Value::as_str).ok_or_else(|| Error::Protocol(format!("the JMAP session has no `{key}`")))
}

pub(super) fn json_body(resp: &Response) -> Result<Value> {
    serde_json::from_slice(&resp.body).map_err(|e| Error::Protocol(format!("JMAP: {e}")))
}

/// Turn an HTTP failure into the error the caller knows how to handle.
pub(super) fn check(resp: &Response) -> Result<()> {
    match resp.status {
        200..=299 => Ok(()),
        401 | 403 => Err(Error::Auth("the server refused the user name or password".into())),
        // Busy or down: reconnect with back-off.
        429 | 500..=599 => Err(Error::Closed(format!("HTTP {}", resp.status))),
        s => {
            // RFC 7807 problem details, when there are any.
            let detail = serde_json::from_slice::<Value>(&resp.body)
                .ok()
                .and_then(|v| v.get("detail").or_else(|| v.get("type")).and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            Err(Error::Bad(format!("HTTP {s} {detail}").trim().to_string()))
        }
    }
}

/// `{accountId}`-style template variables (RFC 6570 level 1), percent-encoded.
pub(super) fn expand(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), &pct(v));
    }
    out
}

impl<H: Http> Client<H> {
    /// Fetch the session and check it offers mail. `bearer` signs in with an OAuth access token.
    pub async fn connect(http: H, url: &str, user: &str, secret: &str, bearer: bool) -> Result<Client<H>> {
        let auth = if bearer { format!("Bearer {secret}") } else { format!("Basic {}", base64(format!("{user}:{secret}").as_bytes())) };
        let session = session_url(url);
        let resp = http.send(Request::get(&session, 1 << 20).header("Authorization", &auth).header("Accept", "application/json")).await?;
        check(&resp)?;
        let v = json_body(&resp)?;
        let caps = v.get("capabilities").cloned().unwrap_or(Value::Null);
        if caps.get(MAIL).is_none() {
            return Err(Error::Protocol("this JMAP server doesn't offer mail".into()));
        }
        let account = v
            .get("primaryAccounts")
            .and_then(|p| p.get(MAIL))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol("no mail account in the JMAP session".into()))?;
        let base = Url::parse(&session)?;
        let resolve = |key: &str| -> Result<String> {
            let u = base.join(str_at(&v, key)?)?;
            // Never from HTTPS down to plain HTTP (the password goes with every request).
            if base.https && !u.https {
                return Err(Error::Tls(format!("the JMAP {key} isn't https")));
            }
            Ok(u.full())
        };
        let max_get = caps.get(CORE).and_then(|c| c.get("maxObjectsInGet")).and_then(Value::as_u64).unwrap_or(500);
        Ok(Client {
            api: resolve("apiUrl")?,
            // Templates keep their braces: resolve only the part before them.
            download: template_url(&base, str_at(&v, "downloadUrl")?)?,
            upload: template_url(&base, str_at(&v, "uploadUrl")?)?,
            events: v.get("eventSourceUrl").and_then(Value::as_str).and_then(|t| template_url(&base, t).ok()),
            account: account.to_string(),
            submission: caps.get(SUBMISSION).is_some(),
            quota: caps.get(QUOTA).is_some(),
            vacation: caps.get(VACATION).is_some(),
            max_get: usize::try_from(max_get).unwrap_or(500).clamp(1, 1000),
            auth,
            http,
        })
    }

    /// The `Authorization` header value every request carries.
    pub fn authorization(&self) -> &str {
        &self.auth
    }

    /// Carry a new OAuth access token from now on (the old one ran out).
    pub fn set_bearer(&mut self, token: &str) {
        self.auth = format!("Bearer {token}");
    }

    /// The push address (RFC 8620 §7.3) for every type of change, when the server pushes.
    pub fn events_url(&self) -> Option<String> {
        self.events.as_ref().map(|t| expand(t, &[("types", "*"), ("closeafter", "no"), ("ping", "30")]))
    }
}

/// A download or upload template resolved against the session URL, braces intact.
pub(super) fn template_url(base: &Url, t: &str) -> Result<String> {
    let cut = t.find('{').unwrap_or(t.len());
    let (head, rest) = t.split_at(cut);
    let u = base.join(head)?;
    if base.https && !u.https {
        return Err(Error::Tls("a JMAP address isn't https".into()));
    }
    Ok(format!("{}{rest}", u.full()))
}
