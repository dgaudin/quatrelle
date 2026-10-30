//! The Chromium-family browsers installed (Google Chrome, Chromium, Microsoft Edge), and opening
//! a link in one as an app window (`--app=<url>`): a window of its own, without tabs or address
//! bar, where Teams meetings run as they do in Teams' own app.
//!
//! - **Linux and the BSDs:** the first of each family's programs found on the `PATH`.
//! - **Windows:** under `%ProgramFiles%`, `%ProgramFiles(x86)%` and `%LocalAppData%`.
//! - **macOS:** the app bundles in `/Applications` and `~/Applications`.
//!
//! Finding is a pure function over what the platform offers, so each platform's is tested on
//! every platform.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use quatrelle_engine::model::ical::launchable_link;

/// A browser found: the name shown, and the program to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Browser {
    pub name: &'static str,
    pub program: PathBuf,
}

/// The names shown, in the order offered.
const CHROME: &str = "Google Chrome";
const CHROMIUM: &str = "Chromium";
const EDGE: &str = "Microsoft Edge";

/// Linux and the BSDs: each family's programs, the preferred first.
const ON_PATH: [(&str, &[&str]); 3] = [
    (CHROME, &["google-chrome-stable", "google-chrome"]),
    (CHROMIUM, &["chromium", "chromium-browser"]),
    (EDGE, &["microsoft-edge-stable", "microsoft-edge"]),
];

/// The browsers in the `PATH` directories (`path_var`): for each family, the first of its
/// programs that is `executable` in one of them.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
pub(crate) fn on_path(path_var: Option<&OsStr>, executable: impl Fn(&Path) -> bool) -> Vec<Browser> {
    let dirs: Vec<PathBuf> = path_var.map(|p| std::env::split_paths(p).filter(|d| !d.as_os_str().is_empty()).collect()).unwrap_or_default();
    ON_PATH
        .iter()
        .filter_map(|(name, programs)| {
            programs.iter().find_map(|program| dirs.iter().map(|d| d.join(program)).find(|p| executable(p))).map(|program| Browser { name, program })
        })
        .collect()
}

/// The browsers where Windows' and macOS's installers put them: `env` reads an environment
/// variable (`ProgramFiles`), `home` is the user's folder, `exists` says whether a program is
/// there. One entry per name, the first place found.
#[cfg_attr(all(unix, not(target_os = "macos")), allow(dead_code))]
pub(crate) fn at_known_places(home: Option<&Path>, env: impl Fn(&str) -> Option<PathBuf>, exists: impl Fn(&Path) -> bool) -> Vec<Browser> {
    let windows = |vars: &[&str], tail: &[&str]| -> Vec<PathBuf> {
        vars.iter().filter_map(|v| env(v)).map(|base| tail.iter().fold(base, |p, part| p.join(part))).collect()
    };
    let mac = |bundle: &str, program: &str| -> Vec<PathBuf> {
        let inside = |base: PathBuf| base.join("Applications").join(bundle).join("Contents").join("MacOS").join(program);
        std::iter::once(inside(PathBuf::from("/"))).chain(home.map(|h| inside(h.to_path_buf()))).collect()
    };
    let places = [
        (
            CHROME,
            [
                windows(&["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"], &["Google", "Chrome", "Application", "chrome.exe"]),
                mac("Google Chrome.app", "Google Chrome"),
            ],
        ),
        (CHROMIUM, [Vec::new(), mac("Chromium.app", "Chromium")]),
        (
            EDGE,
            [
                windows(&["ProgramFiles", "ProgramFiles(x86)"], &["Microsoft", "Edge", "Application", "msedge.exe"]),
                mac("Microsoft Edge.app", "Microsoft Edge"),
            ],
        ),
    ];
    places.into_iter().filter_map(|(name, lists)| lists.into_iter().flatten().find(|p| exists(p)).map(|program| Browser { name, program })).collect()
}

/// Whether `path` is a file that can be run (on Unix, one with an execute bit).
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
pub(crate) fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// The browsers installed on this computer.
pub(crate) fn found() -> Vec<Browser> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        on_path(std::env::var_os("PATH").as_deref(), is_executable)
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
        at_known_places(home.as_deref(), |v| std::env::var_os(v).map(PathBuf::from), Path::is_file)
    }
}

