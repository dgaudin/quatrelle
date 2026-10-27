//! A signature written in HTML whose pictures are on the web (a company logo by its address):
//! fetched once and kept with the signature, as Outlook keeps a signature's pictures, so a message
//! shows them while it's written and carries them (whoever reads it isn't sent to the web for
//! them). A picture that couldn't be had is asked for again at the next start.

use super::*;

/// The most web pictures taken from one signature.
const MAX_PICTURES: usize = 4;

/// The largest web picture taken into a signature (it goes with every message).
const MAX_BYTES: usize = 512 * 1024;

/// The web pictures an HTML signature names: (the `src` as written, the address it stands for).
fn web_pictures(html: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower.get(from..).and_then(|s| s.find("<img")).map(|i| i + from) {
        let end = lower.get(at..).and_then(|s| s.find('>')).map_or(lower.len(), |i| i + at);
        from = end;
        let Some(tag) = html.get(at..end) else { break };
        let tag_lower = tag.to_ascii_lowercase();
        let Some(src) = tag_lower.find("src=").map(|i| i + 4) else { continue };
        let quote = tag.get(src..src + 1).unwrap_or("");
        if quote != "\"" && quote != "'" {
            continue;
        }
        let Some(written) = tag.get(src + 1..).and_then(|rest| rest.find(quote).and_then(|e| rest.get(..e))) else { continue };
        let url = written.replace("&amp;", "&");
        let web = url.starts_with("https://") || url.starts_with("http://");
        if web && !out.iter().any(|(w, _)| w == written) && out.len() < MAX_PICTURES {
            out.push((written.to_string(), url));
        }
    }
    out
}

/// `html` with each picture `src` written `written` now `with` (whatever case `src` is in; a
/// link to the same address is left as it is).
fn swap_src(html: &str, written: &str, with: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while let Some(at) = lower.get(i..).and_then(|s| s.find("src=")).map(|k| k + i) {
        let value = at + 4;
        let quote = html.get(value..value + 1).unwrap_or("");
        let matches = (quote == "\"" || quote == "'")
            && html.get(value + 1..).is_some_and(|r| r.starts_with(written) && r.get(written.len()..).is_some_and(|t| t.starts_with(quote)));
        if matches {
            out.push_str(html.get(i..value + 1).unwrap_or(""));
            out.push_str(with);
            i = value + 1 + written.len();
        } else {
            out.push_str(html.get(i..value).unwrap_or(""));
            i = value;
        }
    }
    out.push_str(html.get(i..).unwrap_or(""));
    out
}

/// A picture's type from its first bytes, when it's one a message can carry.
fn picture_type(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG") {
        Some("image/png")
    } else if data.starts_with(&[0xFF, 0xD8]) {
        Some("image/jpeg")
    } else if data.starts_with(b"GIF8") {
        Some("image/gif")
    } else if data.len() > 12 && data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

impl Session {
    /// Ask for the web pictures of every account's HTML signature not fetched yet.
    pub(crate) fn fetch_signature_pictures(&mut self) {
        let urls: Vec<String> = self.accounts.iter().flat_map(|a| web_pictures(&a.config.signature_raw_html)).map(|(_, url)| url).collect();
        for url in urls {
            if !matches!(self.pictures.get(&url), Some(Picture::Loading | Picture::Ready(_))) {
                self.pictures.insert(url.clone(), Picture::Loading);
                self.jobs.push(Job::FetchUrl { url });
            }
        }
        self.keep_signature_pictures();
    }

    /// The web pictures fetched go into their signature: each kept as one of its pictures, its
    /// address in the HTML written as that picture's `cid:`.
    pub(crate) fn keep_signature_pictures(&mut self) {
        let ids: Vec<String> = self.accounts.iter().map(|a| a.config.id.clone()).collect();
        for id in ids {
            let Some(mut config) = self.account(&id).map(|a| a.config.clone()) else { continue };
            let mut changed = false;
            for (written, url) in web_pictures(&config.signature_raw_html) {
                let Some(Picture::Ready(data)) = self.pictures.get(&url) else { continue };
                let Some(mime) = picture_type(data).filter(|_| data.len() <= MAX_BYTES) else {
                    log::info!("{id}: the signature's picture isn't one a message can carry, or is too large");
                    continue;
                };
                let n = config.signature_pictures.len() + 1;
                let cid = format!("signature-{n}@quatrelle");
                let name = url.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or("picture").chars().take(80).collect::<String>();
                config.signature_pictures.push(quatrelle_model::SignaturePicture {
                    cid: cid.clone(),
                    name,
                    mime: mime.to_string(),
                    data: data.to_vec(),
                });
                config.signature_raw_html = swap_src(&config.signature_raw_html, &written, &format!("cid:{cid}"));
                changed = true;
            }
            if !changed {
                continue;
            }
            // Its text and look again, the pictures in them now.
            let (text, styles) = mime::signature_html(&config.signature_raw_html);
            if !text.trim().is_empty() {
                config.signature = text.chars().take(10_000).collect();
                config.signature_html = if styles.is_plain() { String::new() } else { mime::rich::to_html(&text, &styles) };
            }
            log::info!("{id}: the signature's web pictures are kept with it");
            if let Err(e) = self.update_account(config, None) {
                log::warn!("{id}: the signature's pictures couldn't be kept: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signatures_web_pictures_are_found() {
        let html = r#"<p>Riley</p><IMG alt="logo" SRC="https://www.example.com/logo.gif?a=1&amp;b=2"><img src='http://x.example/p.png'><img src="cid:kept"><img src="https://www.example.com/logo.gif?a=1&amp;b=2">"#;
        assert_eq!(
            web_pictures(html),
            [
                ("https://www.example.com/logo.gif?a=1&amp;b=2".to_string(), "https://www.example.com/logo.gif?a=1&b=2".to_string()),
                ("http://x.example/p.png".to_string(), "http://x.example/p.png".to_string())
            ]
        );
        assert!(web_pictures("<img src=https://unquoted.example/x.png>").is_empty());
        let swapped = swap_src(r#"<a href="https://e.example/l.gif"><IMG SRC="https://e.example/l.gif"></a>"#, "https://e.example/l.gif", "cid:s1");
        assert_eq!(swapped, r#"<a href="https://e.example/l.gif"><IMG SRC="cid:s1"></a>"#, "the link stays");
        assert_eq!(picture_type(b"GIF89a..."), Some("image/gif"));
        assert_eq!(picture_type(b"<svg"), None);
    }
}
