//! An in-memory Webex on localhost, for tests: meetings (made, changed, deleted), who is signed
//! in, and the sign-in service's token form, with a bearer token.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::oauth::unpct;

pub const TOKEN: &str = "test-token";
/// The integration's client secret: the token form takes no other.
pub const SECRET: &str = "test-secret";
/// A refresh token the token form no longer takes.
pub const REVOKED: &str = "revoked";

#[derive(Default)]
pub struct Data {
    /// The meetings, as Webex answers them.
    pub meetings: Vec<Value>,
    /// Every request, as `METHOD path` with its query.
    pub calls: Vec<String>,
    /// The JSON bodies of the requests.
    pub bodies: Vec<Value>,
    /// The next answers to anything but the token form, (status, body), in place of what the
    /// request would get. A string goes out as it is: an answer that isn't JSON.
    pub refuse: VecDeque<(u16, Value)>,
    /// Tokens the form has given.
    pub refreshes: usize,
    /// Each token comes with a new refresh token, not the one it was asked with.
    pub rotate: bool,
}

impl Data {
    /// The answer to a request: its status and body (`null`: none).
    fn answer(&mut self, method: &str, target: &str, authorized: bool, body: &[u8]) -> (u16, Value) {
        self.calls.push(format!("{method} {target}"));
        if (method, target) == ("POST", "/v1/access_token") {
            return self.token(body);
        }
        if let Ok(v) = serde_json::from_slice(body) {
            self.bodies.push(v);
        }
        if let Some(r) = self.refuse.pop_front() {
            return r;
        }
        if !authorized {
            return (401, json!({"message": "The request requires a valid access token."}));
        }
        let id = target.split('?').next().and_then(|p| p.strip_prefix("/v1/meetings/"));
        match (method, target, id) {
            ("POST", "/v1/meetings", _) => self.make(body),
            ("GET", "/v1/people/me", _) => (200, json!({"id": "p1", "emails": ["zoe@acme.example", "z@acme.example"], "displayName": "Zoe"})),
            ("PATCH", _, Some(id)) => self.change(id, body),
            ("DELETE", _, Some(id)) => self.remove(id),
            _ => (404, json!({"message": "The requested resource could not be found."})),
        }
    }

    fn token(&mut self, form: &[u8]) -> (u16, Value) {
        let text = String::from_utf8_lossy(form);
        let get = |k: &str| text.split('&').filter_map(|kv| kv.split_once('=')).find(|(n, _)| *n == k).map(|(_, v)| unpct(v));
        let asked = get("refresh_token").unwrap_or_default();
        if get("client_secret").as_deref() != Some(SECRET) || asked == REVOKED {
            return (400, json!({"message": "The refresh token provided is expired, revoked, malformed, or invalid."}));
        }
        self.refreshes += 1;
        let refresh = if self.rotate { format!("rotated-{}", self.refreshes) } else { asked };
        (200, json!({"access_token": TOKEN, "expires_in": 1_209_600, "refresh_token": refresh, "token_type": "Bearer"}))
    }

    fn make(&mut self, body: &[u8]) -> (u16, Value) {
        let n = self.meetings.len() + 1;
        let id = format!("m{n}");
        let number = format!("26234567{n:02}");
        let mut m: Value = serde_json::from_slice(body).unwrap_or_else(|_| json!({}));
        m["id"] = json!(id);
        m["webLink"] = json!(format!("https://acme.webex.com/acme/j.php?MTID={id}"));
        m["meetingNumber"] = json!(number);
        m["password"] = json!("Xy7qT2");
        m["sipAddress"] = json!(format!("{number}@acme.webex.com"));
        self.meetings.push(m.clone());
        (200, m)
    }

    fn change(&mut self, id: &str, body: &[u8]) -> (u16, Value) {
        let Some(m) = self.meetings.iter_mut().find(|m| m["id"] == id) else {
            return (404, json!({"message": "The requested resource could not be found."}));
        };
        if let Ok(Value::Object(changed)) = serde_json::from_slice(body) {
            for (k, v) in changed {
                m[k] = v;
            }
        }
        (200, m.clone())
    }

    fn remove(&mut self, id: &str) -> (u16, Value) {
        let was = self.meetings.len();
        self.meetings.retain(|m| m["id"] != id);
        if self.meetings.len() == was { (404, json!({"message": "The requested resource could not be found."})) } else { (204, Value::Null) }
    }
}

pub struct Server {
    /// The API address (`…/v1`).
    pub base: String,
    pub data: Arc<Mutex<Data>>,
}

/// Start the server; it runs until the test process ends.
pub fn serve(data: Data) -> Server {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}/v1", l.local_addr().unwrap().port());
    let data = Arc::new(Mutex::new(data));
    let shared = data.clone();
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(mut s) = conn else { continue };
            let mut reader = BufReader::new(s.try_clone().unwrap());
            let mut first = String::new();
            if reader.read_line(&mut first).unwrap_or(0) == 0 {
                continue;
            }
            let (mut len, mut authorized) = (0usize, false);
            let bearer = format!("Bearer {TOKEN}");
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                authorized |= line.split_once(':').is_some_and(|(k, v)| k.eq_ignore_ascii_case("authorization") && v.trim() == bearer);
            }
            let mut body = vec![0u8; len];
            let _ = reader.read_exact(&mut body);
            let mut words = first.split(' ');
            let (method, target) = (words.next().unwrap_or("GET"), words.next().unwrap_or("/"));
            let (status, out) = shared.lock().unwrap().answer(method, target, authorized, &body);
            let out = match out {
                Value::Null => Vec::new(),
                Value::String(raw) => raw.into_bytes(),
                json => json.to_string().into_bytes(),
            };
            let head = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", out.len());
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(&out);
        }
    });
    Server { base, data }
}
