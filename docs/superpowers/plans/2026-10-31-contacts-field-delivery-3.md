# The contacts field, delivery 3 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The last four places get the contacts field:
- Add calendar takes several entries and opens one calendar per entry. This is the owner's first request: Ctrl+K was missing there.
- Open another mailbox takes one entry.
- A rule's From/To conditions and the "Mail from specific people" Search Folder hand on addresses alone, keep names as typed, and reopen saved entries as chips.

**Architecture:**
- The modal dialogs of the main window get FieldIds of their own: `DIALOG_WINDOWS | n`.
- The glue learns two things:
  - fields whose unresolved text is valid as typed (`Accepts::as_typed`: rules, Search Folders);
  - a dialog's OK held for its names (`gate_dialog`), as a meeting's Send is.
- A new `dialogs/people_field/places.rs` says which windows the open dialog has, and makes its fields ready each frame.
- Add calendar becomes the address book with a "Calendar ->" row, as Outlook's Add Calendar › From Address Book is. It runs `calendar.openShared` once per entry itself.
- Rules and Search Folders lose their own `,`/`;` splitting. They read the field's text through one model helper (`addr::matched`).

**Tech Stack:** Rust, egui; no new dependency, no new icon.

**Spec:** `docs/superpowers/specs/2026-10-30-contacts-field-design.md` (approved 2026-10-30), section "Delivery", item 3; the table "What each place accepts" and its paragraphs "Rules and Search Folders", "Add calendar" and "The commands". This plan builds on deliveries 1 (e9d9d96) and 2 (c0d86aa2).

## Decisions this plan makes (the spec doesn't settle them)

1. **The main window's dialogs number their fields apart:** `DIALOG_WINDOWS | n`, with `DIALOG_WINDOWS = 1 << 51`.
   - `n = 0` for Add calendar, Open another mailbox and the Search Folder. Only one modal dialog shows at a time.
   - `n = i` for a rule's condition `i`.
   - Compose ids are about 2^41 and meeting windows are `1 << 52 | wid`, so neither carries bit 51.
2. **In rules and Search Folders, a name is checked only when asked:** Ctrl+K, a suggestion taken, or a book pick. Leaving the field or typing `;` keeps the name as typed.
   - Why: a name there is a pattern ("Dupont" matches every Dupont). Turning it into one person's address unasked would narrow the rule silently.
   - An address typed there is still a chip.
   - *Cost if wrong:* the field resolves names one step later than elsewhere.
3. **In rules, a full address that nothing knows is a chip of unknown kind** (the plain address icon), as text fed in is elsewhere. The spec's "else it stays text" is read as being about pieces that aren't addresses (`@proginov.com`, `Dupont`).
   - *Cost if wrong:* an icon. The rule matches the same.
4. **Add calendar is the address book with a "Calendar ->" row:** the book's search and list sit above the field (`names::search`).
   - Open another mailbox, rules and Search Folders have no book button: a modal dialog can't open another one over itself. Suggestions and Ctrl+K reach the same people.
   - *Cost if wrong:* embedding the book in one more dialog later.
5. **A calendar the command fails stays in the field as red text with the command's words** (`PieceError::Failed`), not as a red chip. A chip's refusal is worked out again from its kind each time the book changes, so the failure would vanish.
   - *Cost if wrong:* it reads as text, not a chip.
6. **Add calendar's OK opens what it can and leaves the rest:**
   - it waits only for names still on the directory (a lenient wait);
   - entries the field refuses or the command fails stay red;
   - the dialog closes when the field is empty, every entry opened.
   - Open another mailbox waits as a meeting's Send does: a refused entry stops it, and why shows in the dialog.
7. **Add calendar's book isn't filtered by kind**, as Select Attendees' isn't (delivery 2). A list picked there becomes a red chip.
8. **"From contains" and "To or Cc contains" are one-entry fields**, per the spec's table. A saved text with a comma or `;` is quoted, so it stays one entry: `"Doe, John"`.
9. **A Search Folder saved as people reopens as "Mail from specific people"** when its search is `from:…` alone. Today it opens as Custom.

## Global Constraints

**Code and layering**
- No `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!` outside tests, and no `unsafe`. Errors are `Result` through `?`.
- Layering (`cargo xtask layers`): `look/` never sees the `Session`. The engine stays sans-I/O. Portable crates never read the clock or temp files.

**Sizes** (`cargo xtask sizes`)
- At most 1000 lines per file and 16 `.rs` per folder; split before ~800.
- Near the limit, add nothing beyond what's named:
  - `crates/ui-egui/src/lib.rs` (921): one `Action` variant only (3 lines).
  - `crates/model/src/search.rs` (729): `from_people` and its tests (≈ 40 lines).
  - `app/actions.rs` (713) and `dialogs.rs` (707): one match arm, and the variant's change.
- Folders with room:
  - `dialogs/` has 12 files: Task 3 adds `calendars.rs`;
  - `dialogs/people_field/` has 2: Task 3 adds `places.rs`;
  - `tests/writing/people_field/` has 4: Task 2 adds `places.rs`;
  - `engine/src/tests/writing/` has 7: Task 1 adds `places.rs`.

