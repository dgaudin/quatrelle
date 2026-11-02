# Calendar search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In the Calendar, the search box finds appointments in every calendar and lists them in place of the days; `calendar.search` does the same for the CLI.

**Architecture:** A query parser and matcher in the model (`ical/find.rs`), a search over the calendars in memory in the engine (`session/pim/find.rs`, command `calendar.search`), and a results pane in the UI (`panes/calendar/found.rs`) shown instead of the days while the box holds something. Ported from upstream ZIDAN44 `9539f80e` and adapted to this fork.

**Tech Stack:** Rust, egui, serde_json.

**Spec:** `docs/superpowers/specs/2026-11-02-calendar-search-design.md`

**Upstream reference:** the renamed diff of `9539f80e` is `~/mailcraft/merge-tools/tmp/9539f80e.patch` (read it; do not `git apply` it). Each task says what to take from it and what differs here.

## Global Constraints

- Read `CLAUDE.md` first: no `unwrap`/`expect`/`panic!`/indexing that can panic in non-test code, files ≤ 1000 lines (split before ~800), ≤ 16 Rust files per folder, interface text through `tl!`/`tf!` with all 13 catalogs.
- Every cargo command: `TMPDIR=$HOME/.tq CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2` (the machine's swap is full). Run focused tests only (`cargo test -p <crate> --lib <path>`), then `cargo clippy -p <crate> --all-targets -- -D warnings` and `cargo fmt`. Never `cargo xtask ci`, never a release build, never two cargo commands at once: the controller runs the full gates.
- Work on branch `calendar-search` in the main checkout; commit at the end of each task. Commit messages in this repo's style (a plain sentence title, a body saying why), ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Private is `CalEvent::private` (a `bool`; `private_kind` says which kind). Show as is `CalEvent::show_as` (`ShowAs`, `crates/model/src/ical/meeting.rs`). Upstream's `Privacy`, `privacy()`, `show_as()`, `ical/showing.rs` and `event_title` do not exist here and are not added.
- The calendar's search never touches mail search (`Session::search` stays `None`).

## Review Focus

- Typing while others' calendars are in memory (25,000 appointments): each keystroke re-searches; it must stay quick → the timing test in Task 2.
- An all-day appointment in a zone east or west of UTC: listed on its own day, its start the owner's local midnight → test in Task 2.
- An occurrence moved on its own (a `recurrence_id`) clicked in the results opens that occurrence, not the series' first → test in Task 2 (the hit's `occurrence` is the `recurrence_id`) and Task 3 (`open_hit` opens it).
- The scope menu pointing at a calendar that no longer exists (account removed, calendar deleted on the server): the search falls back to every calendar shown and the menu says "All calendars", never an empty list under a wrong label → test in Task 3.
- Leaving the Calendar with a query and coming back: the results are still there; in Mail the box searches mail as before → test in Task 3.

---

### Task 1: The query (model)

**Files:**
- Modify: `crates/model/src/search.rs` (`split_tokens` → `split_tokens_with`)
- Create: `crates/model/src/ical/find.rs`
- Modify: `crates/model/src/ical.rs` (`mod find; pub use find::*;` only)
- Create: `crates/model/src/ical/tests/find.rs`; Modify: `crates/model/src/ical/tests.rs` (`mod find;`)

**Interfaces:**
- Produces: `pub(crate) fn split_tokens_with(input: &str, waiting: impl Fn(&str) -> bool) -> Vec<String>` in `search.rs` (`split_tokens` becomes `split_tokens_with(input, field_waiting)`).
- Produces, in `quatrelle_model::ical`: `pub enum EventField { Any, Subject, Location, Notes, Organizer, Attendees, Categories }`, `pub enum Needle { Words(Vec<String>), Phrase(String) }`, `pub enum EventTerm { Text(EventField, Vec<Needle>), Private, Recurring }`, `pub struct EventQuery { pub terms: Vec<(EventTerm, bool)>, pub after: Option<i64>, pub before: Option<i64> }` (all `Clone, Debug, PartialEq, Eq`; `EventQuery` also `Default`), `EventQuery::parse(input: &str) -> EventQuery`, `EventQuery::is_empty(&self) -> bool`, `EventQuery::matches(&self, e: &CalEvent) -> bool`. `after`/`before` are days since the epoch (the caller places them in its zone).

