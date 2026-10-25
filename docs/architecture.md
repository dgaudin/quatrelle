# Architecture

```
            ┌──────────────── apps/quatrelle (eframe) ────────────────┐
            │  keychain · file dialogs · system font · clock           │
            │                                                          │
   input ──▶│  ui-egui (L6) ──commands──▶ engine::Session (L4)         │
            │      ▲  reads state              │  queues Jobs           │
            │      └──────────────── Events ◀──┼── net::Pool (L3) ◀── IMAP·SMTP·JMAP·Graph·EWS·DAV
            │                                  ▼        │               │
            │                          store (L1) ◀─────┘ writes cache  │
            └──────────────────────────────────────────────────────────┘

            ┌──────────── apps/quatrelle-web (eframe web runner) ─────────┐
            │  the same ui-egui and engine; store in memory ⇄ IndexedDB    │
            │  WebBackend: jobs run per account ── jmap (L2) over fetch ──▶ JMAP
            └──────────────────────────────────────────────────────────────┘
```

## Layers

| Layer | Crate | Depends on | Notes |
|---|---|---|---|
| L0 | `quatrelle-model` | — | Pure data + algorithms; `Job`/`Event` are defined here so `net` and `engine` agree without depending on each other |
| L1 | `quatrelle-mime` | model | `mail-parser`, `mail-builder`, `html2text` (formatted text), `html5ever` (the tokenizer, for table layouts) |
| L1 | `quatrelle-store` | model | Plain files + JSON; atomic writes; no database engine |
| L1 | `quatrelle-print` | — | Messages laid out as PDF pages with an embedded TrueType font (`ttf-parser`, `miniz_oxide`) |
| L2 | `quatrelle-jmap` | model, mime, store | The JMAP transport, written once as async code over a two-method `Http` trait: the desktop runs it over `net`'s client (answers are ready at once, so a no-op `block_on` drives it), the browser over `fetch`. Portable, wasm-safe |
| L3 | `quatrelle-net` | model, mime, store, jmap | Native only. IMAP (`imap-proto`), POP3, SMTP (`lettre`), JMAP push, Microsoft Graph, Exchange Web Services with NTLM, CalDAV/CardDAV (`quick-xml`), ManageSieve, a small HTTP/1.1 client, DNS for autoconfiguration, OAuth, Outlook data files (`outlook-pst` to read, `outlook-pst-rw` to write); TLS through `rustls` |
| L4 | `quatrelle-engine` | model, mime, store | Sans-I/O, wasm-safe |
| L6 | `quatrelle-ui-egui` | engine | egui only; platform services behind traits; interface languages (`i18n/*.tsv`) |

`cargo xtask layers` enforces this, including that `egui` stays in L6 and network crates stay in `net`.
`cargo xtask wasm` checks that every crate but `net`, and the web app, build for `wasm32`.

## Source layout

Modules use the 2018+ layout (`foo.rs` beside `foo/`, no `mod.rs`), split by topic, with every
file at most 1000 lines and every folder at most 16 Rust files (`cargo xtask sizes`, rules in
`CLAUDE.md` › File size). The big types are one `impl` continued across files:

| Where | What |
|---|---|
| `engine/src/session.rs` + `session/` | `Session`'s state; its `impl` continued per area (`opening`, `changing`, `writing`, `outbox`, `searching`, `accounts`, `events`, `prefs`…; `organize/` for junk, Quick Steps, Search Folders; `mailboxes/` for folder edits, server-side mailbox settings and shared mailboxes; `pim/` for people, notes, tasks and reminders; `writing/` for the message being written) |
| `engine/src/cmd.rs` + `cmd/` | the registry; each area (`home`, `navigation`, `compose`, `view`, `pim`, `accounts`) returns its commands |
| `ui-egui/src/app.rs` + `app/` | `MailApp`, the window: `frame`, `ribbon`, `bars`, `search_box`, `keys`, `actions`, `windows` |
| `ui-egui/src/panes/`, `dialogs/`, `reading/`, `rich_edit/`, `look/` | the panes, the dialogs by group, the reading pane and compose window, the editor, and theme/icons/motion/widgets |
| `net/src/protocols/`, `exchange/`, `data_files/`, `fakes/`, `worker/` | IMAP/SMTP/POP/JMAP/DAV/Sieve; Graph/EWS/NTLM; `.pst`; the in-memory test servers; the worker pool |

## Transports

