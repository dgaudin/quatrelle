# Private and Show As in the calendar — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Private and Show As seen in the calendar's grid (lock, Outlook's left strip), set from a
right-click menu, carried by Microsoft Graph, and kept right when an invitation is answered.

**Architecture:** The model gains the answered-invitation rule and Graph's names; the engine applies
the rule in `set_answer`, drops alarms from replies, and gives each `AgendaItem` its privacy, Show As
and the user's answer; `event.update` learns `"send": false`. The UI draws from `AgendaItem` and its
menu dispatches existing commands.

**Tech Stack:** Rust 2024, egui 0.37, serde_json; fake Graph server in `crates/net/src/fakes/`.

**Spec:** `docs/superpowers/specs/2026-11-03-private-and-show-as-design.md`

## Global Constraints

- CLAUDE.md applies whole: no `unwrap`/`expect`/`panic!`/indexing that can panic in non-test code; `get()`, checked or saturating arithmetic.
- Every new `.rs` file starts with `// SPDX-License-Identifier: MIT OR Apache-2.0` then `// Copyright (c) 2026 Didier Gaudin`.
- At most 1000 lines per Rust file and 16 Rust files per folder; split before ~800. `crates/ui-egui/src/tests/` is full (16 files): new UI tests go in a new folder `crates/ui-egui/src/tests/pim/`, declared from `tests/pim.rs`.
- Interface text through `tl!`/`tf!`, with a row in every catalog of `crates/ui-egui/src/i18n/*.tsv` (13 files); our own wording.
- Builds one at a time, `TMPDIR=$HOME/.tq`; before each commit `cargo xtask ci` passes (or, for an intermediate commit, `cargo test -p <crate>` plus `cargo clippy -p <crate> --all-targets` and `cargo fmt --check`; the last task runs `cargo xtask ci`).
- Commit messages in English, the repo's style (subject "Area: what changed"), ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Show As's order everywhere: `ShowAs::ALL` (Free, Working Elsewhere, Tentative, Busy, Out of Office).

## Review Focus

1. An Outlook invitation whose organizer meant it free (`X-MICROSOFT-CDO-INTENDEDSTATUS:FREE`) accepted here shows free, not busy → Task 1 test.
2. The reply to an invitation carrying only an alarm Quatrelle can't read (no reminder) has no `BEGIN:VALARM` and no `END:VALARM` left → Task 2 test.
3. Graph returns `showAs: "unknown"`, or no `sensitivity`: the event is busy and public, nothing fails → Task 4 test.
4. A right click on an appointment neither opens it nor drags it → Task 6 test.
5. A colleague's private appointment with no subject in a read-only calendar: the grid says "Private appointment", the menu has only Open (and Join) → Tasks 5 and 6 tests.

---

### Task 1: Model — the answered-invitation rule and Graph's Show As names

**Files:**
- Modify: `crates/model/src/ical/meeting.rs`
- Modify: `crates/model/src/ical.rs` (re-export next to `pub use meeting::{…}`)
- Create: `crates/model/src/ical/tests/showing.rs`; declare `mod showing;` in `crates/model/src/ical/tests.rs` beside `mod find; mod zones;`

**Interfaces:**
- Produces: `ShowAs::graph(&self) -> &'static str`; `pub fn intended_show_as(e: &CalEvent) -> Option<ShowAs>`; `pub fn show_as_answered(e: &mut CalEvent, partstat: &str)`, all reachable as `quatrelle_model::ical::…`.

