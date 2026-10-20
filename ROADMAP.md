# Roadmap

**Stage:** v0.19 — Shared mailboxes of their own, and SMTP's own sign-in (2026-10-20). Each milestone below is a version: v0.1 was
the first working client, and every later one is named for what it added.
Scope: [storytold/artcraft#2177](https://github.com/storytold/artcraft/issues/2177) (lightweight,
local-first, media-aware v1); the fuller request is
[storytold/artcraft-services#80](https://github.com/storytold/artcraft-services/issues/80).

## Headline numbers

| | |
|---|---|
| Crates | 8 libraries + 3 apps + xtask |
| Commands | 141 (`mailcraft-cli commands`) |
| Tests | 340 (unit, in-memory IMAP, JMAP, Graph, EWS (mail, calendars, contacts, Autodiscover), CalDAV/CardDAV and ManageSieve servers, NTLM test vectors, SMTP and HTTP stubs, data files, TCP end-to-end, an X server drag, headless UI) |
| Languages | 14 (English, Spanish, French, German, Dutch, Brazilian Portuguese, Italian, Russian, Japanese, Korean, Simplified and Traditional Chinese, Arabic, Hebrew) |
| Platforms | Windows, macOS, Linux; the browser (JMAP accounts) |

## Milestones

1. **v0.1 — Read and send** ✅ accounts, sync, reading, writing, organising, search, sample
   mailbox, CLI, packaging.
2. **v0.2 — Daily driver** ✅ rich-text compose, formatted HTML view with per-sender pictures,
   outbox, folder management, notifications, contacts autocomplete, categories, rules, snooze.
3. **v0.3 — Modern sign-in** ✅ OAuth2 (Gmail, Microsoft 365), autoconfig, QRESYNC.
4. **v0.4 — Media and Craft integration** ✅ attachment previews and thumbnails (PDF, video),
   "Open in Craft app", dragging attachments out, print and PDF export, eleven languages, JMAP
   with push.
5. **v0.5 — Beyond mail (deferred by #2177)** ✅ calendar and contacts (CalDAV/CardDAV, meeting
   invitations), the File page (automatic replies, mailbox size), messages in their own windows,
   Exchange Online through Microsoft Graph, Outlook data files, the browser version.
6. **v0.6 — Outlook's reach** ✅ Outlook's message editor and windows, Exchange calendars and
   contacts (Graph), on-premises Exchange (EWS, NTLM), `.pst` contacts, calendar and export,
   newsletter layouts, Arabic and Hebrew, browser push and a kept cache, video frames and
   drag-out everywhere.
7. **v0.7 — The open gaps closed** ✅ CSS layouts, EWS calendars, contacts and Autodiscover,
   mirrored right-to-left windows, `.pst` tasks and the calendar and contacts exported beside it.
8. **v0.8 — POP and Outlook's own data files** ✅ POP accounts with folders on this computer,
   starting from an existing `.pst`, and real Outlook `.pst` files checked end to end.
9. **v0.9 — Outlook's everyday commands** ✅ From an audit against Outlook: Arrange by, a Send /
   Receive tab with Work offline, Undo, more filters, Ignore, importance and read receipts, File's
   Save As and Print, POP copies removed after days; drafts no longer lost offline.
10. **v0.10 — Windows, folders and rules as Outlook keeps them** ✅ Remembered window places and
   sizes, subfolders under their parents, rules run at once, attachments in message windows; from
   a second audit: the Search tab, Follow Up dates, Delay delivery and Replies to, Outlook's menu
   items, hover everywhere.
11. **v0.11 — Notes, previews and Exchange beyond the mailbox** ✅ From issue #1: attachments
   previewed in the reading pane (Word, Excel and PowerPoint too), Notes, three People layouts,
   other people's Exchange calendars, public folders, Dutch; Outlook.com and Hotmail say why
   they need Microsoft sign-in.
12. **v0.12 — Microsoft sign-in that works** ✅ Microsoft sign-in tried end to end: the sign-in page
   opens in the browser on Windows, accounts that sign in that way ask to sign in again rather
   than for a password, an account Microsoft refuses says why, sent mail shows in Sent Items at
   once; dialogs and the folder pane keep their sizes.
13. **v0.13 — The open gaps, again** ✅ From an audit of [docs/gaps.md](docs/gaps.md): other
   people's Exchange calendars can be changed where they allow it, HTML mail's background
   pictures and boxes placed over others, text in its cell's colour, Outlook's notes from a data
   file, right-to-left mail copied as written; calendar boxes that fit their lines.
14. **v0.14 — What testers asked for** ✅ From issues #1 and #2: Hotmail accounts with a password
   switch to Microsoft sign-in, a mailbox Microsoft won't open over IMAP says why (or moves to
   Graph), generated HTML reports as Outlook shows them, quick search, reading and signature
   options, calendar colours, ticks, side by side and more months, List columns, public contacts,
   Exchange's outside replies, how much mail is kept.