An account syncs over one of four transports behind the same `Job`s and `Event`s, chosen by its
configuration: IMAP + SMTP (`imap`, `smtp`), JMAP (`jmap`, a session address), Microsoft Graph
(`graph`, Exchange Online) or Exchange Web Services (`ews`, on-premises Exchange). JMAP, Graph and
EWS number their string message ids the same way
(`jmap::State`), so the engine and cache only ever see IMAP-style keys. Calendars and address
books sync over CalDAV / CardDAV for password accounts, and automatic replies over ManageSieve on
IMAP hosts. An account with none of these (the sample mailbox, an opened Outlook data file) lives
on this computer only: the pool applies its changes to the cache directly. A POP account
(`pop`) is the same after its mail is downloaded: only the Inbox talks to the server.

Each account is one data file, `<address>.quatrelle`, in Documents\Quatrelle Files, as Outlook
keeps an account in `<address> - <name>.pst` in Documents\Outlook Files
(`crates/store/src/packed.rs`): a crash-safe single-file database (redb) holding the account's
cache, keyed by the paths the store always used. The account list and the app's own state stay
in the per-user app folder (`%APPDATA%\Quatrelle`). `.pst` is only read (an Outlook data file
opened gets a `.quatrelle` file of its own; Outlook's file isn't touched) and written (export).
Account folders from before are brought into data files when the store opens, and removed once
read back. The data files' folder can be moved (`crates/store/src/location.rs`): `location.json`
in the app folder points at it, and a move asked for (`move-pending.json`) is carried out at the
next start, before anything opens the files, every file checked before the old ones go. The
sample mailbox and the browser keep plain files and memory.

## The browser (`apps/quatrelle-web`)

The same `MailApp` runs in eframe's web runner. The store is `Store::memory()`, loaded from
IndexedDB at start; it notes the files that change (`Store::take_changes`) and each frame's
changes are written back, so mail stays between visits (settings stay in local storage; never
passwords). The `WebBackend` runs each account's jobs in order on the browser's event loop with
`quatrelle-jmap` over `fetch`, holds the push stream open (read through `fetch`, parsed by
`quatrelle_jmap::sse`), polls as the fallback, and answers what a browser can't do (IMAP,
OAuth sign-in, data files) with an explanation. `Services` tells the UI files are downloads and
that the file picker answers on a later frame. `Session::jmap_only` keeps the account form to
JMAP.

## The session (`crates/engine`)

`Session` holds the accounts (config, folders, connection status), the view (a folder, or a role
across all accounts such as *All Inboxes*), the filter and search, the message list, selection and
focus, the open message (`Reading`, parsed on demand from the cached source), the message being
written (`Compose`), preferences and status-bar notices.

- `run(id, params)` dispatches a command from the registry (`cmd.rs`), checking `enabled` first and
  catching panics.
- `take_jobs()` hands queued work to the network layer; `apply(event)` folds results back in.
- Changes are optimistic: `apply_op` updates the in-memory list immediately, then the worker applies
  it to the cache and server and reports `FolderChanged`, which reloads from the cache (the single
  source of truth).
- Local-only accounts (the sample mailbox) apply changes to the cache directly, including moves
  (new UIDs in the destination) and sending (filed in Sent).

### Scheduling a meeting (`session/scheduling.rs`, `cmd/scheduling.rs`)