- [ ] **Step 1: Write the failing tests** in `tests/showing.rs`:
  - `accepted_shows_as_the_organizer_meant`: event `show_as: Tentative`, `extra: ["X-MICROSOFT-CDO-INTENDEDSTATUS:BUSY"]` → `show_as_answered(&mut e, "ACCEPTED")` gives `Busy`; with `INTENDEDSTATUS:FREE` → `Free` (Review Focus 1); with `INTENDEDSTATUS:OOF` → `OutOfOffice`; the `extra` line is still there once.
  - `accepted_without_intent_is_busy_when_it_was_tentative`: no intended line, `Tentative` → `Busy`; `Free` stays `Free`; `WorkingElsewhere` stays.
  - `tentatively_accepted_is_tentative_unless_free`: `"TENTATIVE"` on `Busy` → `Tentative`; on `Free` → `Free`.
  - `declined_or_junk_leaves_it`: `"DECLINED"`, `""`, `"NEEDS-ACTION"`, `"x\u{0}"` leave `Busy` as `Busy`; partstat matched any case (`"accepted"` works).
  - `intended_status_with_junk`: `"X-MICROSOFT-CDO-INTENDEDSTATUS:NONSENSE"` → `intended_show_as` is `None`; an empty `extra` → `None`; the first readable line counts.
  - `graph_names_both_ways`: for each `ShowAs::ALL`, `ShowAs::from_cdo(x.graph()) == Some(x)`; `graph()` gives exactly `free`, `workingElsewhere`, `tentative`, `busy`, `oof`; `from_cdo("unknown")` is `None`.
- [ ] **Step 2: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-model ical::tests::showing` — expect compile failure (functions missing).
- [ ] **Step 3: Implement** in `meeting.rs`: `graph()` (the five names above); `intended_show_as` reads the first `extra` line whose name (before `:` or `;`, any case) is `X-MICROSOFT-CDO-INTENDEDSTATUS` and whose value `ShowAs::from_cdo` reads; `show_as_answered` applies: `ACCEPTED` → intended, else `Busy` if now `Tentative`, else unchanged; `TENTATIVE` → `Tentative` unless now `Free`; anything else unchanged. Doc comments say why (Outlook sends the guest's copy tentative, `1ae7e345`).
- [ ] **Step 4: Run** the same command — expect PASS; then `cargo test -p quatrelle-model`.
- [ ] **Step 5: Commit** — "Calendar model: an answered invitation shows as its organizer meant, and Graph's Show As names".

### Task 2: Engine — answering sets Show As, the reply carries no alarm

**Files:**
- Modify: `crates/engine/src/session/invite.rs` (`set_answer` ~l.562, `send_reply` ~l.480)
- Create: `crates/engine/src/tests/invites/showing.rs`; declare `mod showing;` in `crates/engine/src/tests/invites.rs` beside `mod answered;` (it uses the parent's `deliver`, `me`, `ANA`, `NOW` through `use super::*;`)

**Interfaces:**
- Consumes: `quatrelle_model::ical::show_as_answered` (Task 1); existing `ical::set_reminder(&mut CalEvent, Option<i64>)`.

- [ ] **Step 1: Write the failing tests** (sample session, as `tests/invites.rs` does; the reply is read with `sent_calendar(&s, "Sent", "Accepted: <summary>")`):
  - `an_outlook_invitation_accepted_shows_as_busy`: deliver a `REQUEST` from `ANA`, event organized by Ana, `attendees: [me("NEEDS-ACTION")]`, `show_as: Tentative`, `extra: ["X-MICROSOFT-CDO-INTENDEDSTATUS:BUSY"]`; `invite.respond` accepted → `find_event(uid).event.show_as == Busy`, and `quatrelle_model::ical::write` of the stored event contains no `X-MICROSOFT-CDO-BUSYSTATUS` line (busy is the default) and the intended line once. A second event answered `tentative` → `Tentative`.
  - `the_reply_carries_no_alarm`: the delivered event has `reminder: Some(15)` and, in `extra`, an alarm Quatrelle can't read: `BEGIN:VALARM`, `ACTION:AUDIO`, `TRIGGER;RELATED=END:PT5M`, `END:VALARM`; after `invite.respond` accepted, the sent reply's raw text contains neither `VALARM` nor `TRIGGER`; the user's own calendar copy still has its reminder (15).
  - `an_alarm_alone_leaves_the_reply_whole` (Review Focus 2): same, without `reminder`; the reply contains no `BEGIN:VALARM` and no `END:VALARM`.
- [ ] **Step 2: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-engine tests::invites::showing` — expect FAIL (show_as stays Tentative; reply has `TRIGGER`/`END:VALARM`).
- [ ] **Step 3: Implement**: `set_answer` ends with `ical::show_as_answered(e, stat);`; in `send_reply` replace the `reply.extra.retain(… BEGIN:VALARM …)` line with `ical::set_reminder(&mut reply, None);` and a comment (RFC 5546: no alarm in a `REPLY`; an alarm cut at its first line left an `END:VALARM` never begun).
- [ ] **Step 4: Run** the tests — PASS; then `cargo test -p quatrelle-engine` (the existing invitation tests must stay green).
- [ ] **Step 5: Commit** — "Invitations: answering one sets how it shows, and the answer goes without alarms".