15. **v0.15 — What testers asked for, again** ✅ From issue #1's next round: calendars side by
   side each in a pane of its own, other people's Exchange calendars editable by their editors,
   signatures and text pasted from Outlook with their look, public folder contacts on Exchange
   2013 and later, the address book to open with, and long names in it.
16. **v0.16 — Contacts as Outlook keeps them** ✅ From issue #1's third round: contacts' numbers
   by kind, their business, home and other addresses and birthday (vCard, Exchange, Graph, data
   files); signatures pasted with their sizes and typefaces; a wrong recipient gone with one
   Backspace; someone's Exchange calendar editable when its events say so; public folders routed
   on Exchange 2013 and later (listing and mail too), and People's address books to find them.
   A second connection per account for sending and automatic replies, so neither waits for a
   sync; diagnostics in mailcraft.log; French values kept away from words that elide; address
   suggestions that fit and click anywhere. Big data files and mailboxes: a 64.5 GB `.pst` of
   800,000 messages imports in 13 minutes under 850 MB of memory, and a mailbox that size opens,
   searches and changes without reading all of it. The File pages laid out as settings
   (Options, Account information, Open & Export), rules exported, imported and run all at once,
   Print asking where the PDF goes, Ctrl+A for the text or the attachments being looked at.
17. **v0.17 — Copy and paste, and public folder contacts** ✅ From issue #1's fourth round: the
   clipboard back (nothing could be pasted or copied since egui 0.37), so signatures paste from
   Outlook again; the organisation's shared contacts and calendars found in Exchange's public
   folders, and the log saying how many.
18. **v0.18 — Shared mailboxes and the contact card** ✅ From issue #1's fifth round: an
   Exchange account's shared mailboxes open under it by themselves (the ones Autodiscover names)
   or by hand (Info › Account settings › Shared mailboxes); a name in a message's header opens a
   contact card (send an email, add or edit the contact, copy the address) and its right-click
   menu has the same; Enter submits the one-field dialogs again. A removed account's data file
   stays, as Outlook keeps a removed account's `.pst`.
19. **v0.19 — Shared mailboxes of their own, and SMTP's own sign-in** ✅ From issue #1's sixth
   round and issue #3: an Exchange shared mailbox shown as a mailbox of its own, Exchange's
   folders under the names the server gives them, the conversation above a message only when
   the list is grouped by conversation, and an outgoing server signed in to with a user name
   and password of its own.
20. **Next:** what's left in [docs/gaps.md](docs/gaps.md): a relay for IMAP accounts in the
   browser, bidirectional editing, Wayland drag-out, Kerberos-only Exchange, what the `.pst`
   libraries can't do yet; then the search index on disk, rules on IMAP (Sieve) servers and every
   contact field with its photo.

## Progress log

