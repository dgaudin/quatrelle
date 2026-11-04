# Several Webex rooms, and CONFERENCE read and written — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Several Webex rooms (a name, a link, free lines) in the Webex Meeting button's menu and in
Options › Calendar, and RFC 7986 `CONFERENCE` read and written wherever Quatrelle reads and writes
iCalendar.

**Architecture:** The model gains `Conference` (`ical/conference.rs`), read in `parse_in`, written in
`write_event`, used by `join_link`, `links` and the calendar search. The engine keeps
`Prefs::webex_rooms` (the old `webex_room` carried over), changes them through `calendar.webexRoom`,
and takes `"conference"` in `event.create`/`event.update`. The UI's event form keeps the conference
and the rule that takes it out; a new `pim_dialogs/webex_rooms.rs` draws the menu's rooms and replaces
a room's block; a new `options/webex_rooms.rs` draws the Options rows.

**Tech Stack:** Rust 2024, egui 0.37, serde/serde_json, proptest; fake CalDAV server
`crates/net/src/fakes/fake_dav.rs`; cargo-fuzz target `fuzz/fuzz_targets/ical.rs`.

**Spec:** `docs/superpowers/specs/2026-11-04-webex-rooms-conference-design.md`

## Global Constraints

- CLAUDE.md applies whole: no `unwrap`/`expect`/`panic!`/`unreachable!`/indexing that can panic in non-test code; `get()`, checked or saturating arithmetic, char-boundary cuts (`chars().take(n)`).
- Every new `.rs` file starts with `// SPDX-License-Identifier: MIT OR Apache-2.0` then `// Copyright (c) 2026 Didier Gaudin`.
- At most 1000 lines per Rust file, 16 Rust files per folder; split before ~800. `crates/engine/src/tests/` and `crates/ui-egui/src/tests/` are full (16 files each): engine tests go in `crates/engine/src/tests/calendar/`, UI tests in `crates/ui-egui/src/tests/scheduling/`.
- Interface text through `tl!`/`tf!`, a row in every catalog of `crates/ui-egui/src/i18n/*.tsv` (13 files), our own wording; the French rows say exactly what the spec says: « Salles Webex », « Ma salle personnelle », « Ajouter une salle », « Ajouter une salle… », « Ce rendez-vous a déjà sa réunion Webex ». Strings no longer used leave the catalogs if `crates/ui-egui/src/i18n/tests.rs` asks it.
- Caps (spec): link ≤ 2 048 bytes and a Webex personal room (`ical::is_webex_link`); room name and conference label ≤ 80 characters; ≤ 20 free lines of ≤ 200 characters; ≤ 20 rooms; two rooms never share a link (compared ignoring ASCII case).
- Work on branch `webex-rooms` from `main`, in the main checkout (no worktree: one `target/`). One build at a time, `TMPDIR=$HOME/.tq`. Before each commit: `cargo test -p <crate>`, `cargo clippy -p <crate> --all-targets -- -D warnings`, `cargo fmt --check`; the last task runs `cargo xtask ci`.
- Commit messages in English, the repo's style ("Area: what changed"), ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Exchange (EWS, Graph) gets nothing new: no code under `crates/net/src/exchange/` changes.

## Review Focus

1. An invitation from another app with `CONFERENCE` and the same link in `DESCRIPTION`, saved here after a change elsewhere (its reminder): the line is written once, with its own label and features, never twice (once as the conference, once from `extra`) → Task 1 test `kept_lines_are_written_once`, Task 4 test `a_foreign_conference_stays`.
2. A 1.0 settings file with `webexRoom` set, read, saved, read again: one room, first, unnamed; `webexRoom` not written again; not two rooms on the second read → Task 2 test `an_old_personal_room_becomes_the_first_room`.
3. Two links that differ only in case (`…/meet/Ana`, `…/meet/ana`): the second room is refused as the same link → Task 2 test `webex_rooms_refuse_what_they_cannot_keep`.
4. A room chosen, then « Nouvelle réunion Webex »: the room's block gives way to the meeting's, the conference is the meeting's link, the place too if it was the room's → Task 4 test `a_scheduled_meeting_replaces_a_room`.
5. A hand-edited settings file with 30 rooms, a 500-character name, 25 lines of 300 characters, duplicate and non-Webex links: cut to the caps, no panic, nothing else in the settings lost → Task 2 test `a_hand_edited_rooms_list_is_cut_when_read`.

---

### Task 1: Model — `CONFERENCE` read, written, joined and searched

**Files:**
- Create: `crates/model/src/ical/text.rs` (moved: `split_text_list`, `unescape`, `escape`, `duration` from `ical.rs` l.290-388), `crates/model/src/ical/conference.rs`, `crates/model/src/ical/tests/conference.rs`, `fuzz/seeds/ical/conference.ics`
- Modify: `crates/model/src/ical.rs` (`mod text; pub use text::*;`, `mod conference; pub use conference::{Conference, CONFERENCE_LABEL_MAX};`, field, parse arm ~l.568, write ~l.754, `join_link`, `links`), `crates/model/src/ical/find.rs` (`place`, l.210), `crates/model/src/ical/tests.rs` (`mod conference;` beside `mod find;`), `crates/model/src/ical/tests/find.rs`, `fuzz/fuzz_targets/ical.rs`

