// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Local signs of trouble in a message, read without asking anyone: a link that shows one address
//! and goes to another, an attachment that hides or runs code, a failed sender authentication.
//! They decide whether a known sender's message is shown to the scam check and are given to the
//! language model as facts. None of them makes a banner by itself (a newsletter's tracked links
//! point elsewhere all the time).

/// One sign of trouble.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sign {
    /// The sender check had a warning (a known name or signature on another address, a look-alike domain).
    SenderWarning,
    /// A link shows an address or domain (`shown`) but goes to another one (`target`).
    MisleadingLink { shown: String, target: String },
    /// An attachment that hides or runs code (its name).
    RiskyAttachment(String),
    /// The receiving server's `Authentication-Results` says `spf`, `dkim` or `dmarc` failed.
    AuthFailed(&'static str),
}

/// The most misleading links kept.
const MAX_LINKS: usize = 5;
/// How much of the headers is read for [`auth_failures`].
const MAX_HEADERS: usize = 8 * 1024;

/// The host of a web address (`http` or `https`), lowercased, without user, port or path.
fn target_host(target: &str) -> Option<String> {
    let lower = target.trim().to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    host_shape(host).then(|| host.to_string())
}

/// Whether `host` looks like a domain name: dotted labels of letters, digits and hyphens, the last
/// one letters.
fn host_shape(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| !l.is_empty() && l.len() <= 63 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        && labels.last().is_some_and(|l| l.len() >= 2 && l.chars().all(|c| c.is_ascii_alphabetic()))
}

/// The domain a link's shown text names, when the text is itself an address or a domain.
fn shown_host(shown: &str) -> Option<String> {
    let lower = shown.trim().to_ascii_lowercase();
    if lower.is_empty() || lower.contains(char::is_whitespace) || lower.len() > 253 {
        return None;
    }
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://")).unwrap_or(&lower);
    let rest = rest.rsplit('@').next().unwrap_or(rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    host_shape(host).then(|| host.to_string())
}

/// The domain a site is registered under: the last two labels, three when the second to last is
/// at most three letters (`co.uk`, `com.fr`). No public-suffix list: good enough to tell
/// `www.bank.example` from `bank.example.evil.example`.
fn registrable(host: &str) -> String {
    let labels: Vec<&str> = host.trim_start_matches("www.").split('.').collect();
    let n = labels.len();
    let take = if n >= 3 && labels.get(n - 2).is_some_and(|l| l.len() <= 3) { 3 } else { 2 };
    labels.get(n.saturating_sub(take)..).unwrap_or(&[]).join(".")
}

/// The links whose shown text is an address or a domain that isn't the target's: `(shown, target)`
/// pairs as `quatrelle_mime::html_link_pairs` gives them. A link shown as words ("Click here") doesn't
/// count; at most 5 are kept.
pub fn misleading_links(pairs: &[(String, String)]) -> Vec<Sign> {
    let mut out = Vec::new();
    for (shown, target) in pairs {
        if out.len() >= MAX_LINKS {
            break;
        }
        let (Some(from), Some(to)) = (shown_host(shown), target_host(target)) else { continue };
        if registrable(&from) != registrable(&to) {
            out.push(Sign::MisleadingLink { shown: shown.clone(), target: target.clone() });
        }
    }
    out
}

/// Extensions of files that hide or run code.
const RISKY: [&str; 17] = ["html", "htm", "shtml", "zip", "rar", "7z", "iso", "img", "exe", "scr", "js", "vbs", "lnk", "bat", "cmd", "ps1", "docm"];
/// Office files with macros, and the extensions that make a double extension suspicious.
const MACROS: [&str; 2] = ["xlsm", "pptm"];
const EXECUTABLE: [&str; 8] = ["exe", "scr", "js", "vbs", "lnk", "bat", "cmd", "ps1"];

/// The attachments that hide or run code, by name: archives and disk images, web pages,
/// programs and scripts, Office files with macros, and a double extension ending in a program
/// (`facture.pdf.exe`). Case does not matter.
pub fn risky_attachments(names: &[String]) -> Vec<Sign> {
    names
        .iter()
        .filter(|name| {
            let lower = name.trim().to_ascii_lowercase();
            let parts: Vec<&str> = lower.split('.').collect();
            let Some(last) = parts.last().filter(|_| parts.len() >= 2) else { return false };
            RISKY.contains(last) || MACROS.contains(last) || (parts.len() >= 3 && EXECUTABLE.contains(last))
        })
        .map(|name| Sign::RiskyAttachment(name.clone()))
        .collect()
}

/// The failures the `Authentication-Results` headers report (`spf=fail`, `dkim=fail`,
/// `dmarc=fail`), one of each at most, in that order. Only that header is read (folded lines
/// joined), in the first 8 KiB of `headers`; a `Received:` line that says the same counts for nothing.
pub fn auth_failures(headers: &str) -> Vec<Sign> {
    let mut end = headers.len().min(MAX_HEADERS);
    while !headers.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let text = headers.get(..end).unwrap_or("").replace("\r\n", "\n");
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        if line.is_empty() {
            break;
        }
        match (line.starts_with([' ', '\t']), lines.last_mut()) {
            (true, Some(last)) => {
                last.push(' ');
                last.push_str(line.trim());
            }
            _ => lines.push(line.to_string()),
        }
    }
    let mut out = Vec::new();
    for key in ["spf", "dkim", "dmarc"] {
        let wanted = format!("{key}=fail");
        let failed = lines.iter().any(|l| {
            let lower = l.to_ascii_lowercase();
            lower.starts_with("authentication-results:") && lower.contains(&wanted)
        });
        if failed {
            out.push(Sign::AuthFailed(key));
        }
    }
    out
}

#[cfg(test)]
mod tests;
