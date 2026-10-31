# The contacts field, delivery 2 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The meeting's Required, Optional and Rooms lines and the Select Attendees and Resources dialog become the contacts field. A room typed among the guests moves to Rooms. A meeting's Send waits for its names as a message's does. Then "From: other address" becomes a one-entry field that no longer asks the directory as one types.

**Architecture:** Delivery 1's glue (`dialogs/people_field.rs`, 850 lines) is first split by topic. It then learns four things the final review of delivery 1 asked for (M2–M4, M8):
- meeting windows get FieldIds of their own;
- the send wait works over any window's fields;
- a field refuses the kinds its place doesn't take (the red chip);
- a one-entry field replaces its entry.

The meeting window (`pim_dialogs/meeting.rs`) draws its lines through that glue. Their data stays where it is today: `EventForm.guests`, `optional` and `rooms`, the last with a text line beside it. `event.create`/`event.update` from a meeting window are gated in a new `app/sending.rs`. The From menu's box becomes a field with `Accepts::ONE_SENDER`, and its directory search as one types (`reading/sender.rs::candidates`) goes.

**Tech Stack:** Rust, egui; no new dependency, no new icon.

**Spec:** `docs/superpowers/specs/2026-10-30-contacts-field-design.md` (approved 2026-10-30), section "Delivery", item 2. This plan builds on delivery 1 (merged as e9d9d96) and on its final review (`contacts-d1-final-review.md` in the session scratchpad: M2, M3, M4, M8 and the two parked residuals). Delivery 3 (Add calendar, Open another mailbox, rules, Search Folders) gets its own plan.

## Decisions this plan makes (the spec doesn't settle them)

1. **Shared mailboxes are accepted in Required and Optional.** The spec's table lists people, lists, contact groups and Microsoft 365 groups. A shared mailbox receives mail and has a calendar, and refusing it would block "Send Update" on meetings that invite one today. *Cost if wrong:* one constant.
2. **Meeting windows' FieldIds are numbered apart:** `MEETING_WINDOWS | wid`, with `MEETING_WINDOWS = 1 << 32`. Event windows count from 1 on a counter of their own (`UiState.event_window_ids`); compose ids are the engine's.
3. **The attendees dialog's list is not filtered by row.** It feeds three rows, and each row refuses what it doesn't take: a person picked into Resources is a red chip "This isn't a room".
4. **The old appointment dialog (`pim_dialogs::event`) keeps its plain Invite line.** A `Dialog::Event` is moved into a window of its own (`app/windows.rs:147`) before dialogs are drawn, so only tests reach that form.
5. **Hide Recipients is held, with the stop's alert, while a field of the message has lost recipients.** This is the first residual parked after delivery 1 (Ruling 15). The stop's alert now says to empty the field, which closes the second residual.
6. **While its names are checked, the meeting's Send is disabled and its window says "Checking names…".**

## Global Constraints

**Code and layering**
- No `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!` outside tests, and no `unsafe`. Errors are `Result` through `?`.
- Layering (`cargo xtask layers`): `look/` never sees the `Session`. The engine stays sans-I/O.

**Sizes** (`cargo xtask sizes`)
- At most 1000 lines per file and 16 `.rs` per folder; split before ~800.
- Over or near the limit:
  - `crates/ui-egui/src/dialogs/people_field.rs` (850): Task 2 splits it before anything is added.
  - `crates/ui-egui/src/lib.rs` (916): nothing is added.
  - `crates/engine/src/tests/writing/people_field.rs` (850): new engine tests go in a new `tests/writing/attendees.rs`.
  - `crates/ui-egui/src/app/actions.rs` (725) and `app/frame.rs` (605): meeting sending goes in a new `app/sending.rs`, and `frame.rs`'s `send_when_resolved` moves there.
- Folders with room: `dialogs/` (12), `app/` (11), `tests/writing/people_field/` (2), `engine/src/tests/writing/` (6).

