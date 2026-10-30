//! Graph: requests with the bearer token — one, a listing's pages, a `$batch`, and the retries its limits ask for.

use super::*;

impl Client {
    fn request(&self, mut r: Request) -> Result<Value> {
        for (name, value) in [("Authorization", self.auth.as_str()), ("Accept", "application/json"), ("Prefer", PREFER)] {
            r = r.header(name, value);
        }
        let resp = send_through_limits(&r)?;
        check(&resp)?;
        if resp.body.iter().all(u8::is_ascii_whitespace) {
            return Ok(Value::Null);
        }
        json_body(&resp.body)
    }

    pub(crate) fn get(&self, path: &str) -> Result<Value> {
        self.request(Request::get(&format!("{}{path}", self.base), MAX_JSON))
    }

    pub(crate) fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let bytes = serde_json::to_vec(body).map_err(|e| NetError::Protocol(e.to_string()))?;
        self.request(Request::post(&format!("{}{path}", self.base), "application/json", bytes, MAX_JSON))
    }

    pub(crate) fn send_raw(&self, method: &str, path: &str, content_type: &str, body: Vec<u8>) -> Result<Value> {
        let mut r = Request::post(&format!("{}{path}", self.base), content_type, body, MAX_JSON);
        r.method = method.into();
        self.request(r)
    }

    /// Every item of a listing, following `@odata.nextLink` (on the same server only).
    pub(crate) fn all(&self, path: &str, limit: usize) -> Result<Vec<Value>> {
        let origin = http::Url::parse(&self.base)?.origin();
        let mut out = Vec::new();
        let mut url = format!("{}{path}", self.base);
        for _ in 0..MAX_PAGES {
            let v = self.request(Request::get(&url, MAX_JSON))?;
            out.extend(v.get("value").and_then(Value::as_array).cloned().unwrap_or_default());
            if out.len() >= limit {
                out.truncate(limit);
                break;
            }
            match v.get("@odata.nextLink").and_then(Value::as_str) {
                Some(next) if http::Url::parse(next).is_ok_and(|u| u.origin() == origin) => url = next.to_string(),
                _ => break,
            }
        }
        Ok(out)
    }

    /// Run requests through `$batch`, [`BATCH`] at a time, returning (status, body) per request
    /// in input order. Entries answered 429 are resubmitted on their own after the longest
    /// `Retry-After` among them, up to [`BUSY_ROUNDS`] times; whatever is still 429 is returned
    /// as such.
    pub(super) fn batch(&self, requests: &[(&str, String, Option<Value>)]) -> Result<Vec<(u16, Value)>> {
        let mut out = Vec::with_capacity(requests.len());
        for (n, chunk) in requests.chunks(BATCH).enumerate() {
            let mut results: HashMap<usize, (u16, Value)> = HashMap::new();
            let mut todo: Vec<usize> = (0..chunk.len()).collect();
            let mut round = 0;
            while !todo.is_empty() && round < BUSY_ROUNDS {
                round += 1;
                let entries: Vec<Value> = todo.iter().filter_map(|&i| chunk.get(i).map(|req| batch_entry(i, req))).collect();
                let reply = self.post("/$batch", &json!({ "requests": entries }))?;
                let by_id: HashMap<&str, &Value> = reply
                    .get("responses")
                    .and_then(Value::as_array)
                    .map(|rs| rs.iter().filter_map(|r| Some((r.get("id")?.as_str()?, r))).collect())
                    .unwrap_or_default();
                let mut busy = Vec::new();
                let mut pause = 0;
                for &i in &todo {
                    let key = (i + 1).to_string();
                    let Some(r) = by_id.get(key.as_str()) else {
                        return Err(NetError::Protocol(format!("Graph: no answer in batch {n}")));
                    };
                    let Some(status) = r["status"].as_u64().and_then(|s| u16::try_from(s).ok()).filter(|s| *s != 0) else {
                        return Err(NetError::Protocol(format!("Graph: no answer in batch {n}")));
                    };
                    if status == 429 {
                        busy.push(i);
                        pause = pause.max(backoff_secs(entry_retry_after(r).as_deref(), round));
                    }
                    results.insert(i, (status, r.get("body").cloned().unwrap_or(Value::Null)));
                }
                todo = busy;
                if !todo.is_empty() && round < BUSY_ROUNDS {
                    std::thread::sleep(std::time::Duration::from_secs(pause));
                }
            }
            out.extend((0..chunk.len()).map(|i| results.remove(&i).unwrap_or((429, Value::Null))));
        }
        Ok(out)
    }

    /// Run requests as batches and fail on the first refusal.
    pub(super) fn batch_ok(&self, requests: &[(&str, String, Option<Value>)]) -> Result<()> {
        for (status, body) in self.batch(requests)? {
            // Already gone is fine for a delete or a move.
            if status == 404 {
                continue;
            }
            let bytes = serde_json::to_vec(&body).unwrap_or_default();
            status_error(status, &bytes)?;
        }
        Ok(())
    }
}

/// One `$batch` entry: numbered from 1, with the [`PREFER`] header (and a JSON body if any).
fn batch_entry(i: usize, (method, url, body): &(&str, String, Option<Value>)) -> Value {
    let mut headers = serde_json::Map::new();
    headers.insert("Prefer".into(), json!(PREFER));
    let mut e = json!({"id": (i + 1).to_string(), "method": method, "url": url});
    if let Some(b) = body {
        headers.insert("Content-Type".into(), json!("application/json"));
        e["body"] = b.clone();
    }
    e["headers"] = Value::Object(headers);
    e
}

/// A batch entry's `Retry-After`, whatever case the key has and whether it's a string or number.
fn entry_retry_after(r: &Value) -> Option<String> {
    let headers = r.get("headers")?.as_object()?;
    match ["Retry-After", "retry-after", "RETRY-AFTER"].iter().find_map(|k| headers.get(*k))? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// `http::send`, resubmitted while the answer is 429, up to [`BUSY_ROUNDS`] attempts.
pub(super) fn send_through_limits(r: &Request) -> Result<Response> {
    let mut round = 0;
    loop {
        round += 1;
        let resp = http::send(r)?;
        if resp.status != 429 || round >= BUSY_ROUNDS {
            return Ok(resp);
        }
        std::thread::sleep(std::time::Duration::from_secs(backoff_secs(resp.header("Retry-After"), round)));
    }
}

/// Seconds to pause before attempt `round + 1`: a numeric `Retry-After` hint if present,
/// otherwise 2 s per round so far; clamped to 1..=[`BUSY_WAIT_CAP`].
pub(crate) fn backoff_secs(hint: Option<&str>, round: u32) -> u64 {
    let fallback = 2 * u64::from(round.max(1));
    hint.and_then(|h| h.trim().parse().ok()).unwrap_or(fallback).clamp(1, BUSY_WAIT_CAP)
}
