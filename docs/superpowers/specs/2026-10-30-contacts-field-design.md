# One contacts field for every place a person is typed

Status: approved in conversation on 2026-10-30, section by section; this document is the written
spec to review.

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
- **The meeting's Required, Optional and Resources lines** (`pim_dialogs/meeting.rs`, `event.rs`,
  the attendees dialog `attendees.rs`) are plain text. They have no suggestions and no Ctrl+K, and
  only Resources is validated. The Room Finder (`rooms.rs`) lists the account's room lists.
- **Open a shared calendar** (`dialogs/other.rs`, `Dialog::OpenSharedCalendar`) and **Open another
  mailbox** (`dialogs/accounts.rs`) take one address, accepted once it contains `@`. They have no
  suggestions and no Ctrl+K, although the ribbon's entry says "From address book…".
- **Rules' conditions** (`dialogs/organize/rule_form.rs`) and **Search Folders' "From these
  people"** (`dialogs/organize.rs`) are plain text.

Three error strings say "not an address" differently. Check Names' token splitter cuts a display
name such as `Doe, John <j@x>` at its comma. In the message body, Ctrl+K inserts a link, as in
Outlook, and must keep doing so.

## Owner's decisions (2026-10-30)

- **The model is Outlook's To field, the same everywhere**: suggestions as one types, Ctrl+K checks
  names, recognised entries become chips, and a button opens the address book.
- **Everything Exchange knows as an addressee**: people, distribution lists, contact groups,
  rooms, equipment, shared mailboxes, mail-enabled public folders and Microsoft 365 groups. Each
  kind has its icon. Each place says what it accepts: it doesn't get its own search.
- **Where:** every place listed above, except the junk-senders list, which also takes `@domain`
  and isn't a contact lookup. The future Teams "New conversation" is to use the same field.
- **Approach 1:** one widget in the UI and one search in the engine. The field still hands the
  same `Name <address>; …` text to the existing commands, so commands, saved rules and tests keep
  working.
- **Typing never asks the server.** Suggestions come only from what is already on the computer:
  the local address book, the directory answers already cached, and the room lists already
  loaded. The Exchange directory is asked on Ctrl+K, and for names still unresolved when the field
  is left or the item is sent. Its answers join the cache, so the next keystrokes suggest them.
  "From: other address" loses its as-you-type directory search to follow this rule.
- **"Add calendar" takes several entries**, as Outlook's does: one action opens several colleagues'
  calendars.

## What the owner sees

### The field

- **Chips.** Each recognised entry is a chip: an icon for its kind, and its name.
  - Hovering shows the address. A click opens a small card with the name, address and kind, and
    "Remove".
  - A distribution list's card also offers "Expand", which replaces the chip with chips for its
    members.
  - Text not yet recognised stays plain text beside the chips.
- **Suggestions** from the first character, at most 8. Each row has the icon, name, address and
  kind, and where it comes from: "Recent", "Contacts", "Directory (cached)" or "Room list".
  - Ordering: an exact match first, then a prefix of the first or last name or the address, then
    the most used and most recent. Within that, the kinds the place expects first (rooms first in
    Rooms).
  - Only kinds the place accepts are suggested.
- **Keyboard, the same everywhere:**
  - ↑/↓ move in the list, and Enter or Tab takes the highlighted suggestion;
  - `;` or `,` closes what is typed: it becomes a chip when it resolves locally, else stays text;
  - Backspace in an empty input selects the last chip, and a second Backspace removes it;
  - Delete on a "Recent" suggestion forgets it (`contacts.forget`), and Esc closes the list;
  - Enter with the list closed does what the place's dialog does with Enter, unchanged.
- **Ctrl+K** in the field resolves the remaining text, entry by entry, through the engine:
  - one match becomes a chip;
  - several matches open the existing choice dialog (`check_names::choice_dialog`), now for any
    field;
  - none leaves the text with a red wavy underline and the single message "Not found: {name}"
    (one i18n string replacing the three);
  - the directory being unreachable or offline leaves the text, with "The directory can't be
    reached" instead of "Not found".

  The same resolution runs when the field loses focus and, for a message or a meeting, when it is
  sent. In the message body, Ctrl+K keeps inserting a link.
- **The book button** to the left of a field ("To…", "Required…", "Rooms…") opens the address book
  dialog for that field, filtered to the kinds it accepts. The dialog's own rows are this field.
- **Single-entry fields** ("From: other address", "Open another mailbox") hold one chip; another
  entry replaces it.
- **Copy and paste:**
  - pasting `a@x.fr; John Doe <b@y.fr>` gives two chips (resolved locally when possible, else
    chips for the bare addresses);
  - copying chips gives the same text back;
  - a quoted or comma-containing display name (`"Doe, John" <j@x>` or `Doe, John <j@x>`) stays
    one entry.
