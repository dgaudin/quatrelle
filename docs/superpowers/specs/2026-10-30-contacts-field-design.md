# One contacts field for every place a person is typed

Status: approved in conversation on 2026-10-30, section by section; revised the same day to take
in an independent review (code references and EWS documentation checked). This document is the
written spec.

## Why

The owner found that Ctrl+K ("Check Names") does nothing when adding a calendar. He then asked for
the whole thing to be reworked: every place where a person, a list or a room is typed should use
one widget, called the same way everywhere.

A survey of the code (2026-10-30) found nine such places, built five different ways:
- **Compose To/Cc/Bcc** (`ui-egui/src/reading/compose.rs`, `recipient_field`, private).
  - It suggests from the local address book as one types.
  - Ctrl+K runs `compose.checkNames`, and several matches open the choice dialog
    (`reading/check_names.rs`).
  - There are no chips. The box is plain text checked after it loses focus.
- **Compose "From: other address"** (`reading/sender.rs`).
  - It searches the Exchange directory from 3 characters as one types.
  - It has its own Ctrl+K, which writes a single candidate and offers no choice.
- **The address book dialog** (`dialogs/names.rs`, "Select Names") searches the books and the
  directory. Its To/Cc/Bcc rows are plain text with no Ctrl+K.
- **The meeting's Required and Optional lines** (`dialogs/pim_dialogs/meeting.rs`, `event.rs`)
  are plain text, with no suggestions and no Ctrl+K. They are validated only when the item is
  saved (`event.create` refuses "attendees: not an address"), not in the form.
  - The meeting's **Rooms** are already typed chips: `EventForm.rooms: Vec<(String, String)>`,
    drawn with × and fed by the Room Finder (`dialogs/pim_dialogs/rooms.rs`), which lists the
    account's room lists.
  - The attendees dialog (`dialogs/pim_dialogs/attendees.rs`, "Select Attendees and Resources")
    has Required, Optional and Resources rows as plain text; it checks Resources' addresses
    itself. Its rows are the Select Names dialog's (`names::field_row`).
- **Open a shared calendar** (`dialogs/other.rs`, `Dialog::OpenSharedCalendar`, command
  `calendar.openShared`) and **Open another mailbox** (`dialogs/accounts.rs`, command
  `account.openMailbox`) take one address, accepted once it contains `@`. They have no
  suggestions and no Ctrl+K, although the ribbon's entry says "From address book…".
- **Rules' conditions** (`dialogs/organize/rule_form.rs`) and **Search Folders' "From these
  people"** (`dialogs/organize.rs`) are plain text, each split by its own code on `,` and `;`.

Three error strings say "not an address" differently. Check Names' token splitter
(`tokens()` in `session/writing/names.rs`) cuts a display name such as `Doe, John <j@x>` at its
comma. In the message body, Ctrl+K inserts a link, as in Outlook, and must keep doing so.

## Owner's decisions (2026-10-30)

- **The model is Outlook's To field, the same everywhere**: suggestions as one types, Ctrl+K checks
  names, recognised entries become chips, and a button opens the address book.
- **Everything Exchange knows as an addressee**: people, distribution lists, contact groups,
  rooms, equipment, shared mailboxes, mail-enabled public folders and Microsoft 365 groups. Each
  kind has its icon. Each place says what it accepts: it doesn't get its own search.
- **Where:** every place listed above, except the junk-senders list, which also takes `@domain`
  and isn't a contact lookup. The future Teams "New conversation" is to use the same field.
- **Approach 1:** one widget in the UI and one search in the engine. The field hands each place
  the same data it already uses, so commands, saved rules and tests keep working:
  - text (`Name <address>; …`) for the message's fields and the meeting's attendees;
  - typed `(name, address)` entries for the meeting's Rooms (`EventForm.rooms`);
  - one address per command for Add calendar and Open another mailbox;
  - addresses alone, plus the unresolved text unchanged, for rules and Search Folders.