- **2026-10-15** — **v0.1**: workspace, model (threading, search, dates, UTF-7), MIME, store, IMAP client
  and sync (CONDSTORE, IDLE, offline replay), SMTP, worker pool, engine with 44 commands, egui UI
  (three panes, ribbon, dialogs, dark mode), desktop app (keychain, file dialogs), CLI, docs, CI.
- **2026-10-15** — **v0.1**: packaging and release workflows (Windows MSI incl. ARM64, macOS dmg, Linux
  AppImage/deb/rpm/Flatpak, FreeBSD) on the shared Craft pipelines; app icon; `--version`.
- **2026-10-15** — **v0.2–v0.4** (the gap list): folder management, Outbox, QRESYNC, contacts autocomplete,
  notifications, categories / pins / snooze / rules / Focused, an HTTP client, formatted HTML with
  pictures on request, rich-text compose with inline pictures, autoconfig (ISPDB, MX, SRV), OAuth
  2.0, attachment previews and Craft apps, print / PDF / `.eml`, seven interface languages, and
  JMAP as a second transport.
- **2026-10-15** — **v0.4–v0.5**: pop-out message windows and the File page (automatic replies over ManageSieve
  and JMAP, quotas, emptying folders); JMAP push; HTML colours and tables; dragging attachments
  out; PDF and video thumbnails; Italian, Russian, Korean and Traditional Chinese; calendar and
  contacts (iCalendar, vCard, CalDAV/CardDAV, invitations).
- **2026-10-15** — **v0.5**: Outlook data files (`.pst`) open as mailboxes; Exchange Online through
  Microsoft Graph; the store in memory, JMAP as a portable crate, and the browser version
  (`apps/mailcraft-web`).
- **2026-10-15** — **v0.6**: Outlook's message editor and windows; Exchange calendars and contacts; `.pst`
  contacts, calendar and export; video frames from the system; browser push and IndexedDB;
  Arabic and Hebrew with right-to-left text; newsletter layouts; on-premises Exchange through
  EWS with NTLM; drag-out under X11.
- **2026-10-15** — **v0.7**: CSS columns (flexbox, grid, inline-block, floats); EWS calendars,
  contacts and Autodiscover; mirrored windows for Arabic and Hebrew; `.pst` tasks, and the
  calendar and contacts exported beside the `.pst`.
- **2026-10-15** — **v0.8**: POP accounts (downloaded mail in folders on this computer, a copy left
  on the server), "use an existing data file", and real Outlook `.pst` files checked: open files,
  `.ost` files, RSS Feeds and the Outbox handled; each account in a `.mailcraft` data file in
  Documents\MailCraft Files.
- **2026-10-15** — **v0.9**: an audit against Outlook: Arrange by and Reverse sort, a Send / Receive
  tab with Work offline, Undo, Filter (categorized, important, to me), Ignore a conversation, New
  items, a minimizable folder pane, File › Save As / Save Attachments / Print / Exit, importance
  and read receipts, POP server copies removed after days or once deleted; drafts that can't
  reach the server are kept.
- **2026-10-15** — **v0.10**: windows open where they were (maximized too), a folder made in the
  Inbox shows under it, a new rule runs at once on the Inbox, attachments as Outlook's chips that
  open with a double-click, and notifications for mail rules file away.
- **2026-10-16** — **v0.10**: a second audit against Outlook: a Search tab, Follow Up with dates,
  Delay delivery and Replies to, Forward as attachment, Reply with meeting, Find related, Delete
  all and folder Properties, hover on every control; CJK names drawn in any interface language.
- **2026-10-17** — **v0.11**: issue #1: attachments preview in the reading pane (pictures, PDF
  pages, text, Word, Excel, PowerPoint), Notes, three People layouts, other people's Exchange
  calendars, public folders, Dutch; Outlook.com and Hotmail say why they need Microsoft sign-in.
- **2026-10-17** — **v0.12**: Microsoft sign-in tried end to end (Outlook.com over IMAP, a
  personal Microsoft account through Graph): the sign-in page opens on Windows, "Sign in again"
  instead of a password, refusals say why, Sent Items catches up at once; dialog and pane sizes.
