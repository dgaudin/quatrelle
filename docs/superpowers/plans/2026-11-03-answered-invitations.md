# Answered invitations leave the Inbox — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Once a meeting invitation is answered (in the message or in the calendar), or a cancelled meeting removed from the calendar, its message and the meeting's earlier invitations in the Inbox go to Deleted Items, behind one switch in File › Options › Calendar (on by default).

**Architecture:** A new engine module `session/invite/tidy.rs` finds the meeting's mail in the account's Inbox (summaries filtered by title or organizer, then only downloaded sources parsed) and moves it through a move helper taken out of `move_selection`. The three answer paths call it. On Exchange, appointments get their `SEQUENCE` and a move of an item already gone is not an error.

**Tech Stack:** Rust, egui, serde_json; EWS (SOAP) in `net`.

**Spec:** `docs/superpowers/specs/2026-11-03-answered-invitations-design.md` (read it first; rules 1–6 are referred to by number below).

**Upstream reference (read, never apply):** `git show 484e4c2f 25c6997b 3c4febce` (remote `zidan`); its `tidy.rs` is the model for Task 2, but its code builds on `497a480a` and reads 2000 sources: follow the spec where they differ.

## Global Constraints

- Read `CLAUDE.md` first: no `unwrap`/`expect`/`panic!`/panicking indexing in non-test code; files ≤ 1000 lines (split before ~800); ≤ 16 Rust files per folder (`crates/engine/src/session/` is full: new code goes in `session/invite/`); a new `.rs` file starts with `// SPDX-License-Identifier: MIT OR Apache-2.0` then `// Copyright (c) 2026 Didier Gaudin`; interface text through `tl!`/`tf!` with all 13 catalogs in `crates/ui-egui/src/i18n/`.
- Every cargo command: `mkdir -p ~/.tq` once, then `TMPDIR=$HOME/.tq CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo …`. Focused tests only (`cargo test -p <crate> --lib <path>`), then `cargo clippy -p <crate> --all-targets -- -D warnings` and `cargo fmt --all`. Never `cargo xtask ci`, never a release build, never two cargo commands at once: the controller runs the full gates.
- Work on branch `answered-invites` in the main checkout; commit at the end of each task, author `Didier Gaudin <didier.gaudin@gmail.com>` (the repo's local config). Commit messages in this repo's style (a plain sentence title, a body saying why), ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Test data uses `*.example` domains only.
- The switch is `Prefs::tidy_answered_invites` and the command `view.tidyAnsweredInvites`; the moves go to Deleted Items only (`FolderRole::Trash`, beside each message); versions compare by `SEQUENCE` only, never `DTSTAMP`.

## Review Focus

- A message whose source isn't downloaded: never read, never fetched, stays → Task 2, `a_message_not_downloaded_stays_and_isnt_fetched`.
- A meeting with a title shorter than 3 characters: the subject filter must not match every message → Task 2, `a_short_title_matches_no_subject`.
- An invitation answered while it sits in Deleted Items: it stays there, the Inbox's earlier versions still go → Task 2, `answered_in_deleted_items_it_stays_and_the_inbox_ones_go`.
- An Exchange colleague's meeting (no organizer known) answered in the calendar: counted as someone else's, found by its title, newer update kept → Task 3, `an_exchange_colleagues_meeting_is_tidied_by_its_title`.
- An Exchange move where the server already took one of the items: the others move and no "The server refused a change" notice → Task 1, `a_move_of_an_item_already_gone_moves_the_rest_quietly`.

---

### Task 1: Exchange — the appointment's version, and a move that tolerates an item gone

Model: sonnet, effort medium.

**Files:**
- Modify: `crates/net/src/exchange/ews_pim.rs` (`APPOINTMENT_FIELDS`, ~line 100)
- Modify: `crates/net/src/exchange/ews_pim/events.rs` (`event_of`, ~line 284)
- Modify: `crates/net/src/exchange/ews/requests.rs` (beside `call_all`, ~line 292)
- Modify: `crates/net/src/exchange/ews/changes.rs` (`Op::Move`, ~line 113)
- Test: `crates/net/src/exchange/ews_pim/tests.rs`, `crates/net/src/fakes/fake_ews/tests/account.rs`

**Interfaces:**
- Produces: `CalEvent::sequence` (a `u32`) filled from `calendar:AppointmentSequenceNumber` for Exchange appointments (0 when absent or unreadable). Task 3 relies on it.
- Produces: `pub(crate) fn call_all_present(&self, body: &str) -> Result<usize>` on the EWS client: like `call_all`, but a response message whose `ResponseCode` is `ErrorItemNotFound` is skipped (not an error); returns how many were skipped.

- [ ] **Step 1: Write the failing tests**
  - `ews_pim/tests.rs`: `an_appointment_carries_its_version` — parse a `CalendarItem` built like the one in `ones_own_meeting_is_known_by_the_answer_organizer` with `<t:AppointmentSequenceNumber>3</t:AppointmentSequenceNumber>`: `event_of(it).unwrap().sequence == 3`; without the element, and with `x`, `sequence == 0`; `APPOINTMENT_FIELDS.contains(&"calendar:AppointmentSequenceNumber")`.
  - `fake_ews/tests/account.rs`: `a_move_of_an_item_already_gone_moves_the_rest_quietly` — set up like `folder_edits_queued_together_each_find_their_folder` with two messages in the Inbox; once online, take one item out of `data.lock().unwrap().items` (as if the server had moved it); submit `Op::Move` of both keys to `"Deleted Items"`; `eventually` the other item's `folder` is Deleted Items' id on the server; collect events for a moment and assert none is `Event::OpFailed`.
- [ ] **Step 2: Run them to see them fail**
  Run: `TMPDIR=$HOME/.tq CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p quatrelle-net --lib an_appointment_carries_its_version a_move_of_an_item_already_gone`
  Expected: the sequence test fails (`0 != 3`); the move test sees an `OpFailed`.
- [ ] **Step 3: Implement** — add the field name to `APPOINTMENT_FIELDS` (the array becomes `[&str; 28]`; update the doc comment's list if it names fields); read it in `event_of` (trimmed, parsed, else 0); add `call_all_present` (log at `info` how many were already gone, with the operation's first word); use it in `Op::Move` instead of `call_all`. Nothing writes the sequence back to Exchange.
- [ ] **Step 4: Run the tests to see them pass**, plus `cargo test -p quatrelle-net --lib ews_pim` and `--lib fake_ews` (all green), then clippy and fmt.
- [ ] **Step 5: Commit** — "Exchange: appointments carry their version, and a move of an item already gone isn't an error".

### Task 2: Engine — the switch, the tidy, and an answer in the message

Model: opus, effort high.

**Files:**
- Modify: `crates/engine/src/session/mailboxes/folder_edits.rs` (take the moving out of `move_selection`; add `successor_of`)
- Create: `crates/engine/src/session/invite/tidy.rs`
- Modify: `crates/engine/src/session/invite.rs` (`mod tidy;`, `calendar_in`, `respond_to_invite`)
- Modify: `crates/engine/src/session/prefs.rs` (field + default)
- Modify: `crates/engine/src/cmd/navigation.rs` (command beside `view.signatureDashes`)
- Test: create `crates/engine/src/tests/invites/answered.rs`; modify `crates/engine/src/tests/invites.rs` (`mod answered;`, the delivery helper split)

**Interfaces:**
- Produces in `folder_edits.rs`:
  - `pub(crate) fn move_messages(&mut self, msgs: &[Arc<Summary>], target: &MoveTarget) -> Result<usize, CmdError>` — `move_selection`'s grouping by account and mailbox, destination (`folder_beside` for a role), skipping those already there, `Undo::Move` (Message-ID from each `Summary`) and `apply_op`; no `after_removal`. `move_selection` becomes `successor` + `move_messages(&self.selected(), &target)` + `after_removal`, unchanged in behaviour.
  - `pub(crate) fn successor_of(&self, read: &MessageKey, gone: &[MessageKey]) -> Option<MessageKey>` — rule 6: in `self.list`, the first after `read` not in `gone`, else the nearest before it not in `gone`.
- Produces in `invite.rs`: `fn calendar_in(&self, raw: &[u8]) -> Option<(ICalendar, String)>` (the body of `message_invitation`, which becomes `readable_raw` + `calendar_in`).
- Produces in `tidy.rs` (`impl Session`):
  - `pub(super) fn tidy_meeting_mail(&mut self, account: &str, uid: &str, methods: &[&str], only: Option<&[i64]>, upto: &[CalEvent], acted_on: Option<&MessageKey>) -> usize` — returns 0 when `prefs.tidy_answered_invites` is off. `upto` is the version dealt with (its parts); the meeting's title and organizer for the candidate filter come from its series part, else its first.
  - constants `MAX_CANDIDATES: usize = 200`, `MAX_READ: usize = 16 << 20`, `MAX_MESSAGE: usize = 8 << 20`, `MIN_TITLE: usize = 3`.
  - free fn `newer_than(part: &CalEvent, upto: &[CalEvent]) -> bool` — rule 3: compare with the `upto` part of the same `recurrence_id`, else the series; `part.sequence > that.sequence`; nothing to compare with → false.
- Produces in `prefs.rs`: `pub tidy_answered_invites: bool`, `#[serde(default = "yes")]`, `true` in `Default`.
- Produces: command `view.tidyAnsweredInvites`, label "Move meeting invitations to Deleted Items once answered", location "File › Options", params `{"on"?: bool}` (toggles without), result `{"on": bool}`.
- Produces: `invite.respond`'s result gains `"moved": n`.

- [ ] **Step 1: Split the test helper.** In `tests/invites.rs`, `deliver` becomes `put(s, folder: &str, n, from, subject: &str, method, events, stamp) -> MessageKey` (writes the message into that folder of `sample`, no opening) and `deliver` = `put(s, "INBOX", n, from, &format!("{method} {n}"), …)` then opening it, as now. Existing tests unchanged and green.
- [ ] **Step 2: Write the failing tests** in `tests/invites/answered.rs` (`use super::*;`; meeting "Mix review" organized by `ANA`, the user `me(..)` among its guests; the sample account is not Exchange unless said):
  - `answered_in_the_message_takes_its_earlier_versions_and_leaves_a_newer_one` — Inbox: version seq 1 (n=1), seq 3 (n=3), then seq 2 delivered and opened. Note the expected next message from `s.list` before answering (`successor_of(answered, [n1, n2])`). `invite.respond {"answer":"accepted"}` → result `moved == 2`; the sample's Trash (`folder_state`) holds Message-IDs `<i1@x>` and `<i2@x>` (the sample account moves at once, as in `organize.rs`'s `undo_puts_back_moves_flags_and_categories`; assert on folders, not jobs), the Inbox list doesn't; n3 still in the Inbox list; `s.reading` is the expected next and it isn't marked read.
  - `with_the_switch_off_nothing_moves` — `view.tidyAnsweredInvites {"on": false}` returns `{"on": false}`; answering moves nothing (`moved == 0`), the message stays in the Inbox list and its window, opened first with `pop_out`, is closed (`s.windows` empty).
  - `undo_puts_them_back` — after an answer that moved two, `edit.undo` succeeds and both are back in the Inbox list, gone from Trash.
  - `only_messages_named_by_title_or_organizer_are_looked_at` — put (not open) an invitation to the meeting from `ANA` under subject "New time" (found: sender), and one from `zoe@soundlab.example` under subject "Agenda" carrying the same meeting (not looked at: neither title nor organizer) → after answering, the first moved, the second stayed.
  - `a_message_not_downloaded_stays_and_isnt_fetched` — an earlier version whose body is removed from the store (`store.remove_body`) stays in the Inbox, and no `Job::FetchBody` for it is queued.
  - `a_short_title_matches_no_subject` — meeting titled "ab": a message from `zoe@…` with subject "ab ab" carrying the meeting stays; one from `ANA` goes.
  - `answered_in_deleted_items_it_stays_and_the_inbox_ones_go` — the answered invitation is put in the sample's Trash folder and opened there; an earlier version in the Inbox → `moved == 1`: the Inbox one is now in Trash, the answered one is in Trash once.
  - `an_occurrence_answered_moves_only_its_invitations` — with `weekly_series`: an invitation naming only occurrence X (seq 2, opened, answered with `invite.respond`) and an invitation to the whole series in the Inbox → only X's moves.
  - The existing `a_group_invitation_shows_its_answer_and_goes` stays green unchanged.
