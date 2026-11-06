// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! `cargo xtask git-sources`: what storytold's shared release checks before it builds anything
//! (`storytold/craft-common-actions`, `.github/workflows/release.yml`, job `git-sources`), checked
//! here first so that a release is never the first to find out. In each workspace (the main one
//! and `fuzz/`):
//! - no cargo config (`.cargo/config` or `.cargo/config.toml`) that replaces a source, patches one
//!   or overrides a path: none of that shows in a lock file;
//! - a lock file in step with the manifests (`cargo metadata --locked`);
//! - git dependencies from `github.com/storytold/` only.
//!
//! Parsed as TOML, as the release does, never grepped: TOML spells one key in many ways.

use std::path::Path;
use std::process::Command;

/// The workspaces checked, as folders from the repository root (`""` is the root).
const WORKSPACES: &[&str] = &["", "fuzz"];
/// The one place git dependencies may come from (the release's own prefix).
const ALLOWED_GIT: &str = "git+https://github.com/storytold/";
/// The cargo config files cargo reads in a folder, the legacy name included.
const CONFIGS: &[&str] = &[".cargo/config", ".cargo/config.toml"];

pub fn run(root: &Path) -> Result<(), String> {
    let mut bad = Vec::new();
    for ws in WORKSPACES {
        let dir = root.join(ws);
        let shown = |file: &str| if ws.is_empty() { file.to_string() } else { format!("{ws}/{file}") };
        for config in CONFIGS {
            let Ok(text) = std::fs::read_to_string(dir.join(config)) else { continue };
            if let Some(why) = config_problem(&text).map_err(|e| format!("{}: {e}", shown(config)))? {
                bad.push(format!("{}: {why}", shown(config)));
            }
        }
        let lock = shown("Cargo.lock");
        let text = std::fs::read_to_string(dir.join("Cargo.lock")).map_err(|e| format!("{lock}: {e}"))?;
        for source in outside_storytold(&text).map_err(|e| format!("{lock}: {e}"))? {
            bad.push(format!("{lock}: git dependency outside github.com/storytold: {source}"));
        }
        // After the files are read, so that a config swapping a source is named even when it
        // also breaks cargo itself.
        if let Err(e) = locked(&dir) {
            bad.push(format!("{lock}: {e}"));
        }
    }
    if bad.is_empty() {
        println!("git-sources: git dependencies come from storytold, no source replaced (as the shared release checks)");
        Ok(())
    } else {
        Err(format!("storytold's shared release would refuse this:\n  {}", bad.join("\n  ")))
    }
}

/// What, in a cargo config file, swaps a source without the lock file showing it: a source
/// with `replace-with`, a `[patch]` table or `paths` overrides.
fn config_problem(text: &str) -> Result<Option<String>, String> {
    let config: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
    let replaced: Vec<&str> = config
        .get("source")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten()
        .filter(|(_, source)| source.as_table().is_some_and(|s| s.contains_key("replace-with")))
        .map(|(name, _)| name.as_str())
        .collect();
    Ok(if !replaced.is_empty() {
        Some(format!("replaces the source(s) {}", replaced.join(", ")))
    } else if config.contains_key("patch") {
        Some("patches a source ([patch])".to_string())
    } else if config.contains_key("paths") {
        Some("overrides crates by path (paths)".to_string())
    } else {
        None
    })
}

/// The git sources in a lock file that aren't storytold's, each once, sorted.
fn outside_storytold(lock: &str) -> Result<Vec<String>, String> {
    let lock: toml::Table = toml::from_str(lock).map_err(|e| e.to_string())?;
    let mut bad: Vec<String> = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| p.get("source").and_then(toml::Value::as_str))
        .filter(|s| s.starts_with("git+") && !s.starts_with(ALLOWED_GIT))
        .map(str::to_string)
        .collect();
    bad.sort();
    bad.dedup();
    Ok(bad)
}

/// Fails when the lock file in `dir` is out of step with its manifests.
fn locked(dir: &Path) -> Result<(), String> {
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(dir)
        .args(["metadata", "--locked", "--format-version", "1"])
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("cargo metadata --locked failed (out of step with the manifests?):\n{}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGISTRY: &str = "registry+https://github.com/rust-lang/crates.io-index";

    fn lock(sources: &[&str]) -> String {
        let mut s = String::from("# This file is automatically @generated by Cargo.\nversion = 4\n");
        for (n, source) in sources.iter().enumerate() {
            s.push_str(&format!("\n[[package]]\nname = \"c{n}\"\nversion = \"1.0.0\"\nsource = \"{source}\"\n"));
        }
        s.push_str("\n[[package]]\nname = \"ours\"\nversion = \"0.1.0\"\n");
        s
    }

    /// A made-up lock file with a git dependency from elsewhere fails, each source named once;
    /// crates.io, storytold's forks and path crates pass.
    #[test]
    fn a_git_dependency_outside_storytold_fails() {
        let storytold = "git+https://github.com/storytold/egui.git?rev=6d518fc#6d518fc799681571963842f95b55a6b68431b15a";
        assert_eq!(outside_storytold(&lock(&[REGISTRY, storytold])), Ok(vec![]));
        let elsewhere = "git+https://github.com/dgaudin/outlook-pst-rs?rev=60cefd98#60cefd98";
        let lookalike = "git+https://github.com/storytold.example/x.git#1";
        assert_eq!(
            outside_storytold(&lock(&[REGISTRY, elsewhere, storytold, elsewhere, lookalike])),
            Ok(vec![elsewhere.to_string(), lookalike.to_string()])
        );
        assert!(outside_storytold("[[package]\n").is_err(), "a damaged lock file is an error, not a pass");
    }

    /// A cargo config that replaces, patches or overrides a source fails, whatever its TOML
    /// spelling; one with aliases only passes.
    #[test]
    fn a_config_that_swaps_a_source_fails() {
        assert_eq!(config_problem("[alias]\nxtask = \"run --package xtask --\"\n"), Ok(None));
        assert_eq!(config_problem("[source.crates-io]\nregistry = \"x\"\n"), Ok(None));
        for swapped in [
            "[source.crates-io]\nreplace-with = \"mirror\"\n[source.mirror]\nregistry = \"https://m.example\"\n",
            "source.crates-io.replace-with = \"vendored\"\n",
            "source = { crates-io = { replace-with = \"v\" } }\n",
            "[patch.crates-io]\negui = { git = \"https://example.com/egui\" }\n",
            "patch.crates-io.egui.path = \"../egui\"\n",
            "paths = [\"../egui\"]\n",
        ] {
            assert!(config_problem(swapped).is_ok_and(|p| p.is_some()), "{swapped}");
        }
        assert!(config_problem("[source\n").is_err());
    }

    /// The repository's own lock files pass as they are.
    #[test]
    fn the_repository_passes() {
        for ws in WORKSPACES {
            let text = std::fs::read_to_string(crate::root().join(ws).join("Cargo.lock")).unwrap();
            assert_eq!(outside_storytold(&text), Ok(vec![]), "{ws}/Cargo.lock");
        }
    }
}