**Interfaces:**
- Produces (all as `quatrelle_model::ical::…`):
  - `pub const CONFERENCE_LABEL_MAX: usize = 80;`
  - `#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)] pub struct Conference { pub url: String, #[serde(default, skip_serializing_if = "String::is_empty")] pub label: String, #[serde(default, skip_serializing_if = "Vec::is_empty")] pub features: Vec<String> }`
  - `impl Conference { pub fn new(url: &str, label: &str) -> Option<Conference> }`: `None` unless `launchable_link(url.trim())`; the label without control characters and `"`, trimmed, cut to 80 characters; features `["AUDIO", "VIDEO"]`.
  - `CalEvent::conference: Option<Conference>` after `webex_id`, `#[serde(default, skip_serializing_if = "Option::is_none")]`, doc: where to join it online (RFC 7986 `CONFERENCE`).
  - In `conference.rs`, private to `ical`: `pub(super) fn take(e: &mut CalEvent, l: &Line) -> bool` (sets `e.conference` when it has none and the line qualifies) and `pub(super) fn line(c: &Conference) -> Option<String>` (the unfolded content line; `None` when `url` isn't launchable).

- [ ] **Step 1: Branch and move the text helpers.** `git switch -c webex-rooms`. Move the four functions to `text.rs` unchanged (the doc comment "A TEXT value with its escapes undone." now sits above `unescape`, where it belongs), `pub use text::*;` in `ical.rs`. Run `TMPDIR=$HOME/.tq cargo test -p quatrelle-model ical` — expect PASS, no test changed. Commit "Calendar model: the iCalendar text helpers in ical/text.rs".

- [ ] **Step 2: Write the failing tests** in `tests/conference.rs` (events read with `ical::parse(text, 0)` from a `BEGIN:VCALENDAR`/`VEVENT` around the lines):
  - `a_conference_is_read_in_any_order`: `CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=Webex:https://acme.webex.com/meet/ana` → `Conference { url: "https://acme.webex.com/meet/ana", label: "Webex", features: ["AUDIO","VIDEO"] }`, no `CONFERENCE` in `extra`; `CONFERENCE;LABEL="Salle A, 2e";FEATURE=video;value=uri:https://meet.example.org/abc` → label `Salle A, 2e`, features `["VIDEO"]`; `CONFERENCE:https://jitsi.example.org/team` → that url, label `""`; `FEATURE=PHONE,VIDEO` with an https link → read.
  - `other_conference_lines_stay_as_they_came`: one event with, in order, `CONFERENCE;VALUE=URI;FEATURE=PHONE;LABEL=Appel:tel:+33-1-23-45-67-89`, `CONFERENCE;VALUE=URI;FEATURE=CHAT:https://chat.example.org/r`, `CONFERENCE;VALUE=URI;FEATURE=PHONE:https://dial.example.org/`, `CONFERENCE;VALUE=URI:https://first.example.org/`, `CONFERENCE;VALUE=URI:https://second.example.org/` → conference `https://first.example.org/`; `extra` holds the other four lines exactly as written, in order.
  - `hostile_links_are_no_conference`: values `javascript:alert(1)`, `https://user@evil.example/`, `https://acme.webex.com:443@evil.example/`, `data:text/html,x`, `https://a.example/` + 2 040 × `a` (over 2 048 bytes), and `CONFERENCE;VALUE=TEXT:https://a.example/` → `conference` is `None`, each line in `extra`.
  - `a_conference_is_written`: an event with `Conference::new("https://acme.webex.com/meet/ana", "Webex")` → the unfolded output has `CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=Webex:https://acme.webex.com/meet/ana`; label `Salle "A"; 2e` → `LABEL="Salle A; 2e"`; label `""` → no `LABEL=`; features `["VIDEO","SCREEN"]` kept as `FEATURE=VIDEO,SCREEN`; a 300-byte link folds (lines ≤ 75 octets) and reads back equal; `Conference { url: "javascript:x", .. }` set by hand (a damaged cache) → no `CONFERENCE` line.
  - `kept_lines_are_written_once` (Review Focus 1): read the event of `other_conference_lines_stay_as_they_came`, write it, read again → same `conference`, same `extra`; the written text has each of the five `CONFERENCE` values exactly once.
  - `a_new_conference_is_cleaned`: a 100-character label → 80 characters; `"Web\u{7}ex\n"` → `Webex`; `" https://x.example/ "` → url `https://x.example/`; `ftp://x.example` and `""` → `None`.
  - `join_takes_the_server_then_the_conference_then_the_notes`: `join_url` a Teams link + conference `https://jitsi.example.org/team` + notes with `https://us02web.zoom.us/j/1` → the Teams link; no `join_url` → the jitsi link (not a known service: the calendar says it joins); no conference → the Zoom link; `join_url: "https://example.com/"` (no meeting link) + the conference → the conference.
  - `links_list_the_conference`: `links()` holds the conference's url once, also when the notes hold it too.
  - Property test `a_conference_written_is_read_back` (proptest, as `tests/find.rs` uses it): url `"https://[a-z]{1,12}\\.example(/[a-zA-Z0-9._~-]{0,20}){0,3}"`, label `"\\PC{0,100}"`; `Conference::new` then `write` then `parse` → the event's conference equals the one made.
  - In `tests/find.rs`, `the_search_finds_the_conference`: an event with location `Salle 3` and conference label `Jitsi équipe`, url `https://jitsi.example.org/team`: `location:équipe` and the bare word `jitsi.example.org` find it; the same event without the conference isn't found by either.
- [ ] **Step 3: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-model ical::tests` — expect compile failure (`Conference` missing).
- [ ] **Step 4: Implement** `conference.rs` and its uses. Reading: the line qualifies when `VALUE` is absent or `URI` (any case), the trimmed value is `launchable_link`, and `FEATURE` (split at `,`, trimmed, upper-cased) is absent or not only `PHONE`/`CHAT`/`FEED`; features kept: tokens of ASCII letters, digits and `-`, 1 to 40 bytes, at most 10. Parse arm in `parse_in` before the `meeting::read` arm: `"CONFERENCE" if conference::take(e, l) => {}` (a line not taken falls to `extra`). Writing: after `lines.extend(meeting::lines(e));` push `conference::line`; features written as kept, `AUDIO,VIDEO` when empty; the label in double quotes only when it holds `,`, `;` or `:`. Invitations keep the conference (only `webex_id` is left out of them). `join_link`: `join_url` when a meeting link, then the conference's url when `launchable_link`, then the notes as today. `links`: the conference's url after the join address, once. `find.rs` `place`: location, join url, then the conference's label and url when the location doesn't hold them, joined by `SEP`.
- [ ] **Step 5: Run** the same command — expect PASS; then `TMPDIR=$HOME/.tq cargo test -p quatrelle-model`.
- [ ] **Step 6: Fuzz.** In `fuzz/fuzz_targets/ical.rs` add `let _ = e.join_link();` beside `e.links()`. Write `fuzz/seeds/ical/conference.ics`: one event with every line of the tests above plus `CONFERENCE;FEATURE=;LABEL="unterminated:https://x.example/`, `CONFERENCE;;;:https://x.example/`, `CONFERENCE;VALUE=URI;LABEL=` + 300 × `é` + `:https://x.example/`. Build it: `cd fuzz && TMPDIR=$HOME/.tq cargo +nightly fuzz build ical` (if nightly is missing, `cargo check` in `fuzz/` and say so in the report).
- [ ] **Step 7: Commit** — "Calendar model: CONFERENCE (RFC 7986) read and written, Join and the search take it".

### Task 2: Engine — the rooms in the settings, `calendar.webexRoom`, `conference` in `event.*`

**Files:**
- Create: `crates/engine/src/session/prefs/webex_rooms.rs` (declared `mod webex_rooms; pub use webex_rooms::*;` in `session/prefs.rs`), `crates/engine/src/tests/calendar/webex_rooms.rs` (declared `mod webex_rooms;` in `tests/calendar.rs`)
- Modify: `crates/engine/src/session/prefs.rs` (l.207-210, `Default` l.287), the re-exports of `Prefs` in `crates/engine/src/session.rs` l.55 and `crates/engine/src/lib.rs` l.34 (add `WebexRoom`), `crates/engine/src/cmd/view.rs` (l.133-142), `crates/engine/src/cmd/pim.rs` (`event_from` after `webexId` l.460; `event.create`'s params doc l.111), `crates/engine/src/session/pim/webex.rs` (`webex_link_changed` l.186), `crates/ui-egui/src/lib.rs` (`restore_state` l.847), `crates/engine/src/tests/calendar.rs` (remove `a_webex_personal_room_is_set_and_taken_off`, l.590-608: its cases move to the new file), `crates/engine/src/tests/organize.rs` (junk list l.13)

**Interfaces:**
- Consumes: `quatrelle_model::ical::{Conference, is_webex_link}` (Task 1).
- Produces (as `quatrelle_engine::…`):
  - `#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)] #[serde(default, rename_all = "camelCase")] pub struct WebexRoom { pub name: String, pub url: String, #[serde(skip_serializing_if = "Vec::is_empty")] pub details: Vec<String> }` (empty `name`: the room carried over, shown « Ma salle personnelle »).
  - `pub const WEBEX_ROOMS_MAX: usize = 20; pub const WEBEX_ROOM_NAME_MAX: usize = 80; pub const WEBEX_ROOM_LINES_MAX: usize = 20; pub const WEBEX_ROOM_LINE_MAX: usize = 200;`
  - `Prefs::webex_rooms: Vec<WebexRoom>` (`#[serde(skip_serializing_if = "Vec::is_empty")]`); `Prefs::webex_room: String` stays only to be read: `#[serde(skip_serializing)]`, doc "the single room of 1.0, carried over".
  - `impl Prefs { pub fn carry_over_webex_rooms(&mut self); pub fn set_webex_room(&mut self, url: &str, was: Option<&str>, name: Option<&str>, details: Option<Vec<String>>, remove: bool) -> Result<(), CmdError> }`
  - `calendar.webexRoom` params: `{"url": string, "was"?: string (the link it had: that room's link changes, in its place), "name"?: string, "details"?: string ("\n" between lines) | [string], "remove"?: bool}` → `{"rooms": [WebexRoom…]}`.
  - `event.create`/`event.update` param `"conference"?: {"url": string, "label"?: string} | null`.

- [ ] **Step 1: Write the failing tests** in `tests/calendar/webex_rooms.rs` (`sample_session`, as the parent's tests):
  - `webex_rooms_are_added_changed_and_removed`: add `{"url": " https://acme.webex.com/meet/ana ", "name": "Ana", "details": "N° 123\nVidéo : ana@acme.webex.com"}` → answer `rooms[0]` is `{name: "Ana", url: "https://acme.webex.com/meet/ana", details: ["N° 123", "Vidéo : ana@acme.webex.com"]}`; add `Équipe` `https://acme.webex.com/meet/team` with `"details": ["  ", "N° 9 "]` → details `["N° 9"]`, two rooms in that order; `{"url": "…/meet/ana", "details": []}` → ana's lines gone, name kept; `{"url": "https://acme.webex.com/meet/ana2", "was": "https://acme.webex.com/meet/ana"}` → still first, new link; `{"url": "…/meet/team", "remove": true}` → one room; removing a link no room has → `Err(CmdError::Params(_))`.
  - `webex_rooms_refuse_what_they_cannot_keep` (Review Focus 3): each refused with `Params` and the rooms unchanged: a new room without a name or with `"  "`; an 81-character name (80 passes); 21 lines (20 pass); a 201-character line; `"url": ""`; the four non-Webex links of the old test (`https://evil.example/meet/ana`, `http://acme.webex.com/meet/ana`, `acme.webex.com/meet/ana`, `https://us02web.zoom.us/j/1`); `https://acme.webex.com/meet/ANA` while `…/meet/ana` exists; a link changed (`was`) to another room's; the 21st room after 20; a named room's name set to `""`.
  - `an_old_personal_room_becomes_the_first_room` (Review Focus 2): `serde_json::from_value::<Prefs>(json!({"webexRoom": "https://acme.webex.com/meet/ana", "webexRooms": [{"name": "Équipe", "url": "https://acme.webex.com/meet/team"}]}))`, `carry_over_webex_rooms()` → rooms `[("", …/meet/ana), ("Équipe", …/meet/team)]`, `webex_room` empty; `serde_json::to_value` has no `webexRoom` key; that value read and carried over again → the same two rooms. `{"webexRoom": "https://evil.example/x"}` → no room. The old room already among the rooms → not added twice.
  - `a_hand_edited_rooms_list_is_cut_when_read` (Review Focus 5): 30 rooms built in JSON: room 0 named 500 × `é` with 25 lines of 300 × `x`, room 1 a non-Webex link, room 2 the same link as room 0 in upper case, rooms 3-29 distinct Webex links → after `carry_over_webex_rooms`: 20 rooms, all `is_webex_link`, links unique, room 0's name 80 characters, 20 lines of 200 characters; another setting in the same JSON (`"externalTag": true`) still read.
  - `an_event_takes_a_conference_and_lets_it_go`: `event.create` with `"conference": {"url": "https://jitsi.example.org/team", "label": "Jitsi"}` → stored conference `{url, label "Jitsi", features ["AUDIO","VIDEO"]}`; set its features to `["VIDEO","SCREEN"]` through `s.update_event`; `event.update` without `conference` → unchanged; with `{"url": <same>}` → label and features kept; with `{"url": "https://jitsi.example.org/other", "label": "Autre"}` → replaced, features `AUDIO,VIDEO`; with `null` → `None`. Refused with `Params`, event unchanged: `"conference": "x"`, `{"url": "javascript:alert(1)"}`, `{"url": 5}`, `{"label": "x"}`, `{"url": "https://x.example/", "label": 3}`.
  - `an_invitation_by_mail_carries_the_conference`: on the sample (IMAP) account, `event.create` with a guest and a conference → `sent_calendar(&s, "Sent", <summary>)`'s calendar has the `CONFERENCE;VALUE=URI;…` line once (unfold before searching).
  - `a_webex_link_changed_moves_the_conference`: as `changing_the_link_reaches_the_notes` in `tests/calendar/webex.rs` (make the helpers it needs `pub(super)` there), the appointment also carrying the old link as its conference → after Webex's answer with the new link, the conference's url is the new link and its label unchanged.
  - In `tests/organize.rs`, add to `junk`: `json!({"url": "https://acme.webex.com/meet/x", "was": 3, "name": [1], "details": {"a": 1}, "remove": "yes", "uid": 5, "conference": {"url": [], "label": {}}})` and `json!({"summary": "s", "start": "2026-01-01", "conference": "x"})`.
- [ ] **Step 2: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-engine webex_rooms` — expect compile failure.
- [ ] **Step 3: Implement.** `set_webex_room`: link trimmed and `is_webex_link` (else the old message, "`…` is not a Webex personal room (https://<site>.webex.com/meet/<name>)"); `remove` takes out the room with that link; otherwise the room found by `was` (trimmed), else by `url`, changes; none found: a new room at the end, name required. Lines: control characters out, trimmed, empty ones dropped, then checked against the caps; names trimmed and checked; links compared with `eq_ignore_ascii_case`; a refusal changes nothing. `carry_over_webex_rooms`: the old `webex_room`, when a Webex link no room has, goes first unnamed; `webex_room` is then emptied; then every room is cut, never refused: bad links dropped, duplicates after the first dropped, names and lines cut to the caps (`chars().take`), at most 20. The command (`cmd/view.rs`) reads `details` as a string split at `\n` or an array of strings (anything else: `Params`), runs `set_webex_room`, answers `{"rooms": s.prefs.webex_rooms}`; its params doc says the above. `restore_state` calls `self.session.prefs.carry_over_webex_rooms()` right after `self.session.prefs = p;`. `event_from`: `conference` absent → unchanged; `null` → `None`; an object → `Conference::new(url, label or "")` (`Params` when `None` or a value isn't a string); the same url as the event's keeps its features, and its label when none is given. `webex_link_changed` also moves a conference whose url is the old link.
- [ ] **Step 4: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-engine` — expect PASS (`every_command_survives_junk_params` included); then `TMPDIR=$HOME/.tq cargo test -p quatrelle-ui-egui scheduling` (the UI still reads `prefs.webex_room` and compiles; Tasks 5 and 6 change it). A UI test that runs `calendar.webexRoom` the old way and now fails (expected: `use_my_personal_room_asks_for_it_under_the_button`) gets `#[ignore = "rewritten in Task 5"]` in this commit, nothing else; the report names each one.
- [ ] **Step 5: Commit** — "Calendar: several Webex rooms in the settings (calendar.webexRoom), the old one carried over; event.create and event.update take a conference".

### Task 3: Servers — the conference to CalDAV and back, nothing to Exchange

**Files:**
- Modify: `crates/net/src/fakes/fake_dav.rs` (its test module, after `finds_reads_writes_and_deletes`); the test modules that already test `event_fields` (`crates/net/src/exchange/ews_pim/events.rs`) and `event_json` (`crates/net/src/exchange/graph_pim.rs`) — find them with `grep -rn "event_fields(\|event_json(" crates/net/src`

**Interfaces:**
- Consumes: `quatrelle_model::ical::Conference` (Task 1). Production code changes only if a test shows a gap.

- [ ] **Step 1: Write the tests:**
  - `a_conference_goes_to_the_server_and_back` (fake_dav.rs, set up as `finds_reads_writes_and_deletes`): the server holds an event whose file has `CONFERENCE;VALUE=URI;FEATURE=VIDEO;LABEL=Jitsi:https://jitsi.example.org/team` and `CONFERENCE;VALUE=URI;FEATURE=PHONE:tel:+33-1-23-45-67-89`; the first sync reads conference `{url, "Jitsi", ["VIDEO"]}` and the tel line in `extra`; the event changed here (`summary`, `dirty: true`) and synced → the server's file has each of the two lines exactly once (unfolded); read back, the same conference.
  - `exchange_gets_no_conference`: an event with location and notes, and the same event with `conference: Conference::new("https://acme.webex.com/meet/ana", "Webex")` → `event_fields(e, me)` equal for both; `event_json(e)` equal for both.
- [ ] **Step 2: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-net conference` — expect PASS (Task 1 made it so); a failure is a real gap: fix it in the writer it shows, not in the test.
- [ ] **Step 3: Commit** — "Calendar: a conference goes to CalDAV and back, and nothing of it to Exchange (tests)".

### Task 4: UI — the window keeps the conference; a room's block found and replaced

**Files:**
- Create: `crates/ui-egui/src/dialogs/pim_dialogs/webex_rooms.rs` (declared `mod webex_rooms;` in `dialogs/pim_dialogs.rs` beside `mod webex;`), `crates/ui-egui/src/tests/scheduling/webex_rooms.rs` (declared `mod webex_rooms;` in `tests/scheduling.rs` beside `mod webex;`)
- Modify: `crates/ui-egui/src/dialogs/pim_dialogs/event.rs` (`EventForm` fields after `webex_wait`, `open` l.132, `params` l.295), `crates/ui-egui/src/dialogs/pim_dialogs/webex.rs` (`take_meeting` l.39), `crates/ui-egui/src/app/webex.rs` l.27, `crates/ui-egui/src/i18n.rs`, the `take_meeting(` calls in `crates/ui-egui/src/tests/scheduling/webex.rs` (add `&[]`)

**Interfaces:**
- Consumes: `Conference` (Task 1); `WebexRoom` (Task 2).
- Produces:
  - `pub fn in_every_language(s: &'static str) -> Vec<&'static str>` in `i18n.rs`: `s` and its translation in each catalog, once each.
  - `EventForm::conference: Option<Conference>` (the conference it's saved with) and `EventForm::conference_shown: bool` (its link was in the place or the notes when the window opened, or a room or a meeting put it there).
  - In `webex_rooms.rs`: `pub(crate) fn without_room_block(notes: &str, rooms: &[WebexRoom]) -> String` and `pub(crate) fn add_room(f: &mut EventForm, room: &WebexRoom, rooms: &[WebexRoom])`.
  - `pub(crate) fn take_meeting(f: &mut EventForm, m: &WebexMeeting, rooms: &[WebexRoom])`.

- [ ] **Step 1: Write the failing tests.** Unit tests in `webex_rooms.rs` (`#[cfg(test)] mod tests`), rooms `ana` (`https://acme.webex.com/meet/ana`, details `["N° 123", "Vidéo : ana@acme.webex.com"]`) and `team` (`https://acme.webex.com/meet/team`, no details):
  - `a_rooms_block_is_found_as_it_was_written`: `"Ordre du jour\n\nJoin the Webex meeting:\nhttps://acme.webex.com/meet/ana\nN° 123\nVidéo : ana@acme.webex.com\n"` → `"Ordre du jour"`; the same with `Rejoindre la réunion Webex :` and with `Webex 会議に参加:` → `"Ordre du jour"`; the block alone → `""`.
  - `what_the_room_did_not_write_stays`: a line `Merci d'arriver à l'heure` after the block stays; ana's lines now `["N° 456"]` and the notes' `N° 123` → `N° 123` stays (label and link go); a scheduled meeting's block (`Join the Webex meeting:` then `https://acme.webex.com/acme/j.php?MTID=m1`) stays whole; the label with no link after it, or with a link no room has, stays.
  - `another_room_replaces_the_first`: `EventForm::default()`, `add_room(ana)` then `add_room(team)` → the notes hold team's block once and no ana line; location `…/meet/team` (it was ana's link); `join` team; conference `{team, "Webex", ["AUDIO","VIDEO"]}`; `conference_shown`. With location `Salle 3` typed first → `Salle 3` kept.
  - `a_scheduled_meeting_replaces_a_room` (Review Focus 4): `add_room(ana)`, then `take_meeting(f, &m, &[ana])` with `m.web_link` `https://acme.webex.com/acme/j.php?MTID=m1` → no ana line in the notes, the meeting's block once; location and conference url the meeting's link; `webex_id` set.
  - `the_conference_goes_when_its_link_is_erased`: a form with `conference: Some(jitsi)`, `conference_shown: true`, the link in the notes → `params()["conference"] == {"url": …, "label": …}`; link erased from the notes, not in the place → `params()["conference"].is_null()`; erased from the notes but in the place → kept. A form with no conference → `params()` has no `conference` key.
  - In `tests/scheduling/webex_rooms.rs` (an app as `tests/scheduling/webex.rs` builds one), `a_foreign_conference_stays` (Review Focus 1): an event created through `event.create` with conference `https://jitsi.example.org/team` and notes that don't hold it → `EventForm::open` gives `conference_shown == false`; its reminder changed and saved → the stored event keeps the conference, label and features; an event whose notes hold the link → `conference_shown == true`.
- [ ] **Step 2: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-ui-egui webex_rooms` — expect compile failure.
- [ ] **Step 3: Implement.** `without_room_block`: a block starts at a line equal (trimmed) to one of `in_every_language("Join the Webex meeting:")`, followed by a line equal (trimmed, ASCII case ignored) to a room's link; it takes those two lines and then each following line that is (trimmed) one of that room's lines; every such block goes, and the blank lines left at the end or doubled where it was. `add_room`: nothing while `f.webex_id` is some; the notes without any room's block, then the block (`tl!("Join the Webex meeting:")`, the link, the room's lines, as `add_webex` in `meeting.rs` writes it today); the place set to the link when empty or another room's link; `join`, `conference` (`Conference::new(&room.url, "Webex")`), `conference_shown = true`. `take_meeting` does the same with the meeting's link (its own block, as today). `open`: `conference` from the event, `conference_shown` when its url is in the location or the description. `params`: `"conference"` sent when `conference` is some — `null` when `conference_shown` and the url is in neither the place nor the notes, else `{"url", "label"}`. The app's `take_meeting` call passes `&self.session.prefs.webex_rooms`.
- [ ] **Step 4: Run** the same command, then `TMPDIR=$HOME/.tq cargo test -p quatrelle-ui-egui scheduling` — expect PASS.
- [ ] **Step 5: Commit** — "Appointment window: the conference kept and taken out with its link; a Webex room's block found in any language and replaced".

### Task 5: UI — the Webex Meeting button's menu lists the rooms

**Files:**
- Modify: `crates/ui-egui/src/dialogs/pim_dialogs/meeting.rs` (l.187-295: `add_webex`, `webex_room` and both `webex_button`s; its test `webex_meeting_puts_the_room_in_once` l.632 moves to `webex_rooms.rs`'s tests, rewritten on `add_room`), `crates/ui-egui/src/dialogs/pim_dialogs/webex_rooms.rs`, `crates/ui-egui/src/i18n/*.tsv`, `crates/ui-egui/src/tests/scheduling.rs` (`use_my_personal_room_asks_for_it_under_the_button` l.369 rewritten as below), `crates/ui-egui/src/tests/scheduling/webex.rs` (`the_personal_room_stays_a_choice` l.351 rewritten on the rooms), `crates/ui-egui/src/tests/scheduling/webex_rooms.rs`

**Interfaces:**
- Consumes: `add_room`, `without_room_block` (Task 4); `calendar.webexRoom` (Task 2); `super::webex::new_meeting`.
- Produces: in `webex_rooms.rs`, `pub(super) fn rooms_menu(ui: &mut Ui, t: &Tokens, f: &mut EventForm, s: &Session, acts: &mut Vec<Action>) -> bool` (draws the rooms and « Ajouter une salle… » inside an open menu; `true` when the menu should close) and `fn asking(ui: &mut Ui, t: &Tokens, window: u64, rooms: &[WebexRoom]) -> Option<WebexRoom>` (the name and the link typed, once both are right).

- [ ] **Step 1: Write the failing tests** in `tests/scheduling/webex_rooms.rs` (helpers `open_webex_menu`, `settled_text_at`, `click_at`, `text_at`, `frame_sized` as `tests/scheduling/webex.rs` uses them; rooms set with `a.session.prefs.webex_rooms = vec![…]`):
  - `the_menu_lists_the_rooms_by_name`: rooms unnamed `ana`, `Équipe` `team` → the menu shows `New Webex meeting`, `My personal room`, `Équipe`, `Add a room…`; a click on `Équipe` → the form's location, `join` and conference url are team's link, and the menu is closed.
  - `rooms_are_greyed_with_a_scheduled_meeting`: the form's `webex_id = Some("m1")` → a click on `Équipe` or `Add a room…` changes neither notes nor conference.
  - `add_a_room_asks_its_name_and_link` (replaces `use_my_personal_room_asks_for_it_under_the_button`): no rooms; `Add a room…` → `New Webex room` shown under the button, Options not opened; a link with no name → nothing added; a name and `https://evil.example/meet/ana` → `Not a Webex personal room: https://<site>.webex.com/meet/<name>` shown; with a link another room has → `Another room has this link`; name `Ana` and `https://acme.webex.com/meet/ana`, Enter → `prefs.webex_rooms` is `[Ana]` once the action ran, the form holds its block, `join` and conference.
  - `the_personal_room_stays_a_choice` (in `tests/scheduling/webex.rs`): what it checks today, with the room in `webex_rooms`.
- [ ] **Step 2: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-ui-egui scheduling` — expect the new tests to fail. The tests Task 2 set `#[ignore]` lose it here, rewritten.
- [ ] **Step 3: Implement.** Desktop menu: `New Webex meeting` (enabled as `schedules` today), `ui.separator()`, one button per room (its name, `tl!("My personal room")` when unnamed; hover: its link; disabled while `f.webex_id` is some, disabled hover `tl!("This appointment already has its Webex meeting")`), then `tl!("Add a room…")` (disabled likewise, and at 20 rooms); its asking mode keeps today's temp-data pattern (`webex-room-asking`, `webex-room-typed`, now a name box too, `char_limit(80)`), heading `tl!("New Webex room")`, help `tl!("Its link is on your Webex profile, under Personal Room. Its other lines are added in File › Options › Calendar.")`, button `Add to the meeting`; a taken room runs `calendar.webexRoom {"url", "name"}` and `add_room`. Browser button (`#[cfg(target_arch = "wasm32")]`): one room and no `webex_id` → a click adds it; otherwise the click opens the menu (rooms and « Ajouter une salle… », no scheduled meeting). Hover of the browser button with no rooms: `tl!("Puts your Webex room in the meeting")`. Catalog rows for every new string (French: « Ma salle personnelle », « Ajouter une salle… », « Ce rendez-vous a déjà sa réunion Webex », « Nouvelle salle Webex », « Son lien est sur votre profil Webex, sous Salle personnelle. Ses autres lignes s'ajoutent dans Fichier › Options › Calendrier. », « Une autre salle a déjà ce lien », « Nom de la salle », « Met votre salle Webex dans la réunion »); `meeting.rs` no longer reads `prefs.webex_room`.
- [ ] **Step 4: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-ui-egui` — expect PASS (the i18n tests included); `TMPDIR=$HOME/.tq cargo xtask wasm` — expect the browser build to pass.
- [ ] **Step 5: Commit** — "Appointment window: the Webex Meeting menu lists the Webex rooms, greyed while a meeting is scheduled, and adds one".

### Task 6: UI — « Salles Webex » in File › Options › Calendar

**Files:**
- Create: `crates/ui-egui/src/panes/backstage/options/webex_rooms.rs` (declared `mod webex_rooms;` in `options.rs` beside `mod webex;`), `crates/ui-egui/src/tests/scheduling/room_options.rs` (declared in `tests/scheduling.rs`)
- Modify: `crates/ui-egui/src/panes/backstage/options.rs` (the `Webex personal room` setting, l.315-329), `crates/ui-egui/src/i18n/*.tsv`

**Interfaces:**
- Consumes: `calendar.webexRoom` with `was`, `details`, `remove` (Task 2); `WEBEX_ROOM_*` caps.
- Produces: `pub(super) fn webex_rooms(ui: &mut Ui, s: &Session, t: &Tokens, acts: &mut Vec<Action>)`.

- [ ] **Step 1: Write the failing tests** in `room_options.rs` (Options › Calendar opened as the existing Options tests open it — find one with `grep -rn "Webex personal room" crates/ui-egui/src/tests/`):
  - `options_lists_the_rooms`: rooms unnamed `ana` (details `["N° 123"]`), `Équipe` `team` → `Webex rooms` shown; the unnamed row's name box hints `My personal room`; `Équipe`, both links and `N° 123` shown; an `Add a room` button.
  - `options_adds_edits_and_removes_rooms`: name `Salle B` and link `https://acme.webex.com/meet/b` typed in the add row, `Add a room` clicked → three rooms; team's lines box given `N° 999` and left → team's details `["N° 999"]`; team's link changed to `https://evil.example/x` and left → `Not a Webex personal room: …` under its row, the text still in the box, the rooms unchanged; changed to `https://acme.webex.com/meet/team2` → team's link changed in place; the × of ana → ana gone.
  - `the_unnamed_room_takes_a_name`: `Ana` typed in the unnamed row's name box and left → `prefs.webex_rooms[0].name == "Ana"`.
- [ ] **Step 2: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-ui-egui room_options` — expect failure.
- [ ] **Step 3: Implement.** The setting titled `tl!("Webex rooms")`, about `tl!("Under Webex Meeting in an appointment's window, each room puts its link and its lines in the meeting.")`. A row per room: name box (`char_limit(80)`, hint `My personal room` when unnamed), link box (`hint_text("https://site.webex.com/meet/name")`), lines box (multiline, 2 rows, hint `tl!("Meeting number, video address, dial-in numbers… copied into the notes")`), × (`Icon` close, hover `tl!("Remove this room")`). Each box is saved when left, as today's single field, through `calendar.webexRoom` (`was` for a link; the room's link and `name` or `details` otherwise); text kept in temp data keyed by row and field until saved; a link checked first with `is_webex_link` and against the other rooms, the reason under the row in `t.danger` and nothing sent. The add row: name, link, `tl!("Add a room")` enabled once both are right and under 20 rooms. Catalog rows (French: « Salles Webex », « Ajouter une salle », « Retirer cette salle », « N° de réunion, adresse vidéo, numéros d'appel… recopiés dans les notes », « Sous Réunion Webex, dans la fenêtre d'un rendez-vous, chaque salle met son lien et ses lignes dans la réunion. »).
- [ ] **Step 4: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-ui-egui` — expect PASS. Look at it: `QUATRELLE_SCREENSHOT=$HOME/.tq/rooms.png cargo run -p quatrelle -- --sample --cmd 'calendar.webexRoom={"url":"https://acme.webex.com/meet/ana","name":"Ana","details":"N° 123"}' --cmd ui.options`, read the PNG (Calendar section), fix what reads badly.
- [ ] **Step 5: Commit** — "Options › Calendar: Webex rooms, each with its name, its link and its lines".

### Task 7: Docs and the full gate

**Files:**
- Modify: `docs/webex.md` (the personal room's section: several rooms, the menu, `CONFERENCE`), `docs/gaps.md` (l.806 row: the rooms; no new gap unless one is found), `ROADMAP.md` (a dated line under 2026-11-04), `FORK.md` (a new section after l.795's: « v0.31, septième partie : plusieurs salles Webex et CONFERENCE (2026-11-04) », in French, the same shape as the sixth: taken (`e838af15`'s reading of `CONFERENCE`, rewritten), added here (rooms with free lines, the replacement of a room's block, the taking-out rule, writing everywhere), left (the spec's « Left from upstream » list))

- [ ] **Step 1: Write the docs** above; the command docs come from the params strings, nothing else to regenerate.
- [ ] **Step 2: Run** `TMPDIR=$HOME/.tq cargo xtask ci` — expect every gate green (sizes, authors, layers, wasm, assets, tests, clippy, fmt). Fix what fails in the task it belongs to.
- [ ] **Step 3: Commit** — "Docs: several Webex rooms and CONFERENCE (ZIDAN44 v0.31 calendar feature 4, issue #25)".
