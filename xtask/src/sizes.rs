// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
//! `cargo xtask sizes`: files and folders stay small enough to read.
//!
//! Every Rust file is at most [`MAX_LINES`] lines, and every folder holds at most
//! [`MAX_FILES`] Rust files of its own (more go into submodule folders, `foo.rs` beside `foo/`).
//! What was over a limit when the rule came in is listed in `xtask/sizes.txt` with its size
//! then: it may shrink, never grow, and `--update` only ever lowers those figures (dropping
//! what's back within the limits). Nothing can be added to the list: a new file or folder
//! meets the limits outright. A file's licence-and-authors header isn't counted ([`counted_lines`]).

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use crate::authors;

/// The most lines in one Rust file.
pub const MAX_LINES: usize = 1000;
/// The most Rust files directly in one folder.
pub const MAX_FILES: usize = 16;
/// The budget file, from the workspace root.
const BUDGET: &str = "xtask/sizes.txt";

/// Line counts by file, Rust files by folder (`dir/`).
type Sizes = (BTreeMap<String, usize>, BTreeMap<String, usize>);

/// The workspace's Rust files (tracked or new, not ignored) and their line counts.
fn measure(root: &Path) -> Result<Sizes, String> {
    let out = Command::new("git")
        .current_dir(root)
        .args(["ls-files", "--cached", "--others", "--exclude-standard", "*.rs"])
        .output()
        .map_err(|e| format!("git ls-files: {e}"))?;
    let mut files = BTreeMap::new();
    let mut dirs: BTreeMap<String, usize> = BTreeMap::new();
    for f in String::from_utf8_lossy(&out.stdout).lines() {
        // Listed but deleted from the working tree: not there to count.
        let Ok(text) = std::fs::read_to_string(root.join(f)) else { continue };
        files.insert(f.to_string(), counted_lines(&text));
        let dir = f.rsplit_once('/').map_or(".", |(d, _)| d);
        *dirs.entry(format!("{dir}/")).or_default() += 1;
    }
    Ok((files, dirs))
}

/// A file's line count without its licence-and-authors header (`xtask/src/authors.rs`): the
/// header is the same everywhere and must not push a file over its limit or its budget.
pub fn counted_lines(text: &str) -> usize {
    text.lines().count().saturating_sub(authors::header_len(text))
}

/// `path count` lines (`#` comments and blank lines skipped).
fn read_budget(root: &Path) -> Result<BTreeMap<String, usize>, String> {
    let text = std::fs::read_to_string(root.join(BUDGET)).unwrap_or_default();
    let mut out = BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (path, count) = line.rsplit_once(' ').ok_or_else(|| format!("{BUDGET}:{}: `path count` expected", n + 1))?;
        let count = count.parse().map_err(|_| format!("{BUDGET}:{}: `{count}` isn't a number", n + 1))?;
        out.insert(path.trim().to_string(), count);
    }
    Ok(out)
}

/// What breaks the rule: (path, measured, allowed), files then folders.
pub fn check(files: &BTreeMap<String, usize>, dirs: &BTreeMap<String, usize>, budget: &BTreeMap<String, usize>) -> Vec<(String, usize, usize)> {
    let mut bad = Vec::new();
    for (p, &n) in files {
        let allowed = budget.get(p).copied().unwrap_or(MAX_LINES).max(MAX_LINES);
        if n > allowed {
            bad.push((p.clone(), n, allowed));
        }
    }
    for (d, &n) in dirs {
        let allowed = budget.get(d).copied().unwrap_or(MAX_FILES).max(MAX_FILES);
        if n > allowed {
            bad.push((d.clone(), n, allowed));
        }
    }
    bad
}

/// The budget lowered to what's there now: never raised, never added to, and what's back
/// within the limits gone from it.
pub fn lowered(files: &BTreeMap<String, usize>, dirs: &BTreeMap<String, usize>, budget: &BTreeMap<String, usize>) -> BTreeMap<String, usize> {
    budget
        .iter()
        .filter_map(|(p, &allowed)| {
            let (now, max) = if p.ends_with('/') { (dirs.get(p).copied()?, MAX_FILES) } else { (files.get(p).copied()?, MAX_LINES) };
            (now > max).then(|| (p.clone(), now.min(allowed)))
        })
        .collect()
}