The event form has Outlook's two helpers, adapted. The **Room Finder** (`calendar.rooms`) lists
the rooms to book: an Exchange or Microsoft 365 account asks its server once
(`Job::GetRooms` → `Event::Rooms`: the room lists and their rooms), and the rooms the calendar
already knows come after them — the mailboxes of the calendars Outlook keeps in its Rooms group
("Salles" in a French Outlook, `model::scheduling::is_rooms_group`) and every room booked in a
meeting kept here — so a server with no room lists set up (most on-premises ones) still offers
something; any other account offers the calendar's at once. A room picked is a resource
attendee (`Attendee::cutype` `ROOM`, written as iCalendar's `CUTYPE`, EWS `Resources`, Graph
`type: resource`), which Exchange books itself, and the place follows the rooms until it's typed
by hand. The **Scheduling Assistant** (`calendar.availability`) draws a day's hours, a row each
for the organizer, the guests and the rooms, and their busy time (`model::scheduling::BusySlot`,
by kind: busy, tentative, out of office, working elsewhere, no information) with the meeting
over them; a click or a drag moves the meeting, and the start and end fields follow. Exchange
accounts ask their server once per window (`Job::GetAvailability` → `Event::Availability`, on
the account's second connection, so a long sync never holds it); everyone else is answered from
the calendars kept here (the account's own for its owner, the one opened by their address for a
colleague or a room). What's asked is kept in `Session::rooms` and `Session::free_busy`
(`Fetched`: loading, ready or failed, with why), and the form draws from that frame after
frame, asking only for what it hasn't.

## Languages

Interface text is English in the code and looked up in a catalog per language
(`crates/ui-egui/src/i18n/*.tsv`: context, English, translation) through `tl!` and `tf!`; anything
untranslated shows in English. Commands, the CLI and mail stay English. The language follows the
system unless one is chosen in Settings; Chinese and Japanese load an installed CJK font.
Release builds also compile in fonts from [storytold/craft-fonts](https://github.com/storytold/craft-fonts)
(Arabic, Japanese, Chinese, Thai; in the browser only the small ones) as the last fallbacks, with
their licences on the About page. It is an optional build input, never a Cargo dependency:
`CRAFT_FONTS_DIR=<checkout>` makes `crates/ui-egui/build.rs` embed what its manifest lists;
unset, the table is empty and nothing changes.

Emoji are pictures, not line drawings: egui draws text from outlines in the text's colour, so
its own emoji fonts give black outlines. `ui-egui/src/look/emoji.rs` reads the system's colour
emoji font once through `Services::emoji_font` (Noto Color Emoji on Linux, found by fontconfig's
`emoji` or its packaged paths; Segoe UI Emoji on Windows; Apple Color Emoji on macOS;
`QUATRELLE_EMOJI_FONT=<file>` to try another; `apps/quatrelle/src/appearance.rs`) and installs
an egui glyph rasterizer asked about every grapheme cluster before any font. It answers only for
clusters with Unicode's emoji presentation (UTS #51: 😀, 🇫🇷, 👨‍👩‍👧, 1️⃣, ☺️ with its selector; not ☺
or ✔, which are text), shapes the cluster with the font (`harfrust`, egui's own shaper: a flag,
a skin tone, a family or a keycap is a ligature in the font's tables) and draws its glyphs from
the font's bitmaps (`CBDT`, `sbix`: PNGs scaled down with a proper filter, premultiplied) or
its layered outlines (`COLR`, with `vello_cpu`, egui's own rasterizer). egui keeps each result
in its glyph atlas, once per emoji and size, so every text egui draws — the message list, the
reading pane, the editor, calendar chips — shows the picture at no cost per frame. A cluster
the font can't draw (an emoji newer than it), an over-long cluster (a run of joiners) or a
cluster past 256 pixels is left to the fonts, as before; so is everything when there's no colour
font (the browser, a bare system).

egui shapes each run of text in its script's direction but lays runs out left to right, with no
bidirectional algorithm. `ui-egui/src/look/bidi.rs` orders right-to-left text itself: each line goes
through the Unicode Bidirectional Algorithm (`unicode-bidi`), is cut where egui will cut it into
runs (the main font's coverage, read with `skrifa`), and runs egui will turn around are handed
over turned around; Arabic that egui won't shape gets its joined forms. Wrapped text (mail bodies,
subjects) is broken into lines by Quatrelle first (`bidi::job`). Translations of Arabic and
Hebrew go through it in `tl!`/`tf!`; text for the system (window titles, notifications) is taken
as written (`i18n::as_written`).

## Search

`model::search` parses the query language into clauses. The session builds an inverted index
(`BTreeMap<word, docs>`, so a word prefix is one range scan) over summaries, cached bodies and
attachment names for the chosen scope; candidates are verified against the full query, so the index
may over-approximate but never misses. The index is rebuilt lazily after changes.

The index is built a slice at a time (`Session::index_step`, `search.run` with an `index` budget):
the window indexes a few hundred messages per frame and draws between slices, so a big mailbox's
first search never freezes it. The address book built from the mail (`model::contacts`, for the
recipient boxes' suggestions, Select Names and the Focused Inbox) goes the same way
(`Session::contacts_step`): the window asks for what is known so far (`suggest_contacts_now`,
`address_book_now`) and takes a slice per frame while there's mail left to read; commands and
the CLI, with no frames to draw, read what's left in one go (`suggest_contacts`, `address_book`).
Like Outlook's AutoComplete list the book is kept on disk (`contacts.json`) with how far each
folder was read: a start reads no folder, a folder a sync changed has only its new mail read,
and every folder is read whole once. The people saved in Contacts are a layer over it, laid
again when they change. An address taken out of the suggestions (the × beside one,
`contacts.forget`) stays out until it's written to.

The index only knows what is here, and what is here is bounded (a thousand headers, forty bodies
per folder on some accounts). So the session also asks the server's own index, once per query
(`Job::Search` → `Event::Found`): Exchange's `FindItem` with a `QueryString` made from the query's
clauses (`ews::search`), over every folder the account knows, and the hits — whole summaries, with
the item ids numbered as a sync numbers them, so a message found this way can be read and kept —
are shown after the local ones, once each.

An IMAP server is asked the same way (`sync::search_folders`): the query's clauses become `SEARCH`
keys (`sync::imap_criteria`, RFC 3501 §6.4.4 — `TEXT` for words anywhere, `FROM`/`SUBJECT`/
`BODY`, `OR TO … CC …` for `to:`, `UNSEEN`/`FLAGGED`/`KEYWORD`, `NOT`), a word that isn't plain
ASCII going as a literal under `CHARSET UTF-8`. A string IMAP can't ask for (an attachment, an
importance, a category) stays with the index. Each folder is `EXAMINE`d and searched, the newest
two hundred hits kept; the headers of those this computer doesn't have are fetched into the
folder's cache, so a hit can be read, kept, marked or moved like any other. A search of all
folders asks the biggest first (Gmail's All Mail holds everything) and stops at eighty folders or
five seconds, so the search box answers while the user waits; the folder on view alone costs one
round trip.

A folder keeps a window of its own (`windows.json` with the account) when Show more asked for
more than Options keeps: the worker syncs it that wide from then on, and a wider window takes the
newest of what isn't kept yet rather than only what is older than the oldest kept, since a hit of
the server's search can be older than the messages skipped.

## Threading

`model::thread` implements JWZ threading (as used by IMAP `THREAD=REFERENCES`): containers per
Message-ID, parent links from `References`/`In-Reply-To` without creating loops, empty containers
promoted away, then replies that lost their references grouped by base subject. Every message
appears in exactly one conversation.

## HTML mail (`crates/mime`, `ui-egui/src/reading/body.rs`)

There is no browser engine: an HTML body is read into Quatrelle's own model and drawn with
egui. `layout::inline_css` first writes the message's `<style>` rules into each element's
`style` (inline wins, `!important` over both). `layout::html_layout` then reads the HTML with
html5ever's tokenizer and keeps what gives a message its shape as `Block`s: `Table`s of rows of
`Cell`s (tables, and boxes laid out side by side with CSS: flexbox, grids, floats, Word's
`align="left"` signatures), each with its width, colour, background picture, overlays
(`position: absolute`), and since the receipt-and-newsletter round: borders on every edge
(`Edges`: the `border` shorthand, its sides and parts, `border="1"`, in px/pt/cm…, capped at 8
px), `border-radius`, per-cell `padding` (CSS or `cellpadding`), and `VAlign` (`valign` /
`vertical-align`; a table cell's default is the middle, a CSS column's the top). Text inside is
`html_to_rich`'s `RichText` (html2text with the message's CSS for colours and hidden parts);
what html2text doesn't carry — font sizes, typefaces, CSS bold/italic, underline (`<u>`,
`<ins>`, `text-decoration`) — `fonts.rs` marks in the HTML first and resolves into the
`Span`s afterwards. The reader (`Body::blocks` / `row` / `lines` / `span`) draws the blocks:
cells side by side sharing the width (stacked when too narrow; a table without a width that is
one box of text, a button, is as wide as its text and centred when its cell centres), each cell
a frame with its padding, fill, rounded corners and borders (drawn on whole pixels: top and left just outside,
right and bottom just inside, so neighbours' lines overlap as a collapsed table's), and its
content moved down to the row's middle or bottom from the height measured the frame before
(`CellHeights`, by message and cell number: a lookup per cell, no second layout pass). The
editor's pasted signatures read the same spans (`rich::span_style`), so what the reader keeps,
the composer keeps too.

## Testing

- `model`, `mime`, `store`: unit tests, including hostile inputs.
- `net`: a scripted stream for framing edge cases, an in-memory IMAP server (`fake.rs`) for the sync
  algorithm and server-side changes, a local SMTP stub, an end-to-end test of the threaded worker
  pool over real TCP sockets, and in-memory JMAP (`fake_jmap.rs`), Microsoft Graph
  (`fake_graph.rs`), Exchange Web Services with NTLM (`fake_ews.rs`; NTLM also against the
  MS-NLMP test vectors), CalDAV/CardDAV (`fake_dav.rs`) and ManageSieve servers driving accounts
  through the same pool. Outlook data files are exported with `outlook-pst-rw` and read back
  by the importer; contact and appointment mapping and recurrence patterns are tested on their own. HTTP, DNS, autoconfig and OAuth run against local stubs. `serve_for_the_browser` (ignored)
  keeps a JMAP server with CORS running for trying the web app by hand.
- `jmap`: the session, URLs and paths against canned answers.
- `engine`: commands against the sample mailbox and scripted network events; every command with junk params.
- `ui-egui`: headless egui frames in every layout and dialog, shortcuts as input events, and a 2,000-message folder;
  every catalog covers every interface string, and a tour in a pseudo-language finds text that skips the catalogs.