- [ ] **Step 1: Write the tests** in `ical/tests/find.rs`: port upstream's `crates/model/src/ical/tests/find.rs` from the patch, with private events built as `CalEvent { private: true, .. }` (and one with `private_kind: PrivateKind::Confidential`, found by `is:private` and by `is:confidential`). Add:

```rust
#[test]
fn the_join_link_is_found_in_the_place() {
    let teams = CalEvent { summary: "Sync".into(), join_url: "https://teams.microsoft.com/l/meetup-join/19%3a".into(), ..Default::default() };
    let m = |q: &str| EventQuery::parse(q).matches(&teams);
    assert!(m("teams.microsoft.com") && m("location:teams") && m("where:meetup"));
    assert!(!m("notes:teams"), "not in its notes");
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p quatrelle-model --lib ical::tests::find`
Expected: does not compile (`EventQuery` not found).

- [ ] **Step 3: Implement** `split_tokens_with` and `ical/find.rs` as upstream's, with two differences: `EventTerm::Private` matches `e.private`; `EventField::Location`'s text is the location followed by `join_url` when that is not empty and not already in it (joined with `" \u{1f} "`). Keep upstream's caps: 64 terms, 50 list items, notes cut at 64 KiB on a char boundary.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p quatrelle-model --lib ical::tests::find` then `cargo test -p quatrelle-model --lib search`
Expected: all pass (the mail search's tests unchanged).

- [ ] **Step 5: Check wasm and commit** — `cargo check -p quatrelle-model --target wasm32-unknown-unknown` (skip if the target isn't installed, and say so), clippy, fmt, then commit "Calendar search: the query, as the mail search box reads it".

### Task 2: The search (engine) and `calendar.search`

**Files:**
- Create: `crates/engine/src/session/pim/find.rs`
- Modify: `crates/engine/src/session/pim.rs` (`mod find; pub use find::{EVENT_HITS_MAX, EventHit, EventWhen};`)
- Modify: `crates/engine/src/lib.rs` (re-export `EVENT_HITS_MAX, EventHit, EventWhen` beside `AgendaItem`)
- Modify: `crates/engine/src/cmd/pim.rs` (the command, after `calendar.events`)
- Create: `crates/engine/src/tests/calendar/find.rs`; Modify: `crates/engine/src/tests/calendar.rs` (`mod find;`)

**Interfaces:**
- Consumes: Task 1's `EventQuery`.
- Produces: `pub enum EventWhen { #[default] All, Upcoming, Past }` (`Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize`, serde lowercase) with `pub const ALL: [EventWhen; 3]`, `pub fn code(self) -> &'static str`, `pub fn from_code(s: &str) -> Option<EventWhen>`; `pub struct EventHit { pub account: String, pub calendar: String, pub uid: String, pub occurrence: i64, pub start: i64, pub end: i64, pub all_day: bool, pub summary: String, pub location: String, pub recurring: bool, pub upcoming: bool, pub categories: Vec<String>, pub color: u8, pub category: Option<u8>, pub private: bool, pub show_as: ShowAs }` (`Clone, Debug, PartialEq, Eq`); `pub const EVENT_HITS_MAX: usize = 1000;`; `impl Session { pub fn find_events(&self, query: &str, when: EventWhen, account: Option<&str>, calendar: Option<&str>, limit: usize) -> Vec<EventHit> }`.

- [ ] **Step 1: Write the tests** in `tests/calendar/find.rs`: port upstream's `the_calendar_search_finds_events_in_every_calendar` and its other search tests from the patch (`crates/engine/src/tests/calendar_find.rs`, the part after `private_and_show_as_are_set_kept_and_shown`, which is not ported), adapting to this fork's sample (`crates/engine/src/sample/pim.rs`): where upstream relies on a sample appointment being private or categorized, make it so in the test with `event.update` / `event.create` (`"private": true`, `"categories": [...]`) rather than changing the sample. The command answers `"private": bool` and `"showAs": ShowAs::param()` instead of upstream's `"privacy"`. Add:
  - `an_all_day_appointment_is_found_on_its_own_day`: with the clock's zone at UTC+10 and at UTC−8 (as the existing zone tests set it), an all-day appointment on 2026-10-22 is found with `start == s.local_midnight(day)` of that day.
  - `an_occurrence_moved_on_its_own_is_found_at_its_place`: a weekly series and one occurrence moved (`event.move` with its `occurrence`) and renamed "Moved stand-up"; `calendar.search "moved"` answers one hit whose `occurrence` is that occurrence's `recurrence_id` and whose `start` is the new time.
  - `searching_25000_appointments_stays_quick`: 25,000 appointments in a second account's calendar (200 of them daily series), `find_events("a", All, None, None, 500)` timed, under 1 s (debug build); a second call with a field query (`location:room`) too.
  - The junk-params test (`tests.rs::every_command_survives_junk_params`) runs the new command by itself: run it.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p quatrelle-engine --lib tests::calendar::find`
Expected: does not compile.

- [ ] **Step 3: Implement** `find.rs` as upstream's `session/pim/find.rs` (the series grouping, `shown_occurrence` with `NEAR = 400 days` then `FAR = 20 × 366 days`, the order: upcoming soonest first, then past latest first, ties by summary), with `private: e.private` and `show_as: e.show_as`. The command `calendar.search` as upstream's, its params doc saying `"private"` where the answer has it; `"when"` other than all/upcoming/past is `CmdError::Params`; `limit` clamped to 1..=`EVENT_HITS_MAX`, default 200.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p quatrelle-engine --lib tests::calendar` and `cargo test -p quatrelle-engine --lib every_command_survives_junk_params`
Expected: pass.

- [ ] **Step 5: Commit** — clippy, fmt, commit "Calendar search: every calendar searched, a series found once, and calendar.search".

### Task 3: The results in the window (UI)

**Files:**
- Create: `crates/ui-egui/src/panes/calendar/side.rs` — first, move the side pane out of `calendar.rs` (807 lines): `nav`, `section_key`, `section_heading`, `calendar_row`, `months`, `mini_month`, `mini_month_to`, `todo_bar` and what only they use; `calendar.rs` declares `mod side; pub use side::{nav, todo_bar};` so `calendar::nav` / `calendar::todo_bar` paths don't change. Visibility per `CLAUDE.md` (one module down: private → `pub(super)`). Build and run the UI tests before going on; commit this move on its own ("Calendar pane: the side pane in its own file").
- Create: `crates/ui-egui/src/panes/calendar/found.rs`
- Modify: `crates/ui-egui/src/panes/calendar.rs` (`mod found;`, `show` draws `found` while `st.cal_search.active()`)
- Modify: `crates/ui-egui/src/app/bars.rs` (the box in `Module::Calendar` edits `self.ui.cal_search.query`, hint `tl!("Search calendar (location:, organizer:, after:…)")`; Escape in it, or its × button, empties it)
- Modify: `crates/ui-egui/src/lib.rs` (`pub(crate) cal_search: calendar::CalSearch` in `UiState`, default in `MailApp::new`)
- Modify: all 13 `crates/ui-egui/src/i18n/*.tsv` (take upstream's lines from the patch for "All calendars", "No events match.", "Past", "Search calendar (location:, organizer:, after:…)" in the same section; add any other new string the catalogs test names)
- Test: `crates/ui-egui/src/tests/pim.rs`, `crates/ui-egui/src/tests.rs` (`pim_tour`)

**Interfaces:**
- Consumes: Task 2's `EventHit`, `EventWhen`, `Session::find_events`; `Dialog::open_event(s, uid, occurrence)`; the command `calendar.goto {"date": "YYYY-MM-DD"}`.
- Produces: `pub struct CalSearch { pub query: String, pub when: EventWhen, pub scope: Option<(String, String)>, found: Option<(Key, Vec<EventHit>)>, slow_said: Option<web_time::Instant> }` (`Clone, Debug, Default`), `CalSearch::active(&self) -> bool`, `pub(crate) fn results(&mut self, s: &Session) -> &[EventHit]`, `pub(crate) fn found(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>)`, `pub(crate) fn open_hit(s: &Session, st: &mut UiState, h: &EventHit, acts: &mut Vec<Action>)`; `Key = (String, EventWhen, Option<(String, String)>, u64 /* pim_revision */, i64 /* minute */)`.

- [ ] **Step 1: Write the tests** in `tests/pim.rs`: port upstream's `the_calendar_search_box_lists_and_opens_events`. Add:
  - `the_calendar_search_box_empties_on_escape`: query "storyboard" typed in the Calendar, then `Event::Key { key: Key::Escape, pressed: true, .. }` → `a.ui.cal_search.query` is empty and the days are drawn again.
  - `a_scope_whose_calendar_is_gone_searches_every_calendar`: `a.ui.cal_search.scope = Some(("sample".into(), "no-such".into()))`, query "storyboard" → `results` finds "Storyboard review" and `scope` is `None` after a frame.
  - `the_calendar_query_waits_while_mail_is_shown`: query "storyboard" in the Calendar, `view.module` mail then calendar → the query is still "storyboard" and `a.session.search` is `None`; in Mail the box is the mail search box (typing "x" there sets the mail search as before).
  - In `pim_tour`, upstream's three frames (a query under All, Past, Upcoming, and one that finds nothing).

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p quatrelle-ui-egui --lib tests::pim`
Expected: does not compile.

- [ ] **Step 3: Implement** `found.rs` as upstream's, adapted: the strip is the calendar's (or first category's) colour drawn as the fork's chips draw theirs (`item_color`'s rule; no show-as pattern, which Task 3 of the private feature adds later); the lock icon (`Icon::Lock`) when `h.private`; the repeat mark (`Icon::Sync` or the icon the fork's event dialog uses for a series); the subject as stored (no `event_title`). In `results`, a scope naming no existing calendar is set to `None` before searching; a search taking 50 ms or more is logged (`log::info!("calendar: searched in {} ms, {} found", ..)`) at most every 10 s (`slow_said`). Escape: in `bars.rs`, `resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape))` empties the calendar query.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p quatrelle-ui-egui --lib tests` (all UI tests: the headless frames and the catalogs test)
Expected: pass.

- [ ] **Step 5: Commit** — clippy, fmt, commit "Calendar search: the results in place of the days, opened on their day".

### Task 4: Gates, look, docs, test build (controller)

- [ ] `find crates apps xtask -name '*.rs' -exec touch {} +` is not needed (one checkout); run `TMPDIR=$HOME/.tq CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=4 cargo xtask ci`. Expected: exit 0.
- [ ] Whole-branch review (fable, high) against the spec; fix what it finds.
- [ ] Look at it: `QUATRELLE_SCREENSHOT=… cargo run -p quatrelle -- --sample --cmd 'view.module={"name":"calendar"}'` with a query typed (or a `--cmd` that sets it, if one exists), read the PNG, light and dark.
- [ ] FORK.md: an entry under v0.31 for the calendar features (« troisième partie »), French, as the earlier ones; the survey's « Décidé le 02/11 » item 1 marked done.
- [ ] Merge `calendar-search` into main (fast-forward), push, `~/mailcraft/merge-tools/publish.sh <SHA>` in the background, memory and carnet notes.