- **Typing never asks the server.** Suggestions come only from what is already on the computer:
  the local address book, the directory answers already cached, and the room lists already
  loaded. The Exchange directory is asked on Ctrl+K, and for names still unresolved when the field
  is left or the item is sent. Its answers join the cache, so the next keystrokes suggest them.
  "From: other address" loses its as-you-type directory search to follow this rule.
  - **The one exception is the address book dialog.** With the "Global Address List" book
    selected, it keeps searching the server as one types, debounced, as today. It is the explicit
    way to browse the directory. As the owner was told on 2026-10-30: "pour parcourir l'annuaire
    sans connaître le nom exact, il reste le bouton du carnet d'adresses".
- **"Add calendar" takes several entries**, as Outlook's does: one action opens several colleagues'
  calendars.

## What the owner sees

### The field

- **Chips.** Each recognised entry is a chip: an icon for its kind, and its name.
  - Hovering shows the address. A click opens a small card with the name, address and kind, and
    "Remove". One card or hover is shown at a time.
  - A distribution list's or contact group's card also offers "Expand", which replaces the chip
    with chips for its members. "Expand" is offered only when the account has a directory (an
    Exchange account); elsewhere it would ask a server that never answers.
  - Text not yet recognised stays plain text beside the chips.
- **Layout.** Chips wrap to new lines. The field grows up to 4 lines, then scrolls inside itself
  (as `widgets::chip_grid` does for attachments), so a long To never pushes the body off the
  screen. The suggestion list anchors below the input, not below the last line of chips.
- **The input** beside the chips is egui's `TextEdit`: IME composition, undo of the text and
  right-to-left names come from it as egui gives them. Making a chip and removing one are not in
  its undo. A chip's name is drawn through `look/bidi.rs`, as the list draws a subject.
- **Suggestions** from the first character, at most 8. Each row has the icon, name, address and
  kind, and where it comes from: "Recent", "Contacts", "Directory (cached)" or "Room list".
  - Ordering: an exact match first, then a prefix of the first or last name or the address, then
    the most used and most recent. Within that, the kinds the place expects first (rooms first in
    Rooms).
  - Only kinds the place accepts are suggested.
- **Keyboard, the same everywhere:**
  - ↑/↓ move in the list. Enter takes the highlighted suggestion. Tab takes it when the list is
    open and moves to the next field when the list is closed, as `recipient_field` does today;
  - `;` or `,` closes what is typed: it becomes a chip when it resolves locally, else stays text;
  - Backspace in an empty input selects the last chip, and a second Backspace removes it;
  - Delete on a "Recent" suggestion forgets it (`contacts.forget`), and Esc closes the list;
  - Enter with the list closed does what the place's dialog does with Enter, unchanged.
- **Ctrl+K** in the field resolves the remaining text, entry by entry, through the engine:
  - one match becomes a chip;
  - several matches open the choice dialog (`check_names::choice_dialog`), now for any field;
  - a piece with `@` that isn't a valid address is underlined in red with "Not an address:
    {what}" (one i18n string replacing the three that exist today);
  - a name that nothing matches is underlined in red with "Not found: {name}";
  - the directory being unreachable or offline leaves the text, with "The directory can't be
    reached" instead of "Not found".

  The same resolution runs when the field loses focus and when the item is sent (see "Sending").
  In the message body, Ctrl+K keeps inserting a link: the field handles Ctrl+K only when it has
  the focus, and the compose window keeps its guard that the body and the "other sender" box
  don't have it.
- **Sending.** "Sent" means two things:
  - for a message, the compose window's Send (`compose.send`);
  - for a meeting, its Send or Send update (`event.create` or `event.update` with attendees).

  Before the command runs, sending waits for the fields' pending resolutions, at most the
  directory's timeout (a request's 60 s, `IO_TIMEOUT` in `net/src/stream.rs`). Then any ambiguous
  name opens the choice dialog, and any name not found (or not reached) stops the send with its
  message. A contact group, which has no address, is expanded to its members before the command
  runs. Addresses never need the directory, so a message to addresses alone sends offline.