- **A refused kind** (pasted, typed or picked in the book) becomes a red chip that says why. It
  blocks sending or OK until removed. Examples:
  - "A distribution list's calendar can't be opened";
  - "This isn't a room";
  - "Only people can be in a Teams conversation".
- Light and dark themes, the 13 catalogues, and screen-reader labels on chips ("John Doe, person,
  remove with Delete").

### What each place accepts

| Place | How many | Accepts | Special |
|---|---|---|---|
| Message To / Cc / Bcc / Reply-To | several | anything with an address: people, lists, contact groups, rooms, equipment, shared mailboxes, public folders, Microsoft 365 groups | — |
| From: other address | one | people, shared mailboxes, lists | the server decides at sending whether one may send as them |
| Meeting Required / Optional | several | people, lists, contact groups, Microsoft 365 groups | a room or equipment entered here moves to Rooms, with a notice ("{room} was moved to Rooms") |
| Meeting Rooms (Resources) | several | rooms, equipment (suggested first) | anything else: "This isn't a room" |
| Address book dialog rows | as the field it was opened for | same | — |
| Add calendar | several | people, rooms, equipment, shared mailboxes | lists and groups: "A distribution list's calendar can't be opened" |
| Open another mailbox | one | people, shared mailboxes, rooms | lists, groups, public folders refused |
| Rules: From / Sent to | several | anything with an address | **unresolved text stays valid** (the rule matches it in the name or address, as today, e.g. `@proginov.com` or `Dupont`); no red underline, tooltip "Matched as typed" |
| Search Folders: From these people | several | same as rules | same as rules |
| Teams: New conversation (later) | several | people only | its own source, the Microsoft 365 directory, added then |

The engine's commands check the same rules, so the CLI gets the same refusals. For example,
`calendar.openShared` refuses a distribution list.

## The engine's search

- **One service, two uses:**
  - **suggest**, local only, synchronous and bounded per frame;
  - **resolve**, which may ask the directory, asynchronously.
  Both return typed entries: `name`, `address`, `kind`, `source`.
- **Kinds:** person, distribution list, contact group, room, equipment, shared mailbox, public
  folder, Microsoft 365 group, unknown (an address only).
  - The directory gives them from EWS `ResolveNames`' `MailboxType`:
    - `Mailbox` → person (or room, equipment or shared mailbox, see below);
    - `PublicDL` → distribution list;
    - `PrivateDL` → contact group;
    - `PublicFolder` → public folder;
    - `GroupMailbox` → Microsoft 365 group;
    - `Contact` / `OneOff` → person.
  - Rooms are recognised by the room lists 4L already loads for the Room Finder (`calendar.rooms`).
  - Shared mailboxes are recognised by those the user has opened.
  - Equipment is recognised from the room lists' resources when the server says so. Whether
    `ResolveNames` itself tells rooms from people on the owner's server is checked in delivery 1.
- **Sources:**
  1. **The local address book** (`session/mailboxes/contacts.rs`): contacts plus everyone written to
     or heard from. It is instant and is already built a slice per frame.
  2. **The directory cache** (`session/mailboxes/directory.rs`): the answers already received (up to
     300 queries kept, as today), now with each entry's kind. Suggestions read it; they never fill
     it.
  3. **The room lists** already loaded.
  4. **The shared mailboxes** already opened.
- **Resolve:**
  - Local matches first. Tokens with `@` are resolved to their known name and kind when known,
    else kept as an address of unknown kind.
  - Names with no single local match are asked of the directory (`Job::SearchDirectory`, EWS
    `ResolveNames`, as today), on Exchange accounts only.
  - The answer comes back by the existing event and settles the field's pending entries.
  - This generalises today's Check Names (`session/writing/names.rs`, `compose.checkNames` /
    `compose.chooseName`) from the compose window to any field. The two command ids stay as thin
    aliases for compatibility.
- **Expand a list:** a new `Job::ExpandList { account, address }` using EWS `ExpandDL`, answered by
  `Event::ListExpanded { address, members }`. Members are typed like any entry. A nested list stays
  a list chip, expandable in turn.
- **Ordering** as in "Suggestions" above. Use and recency come from the address book's counts.
- **Bounded work:** suggestion and resolution never scan the whole book in one frame; the lesson of
  the search bug of 2026-10-30. A test with a 50,000-entry book shows flat per-keystroke cost.
- **Privacy:** the log never carries a name, an address or a query, only counts. Directory queries
  go only to the user's own Exchange server, as today.
