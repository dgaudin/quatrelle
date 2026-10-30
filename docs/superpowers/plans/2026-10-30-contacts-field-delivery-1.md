# The contacts field, delivery 1 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One people search in the engine (suggest, resolve, kinds, expand), the room lists loaded at start-up and daily, the address-list splitter fixed, and the chip field in the message's To/Cc/Bcc/Reply-To and in the Select Names dialog, with Send waiting for the names still being resolved.

**Architecture:** The model gains the typed entry (`Addressee`, its `AddresseeKind`, what a place `Accepts`) and one splitter shared by `parse_list` and Check Names. The engine's `People` service (`session/mailboxes/people.rs`) owns the room set, the directory index, the per-field pending state keyed by `FieldId`, and a bounded suggestion pass; Check Names is generalised to any field and its commands become aliases. `net` reads `MailboxType` and a contact group's `ItemId` from `ResolveNames` (scope `ActiveDirectoryContacts`) and adds `ExpandDL`. The UI splits the field in two: pure drawing in `look/people_field.rs` (pieces in, events out) and session glue in `dialogs/people_field.rs` (`PeopleFields`, keyed by `FieldId`), which replaces `recipient_field` and the Select Names rows and gates `compose.send` on resolution.

**Tech Stack:** Rust, egui, serde, EWS SOAP (`ResolveNames`, `ExpandDL`, `GetRoomLists`/`GetRooms`); no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-30-contacts-field-design.md` (approved 2026-10-30). Delivery 1 only; deliveries 2 (meetings, From) and 3 (Add calendar, Open mailbox, rules, Search Folders) get their own plans and consume the interfaces marked **D2/D3** below.

## Global Constraints

- No `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!` outside tests, no `unsafe`; errors are `Result` through `?`; everything from the server is hostile (caps below).
- Layering (`cargo xtask layers`): `model` pure (no egui, no network, no clock); only `net` talks to the server; the engine is sans-I/O (`Job`s out, `Event`s in); `look/` never sees the `Session`.
- Sizes (`cargo xtask sizes`): ≤ 1000 lines per file, ≤ 16 `.rs` per folder; split before ~800. Full folders (16): `crates/model/src`, `crates/engine/src/session`, `crates/engine/src/tests`, `crates/ui-egui/src/tests`. Files near the limit: `crates/engine/src/tests/writing.rs` (930; add `mod` lines only), `crates/ui-egui/src/lib.rs` (916; it loses the six `suggest_*` fields), `crates/net/src/fakes/fake_ews/route.rs` (897; Task 2 moves its directory operations out), `crates/ui-egui/src/reading/compose.rs` (703; it loses `recipient_field`).
- Interface text through `tl!`/`tf!`, with a row in each of the 13 catalogs `crates/ui-egui/src/i18n/{ar,de,es,fr,he,it,ja,ko,nl,pt-br,ru,zh-hans,zh-hant}.tsv` (`every_interface_string_is_translated` fails otherwise; three TAB-separated columns `context`, English, translation).
- Everything is a command (`CommandSpec` in `crates/engine/src/cmd/`): new ids `people.suggest`, `people.resolve`, `people.choose`, `people.expand`; `compose.checkNames`, `compose.chooseName`, `contacts.suggest`, `contacts.directory` keep their ids and JSON; `every_command_survives_junk_params` (`crates/engine/src/tests/organize.rs`) must stay green.
- The spec's caps, verbatim: 8 suggestions; 500 entries per field; 500 members per expansion; 30,000 entries in the directory index; 300 directory answers kept (`KEPT`); 100 resolutions per answer plus `more`; one directory question at a time per account; the book's 20,000 (`MAX_CONTACTS`, not raised); `IO_TIMEOUT` 60 s for the wait at sending; `RETRY_AFTER` 60 s on a failed directory answer; the room set refreshed every 86,400 s.
- The scope: `ResolveNames` with `SearchScope="ActiveDirectoryContacts"` and `ReturnFullContactData="false"`; `directory_certificates` keeps `ActiveDirectory`.
- Privacy: a `log::` line never carries a name, an address, an item id or a query — counts only.
- Memory on this machine, for **every** cargo command: `mkdir -p ~/.tq` first; `TMPDIR=$HOME/.tq` (a deep path breaks the Unix-socket tests); `CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=4`; `CARGO_TARGET_DIR=$PWD/target` (and `$PWD/target-stable` for the stable clippy gate); **only one full build at a time** (another agent may be building in a worktree: check `pgrep -a cargo` first).
- Gates before every commit: `cargo fmt --all`; `TMPDIR=$HOME/.tq CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=4 CARGO_TARGET_DIR=$PWD/target cargo xtask ci`; `TMPDIR=$HOME/.tq CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=$PWD/target-stable cargo +stable clippy --workspace --all-targets -- -D warnings`. Known flake: `a_big_folder_changes_all_at_once_quickly` — rerun it alone once before concluding anything.
- Commits: small and topical, in English, `Area: what changed` plus a prose body saying why, ending with `Co-Authored-By: Claude <model name> <noreply@anthropic.com>` for the model that made it. Each fix's test is run without the fix once, to see it fail.
- Test shorthand below: `T` = `TMPDIR=$HOME/.tq CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=4 CARGO_TARGET_DIR=$PWD/target`.

## Review Focus

1. The owner's busy shared mailbox (550k items, 109 folders): a suggestion pass that runs on frames where nothing was typed, or a chip field that re-parses 300 recipients every frame, is the search bug of 2026-10-30 again → `suggestions_run_one_pass_per_change_of_text_over_a_book_at_its_cap` (Task 4) and `a_field_parses_its_text_once_per_change_not_per_frame` (Task 8).
2. The on-prem Exchange unreachable (VPN down, laptop asleep): Ctrl+K must leave the text with "The directory can't be reached", and a message to bare addresses must still send → `addresses_alone_resolve_without_the_directory_and_a_name_unreached_is_said` (Task 5) and `sending_waits_for_a_pending_name_then_stops_on_one_not_found` (Task 8).
3. A directory that never answers must not hold Send forever → `sending_gives_up_after_the_directorys_timeout` (Task 8).
4. Exchange display names with a comma (`Dupont, Marc <m@x>`) in a draft reopened or a reply: one chip, never two → `a_display_name_with_a_comma_stays_one_entry` (Task 1) and `text_fed_in_becomes_chips_with_their_kinds` (Task 8).
5. A hostile `ResolveNames`/`ExpandDL` answer (10,000 resolutions, a 5 kB name, a `PrivateDL` with no `ItemId`, a `MailboxType` nobody documented): capped, read as unknown, never a crash → `a_hostile_directory_answer_is_capped_and_unknown_kinds_read_as_unknown` (Task 2).

---

### Task 1: The typed entry, its kinds, and one splitter — `sonnet`, effort `medium` (pure model code with exact tests)

**Files:**
- Create: `crates/model/src/addr/entries.rs` (`crates/model/src/` has 16 files: nothing new goes there)
- Modify: `crates/model/src/addr.rs` (`pub mod entries;` beside `pub(crate) mod providers;`, `pub use entries::*;`, `parse_list` and `split_outside_quotes` use the splitter), `crates/model/src/contacts.rs:35` (`fn score` becomes `pub fn score`), `crates/model/src/lib.rs:31` (`pub use addr::{Addressee, AddresseeKind, EmailAddress};`)
- Test: `crates/model/src/addr.rs` (`mod tests`), `crates/model/src/addr/entries.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces, in `entries.rs`:
  - `#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)] #[serde(rename_all = "camelCase")] pub enum AddresseeKind { Person, DistributionList, ContactGroup, Room, Equipment, SharedMailbox, PublicFolder, Microsoft365Group, #[default] Unknown }` with `pub fn code(self) -> &'static str` (`"person"`, `"distributionList"`, …, `"unknown"`), `pub fn from_code(s: &str) -> Option<AddresseeKind>`, `pub fn from_mailbox_type(s: &str) -> AddresseeKind` (`Mailbox`/`Contact`/`OneOff` → `Person`, `PublicDL` → `DistributionList`, `PrivateDL` → `ContactGroup`, `PublicFolder` → `PublicFolder`, `GroupMailbox` → `Microsoft365Group`, anything else → `Unknown`), `pub fn is_list(self) -> bool` (`DistributionList | ContactGroup`).
  - `#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)] pub struct Addressee { pub name: String, pub address: String, #[serde(default)] pub kind: AddresseeKind, #[serde(default, skip_serializing_if = "String::is_empty")] pub item_id: String }` with `pub fn new(name: impl Into<String>, address: impl Into<String>, kind: AddresseeKind) -> Addressee`, `pub fn email(&self) -> EmailAddress`, `pub fn display(&self) -> &str`, `pub fn text(&self) -> String` (the entry as a field writes it: `EmailAddress::to_string` on the address, or on `group_ref(&self.item_id)` for a contact group), `impl From<EmailAddress> for Addressee` (kind `Unknown`).
  - `pub fn group_ref(item_id: &str) -> String` (`group:<item_id>`) and `pub fn parse_group_ref(s: &str) -> Option<&str>`: the textual form of a contact group, which has no address, inside `<…>` (`Team <group:AAMk…>`); `is_valid_email` stays false for it, so `parse_list` keeps refusing it and an unexpanded group can't be sent.
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct Accepts { pub kinds: &'static [AddresseeKind], pub one: bool }` with `pub const RECIPIENTS: Accepts` (every kind, several), `pub fn takes(&self, kind: AddresseeKind) -> bool` (`Unknown` is always taken: an address alone). **D2/D3** add `ATTENDEES`, `ROOMS`, `ONE_SENDER`, `CALENDARS`, `ONE_MAILBOX`, `MATCHES` here.
  - `pub const MAX_ENTRIES: usize = 500;`
  - `pub fn split_entries(input: &str) -> Vec<&str>`: parts split on `,` and `;` outside quotes and `<…>` (as `split_outside_quotes` does), trimmed, empties dropped, then the spec's join: a comma-separated part that is not an address (`is_valid_email` false) and has no `<…>` is joined (with `, `) to the part that follows when that one has `<…>`. `;` never joins.
- `parse_list` iterates `split_entries` (the old `split_outside_quotes` goes). Task 5's `tokens()` uses it too.

- [ ] **Step 1: Write the failing tests** in `entries.rs`:

```rust
#[test] fn a_display_name_with_a_comma_stays_one_entry() {
    assert_eq!(split_entries("Doe, John <j@x>"), ["Doe, John <j@x>"]);
    assert_eq!(split_entries("doe@x, John <j@x>"), ["doe@x", "John <j@x>"]);
    assert_eq!(split_entries("a@x,b@y"), ["a@x", "b@y"]);
    assert_eq!(split_entries("\"Doe, John\" <j@x>; Zoe"), ["\"Doe, John\" <j@x>", "Zoe"]);
    assert_eq!(split_entries("Zoe; nobody here, dupont"), ["Zoe", "nobody here", "dupont"]);
    assert_eq!(split_entries(" ; , "), Vec::<&str>::new());
    assert_eq!(parse_list("Doe, John <j@x>").unwrap(), [EmailAddress::new("Doe, John", "j@x")]);
    assert_eq!(parse_list("Doe, John <j@x>").unwrap()[0].to_string(), "\"Doe, John\" <j@x>");
}
#[test] fn kinds_come_from_mailbox_types_and_groups_have_a_text_form() {
    assert_eq!(AddresseeKind::from_mailbox_type("PrivateDL"), AddresseeKind::ContactGroup);
    assert_eq!(AddresseeKind::from_mailbox_type("GroupMailbox"), AddresseeKind::Microsoft365Group);
    assert_eq!(AddresseeKind::from_mailbox_type("Something2029"), AddresseeKind::Unknown);
    for k in [AddresseeKind::Person, AddresseeKind::Room, AddresseeKind::Unknown] { assert_eq!(AddresseeKind::from_code(k.code()), Some(k)); }
    let g = Addressee { name: "Team".into(), address: String::new(), kind: AddresseeKind::ContactGroup, item_id: "AAMk=".into() };
    assert_eq!(g.text(), "Team <group:AAMk=>");
    assert_eq!(parse_group_ref("group:AAMk="), Some("AAMk="));
    assert!(parse_list(&g.text()).is_err(), "a group isn't an address to send to");
    assert!(Accepts::RECIPIENTS.takes(AddresseeKind::Unknown) && Accepts::RECIPIENTS.takes(AddresseeKind::ContactGroup));
    assert_eq!(Addressee::from(EmailAddress::new("Zoe", "z@x")).kind, AddresseeKind::Unknown);
}
```

- [ ] **Step 2:** Run `T cargo test -p quatrelle-model --lib addr` — compile errors (the module is missing).
- [ ] **Step 3:** Implement `entries.rs` and rewire `parse_list`; keep `parses_mixed_lists`, `display_and_initials`, `validity` as they are.
- [ ] **Step 4:** Run `T cargo test -p quatrelle-model --lib` — all pass; `T cargo xtask wasm` still builds the model.
- [ ] **Step 5:** Commit: `Addresses: a typed entry with its kind, and a splitter that keeps "Doe, John <j@x>" whole`.

### Task 2: The directory answers with kinds, and a list expands — `opus`, effort `high` (SOAP parsing, a fake server, and a type change that crosses four crates)

**Files:**
- Modify: `crates/model/src/job.rs:350` (`Job::ExpandList`, and its arm in `Job::account` at :432), `crates/model/src/job/events.rs:249-255` (`DirectoryPeople.people: Vec<Addressee>`, `Event::ListExpanded`)
- Modify: `crates/net/src/exchange/ews/directory.rs` (scope, `people_of` typed, `expand_list`, `members_of`)
- Modify: `crates/net/src/worker/changes.rs:213` (dispatch), `crates/net/src/worker/scheduling.rs` (`expand_list`, beside `search_directory`), `crates/net/src/worker/pool.rs:437-440` (the offline answer), `crates/net/src/worker/send.rs:37` and `crates/net/src/worker.rs:178` and `:822` (`ExpandList` listed beside `SearchDirectory` in each)
- Create: `crates/net/src/fakes/fake_ews/directory.rs` (`mod directory;` in `fake_ews.rs`; `route.rs:134-178`'s `ResolveNames` arm moves here as `Data::resolve_names(&self, op: &El) -> (String, Vec<String>)`, and `expand_dl` joins it)
- Modify: `crates/net/src/fakes/fake_ews.rs:137` (`Data` gains `pub mailbox_types: Vec<(String, String)>` (address, `MailboxType`), `pub contact_groups: Vec<(String, String)>` (name, item id), `pub list_members: Vec<(String, Vec<(String, String)>)>` (a list's address or a group's item id → members as (name, address)), `pub scopes: Vec<String>` (each `ResolveNames`' `SearchScope`))
- Create: `crates/net/src/fakes/fake_ews/tests/directory.rs` (`mod directory;` in `fake_ews/tests.rs`)
- Mechanical, to keep the workspace building (the people of an answer are `Addressee`s now): `crates/engine/src/session/mailboxes/directory.rs:22,102` (`DirectoryAnswer.people: Vec<Addressee>`, `apply_directory(.., people: Vec<Addressee>, ..)`), `crates/engine/src/session/writing/names.rs:136` (`.map(Addressee::email)`), `crates/engine/src/session/mailboxes/contacts.rs:149` (same), `crates/engine/src/cmd/compose.rs:407` (`"people"` built as `[{"name", "email", "kind": kind.code()}]`, the key `email` kept), `crates/ui-egui/src/reading/sender.rs:52` (same map), the test helpers `crates/engine/src/tests/writing/names.rs:33` and `crates/ui-egui/src/tests/writing.rs:659` (`Addressee::from(EmailAddress::new(n, e))`).
- Test: `crates/net/src/exchange/ews/directory.rs` (`mod tests`), `crates/net/src/fakes/fake_ews/tests/directory.rs`

**Interfaces:**
- Produces in `model`: `Job::ExpandList { account: String, address: String, item_id: Option<String> }` (one of the two set; `account()` returns `account`); `Event::ListExpanded { account: String, address: String, item_id: Option<String>, members: Vec<Addressee>, more: bool, total: usize, error: Option<String> }`; `Event::DirectoryPeople { account, query, people: Vec<Addressee>, more, error }`.
- Produces in `net`: `Client::search_directory(&self, query: &str) -> Result<(Vec<Addressee>, bool)>` (scope `ActiveDirectoryContacts`); `Client::expand_list(&self, address: &str, item_id: Option<&str>) -> Result<(Vec<Addressee>, bool, usize)>` (members, `more` from `IncludesLastItemInRange="false"`, `total` from `TotalItemsInView`; the request is `<m:ExpandDL><m:Mailbox><t:EmailAddress>…</t:EmailAddress></m:Mailbox></m:ExpandDL>`, or `<t:ItemId Id="…"/>` in place of the address); `pub(super) fn people_of(m: &El) -> (Vec<Addressee>, bool)` and `pub(super) fn members_of(m: &El) -> (Vec<Addressee>, bool, usize)`.
- Caps (hostile input), as constants in `ews/directory.rs`: `MAX_ANSWER: usize = 100` resolutions read, `MAX_MEMBERS: usize = 500`, `MAX_NAME: usize = 200` chars, `MAX_ADDRESS: usize = 320`, `MAX_ITEM_ID: usize = 512`. A resolution with no `@` address is kept only as a contact group (`MailboxType` `PrivateDL` with an `ItemId`); duplicates by address (or by item id) are dropped; a `MailboxType` outside the list is `Unknown`.
- The fake answers `ResolveNames` with each mailbox's `<t:MailboxType>` (`mailbox_types`, else `Mailbox`), the contact groups whose name begins with the query as `<t:Mailbox><t:Name>…</t:Name><t:ItemId Id="…"/><t:MailboxType>PrivateDL</t:MailboxType></t:Mailbox>`, and `ExpandDL` from `list_members` (`<m:DLExpansion TotalItemsInView="n" IncludesLastItemInRange="true">` of `<t:Mailbox>`s, each with its `MailboxType`; an unknown list answers an empty expansion with `NoError`).

- [ ] **Step 1: Write the failing tests.** In `ews/directory.rs`, change `the_directory_gives_people_with_an_internet_address` to assert `people[0].kind == AddresseeKind::Person` on the same answer, and add:

```rust
#[test] fn kinds_and_contact_groups_come_from_resolve_names() {
    // One Resolution per MailboxType: Mailbox, PublicDL, PrivateDL (no EmailAddress, ItemId Id="AAMk=") , PublicFolder, GroupMailbox, Contact, and one <t:MailboxType>Something</t:MailboxType>.
    let (people, _) = people_of(&parse(answer));
    assert_eq!(people.iter().map(|p| p.kind).collect::<Vec<_>>(), [Person, DistributionList, ContactGroup, PublicFolder, Microsoft365Group, Person, Unknown]);
    assert_eq!((people[2].address.as_str(), people[2].item_id.as_str()), ("", "AAMk="));
}
#[test] fn a_hostile_directory_answer_is_capped_and_unknown_kinds_read_as_unknown() {
    // 10,000 Resolutions, one name of 5,000 chars, one PrivateDL without ItemId, one ItemId of 10,000 chars.
    let (people, more) = people_of(&parse(&answer));
    assert!(people.len() <= 100 && more);
    assert!(people.iter().all(|p| p.name.chars().count() <= 200 && p.address.len() <= 320 && p.item_id.len() <= 512));
    assert!(!people.iter().any(|p| p.kind == AddresseeKind::ContactGroup && p.item_id.is_empty()), "a group with no id is dropped");
}
#[test] fn a_list_is_expanded_with_its_members_and_how_many_more() {
    // <m:DLExpansion TotalItemsInView="3" IncludesLastItemInRange="false"> with two Mailboxes (one PublicDL, one Mailbox) and one with no address.
    let (members, more, total) = members_of(&parse(answer));
    assert_eq!((members.len(), more, total), (2, true, 3));
    assert_eq!(members[0].kind, AddresseeKind::DistributionList, "a nested list stays a list");
}
```

In `fake_ews/tests/directory.rs`, `the_directory_is_searched_with_contacts_and_a_list_expands`: a `Data` with `people` (Anne Dupont, Marc Dupont), `mailbox_types` (`reseaux@contoso.example` → `PublicDL`), `contact_groups` (`("Dupont family", "AAMk=")`), `list_members` for `reseaux@contoso.example` (two people) and for `AAMk=` (one person); `client.search_directory("dupont")` gives Anne, Marc and the group (kind `ContactGroup`, `item_id == "AAMk="`); `server.data.lock().unwrap().scopes == ["ActiveDirectoryContacts"]`; `expand_list("reseaux@contoso.example", None)` gives 2 members, `expand_list("", Some("AAMk="))` gives 1; `expand_list("nobody@contoso.example", None)` gives `(vec![], false, 0)`.

- [ ] **Step 2:** Run `T cargo test -p quatrelle-net --lib directory` — compile errors.
- [ ] **Step 3:** Add the job and event; implement `net`; move the fake's `ResolveNames` into `fake_ews/directory.rs` and add `ExpandDL`; make the mechanical engine/UI edits listed above (no behaviour change there).
- [ ] **Step 4:** Run `T cargo test -p quatrelle-net --lib directory`, then `T cargo xtask ci` — all pass, `every_command_survives_junk_params` and the three Check Names tests of `crates/engine/src/tests/writing/names.rs` untouched in behaviour.
- [ ] **Step 5:** Commit: `Directory: ResolveNames answers with each entry's kind and a group's id, and a list expands`.

### Task 3: The room set, loaded at start-up and refreshed daily — `sonnet`, effort `high` (small code, but the refresh must not disturb the Room Finder's state)

**Files:**
- Create: `crates/engine/src/session/mailboxes/people.rs` (`mod people; pub(crate) use people::People;` in `crates/engine/src/session/mailboxes.rs:7-17`)
- Modify: `crates/engine/src/session.rs` (`pub(crate) people: mailboxes::People` beside `contacts` at :392, `People::default()` at :546), `crates/engine/src/session/scheduling.rs` (`refresh_rooms_due` beside `request_rooms`; `rooms_answered` feeds the set and keeps a `Ready` on a failed quiet refresh), `crates/engine/src/session/organize.rs:194` (`tick` calls `self.refresh_rooms_due()`)
- Create: `crates/engine/src/tests/writing/people_field.rs` (`mod people_field;` in `crates/engine/src/tests/writing.rs:5-9`; `use super::*;` and the `with_exchange`, `asked`, `answer` helpers of `tests/writing/names.rs` made `pub(super)` there and reused)

**Interfaces:**
- Produces: `pub(crate) struct People` (Default) holding `rooms: HashMap<String, HashMap<String, String>>` (account → room address, lower case → name), `rooms_asked: HashMap<String, (i64, bool)>` (when, failed), plus the fields Tasks 4–6 add.
- Constants in `people.rs`: `pub const ROOMS_EVERY: i64 = 86_400;`, `pub const ROOMS_RETRY: i64 = 600;`.
- `Session::refresh_rooms_due(&mut self)` (in `scheduling.rs`, `pub(crate)`): for each account where `server_schedules` and `!prefs.work_offline` and `status` is `Online { .. }` or `Syncing { .. }`, when never asked, or asked ≥ `ROOMS_EVERY` ago, or failed ≥ `ROOMS_RETRY` ago: push `Job::GetRooms` (one per account at a time), record `rooms_asked`; `self.rooms` is set to `Loading` only when it holds nothing for the account (a `Ready` Room Finder isn't blanked by a refresh).
- `rooms_answered`: with no error, or with rooms, the account's set is rebuilt from the merged `rooms` (the calendar's when the server has no lists, as `from_calendar` has it) and `self.contacts.changed()` is called; with an error and nothing known, `Failed` replaces only a `Loading` or absent entry (a `Ready` stays); `rooms_asked` gets `(now, error.is_some() && rooms.is_empty())`.
- `pub fn room_name(&self, account: &str, email: &str) -> Option<&str>` and `pub fn addressee_kind(&self, account: Option<&str>, email: &str) -> AddresseeKind`: the room set (`Room`) → any account's `config.shared_mailboxes` (`SharedMailbox`) → Task 4's directory index when its entry's kind isn't `Unknown` → `known_contact` (`Person`) → `Unknown`. Until Task 4 lands, the index step is absent.
- The test file's imports: `use super::names::{answer, asked, with_exchange};` (a private sibling module's `pub(super)` items), `crate::session::scheduling::Fetched`, `quatrelle_model::scheduling::{Room, RoomList}`. **D2/D3** consume `addressee_kind` for the room move and the commands' best-effort refusals (`calendar.openShared`, `account.openMailbox`).

- [ ] **Step 1: Write the failing test** `rooms_are_known_from_the_lists_loaded_at_start_up_and_a_day_later`:

```rust
let (mut s, gal) = with_exchange("rooms-startup");
s.tick(); assert!(asked_rooms(&mut s).is_empty(), "offline: nothing asked");   // asked_rooms: take_jobs filtered on Job::GetRooms, in this file
s.apply(Event::Status { account: gal.clone(), status: SyncStatus::Online { at: NOW } });
s.tick(); assert_eq!(asked_rooms(&mut s), [gal.clone()]);
s.tick(); assert!(asked_rooms(&mut s).is_empty(), "asked once");
s.apply(Event::Rooms { account: gal.clone(), lists: vec![RoomList { email: "rooms.paris@contoso.example".into(), name: "Paris".into() }], rooms: vec![Room { email: "room.seine@contoso.example".into(), name: "Salle Seine".into(), list: "Paris".into() }], error: None });
assert_eq!(s.addressee_kind(Some(&gal), "Room.Seine@contoso.example"), AddresseeKind::Room);
assert_eq!(s.room_name(&gal, "room.seine@contoso.example"), Some("Salle Seine"));
assert!(matches!(s.rooms_now(&gal), Some(Fetched::Ready(_))), "the Room Finder was never opened, and has them");
s.set_clock(NOW + 86_401, 0); s.tick(); assert_eq!(asked_rooms(&mut s), [gal.clone()], "a day later");
s.apply(Event::Rooms { account: gal.clone(), lists: Vec::new(), rooms: Vec::new(), error: Some("timeout".into()) });
assert_eq!(s.addressee_kind(Some(&gal), "room.seine@contoso.example"), AddresseeKind::Room, "a failed refresh keeps the last set");
assert!(matches!(s.rooms_now(&gal), Some(Fetched::Ready(_))));
s.set_clock(NOW + 86_401 + 601, 0); s.tick(); assert_eq!(asked_rooms(&mut s), [gal.clone()], "a failure is tried again sooner");
assert_eq!(s.addressee_kind(Some(&gal), "zoe@studio.example"), AddresseeKind::Person, "known from mail");
assert_eq!(s.addressee_kind(Some(&gal), "nobody@elsewhere.example"), AddresseeKind::Unknown);
```

- [ ] **Step 2:** Run `T cargo test -p quatrelle-engine --lib rooms_are_known` — compile errors.
- [ ] **Step 3:** Implement `People`, `refresh_rooms_due`, the `rooms_answered` changes, `room_name`, `addressee_kind`.
- [ ] **Step 4:** Run the test and `T cargo test -p quatrelle-engine --lib scheduling` — `an_exchange_account_asks_its_server_once` (`crates/engine/src/tests/scheduling.rs:98`) passes unchanged (it never calls `tick`, and its `refresh: true` asks set `Loading` as before).
- [ ] **Step 5:** Commit: `Rooms: the room lists are loaded as the account comes online and once a day, so rooms are told from people`.

### Task 4: Suggest — local, bounded, with sources and the directory index — `opus`, effort `high` (the ordering rules and the bounded-work invariant)

**Files:**
- Modify: `crates/engine/src/session/mailboxes/people.rs` (the index, the cache, `people_suggest`), `crates/engine/src/session/mailboxes/directory.rs:102-124` (`apply_directory` calls `index_directory`), `crates/engine/src/lib.rs:28` (re-export `FieldId, Suggestion, Source`)
- Create: `crates/engine/src/cmd/people.rs` (`mod people;` in `crates/engine/src/cmd.rs:57-66`, `all.extend(people::commands())` in `registry()`)
- Modify: `crates/engine/src/cmd/compose.rs:112-122` (`contacts.suggest` answers through `people_suggest`, its keys `name`, `email`, `address`, `sent`, `received` kept and `kind`, `source` added)
- Test: `crates/engine/src/tests/writing/people_field.rs`

**Interfaces:**
- Produces in `people.rs`:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)] pub struct FieldId { pub window: u64, pub name: &'static str }` with `pub const FIELD_NAMES: &[&str] = &["to", "cc", "bcc", "replyTo", "book-to", "book-cc", "book-bcc", "cli"]` (**D2/D3** add `"required"`, `"optional"`, `"rooms"`, `"from"`, `"email"`, `"from-any"`, `"to-any"`) and `pub fn parse(window: u64, name: &str) -> Option<FieldId>` (one of `FIELD_NAMES`).
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)] #[serde(rename_all = "camelCase")] pub enum Source { Recent, Contacts, Directory, RoomList }`; `#[derive(Clone, Debug, PartialEq, Eq, Serialize)] pub struct Suggestion { pub who: Addressee, pub source: Source }`; `pub const MAX_SUGGESTIONS: usize = 8;`, `pub const INDEX_MAX: usize = 30_000;`.
  - `People` gains `index: HashMap<String, Addressee>` (by lower-cased address, or by `group:<item_id>` for a contact group), `index_order: VecDeque<String>`, `suggest_cache: HashMap<FieldId, (String, u64, Vec<Suggestion>)>` (text, revision, result), `passes: u64`, `scanned: u64`.
  - `Session::people_suggest(&mut self, field: FieldId, account: Option<&str>, accepts: &Accepts, text: &str) -> Vec<Suggestion>`: empty for a blank `text`; otherwise one pass when `(text, contacts_revision())` differs from the cache's entry for `field`, else the cached answer cloned. A pass: `contacts_step(0)`, `book.suggest(text, MAX_SUGGESTIONS * 4, now)` (`Recent`, or `Contacts` when `saved`), the index's entries whose name word or address starts with the text (`Directory`), the account's room set likewise (`RoomList`), the accounts' `config.shared_mailboxes` likewise (kind `SharedMailbox`, `Source::Recent`); only `accepts.takes(kind)`; ordered exact match (name or address equal, case-insensitive) first, then prefix of the first or last name or of the address, then `Contact::score` descending (0 for index and rooms), then the position of the kind in `accepts.kinds`, then address; cut to `MAX_SUGGESTIONS`. `passes += 1`, `scanned += book.len() + index.len() + rooms.len()`.
  - `Session::people_suggest_passes(&self) -> (u64, u64)` (passes, entries scanned).
  - `Session::index_directory(&mut self, people: &[Addressee])` (`pub(crate)`): called by `apply_directory` (and by Task 6's `list_expanded`), never by suggestions; FIFO at `INDEX_MAX`; cleared with the answers in `apply_directory` when `KEPT` evicts (the index is cleared whole then, as the spec allows: "cleared with the answers").
  - `Session::directory_known(&self, email: &str) -> Option<&Addressee>`. Task 3's `addressee_kind` consults it.
- Command `people.suggest` (`cmd/people.rs`): params `{"window": number, "field": string (one of FIELD_NAMES), "text": string, "account"?: string, "accepts"?: [kind codes]}` (none: every kind); answer `[{"name", "email", "kind", "itemId"?, "source"}]`; a bad `field` is `CmdError::Params`.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn suggestions_are_ordered_exact_then_prefix_then_most_used_and_only_accepted_kinds() {
    let (mut s, gal) = with_exchange("suggest-order");
    answer(&mut s, &gal, "res", &[("Proginov Reseau", "reseaux@contoso.example")]);      // the helper gives kind Unknown; Source::Directory
    s.apply(Event::DirectoryPeople { account: gal.clone(), query: "team".into(), people: vec![Addressee::new("Team", "team@contoso.example", AddresseeKind::DistributionList)], more: false, error: None });
    s.apply(Event::Rooms { /* Salle Seine, room.seine@contoso.example */ });
    let f = FieldId { window: 1, name: "to" };
    let got = s.people_suggest(f, Some(&gal), &Accepts::RECIPIENTS, "zoe");
    assert_eq!(got[0].who.address, "zoe@studio.example"); assert_eq!(got[0].source, Source::Recent);
    let got = s.people_suggest(f, Some(&gal), &Accepts::RECIPIENTS, "reseaux@contoso.example");
    assert_eq!((got[0].who.name.as_str(), got[0].source), ("Proginov Reseau", Source::Directory), "an exact match first");
    assert!(s.people_suggest(f, Some(&gal), &Accepts::RECIPIENTS, "te").iter().any(|x| x.who.kind == AddresseeKind::DistributionList));
    let only_rooms = Accepts { kinds: &[AddresseeKind::Room, AddresseeKind::Equipment], one: false };
    let got = s.people_suggest(f, Some(&gal), &only_rooms, "s");
    assert!(got.iter().all(|x| x.who.kind == AddresseeKind::Room) && got[0].source == Source::RoomList, "{got:?}");
    assert!(s.people_suggest(f, Some(&gal), &Accepts::RECIPIENTS, "zzzz-nobody").is_empty());
    assert!(s.people_suggest(f, Some(&gal), &Accepts::RECIPIENTS, "z").len() <= 8);
}
#[test] fn suggestions_run_one_pass_per_change_of_text_over_a_book_at_its_cap() {
    let mut s = sample_session("suggest-bounded");
    let many: Vec<EmailAddress> = (0..20_000).map(|i| EmailAddress::new(format!("Ann {i}"), format!("ann{i}@big.example"))).collect();
    s.contacts_sent(&many);
    let f = FieldId { window: 1, name: "to" };
    for _ in 0..10 { s.people_suggest(f, None, &Accepts::RECIPIENTS, "ann1"); }
    let (passes, scanned) = s.people_suggest_passes();
    assert_eq!(passes, 1, "one pass for one text"); assert!(scanned <= 20_000 + 30_000 + 2_000, "{scanned}");
    s.people_suggest(f, None, &Accepts::RECIPIENTS, "ann12");
    assert_eq!(s.people_suggest_passes().0, 2);
    s.people_suggest(FieldId { window: 2, name: "to" }, None, &Accepts::RECIPIENTS, "ann12");
    assert_eq!(s.people_suggest_passes().0, 3, "another field is another pass");
}
#[test] fn the_directory_index_feeds_suggestions_and_is_never_filled_by_them() {
    // After answer(.., "res", ..): directory_known("RESEAUX@contoso.example") is Some; people_suggest(.., "zo") on a book contact leaves directory_known("zoe@studio.example") None;
    // 30,001 entries indexed through DirectoryPeople answers of 100 each → index len ≤ 30_000 and the oldest gone.
    // people.suggest through run(): [{"name","email","kind","source"}] for "zo"; a field name "nope" → Err(Params).
}
```

- [ ] **Step 2:** Run `T cargo test -p quatrelle-engine --lib people_field` — compile errors.
- [ ] **Step 3:** Implement; `contacts.suggest` keeps its JSON and adds `kind` and `source`.
- [ ] **Step 4:** Run `T cargo test -p quatrelle-engine --lib` — all pass, `every_command_survives_junk_params` included; `recipient_boxes_suggest_people` (UI, still on `suggest_contacts_now`) unchanged for now.
- [ ] **Step 5:** Commit: `People: one bounded suggestion pass per change of text, from the book, the directory's answers and the room lists`.