### Task 3: Engine — `AgendaItem` says private, Show As and the user's answer; `event.update` takes `send`

**Files:**
- Modify: `crates/engine/src/session/pim.rs` (`AgendaItem` l.71, `agenda()` ~l.302)
- Modify: `crates/engine/src/cmd/pim.rs` (`calendar.events` output ~l.70, `event.update` l.153 and its params doc)
- Modify: `crates/engine/src/sample/pim.rs` (lunch private, gallery opening out of office)
- Test: `crates/engine/src/tests/calendar.rs` is 740 lines: add a new file `crates/engine/src/tests/calendar/showing.rs` declared from `tests/calendar.rs`

**Interfaces:**
- Produces on `AgendaItem`: `pub private: bool`, `pub show_as: quatrelle_model::ical::ShowAs`, `pub answer: Option<String>` (doc: the user's answer, `ACCEPTED`/`TENTATIVE`/`DECLINED`/`NEEDS-ACTION`, when it's a meeting someone else organizes in a calendar the user can write; `None` otherwise). `calendar.events` items gain `"private"`, `"showAs"` (`ShowAs::param()`), `"answer"` (string or null). `event.update` params: `"send"?: bool` (default true).

- [ ] **Step 1: Write the failing tests** in `tests/calendar/showing.rs`:
  - `the_agenda_says_private_show_as_and_answer`: sample session; in `s.agenda(…)` around the sample's days, "Lunch with Jun" has `private: true`, `answer: None`; "Winter group show opening" has `show_as: OutOfOffice`; "Sound mix playback" has `answer: Some("NEEDS-ACTION")`; after `event.respond` accepted on it, `Some("ACCEPTED")`; `calendar.events` gives `"private": true` for the lunch and `"showAs": "outOfOffice"` for the opening.
  - `an_organizer_without_a_known_address_is_answered_by_my_answer`: an event with `organizer: None`, `my_answer: "TENTATIVE"` and guests → `answer: Some("TENTATIVE")`; with `organizer: None` and no `my_answer` → `None`.
  - `a_read_only_calendars_meeting_has_no_answer`: the same invitation in a calendar with `read_only: true` (via `Event::DavSynced`, as `weekly_series` does in `tests/invites.rs`) → `answer: None`.
  - `private_and_show_as_change_without_telling_the_guests`: `event.create` a meeting with `"attendees": "zoe@studio.example"`; count the sample's `Sent` messages (`store.load_folder("sample", "Sent")`); `event.update {"uid", "private": true, "showAs": "free", "send": false}` → the event is private and free, `Sent` count unchanged; `event.update {"uid", "summary": "Renamed"}` → one more sent.