- Gmail, IMAP and Microsoft 365 (Graph) accounts have the local sources only. A Microsoft 365
  directory comes with Teams, or sooner if the owner asks.

## Where the code goes

Folder and file limits (`cargo xtask sizes`: 16 `.rs` per folder, 1000 lines per file, split
before ~800) shape this. The plan confirms each count; the spec fixes the layout:
- **Model** (pure): the entry kinds, `Accepts` (the set of kinds plus how many), and the
  address-list parser fix, beside `crates/model/src/addr/` (a new `addr/entries.rs`).
- **Engine:**
  - `crates/engine/src/session/mailboxes/people.rs` for the service: suggest, resolve, the per-field
    pending state, refusals;
  - the commands `people.suggest`, `people.resolve`, `people.choose`, `people.expand` in
    `crates/engine/src/cmd/`;
  - the existing compose ones become aliases.
  `session/` itself is full, so nothing new goes there directly.
- **Net:** `ResolveNames` keeps the `MailboxType` (`crates/net/src/exchange/ews/directory.rs`), and
  `ExpandDL` joins it. The fake EWS server answers both with kinds.
- **UI:** the widget in `crates/ui-egui/src/look/people_field.rs` (+ `look/people_field/` for the
  suggestion list, chips and the card). It replaces:
  - `reading/compose.rs`'s `recipient_field`;
  - the attendees and resources lines;
  - the rows of `dialogs/names.rs`;
  - the boxes of `OpenSharedCalendar`, `OpenSharedMailbox`, `rule_form` and the Search Folder
    dialog.
  The per-field state leaves `lib.rs` (916 lines) for a `PeopleFields` map keyed by field id in the
  widget's module.
- **Tests:** `crates/ui-egui/src/tests/` is full, so the widget's tests go in a subfolder of an
  existing test file (e.g. `tests/writing/people_field.rs`). The engine's go in
  `crates/engine/src/tests/writing/`.

## Errors

- The directory is unreachable or offline: local suggestions only. Ctrl+K leaves text with "The
  directory can't be reached". Nothing blocks writing a draft.
- An ambiguous name at sending: the choice dialog opens before the message leaves, as Outlook
  does.
- A list that can't be expanded (no permission, too large, server error) shows a notice, and the
  chip stays.
- The directory's answer is hostile input, as now: caps on entries per answer (100 plus `more`),
  name and address lengths, and kinds outside the list read as unknown.

## Testing

- **Headless UI tests of the widget:**
  - typing and suggestions;
  - each key above;
  - chips, the card and Expand;
  - copy and paste, including `Doe, John <j@x>`;
  - Ctrl+K with one, several or no match, and offline;
  - a refused kind;
  - single-entry replacement;
  - a room moved from Required to Rooms;
  - Ctrl+K in the message body still inserting a link.
- **Engine:**
  - ordering;
  - kinds from each `MailboxType`;
  - rooms recognised from the room lists;
  - expanding a list (and a nested one);
  - the cache feeding suggestions and never being filled by them;
  - the commands' refusals;
  - the 50,000-entry book with flat per-keystroke cost;
  - `every_command_survives_junk_params` for the new commands.
- **The fake EWS server** returns `MailboxType`s and answers `ExpandDL`.
- **The existing tests keep passing** (the field still produces the same text): those of
  `tests/writing.rs` (suggestions, cross, address book, Ctrl+K, choice, From),
  `tests/scheduling.rs`, `tests/meetings.rs`, `tests/organize.rs`, `tests/reading.rs`'s shared
  calendar, and the engine's Check Names tests. A test changes only where its text was built
  around the old plain box, and the plan names each one.

## Delivery

Three test builds, each usable on its own:
1. The engine's search (suggest, resolve, kinds, expand), the parser fix, and the field in the
   message (To/Cc/Bcc/Reply-To) and in the address book dialog.
2. Meetings: Required, Optional and Rooms (the room move), the attendees dialog, then "From: other
   address".
3. Add calendar (several entries), Open another mailbox, rules and Search Folders.

## Out of scope

- A Microsoft 365 (Graph) directory, LDAP, CardDAV directory search.
- Editing a contact from the chip's card; it opens nothing beyond the card.
- Delegates and sharing permissions (no UI exists for them).
- The junk-senders list.

## Risks

- Whether the owner's Exchange `ResolveNames` tells rooms and equipment from people. If not, the
  room lists alone recognise them, so a room missing from every room list reads as a person until
  added to one. Checked in delivery 1.
- Large lists: `ExpandDL` on a list of thousands is capped (500 members shown, with "and N more").
- Several fields' state moves out of `lib.rs` and the compose module. The per-field id must stay
  stable across frames and windows (several compose windows open at once).