/// Open `url` in `program` as an app window (`--app=<url>`), without a shell and without
/// waiting. A link that isn't a plain web address with a host is refused before anything runs.
pub(crate) fn launch(program: &Path, url: &str) -> Result<(), String> {
    if !launchable_link(url) {
        return Err("This meeting link can't be opened".into());
    }
    let mut cmd = Command::new(program);
    cmd.arg(format!("--app={url}")).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up.
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    // A browser already running takes the link and this process ends at once; reaped here, so it
    // isn't left a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

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

    fn names(found: &[Browser]) -> Vec<&'static str> {
        found.iter().map(|b| b.name).collect()
    }

    #[test]
    fn the_browsers_on_the_path_are_found_in_order() {
        let dir = TempDir::new("browsers-path");
        let other = TempDir::new("browsers-path-other");
        for program in ["chromium", "google-chrome", "microsoft-edge"] {
            let _ = std::fs::write(dir.0.join(program), b"");
        }
        // A second Chrome later on the PATH: the first is kept, one entry per name.
        let _ = std::fs::write(other.0.join("google-chrome-stable"), b"");
        let path: OsString = std::env::join_paths([&dir.0, &other.0]).unwrap_or_default();
        // Edge's file is there but can't be run.
        let executable = |p: &Path| p.is_file() && p.file_name().is_some_and(|n| n != "microsoft-edge");
        let found = on_path(Some(&path), executable);
        assert_eq!(names(&found), ["Google Chrome", "Chromium"]);
        assert_eq!(found[0].program, other.0.join("google-chrome-stable"));
        assert_eq!(found[1].program, dir.0.join("chromium"));
        // An empty PATH, or none: nothing.
        assert!(on_path(Some(std::ffi::OsStr::new("")), |p: &Path| p.is_file()).is_empty());
        assert!(on_path(None, |_: &Path| true).is_empty());
    }

    /// The real check: an executable file (Unix permission bits).
    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_run_is_not_a_browser() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("browsers-exec");
        let run = dir.0.join("chromium");
        let plain = dir.0.join("microsoft-edge");
        let _ = std::fs::write(&run, b"");
        let _ = std::fs::write(&plain, b"");
        let _ = std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644));
        assert!(is_executable(&run));
        assert!(!is_executable(&plain));
        assert!(!is_executable(&dir.0));
        let found = on_path(Some(dir.0.as_os_str()), is_executable);
        assert_eq!(names(&found), ["Chromium"]);
    }

    #[test]
    fn the_known_places_are_looked_at() {
        let files = PathBuf::from("C:/Program Files");
        let files86 = PathBuf::from("C:/Program Files (x86)");
        let local = PathBuf::from("C:/Users/me/AppData/Local");
        let env = |name: &str| match name {
            "ProgramFiles" => Some(files.clone()),
            "ProgramFiles(x86)" => Some(files86.clone()),
            "LocalAppData" => Some(local.clone()),
            _ => None,
        };
        // Windows: Edge where every Windows has it.
        let edge = files86.join("Microsoft").join("Edge").join("Application").join("msedge.exe");
        let found = at_known_places(None, env, |p: &Path| p == edge);
        assert_eq!(names(&found), ["Microsoft Edge"]);
        assert_eq!(found[0].program, edge);
        // Chrome installed for the user only, and Edge: Chrome first.
        let chrome = local.join("Google").join("Chrome").join("Application").join("chrome.exe");
        let found = at_known_places(None, env, |p: &Path| p == edge || p == chrome);
        assert_eq!(names(&found), ["Google Chrome", "Microsoft Edge"]);
        assert_eq!(found[0].program, chrome);
        // macOS: the program inside the app bundle, in /Applications or the user's own.
        let mac_chrome = Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
        let found = at_known_places(None, |_: &str| None, |p: &Path| p == mac_chrome);
        assert_eq!(names(&found), ["Google Chrome"]);
        assert_eq!(found[0].program, mac_chrome);
        let home = Path::new("/Users/me");
        let mine = home.join("Applications/Chromium.app/Contents/MacOS/Chromium");
        let found = at_known_places(Some(home), |_: &str| None, |p: &Path| p == mine);
        assert_eq!(names(&found), ["Chromium"]);
        assert_eq!(found[0].program, mine);
        // Nothing there: nothing found.
        assert!(at_known_places(Some(home), env, |_: &Path| false).is_empty());
    }

    #[test]
    fn a_doubtful_link_is_never_launched() {
        assert!(launch(Path::new("/bin/true"), "https://x.example/a b").is_err());
        assert!(launch(Path::new("/bin/true"), "file:///etc/passwd").is_err());
        assert!(launch(Path::new("/bin/true"), "https://x.example/a\n--disable-web-security").is_err());
    }

    /// A program that isn't there: an error, not a crash.
    #[test]
    fn a_missing_browser_is_an_error() {
        let dir = TempDir::new("browsers-missing");
        assert!(launch(&dir.0.join("no-such-browser"), "https://x.example/a").is_err());
    }
}