- [ ] **Step 2: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-engine tests::calendar::showing` — expect compile failure.
- [ ] **Step 3: Implement**: the three fields filled in `agenda()` (someone else's = organizer known and not among the user's addresses after `without_mailto`, or organizer unknown and `my_answer` non-empty; and guests or `my_answer`; calendar not read-only; the answer is `my_answer` when set, else the partstat of the user's own guest line, else `NEEDS-ACTION`); `calendar.events` fields; `event.update` reads `p::bool(v, "send").unwrap_or(true)` and passes it to `tell_guests`; params doc `{"uid": string, "send"?: bool (the guests of a meeting you organize hear of the change; default true), … the fields of event.create to change}`. Sample: `private: true` on the lunch, `show_as: ShowAs::OutOfOffice` on the opening.
- [ ] **Step 4: Run** `cargo test -p quatrelle-engine` (whole crate: `every_command_survives_junk_params` and sample-based tests included; fix any sample-based expectation the two sample changes break, by updating the expectation, not by reverting the sample).
- [ ] **Step 5: Commit** — "Calendar: each appointment shown says if it's private, how it shows and the user's answer; event.update can keep the guests out of it".

### Task 4: Microsoft Graph — sensitivity and Show As both ways

**Files:**
- Modify: `crates/net/src/exchange/graph_pim.rs` (`EVENT_FIELDS` l.24, `event_of` l.193, `event_json` l.255)
- Create: `crates/net/src/fakes/fake_graph/tests/events.rs`; declare `mod events;` in `crates/net/src/fakes/fake_graph/tests.rs` beside `mod cached_mode;` (that file is 775 lines: don't add to it)

**Interfaces:**
- Consumes: `ShowAs::graph()` (Task 1), `ShowAs::from_cdo`, `PrivateKind::from_sensitivity`, `PrivateKind::sensitivity`.

- [ ] **Step 1: Write the failing tests** in `tests/events.rs`, modelled on `an_events_webex_id_round_trips_through_graph` (same `serve`, `Data`, `Client::connect`, `graph_pim::sync`):
  - `privacy_and_show_as_round_trip_through_graph`: a dirty local event `private: true`, `show_as: OutOfOffice` → after sync the fake's stored event has `"sensitivity": "private"`, `"showAs": "oof"`, and the event read back is private, `PrivateKind::Private`, `OutOfOffice`. Then the read-back event changed to `private_kind: Confidential`, `show_as: Free`, dirty, synced again → stored `"confidential"`, `"free"`; read back the same. A public busy event is stored `"normal"`, `"busy"`.
  - `unknown_show_as_and_no_sensitivity_read_as_busy_and_public` (Review Focus 3): put in the fake's `Data::events` a Graph event with `"showAs": "unknown"` and no `sensitivity`, and one with `"sensitivity": "personal"`, `"showAs": "workingElsewhere"` → the first reads busy and not private; the second private `Personal`, `WorkingElsewhere`; the sync has no errors.
- [ ] **Step 2: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-net fake_graph::tests::events` — expect FAIL.
- [ ] **Step 3: Implement**: `EVENT_FIELDS` gains `,sensitivity,showAs`; `event_of` sets `private`, `private_kind`, `show_as` (missing or unknown → default); `event_json` writes `"sensitivity"` (`private_kind.sensitivity()` lower-cased when private, else `"normal"`) and `"showAs"` (`show_as.graph()`). If the fake doesn't keep or return these fields, extend the fake the least needed (it should store posted JSON as is).
- [ ] **Step 4: Run** the tests — PASS; then `cargo test -p quatrelle-net fake_graph`.
- [ ] **Step 5: Commit** — "Graph: an appointment's sensitivity and Show As come and go with it".

### Task 5: The grid draws Show As, the lock and "Private appointment"

**Files:**
- Modify: `crates/ui-egui/src/panes/calendar.rs` (`chip` l.170, `hover_card` l.312, month view l.~395)
- Modify: `crates/ui-egui/src/panes/calendar/days.rs` (chip texts l.117 and l.216)
- Modify: every `crates/ui-egui/src/i18n/*.tsv` ("Private appointment")
- Create: `crates/ui-egui/src/tests/pim/calendar_look.rs`; declare `mod calendar_look;` in `crates/ui-egui/src/tests/pim.rs`