- [ ] **Step 3: Run them to see them fail**
  Run: `TMPDIR=$HOME/.tq CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p quatrelle-engine --lib tests::invites`
  Expected: does not compile (`view.tidyAnsweredInvites` unknown / `put` missing), then fails on `moved`.
- [ ] **Step 4: Implement** the interfaces above. `tidy_meeting_mail`, in order: the switch; `acted_on` when not in Deleted Items, Sent Items, Drafts or the Outbox (`OUTBOX`); then the account's Inbox (`folder_state`), newest first, keeping summaries whose subject contains the title (case aside, title ≥ `MIN_TITLE` chars after trimming) or whose `from` is the organizer (`same_address`), at most `MAX_CANDIDATES`; for each, `store.read_body` (never a fetch) within `MAX_READ`, skipping those over `MAX_MESSAGE` or without `text/calendar` / `.ics` (ASCII case aside), then `calendar_in`: kept when its method is one of `methods`, it has parts with `same_meeting(uid)`, `only` holds (every part's `recurrence_id` in it) and no part is `newer_than(upto)`. Then the next message (only when the message read is among them: `successor_of`), `move_messages(.., &MoveTarget::Role(FolderRole::Trash))` (an error is logged at `info`, 0 moved), and `after_removal(next)` in that case only. `respond_to_invite`: answer, close the window (always, as now), then `tidy_meeting_mail(&key.account, &uid, &["REQUEST"], only, &answered_parts, Some(&key))` where `only` is the answered occurrences when the series wasn't among the parts; add `"moved"` to the result. `answer_invite` keeps its signature; get `uid`/parts from the message's calendar.
- [ ] **Step 5: Run the tests to see them pass**, then the whole engine lib (`cargo test -p quatrelle-engine --lib`, including `every_command_survives_junk_params`), clippy and fmt.
- [ ] **Step 6: Commit** — "Invitations: an answered one takes its earlier versions out of the Inbox, behind a switch".