### Task 5: Resolve and choose for any field — Check Names generalised — `opus`, effort `xhigh` (the existing Check Names tests must keep passing while the state is re-keyed)

**Files:**
- Modify: `crates/engine/src/session/writing/names.rs` (whole file: `NameCheck` and `NameChoice` keyed by `FieldId`, `tokens()` on `split_entries`, `people_resolve`, `people_choose`, outcomes; `check_names` and `choose_name` kept as the compose aliases), `crates/engine/src/session/mailboxes/people.rs` (`pending`, `outcomes`), `crates/engine/src/cmd/people.rs` (`people.resolve`, `people.choose`), `crates/engine/src/cmd/compose.rs:37-40` (unchanged ids, same functions)
- Modify: `crates/ui-egui/src/reading/check_names.rs:10-11,63-66` (`choice.field.window` for the id, `choice.field.name` in the picked id, `p.address`; the commands it runs stay `compose.chooseName` until Task 8)
- Test: `crates/engine/src/tests/writing/people_field.rs`; `crates/engine/src/tests/writing/names.rs` (the three tests keep passing as written)

**Interfaces:**
- `pub struct NameChoice { pub field: FieldId, pub token: String, pub people: Vec<Addressee> }` (was `compose: u64`, `Vec<EmailAddress>`); `pub(crate) struct NameCheck { field: FieldId, account: String, token: String }`.
- `#[derive(Clone, Debug, PartialEq, Eq)] pub enum Outcome { Found(Addressee), NotFound, NotAnAddress, Unreached(String), Ambiguous }` (never serialised: commands answer with `Resolved`); `#[derive(Clone, Debug, PartialEq, Eq, Serialize)] pub struct Resolved { pub text: String, pub resolved: usize, pub unresolved: Vec<String>, pub searching: Vec<String> }`.
- `Session::people_resolve(&mut self, field: FieldId, account: Option<&str>, accepts: &Accepts, text: Option<&str>) -> Result<Resolved, CmdError>`: for a compose field (`name` in `to`/`cc`/`bcc`/`replyTo` of a message being written whose id is `window`) the text is the message's own and the result is written back to it, as `check_names` writes today; for any other field `text` is required and only `Resolved.text` carries the rewrite. Per token of `split_entries(text)` (at most `MAX_NAMES` = 500): a token with `@` is `parse_one`d — valid: rewritten `Name <address>` when the book, the index or the room set knows the address (the name filled as `fill_name` does), else kept as typed (`Unknown`); invalid: `Outcome::NotAnAddress`, counted unresolved; a `<group:…>` ref is kept; a name: `local_names` as today (one match written, several → `NameChoice`, none → the directory of `account` (`directory_account`) when there's one, else `NotFound`). Answers come as today through `directory_checked`, which now settles by `FieldId`: one match → written (compose fields) and `Outcome::Found`; several → `NameChoice` and `Ambiguous`; none → `NotFound`, or `Unreached(error)` when the answer carries an error; `NAMES_NOT_FOUND` is still said once per batch with a `NotFound`.
- `Session::people_choose(&mut self, field: FieldId, token: &str, person: Option<Addressee>) -> CmdResult` (`{"token", "written"}` as today); `Session::name_choice(&self, window: u64) -> Option<&NameChoice>` (the first for any field of the window; the signature stays); `Session::name_choice_for(&self, field: FieldId) -> Option<&NameChoice>`; `Session::take_people_outcomes(&mut self, field: FieldId) -> Vec<(String, Outcome)>`; `Session::people_pending(&self, field: FieldId) -> usize` (names still waiting on the directory).
- `check_names()` = `people_resolve` on `to`, `cc`, `bcc`, `replyTo` of the current message, its JSON `{resolved, unresolved, searching}` summed as today; `choose_name(token, person)` = `people_choose` on whichever field of the current message holds a choice for `token`.
- Commands (`cmd/people.rs`): `people.resolve {"window": number, "field": string, "text"?: string, "account"?: string, "accepts"?: [codes]}` → `{"text", "resolved", "unresolved", "searching"}`; `people.choose {"window", "field", "token", "email"?, "name"?, "kind"?, "itemId"?}` → `{"token", "written"}`. `compose.checkNames` and `compose.chooseName` keep calling `check_names`/`choose_name` (`.when(has_compose)`), their JSON unchanged.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn check_names_with_the_contacts_scope_finds_a_person_once_and_offers_a_contact_group() {
    let (mut s, gal) = with_exchange("scope-contacts");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "joanna; family"})).unwrap();
    s.run("compose.checkNames", &json!({})).unwrap();
    assert_eq!(asked(&mut s), ["joanna"]);
    // The directory and the Contacts folder both have her: one person, written once.
    answer(&mut s, &gal, "joanna", &[("Joanna Smith", "jo@contoso.example"), ("Joanna Smith", "JO@contoso.example")]);
    assert_eq!(s.compose.as_ref().unwrap().to, "Joanna Smith <jo@contoso.example>; family; ");
    assert_eq!(asked(&mut s), ["family"]);
    let group = Addressee { name: "Dupont family".into(), address: String::new(), kind: AddresseeKind::ContactGroup, item_id: "AAMk=".into() };
    s.apply(Event::DirectoryPeople { account: gal.clone(), query: "family".into(), people: vec![group.clone()], more: false, error: None });
    assert_eq!(s.compose.as_ref().unwrap().to, "Jo (from the gym) <jo@contoso.example>; Dupont family <group:AAMk=>; ", "a group found alone is written as its reference");
    let id = s.compose.as_ref().unwrap().id;
    assert!(s.name_choice(id).is_none());
    assert_eq!(s.take_people_outcomes(FieldId { window: id, name: "to" }).last().map(|(t, o)| (t.as_str(), o.clone())), Some(("family", Outcome::Found(group))));
}
#[test] fn addresses_alone_resolve_without_the_directory_and_a_name_unreached_is_said() {
    let (mut s, gal) = with_exchange("resolve-offline");
    s.run("mail.new", &json!({})).unwrap();
    let id = s.compose.as_ref().unwrap().id;
    let f = FieldId { window: id, name: "to" };
    s.run("compose.update", &json!({"to": "zoe@studio.example; a@; dupont"})).unwrap();
    let r = s.people_resolve(f, Some(&gal), &Accepts::RECIPIENTS, None).unwrap();
    assert_eq!((r.resolved, r.unresolved.as_slice(), r.searching.as_slice()), (1, &["a@".to_string()][..], &["dupont".to_string()][..]));
    assert_eq!(s.compose.as_ref().unwrap().to, "Zoe Park <zoe@studio.example>; a@; dupont; ", "a known address takes its name; nothing asked for it");
    assert_eq!(asked(&mut s), ["dupont"]);
    assert_eq!(s.people_pending(f), 1);
    s.apply(Event::DirectoryPeople { account: gal.clone(), query: "dupont".into(), people: Vec::new(), more: false, error: Some("offline".into()) });
    let out = s.take_people_outcomes(f);
    assert!(out.contains(&("a@".into(), Outcome::NotAnAddress)) && out.contains(&("dupont".into(), Outcome::Unreached("offline".into()))), "{out:?}");
    assert_eq!(s.people_pending(f), 0);
    assert_eq!(s.compose.as_ref().unwrap().to, "Zoe Park <zoe@studio.example>; a@; dupont; ", "the text stays");
}
#[test] fn a_dialogs_field_resolves_its_own_text_and_chooses_by_field() {
    // window 7, field "book-to", text "Zoe Park; dupont": Resolved.text == "Zoe Park <zoe@studio.example>; dupont; ", searching ["dupont"]; the message being written is untouched;
    // two Dupont answered → name_choice_for(FieldId{7,"book-to"}) is Some and name_choice(7) too; people.choose through run() with window 7 → Outcome::Found(Marc) in take_people_outcomes; a choose for window 8 → Err(Params).
}
```

- [ ] **Step 2:** Run `T cargo test -p quatrelle-engine --lib people_field` — compile errors.
- [ ] **Step 3:** Rewrite `names.rs` around `FieldId`; add the commands; update `check_names.rs` (UI) to the new `NameChoice`.
- [ ] **Step 4:** Run `T cargo test -p quatrelle-engine --lib` and `T cargo test -p quatrelle-ui-egui --lib writing` — the three tests of `tests/writing/names.rs` and the UI's `ctrl_k_checks_the_names_typed` and `check_names_asks_which_person_a_name_means` pass as written.
- [ ] **Step 5:** Commit: `Check Names: names resolve for any field, keyed by its window and name, with each name's outcome kept`.

