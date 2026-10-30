//! The profiles of a Chromium-family browser, so Teams meetings open in the one signed in to
//! Microsoft 365 rather than whichever the browser used last.
//!
//! Each browser lists them in the `Local State` file of its user data folder:
//! `profile.info_cache` maps each profile's folder to what it shows (`name`) and the Google
//! account signed in to it (`user_name`). That file is the browser's: it's only read, at most
//! 8 MiB of it, and nothing else in that folder is.
//!
//! Where the folder is, is a pure function over the platform's inputs, so each platform's is
//! tested on every platform.

use std::io::Read;
use std::path::{Path, PathBuf};

use quatrelle_engine::is_profile_dir;
use quatrelle_ui_egui::BrowserProfile;
use serde_json::Value;

use super::browsers::{CHROME, CHROMIUM, EDGE};

/// The most of a `Local State` file read: a bigger one is not the browser's own, and has no
/// profiles worth showing.
const MAX_LOCAL_STATE: u64 = 8 << 20;
/// The most profiles listed, and characters of a name or an account shown.
const MAX_PROFILES: usize = 100;
const MAX_TEXT: usize = 100;

/// The platforms whose browsers keep their data in different places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Platform {
    /// Linux and the BSDs.
    Unix,
    Windows,
    Mac,
}

impl Platform {
    /// The one Quatrelle runs on.
    pub(crate) const HERE: Platform = if cfg!(windows) {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::Mac
    } else {
        Platform::Unix
    };
}

/// The user data folder of the browser shown as `browser` on `platform`: `home` is the user's
/// folder, `env` reads an environment variable. `None` for a browser not known, or when what
/// it takes isn't set.
pub(crate) fn user_data_dir(browser: &str, platform: Platform, home: Option<&Path>, env: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    let tail: &[&str] = match (platform, browser) {
        (Platform::Unix, CHROME) => &["google-chrome"],
        (Platform::Unix, CHROMIUM) => &["chromium"],
        (Platform::Unix, EDGE) => &["microsoft-edge"],
        (Platform::Windows, CHROME) => &["Google", "Chrome", "User Data"],
        (Platform::Windows, CHROMIUM) => &["Chromium", "User Data"],
        (Platform::Windows, EDGE) => &["Microsoft", "Edge", "User Data"],
        (Platform::Mac, CHROME) => &["Google", "Chrome"],
        (Platform::Mac, CHROMIUM) => &["Chromium"],
        (Platform::Mac, EDGE) => &["Microsoft Edge"],
        _ => return None,
    };
    let base = match platform {
        // `$XDG_CONFIG_HOME` counts only as a full path, as its specification says.
        Platform::Unix => env("XDG_CONFIG_HOME").filter(|p| p.has_root()).or_else(|| home.map(|h| h.join(".config")))?,
        Platform::Windows => env("LOCALAPPDATA")?,
        Platform::Mac => home?.join("Library").join("Application Support"),
    };
    Some(tail.iter().fold(base, |p, part| p.join(part)))
}

/// The profiles a `Local State` file lists, sorted by name (whatever the case). The file is
/// another program's: anything that isn't as expected gives none, a folder that isn't one name
/// is left out, names and accounts lose their control characters and are cut short, and no
/// more than [`MAX_PROFILES`] are listed.
pub(crate) fn profiles_in(local_state: &str) -> Vec<BrowserProfile> {
    let Ok(state) = serde_json::from_str::<Value>(local_state) else { return Vec::new() };
    let Some(cache) = state.get("profile").and_then(|p| p.get("info_cache")).and_then(Value::as_object) else { return Vec::new() };
    let text = |info: &Value, key: &str| info.get(key).and_then(Value::as_str).map(shown);
    let mut found: Vec<BrowserProfile> = cache
        .iter()
        .filter(|(dir, _)| is_profile_dir(dir))
        .map(|(dir, info)| BrowserProfile {
            dir: dir.clone(),
            // No name the browser shows: its folder's.
            name: text(info, "name").filter(|n| !n.is_empty()).unwrap_or_else(|| shown(dir)),
            account: text(info, "user_name").unwrap_or_default(),
        })
        .collect();
    found.sort_by_cached_key(|p| (p.name.to_lowercase(), p.dir.clone()));
    found.truncate(MAX_PROFILES);
    found
}

/// `text` as it's shown: without control characters, at most [`MAX_TEXT`] characters.
fn shown(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(MAX_TEXT).collect()
}