### Task 3: Engine — answered in the calendar, and a cancellation removed

Model: opus, effort high.

**Files:**
- Modify: `crates/engine/src/session/invite.rs` (`respond_to_event`, `apply_invite`)
- Test: `crates/engine/src/tests/invites/answered.rs`

**Interfaces:**
- Consumes: `tidy_meeting_mail` (Task 2); `CalEvent::sequence` from Exchange (Task 1).
- Produces: `event.respond`'s and `invite.apply`'s (`CANCEL`) results gain `"moved": n`.

- [ ] **Step 1: Write the failing tests** (meeting put in the calendar with `Event::DavSynced`, as `weekly_series` and `a_group_invitation_shows_its_answer_and_goes` do):
  - `answered_in_the_calendar_takes_the_inbox_invitations` — calendar holds the meeting at seq 2; Inbox: its invitations seq 1 and seq 2, plus a sample message opened in the reading pane → `event.respond {"uid", "answer":"accepted"}`: `moved == 2`, both now in Trash; `s.reading` still the sample message.
  - `answered_in_the_calendar_while_its_invitation_is_read_goes_on` — the same, with the seq 2 invitation open: the reading pane shows `successor_of` it.
  - `ones_own_meeting_answered_in_the_calendar_moves_nothing` — organizer `riley@studio.example` (the user): `moved == 0`.
  - `an_exchange_colleagues_meeting_is_tidied_by_its_title` — account given `config.ews` as in the group test; calendar meeting "Budget review", `organizer: None`, seq 2, `href` set; Inbox: seq 1 and seq 2 from `pat@contoso.example` under subject "Budget review", seq 3 under the same subject → after `event.respond`, seq 1 and 2 moved, seq 3 stayed.
  - `removing_a_cancelled_meeting_takes_the_cancellation_and_its_invitations` — meeting in the calendar (seq 1, `ANA`); Inbox: REQUEST seq 1, a later REQUEST seq 3 (sent again after the cancellation), and the CANCEL seq 2 opened → `invite.apply`: `removed == true`, `moved == 2` (the cancellation and REQUEST seq 1), seq 3 stayed.
