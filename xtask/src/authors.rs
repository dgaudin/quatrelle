// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! `cargo xtask authors`: every Rust file names its licence and its authors.
//!
//! A file starts with [`SPDX`], then one or both of [`UPSTREAM`] and [`FORK`], in that order and
//! never twice, then the file as it was (`//!` doc, attributes...). The code Quatrelle started
//! from is MailCraft's; what the fork added is Didier Gaudin's (CLAUDE.md › Authors). `cargo xtask
//! sizes` doesn't count these lines ([`header_len`]).

use std::path::Path;
use std::process::Command;

/// The licence line, first in every Rust file.
pub const SPDX: &str = "// SPDX-License-Identifier: MIT OR Apache-2.0";
/// The code taken from upstream (ZIDAN44/mailcraft).
pub const UPSTREAM: &str = "// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors";
/// The code the fork added.
pub const FORK: &str = "// Copyright (c) 2026 Didier Gaudin";

/// How many lines at the top of `text` are the licence-and-authors header: the SPDX line and
/// the copyright lines right after it, 0 when the file doesn't start with the SPDX line.
pub fn header_len(text: &str) -> usize {
    let mut lines = text.lines();
    if lines.next() != Some(SPDX) {
        return 0;
    }
    1 + lines.take_while(|l| l.starts_with("// Copyright (c)")).count()
}

/// What is wrong with the header of `text`, if anything.
pub fn problem(text: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next() != Some(SPDX) {
        return Some(format!("doesn't start with `{SPDX}`"));
    }
    let copyrights: Vec<&str> = lines.take_while(|l| l.starts_with("// Copyright (c)")).collect();
    match copyrights.as_slice() {
        [] => Some("no copyright line after the SPDX line".to_string()),
        [a] if *a == UPSTREAM || *a == FORK => None,
        [a, b] if *a == UPSTREAM && *b == FORK => None,
        [a, b] if *a == FORK && *b == UPSTREAM => Some("copyright lines in the wrong order (MailCraft first, then Didier Gaudin)".to_string()),
        [a, b] if a == b => Some(format!("`{a}` twice")),
        other => match other.iter().find(|l| **l != UPSTREAM && **l != FORK) {
            Some(unknown) => Some(format!("unknown copyright line `{unknown}`")),
            None => Some(format!("{} copyright lines, one or two expected", other.len())),
        },
    }
}

/// The tracked or new Rust files (not ignored) that fail the rule, with what is wrong.
fn offenders(root: &Path) -> Result<Vec<(String, String)>, String> {
    let out = Command::new("git")
        .current_dir(root)
        .args(["ls-files", "--cached", "--others", "--exclude-standard", "*.rs"])
        .output()
        .map_err(|e| format!("git ls-files: {e}"))?;
    let mut bad = Vec::new();
    for f in String::from_utf8_lossy(&out.stdout).lines().filter(|f| !f.starts_with("vendor/")) {
        // Listed but deleted from the working tree: not there to check.
        let Ok(bytes) = std::fs::read(root.join(f)) else { continue };
        if let Some(why) = problem(&String::from_utf8_lossy(&bytes)) {
            bad.push((f.to_string(), why));
        }
    }
    Ok(bad)
}

pub fn run(root: &Path) -> Result<(), String> {
    let bad = offenders(root)?;
    if bad.is_empty() {
        println!("authors: every Rust file names its licence and authors");
        return Ok(());
    }
    let mut s = format!("{} Rust file(s) without a proper licence-and-authors header:\n", bad.len());
    for (f, why) in &bad {
        s.push_str(&format!("  {f}: {why}\n"));
    }
    s.push_str(&format!(
        "a file starts with\n  {SPDX}\n  {UPSTREAM}   (code taken from upstream)\n  {FORK}   (code the fork added)\n\
         one or both copyright lines, in that order. A new file gets the SPDX line and Didier Gaudin. See CLAUDE.md › Authors."
    ));
    Err(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(head: &[&str]) -> String {
        format!("{}\n//! Doc.\n\nfn f() {{}}\n", head.join("\n"))
    }

    #[test]
    fn good_headers_pass() {
        assert_eq!(problem(&file(&[SPDX, FORK])), None);
        assert_eq!(problem(&file(&[SPDX, UPSTREAM])), None);
        assert_eq!(problem(&file(&[SPDX, UPSTREAM, FORK])), None);
    }

    #[test]
    fn missing_spdx_fails() {
        assert!(problem("//! Doc.\nfn f() {}\n").is_some());
        assert!(problem(&file(&[FORK, SPDX])).is_some());
        assert!(problem("").is_some());
    }

    #[test]
    fn missing_or_unknown_copyright_fails() {
        assert!(problem(&file(&[SPDX])).is_some());
        let unknown = problem(&file(&[SPDX, "// Copyright (c) 2026 Somebody Else"]));
        assert!(unknown.is_some_and(|m| m.contains("unknown")));
        assert!(problem(&file(&[SPDX, UPSTREAM, "// Copyright (c) 2026 Somebody Else"])).is_some());
        assert!(problem(&file(&[SPDX, UPSTREAM, FORK, "// Copyright (c) 2025 Third"])).is_some());
    }

    #[test]
    fn wrong_order_or_duplicate_fails() {
        assert!(problem(&file(&[SPDX, FORK, UPSTREAM])).is_some_and(|m| m.contains("order")));
        assert!(problem(&file(&[SPDX, FORK, FORK])).is_some());
        assert!(problem(&file(&[SPDX, UPSTREAM, UPSTREAM])).is_some());
    }

    #[test]
    fn header_len_counts_the_header_only() {
        assert_eq!(header_len(&file(&[SPDX, UPSTREAM, FORK])), 3);
        assert_eq!(header_len(&file(&[SPDX, FORK])), 2);
        assert_eq!(header_len("//! Doc.\n// Copyright (c) 2026 x\n"), 0);
        assert_eq!(header_len(""), 0);
    }
}