fn write_budget(root: &Path, budget: &BTreeMap<String, usize>) -> Result<(), String> {
    let mut text = String::from(
        "# Files and folders over the size limits (xtask/src/sizes.rs) when the rule came in, and\n\
         # their size now: they may shrink, never grow. `cargo xtask sizes --update` lowers these\n\
         # figures; nothing is ever added here. Split a file by topic: `foo.rs` keeps the shared\n\
         # part and declares `mod bar;`, its pieces go in `foo/bar.rs`.\n",
    );
    for (p, n) in budget {
        text.push_str(&format!("{p} {n}\n"));
    }
    std::fs::write(root.join(BUDGET), text).map_err(|e| format!("{BUDGET}: {e}"))
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let (files, dirs) = measure(root)?;
    let budget = read_budget(root)?;
    let bad = check(&files, &dirs, &budget);
    if args.contains(&"--update") {
        if !bad.is_empty() {
            return Err(report(&bad));
        }
        let new = lowered(&files, &dirs, &budget);
        write_budget(root, &new)?;
        println!("sizes: {BUDGET} lowered ({} entries left)", new.len());
        return Ok(());
    }
    if !bad.is_empty() {
        return Err(report(&bad));
    }
    let shrunk = lowered(&files, &dirs, &budget);
    if shrunk != budget {
        println!("sizes: ok; some files shrank: `cargo xtask sizes --update` lowers their budget");
    } else {
        println!("sizes: ok ({} files, {} over the limits on the budget)", files.len(), budget.len());
    }
    Ok(())
}

fn report(bad: &[(String, usize, usize)]) -> String {
    let mut s = String::from("files or folders grew past the size rule (xtask/src/sizes.rs):\n");
    for (p, n, allowed) in bad {
        let what = if p.ends_with('/') { "Rust files" } else { "lines" };
        s.push_str(&format!("  {p}: {n} {what}, {allowed} allowed\n"));
    }
    s.push_str(
        "split by topic: `foo.rs` keeps what's shared and declares `mod bar;`, the pieces go in \
         `foo/bar.rs` (an `impl` can be continued there, a test module's tests too); a full folder \
         gets subfolders. See CLAUDE.md › File size.",
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(v: &[(&str, usize)]) -> BTreeMap<String, usize> {
        v.iter().map(|(k, n)| (k.to_string(), *n)).collect()
    }

    /// The ratchet: new files meet the limit, old ones never grow, the budget only goes down.
    #[test]
    fn files_may_shrink_never_grow() {
        let budget = map(&[("a.rs", 1500), ("d/", 20)]);
        let ok = check(&map(&[("a.rs", 1500), ("b.rs", 1000)]), &map(&[("d/", 20)]), &budget);
        assert!(ok.is_empty(), "{ok:?}");
        let bad = check(&map(&[("a.rs", 1501), ("b.rs", 1001)]), &map(&[("d/", 21), ("e/", 17)]), &budget);
        assert_eq!(bad.iter().map(|b| b.0.as_str()).collect::<Vec<_>>(), ["a.rs", "b.rs", "d/", "e/"]);
        // Lowered to what's there; gone once within the limits; a deleted file gone too.
        let new = lowered(&map(&[("a.rs", 1200)]), &map(&[("d/", 12)]), &budget);
        assert_eq!(new, map(&[("a.rs", 1200)]));
        // Never raised, never added.
        let same = lowered(&map(&[("a.rs", 1600), ("z.rs", 5000)]), &map(&[]), &budget);
        assert_eq!(same, map(&[("a.rs", 1500)]));
    }

    /// The licence-and-authors header (SPDX line and copyright lines) isn't counted.
    #[test]
    fn the_header_is_not_counted() {
        let body = "//! Doc.\nfn f() {}\n";
        let with = |head: &[&str]| format!("{}\n{body}", head.join("\n"));
        assert_eq!(counted_lines(body), 2);
        assert_eq!(counted_lines(&with(&[authors::SPDX, authors::FORK])), 2);
        assert_eq!(counted_lines(&with(&[authors::SPDX, authors::UPSTREAM, authors::FORK])), 2);
        // A copyright-looking line without the SPDX line first is just a line.
        assert_eq!(counted_lines(&with(&[authors::FORK])), 3);
        assert_eq!(counted_lines(""), 0);
    }
}