**Interface text**
- Everything goes through `tl!`/`tf!`, with a row in each of the 13 catalogs `crates/ui-egui/src/i18n/*.tsv` (`every_interface_string_is_translated`).
- New strings, verbatim, each with its French row:

| English | French |
|---|---|
| "Calendar ->" | "Calendrier ->" |
| "A distribution list's calendar can't be opened" | "Le calendrier d'une liste de distribution ne peut pas être ouvert" |
| "Only a person's, a room's or a shared mailbox can be opened" | "Seule une boîte de personne, de salle ou partagée peut être ouverte" |
| "A contact group has no address: expand it" | "Un groupe de contacts n'a pas d'adresse : développez-le" |
| "Matched as typed" | "Recherché tel quel" |

- Reused as they are: "Checking names…", "Open a shared calendar", "Open", "Text or address", "From these people (addresses or names, separated by commas)".
- The commands' errors stay English and untranslated, as today.

**Commands**
- Commands keep their parameters. `every_command_survives_junk_params` stays green.
- `FIELD_NAMES` gains `"calendar"`, `"mailbox"`, `"rule-one"`, `"rule-any"`, `"search-people"`.

**The spec's rules that apply here**
- Typing never asks the server.
- A field holds at most 500 entries.
- A wait gives up after `SEND_WAIT_SECS` (60 s), paused while a choice dialog is open.
- Saved rules keep their stored form (`Condition::From(String)`, `FromAny(Vec<String>)`…, unchanged).

**Memory, for every cargo command**
- `mkdir -p ~/.tq` first.
- Prefix every command with `TMPDIR=$HOME/.tq CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=4 CARGO_TARGET_DIR=$PWD/target` (`T` below). The stable clippy uses `$PWD/target-stable`.
- Only one full build at a time: check `pgrep -a cargo` first.

**Gates before every commit**
- `cargo fmt --all`
- `T cargo xtask ci`
- `TMPDIR=$HOME/.tq CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=$PWD/target-stable cargo +stable clippy --workspace --all-targets -- -D warnings`
- Known flake: `a_big_folder_changes_all_at_once_quickly`. Rerun it alone once before concluding anything.

**Commits**
- Small and topical, in English: `Area: what changed`, with a prose body saying why.
- End with `Co-Authored-By: Claude <model name> <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_019dhnBJAQnyMdiQr8krNEX4`.
- Run each fix's test once without the fix, to see it fail.

## Review Focus

1. **A saved rule whose text has a comma must save back unchanged.** Example: "From contains: Doe, John". It reopens as one entry, not cut at its comma. Test: `a_saved_rule_with_a_comma_saves_unchanged` (Task 4).
2. **A name typed in a rule stays a pattern.** "Dupont" followed by `;`, or by leaving the field, never becomes one person's address unasked. Ctrl+K does make it a chip. Test: `a_name_in_a_rule_field_stays_as_typed_until_ctrl_k` (Task 2).
3. **Add calendar's second OK must not open an entry twice.** After a first OK that opened two entries and failed one, a second OK opens nothing again and gives no "already open" noise. Test: `add_calendar_opens_each_entry_once_and_keeps_the_one_refused` (Task 3).
4. **Removing a rule condition in the middle must not mix fields.** The conditions after it show their own entries, not their neighbour's. Test: `removing_a_rule_condition_keeps_each_field_its_own` (Task 4).
5. **Open another mailbox with a name typed must wait for it.** Enter with a name, not an address, waits for the directory. It never sends the name as an address, and never closes with nothing opened. Test: `open_mailbox_waits_for_a_typed_name` (Task 3).

---

### Task 1: What the new places take, the rule text helpers, and the commands' best-effort refusals — `sonnet`, effort `high`

**Files:**
- Modify:
  - `crates/model/src/addr/entries.rs`: `Accepts.as_typed`, four constants, `matched` and `matched_text`;
  - `crates/model/src/search.rs`: `from_people`;
  - `crates/engine/src/session/mailboxes/people/suggest.rs:21`: `FIELD_NAMES`;
  - `crates/engine/src/cmd/people.rs`: the params docs list the new names;
  - `crates/engine/src/session/pim.rs:482`: `open_shared_calendar`;
  - `crates/engine/src/session/mailboxes/shared_mailboxes.rs:28`: `open_shared_mailbox`.
- Create: `crates/engine/src/tests/writing/places.rs`, declared with `mod places;` in `crates/engine/src/tests/writing.rs`.
- Test: `entries.rs` `mod tests`, `search.rs` tests, `tests/writing/places.rs`.