/// The profiles of the browser whose user data folder is `user_data`: none when its
/// `Local State` can't be read, isn't a plain file, or is over [`MAX_LOCAL_STATE`].
pub(crate) fn read_profiles(user_data: &Path) -> Vec<BrowserProfile> {
    let path = user_data.join("Local State");
    // A plain file only: opening a pipe put there would wait for ever.
    if !std::fs::metadata(&path).is_ok_and(|m| m.is_file()) {
        return Vec::new();
    }
    let Ok(file) = std::fs::File::open(&path) else { return Vec::new() };
    let mut bytes = Vec::new();
    if file.take(MAX_LOCAL_STATE + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > MAX_LOCAL_STATE {
        return Vec::new();
    }
    String::from_utf8(bytes).map(|text| profiles_in(&text)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(profiles: &[BrowserProfile]) -> Vec<&str> {
        profiles.iter().map(|p| p.dir.as_str()).collect()
    }

    #[test]
    fn the_profiles_are_read_from_local_state() {
        let state = r#"{"browser": {"x": 1}, "profile": {"last_used": "Profile 1", "info_cache": {
            "Default": {"name": "Personne 1", "user_name": "", "avatar_icon": "x"},
            "Profile 1": {"name": "work", "user_name": "me@studio.example"},
            "Profile 3": {"name": "Home"}
        }}}"#;
        let found = profiles_in(state);
        assert_eq!(
            found,
            [
                BrowserProfile { dir: "Profile 3".into(), name: "Home".into(), account: String::new() },
                BrowserProfile { dir: "Default".into(), name: "Personne 1".into(), account: String::new() },
                BrowserProfile { dir: "Profile 1".into(), name: "work".into(), account: "me@studio.example".into() },
            ]
        );
    }

    #[test]
    fn a_local_state_not_as_expected_has_no_profiles() {
        for junk in [
            "",
            "{",
            "not json",
            "[1, 2]",
            "\"profile\"",
            "{}",
            r#"{"profile": 5}"#,
            r#"{"profile": {}}"#,
            r#"{"profile": {"info_cache": []}}"#,
            r#"{"profile": {"info_cache": "Default"}}"#,
            r#"{"profile": {"info_cache": null}}"#,
        ] {
            assert!(profiles_in(junk).is_empty(), "{junk}");
        }
        // A profile that isn't an object, or whose name isn't text: its folder's name is shown.
        let found = profiles_in(r#"{"profile": {"info_cache": {"Default": 5, "Profile 1": {"name": 7, "user_name": ["x"]}}}}"#);
        assert_eq!(dirs(&found), ["Default", "Profile 1"]);
        assert_eq!(found[1].name, "Profile 1");
        assert_eq!(found[1].account, "");
    }

    #[test]
    fn a_folder_that_isnt_one_name_is_left_out() {
        let state = r#"{"profile": {"info_cache": {
            "../../.ssh": {"name": "a"}, "a/b": {"name": "b"}, "..": {"name": "c"}, "": {"name": "d"},
            "x\\y": {"name": "e"}, " Default": {"name": "f"}, "x\u0001y": {"name": "g"}, "Default": {"name": "h"}
        }}}"#;
        assert_eq!(dirs(&profiles_in(state)), ["Default"]);
    }

    #[test]
    fn names_lose_control_characters_and_are_cut_short() {
        let long = "é".repeat(150);
        let state = format!(
            r#"{{"profile": {{"info_cache": {{
                "Default": {{"name": "Wo\u0000rk\n\u001b[31m", "user_name": "me\r\n@x.example"}},
                "Profile 1": {{"name": "{long}", "user_name": "{long}"}}
            }}}}}}"#
        );
        let found = profiles_in(&state);
        assert_eq!(found[0].name, "Work[31m");
        assert_eq!(found[0].account, "me@x.example");
        assert_eq!(found[1].name, "é".repeat(100));
        assert_eq!(found[1].account, "é".repeat(100));
    }

    #[test]
    fn no_more_than_a_hundred_profiles_in_order() {
        let entries: Vec<String> = (0..250).map(|i| format!(r#""Profile {i}": {{"name": "p{i:03}"}}"#)).collect();
        let state = format!(r#"{{"profile": {{"info_cache": {{{}}}}}}}"#, entries.join(","));
        let found = profiles_in(&state);
        assert_eq!(found.len(), 100);
        assert_eq!(found[0].name, "p000");
        assert_eq!(found[99].name, "p099");
        // Whatever the case; the same name twice keeps its folders' order.
        let found = profiles_in(
            r#"{"profile": {"info_cache": {"B": {"name": "beta"}, "A": {"name": "Alpha"}, "C": {"name": "alpha"}, "D": {"name": "Beta"}}}}"#,
        );
        assert_eq!(dirs(&found), ["A", "C", "B", "D"]);
    }

    /// A temporary folder, removed when the test ends (a failed assertion too).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let dir = std::env::temp_dir().join(format!("quatrelle-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::create_dir_all(&dir);
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn local_state_is_read_within_bounds() {
        let state = r#"{"profile": {"info_cache": {"Default": {"name": "Me"}}}}"#;
        let dir = TempDir::new("profiles-read");
        let _ = std::fs::write(dir.0.join("Local State"), state);
        assert_eq!(dirs(&read_profiles(&dir.0)), ["Default"]);
        // Over 8 MiB: none, though it starts as a real one.
        let big = TempDir::new("profiles-big");
        let mut padded = state.trim_end_matches('}').to_string();
        padded.push_str("}}}");
        padded.push_str(&format!(r#", "pad": "{}"}}"#, "x".repeat(9 << 20)));
        let _ = std::fs::write(big.0.join("Local State"), padded);
        assert!(read_profiles(&big.0).is_empty());
        // No file, a folder in its place, or no user data folder at all.
        let none = TempDir::new("profiles-none");
        assert!(read_profiles(&none.0).is_empty());
        let _ = std::fs::create_dir(none.0.join("Local State"));
        assert!(read_profiles(&none.0).is_empty());
        assert!(read_profiles(&none.0.join("missing")).is_empty());
    }

    #[test]
    fn each_browser_keeps_its_data_where_its_platform_says() {
        let home = Path::new("/home/me");
        let no_env = |_: &str| None;
        let unix = |browser: &str, env: &dyn Fn(&str) -> Option<PathBuf>| user_data_dir(browser, Platform::Unix, Some(home), env);
        // Linux and the BSDs: under ~/.config, or $XDG_CONFIG_HOME when it's a full path.
        assert_eq!(unix(CHROME, &no_env), Some(home.join(".config").join("google-chrome")));
        assert_eq!(unix(CHROMIUM, &no_env), Some(home.join(".config").join("chromium")));
        assert_eq!(unix(EDGE, &no_env), Some(home.join(".config").join("microsoft-edge")));
        let xdg = |v: &str| (v == "XDG_CONFIG_HOME").then(|| PathBuf::from("/data/conf"));
        assert_eq!(unix(CHROME, &xdg), Some(Path::new("/data/conf").join("google-chrome")));
        let relative = |v: &str| (v == "XDG_CONFIG_HOME").then(|| PathBuf::from("conf"));
        assert_eq!(unix(CHROMIUM, &relative), Some(home.join(".config").join("chromium")));
        let empty = |v: &str| (v == "XDG_CONFIG_HOME").then(PathBuf::new);
        assert_eq!(unix(EDGE, &empty), Some(home.join(".config").join("microsoft-edge")));
        assert_eq!(user_data_dir(CHROME, Platform::Unix, None, no_env), None, "no home");
        assert_eq!(user_data_dir(CHROME, Platform::Unix, None, xdg), Some(Path::new("/data/conf").join("google-chrome")));
        // Windows: under %LOCALAPPDATA%.
        let local = PathBuf::from("C:/Users/me/AppData/Local");
        let win = |v: &str| (v == "LOCALAPPDATA").then(|| local.clone());
        assert_eq!(user_data_dir(CHROME, Platform::Windows, None, win), Some(local.join("Google").join("Chrome").join("User Data")));
        assert_eq!(user_data_dir(CHROMIUM, Platform::Windows, None, win), Some(local.join("Chromium").join("User Data")));
        assert_eq!(user_data_dir(EDGE, Platform::Windows, None, win), Some(local.join("Microsoft").join("Edge").join("User Data")));
        assert_eq!(user_data_dir(CHROME, Platform::Windows, Some(home), no_env), None, "no %LOCALAPPDATA%");
        // macOS: under ~/Library/Application Support.
        let mac = Path::new("/Users/me");
        let support = mac.join("Library").join("Application Support");
        assert_eq!(user_data_dir(CHROME, Platform::Mac, Some(mac), no_env), Some(support.join("Google").join("Chrome")));
        assert_eq!(user_data_dir(CHROMIUM, Platform::Mac, Some(mac), no_env), Some(support.join("Chromium")));
        assert_eq!(user_data_dir(EDGE, Platform::Mac, Some(mac), no_env), Some(support.join("Microsoft Edge")));
        assert_eq!(user_data_dir(CHROME, Platform::Mac, None, no_env), None, "no home");
        // A browser not known.
        assert_eq!(user_data_dir("Firefox", Platform::Unix, Some(home), no_env), None);
    }
}