### Task 6: Expand a list or a contact group — `sonnet`, effort `high` (a job/event round trip in the pattern of the rooms)

**Files:**
- Modify: `crates/engine/src/session/mailboxes/people.rs` (`expansions`, `people_expand`, `list_expanded`), `crates/engine/src/session/events.rs:150` (`Event::ListExpanded { .. } => self.list_expanded(..)`), `crates/engine/src/cmd/people.rs` (`people.expand`)
- Test: `crates/engine/src/tests/writing/people_field.rs`

**Interfaces:**
- `#[derive(Clone, Debug, PartialEq, Eq)] pub struct Expansion { pub members: Vec<Addressee>, pub more: bool, pub total: usize }`; `pub const MAX_SHOWN_MEMBERS: usize = 500;`
- `People.expansions: HashMap<(String, String), Fetched<Expansion>>` keyed by (account, the list's lower-cased address or `group:<item_id>`).
- `Session::people_expand(&mut self, account: Option<&str>, who: &Addressee) -> CmdResult`: `Disabled("the directory is an Exchange account's")` without `directory_account(account)`; `Params` unless `who.kind.is_list()` and it has an address or an item id; pushes `Job::ExpandList` once (`Loading` kept while it's out), answers `{"account", "state": "loading"|"ready"|"failed", "members": [...], "more", "total", "error"?}` as `rooms_json` does.
- `Session::expansion_now(&self, account: &str, who: &Addressee) -> Option<&Fetched<Expansion>>`; `Session::list_expanded(&mut self, account: &str, address: String, item_id: Option<String>, members: Vec<Addressee>, more: bool, total: usize, error: Option<String>)` (`pub(crate)`): members cut to `MAX_SHOWN_MEMBERS` and indexed (`index_directory`); an error → `Failed(error)`.
- Command `people.expand {"account"?: string, "email"?: string, "itemId"?: string, "name"?: string, "kind"?: code (default distributionList)}` → the answer above.

- [ ] **Step 1: Write the failing test** `a_list_is_expanded_by_address_a_group_by_item_id_a_nested_list_stays_a_list_and_an_empty_one_is_said`: `people.expand {"email": "team@contoso.example"}` → `state == "loading"`, `take_jobs() == [Job::ExpandList { account: gal, address: "team@contoso.example", item_id: None }]`, again → no new job; `apply(Event::ListExpanded { members: [Addressee Person, Addressee DistributionList "sub@…"], more: true, total: 700, error: None })` → `state == "ready"`, `members[1].kind == "distributionList"`, `more == true`, `total == 700`, `directory_known("sub@contoso.example").is_some()`; `{"itemId": "AAMk=", "name": "Family", "kind": "contactGroup"}` → `Job::ExpandList { address: "", item_id: Some("AAMk=") }`; an answer with no member and no error → `ready` with `members == []`; an answer with `error: Some("ErrorAccessDenied")` → `failed`; on the sample account (no Exchange) → `Err(CmdError::Disabled(_))`.
- [ ] **Step 2:** Run `T cargo test -p quatrelle-engine --lib a_list_is_expanded` — compile errors.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `T cargo test -p quatrelle-engine --lib` — all pass (`every_command_survives_junk_params` covers `people.expand`).
- [ ] **Step 5:** Commit: `People: a distribution list or a contact group expands to its members, 500 shown`.

### Task 7: The field's drawing, and the kinds' icons — `opus`, effort `high` (an egui widget with wrapping, a popup list and keyboard rules, tested headless)

**Files:**
- Create: `crates/ui-egui/src/look/people_field.rs` (`pub(crate) mod people_field;` in `crates/ui-egui/src/look.rs:4-12`; `look/` has 9 files) and `crates/ui-egui/src/look/people_field/{chips.rs, list.rs, card.rs}`
- Modify: `crates/ui-egui/src/look/icons.rs:14` (`Icon::{DistributionList, ContactGroup, Room, Equipment, SharedMailbox, PublicFolder, Group365, Address}`) and `:131` (`svg`: `people_list`, `people_team`, `conference_room`, `projection_screen`, `mail_multiple`, `folder_mail`, `people_community`, and `Address => f!("mail")` reusing the file `Unread` uses), `crates/ui-egui/src/look/icons/drawn.rs` (a line drawing per new icon), `ATTRIBUTION.md:56-` (one row per new file, the `people_20_regular.svg` row copied), the 13 catalogs (the kind names and the strings below)
- Assets: seven files `assets/icons/fluent/{people_list,people_team,conference_room,projection_screen,mail_multiple,folder_mail,people_community}_20_regular.svg`, taken unchanged from `@fluentui/svg-icons` 1.1.292 (`npm pack @fluentui/svg-icons@1.1.292` in the scratchpad, `package/icons/<name>_20_regular.svg`); `cargo xtask assets` checks the rows.
- Create: `crates/ui-egui/src/tests/writing/people_field.rs` (`mod people_field;` in `crates/ui-egui/src/tests/writing.rs`; `crates/ui-egui/src/tests/` has 16 files)

**Interfaces:**
- Produces in `look/people_field.rs` (pure: no `Session`):
  - `#[derive(Clone, Debug, PartialEq, Eq)] pub enum PieceError { NotAnAddress, NotFound, Unreached, Refused(String) }` (`Refused` carries the translated reason); `#[derive(Clone, Debug, PartialEq, Eq)] pub enum Piece { Chip(Addressee), Text { text: String, error: Option<PieceError> } }`.
  - `#[derive(Default)] pub struct FieldState { pub pieces: Vec<Piece>, pub input: String, pub selected: Option<usize>, pub open: bool, pub highlighted: usize, pub card: Option<usize>, pub dismissed_for: Option<String>, pub left_out: Option<usize>, pub synced: String, pub syncs: u32 }` (`synced`/`syncs` are Task 8's: the text last written back, and how many times the text was parsed).
  - `pub struct FieldInput<'a> { pub id: egui::Id, pub label: &'a str, pub hint: &'a str, pub accepts: &'a Accepts, pub suggestions: &'a [Suggestion], pub can_expand: bool, pub focus: bool, pub pending: usize }`.
  - `#[derive(Clone, Debug, PartialEq, Eq)] pub enum PeopleEvent { Changed, Closed(String), Take(usize), Forget(String), RemoveChip(usize), Expand(usize), Resolve, OpenBook }` — `Closed(text)` on `;`, `,`, a paste, or the input losing focus with text in it; `Take(i)` a suggestion taken (Enter, Tab with the list open, a click); `Resolve` on Ctrl+K with the focus in the field.
  - `pub fn people_field(ui: &mut Ui, t: &Tokens, st: &mut FieldState, input: &FieldInput<'_>) -> Vec<PeopleEvent>`: the book button (`label_button` look, `OpenBook`), the chips wrapped with the input after them, growing to `MAX_LINES: usize = 4` then a `ScrollArea`; the suggestion list as an `egui::Area` (`Order::Foreground`) anchored under the input with at most 8 rows of icon, name, address, kind and source, `Delete` on a row → `Forget(address)`; the keys of the spec (↑/↓, Enter, Tab open/closed, Esc, Backspace selects then removes, `;`/`,`); a chip: `kind_icon(kind)`, the name through `bidi::label`, hover → the address, click → the card (`card.rs`: name, address, kind, "Remove", and "Expand" for a list when `can_expand`); a `Text` piece with an error drawn red-underlined with its tooltip; `widgets::named` on each chip with `tf!("{name}, {kind}, remove with Delete")`; Ctrl+C with a chip selected copies `Addressee::text`, with the input empty and nothing selected copies every chip joined `"; "`.
  - `pub fn kind_icon(kind: AddresseeKind) -> Icon`; `pub fn kind_name(kind: AddresseeKind) -> &'static str` (`tl!` keys: "Person", "Distribution list", "Contact group", "Room", "Equipment", "Shared mailbox", "Public folder", "Microsoft 365 group", "Address"); `pub fn source_name(source: Source) -> &'static str` ("Recent", "Contacts", "Directory (cached)", "Room list").
  - `pub fn text_of(pieces: &[Piece], input: &str) -> String`: entries (`Addressee::text` for a chip, the text for a piece) joined by `"; "`, then `"; "` and the input when there are entries (so `[Zoe chip, "nobody here"]` + `""` gives `Zoe Park <zoe@studio.example>; nobody here; `, as Check Names writes today).
- New strings: the kind and source names above, "and {n} more were left out", "and {n} more", "This list has no members", "Remove", "Expand", "Not an address: {what}", "Not found: {name}", "The directory can't be reached", "Matched as typed" (**D3**), "{name}, {kind}, remove with Delete".

- [ ] **Step 1: Write the failing tests** in `tests/writing/people_field.rs`, each driving `people_field` alone inside `egui::Context::run_ui` with `Tokens::light()` and a scripted `FieldInput` (a helper `draw(ctx, st, input, events: Vec<egui::Event>) -> Vec<PeopleEvent>` in the file):
  - `typing_opens_suggestions_and_the_keys_take_move_and_close`: two suggestions, `Event::Text("zo")` → `Changed`, `st.open`; ArrowDown → `highlighted == 1`; Enter → `Take(1)`; with the list open, Tab → `Take`; with no suggestions, Tab → no event (the focus moves on); Esc → `!st.open` and `dismissed_for == Some("zo")`.
  - `a_separator_closes_the_text_and_backspace_takes_a_chip_in_two_presses`: `Event::Text("ann;")` → `Closed("ann")` and `input.is_empty()`; with one chip and an empty input, Backspace → `selected == Some(0)`, Backspace again → `RemoveChip(0)`; a paste `Event::Paste("a@x.fr; John Doe <b@y.fr>")` → `Closed("a@x.fr; John Doe <b@y.fr>")`.
  - `chips_wrap_and_scroll_past_four_lines`: 60 chips in a 400 px wide ui → the field's rect is at most `4 * CHIP_ROW_H + margins` high (`pub const CHIP_ROW_H: f32`), and the list Area, opened by typing, sits at the input's bottom, not the last chip's.
  - `a_chip_is_a_named_widget_with_its_kind_and_a_refused_one_is_red`: a `Room` chip's `WidgetInfo` label is `"Salle Seine, Room, remove with Delete"`; a `Text { error: Some(Refused("This isn't a room")) }` piece draws a red stroke under it; `card == Some(0)` after a click on the chip, and the card offers "Expand" only for a list chip with `can_expand`.
  - `text_of_round_trips_check_names_output`: `text_of(&[Chip(Zoe), Text("nobody here")], "") == "Zoe Park <zoe@studio.example>; nobody here; "`, `text_of(&[], "dup") == "dup"`, `text_of(&[Chip(group)], "") == "Dupont family <group:AAMk=>; "`.
- [ ] **Step 2:** Run `T cargo test -p quatrelle-ui-egui --lib people_field` — compile errors.
- [ ] **Step 3:** Fetch the seven SVGs, add the `Icon`s, their rows and drawings; implement the widget in `people_field.rs` and its three pieces; add the strings to the 13 catalogs.
- [ ] **Step 4:** Run the tests, `T cargo xtask assets`, `T cargo test -p quatrelle-ui-egui --lib i18n` — pass.
- [ ] **Step 5:** Commit: `Look: the contacts field draws chips with their kinds, a suggestion list and a card`.

### Task 8: The field in the message and in Select Names, and Send that waits — `fable`, effort `xhigh` (integration across the engine, two dialogs, the send path and nine tests, with the owner's mailbox in mind)

**Files:**
- Create: `crates/ui-egui/src/dialogs/people_field.rs` (`pub(crate) mod people_field;` in `crates/ui-egui/src/dialogs.rs:13-23`; `dialogs/` has 11 files)
- Modify: `crates/ui-egui/src/lib.rs:372-384` (the six `suggest_*` fields go; `pub(crate) people: dialogs::people_field::PeopleFields` comes; `:539-548` init), `:668` (`run`: the send gate), `crates/ui-egui/src/app/frame.rs:12` (`logic`: `send_when_resolved`) and `:487-489` (`prepare` the dialog's three fields beside `f.search.refresh`)
- Modify: `crates/ui-egui/src/reading/compose.rs` (`recipient_field` :156-324 and `address_check`'s recipient use go; `:364-401` the suggestions block goes; `:558-583` To/Cc/Bcc and Replies to drawn by `PeopleFields`; `:535-545` the Ctrl+K guard stays, the choice dialog drawn by `PeopleFields::choice_dialog`), `crates/ui-egui/src/reading/check_names.rs` (`choice_dialog` takes the field and runs `people.choose` with `window` and `field`; `pub(crate)`), `crates/ui-egui/src/dialogs/names.rs:235-257` (the three rows drawn by `PeopleFields`; `field_row` stays for the attendees dialog), `crates/ui-egui/src/rich_edit/ribbon.rs:347` (Check Names → `compose.checkNames` unchanged)
- Modify tests: `crates/ui-egui/src/tests/writing.rs` (`recipient_boxes_suggest_people` :45 and `a_suggestion_is_taken_out_with_its_cross` :88 move to `tests/writing/people_field.rs`, rewritten for chips: `;` instead of `, ` in the text, `a.ui.people.open_field() == Some(FieldId { window: id, name: "to" })` instead of `a.ui.suggest_field == Some("to")`, the × of the first row found by `settled_text_at` on the suggestion's name); `a_name_typed_into_to_never_reads_a_big_mailbox_in_one_frame`, `the_address_book_adds_people_to_to_cc_and_bcc`, `ctrl_k_checks_the_names_typed`, `check_names_asks_which_person_a_name_means`, `several_messages_are_written_at_once`, `from_offers_shared_mailboxes_and_another_address`, `from_finds_another_address_by_its_name`, `tests.rs::tour` keep passing as written
- Test: `crates/ui-egui/src/tests/writing/people_field.rs`

**Interfaces:**
- Consumes: Tasks 4–6's `people_suggest`, `people_resolve`, `take_people_outcomes`, `people_pending`, `name_choice_for`, `people_expand`, `expansion_now`, `addressee_kind`, `directory_known`, `room_name`; Task 7's `people_field`, `FieldState`, `Piece`, `PeopleEvent`, `text_of`; Task 1's `split_entries`, `parse_group_ref`, `MAX_ENTRIES`.
- Produces in `dialogs/people_field.rs`:
  - `#[derive(Default)] pub(crate) struct PeopleFields { fields: HashMap<FieldId, FieldState>, open: Option<FieldId>, send_wait: Option<SendWait> }`; `struct SendWait { compose: u64, params: Value, since: f64 }`; `pub const SEND_WAIT_SECS: f64 = 60.0;` (`IO_TIMEOUT`).
  - `pub(crate) fn prepare(&mut self, s: &mut Session, field: FieldId, account: Option<&str>, accepts: &Accepts, text: &str)`: when `text != state.synced`, parse it once (`pieces_of`), `syncs += 1`; take the field's outcomes (`Found` → the piece becomes a chip, `NotFound`/`NotAnAddress`/`Unreached` → the piece's error, `Ambiguous` → nothing, the choice dialog shows it); when the field is `open`, ask `people_suggest` for the input (the engine's cache makes this free on an unchanged frame).
  - `pub(crate) fn draw(&mut self, ui: &mut Ui, t: &Tokens, field: FieldId, account: Option<&str>, accepts: &Accepts, label: &str, hint: &str, text: &mut String, focus: bool, can_expand: bool, acts: &mut Vec<Action>) -> bool` (the book button pressed): draws `people_field`, applies its events (`Take`, `Closed` through `pieces_of` with the 500 cap and `left_out`, `RemoveChip`, `Forget` → `Action::Run("contacts.forget", ..)`, `Expand(i)` → `Action::Run("people.expand", ..)` and, once `expansion_now` is `Ready`, the chip replaced by its members, `Resolve` → `Action::Run("people.resolve", {window, field, text?})`), then writes `*text = text_of(..)` and `synced` when anything changed.
  - `pub(crate) fn pieces_of(s: &Session, account: Option<&str>, text: &str) -> Vec<Piece>`: `split_entries`, each entry `parse_one` → `Chip(Addressee { kind: addressee_kind(..), name: the known name when the entry has none })`, a `<group:…>` → `Chip(ContactGroup)` with the index's name, else `Text`; cut to `MAX_ENTRIES`.
  - `pub(crate) fn choice_dialog(&self, ctx: &egui::Context, t: &Tokens, s: &Session, window: u64, acts: &mut Vec<Action>)`: `check_names::choice_dialog` for `name_choice(window)`.
  - `pub(crate) fn open_field(&self) -> Option<FieldId>` and `pub(crate) fn pieces(&self, field: FieldId) -> &[Piece]` (the tests read them).
  - `pub(crate) fn gate_send(&mut self, s: &mut Session, params: &Value, now: f64) -> bool` (called by `MailApp::run` for `compose.send`, with `now` = `self.ui.close_checked.unwrap_or(0.0)`, the last frame's `input.time`; `logic` passes `ctx.input(|i| i.time)`): the current message's four fields are `people_resolve`d; a `ContactGroup` chip is `people_expand`ed; when nothing is pending and no piece has an error, returns `false` (send now); else keeps `SendWait` and returns `true`. `pub(crate) fn send_when_resolved(&mut self, s: &mut Session, now: f64) -> Result<Option<Value>, String>` (called from `logic` each frame): `Ok(Some(params))` once every pending name is settled and every group expanded with no error; `Err(message)` on a `NotFound` (`tf!("Not found: {name}")`), `NotAnAddress`, `Unreached`, or past `SEND_WAIT_SECS` ("The directory can't be reached") — `MailApp` puts it in `compose_alert`. A `NameChoice` pending leaves the wait running (the dialog settles it).
- Field ids: the message `c.id` with `"to"`, `"cc"`, `"bcc"`, `"replyTo"`; the Select Names dialog opened for message `c.id` with `"book-to"`, `"book-cc"`, `"book-bcc"` (`NamesForm.to/cc/bcc` stay `String`s; `Action::Submit("compose.update", ..)` unchanged).
- `Accepts::RECIPIENTS` for all seven fields; `can_expand = s.directory_account(Some(&c.account)).is_some()`.

- [ ] **Step 1: Write the failing tests** in `tests/writing/people_field.rs` (the app harness of `tests.rs`: `app`, `app_big`, `frame`, `key`, `settled_text_at`, `click_at`; `with_exchange` and `directory_answers` of `tests/writing.rs` made `pub(super)`):
  - `recipient_boxes_suggest_people` (moved, rewritten): `Event::Text("zo")` → `to == "zo"`, `open_field()` is the To field; Enter → `to == "Zoe Park <zoe@studio.example>; "`; a click on the next suggestion's row keeps the cursor (`Event::Text("x")` lands in the input); Esc hides the list and Enter then leaves `to` ending in `"; sh"`.
  - `a_suggestion_is_taken_out_with_its_cross` (moved, rewritten): the × on the first row, then `Delete` on the highlighted one; asserts on `suggest_contacts_now` as today, and `open_field()` instead of `suggest_field`.
  - `text_fed_in_becomes_chips_with_their_kinds`: `compose.update {"to": "\"Dupont, Marc\" <m@contoso.example>; Dupont, Anne <a@contoso.example>; room.seine@contoso.example; nobody here"}` after an `Event::Rooms` with Salle Seine on the Exchange account → three chips (kinds `Unknown`, `Unknown`, `Room`) and one `Text`, read from `a.ui.people` through `pub(crate) fn pieces(&self, field) -> &[Piece]`; `to` unchanged except normalised to `"Dupont, Marc" <m@…>; "Dupont, Anne" <a@…>; Salle Seine <room.seine@…>; nobody here; `.
  - `a_field_parses_its_text_once_per_change_not_per_frame`: a message with 300 recipients fed in; 20 idle frames → `syncs == 1` and `people_suggest_passes().0` unchanged; one keystroke → `syncs` unchanged (the input isn't the text) and passes `+1`.
  - `pasting_a_list_gives_chips_and_keeps_the_first_500`: `Event::Paste` of 503 addresses → 500 chips, `left_out == Some(3)`, the notice "and 3 more were left out" drawn; a paste of `a@x.fr; John Doe <b@y.fr>` → two chips, the second named John Doe.
  - `ctrl_k_in_the_field_resolves_and_marks_what_it_cant`: on the Exchange account, `"Zoe Park; a@; dupont"` then Ctrl+K → Zoe a chip, `a@` red "Not an address: a@", `dupont` pending; `directory_answers(.., "dupont", &[])` → `dupont` red "Not found: dupont"; an answer with `error: Some("offline")` for a second name → "The directory can't be reached" tooltip; Ctrl+K with the focus in the body still inserts a link (`ctrl_k_checks_the_names_typed`'s second half, kept there).
  - `sending_waits_for_a_pending_name_then_stops_on_one_not_found`: `to = "zoe@studio.example; dupont"` on the Exchange account; `a.run("compose.send", json!({}))` → nothing sent (`a.session.compose.as_ref().unwrap().sending == false`, `take_jobs()` has no `Job::Send`), `directory_searching(&gal, "dupont")`; `directory_answers(.., "dupont", &[("Marc Dupont", "marc@contoso.example")])` + a frame → the message goes (`Job::Send` taken, `to == "Zoe Park <zoe@studio.example>; Marc Dupont <marc@contoso.example>; "`); a second message to `"nobody"` answered empty → `compose_alert == Some("Not found: nobody")`, no `Job::Send`; a third to `"zoe@studio.example"` alone with the account offline → sent without asking the directory.
  - `sending_gives_up_after_the_directorys_timeout`: a name never answered; frames with `ctx.input.time` moved past 60 s (`RawInput.time = Some(..)` through a `frame_at` helper in this file) → `compose_alert == Some("The directory can't be reached")`, no `Job::Send`.
  - `a_contact_group_is_expanded_before_the_message_goes`: `to = "Dupont family <group:AAMk=>"` → `compose.send` pushes `Job::ExpandList { item_id: Some("AAMk=") }`; `Event::ListExpanded` with two members → the chip replaced by two, then `Job::Send` with both addresses.
  - `the_select_names_rows_are_the_field`: `Dialog::select_names(&c, "to")`, `f.search.query = "zoe"`, a double-click on Zoe's row → `f.to == "Zoe Park <zoe@studio.example>; "`; `Event::Text("ann")` in the To row → suggestions drawn (`settled_text_at` on "Recent"); OK → `compose.update` with that text (`the_address_book_adds_people_to_to_cc_and_bcc` keeps its direct `f.to` assignment).
  - `two_messages_keep_their_own_fields`: two `mail.new`, `compose.update` on each, Ctrl+K on the second → only the second's `to` rewritten and only its outcomes taken (`FieldId.window` differs).
- [ ] **Step 2:** Run `T cargo test -p quatrelle-ui-egui --lib people_field` — compile errors.
- [ ] **Step 3:** Implement the glue, rewire `compose.rs`, `names.rs`, `lib.rs`, `frame.rs`; delete `recipient_field` and the `suggest_*` state; add the Review Focus strings' catalog rows missing from Task 7.
- [ ] **Step 4:** Run `T cargo test -p quatrelle-ui-egui --lib` (the moved tests gone from `writing.rs`, the rest unchanged), then the full gates of Global Constraints. Then look at it: `QUATRELLE_SCREENSHOT=out.png T cargo run -p quatrelle -- --sample --cmd mail.new` and read the PNG (chips, the list under the input, light and dark with `--dark`).
- [ ] **Step 5:** Commit in two: `Message: To, Cc, Bcc and Replies to are the contacts field, and Select Names' rows with them` and `Message: Send waits for the names still being resolved, and stops on one not found`.

---

## What deliveries 2 and 3 consume (so delivery 1 doesn't paint them in)

- `Accepts` (Task 1) takes new constants; `FIELD_NAMES` (Task 4) takes the meeting's, From's, the dialogs' and the rules' names; `FieldId.window` is the event window's number for meetings and the dialog's compose or window id elsewhere.
- `addressee_kind` and `room_name` (Task 3) back the room move into `EventForm.rooms`, "This isn't a room", and the commands' best-effort refusals.
- `PeopleFields::prepare`/`draw` (Task 8) take any `Accepts` and `text: &mut String`; a place that hands typed entries (Rooms) or addresses alone (rules) reads `pieces(field)` instead of the text.
- `PieceError::Refused(reason)` (Task 7) carries the per-place refusals; `"Matched as typed"` is already a string for rules.
- `people_resolve` with `text: Some(..)` (Task 5) serves fields whose text lives in the UI (meetings, dialogs, rules).

## Self-review notes

- Spec coverage: every "Engine" and "fake EWS" test of the spec's Testing section has a task above; of the "Headless UI tests", those for the meeting, From, Add calendar, rules and Search Folders are deliveries 2–3 by the spec's own Delivery section; the rest are in Tasks 7–8.
- Resolved ambiguities (reported to the owner with the plan): a contact group's textual form in a text-backed field (`<group:ItemId>`); `contacts.suggest` keeps its JSON (an alias in behaviour, not in shape); `check_names` covers Reply-To too; the quiet daily refresh never blanks a `Ready` Room Finder, and a failure is retried after 10 minutes; the start-up load waits for the account to be `Online`/`Syncing`; the shared mailboxes opened are suggested under "Recent"; `Event::ListExpanded` carries `total` beside `more`.