- **The book button** to the left of a field ("To…", "Required…", "Rooms…") opens the address book
  dialog for that field, filtered to the kinds it accepts. The dialog's own rows are this field.
- **Single-entry fields** ("From: other address", "Open another mailbox") hold one chip; another
  entry replaces it.
- **Copy and paste:**
  - pasting `a@x.fr; John Doe <b@y.fr>` gives two chips (resolved locally when possible, else
    chips for the bare addresses);
  - copying chips gives the same text back (a contact group gives its name);
  - a quoted or comma-containing display name (`"Doe, John" <j@x>` or `Doe, John <j@x>`) stays
    one entry. The rule: a comma-separated part that is not an address and has no `<…>` is
    joined to the part that follows when that one has `<…>`. So `Doe, John <j@x>` is one entry,
    and `doe@x, John <j@x>` two, since `doe@x` is an address. `a@x,b@y` (no space) is two.
  - a field holds at most 500 entries. A paste beyond that keeps the first 500 and says "and {n}
    more were left out".
- **Text fed in** (a draft reopened, a reply, `compose.update` from a script) turns each
  `Name <address>` into a chip at once. Its kind comes from the book, the directory cache or the
  room set, else it is unknown (the plain address icon). It isn't asked of the server again
  unless the user presses Ctrl+K.
- **A refused kind** (pasted, typed or picked in the book) becomes a red chip that says why. It
  blocks sending or OK until removed. Examples:
  - "A distribution list's calendar can't be opened";
  - "This isn't a room";
  - "Only people can be in a Teams conversation".
- Light and dark themes, and the 13 catalogues. Each chip is a named widget (`widgets::named`)
  for the screen reader: "John Doe, person, remove with Delete".

### What each place accepts

| Place | How many | Accepts | Hands on | Special |
|---|---|---|---|---|
| Message To / Cc / Bcc / Reply-To | several | anything that receives mail: people, lists, contact groups, rooms, equipment, shared mailboxes, public folders, Microsoft 365 groups | text | a contact group is expanded at sending |
| From: other address | one | people, shared mailboxes, lists | text | the server decides at sending whether one may send as them |
| Meeting Required / Optional | several | people, lists, contact groups, Microsoft 365 groups | text | a known room entered here moves to Rooms, with a notice ("{room} was moved to Rooms"); a contact group is expanded at sending |
| Meeting Rooms (Resources) | several | rooms (suggested first); any mailbox typed or pasted as a full address | `(name, address)` entries into `EventForm.rooms` | a list, group or public folder, or a person picked by name: "This isn't a room" |
| Address book dialog rows | as the field it was opened for | same | same | — |
| Add calendar | several | people, rooms, equipment, shared mailboxes | one `calendar.openShared` per entry | lists and groups: "A distribution list's calendar can't be opened" |
| Open another mailbox | one | people, shared mailboxes, rooms | text (`account.openMailbox`) | lists, groups, public folders refused |
| Rules: From / Sent to | one for "From contains" and "To or Cc contains"; several for "From one of these" and "To or Cc one of these" | anything with an address | each chip's address alone, and each unresolved piece unchanged | **unresolved text stays valid** (the rule matches it in the name or address, as today, e.g. `@proginov.com` or `Dupont`); no red underline, tooltip "Matched as typed" |
| Search Folders: From these people | several | same as rules | same as rules | same as rules |
| Teams: New conversation (later) | several | people only | — | its own source, the Microsoft 365 directory, added then |

**Rooms and equipment.** EWS doesn't tell a room or equipment from a person (see "Kinds"). A
mailbox typed as a full address in Rooms is therefore accepted even when the directory said
`Mailbox`, because only the user knows it is equipment outside any room list. A person picked by
name (a suggestion, Ctrl+K) is refused there.

