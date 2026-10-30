# Teams meetings in a Chrome app window — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** "Join meeting" on a Teams meeting opens Chrome, Chromium or Edge in app mode (`--app=<url>`), chosen in File › Options › Applications; every other link opens as today.

**Architecture:** The model says what a Teams link is and whether a link may be launched (pure, wasm). The engine keeps the chosen browser. The UI's `Services` gains `browsers()` and `open_app_window()`; a new `Action::OpenMeeting` picks app window or default browser. The desktop app finds the browsers (a pure function over `PATH` and known paths) and launches them without a shell.

**Tech Stack:** Rust, egui, `std::process::Command`; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-29-meetings-in-chrome-design.md`

## Global Constraints

- No `unwrap`/`expect`/`panic!` outside tests; errors are notices, never crashes (CLAUDE.md).
- ≤ 1000 lines per Rust file, ≤ 16 `.rs` files per folder: `apps/quatrelle/src/` is full (16), so the browser finding goes in `apps/quatrelle/src/desktop/browsers.rs`; `crates/ui-egui/src/tests/` takes one more file (`meetings.rs`, its 16th).
- Interface text through `tl!`/`tf!`, in all 13 catalogs (`crates/ui-egui/src/i18n/*.tsv`; `every_interface_string_is_translated` fails otherwise).
- Teams link: `teams.microsoft.com` (or a subdomain) under `l/meetup-join` or `meet/`, or `teams.live.com` under `meet` — exactly what `is_meeting_link` accepts today.
- Launchable link: `https://` or `http://` only, no whitespace or control character, at most 2,048 bytes, a non-empty host.
- Browser order: Linux/BSD `google-chrome-stable`, `google-chrome`, `chromium`, `chromium-browser`, `microsoft-edge-stable`, `microsoft-edge`; names shown "Google Chrome", "Chromium", "Microsoft Edge".
- Launch: the program directly, one argument `--app=<url>`, detached, stdio null; no shell.
- Before each commit: `cargo fmt --all`, `CARGO_TARGET_DIR=$PWD/target cargo xtask ci`, `CARGO_TARGET_DIR=$PWD/target-stable cargo +stable clippy --workspace --all-targets -- -D warnings`; each fix's test checked to fail without it.

## Review Focus

- A Teams link wrapped by Microsoft's Safe Links (`https://eur01.safelinks.protection.outlook.com/?url=https%3A%2F%2Fteams.microsoft.com%2Fl%2Fmeetup-join%2F…`), as an organisation's invitations often carry: it must still open in the app window, unwrapped first with `mime::clean_link` as `open_link` does → test in Task 4.
- The browser chosen in Options is uninstalled later: the first browser found is used, then the default browser → test in Task 4.
- No Chromium-family browser at all (only Firefox, or a Flatpak browser not in `PATH`): Teams links open in the default browser, as today → test in Task 4.
- The browser fails to start (`open_app_window` returns an error): the user is told and the link opens in the default browser rather than nowhere → test in Task 4.
- A Teams-looking host that isn't Microsoft's (`teams.microsoft.com.evil.example`, `evil.example/teams.microsoft.com/meet/…`) never takes the Teams path → test in Task 1.

---

### Task 1: What a Teams link is, and what may be launched

**Files:**
- Modify: `crates/model/src/ical/notes.rs` (beside `is_meeting_link`, line ~182)
- Test: `crates/model/src/ical/tests.rs`

**Interfaces:**
- Produces: `pub fn is_teams_link(url: &str) -> bool` and `pub fn launchable_link(url: &str) -> bool`, both reached as `quatrelle_model::ical::…` (the module re-exports `notes::*`). `is_meeting_link` calls `is_teams_link` for its Teams cases (one definition).

- [ ] **Step 1: Write the failing test** `a_teams_meeting_link_is_told_from_look_alikes`:

```rust
for yes in ["https://teams.microsoft.com/l/meetup-join/19%3ameeting_x%40thread.v2/0?context=%7b%7d",
            "HTTPS://Teams.Microsoft.com/meet/123?p=abc", "https://eu.teams.microsoft.com/l/meetup-join/x",
            "https://teams.live.com/meet/9876"] { assert!(is_teams_link(yes), "{yes}"); assert!(is_meeting_link(yes)); }
for no in ["https://teams.microsoft.com/v2/", "https://teams.microsoft.com.evil.example/meet/1",
           "https://evil.example/teams.microsoft.com/meet/1", "https://zoom.us/j/123", "teams.microsoft.com/meet/1"] {
    assert!(!is_teams_link(no), "{no}");
}
assert!(launchable_link("https://teams.microsoft.com/meet/1"));
for bad in ["https://teams.microsoft.com/meet/1 x", "https://teams.microsoft.com/meet/1\n", "javascript:alert(1)",
            "file:///etc/passwd", "https://", &format!("https://a.example/{}", "x".repeat(2048))] {
    assert!(!launchable_link(bad), "{bad:?}");
}
```

- [ ] **Step 2:** Run `CARGO_TARGET_DIR=$PWD/target cargo test -p quatrelle-model --lib a_teams_meeting_link` — expect a compile error (functions missing).
- [ ] **Step 3:** Implement both in `notes.rs`; move the two Teams arms of `is_meeting_link` into `is_teams_link` and call it there. `launchable_link`: scheme check case-insensitive, then host = text up to the first `/`, `?` or `#`, without `:port`, non-empty.
- [ ] **Step 4:** Run the test and `cargo test -p quatrelle-model --lib ical` — all pass (the existing `is_meeting_link` tests unchanged).
- [ ] **Step 5:** Commit: `Calendar: a Teams meeting link is told apart, and a link checked before it's launched`.

### Task 2: The browser chosen, kept in the settings

**Files:**
- Modify: `crates/engine/src/session/prefs.rs` (field + `Default`)
- Modify: `crates/engine/src/cmd/view.rs` (command, beside `view.pictures`)
- Test: `crates/engine/src/tests/safety.rs` (or a new test in the file holding view commands' tests)

**Interfaces:**
- Produces: `Prefs.meeting_browser: Option<String>` (`None`: the first browser found; `Some("")`: the default browser; `Some(path)`: that program), `#[serde(skip_serializing_if = "Option::is_none")]`. Command `view.meetingBrowser` with params `{"path": string | null}` returning `{"path": …}`.

- [ ] **Step 1: Write the failing test** `the_meeting_browser_is_kept`: `Prefs::default().meeting_browser == None`; `run("view.meetingBrowser", {"path": "/usr/bin/chromium"})` → `Some("/usr/bin/chromium")`; `{"path": ""}` → `Some("")`; `{"path": null}` → `None`; `{"path": "x\ny"}` → `Err` (control characters refused); a settings file `{}` → `None`.
- [ ] **Step 2:** Run it — compile error.
- [ ] **Step 3:** Add the field and the command (reject a path with control characters or over 4,096 bytes).
- [ ] **Step 4:** Run it and `cargo test -p quatrelle-engine --lib every_command_survives_junk_params` — pass.
- [ ] **Step 5:** Commit: `Options: the browser Teams meetings open in is kept`.

### Task 3: Finding the Chromium-family browsers, and launching one

**Files:**
- Create: `apps/quatrelle/src/desktop/browsers.rs` (declared `pub(crate) mod browsers;` in `apps/quatrelle/src/desktop.rs` beside `appearance`)
- Modify: `apps/quatrelle/src/desktop.rs` (`impl Services for Desktop`: `browsers`, `open_app_window`)
- Modify: `crates/ui-egui/src/services.rs` (trait methods with defaults)
- Test: `browsers.rs`'s own `#[cfg(test)] mod tests`

**Interfaces:**
- Produces in `ui-egui`: `fn browsers(&mut self) -> Vec<(String, String)>` (shown name, program path; default empty) and `fn open_app_window(&mut self, program: &str, url: &str) -> Result<(), String>` (default `Err("not available".into())`) on `Services`.
- Produces in `browsers.rs`: `pub(crate) struct Browser { pub name: &'static str, pub program: std::path::PathBuf }`; `pub(crate) fn on_path(path_var: Option<&std::ffi::OsStr>, executable: impl Fn(&Path) -> bool) -> Vec<Browser>` (Linux/BSD order above, one entry per shown name: the first program of each family); `pub(crate) fn at_known_places(home: Option<&Path>, env: impl Fn(&str) -> Option<PathBuf>, exists: impl Fn(&Path) -> bool) -> Vec<Browser>` (Windows and macOS paths from the spec); `pub(crate) fn found() -> Vec<Browser>` (the platform's, real `PATH`/env/filesystem); `pub(crate) fn launch(program: &Path, url: &str) -> Result<(), String>` (refuses `!launchable_link(url)`; `Command::new(program).arg(format!("--app={url}"))`, stdin/stdout/stderr null, `spawn`, child dropped).

- [ ] **Step 1: Write the failing tests** `the_browsers_on_the_path_are_found_in_order` (a temp dir with `chromium` and `google-chrome` executable and `microsoft-edge` not executable → `["Google Chrome", "Chromium"]`, programs in that dir; an empty or missing `PATH` → empty) and `the_known_places_are_looked_at` (an `exists` that says yes for `<ProgramFiles>/Microsoft/Edge/Application/msedge.exe` → one "Microsoft Edge" on Windows; for `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome` → "Google Chrome" on macOS), and `a_doubtful_link_is_never_launched` (`launch(Path::new("/bin/true"), "https://x.example/a b")` → `Err`).
- [ ] **Step 2:** Run `CARGO_TARGET_DIR=$PWD/target cargo test -p quatrelle --bin quatrelle browsers` — compile errors.
- [ ] **Step 3:** Implement; `Desktop::browsers` maps `found()` to `(name, program.display())`, `Desktop::open_app_window` calls `launch`.
- [ ] **Step 4:** Run the tests — pass.
- [ ] **Step 5:** Commit: `Desktop: the Chromium-family browsers are found, and one opens a link in app mode`.

### Task 4: "Join meeting" opens Teams in the app window

**Files:**
- Modify: `crates/ui-egui/src/lib.rs` (`Action::OpenMeeting(String)`)
- Modify: `crates/ui-egui/src/app/actions.rs` (the action's handler `open_meeting`, beside `open_link`)
- Modify: `crates/ui-egui/src/dialogs/pim_dialogs/event.rs:337` and `meeting.rs:201` (push `Action::OpenMeeting(url)` instead of `OpenUrl`)
- Create: `crates/ui-egui/src/tests/meetings.rs` (declared in `tests.rs`)

**Interfaces:**
- Consumes: Task 1's `is_teams_link`, `launchable_link`; Task 2's `prefs.meeting_browser`; Task 3's `Services::browsers`, `Services::open_app_window`.
- Produces: `pub(crate) fn open_meeting(&mut self, url: &str)` on `MailApp`: unwraps with `mime::clean_link` when `prefs.clean_links`; a non-Teams link → `open_link(url)`; a Teams link that isn't `launchable_link` → nothing opened, a notice `tl!("This meeting link can't be opened")`; else the program = `meeting_browser` if it's among `browsers()`, `None` or a program gone → the first of `browsers()`, `Some("")` or none found → `open_link`. `open_app_window` failing → a notice `tf!("{browser} couldn't open the meeting: {error}")` and `open_link(url)`.

- [ ] **Step 1: Write the failing tests** in `meetings.rs`, with a `Launching` `Services` that records `open_url` and `open_app_window` calls and offers a configurable `browsers()` list and result:
  - `a_teams_meeting_joins_in_the_app_window`: browsers `[("Google Chrome", "/opt/chrome")]`, `act(Action::OpenMeeting(teams))` → one `open_app_window("/opt/chrome", teams)`, no `open_url`.
  - `other_meetings_open_as_before`: a Zoom link → `open_url`, no app window.
  - `the_default_browser_when_chosen_or_none_found`: `meeting_browser = Some("")` → `open_url`; browsers empty → `open_url`.
  - `a_browser_gone_gives_way_to_the_first_found`: `meeting_browser = Some("/gone")`, browsers `[("Chromium", "/usr/bin/chromium")]` → app window with `/usr/bin/chromium`.
  - `a_safe_links_wrapped_teams_link_still_joins`: the Safe Links URL from Review Focus → app window with the unwrapped Teams link.
  - `a_browser_that_fails_still_opens_the_meeting`: `open_app_window` returns `Err("boom")` → a notice and one `open_url(teams)`.
  - `a_doubtful_teams_link_is_not_opened`: `https://teams.microsoft.com/meet/1 x` → no `open_url`, no app window, the last notice says it can't be opened.
  - `join_meeting_asks_for_the_meeting`: the event dialog on a sample event whose `join` is a Teams link, its "Join meeting" button clicked (`settled_text_at` + `click_at`) → the app window is asked.
- [ ] **Step 2:** Run `CARGO_TARGET_DIR=$PWD/target cargo test -p quatrelle-ui-egui --lib meetings` — compile errors.
- [ ] **Step 3:** Add the action, the handler and the two buttons' change; the notice text in all 13 catalogs.
- [ ] **Step 4:** Run the tests and `cargo test -p quatrelle-ui-egui --lib i18n::` — pass.
- [ ] **Step 5:** Commit: `Calendar: Join meeting opens a Teams meeting in Chrome's app window`.

### Task 5: File › Options › Applications

**Files:**
- Modify: `crates/ui-egui/src/panes/backstage/options.rs` (a card "Applications", after "Privacy")
- Test: `crates/ui-egui/src/tests/meetings.rs`

**Interfaces:**
- Consumes: Task 2's `view.meetingBrowser`; Task 3's `Services::browsers` (read once per frame the page is drawn, through `UiState` set by the app — `st.browsers: Option<Vec<(String, String)>>`, filled the first frame the File page is open, by any route, and cleared when it closes).
- Produces: a combo "Open Teams meetings and calls in" listing each browser's name and "The default browser"; picking one runs `view.meetingBrowser` with its program, or `""`.

- [ ] **Step 1: Write the failing test** `the_options_choose_the_meeting_browser`: with `Launching` offering Chrome and Chromium, open Options, the combo shows "Google Chrome" (the first found, nothing chosen yet), click it, click "Chromium" → `prefs.meeting_browser == Some("/usr/bin/chromium")`; click "The default browser" → `Some("")`.
- [ ] **Step 2:** Run it — fails (no such text).
- [ ] **Step 3:** Implement the card; new strings ("Applications", "Open Teams meetings and calls in", "The default browser", the card's line: "Teams meetings open in a window of their own: Chrome, Chromium or Edge, which Teams needs for calls on Linux.") in all 13 catalogs.
- [ ] **Step 4:** Run it and the i18n tests — pass.
- [ ] **Step 5:** Commit: `Options: Applications says which browser Teams meetings open in`.

### Task 6: A test build

- [ ] Run the gates (Global Constraints); push; publish the test build (`$SP/publish.sh <sha>`); tell the owner what to try: a Teams invitation's "Join meeting" opens Chrome's app window; Options › Applications lists Chrome and Chromium.