- [ ] **Step 2: Run them to see them fail** — `cargo test -p quatrelle-engine --lib tests::invites::answered` (as in Task 2): `moved` missing.
- [ ] **Step 3: Implement.** `respond_to_event`: after the answer (both the server path and the mail path), when the meeting's organizer is not one of `my_addresses()` (rule 4; none known counts as someone else's), `tidy_meeting_mail(account, uid, &["REQUEST"], None, &calendar_parts, None)` with `calendar_parts` the account's events `same_meeting(uid)` after the answer; add `"moved"`. `apply_invite`, `CANCEL`: after the removal, `tidy_meeting_mail(&key.account, &uid, &["REQUEST", "CANCEL"], only, &cancelled_parts, Some(&key))`, `only` = the occurrences named when the series isn't (`cancelled_only`, as upstream's); add `"moved"`. Replies (`REPLY`) unchanged.
- [ ] **Step 4: Run the tests to see them pass**, then the whole engine lib, clippy and fmt.
- [ ] **Step 5: Commit** — "Invitations: answered in the calendar or cancelled, their mail leaves the Inbox too".

### Task 4: The switch in File › Options

Model: sonnet, effort medium.

**Files:**
- Modify: `crates/ui-egui/src/panes/backstage/options.rs` (Calendar group, ~line 304)
- Modify: every catalog in `crates/ui-egui/src/i18n/*.tsv`

**Interfaces:**
- Consumes: `Prefs::tidy_answered_invites`, command `view.tidyAnsweredInvites` (Task 2).

- [ ] **Step 1: Add the setting** at the top of the Calendar group, like "Put a “-- ” line before the signature": `setting(ui, false, tl!("Move meeting invitations to Deleted Items once answered"), Some(tl!("Also a cancellation once its meeting is removed from the calendar, and the meeting's earlier invitations.")), …)` with `switch(ui, s.prefs.tidy_answered_invites, <same label>, "view.tidyAnsweredInvites", acts)`.
- [ ] **Step 2: Translate the two strings** in all 13 catalogs. French: « Mettre les invitations aux Éléments supprimés une fois répondues » ; « Aussi une annulation, une fois sa réunion retirée du calendrier, et les invitations précédentes de la réunion. »
- [ ] **Step 3: Run** `cargo test -p quatrelle-ui-egui --lib` (the catalog tests and the headless frames of the File page pass), clippy and fmt.
- [ ] **Step 4: Commit** — "Options: the switch for answered invitations".

### After the tasks (controller)

- [ ] `TMPDIR=$HOME/.tq CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=4 cargo xtask ci` green.
- [ ] Screenshot of the File › Options page (`--sample --cmd ui.options`, `QUATRELLE_SCREENSHOT`) showing the switch.
- [ ] Whole-branch review (fable, high); fixes.
- [ ] `FORK.md`: the entry for calendar feature 2 (upstream `484e4c2f`, `25c6997b`, `3c4febce` adapted, what differs); merge into `main`; test build.