**Interfaces:**
- Consumes: `AgendaItem::{private, show_as}` (Task 3).
- Produces in `calendar.rs`: `pub(crate) fn title(item: &AgendaItem) -> String` (the summary; `tl!("Private appointment")` when it's empty and the item is private) and `pub(crate) fn strip_look(s: ShowAs) -> StripLook` with `pub(crate) enum StripLook { Solid, Hatched, Hollow, Purple, Dotted }` (Busy, Tentative, Free, OutOfOffice, WorkingElsewhere), so the drawing choice is testable without pixels.

- [ ] **Step 1: Write the failing tests** in `tests/pim/calendar_look.rs`:
  - `each_show_as_has_its_strip`: `strip_look` maps the five as above.
  - `a_private_appointment_without_a_subject_has_a_title` (Review Focus 5, grid half): `title` of an item with `summary: ""`, `private: true` is "Private appointment"; with `private: false` it stays empty; a summary is kept as is.
  - `the_week_and_month_draw_every_look`: with `app(...)`, give the sample appointments each Show As and private (via `event.update` with `send: false`), render the Week then Month view for a few `frame`s — every chip is drawn (`ctx.read_response(chip_id(item))` is some) and nothing panics.
- [ ] **Step 2: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-ui-egui tests::pim::calendar_look` — expect compile failure.
- [ ] **Step 3: Implement** in `chip`: a strip 5 px wide (text starts at `rect.min.x + 8.0`), drawn by `strip_look`: Solid = `rect_filled` in the item's colour; Hatched = `t.surface` fill and diagonal lines in the colour every 4 px, clipped to the strip; Hollow = `t.surface` fill with a 1 px outline in the colour; Purple = `rect_filled` with `theme::category_color(4, t.dark)`; Dotted = `t.surface` fill with dots (radius 1) in the colour every 4 px down its middle. The body fill and its hollow look for `needs_reply || status == "TENTATIVE"` stay as they are. When `item.private` and the chip is at least 30 px wide: `icons::paint(painter, 11×11 rect 3 px in from the top right, Icon::Lock, t.dim)`, and the text 14 px narrower. Every chip text built from `it.summary` (days.rs twice, month view, `hover_card`) uses `title(it)`. Add "Private appointment" to the 13 catalogs (French "Rendez-vous privé").
- [ ] **Step 4: Run** `cargo test -p quatrelle-ui-egui` (the i18n test fails on a missing translation) — PASS.
- [ ] **Step 5: Commit** — "Calendar: the grid shows how each appointment's time shows, and a lock on a private one".

### Task 6: The right-click menu on an appointment

**Files:**
- Create: `crates/ui-egui/src/panes/calendar/menu.rs` (declare `mod menu;` in `calendar.rs`)
- Modify: `crates/ui-egui/src/panes/calendar.rs` (`chip` takes `acts: &mut Vec<Action>` and attaches the menu; its three callers pass theirs)
- Modify: `crates/ui-egui/src/panes/calendar/drag.rs` only if a secondary-button drag starts a move today (l.188-190 use `drag_started()`, which counts any button): then take the primary button only
- Modify: `crates/ui-egui/src/dialogs/pim_dialogs/meeting.rs` (lift the Show As label closure of `show_as_choice` into `pub(crate) fn show_as_label(x: ShowAs) -> &'static str`, used by both) and `crates/ui-egui/src/dialogs/pim_dialogs.rs` (`all_categories` becomes `pub(crate)`)
- Create: `crates/ui-egui/src/tests/pim/calendar_menu.rs`; declare it in `tests/pim.rs`

**Interfaces:**
- Consumes: `AgendaItem::{answer, show_as, private, recurring, occurrence}` (Task 3), `event.update` `send` (Task 3), `title` (Task 5).
- Produces: `pub(super) fn menu(ui: &mut Ui, s: &Session, item: &AgendaItem, acts: &mut Vec<Action>)`.

- [ ] **Step 1: Write the failing tests** in `tests/pim/calendar_menu.rs` (Day view of the sample, as `appointments_are_dragged_to_another_time` in `tests/pim.rs` does; a small helper finds a menu entry by its drawn text in the frame's shapes and clicks its centre):
  - `a_right_click_opens_the_appointments_menu`: secondary press and release on "Lunch with Jun" → the menu shows Open, Show As, Categorize, Private, Delete, and no Accept; no event dialog opened (`a.ui.dialog` none) and the lunch hasn't moved (Review Focus 4).
  - `private_from_the_menu_tells_no_one`: on the lunch (private in the sample), click Private → the lunch is no longer private; `Sent` count unchanged.
  - `show_as_from_the_menu`: Show As › Free on the lunch → `show_as == Free`.
  - `an_invitation_is_answered_from_the_menu`: on "Sound mix playback", Accept is there; click it → `answer == Some("ACCEPTED")` and the reply is in Sent.
  - `a_series_asks_which_to_delete`: on a "Team stand-up" occurrence, Delete › Delete this occurrence → that occurrence is gone, the series stays.
  - `a_read_only_calendar_offers_only_open` (Review Focus 5, menu half): an appointment in a read-only calendar (subscribed or `read_only: true`) → Open only (Join when it has a meeting link), no Private, no Delete.
- [ ] **Step 2: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-ui-egui tests::pim::calendar_menu` — expect FAIL.
- [ ] **Step 3: Implement** `menu`, attached with `resp.context_menu(|ui| menu(ui, s, item, acts))` in `chip`. Entries, each closing the menu (`ui.close()`), separators between groups: Open (`open(s, item)`); Join meeting when `s.find_event(&item.uid)` gives a `join_link()` (`Action::OpenMeeting(url)`); writable calendar only (`s.pim` calendar `!read_only`) from here: Accept / Tentative / Decline as `ui.radio` with the current `answer` ticked, when `answer` is some (`Action::Run("event.respond", {"uid", "answer": "accepted"|"tentative"|"declined"})`); Show As ▸ (`ui.menu_button`, `ui.radio` per `ShowAs::ALL` with `show_as_label`, `Action::Run("event.update", {"uid", "showAs": x.param(), "send": false})`); Categorize ▸ (`ui.checkbox` per `all_categories(s, &event.categories)` in its colour as the window's Categorize; the new list sent as `{"uid", "categories": [...], "send": false}`); Private (`ui.checkbox`, the lock beside it; `{"uid", "private": on, "send": false}`); Delete (`event.delete {"uid"}`; on `item.recurring` a submenu "Delete this occurrence" `{"uid", "occurrence": item.occurrence}` / "Delete the series" `{"uid"}`). The event's categories and link are read with `find_event` only inside the menu's closure. Strings reuse the existing catalog keys (Open, Join meeting, Accept, Tentative, Decline, Show As, Categorize, Private, Delete, Delete this occurrence, Delete the series); add any new one to the 13 catalogs.
- [ ] **Step 4: Run** `cargo test -p quatrelle-ui-egui` — PASS; then `TMPDIR=$HOME/.tq cargo xtask ci`.
- [ ] **Step 5: Commit** — "Calendar: a right click on an appointment opens, answers, shows as, categorizes, makes private or deletes it".

### Task 7 (controller, not a subagent): the real window, FORK.md, test build

- [ ] `QUATRELLE_SCREENSHOT=… cargo run -p quatrelle -- --sample --cmd 'calendar.view={"mode":"week"}'` (light and dark): the lock on the lunch, the purple strip on the opening, the hatched strip on the unanswered mix; read each PNG. The menu opened by hand if needed.
- [ ] FORK.md: the fifth part of v0.31 (what was taken from `1ae7e345`, `6d1ccb3c`, what `5fa58c50` and `418faa02` left, the menu and the lock); survey `survey-v0.31.md` n° 3 marked done.
- [ ] `cargo xtask ci`, push, `~/mailcraft/merge-tools/publish.sh <sha>`; carnet items noted in memory (`reference-carnet-entretien`) while the carnet is out of reach.