**Rules and Search Folders.** A chip gives its address alone (`j@x.fr`), never `Name <address>`,
because the rule compares with `contains` against the sender's name and address. Unresolved
pieces are given as typed. The rule editor's and the Search Folder's own `,`/`;` splitting goes:
they take the field's list of pieces, and `final_query` quotes each one as today. Saved rules keep
their stored text form (`Condition::FromAny(Vec<String>)` and the others, unchanged). When a saved
rule or Search Folder is opened again, each stored piece becomes a chip if it is a known address
(the book, the directory cache or the room set), else it stays text.

**Add calendar.** The dialog runs one `calendar.openShared` per entry itself, not through
`Action::Submit`, which closes the dialog at the first success. Entries that open are removed from
the field as they open. An entry refused by the field or failed by the command stays, red, with the
reason. The dialog stays open while any entry is red, and closes when every entry has opened.

**The commands.** Commands keep their parameters, so the CLI and scripts keep working. Their
refusal of a kind is best effort: the engine refuses what the session knows, meaning an address
found in the directory cache, the room set or the shared mailboxes opened. For example,
`calendar.openShared` refuses an address the directory cache knows as a distribution list. An
address the session doesn't know goes through, and the server answers as today. The field
refuses before calling, so the UI never relies on the command's check.

## The engine's search

- **One service, two uses:**
  - **suggest**, local only and synchronous, run at most once per change of the typed text;
  - **resolve**, which may ask the directory, asynchronously.
  Both return typed entries, the model's `Addressee { name, address, kind, item_id }`, with the
  `source` they came from.
- **Kinds (`AddresseeKind`):** person, distribution list, contact group, room, equipment, shared
  mailbox, public folder, Microsoft 365 group, unknown (an address only).
  - The directory gives them from EWS `ResolveNames`' `MailboxType`:
    - `Mailbox` → person, unless the room set or the shared mailboxes say otherwise (below);
    - `PublicDL` → distribution list;
    - `PrivateDL` → contact group;
    - `PublicFolder` → public folder;
    - `GroupMailbox` → Microsoft 365 group;
    - `Contact` / `OneOff` → person.
  - **Rooms come from the room lists.** `ResolveNames` can't tell rooms or equipment from people:
    EWS's `MailboxType` has no room value, and `GetRooms` itself answers `Mailbox` for a room. So:
    - the engine loads each Exchange account's room lists at start-up and again once a day, with
      `GetRoomLists` and `GetRooms` per list, through the job the Room Finder uses
      (`Job::GetRooms`, answered by `Event::Rooms`). Today they load only when the Room Finder
      opens (`calendar.rooms`);
    - the session keeps them as a set of room addresses per account, rebuilt from each
      `Event::Rooms`. When the server has no room lists, the set holds the rooms the calendar
      knows, as the Room Finder does (`from_calendar`). A failed refresh keeps the last set;
    - rooms hidden from the address list aren't in any room list, and read as people.
  - **Equipment** is recognised only when it appears in a room list. Nothing in `GetRooms` tells
    it from a room there, so it takes the room's kind and icon. Outside every room list it reads
    as a person; the Rooms field still accepts it once it is typed as an address. The equipment
    kind and icon stay in the model for a source that says so; none in this spec does.
    `FindPeople` (whose `PersonaType` values are undocumented) is not used.
  - Shared mailboxes are recognised by those the user has opened.
- **Sources:**
  1. **The local address book** (`session/mailboxes/contacts.rs`): contacts plus everyone written to
     or heard from, at most 20,000 entries (`MAX_CONTACTS`). It is already built a slice per frame.
  2. **The directory cache** (`session/mailboxes/directory.rs`): the answers already received (up to
     300 queries kept, as today), now with each entry's kind. The cache is keyed by query, so it
     gains an index of its entries by address (by `ItemId` for a contact group, which has no
     address), kind included: at most 30,000 entries, cleared with the answers. Suggestions read
     the index; only the directory's answers write it, never suggestions.
  3. **The room set** above.
  4. **The shared mailboxes** already opened.