- **2026-10-17** — **v0.13**: the gaps audited again: shared Exchange calendars editable where
  allowed (Graph and EWS), background pictures and placed boxes in HTML mail, cell colours, notes
  from `.pst` files, right-to-left text copied from a right-click; Exchange Online's public
  folders wait on Microsoft (EWS is being retired there).
- **2026-10-17** — **v0.14**: issues #1 and #2 reproduced and fixed: Microsoft sign-in's dead ends
  (passwords, "authenticated but not connected", a stuck "Connecting…"), HTML reports (pictures,
  style sheets, columns), search speed and People/Notes search, reading pane and signature
  options, notes order, calendar categories, ticks, side by side and months, List columns,
  public-folder contacts, outside replies on Exchange, mail kept offline, Open folder.
- **2026-10-18** — **v0.15**: issue #1 again, each reproduced first: side by side as Outlook shows
  it (a pane per calendar), an editor's rights on someone's Exchange calendar read as Exchange
  gives them, public folder content routed to the mailbox holding it, HTML on the clipboard pasted
  with its look and signatures kept formatted, Select Names opening with the address book chosen.
- **2026-10-19** — **v0.16**: issue #1's third round, each reproduced first: contacts' phones by
  kind, addresses and birthdays through every kind of account and the contact form, People's
  address books (an organisation's shared ones among them), pasted fonts and sizes, Backspace on a
  whole recipient, an editor told by the events' rights, public folder hierarchy and content
  routing with errors shown.
- **2026-10-19** — **v0.16**, more: a message sent, or automatic replies asked for, during a big
  mailbox's first sync went only after it (20 s and more in a test holding each folder 4 s): they
  go on a second connection now, sharing the sign-in. Diagnostics had nowhere to be written:
  `mailcraft.log` keeps them. French sentences keep filled-in values away from « de », « le »
  and other words that elide, and a test checks the catalog.
- **2026-10-19** — **v0.16**, big mailboxes, measured on generated data files (3 GB, 9.4 GB of
  800,000 messages, 64.5 GB): imports wrote a transaction per message (386/s), held every folder's
  list, parsed each message twice and stored values rounded up to powers of two (2.1x the file);
  now 6,000/s, 624 MB at most for the 9.4 GB file, a cache smaller than the file (compressed, in
  page-filling pieces), lists written a page at a time. In the app: deleting 8,000 messages took
  65 s (quadratic), one flag 4.7 s, a search 80-150 s and 3.5 GB; now 0.2 s, 0.5 s, 2 s again and
  2.5 GB, with folder lists kept in pages and only the changed ones written, a bounded cache of
  lists and of search text, and the message list's rows made once per change.
- **2026-10-19** — **v0.16**, a round of everyday fixes: Graph and EWS prefetch skips a message
  the server errors on instead of abandoning the folder; Replied/Forwarded flags are owed until
  the Outbox delivers; search accepts `from: zoe`; removing a contact needs confirmation; vCard
  2.1 quoted-printable is decoded; no taskbar highlight for new mail; permanent delete on Gmail
  moves to Trash and expunges there; prefilled reply recipients end with `; `.
- **2026-10-19** — **v0.16**, the File pages and the reading pane: Options, Account information
  and Open & Export as cards of settings, each with its control or button on the right
  (switches, choices side by side; Info's account actions no longer hidden in a tile's menu);
  Export says which account it writes, in a list, names the file after its address and offers
  `.pst` as its type; one stray stroke fixed in the Outbox/Export icon; From's menu drawn with a
  chevron (the font had no ▾). A suggestion picked with the mouse keeps the cursor in To, Cc or
  Bcc. Print asks where the PDF goes. Ctrl+A in a message's text no longer selects the whole
  list too; in the attachments it selects them all (Ctrl+click picks some), and several are
  saved together. Rules go to a file and come back (an account by its address, a missing
  category made), the Rules dialog lists them a line each, with a find box past six, and Run
  all runs every rule that's on.
- **2026-10-19** — **v0.16**, dependencies brought up to date, each changelog read and the change
  checked: egui and eframe 0.37 (ids, `scope_id`, `Role`, text alignment; its emoji fonts
  turned back on, and missing characters, ✓ ▾ or Devanagari, now found among the system's fonts),
  quick-xml 0.42 (text and attributes as `str`; a test of entities, CDATA and references), hmac
  0.13 with md-5 and md4 0.11 (the NTLM test vectors pass), redb 4.4 (same file format: a 4.3
  data file read and written by 4.4 and read back by 4.3), dirs 7 and miniz_oxide 0.9. skrifa,
  windows-registry and x11rb stay on the versions egui, ipconfig and winit use, so each is built
  once; keyring stays on 3.x, as its 4.0 asks of existing applications.
- **2026-10-19** — **v0.17**, issue #1's fourth round: nothing could be pasted (a signature from
  Outlook, or text anywhere) or copied since egui 0.37, which made the clipboard a feature of
  eframe's own, left out with its defaults; named now, with a test that it stays. And the
  organisation's shared contacts and calendars never appeared: Exchange answers each folder in
  an element of its kind (`ContactsFolder`, `CalendarFolder`…), and only `Folder` was read.
  Every kind is read, the fake Exchange answers that way (its test failed until then), and the
  log says how many public address books and calendars were found.
- **2026-10-19** — **v0.18**, issue #1's fifth round: shared mailboxes didn't show for Exchange
  and couldn't be added. Autodiscover's `AlternativeMailbox` entries (delegate and team
  mailboxes; archives left out) open by themselves, others are added by address, and each is
  listed through EWS with its `Mailbox` and the `X-AnchorMailbox` header; the fake Exchange has
  them (with no-access and by-legacy-name cases) and its test lists and syncs them. A name in a
  message's header opens a contact card, its address a new message to them, and right-click ›
  Add to contacts a filled-in contact. Enter hadn't submitted the password, shared calendar or
  shared mailbox dialog: they took the focus back before seeing the Enter that let it go.
  Removing an account deleted its `.mailcraft` file; as Outlook leaves a removed account's
  `.pst` on disk, the file now stays (closed, no one's), the same address added again gets a
  file of its own beside it, and Open & Export's picker starts in MailCraft Files to open it.
- **2026-10-20** — **v0.19**, issue #1's sixth round and issue #3, each reproduced in the app first
  (against a fake Exchange made in Dutch with a shared mailbox mapped to it): a shared mailbox
  showed as a folder of the account's own mailbox; it's a mailbox of its own now, after the
  account, its Inbox, Sent Items… with their icons and places, collapsible, with Add and Remove
  in its menu and the account's. Exchange's special folders showed MailCraft's English words
  ("Inbox") where Outlook shows the server's ("Postvak IN"); they keep the server's names (EWS
  and Graph). The reading pane showed a message's whole conversation above it though the list
  wasn't grouped; only when grouped now, as Outlook. And an outgoing server can sign in with a
  user name and password of its own (issue #3), its password kept apart in the keychain.
- **2026-10-18** — **v0.15**: issue #1 again, each reproduced first: side by side as Outlook shows
  it (a pane per calendar), an editor's rights on someone's Exchange calendar read as Exchange
  gives them, public folder content routed to the mailbox holding it, HTML on the clipboard pasted
  with its look and signatures kept formatted, Select Names opening with the address book chosen.
- **2026-10-18** — The window follows the desktop: under GNOME's Wayland session a title bar like
  the desktop's own (title, minimize, maximize and close where GNOME puts them, light or dark)
  instead of winit's bare grey frame; light or dark read from the desktop's settings on Linux
  (winit doesn't say there), the desktop's accent colour (Windows, macOS, GNOME and KDE) as an
  option on by default, and the desktop's interface font on Linux.
- **2026-10-18** — Voting buttons: on Exchange (EWS) and Microsoft 365 (Graph) accounts, a
  message's voting buttons (`PidLidVerbStream`, read as [MS-OXOMSG] describes it) show above
  it; a click sends that answer as Exchange's own reply carrying the choice
  (`PidLidVerbResponse`), so the sender's Outlook counts it, and the answer is remembered.
- **2026-10-18** — The window as Outlook's, from screenshots of it: a plain top bar, Microsoft's
  Fluent UI icons (MIT, the owner's choice), a ribbon of large buttons in named groups, the
  modules down the window's edge, and the message list as a table with sortable columns
  (automatically when the reading pane is at the bottom or off, or chosen in View).
- **2026-10-18** — The To-Do bar beside the mail (the month, the coming week's appointments), the
  calendar's Work week view, day headings, two months and calendar groups as Outlook's, the
  ribbon folding to its tabs (Ctrl+F1), and folder names without icons.
- **2026-10-18** — Shared and delegated mailboxes open in an Exchange or Microsoft 365 account
  (Graph and EWS), under a folder named after them, with their calendar; mail deleted there
  goes to their own Deleted Items. Microsoft 365's calendar groups (My Calendars, a team's)
  come in as Outlook lists them, colleagues' calendars kept in them included.
- **2026-10-18** — On-premises Exchange (EWS) too: the calendars Outlook keeps in the groups of
  its Calendar pane (a team's, shared calendars, rooms) come in under those groups, read from
  Outlook's navigation shortcuts ([MS-OXOCFG]) and the directory.
- **2026-10-19** — **Search that never freezes:** the index behind the search box was built in one
  go on the window's thread, so a big mailbox froze the window for as long as reading every
  message took (75 s for 800 000). It is now built a slice at a time (500 messages a frame, the
  window drawing between slices), what it has found so far is shown while it goes on with
  "Searching…" in the status bar, and carrying the same query on keeps the open message and the
  selection. Tested slice by slice: the first answer says more is coming, and the match is found
  once the index is done.
- **2026-10-20** — **Whole mailboxes kept offline:** Send / Receive › Download all brings every
  message of an account (or of one folder) that isn't on this computer yet, as Outlook's Download
  All: the engine lists the sources missing (`Store::has_body`) and keeps 32 asked for at a time,
  the worker fetches them over whichever transport the account uses, and each one is indexed for
  search as it arrives. The status bar says how far it has got, messages already kept are passed
  over, and it stops and says so. Tested: a batch at a time, each answer asks for the next.
- **2026-10-20** — **Rules that run on the server:** a rule has "Also run on the server" (Outlook's
  own), and MailCraft creates, changes and removes it in Exchange's inbox rules (EWS
  `UpdateInboxRules`, reading the ids back with `GetInboxRules`) and in Microsoft 365's
  (`messageRules`) — so mail is filed while the app is closed. Only what Exchange can express goes
  there (conditions in AND, no "any text", no newsletters, no flag or pin): the others stay on this
  computer, and the dialog says why. MailCraft only ever changes or removes rules it made, a rule
  turned off there is removed from the server, and rules made offline go up when the account
  connects. Tested: create, change by id, remove, and the engine's job and id keeping.
- **2026-10-20** — **Outlook's contacts, complete:** a card keeps department, office, profession,
  manager, assistant, web page, instant messaging, nickname, spouse and wedding anniversary, and its
  picture (`PHOTO`, base64, capped at 512 KiB; a link to one is never fetched). vCard 3 and 4 read and
  write them, so CardDAV carries them both ways; Exchange (EWS, `AllProperties`) and Microsoft Graph
  (`$select`, POST) map them too; the contact form edits them and the People card shows the photo
  (initials when there is none). Tested: a vCard round trip with the picture and Outlook's `X-`
  fields, and the contact commands keeping them.