**Interface text**
- Everything goes through `tl!`/`tf!`, with a row in each of the 13 catalogs `crates/ui-egui/src/i18n/*.tsv` (`every_interface_string_is_translated`).
- New strings, verbatim:
  - "This isn't a room"
  - "{room} was moved to Rooms"
  - "Only people, lists and groups can be invited"
  - "Send from a person, a shared mailbox or a list"
  - "Checking names…"
  - "Some recipients were left out. Empty the field, then add fewer at once or the list itself" (it replaces the old stop's text)

**Commands**
- Commands keep their parameters. `every_command_survives_junk_params` stays green.
- `FIELD_NAMES` gains `"from"`, `"required"`, `"optional"`, `"rooms"`, `"att-required"`, `"att-optional"` and `"att-rooms"`.

**The spec's rules that apply here**
- Typing never asks the server. The directory is asked on Ctrl+K, when the field is left, and at sending.
- A send waits at most `SEND_WAIT_SECS` (60 s), paused while a choice dialog is open.
- A field holds at most 500 entries.
- A contact group is expanded before the command runs.

**Memory, for every cargo command**
- `mkdir -p ~/.tq` first.
- Prefix every command with `TMPDIR=$HOME/.tq CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=4 CARGO_TARGET_DIR=$PWD/target`. The stable clippy uses `$PWD/target-stable`.
- Only one full build at a time: check `pgrep -a cargo` first.
- `T` below is that prefix.

**Gates before every commit**
- `cargo fmt --all`
- `T cargo xtask ci`
- `TMPDIR=$HOME/.tq CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=$PWD/target-stable cargo +stable clippy --workspace --all-targets -- -D warnings`
- Known flake: `a_big_folder_changes_all_at_once_quickly`. Rerun it alone once before concluding anything.

**Commits**
- Small and topical, in English: `Area: what changed`, with a prose body saying why.
- End with `Co-Authored-By: Claude <model name> <noreply@anthropic.com>`.
- Run each fix's test once without the fix, to see it fail.

## Review Focus

1. **The owner's meetings have 20–40 guests, and the windows redraw at 60 fps.** A Required line that re-parses or re-scans its text every frame is the search bug of 2026-10-30 again. Test: `meeting_lines_parse_once_per_change_not_per_frame` (Task 3).
2. **A meeting made before this delivery must still send its update unchanged.** Its guests may include a shared mailbox, an address the session doesn't know, or a room booked by address. Test: `a_meeting_reopened_sends_its_update_as_before` (Task 4).
3. **The Room Finder, the Rooms field and the place must stay in step.** Booking from the finder shows a chip. Removing the chip frees the room. The place follows until it is typed by hand. Test: `the_rooms_field_and_the_room_finder_stay_in_step` (Task 3).
4. **A message and a meeting window open at once with the same number.** Closing one must never forget the other's names. Test: `a_meeting_and_a_message_numbered_alike_keep_their_own_names` (Task 2).
5. **The From field lives inside the From menu.** Clicking a suggestion there must not close the menu before the address is taken. Test: `a_suggestion_clicked_in_from_is_taken` (Task 5).

---

### Task 1: What each place takes, the new field names, and `event.create`'s best-effort refusal — `sonnet`, effort `high`

**Files:**
- Modify:
  - `crates/model/src/addr/entries.rs`: `Accepts` gains `typed` and three constants.
  - `crates/engine/src/session/mailboxes/people/suggest.rs:21`: `FIELD_NAMES`.
  - `crates/engine/src/cmd/people.rs`: the params docs list the new names.
  - `crates/engine/src/cmd/pim.rs:351-380`: the rooms refusal.
- Create: `crates/engine/src/tests/writing/attendees.rs`, declared with `mod attendees;` in `crates/engine/src/tests/writing.rs`.
- Test: `entries.rs` `mod tests`; `tests/writing/attendees.rs`.

**Interfaces:**
- Produces in `model::addr`:
  - `pub struct Accepts { pub kinds: &'static [AddresseeKind], pub one: bool, pub typed: &'static [AddresseeKind] }`. `typed` holds the kinds a place takes only when they were typed as a full address, never when picked by name. `RECIPIENTS` gets `typed: &[]`.
  - `pub fn takes_typed(&self, kind: AddresseeKind) -> bool`, which is `takes(kind) || typed.contains(&kind)`.
  - `pub const ATTENDEES: Accepts`: kinds Person, DistributionList, ContactGroup, Microsoft365Group, SharedMailbox; several; typed empty.
  - `pub const ROOMS: Accepts`: kinds Room, Equipment; several; typed Person, SharedMailbox.
  - `pub const ONE_SENDER: Accepts`: kinds Person, SharedMailbox, DistributionList; `one: true`; typed empty.
- Produces in the engine:
  - `FIELD_NAMES` = the existing eight plus `"from"`, `"required"`, `"optional"`, `"rooms"`, `"att-required"`, `"att-optional"`, `"att-rooms"`.
  - `event.create`/`event.update` refuse a `rooms` entry that the session knows (`Session::addressee_kind`) as a DistributionList, ContactGroup, PublicFolder or Microsoft365Group. The error is `CmdError::Params(format!("not a room: {address}"))`. An address the session doesn't know goes through, as today.

- [ ] **Step 1: Write the failing tests.**
  - In `entries.rs`, `places_take_what_the_spec_says`:
    - `ROOMS.takes(Room)`, `!ROOMS.takes(Person)`, `ROOMS.takes_typed(Person)`, `!ROOMS.takes_typed(DistributionList)`;
    - `ATTENDEES.takes(SharedMailbox)`, `!ATTENDEES.takes(PublicFolder)`;
    - `ONE_SENDER.one`, `!ONE_SENDER.takes(Room)`;
    - every constant takes `Unknown`.
  - In `attendees.rs`:
    - `a_known_list_is_refused_as_a_room_and_an_unknown_address_goes_through`: index a `DistributionList` `team@contoso.example` with `index_directory`. `event.create` with `"rooms": "Team <team@contoso.example>"` errs with `not a room: team@contoso.example`. With `"rooms": "beamer@contoso.example"` it succeeds and books it as `ROOM`.
    - `the_meeting_fields_are_known_by_name`: `FieldId::parse(5, "required")`, `"att-rooms"` and `"from"` are `Some`.
- [ ] **Step 2:** Run `T cargo test -p quatrelle-model --lib addr` and `T cargo test -p quatrelle-engine --lib tests::writing::attendees`. Both fail.
- [ ] **Step 3:** Implement the code above. In `pim.rs`, the check goes right after `parse_list` for the `rooms` key only.
- [ ] **Step 4:** Run both again; they pass. `T cargo xtask wasm` still builds.
- [ ] **Step 5:** Commit: `Contacts: each place says what it takes, and a known list isn't booked as a room`.

---

### Task 2: The glue split, meeting windows apart, any window's send wait, refusals and one-entry fields — `opus`, effort `xhigh`

This refactors a send path that was reviewed once, so the existing tests are its first gate.

**Files:**
- Split `crates/ui-egui/src/dialogs/people_field.rs` into:
  - `dialogs/people_field.rs`: `PeopleFields`, the consts, `egui_id`, `prepare`/`draw`/`choice_dialog`, `settle`, and the new accessors;
  - `dialogs/people_field/field.rs`: `Field` and its `impl`, `pieces_of`/`piece_of`, `spelled_as`, `same_list`, `is_group`, and the new `refused_by`;
  - `dialogs/people_field/send.rs`: `SendWait`, `SendState`, `gate_send`, `send_when_resolved`, `send_wait_left`, `send_state`, `mark_unreached` and the new `gate_meeting`.
- Modify:
  - `crates/ui-egui/src/app/windows.rs:16-34` (`settle_composes` passes the meeting windows);
  - `crates/ui-egui/src/lib.rs:655-665` (`run` holds `compose.hideRecipients`);
  - the 13 catalogs.
- Test: `crates/ui-egui/src/tests/writing/people_field/sending.rs`. Existing tests change only where the stop's text changed.

**Interfaces:**
- Produces in `dialogs/people_field.rs`:
  - `pub(crate) const MEETING_WINDOWS: u64 = 1 << 32;`
  - `pub(crate) fn meeting_field(wid: u64, name: &'static str) -> FieldId`, with `window: MEETING_WINDOWS | wid`;
  - `pub(crate) fn is_meeting_window(window: u64) -> bool`;
  - `pub(crate) const MEETING_FIELDS: [&str; 3] = ["required", "optional", "rooms"]` and `ATTENDEE_FIELDS: [&str; 3] = ["att-required", "att-optional", "att-rooms"]`;
  - `pub(crate) fn accepts_of(name: &str) -> &'static Accepts`. This is the one definition (V5) of what each field name takes:
    - `required`, `optional`, `att-required`, `att-optional` → `ATTENDEES`;
    - `rooms`, `att-rooms` → `ROOMS`;
    - `from` → `ONE_SENDER`;
    - anything else → `RECIPIENTS`.

    Every caller of `prepare`/`draw` passes `accepts_of(name)`.
- New methods on `PeopleFields`:
  - `text(&self, field) -> Option<String>`: `text_of(pieces, input)`.
  - `addressees(&self, field) -> Vec<Addressee>`: the chips with an address and no refusal, in order.
  - `blocked(&self, field) -> bool`: a refused chip, or text with an error.
  - `add(&mut self, field, who: Addressee)`: an entry picked by name, as a book button gives it. It is refused under the by-name rule below.
  - `say(&mut self, field, note: String)`.
  - `take_moved(&mut self, field) -> Vec<Addressee>`: see the room move below.
  - `has_losses(&self, window) -> bool`.
  - `holding(&self, window) -> bool`.
- `settle(&mut self, s, live: &HashSet<u64>, open_books: &HashSet<u64>)`:
  - `live` holds compose ids and meeting windows (tagged);
  - `open_books` holds the window of an open Select Names, and the tagged window of each meeting whose attendees dialog is open;
  - book rows (`BOOK_FIELDS`, `ATTENDEE_FIELDS`) are kept only while their window is in `open_books`.
- Refusals:
  - `fn refused_by(accepts: &Accepts, who: &Addressee, by_name: bool) -> Option<String>` returns `None` when `accepts.takes(kind)`, or when `!by_name && accepts.typed.contains(&kind)`. Otherwise it gives the place's text:
    - `ROOMS` → "This isn't a room";
    - `ATTENDEES` → "Only people, lists and groups can be invited";
    - `ONE_SENDER` → "Send from a person, a shared mailbox or a list".
  - It is applied in `parse` (by_name false), in `add_chip` from `Take` and `add` (true), in `Outcome::Found` for a token without `@` (true), and in `refresh_kinds`.
  - A refused chip keeps its refusal across a reparse while a chip of the same address stays, as a text piece keeps its error today.
- The room move: in an `ATTENDEES` field, a chip of kind Room or Equipment (from parse, Found, Take, `add` or `refresh_kinds`) is taken out of the pieces into `Field.moved` and the field is marked dirty. `take_moved` drains them. Task 3 books them.
- One-entry fields: when `accepts.one`, a new chip or closed text replaces every piece the field had.
- The send wait:
  - `SendWait { params: Value, since: f64, fields: &'static [&'static str] }`, keyed by window;
  - `send_state(s, window, fields)` and `mark_unreached(s, window, fields)` take the fields;
  - `gate_send` keeps its signature, with `COMPOSE_FIELDS`;
  - `send_when_resolved` keeps its signature and syncs from the engine only for compose windows;
  - `pub(crate) fn gate_meeting(&mut self, s: &mut Session, wid: u64, account: &str, lines: [(&'static str, &mut String); 3], now: f64) -> bool`. For each line it:
    1. closes the input and writes the text back into the line at once;
    2. clears `NotFound`/`Unreached`;
    3. calls `people_resolve(field, Some(account), accepts_of(name), Some(&unresolved_text))`;
    4. asks for each contact group's members.

    It returns `true` when the send is held. `send_when_resolved` then yields `(MEETING_WINDOWS | wid, Ok(Value::Null))` for it: the caller rebuilds the command from the form.
- M8: `expansion_now` returning `None` notes "This list couldn't be expanded: {error}" with the error "gone", where it now drops silently. Use the existing string.
- Residuals:
  - `MailApp::run("compose.hideRecipients", …)` sets `compose_alert` to the stop's text and doesn't run while `people.has_losses(compose id)`;
  - the stop's text is the new string in Global Constraints.

- [ ] **Step 1: Write the failing tests** in `sending.rs`:
  - `a_meeting_and_a_message_numbered_alike_keep_their_own_names`:
    1. on an Exchange account (`with_exchange`), compose id `n` has a To name pending;
    2. `prepare` + `people_resolve` on `meeting_field(n, "required")` leaves a name pending too;
    3. `settle` with `live` = {`MEETING_WINDOWS | n`} (the message closed);
    4. `people_pending(meeting_field(n, "required")) == 1`, and the message's fields are gone.
  - `a_refused_kind_is_a_red_chip_that_stays_red`:
    1. prepare a `rooms` field over a meeting window;
    2. `add` a Person (`zoe@studio.example`) → a chip with `refused == Some("This isn't a room")`;
    3. the same address in the text, re-prepared after another entry was added, is still refused;
    4. a Person typed as `zoe@studio.example` in a fresh `rooms` field is accepted.
  - `a_one_entry_field_keeps_the_last_entry`: in `from`, two entries closed one after the other leave one chip, the second.
  - `a_room_in_the_guests_is_moved_out`: in `required`, the text `Sound Lab <sound.lab@studio.example>, zoe@studio.example` with the sample's room set loaded (run `calendar.rooms` for `sample` first: its set comes from the calendar). `take_moved` gives Sound Lab, and `text(field)` holds only `zoe@studio.example`.
  - `hide_recipients_waits_while_recipients_were_left_out`: paste 501 addresses into To, then run `compose.hideRecipients` through `MailApp::run`. `compose_alert` is the new stop text, and Bcc is still empty.
  - `a_list_gone_from_the_engine_says_it_couldnt_be_expanded`: expand a contact group, `forget_expansions` before the answer, prepare. The field's note says "This list couldn't be expanded", and the send stops with that note.
- [ ] **Step 2:** Run `T cargo test -p quatrelle-ui-egui --lib people_field`. The new tests fail and the existing ones pass.
- [ ] **Step 3:** Split the file first, moving code only. Run the existing tests, then commit: `Contacts: the field's glue split by topic`.
- [ ] **Step 4:** Implement the interfaces above.
- [ ] **Step 5:** Run `T cargo test -p quatrelle-ui-egui --lib`. Everything passes; any test changed for the new stop text is named in the report.
- [ ] **Step 6:** Commit: `Contacts: meeting windows apart, any window's send wait, refusals, one-entry fields`.

---

### Task 3: Required, Optional, Rooms and Select Attendees as the contacts field — `opus`, effort `xhigh`

**Files:**
- Modify:
  - `crates/ui-egui/src/dialogs/pim_dialogs/meeting.rs` (`header`, `meeting`'s signature);
  - `pim_dialogs/event.rs` (`EventForm.rooms_line`, `add_room`/`remove_room`/`open`, `rooms_from_field`);
  - `pim_dialogs/attendees.rs` (its rows and `apply`);
  - `pim_dialogs/rooms.rs` (`row` goes; the Room Finder toggle stays);
  - `crates/ui-egui/src/app/windows.rs:146-212` (prepare each meeting's fields before its viewport, the room move, the attendees rows);
  - `crates/ui-egui/src/dialogs/names.rs` (`field_row` goes: nothing uses it any more);
  - the 13 catalogs.
- Create: `crates/ui-egui/src/tests/writing/people_field/meeting.rs`, declared in `tests/writing/people_field.rs`.
- Test: that file. In `crates/ui-egui/src/tests/scheduling.rs`, `required_and_optional_open_the_address_book` changes where it wrote `attendees_book.resources` / `.required` directly. Those stay `String`s (the rows' texts), so the test should need its assertions only.

**Interfaces:**
- Consumes from Task 2: `meeting_field`, `MEETING_FIELDS`, `ATTENDEE_FIELDS`, `accepts_of`, `prepare`/`draw`, `text`, `addressees`, `add`, `say`, `take_moved`, `blocked`, `settle(.., open_books)`.
- Produces in `pim_dialogs`:
  - `pub fn meeting(ui, f: &mut EventForm, s: &Session, people: &mut PeopleFields, t, acts) -> bool`.
  - `EventForm.rooms_line: String`: the Rooms field's text, which is the booked rooms then any entry that isn't one (text not yet checked, refused chips). It is never sent: `params()` still writes `rooms` from `f.rooms`. Set by `open()` from the event's rooms.
  - `add_room`/`remove_room` keep `rooms_line` in step: an entry is appended or removed by address. A room known by its address alone is written bare, as `AttendeesUi::open` writes it.
  - `pub fn rooms_from_field(&mut self, booked: &[Addressee], line: String)`. It sets `rooms_line = line`, then makes `rooms` equal `booked` (name, or the address when there is none) through `remove_room`/`add_room`, so the place follows.
- Drawing:
  - Required and Optional keep `button_row` (the label is the book button, opening `AttendeesUi` as now). Their field is drawn with `book: false`, over `f.guests` / `f.optional`, using `meeting_field(f.window, "required" | "optional")`.
  - Rooms becomes `button_row("Rooms")`, opening `AttendeesUi` on `Line::Resources`. Its field is drawn over `f.rooms_line`, and "Room Finder" is a toggle after the field, which leaves it room. After the draw, when `rooms_line` changed: `f.rooms_from_field(&people.addressees(field), line)`.
  - The choice dialog for `MEETING_FIELDS` is drawn in the meeting's window.
- `app/windows.rs` does this before each meeting's viewport (where `&mut self.session` is at hand):
  - prepares the three fields from the form's texts;
  - drains `take_moved` from Required and Optional into `f.add_room` with `people.say(field, tf!("{room} was moved to Rooms", room = name))`;
  - prepares the attendees rows when the dialog is open;
  - adds the meeting windows to `settle`'s `live`, and the open attendees dialogs to `open_books`.
- The attendees dialog:
  - its rows are `att-*` fields over `st.required` / `st.optional` / `st.resources`, each drawn with `book: true`. The book press adds the person picked with `people.add(row, who)`, not `names::add_to`;
  - its OK is disabled while any row is `blocked`;
  - `apply` takes the rooms from `people.addressees(att-rooms)`. Text left in Resources keeps the dialog open with "Not an address: {what}" (the existing string), as today;
  - a room among the rows' guests is moved as in the window.

- [ ] **Step 1: Write the failing tests** in `meeting.rs`:
  - `meeting_lines_are_contacts_fields`: a meeting window opened with `f.guests = "Zoe Park <zoe@studio.example>, sam@studio.example"`.
    - Its Required shows two chips.
    - Typing `zo` suggests Zoe.
    - Ctrl+K in Required runs `people.resolve` with `text`, never `compose.checkNames`.
  - `meeting_lines_parse_once_per_change_not_per_frame`: 30 guests. `syncs(field)` is 1 after 10 frames, and 2 after one more guest is typed and closed.
  - `a_room_typed_among_the_guests_moves_to_rooms`: typing `Sound Lab <sound.lab@studio.example>;` in Required leaves the guests without it. `f.rooms` gains it, the place follows, and the note "Sound Lab was moved to Rooms" shows.
  - `the_rooms_field_takes_rooms_and_addresses_and_refuses_the_rest`. In Rooms:
    - `beamer@studio.example` typed → booked;
    - `zoe@studio.example` typed → booked (a mailbox typed as an address);
    - `Zoe` with Ctrl+K resolving to the local Zoe → a red chip "This isn't a room", not booked;
    - a known distribution list typed → red.
  - `the_rooms_field_and_the_room_finder_stay_in_step`:
    1. book Sound Lab from the finder (double-click its row) → its chip shows in Rooms;
    2. remove the chip (its card's Remove) → `f.rooms` is empty, and the place follows;
    3. type a place by hand, then book again → the place stays.
  - `the_attendees_dialog_rows_are_contacts_fields`:
    1. open from Required;
    2. a person picked with "Resources ->" is a red chip, and OK is disabled;
    3. remove it and pick a room → OK books it;
    4. cancel leaves the meeting as it was.
- [ ] **Step 2:** Run `T cargo test -p quatrelle-ui-egui --lib people_field::meeting`. The tests fail.
- [ ] **Step 3:** Implement the code above.
- [ ] **Step 4:** Run `T cargo test -p quatrelle-ui-egui --lib`. Everything passes, including `tests/scheduling.rs` (the finder, the assistant, the address book) and `tests/meetings.rs`. Name any assertion changed in the report.
- [ ] **Step 5:** Take a screenshot of the meeting window, light and dark (`--sample --data <scratchpad folder>`, `QUATRELLE_SCREENSHOT`), and read it. Check that the chips sit on the form's rows, that the Room Finder toggle is visible, and that nothing overlaps.
- [ ] **Step 6:** Commit: `Meetings: Required, Optional, Rooms and Select Attendees are the contacts field`.

---

### Task 4: A meeting's Send waits for its names — `opus`, effort `high`

**Files:**
- Create: `crates/ui-egui/src/app/sending.rs`, declared in `app.rs` or wherever `frame.rs` is declared. `MailApp::send_when_resolved` moves there from `frame.rs:77-96`.
- Modify:
  - `crates/ui-egui/src/app/actions.rs:326-338`: the `ForEvent` arm. It gates, and its body becomes `fn event_command(&mut self, w: u64, id: &'static str, p: Value)` in `sending.rs`.
  - `pim_dialogs/meeting.rs`: Send is disabled and "Checking names…" shows while `people.holding(MEETING_WINDOWS | f.window)` (`holding` takes a FieldId's window, tagged for a meeting).
  - the 13 catalogs.
- Create: `crates/ui-egui/src/tests/writing/people_field/meeting_send.rs`.

**Interfaces:**
- Consumes `gate_meeting`, `send_when_resolved`, `text`, `addressees`, `is_meeting_window` and `holding` from Task 2, and `rooms_from_field` from Task 3.
- `ForEvent(w, "event.create" | "event.update", p)` from a meeting window whose form `can_invite`:
  - calls `gate_meeting(&mut session, w, &f.account_id(), [("required", &mut f.guests), ("optional", &mut f.optional), ("rooms", &mut f.rooms_line)], now)`, then `f.rooms_from_field(..)`;
  - if held, returns; otherwise `event_command(w, id, f.params())`, with the params rebuilt after the gate.
  - Other commands (`event.delete`, `event.respond`, `event.move`) never wait.
- `send_when_resolved`, for a meeting window:
  - `Ok`: set the three texts from `people.text`, call `rooms_from_field`, then `event_command(w, if f.uid.is_some() {"event.update"} else {"event.create"}, f.params())`;
  - `Err(why)`: `f.error = Some(why)`;
  - a window closed meanwhile drops the wait (`settle` already does).

- [ ] **Step 1: Write the failing tests** in `meeting_send.rs`. Use `with_exchange` and `directory_answers` from `tests/writing.rs`.
  - `a_meeting_waits_for_a_name_then_is_sent`: Required `Dupont` on an account with a directory.
    1. Send → nothing is created yet, "Checking names…" shows, and Send is disabled.
    2. The directory answers one person → the event exists with `anne.dupont@contoso.example` among its attendees.
  - `a_meeting_stops_on_a_name_not_found_and_says_why_in_its_window`: the directory answers no one → no event, and `f.error == "Not found: Dupont"`.
  - `an_ambiguous_name_asks_in_the_meeting_window_then_sends`: two people answer. The choice dialog shows in the meeting's viewport; picking one sends with them.
  - `a_contact_group_is_expanded_before_the_meeting_is_sent`: Required holds `Team <group:AAMk=>`. The `ExpandList` answer arrives with two members, and the attendees are those two.
  - `a_refused_room_stops_the_send`: Rooms holds a red chip → `f.error == "This isn't a room"`, and no event.
  - `a_meeting_reopened_sends_its_update_as_before`: an existing sample meeting whose attendees include a shared mailbox, an unknown address and a room booked by address. "Send Update" with nothing changed runs `event.update` at once (no wait), with the same attendees and rooms.
  - `addresses_alone_send_a_meeting_offline`: no account has a directory, and Required holds two addresses → it is created at once.
- [ ] **Step 2:** Run `T cargo test -p quatrelle-ui-egui --lib meeting_send`. The tests fail.
- [ ] **Step 3:** Implement the code above.
- [ ] **Step 4:** Run `T cargo test -p quatrelle-ui-egui --lib`. Everything passes.
- [ ] **Step 5:** Commit: `Meetings: Send waits for the names its lines are still checking`.

---

### Task 5: "From: other address" as a one-entry field — `opus`, effort `high`

**Files:**
- Modify:
  - `crates/ui-egui/src/reading/sender.rs`: `Candidates` and `candidates` go, and `other_address` draws the field;
  - `crates/ui-egui/src/reading/compose.rs:184-192` (prepare the `from` field where `candidates` was called) and `:330-335` (the Ctrl+K guard drops `box_id`; `people.open_field()` covers the box);
  - the 13 catalogs.
- Test: `crates/ui-egui/src/tests/writing.rs`:
  - `from_finds_another_address_by_its_name` is rewritten: it asserted the search as one types, which the spec removes. Keep its name and say so in its doc comment.
  - `from_offers_shared_mailboxes_and_another_address` must pass unchanged.
  - New tests go in `tests/writing/people_field/message.rs` (666 lines; split it by topic first if a test would push it past ~800).

**Interfaces:**
- The field is `FieldId { window: compose id, name: "from" }`, with `accepts_of("from")` (`ONE_SENDER`). Its text is in the egui temp data, as the box's typed text is today (`state_id(compose)`), read for `prepare` in `compose()` and written by the draw.
- It is drawn under "Other email address", with `book: false` and `can_expand: false`, the hint "Name or address", and `focus` set when the section was just opened.
- "Send from it" is enabled when `people.addressees(field)` holds exactly one entry and the field isn't `blocked`. Enter with the list closed does the same.
  - Taking it sets `c.from` as today: empty for the account's own address, else `who.text()`.
  - Then the section closes and the field's text is emptied.
- No directory search as one types: `search_directory` is never called from here. Ctrl+K and leaving the field go through `people.resolve` with `text`, as for any field.
- The menu stays open while the field's suggestion list or card is open. Pick its `close_behavior` accordingly, or keep the list inside the menu's rect.

- [ ] **Step 1: Write the failing tests:**
  - `from_finds_another_address_by_its_name` (rewritten):
    1. typing `proginov` asks nothing (`!directory_searching`);
    2. Ctrl+K asks the directory;
    3. one answer → a chip;
    4. "Send from it" → `c.from == "Proginov Reseau <reseaux@proginov.example>"`.
  - `a_suggestion_clicked_in_from_is_taken`: a local contact typed in part, its suggestion clicked → the chip is there and the menu is still open; "Send from it" takes it.
  - `from_holds_one_entry_and_refuses_a_room`:
    - a second address typed replaces the first;
    - `sound.lab@studio.example`, a room of the sample's set, is a red chip "Send from a person, a shared mailbox or a list", and "Send from it" is disabled.
  - `ctrl_k_in_from_checks_that_field_only`: Ctrl+K in the From field never runs `compose.checkNames`.
- [ ] **Step 2:** Run `T cargo test -p quatrelle-ui-egui --lib from_`. The tests fail.
- [ ] **Step 3:** Implement the code above.
- [ ] **Step 4:** Run `T cargo test -p quatrelle-ui-egui --lib`. Everything passes. Take a screenshot of the From menu, opened, with a chip (light), and read it.
- [ ] **Step 5:** Commit: `Message: From's other address is a one-entry contacts field, checked on Ctrl+K`.

---

## After the tasks

- A final whole-branch review on `fable`, effort `xhigh`. The diff is from `main` at e9d9d96 to the branch head, with the spec and this plan.
- One fix wave, then the gates.
- Merge into `main`, then publish a test build (Linux + Web).
- Update the carnet item `champ-contacts` (build, status) and close its two residuals.