- **Resolve:**
  - Local matches first. Tokens with `@` are resolved to their known name and kind when known,
    else kept as an address of unknown kind. Only names (no `@`) go to the directory; addresses
    never do.
  - Names with no single local match are asked of the directory (`Job::SearchDirectory`, EWS
    `ResolveNames`), on Exchange accounts only, one at a time per account, in series, as today.
  - **The search scope changes** from `ActiveDirectory` to `ActiveDirectoryContacts` (the
    directory first, then the user's Contacts folder), so `Contact` and `PrivateDL` can come back.
    `ReturnFullContactData` stays `false`, as the code has it. A contact group keeps the
    `ItemId` the answer gives it, in its `Addressee`. `directory_certificates`, a separate call,
    keeps `ActiveDirectory`.
  - The answer comes back by the existing `Event::DirectoryPeople`, whose people gain their kind
    and item id, and settles the field's pending entries.
  - This generalises today's Check Names (`session/writing/names.rs`, `compose.checkNames` /
    `compose.chooseName`) from the compose window to any field. **What changes for Check Names**
    with the new scope:
    - a person in both the directory and the Contacts folder stays one match (the answer is
      already merged by address);
    - a contact of the Contacts folder that isn't in the directory can now be found, so a name
      that had one match may now have several, and open the choice dialog;
    - contact groups now come back, with their icon, where today they are dropped for having no
      address (`people_of`).

    Nothing else changes: local matches first, the same queue, the same `NAMES_NOT_FOUND` answer
    from the command.
- **The commands:** `people.suggest`, `people.resolve`, `people.choose`, `people.expand`. They take
  a `field` parameter, the field's key (below). The existing ones stay, their JSON kept:
  `compose.checkNames` and `compose.chooseName` become aliases of `people.resolve` and
  `people.choose` on the compose window's fields, and `contacts.suggest` an alias of
  `people.suggest`. `contacts.directory` stays the directory search it is (the address book's
  browse), its people gaining their kind.