**Interfaces:**
- Produces in `model::addr`:
  - `Accepts` gains `pub as_typed: bool`. It means: text that nothing matches is valid as typed (no error on it, nothing resolved unless asked). Every existing constant gets `as_typed: false`.
  - `pub const CALENDARS: Accepts`: Person, Room, Equipment, SharedMailbox; several; `typed: &[]`; `as_typed: false`.
  - `pub const MAILBOX: Accepts`: Person, SharedMailbox, Room; `one: true`; `typed: &[]`; `as_typed: false`.
  - `pub const MATCHED: Accepts`: Person, DistributionList, Room, Equipment, SharedMailbox, PublicFolder, Microsoft365Group (everything with an address); several; `typed: &[]`; `as_typed: true`.
  - `pub const MATCHED_ONE: Accepts`: the same kinds, `one: true`, `as_typed: true`.
  - `pub fn matched(text: &str) -> Vec<String>`, a field's text as a rule or Search Folder takes it:
    - the entries of `split_entries`;
    - an entry that `parse_one` reads as an address gives that address alone;
    - any other entry gives itself, with one pair of surrounding `"` removed.
  - `pub fn matched_text(pieces: &[String]) -> String`, the reverse:
    - the pieces joined by `"; "`;
    - a piece holding `,` or `;` is written `"…"`, with its own `"` removed.
- Produces in `model::search`: `pub fn from_people(query: &str) -> Option<Vec<String>>`.
  - It gives the people of a query that is only `from:a,"b c",…` (as "Mail from specific people" writes it): split at commas outside quotes, each trimmed and unquoted, empties dropped.
  - It gives `None` when the query has anything else, or no one.
- Produces in the engine:
  - `FIELD_NAMES` = the existing fifteen, plus `"calendar"`, `"mailbox"`, `"rule-one"`, `"rule-any"`, `"search-people"`.
  - `open_shared_calendar` refuses an address the session knows (`Session::addressee_kind(Some(&account), …)`) as a kind `Accepts::CALENDARS` doesn't take: `CmdError::Params(format!("a distribution list's calendar can't be opened: {email}"))`. The check comes after the address check and before the own-calendar check.
  - `open_shared_mailbox` refuses one `Accepts::MAILBOX` doesn't take: `CmdError::Params(format!("not a mailbox to open: {email}"))`.
  - An address the session doesn't know goes through, as today.

- [ ] **Step 1: Write the failing tests**

In `entries.rs` `mod tests`:

```rust
#[test]
fn the_new_places_take_what_the_spec_says() {
    use AddresseeKind::*;
    assert!(Accepts::CALENDARS.takes(Room) && Accepts::CALENDARS.takes(SharedMailbox) && Accepts::CALENDARS.takes(Equipment));
    assert!(!Accepts::CALENDARS.takes(DistributionList) && !Accepts::CALENDARS.takes(ContactGroup) && !Accepts::CALENDARS.takes(PublicFolder));
    assert!(Accepts::MAILBOX.one && Accepts::MAILBOX.takes(Room) && !Accepts::MAILBOX.takes(DistributionList) && !Accepts::MAILBOX.takes(Microsoft365Group));
    assert!(Accepts::MATCHED.as_typed && Accepts::MATCHED_ONE.as_typed && Accepts::MATCHED_ONE.one && !Accepts::MATCHED.one);
    assert!(Accepts::MATCHED.takes(PublicFolder) && Accepts::MATCHED.takes(DistributionList) && !Accepts::MATCHED.takes(ContactGroup));
    assert!(!Accepts::RECIPIENTS.as_typed && !Accepts::ROOMS.as_typed);
}

#[test]
fn a_rule_takes_addresses_alone_and_text_as_typed() {
    assert_eq!(matched("Doe, John <j@x.fr>; @proginov.com"), vec!["j@x.fr", "@proginov.com"]);
    assert_eq!(matched("\"Doe, John\"; Dupont"), vec!["Doe, John", "Dupont"]);
    assert_eq!(matched(" ; "), Vec::<String>::new());
    let v: Vec<String> = vec!["Doe, John".into(), "@proginov.com".into(), "j@x.fr".into(), "a;b".into()];
    assert_eq!(matched_text(&v), "\"Doe, John\"; @proginov.com; j@x.fr; \"a;b\"");
    assert_eq!(matched(&matched_text(&v)), v);
}
```

In `search.rs` tests:

```rust
#[test]
fn a_people_search_gives_its_people_back() {
    assert_eq!(from_people("from:zoe@studio.example,\"Ana Lima\",x"), Some(vec!["zoe@studio.example".to_string(), "Ana Lima".into(), "x".into()]));
    assert_eq!(from_people("FROM:ana"), Some(vec!["ana".to_string()]));
    assert_eq!(from_people("from:ana subject:invoice"), None);
    assert_eq!(from_people("is:unread"), None);
    assert_eq!(from_people("from:"), None);
}
```

In `engine/src/tests/writing/places.rs`, use an Exchange account as `tests/writing/attendees.rs` does, and a directory answer whose people carry their kinds (`Event::DirectoryPeople` with `Addressee::new(name, address, AddresseeKind::DistributionList)`):
- `a_known_list_is_refused_by_open_calendar`: `calendar.openShared` with the list's address gives `Err(CmdError::Params(m))` with `m == "a distribution list's calendar can't be opened: team@contoso.example"`. An unknown `ana@contoso.example` is `Ok`.
- `a_known_list_is_refused_by_open_mailbox`: `account.openMailbox` gives `"not a mailbox to open: team@contoso.example"`. A known room (`Event::Rooms`) and an unknown address are `Ok`.
- `the_new_fields_are_known`: `FieldId::parse(1, n).is_some()` for each of the five new names.

