// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A language model's server on localhost, for tests: an OpenAI-compatible `chat/completions`
//! that answers what a script says (an answer, a slow answer, an error, nonsense), and a
//! `/models` list. It keeps the JSON of the requests it got.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};

/// What the server does with the next question.
#[derive(Clone, Debug)]
pub enum FakeLlm {
    /// Answers with this text as the assistant's message.
    Answer(String),
    /// Waits, then answers with this text.
    Delay(Duration, String),
    /// Answers with this HTTP status and an error body.
    Status(u16),
    /// Answers 200 with these bytes as they are (HTML, a huge body, deep JSON...).
    Raw(Vec<u8>),
}

/// The requests each server got, by its base URL.
fn log() -> &'static Mutex<HashMap<String, Vec<Value>>> {
    static LOG: OnceLock<Mutex<HashMap<String, Vec<Value>>>> = OnceLock::new();
    LOG.get_or_init(Default::default)
}

/// The JSON bodies of the questions the server at `url` (as [`serve_llm`] gave it) was asked, in order.
pub fn last_requests(url: &str) -> Vec<Value> {
    log().lock().map(|l| l.get(url).cloned().unwrap_or_default()).unwrap_or_default()
}

/// Start a server on 127.0.0.1 and give its base URL (`http://127.0.0.1:PORT/v1`). Each question
/// to `POST /v1/chat/completions` gets the next item of `script`, the last one again when it runs
/// out; `GET /v1/models` lists one model, `fake`. It runs until the test process ends.
pub fn serve_llm(script: Vec<FakeLlm>) -> String {
    let Ok(l) = TcpListener::bind("127.0.0.1:0") else { return String::new() };
    let port = l.local_addr().map(|a| a.port()).unwrap_or(0);
    let base = format!("http://127.0.0.1:{port}/v1");
    let script = Arc::new(Mutex::new((script, 0usize)));
    let key = base.clone();
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(s) = conn else { continue };
            let (script, key) = (script.clone(), key.clone());
            std::thread::spawn(move || handle(s, &script, &key));
        }
    });
    base
}

fn handle(mut s: std::net::TcpStream, script: &Mutex<(Vec<FakeLlm>, usize)>, key: &str) {
    let Ok(clone) = s.try_clone() else { return };
    let mut reader = BufReader::new(clone);
    let mut first = String::new();
    if reader.read_line(&mut first).unwrap_or(0) == 0 {
        return;
    }
    let mut len = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; len.min(1 << 20)];
    let _ = reader.read_exact(&mut body);
    let target = first.split(' ').nth(1).unwrap_or("/");
    let (status, out) = if target.ends_with("/models") {
        (200, json!({"object": "list", "data": [{"id": "fake", "object": "model"}]}).to_string().into_bytes())
    } else {
        if let (Ok(v), Ok(mut l)) = (serde_json::from_slice::<Value>(&body), log().lock()) {
            l.entry(key.to_string()).or_default().push(v);
        }
        let item = script.lock().ok().and_then(|mut g| {
            let at = g.1.min(g.0.len().saturating_sub(1));
            g.1 += 1;
            g.0.get(at).cloned()
        });
        match item.unwrap_or(FakeLlm::Status(500)) {
            FakeLlm::Answer(t) => (200, completion(&t)),
            FakeLlm::Delay(wait, t) => {
                std::thread::sleep(wait);
                (200, completion(&t))
            }
            FakeLlm::Status(code) => (code, json!({"error": {"message": format!("scripted status {code}")}}).to_string().into_bytes()),
            FakeLlm::Raw(bytes) => (200, bytes),
        }
    };
    let head = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", out.len());
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(&out);
}

fn completion(text: &str) -> Vec<u8> {
    json!({"choices": [{"index": 0, "message": {"role": "assistant", "content": text}}]}).to_string().into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm;

    #[test]
    fn the_fake_answers_as_scripted() {
        let url = serve_llm(vec![FakeLlm::Answer("a".into()), FakeLlm::Status(500)]);
        let ask = || llm::ask(&url, "fake", None, &[("system".into(), "s".into()), ("user".into(), "u".into())]);
        assert_eq!(ask().unwrap_or_default(), "a");
        assert!(ask().is_err());
        assert!(ask().is_err(), "the last item repeats");
        let seen = last_requests(&url);
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0]["model"], "fake");
        assert_eq!(seen[0]["messages"].as_array().map(Vec::len), Some(2));
        assert_eq!(llm::models(&url, None).unwrap_or_default(), ["fake"]);
        let slow = serve_llm(vec![FakeLlm::Delay(Duration::from_millis(50), "late".into())]);
        assert_eq!(llm::ask(&slow, "fake", None, &[("user".into(), "u".into())]).unwrap_or_default(), "late");
        let junk = serve_llm(vec![FakeLlm::Raw(b"<html>".to_vec())]);
        assert!(llm::ask(&junk, "fake", None, &[("user".into(), "u".into())]).is_err());
    }
}