- **The field's key** is `(window id, field name)`, as `FieldId { window: u64, name: &'static str }`:
  the compose window's id and `"to"`, the meeting window's and `"required"`, a dialog's and
  `"email"`. Pending entries and choices (`NameChoice { field: FieldId, … }`) are kept by it, so
  several compose windows don't mix. Today a single `UiState.suggest_field` and a choice keyed by
  compose id serve only the message on screen.
- **Expand a list:** a new `Job::ExpandList { account, address, item_id: Option<String> }` using
  EWS `ExpandDL`: with the `ItemId` for a contact group (`address` is then empty, it has none), and
  with the address for a distribution list. It is answered by
  `Event::ListExpanded { account, address, item_id, members, more, error }`, since the server's
  answer doesn't name the list. Members are typed like any entry.
  - EWS expands one list per call and not recursively: a nested list stays a list chip,
    expandable in turn.
  - A list the server answers empty or unknown (success, no member) shows "This list has no
    members".
  - At most 500 members are shown, with "and {n} more" (`DLExpansion`'s
    `IncludesLastItemInRange` and `TotalItemsInView`).
- **Ordering** as in "Suggestions" above. Use and recency come from the address book's counts.
- **Bounded work:** the lesson of the search bug of 2026-10-30. `ContactBook::suggest` is a linear
  pass over the book, so its cost is bounded by the book's cap (20,000, `MAX_CONTACTS`), not flat.
  The rule: at most one suggestion pass per change of the typed text, and none on frames where the
  text didn't change (the cache keyed by field, text and book revision, as `suggest_cache` does
  today). Raising the cap is out of scope.
- **Privacy:** the log never carries a name, an address or a query, only counts. Directory queries
  go only to the user's own Exchange server, as today.
- Gmail, IMAP and Microsoft 365 (Graph) accounts have the local sources only; on them, and in the
  web app without an Exchange account, resolve is local and immediate. A Microsoft 365 directory
  comes with Teams, or sooner if the owner asks.

## Where the code goes

Folder and file limits (`cargo xtask sizes`: 16 `.rs` per folder, 1000 lines per file, split
before ~800) shape this. The plan confirms each count; the spec fixes the layout:
- **Model** (pure): `Addressee`, `AddresseeKind`, `Accepts` (the set of kinds plus how many), and
  the address-list splitter, beside `crates/model/src/addr/` (a new `addr/entries.rs`).
  `addr::parse_list` and Check Names' `tokens()` share this one splitter.
- **Engine:**
  - `crates/engine/src/session/mailboxes/people.rs` for the service: suggest, resolve, the per-field
    pending state, the room set, refusals;
  - the commands `people.suggest`, `people.resolve`, `people.choose`, `people.expand` in
    `crates/engine/src/cmd/`;
  - the existing commands become aliases as above;
  - the daily room-list refresh beside `request_rooms` (`session/scheduling.rs`).
  `session/` itself is full, so nothing new goes there directly.
- **Net:** `ResolveNames` uses `ActiveDirectoryContacts` and keeps the `MailboxType` and, for a
  contact group, the `ItemId` (`crates/net/src/exchange/ews/directory.rs`). `ExpandDL` joins it.
  The fake EWS server answers both with kinds.
- **UI**, in two parts:
  - **the widget's drawing** in `crates/ui-egui/src/look/people_field.rs` (+ `look/people_field/`
    for the suggestion list, the chips and the card). It is pure UI like the rest of `look/`:
    given its pieces and the suggestions already computed, it draws them and returns events
    (`PeopleEvent::{Resolve, Expand, Forget, OpenBook}`, and the pieces changed). It never sees
    the `Session`;
  - **its session glue** with the dialogs, in `crates/ui-egui/src/dialogs/people_field.rs`: it
    asks the engine for suggestions, turns the widget's events into actions, keeps the
    `PeopleFields` map keyed by `FieldId`, and holds the one helper that draws a field's choice
    dialog. Each place draws the choice dialog for its own field through that helper; today only
    `compose.rs` draws it.

  Together they replace:
  - `reading/compose.rs`'s `recipient_field`;
  - the meeting's Required and Optional lines, and its Rooms row (which keeps `EventForm.rooms`);
  - the rows of `dialogs/names.rs`;
  - the boxes of `OpenSharedCalendar`, `OpenSharedMailbox`, `rule_form` and the Search Folder
    dialog.
  The per-field state leaves `lib.rs` (916 lines) for the glue's `PeopleFields`.
- **Icons:** up to eight new Fluent icons, one per kind, each taken from `@fluentui/svg-icons`
  with its row in `ATTRIBUTION.md` and a line drawing in `look/icons.rs` (`cargo xtask assets`).
  The plan lists them.
- **Tests:** `crates/ui-egui/src/tests/` is full, so the widget's tests go in a new subfolder of an
  existing test file: `tests/writing/people_field.rs`, declared by `mod people_field;` in
  `tests/writing.rs`. The engine's go in `crates/engine/src/tests/writing/people_field.rs`, a name
  apart from the existing `tests/people.rs`.

## Errors

- The directory is unreachable or offline: local suggestions only. Ctrl+K leaves text with "The
  directory can't be reached", from the cached failure (`DirectoryAnswer.error`, asked again after
  `RETRY_AFTER`, 60 s). Nothing blocks writing a draft.
- At sending: an ambiguous name opens the choice dialog before the item leaves, as Outlook does; a
  name not found, or not reached, stops the send with its message.
- A list that can't be expanded (no permission, too large, server error) shows a notice, and the
  chip stays.
- The directory's answer is hostile input, as now: caps on entries per answer (100 plus `more`),
  name, address and `ItemId` lengths, and kinds outside the list read as unknown.
- The caps: 500 entries per field, 500 members per expansion, 30,000 entries in the cache index;
  the directory queue asks one name at a time per account, as today.

## Testing

- **Headless UI tests of the widget:**
  - typing and suggestions;
  - each key above, including Tab with the list open and closed;
  - chips, the card and Expand (and no Expand without an Exchange account);
  - wrapping, the growth to 4 lines and the scroll past them;
  - copy and paste, including `Doe, John <j@x>`, `doe@x, John <j@x>` and `a@x,b@y`;
  - a paste of more than 500 entries;
  - text fed in (a draft reopened) turned into chips;
  - Ctrl+K with one, several or no match, a malformed address, and offline;
  - a refused kind;
  - single-entry replacement;
  - a room moved from Required to `EventForm.rooms`, and an equipment address typed in Rooms;
  - Add calendar with three entries, one refused: the dialog stays open with that one red, the
    other two gone;
  - a rule and a Search Folder with `Doe, John <j@x>` and `@proginov.com` side by side, getting
    `j@x` and `@proginov.com`; a saved rule reopened with a known address and a piece of text;
  - sending waiting for a pending name, then stopping on a name not found;
  - Ctrl+K in the message body still inserting a link;
  - a chip's screen-reader name.
- **Engine:**
  - ordering;
  - kinds from each `MailboxType`;
  - rooms recognised from the room set, loaded at start-up and refreshed a day later, without the
    Room Finder ever opened;
  - expanding a list by address, a contact group by `ItemId`, a nested list, and an empty one;
  - the cache index feeding suggestions and never being filled by them;
  - Check Names with the new scope: a person in both the directory and the Contacts folder found
    once, a contact group offered;
  - the commands' best-effort refusals: a known list refused by `calendar.openShared`, an unknown
    address let through;
  - bounded work: a book filled to its cap of 20,000 entries; a counter of suggestion passes and
    entries scanned shows one pass (at most the cap's entries) per change of text, and no pass on
    frames where the text didn't change;
  - `every_command_survives_junk_params` for the new commands.
- **The fake EWS server** returns `MailboxType`s, a contact group's `ItemId`, and answers
  `ExpandDL` by address and by `ItemId`.
- **The existing tests keep passing** (each place still gets the data it gets today): those of
  `tests/writing.rs` (suggestions, cross, address book, Ctrl+K, choice, From),
  `tests/scheduling.rs`, `tests/meetings.rs`, `tests/organize.rs`, `tests/reading.rs`'s shared
  calendar, and the engine's Check Names tests. A test changes only where its text was built
  around the old plain box, and the plan names each one.

## Delivery

Three test builds, each usable on its own:
1. The engine's search (suggest, resolve, kinds, the new search scope, the room set loaded at
   start-up and daily, expand by address and by `ItemId`), the splitter fix, and the field in the
   message (To/Cc/Bcc/Reply-To) and in the Select Names dialog. `names::field_row` stays for the
   attendees dialog until delivery 2.
2. Meetings: Required, Optional and Rooms (the room move into `EventForm.rooms`), the attendees
   dialog, sending a meeting, then "From: other address".
3. Add calendar (several entries, one command each), Open another mailbox, rules and Search
   Folders (addresses alone, their splitters gone, saved ones reopened as chips).

## Out of scope

- A Microsoft 365 (Graph) directory, LDAP, CardDAV directory search.
- `FindPeople` and its `PersonaType`, to tell rooms and equipment apart.
- Raising the address book's 20,000-entry cap.
- Editing a contact from the chip's card; it opens nothing beyond the card.
- Dragging chips between fields (To to Cc).
- A chip resolved earlier isn't asked of the server again at sending, even if the directory has
  changed since; the cache lives as long as Quatrelle runs, as today and as Outlook's own cache of
  names does.
- Delegates and sharing permissions (no UI exists for them).
- The junk-senders list.

## Risks

- Large lists: `ExpandDL` on a list of thousands is capped (500 members shown, with "and {n} more").
- The new search scope finds the Contacts folder's people too: Check Names may offer a choice
  where it used to settle on one person. The tests above pin what changes.
- Several fields' state moves out of `lib.rs` and the compose module. The `FieldId` must stay
  stable across frames and windows (several compose windows open at once).