- [ ] **Step 2: Run them to see them fail**

Run: `mkdir -p ~/.tq && T cargo test -p quatrelle-model addr:: search:: && T cargo test -p quatrelle-engine tests::writing::places`
Expected: compile errors (no `as_typed`, `CALENDARS`, `matched`, `from_people`), then the refusals failing.

- [ ] **Step 3: Implement the model's constants and helpers, `from_people`, `FIELD_NAMES` and the two refusals**

`from_people` reuses `split_tokens` and the comma split of `list_term` (factor the split out of `list_term` into a private `split_list(value: &str) -> Vec<String>` both call).

- [ ] **Step 4: Run the tests and the gates**

Run: the Step 2 command, then the three gates. Expected: PASS, ci exit 0, clippy exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/model crates/engine
git commit -m "Contacts field: what Add calendar, Open another mailbox, rules and Search Folders take"
```

---

### Task 2: The glue for dialogs' fields — kept as typed, numbered apart, OK held for names — `opus`, effort `high`

**Files:**
- Modify:
  - `crates/ui-egui/src/dialogs/people_field.rs`: `DIALOG_WINDOWS`, `dialog_field`, `is_dialog_window`, `accepts_of`, `draw`'s ask, and the new methods (below);
  - `crates/ui-egui/src/dialogs/people_field/field.rs`: `refused_by` words, `as_typed` in `parse` and `refresh`;
  - `crates/ui-egui/src/dialogs/people_field/send.rs`: `gate_dialog`, the lenient wait, and dialog windows in `send_when_resolved`;
  - `crates/ui-egui/src/look/people_field.rs` and `look/people_field/chips.rs`: `PieceError::Failed`, `error_text`, the "Matched as typed" hover.
- Create: `crates/ui-egui/src/tests/writing/people_field/places.rs`, declared with `mod places;` in `tests/writing/people_field.rs`.
- Test: `tests/writing/people_field/places.rs`, and one widget test in `tests/writing/people_field.rs`.

**Interfaces:**
- Consumes: Task 1's `Accepts::{CALENDARS, MAILBOX, MATCHED, MATCHED_ONE}`, `as_typed` and `FIELD_NAMES`.
- Produces in `dialogs/people_field.rs`:
  - `pub(crate) const DIALOG_WINDOWS: u64 = 1 << 51;`
  - `pub(crate) fn dialog_field(n: u64, name: &'static str) -> FieldId`, which is `FieldId { window: DIALOG_WINDOWS | n, name }`.
  - `pub(crate) fn is_dialog_window(window: u64) -> bool`: bit 51 set, bit 52 clear.
  - `pub(crate) const DIALOG_FIELDS: [&str; 2] = ["calendar", "mailbox"];` (the fields whose OK can be held).
  - `accepts_of`: `"calendar"` → `CALENDARS`; `"mailbox"` → `MAILBOX`; `"rule-one"` → `MATCHED_ONE`; `"rule-any" | "search-people"` → `MATCHED`.
  - `PeopleFields::opened(&mut self, field: FieldId, address: &str)` takes the chip of that address out (an entry whose calendar opened). The text is written at the next draw.
  - `PeopleFields::failed(&mut self, field: FieldId, address: &str, why: String)` turns that chip into `Piece::Text { text: who.text(), error: Some(PieceError::Failed(why)) }`, written at the next draw.
  - `PeopleFields::reset_window(&mut self, window: u64)` forgets this side's fields of the window, so each is made again from its text at the next prepare.
  - `PeopleFields::gate_dialog(&mut self, s: &mut Session, name: &'static str, account: Option<&str>, text: &mut String, lenient: bool, now: f64) -> bool` holds the OK of the dialog field `dialog_field(0, name)`, `name` one of `DIALOG_FIELDS`:
    - the input is closed and written into `text`, and the names left are asked (handed in), as `gate_meeting` does per line;
    - `false`: it can go now; `true`: held;
    - `send_when_resolved` then gives `(DIALOG_WINDOWS, Ok(Value::Null))`, or `Err(why)`;
    - `lenient` waits only on what is pending (names on the directory, a choice, a group's members) and never stops on an entry;
    - strict stops as a meeting's Send does.
  - `PeopleFields::holding(DIALOG_WINDOWS)` works as is.
- Produces in `look/people_field.rs`: `PieceError::Failed(String)`. `error_text` gives the string itself. `to_check` is false for it: it isn't asked again.

Rules for the glue to implement:
- **`as_typed` fields:**
  - `parse` puts no `NotAnAddress` on their text;
  - `refresh` turns `NotFound`, `NotAnAddress` and `Unreached` into nothing (the text stays, error `None`);
  - `draw` asks the engine (`people.resolve`) only on `PeopleEvent::Resolve` (Ctrl+K), never on `Closed` or on leaving the field.
- **`refused_by`'s words:**
  - `CALENDARS`: "A distribution list's calendar can't be opened";
  - `MAILBOX`: "Only a person's, a room's or a shared mailbox can be opened";
  - `MATCHED`/`MATCHED_ONE`: "A contact group has no address: expand it".
- **The widget:** a `Piece::Text` with no error in a field whose `accepts.as_typed` is set shows "Matched as typed" on hover. It is drawn with no red underline.
- **Shared code:** `gate_meeting`'s per-line body becomes a private helper that `gate_dialog` shares. Meetings behave exactly as before.
- **`send_when_resolved`:** a dialog window, like a meeting's, isn't looked up among the composes. Its field is made ready by its dialog each frame.

- [ ] **Step 1: Write the failing tests** in `tests/writing/people_field/places.rs`

Use `with_exchange` and the directory helpers of `tests/writing.rs`, and the `type_in`/`frames` helpers of `meeting.rs` (make them `pub(super)` where they aren't). Draw a dialog field through a small test harness: `prepare` plus `draw` inside `ctx.run_ui`, as `tests/writing/people_field/sending.rs` draws compose fields.

- `a_name_in_a_rule_field_stays_as_typed_until_ctrl_k`:
  - in `dialog_field(0, "rule-any")`, the book knows one "Jean Dupont";
  - type `Dupont; @proginov.com; ` and leave the field;
  - `pieces` = two `Piece::Text` with `error: None`, and no `people.resolve` action was pushed;
  - Ctrl+K: "Dupont" becomes the chip of Jean Dupont, and `@proginov.com` stays text with no error.
- `a_rule_field_keeps_a_name_nothing_matches_without_error`:
  - Ctrl+K on `Zz` with the directory answering no one;
  - the piece stays `Piece::Text { error: None, .. }`, and `blocked(field)` is false.
- `a_contact_group_is_refused_where_addresses_are_matched`:
  - `add(dialog_field(0, "search-people"), group)` gives a refused chip with "A contact group has no address: expand it";
  - `blocked` is true.
- `a_list_is_refused_where_calendars_open`: a `DistributionList` chip added to `"calendar"` is refused with "A distribution list's calendar can't be opened"; added to `"mailbox"`, it is refused with "Only a person's, a room's or a shared mailbox can be opened".
- `a_dialog_ok_waits_for_its_names`:
  - `gate_dialog(s, "mailbox", Some(&gal), &mut text, false, now)` with `text = "Ana"` and the directory not yet answering gives `true`, and `holding(DIALOG_WINDOWS)`;
  - once `directory_answers(.., "Ana", &[("Ana Lima", "ana@contoso.example")])` and a prepare, `send_when_resolved` gives `[(DIALOG_WINDOWS, Ok(Value::Null))]`;
  - the field's `addressees` are `[ana@contoso.example]`.
- `a_lenient_ok_goes_with_a_refused_entry_and_a_strict_one_stops`:
  - field `"calendar"` holding `ana@contoso.example; team@contoso.example` (team a known list);
  - `gate_dialog(.., lenient: true, ..)` gives `false` (nothing pending);
  - field `"mailbox"` holding team, strict, gives a stop whose `Err` is the refusal's words.
- `an_entry_that_failed_stays_red_with_why_and_one_opened_goes`:
  - `opened(f, "ana@contoso.example")` and `failed(f, "bo@contoso.example", "no access".into())`;
  - after a draw, the text holds no ana, and bo's piece is `Text { text: "bo@contoso.example", error: Some(PieceError::Failed("no access")) }` (as `who.text()` writes a chip with no name);
  - the failure is still there after the book's revision moves (`contacts_revision`) and two frames.
- `a_dialog_field_is_numbered_apart_and_goes_with_its_dialog`:
  - `dialog_field(0, "calendar")` and a compose field of the same message are both prepared;
  - `settle` with a `live` set lacking `DIALOG_WINDOWS` empties the dialog field's pieces and keeps the message's;
  - `is_dialog_window(DIALOG_WINDOWS)` is true; `is_dialog_window(MEETING_WINDOWS | 1)` is false; `is_dialog_window(1 << 41)` is false.

In `tests/writing/people_field.rs`, `matched_as_typed_text_says_so_on_hover`:
- draw the widget alone with `accepts: &Accepts::MATCHED` and a `Piece::Text { text: "@proginov.com", error: None }`;
- hover it: the hover text "Matched as typed" is in the output;
- with `Accepts::RECIPIENTS`, it isn't.

- [ ] **Step 2: Run them to see them fail**

Run: `T cargo test -p quatrelle-ui-egui people_field::places matched_as_typed`
Expected: compile errors, then failures.

- [ ] **Step 3: Implement the glue and the widget's hover** as the Interfaces and rules above say.

- [ ] **Step 4: Run the whole `people_field` tests** (delivery 1 and 2's must stay green), then the gates.

Run: `T cargo test -p quatrelle-ui-egui people_field` then the three gates. Expected: PASS, exit 0.

- [ ] **Step 5: Commit**

```bash
git commit -m "Contacts field: dialogs' fields numbered apart, names kept as typed where rules match them, a dialog's OK held for its names"
```

---

### Task 3: Add calendar (several entries, its book) and Open another mailbox — `opus`, effort `medium`

**Files:**
- Create:
  - `crates/ui-egui/src/dialogs/calendars.rs`: `CalendarsForm` and its drawing, moved out of `other.rs`;
  - `crates/ui-egui/src/dialogs/people_field/places.rs`: the dialogs' windows and fields made ready.
- Modify:
  - `crates/ui-egui/src/dialogs.rs`: the variant `OpenSharedCalendar(Box<CalendarsForm>)`, `Dialog::open_calendars()`, `set_error`, and `body`, which passes `people` to `accounts::draw` and `calendars::draw`;
  - `dialogs/other.rs:141-165`: the old box goes;
  - `dialogs/accounts.rs:464-494`: the mailbox field;
  - `lib.rs`: `Action::DialogOk`;
  - `app/actions.rs`: its arm;
  - `app/sending.rs`: `dialog_ok` and `dialog_resolved`, and the dialog window in `send_when_resolved`;
  - `app/windows.rs:24-34`: the dialog's windows in `settle`;
  - `app/frame.rs:488-496`: `places::prepare` beside Select Names';
  - `app/ribbon.rs:105` and `panes/calendar.rs:145`: `Dialog::open_calendars()`;
  - `tests.rs:253`, the gallery's dialog;
  - `tests/reading.rs:414-424`.
- Test: `tests/writing/people_field/places.rs`, and `tests/reading.rs::enter_submits_a_one_field_dialog`.

**Interfaces:**
- Consumes: Task 2's `dialog_field`, `DIALOG_WINDOWS`, `gate_dialog`, `opened`, `failed`, `addressees`, `holding`, `blocked`, `choice_dialog`; `names::search` and `AddressSearch` (`dialogs/names.rs`).
- Produces:
  - `pub struct CalendarsForm { pub search: AddressSearch, pub text: String, pub error: Option<String> }` (`Clone, Debug, Default`) and `Dialog::open_calendars() -> Dialog`, with an `AddressSearch::new()`;
  - `Action::DialogOk`, the open dialog's OK once its field's names are settled (Add calendar, Open another mailbox);
  - in `places.rs`:
    - `pub(crate) fn dialog_windows(d: &Dialog) -> Vec<u64>`: `[DIALOG_WINDOWS]` for these two dialogs (Task 4 adds its own), `[]` otherwise;
    - `pub(crate) fn prepare(people: &mut PeopleFields, s: &mut Session, d: &mut Dialog)`, which makes the dialog's field ready (and refreshes Add calendar's `search`).
- The accounts the fields use:
  - Add calendar uses `s.directory_account(None)`;
  - Open another mailbox uses its `account`.

The behaviour to build:
- **Add calendar's layout:** title "Open a shared calendar" and its line of text as today, then `names::search(…)`, then the field `dialog_field(0, "calendar")`:
  - label "Calendar ->", `book: true`;
  - the book button, or a double-click in the list, adds the person picked (`PeopleFields::add` with that `Addressee`);
  - then the choice dialog for the field, then `f.error`, and "Checking names…" while `holding(DIALOG_WINDOWS)`;
  - OK reads "Open". It is enabled when the field has something that isn't red, and nothing is held.
- **Add calendar's OK and Enter:** OK, or Enter while the field's input has the keyboard and its list is closed, pushes `Action::DialogOk`. The app's `dialog_ok` calls `gate_dialog("calendar", …, lenient: true)`; unless held, it goes on to `dialog_resolved(Ok)` at once.
- **What Add calendar's OK comes to (`dialog_resolved`):**
  - every addressee of the field is opened with `session.run("calendar.openShared", {"email": address})`;
  - `Ok` gives `opened`, and `Err(e)` gives `failed(.., e.to_string().trim_start_matches("bad parameters: "))`;
  - an `Err(why)` from the wait goes to `f.error`;
  - once the field has no piece left, the dialog closes (`ui.dialog = None`).
- **Open another mailbox's field:** `dialog_field(0, "mailbox")`, label `""`, hint `"support@example.com"`, no book. The checkbox stays.
- **Open another mailbox's OK and Enter:** they push `Action::DialogOk`. `dialog_ok` calls `gate_dialog("mailbox", Some(account), …, lenient: false)`.
- **What Open another mailbox's OK comes to:** `Ok` runs `Action::Submit("account.openMailbox", {"account", "email": the one addressee's address, "calendar"})`, which closes on success or shows the error. `Err(why)` goes to the dialog's `error`.
- **Each frame:** `settle` gets `dialog_windows(dialog)` among its `live` windows. `frame.rs` calls `places::prepare` before `dialogs::show`. `send_when_resolved` calls it first while `holding(DIALOG_WINDOWS)`, as it does for a meeting's lines.

- [ ] **Step 1: Write the failing tests** in `tests/writing/people_field/places.rs`

- `add_calendar_opens_each_entry_once_and_keeps_the_one_refused`:
  - set-up: `with_exchange`, then a directory answer making `team@contoso.example` a `DistributionList`;
  - open `Dialog::open_calendars()`, then type `ana@contoso.example; bo@contoso.example; team@contoso.example; riley@contoso.example; ` (riley is the account's own address);
  - press "Open";
  - `session.pim(gal).shared` holds ana and bo, once each;
  - the dialog is still open: team is a refused chip, and riley is `Text { error: Some(Failed("that's this account's own calendar")) }`;
  - press "Open" again: `shared` still holds ana and bo once each, and the two red entries are unchanged;
  - remove both red entries: the field is empty, "Open" is disabled, and the dialog is still open.
- `add_calendar_waits_for_a_name_then_opens_it`:
  - type `Ana`, press "Open": "Checking names…" shows and nothing is opened;
  - `directory_answers(.., "Ana", &[("Ana Lima", "ana@contoso.example")])` and two frames later, ana is in `shared` and the dialog is closed.
- `add_calendar_takes_a_person_from_its_book`:
  - with the sample's address book, type a query matching a contact in the book's search box;
  - select the first row and press "Calendar ->": the field has that person's chip;
  - a double-click on a row does the same.
- `open_mailbox_waits_for_a_typed_name`:
  - `Dialog::OpenSharedMailbox { account: gal, email: String::new(), calendar: false, error: None }`;
  - type `Support`, then Enter: the dialog is open, held, and `accounts[gal].config.shared_mailboxes` is empty;
  - the directory answers `("Support", "support@contoso.example")`;
  - two frames later the mailbox is open (`shared_mailboxes == ["support@contoso.example"]`) and the dialog is closed.
- `open_mailbox_stops_on_a_list_and_says_why`: with team known as a list, type team's address and press "Open". The dialog stays open with `error == Some("Only a person's, a room's or a shared mailbox can be opened")`, and nothing is opened.

Changed tests:
- `tests/reading.rs::enter_submits_a_one_field_dialog` keeps its purpose (Enter submits). It now opens `Dialog::open_calendars()` with `ana@contoso.example` fed in as the form's `text`. After the Enter, the sample account isn't Exchange, so the dialog stays open and the field's piece is `Text { error: Some(PieceError::Failed(_)) }`.
- The gallery in `tests.rs` builds `Dialog::open_calendars()` with an `error`.

- [ ] **Step 2: Run them to see them fail**

Run: `T cargo test -p quatrelle-ui-egui people_field::places enter_submits`
Expected: compile errors, then failures.

- [ ] **Step 3: Implement the two dialogs, the action and their wiring** as above.

- [ ] **Step 4: Look at it.** Run `cargo run -p quatrelle -- --sample --data <scratchpad folder> --cmd ui.<…>`, or set `a.ui.dialog` in a throwaway embed, with `QUATRELLE_SCREENSHOT=$SP/d3-calendar.png`. Read the PNG: the book above, the "Calendar ->" row, chips, and one red entry. Never use the owner's data folder.

- [ ] **Step 5: Run all UI tests and the gates**

Run: `T cargo test -p quatrelle-ui-egui`, then the three gates. Expected: PASS, exit 0.

- [ ] **Step 6: Commit**

```bash
git commit -m "Add calendar takes several people from its book, and Open another mailbox is a contacts field"
```

---

### Task 4: Rules and Search Folders — `sonnet`, effort `high`

**Files:**
- Modify:
  - `crates/ui-egui/src/dialogs/organize/rule_form.rs:34-51`, the condition rows;
  - `rule_form.rs:285-306`, `to_rule`;
  - `crates/ui-egui/src/dialogs.rs:676-691`, `RuleCond::of`;
  - `crates/ui-egui/src/dialogs/organize.rs:263-311`, the Search Folder's row;
  - `organize.rs:425-460`: `SearchFolderForm::edit` and `final_query`;
  - `dialogs.rs`'s `body`, which passes `people` to `organize::draw`;
  - `dialogs/people_field/places.rs`, which gains these two dialogs.
- Test: `crates/ui-egui/src/tests/organize.rs`.

**Interfaces:**
- Consumes:
  - from Task 1: `addr::matched`, `addr::matched_text`, `search::from_people`;
  - from Task 2: `dialog_field`, `DIALOG_WINDOWS`, `reset_window`, `blocked`, `choice_dialog`;
  - from Task 3: `places::{dialog_windows, prepare}`.
- Produces:
  - `pub(crate) fn rule_field(i: usize, test: RuleTest) -> Option<FieldId>` in `rule_form.rs`:
    - `From` and `To` give `dialog_field(i as u64, "rule-one")`;
    - `FromAny` and `ToAny` give `dialog_field(i as u64, "rule-any")`;
    - every other test gives `None`, and keeps its plain box.
  - `dialog_windows`:
    - `EditRule` gives `DIALOG_WINDOWS | i` for each condition `i` with a `rule_field`;
    - `SearchFolder` gives `[DIALOG_WINDOWS]` when its kind is people.
  - `prepare` makes each one ready, with the account `s.directory_account(f.account.as_deref())` for a rule and `s.directory_account(None)` for a Search Folder.

The behaviour to build:
- **A condition row:** a test with a `rule_field` draws the field there: label `""`, hint "Text or address", no book, `can_expand` when an account has a directory. The field takes 260 points; the row's × follows it.
- **The choice dialog:** it is drawn for each rule field, and for the Search Folder's.
- **Removing condition `i`:** this calls `people.reset_window(DIALOG_WINDOWS | j)` for each `j >= i`.
- **Save:** it is enabled unless a rule field is `blocked`.
- **`to_rule`:**
  - `From`/`To` give `Condition::From(matched(&c.text).into_iter().next().unwrap_or_default())`, `To` alike. A text that is empty is still refused with "Say what to look for".
  - `FromAny`/`ToAny` give `matched(&c.text)`.
  - `SubjectAny` keeps its own `,`/`;` split.
- **`RuleCond::of`:**
  - `From(t)` and `To(t)` give `matched_text(&[t])`;
  - `FromAny(v)` and `ToAny(v)` give `matched_text(&v)`;
  - `SubjectAny` is as today.
- **The Search Folder's "From these people":** its label stays, and the field is `dialog_field(0, "search-people")`, hint `ana@example.com, Jun`, no book.
- **`final_query`:** it takes `matched(&self.people)`. Each piece has its `"` removed, and is quoted when it has whitespace or `,`. The pieces are joined by `,` after `from:`.
- **`SearchFolderForm::edit`:** a query that `from_people` reads becomes `preset = PRESET_PEOPLE`, `people = matched_text(&list)`, `query` empty. Any other query is as today.

- [ ] **Step 1: Write the failing tests** in `tests/organize.rs`

- `a_rule_matches_addresses_alone_and_text_as_typed` (the spec's test):
  - `Dialog::rule_from("", Some("sample"), false, None)` with its condition set to `FromAny`;
  - type `Doe, John <j@x.fr>; @proginov.com; ` in `rule_field(0, FromAny)`;
  - the field has a chip and a text piece with no error;
  - `to_rule()` gives `Condition::FromAny(vec!["j@x.fr", "@proginov.com"])`.
- `a_search_folder_matches_addresses_alone_and_text_as_typed`:
  - the same text in the people Search Folder's field;
  - `final_query() == "from:j@x.fr,@proginov.com"`.
- `a_saved_rule_reopens_with_a_known_address_as_a_chip_and_text_as_text`:
  - a saved rule with `FromAny(vec!["zoe@studio.example", "Dupont"])` (zoe is in the sample's book) reopened with `RuleForm::from_rule`;
  - after two frames, `pieces(rule_field(0, FromAny))` is a chip for zoe (her name from the book) and `Text { text: "Dupont", error: None }`;
  - `to_rule()` gives back the same `FromAny`.
- `a_saved_rule_with_a_comma_saves_unchanged`: `Condition::From("Doe, John".into())` reopened, drawn twice and saved gives `Condition::From("Doe, John")`. The field shows one piece.
- `removing_a_rule_condition_keeps_each_field_its_own`:
  - three `FromAny` conditions `a@x.fr`, `b@x.fr`, `c@x.fr`, drawn;
  - remove the second (its × button);
  - after two frames, the fields of conditions 0 and 1 show `a@x.fr` and `c@x.fr`;
  - `to_rule()` agrees.
- `a_people_search_folder_reopens_as_people_and_saves_the_same`: a saved `from:zoe@studio.example,"Ana Lima",x` reopened has `preset == 8`, and `final_query()` is that same query.

Changed test: `search_folders_are_made_from_the_dialog_and_open` now expects the edit to reopen as people. The line `assert_eq!((edit.preset, edit.query.as_str()), (9, "from:…"))` becomes `assert_eq!((edit.preset, edit.final_query().as_str()), (8, "from:zoe@studio.example,\"Ana Lima\",x"))`. Its other assertions are unchanged.

- [ ] **Step 2: Run them to see them fail**

Run: `T cargo test -p quatrelle-ui-egui tests::organize`
Expected: failures, compile errors first.

- [ ] **Step 3: Implement the rows, the conversions and the wiring** as above.

- [ ] **Step 4: Look at it**, as Task 3's Step 4 does, with the rule editor holding two address conditions and one subject condition. Read the PNG: the rows aligned, the × after the field, the chips.

- [ ] **Step 5: Run all UI tests and the gates**

Run: `T cargo test -p quatrelle-ui-egui`, then the three gates. Expected: PASS, exit 0.

- [ ] **Step 6: Commit**

```bash
git commit -m "Rules and Search Folders: their people are contacts fields, addresses handed on alone, names kept as typed"
```

---

## After the tasks

- The final whole-branch review runs on `fable`, effort `xhigh`, against the spec's delivery 3 and this plan's decisions.
- Then one fix wave and a scoped re-review.
- Then the merge to main, a test build (Linux + Web), and the carnet item `champ-contacts` with what to try.
