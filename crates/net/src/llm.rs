//! The language model set in Options: an OpenAI-compatible `chat/completions` (a server on this
//! computer, the organisation's, or an online service), asked one question at a time.

use quatrelle_model::AccountConfig;

use crate::http::{self, Request};
use crate::{NetError, Result};

/// The longest answer read.
const MAX_ANSWER: usize = 2 << 20;

/// Where the keychain keeps the language model's key: an entry of its own, no account's.
pub fn keyring() -> AccountConfig {
    AccountConfig { id: "language-model".into(), email: "language-model".into(), ..Default::default() }
}

/// What the keychain entry is called.
pub const KEY: &str = "llm";

/// Ask `model` at `url` (the API's base, `…/v1`, or its `chat/completions` itself) these
/// messages, (role, text); the answer's text.
pub fn ask(url: &str, model: &str, key: Option<&str>, messages: &[(String, String)]) -> Result<String> {
    let base = url.trim().trim_end_matches('/');
    let endpoint = if base.ends_with("/chat/completions") { base.to_string() } else { format!("{base}/chat/completions") };
    let body = serde_json::json!({
        "model": model.trim(),
        "stream": false,
        "messages": messages.iter().map(|(role, content)| serde_json::json!({"role": role, "content": content})).collect::<Vec<_>>(),
    });
    let mut req = Request::post(&endpoint, "application/json", body.to_string().into_bytes(), MAX_ANSWER);
    if let Some(k) = key.map(str::trim).filter(|k| !k.is_empty()) {
        req = req.header("Authorization", &format!("Bearer {k}"));
    }
    let resp = http::send(&req)?;
    answer(resp.status, &resp.body)
}

/// The text of a `chat/completions` answer, or why there's none.
fn answer(status: u16, body: &[u8]) -> Result<String> {
    let v: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    if !(200..300).contains(&status) {
        let said = v
            .pointer("/error/message")
            .and_then(|m| m.as_str())
            .map(|m| m.chars().take(300).collect::<String>())
            .unwrap_or_else(|| String::from_utf8_lossy(body).chars().take(200).collect());
        return Err(NetError::No(format!("the language model answered HTTP {status}: {}", said.trim())));
    }
    v.pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .ok_or_else(|| NetError::Protocol("the language model's answer has no text".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_gives_its_text_or_why_not() {
        let ok = br#"{"choices":[{"index":0,"message":{"role":"assistant","content":"  - Le devis est accepte.\n"}}]}"#;
        assert_eq!(answer(200, ok).unwrap(), "- Le devis est accepte.");
        let refused = br#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error"}}"#;
        let e = answer(401, refused).unwrap_err().to_string();
        assert!(e.contains("401") && e.contains("Incorrect API key"), "{e}");
        assert!(answer(502, b"Bad Gateway").unwrap_err().to_string().contains("Bad Gateway"));
        assert!(answer(200, br#"{"choices":[]}"#).is_err());
        assert!(answer(200, b"not json").is_err());
    }

    /// A server on this computer, answering as Ollama or llama.cpp's do.
    #[test]
    fn a_local_server_is_asked() {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/v1/", l.local_addr().unwrap().port());
        let server = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 4096];
            while !String::from_utf8_lossy(&got).contains("\"stream\"") {
                let n = s.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
            }
            let body = r#"{"choices":[{"message":{"role":"assistant","content":"Résumé"}}]}"#;
            write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                .unwrap();
            String::from_utf8_lossy(&got).to_string()
        });
        let text = ask(&url, "llama3", Some("k1"), &[("user".into(), "Bonjour".into())]).unwrap();
        assert_eq!(text, "Résumé");
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/chat/completions"), "{request}");
        assert!(request.contains("Bearer k1") && request.contains("\"model\":\"llama3\""), "{request}");
    }
}
