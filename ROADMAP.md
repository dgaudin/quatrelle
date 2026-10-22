# Roadmap

**Stage:** v0.22 — Outlook's everyday tools, and test builds that update themselves (2026-10-22). Each milestone below is a version: v0.1 was
the first working client, and every later one is named for what it added.
Scope: [storytold/artcraft#2177](https://github.com/storytold/artcraft/issues/2177) (lightweight,
local-first, media-aware v1); the fuller request is
[storytold/artcraft-services#80](https://github.com/storytold/artcraft-services/issues/80).

## Headline numbers

| | |
|---|---|
| Crates | 8 libraries + 3 apps + xtask |
| Commands | 141 (`quatrelle-cli commands`) |
| Tests | 369 (unit, in-memory IMAP, JMAP, Graph, EWS (mail, calendars, contacts, Autodiscover), CalDAV/CardDAV and ManageSieve servers, NTLM test vectors, SMTP and HTTP stubs, data files, TCP end-to-end, an X server drag, headless UI) |
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
   sync; diagnostics in quatrelle.log; French values kept away from words that elide; address
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
20. **v0.20 — A window on every Windows PC** ✅ Windows draws with DirectX 12 alone: a real GPU
   first, Windows' software renderer (WARP) on a PC without one, where the window never opened;
   a window that can't open says why; and the other Craft apps' fixes to shared ground that
   apply to Quatrelle (GPU device limits and errors, Intel Macs, Linux's GPU, XWayland for drag
   and drop, clipboard reads that can't hang, a maximized window that doesn't blink, the DMG's
   strict signature, Windows 10 at install), and from WordCraft, PdfCraft, DeckCraft and
   GridCraft's desktop glue: winit's input-method and monitor fixes, windows back on screen,
   arguments and quitting that can't crash, DPI awareness, files for other apps kept private;
   and from issue #1, calendars told apart by account, shared mailboxes' calendars, Join and
   links in meetings, Check for updates; and from issue #5, a Microsoft 365 sign-in kept in
   Windows' Credential Manager whatever its length; and from issue #4, accounts kept online only
   (none of their mail written to the computer) and a portable mode that leaves nothing behind.
21. **v0.21 — One Quatrelle at a time** ✅ Started again, Quatrelle brings its window forward
   rather than opening a second one on the same data; and from issues #1 and #4, shared
   mailboxes as mailboxes of their own, invitations that read with their links in dialogs that
   fit the window, older mail shown and searched on the server (every kind of account, and in
   the browser), and online only made clearer (a password asked for at start, a warning before
   closing loses anything).
22. **v0.22 — Outlook's everyday tools, and test builds that update themselves** ✅ What a 1:1
   look at Outlook's Home ribbon and calendar found missing: reminders that pop up (snooze,
   dismiss, open), blocked and safe senders, Clean Up, Search Folders, Quick Steps
   (Ctrl+Shift+1…9), the Work Week view, Insert date & time and Word count. A build from the
   unsigned test-build workflow fetches the next test build in the background, checks its
   SHA-256 against the release, and installs it when Quatrelle closes (or at once with Restart
   to update), the way that copy was installed: `.msi`, portable `.zip`, `.dmg` or
   `.AppImage`. Signed releases are unaffected.
23. **Next:** what's left in [docs/gaps.md](docs/gaps.md): a relay for IMAP accounts in the
   browser, bidirectional editing, Wayland drag-out, Kerberos-only Exchange, what the `.pst`
   libraries can't do yet; then rules on IMAP (Sieve) servers, the `.pst`'s contact pictures and
   the business-card view.

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
  (`apps/quatrelle-web`).
- **2026-10-15** — **v0.6**: Outlook's message editor and windows; Exchange calendars and contacts; `.pst`
  contacts, calendar and export; video frames from the system; browser push and IndexedDB;
  Arabic and Hebrew with right-to-left text; newsletter layouts; on-premises Exchange through
  EWS with NTLM; drag-out under X11.
- **2026-10-15** — **v0.7**: CSS columns (flexbox, grid, inline-block, floats); EWS calendars,
  contacts and Autodiscover; mirrored windows for Arabic and Hebrew; `.pst` tasks, and the
  calendar and contacts exported beside the `.pst`.
- **2026-10-15** — **v0.8**: POP accounts (downloaded mail in folders on this computer, a copy left
  on the server), "use an existing data file", and real Outlook `.pst` files checked: open files,
  `.ost` files, RSS Feeds and the Outbox handled; each account in a `.quatrelle` data file in
  Documents\Quatrelle Files.
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
  `quatrelle.log` keeps them. French sentences keep filled-in values away from « de », « le »
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
  Removing an account deleted its `.quatrelle` file; as Outlook leaves a removed account's
  `.pst` on disk, the file now stays (closed, no one's), the same address added again gets a
  file of its own beside it, and Open & Export's picker starts in Quatrelle Files to open it.
- **2026-10-20** — **v0.19**, issue #1's sixth round and issue #3, each reproduced in the app first
  (against a fake Exchange made in Dutch with a shared mailbox mapped to it): a shared mailbox
  showed as a folder of the account's own mailbox; it's a mailbox of its own now, after the
  account, its Inbox, Sent Items… with their icons and places, collapsible, with Add and Remove
  in its menu and the account's. Exchange's special folders showed Quatrelle's English words
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
  own), and Quatrelle creates, changes and removes it in Exchange's inbox rules (EWS
  `UpdateInboxRules`, reading the ids back with `GetInboxRules`) and in Microsoft 365's
  (`messageRules`) — so mail is filed while the app is closed. Only what Exchange can express goes
  there (conditions in AND, no "any text", no newsletters, no flag or pin): the others stay on this
  computer, and the dialog says why. Quatrelle only ever changes or removes rules it made, a rule
  turned off there is removed from the server, and rules made offline go up when the account
  connects. Tested: create, change by id, remove, and the engine's job and id keeping.
- **2026-10-20** — **Outlook's contacts, complete:** a card keeps department, office, profession,
  manager, assistant, web page, instant messaging, nickname, spouse and wedding anniversary, and its
  picture (`PHOTO`, base64, capped at 512 KiB; a link to one is never fetched). vCard 3 and 4 read and
  write them, so CardDAV carries them both ways; Exchange (EWS, `AllProperties`) and Microsoft Graph
  (`$select`, POST) map them too; the contact form edits them and the People card shows the photo
  (initials when there is none). Tested: a vCard round trip with the picture and Outlook's `X-`
  fields, and the contact commands keeping them.
- **2026-10-20** — **A colleague's picture:** the picture Exchange (`GetUserPhoto`) or Microsoft 365
  (`/users/{id}/photo/$value`) keeps for an address is fetched when a contact has none of their own,
  asked for once per address (a "none" is remembered), capped at 512 KiB, and shown in the People
  list and on the contact card, with the initials when there is none. Tested: the EWS look-up against
  the in-memory server, and the engine asking once and keeping the answer. GitHub's CI (stable 1.99)
  also caught a new clippy lint in the model, fixed with `as_chunks`.
- **2026-10-20** — **The search index kept on disk:** each folder's index (its words, its messages
  and whether each one's body was read) is kept beside its cache in a compact binary form (words'
  documents as gaps, one or two bytes a message), written when the folder has been read. A later
  run takes a folder as it is instead of reading its messages again, and only a folder whose
  messages changed — a fingerprint of their text, flags aside, says so — is read again; the bodies
  that came in since are added one at a time. Marking mail read or filing it in a category no
  longer throws the index away, a big folder still joins the mailbox's a slice a frame, and a
  damaged file is refused and rewritten rather than trusted. Tested: a word only a body held is
  found with the body gone from the cache, a body downloaded after the index was kept joins it,
  and a folder that changed is read again on its own.
- **2026-10-20** — **Rules on IMAP servers:** a rule marked "also run on the server" now runs on an
  IMAP account too, in its Sieve script (`quatrelle` through ManageSieve): conditions become
  `header :contains` tests (all or any of them), actions `fileinto` (the folder, by its UTF-8 name)
  and `addflag` (read, flagged, `$Pinned`, a category's keyword), and `stop` keeps the rest from
  running. The automatic replies stay in the same script, our rules run before the user's own
  script (which ours `include`s), and the server is only asked for the extensions the rules need
  (`fileinto`, `imap4flags`). A rule whose folder isn't on the server waits and says so; what Sieve
  can't look at (attachments, newsletters) stays here, where the dialog now says what the server
  could not run instead of naming Exchange. Tested: the script the manager writes, the rules kept
  while the replies change and the other way round, a rule turned off, and the engine sending a
  rule Exchange would have refused.
- **2026-10-20** — **Contact pictures, everywhere they're kept:** a `.pst`'s contacts carry theirs
  as a hidden attachment (Outlook's "ContactPicture.jpg"), now read — the item's own image type, or
  what the bytes say, capped like any contact photo, and never a visible attachment or something
  else hidden (a signature, an OLE object). The People business-card view draws the picture (their
  card's, or the one the directory keeps for their address, asked for once) instead of initials,
  its shape kept; initials remain when there is none. Tested: the hidden-picture rule, and a
  headless frame showing the card's texture actually decoded and drawn.
- **2026-10-20** — **Rules made elsewhere, listed:** when an account connects its server is asked
  what it keeps — Exchange's inbox rules (EWS), Microsoft 365's (`messageRules`), an IMAP account's
  Sieve scripts — and the rules Quatrelle didn't make (Outlook's own, the server's web page, a
  script the user wrote) are listed in the Rules dialog under "On the server, made elsewhere", with
  the reminder that whoever made them owns them. Reading also heals Quatrelle's own ids (a rule
  whose copy vanished from the server gets one again) and refreshes the list after every change,
  so a rule made in Outlook shows up without a restart. Only the ids Quatrelle holds are ever
  treated as its own. Tested: the split by id, the EWS listing with a rule from Outlook, the Sieve
  scripts, and the engine keeping the ids and listing the rest.
- **2026-10-20** — **v0.20**: on some Windows PCs without a GPU the window never opened; the
  process stayed in the background holding the data files (so later launches were told another
  copy had them) and crashed when Windows shut it down. wgpu started Vulkan and OpenGL beside
  DirectX 12, and without a GPU those find only leftover or overlay drivers and Windows' OpenGL
  1.1. Only DirectX 12 starts now, the adapter picked by Quatrelle (a real GPU first, WARP when
  there's none), its shaders compiled with FXC; a window that can't open says why in a message.
- **2026-10-20** — **v0.20**, more: the sixteen sibling Craft repositories were cloned and their
  fixes to what every Craft app shares were read and checked against Quatrelle. Adopted, each
  tested: device limits from the adapter, wgpu errors logged after start-up, Intel Macs without
  indirect-call validation, Linux on the desktop's GPU and through XWayland (attachments drag on
  Wayland desktops; a stale DISPLAY falls back), clipboard reads with a time limit, no blink
  from a maximized window at start-up, the DMG built so its app passes `codesign --strict`, and
  the MSI's Windows 10 condition.
- **2026-10-20** — **v0.20**, the four closest siblings again, every commit to their desktop apps,
  packaging and build tools (367) read rather than searched: winit 0.30.13 vendored as PdfCraft
  carries it (Windows 11 monitor scale, input methods on every platform, windows created
  maximized: now shown once, maximized), windows brought back on screen and fitted to it,
  non-Unicode arguments, broken standard error and quitting on macOS made safe, per-monitor DPI
  in the manifest, and attachments for other apps in a folder of the user's own.
- **2026-10-20** — **v0.20**, issue #1's calendars, reproduced against a fake Exchange first: each
  account's calendars under its address in its colour and the server's name, a shared mailbox's
  calendar read with it, Join meeting and the notes' links (a long Teams link read whole from
  the HTML), a meeting in two calendars that opens again, and File › About's Check for updates.
- **2026-10-20** — **v0.20**, issue #5: an Office 365 sign-in couldn't be kept on Windows (its
  refresh token, about 2,000 characters, is over the 1,280 a Credential Manager entry held as
  the keyring crate wrote it); kept as UTF-8 over as many entries as it needs, reproduced and
  tested on the real Credential Manager.
- **2026-10-20** — **v0.20**, issue #4: an account can be kept online only (its mail held in
  memory, each message downloaded when opened, its data file gone) and `portable.txt` beside the
  app keeps everything in QuatrelleData there, passwords in memory only; reproduced with the app
  against a fake IMAP server (a 557 KB data file holding every subject and body, then nothing),
  the host's folders, Credential Manager and registry compared before and after.
- **2026-10-20** — **v0.20 merged into this fork:** upstream's release (24 commits: online-only
  accounts, portable mode, Check for updates, a Microsoft 365 sign-in kept on Windows, shared
  mailboxes' calendars and the server's folder names, the Windows/XWayland/packaging fixes,
  `winit` 0.30.13 vendored) came in beside this fork's work. Twenty-two conflicts resolved: the
  calendar pane keeps **Outlook's groups** (My calendars, the server's groups, Shared calendars)
  and gains shared mailboxes' calendars and their tips, a meeting in two calendars opens again
  (the box's id names whose it is), the Graph contact fields this fork reads are kept, and the
  port's own commands (`app.checkUpdates`, `account.onlineOnly`) join the 64 commits already
  here. Tested: `cargo xtask ci` green on the merge (23 suites), the port's own tests for an
  online-only account and a shared mailbox's calendar among them.
- **2026-10-20** — **Reminders (`VALARM`):** an event's alarm is read as the reminder (its
  `TRIGGER` a plain duration: `-PT15M`, `-P1D`), written back as Outlook writes it
  (`ACTION:DISPLAY`), and mapped both ways on Exchange (`ReminderIsSet`,
  `ReminderMinutesBeforeStart`) and Microsoft 365 (`isReminderOn`,
  `reminderMinutesBeforeStart`); an alarm Quatrelle can't read (an absolute time, one relative
  to the end) is kept as written rather than lost. The calendar's card says when it sounds
  ("15 minutes before", "1 hour before", "1 day before", "When it starts"), with the appointment
  dialog's Reminder list beside it. Tested: a `VALARM` parsed, written and read again; an
  unreadable one kept; a reminder set, changed, kept across a restart and taken off.
- **2026-10-20** — **Renamed Quatrelle:** the project, its crates, its binaries, its window and
  its data files take the new name (the postman's yellow Renault 4L, a little rusty, like Rust).
  Nothing a user had is lost: the old app folder (`%APPDATA%\mailcraft`, `~/.local/share/mailcraft`,
  `~/Library/Application Support/mailcraft`), the `MailCraft Files` folder, each account's
  `.mailcraft` data file, the keychain entry (`MailCraft`) and portable sticks (`MailCraft.portable`,
  `MailCraftData`) are renamed, or read where they are, once, at the first start; `MAILCRAFT_DATA`
  and `MAILCRAFT_LANGUAGE` still work. The MSI's UpgradeCode is the same, so an installed copy
  upgrades. Tested: an old app folder taken over, an old data file renamed with its mail read
  back, an old portable stick, and `cargo xtask ci` green (455 tests).
- **2026-10-20** — **The rename broke the data files' own notes (fixed):** a data file keeps
  `\0Quatrelle:deflate\0` or `\0Quatrelle:pieces\0` in front of every value it compressed or cut
  into pieces, and the rename rewrote those bytes too, so every compressed setting, every folder's
  sync state and every big message read back as garbage: "unreadable, starting fresh" for the
  states, an empty calendar list (so all 55 of the organisation's shared calendars were read at
  once, minutes of Exchange before any mail), nothing readable in the folders. The fork's notes
  (`\0MailCraft:deflate\0`, `\0MailCraft:pieces\0`) are read beside the new ones, and a test writes
  a data file carrying them and reads both kinds of value back. The lesson: a marker kept inside
  the data is a format, not a name — it may only change with a migration.
- **2026-10-21** — **A timed-out folder no longer stops at "Updating…" for an hour:** on a fresh
  install, Exchange slowed down (or throttled the first full sync of 110 folders on four
  connections), a request ran past the 60 s read timeout, and every folder after it was asked
  for on the main connection, one 60 s timeout each: the window sat on "Updating Drafts" for
  minutes. A reader's error was being flattened to "not found", so it never stopped the pass;
  it now keeps its kind, a fatal one stops the pass and backs off (5 s, doubling), the account
  goes back to one connection, and a connection left half-read after a timeout is dropped
  rather than reused (a stale 401 on it read as "the server refused the user name or password").
- **2026-10-21** — **A first sync no longer reads a whole organisation's calendars:** on a fresh
  profile nothing was ticked or unticked yet, so the calendar job read every shared calendar it
  found (53 of them, four minutes, one Exchange round trip after another) before it got to the
  contacts, and the Calendar and People pages looked empty. A sync that knows no shared calendar
  yet lists them and reads none (as Outlook leaves them unticked, and as the engine records them
  a moment later); contacts are read first, before any of it. Tested against the fake Exchange:
  a first sync lists Ana's calendar, reads nothing of it, and has the contacts.
- **2026-10-20** — **Spelling, underlined as you write:** a `quatrelle-spell` crate wraps
  `spellbook` (MPL-2.0) with the French and English (US) dictionaries carried gzipped in
  `assets/dictionaries`, so no system tool is needed and it works on Windows, macOS, Linux and in
  the browser. The composer underlines in red the words neither dictionary nor the user's own list
  knows, in the body and in the subject, as they're typed (the dictionary loads and the check runs
  a slice at a time, never in the frame that draws); `Prefs::spell_check` turns it off. F7 or
  Spelling opens the dialog: the word in its sentence, the suggestions, Ignore / Ignore all / Add
  to the dictionary (`spelling.add`, kept in `Prefs::spelling`), and the correction made where the
  word stands in the body or the subject (`compose.replaceText {from, to, text, field}`).
- **2026-10-20** — **Outlook's card when the pointer rests on an appointment:** resting it on a
  chip in the month, week or day view brings the card Outlook shows — when it is (both days for
  one over several, "All day" for an all-day one), where, which calendar it's in (named as the
  side pane names it), and when its reminder sounds, in words ("15 minutes before", "1 day
  before", "When it starts"). `card_rows` builds the rows, `reminder_text` says the alarm, and the
  same card follows the chip in every view; the reminder line is there when the appointment has
  one.
- **2026-10-21** — **A window keeps the place the desktop gave it:** the app handed every separate
  window its saved position and size at every frame, so a window the window manager had just
  placed — or one being dragged to the other screen — jumped between the two places several times
  a second (a compose window flashing, and looking like two). The place is now asked for at the
  window's opening only, and its last place is still remembered for the next one. `QUATRELLE_TRACE_WINDOWS=1`
  logs what each window reports about itself (outer and inner rectangles, monitor, scale) whenever
  it changes, which is how this was followed: the same window alternating between `2129,99` and
  `2190,154` frame after frame.
- **2026-10-21** — **A search reaches the server:** the local index only knows what this computer
  kept — a thousand headers and forty bodies per folder, by default — so a word in a February 2005
  message found nothing, which is what a mail client is for. A search now also asks Exchange's own
  index (`FindItem` with a `QueryString` built from the query's clauses: the words, and `from:`,
  `to:`, `subject:`, a phrase in quotes; the rest stays with the index), over every folder of the
  account, two hundred hits at most, and merges its findings with the local ones the way Outlook
  does: after them, once each by key or Message-ID, cleared with the next query. A hit is numbered
  as a sync numbers it — saved with the account's state — so it can be opened, kept, marked read or
  moved. IMAP's `SEARCH` is next. The app icon is now the owner's own picture (`assets/icon.png`,
  the 4L and the envelope): every size, the `.ico`, the `.icns` and the scalable SVG are sampled
  from it by `cargo xtask icons`.
- **2026-10-21** — **An invitation reads as Outlook reads it:** Exchange makes an invitation's HTML
  notes text, each link written after its words in angle brackets, and they were shown as that.
  They now read as the invitation does (`model::ical::note_lines`): `Join with Google Meet<https://…>`
  shows its words and opens the address from them, a phone link stays its words, an address alone on
  its line is a link of its own, and words on the line above (Google's Yes, No, More options) name
  the address below them. An invitation's notes (or any event's with a link) are shown to read
  rather than in the box that edits them, scrolling when tall, with Edit notes to change them; a
  dialog is never taller than the window any more, so its title and buttons stay on screen.
  `folderPane.move` comes with it: a mailbox is where the user put it, dragged or from its menu.
- **2026-10-21** — **A shared mailbox is a mailbox of its own:** one an Exchange account opens was
  a top folder inside the account's tree; it is now its own heading in the folder pane, after its
  account, with its own folders, its own menu and its own place among the mailboxes — moved up or
  down (an account's order follows), kept with the settings, and a new one coming after the
  mailbox it came with. The calendar pane follows that order, each shared mailbox's calendars
  under its name. A folder also says what's past what's kept ("40 more on the server", Show
  more), the worker keeping that folder wider from then on; closing Quatrelle with an
  online-only account's unsent mail or changes asks first, and a portable copy's missing password
  is asked for at start-up.
- **2026-10-21** — **A search reaches an IMAP server too:** the local index only knows the words
  of what this computer keeps, so a Gmail account's search would stop at its window. The query's
  clauses now become `SEARCH` keys (`TEXT` for words anywhere, `FROM`/`SUBJECT`/`BODY`, `OR TO …
  CC …` for `to:`, `UNSEEN`/`FLAGGED`/`KEYWORD`, a leading `-` as `NOT`), a word outside ASCII
  going as a literal under `CHARSET UTF-8`. Each folder is `EXAMINE`d and searched, the newest two
  hundred hits kept, and the headers of those this computer doesn't have are fetched into the
  folder's cache — so a hit older than the window can be opened, kept, marked or moved like any
  other, and arrives merged with the local results the way Exchange's do. A search of all folders
  asks the biggest first (Gmail's All Mail holds everything) and stops at eighty folders or five
  seconds; JMAP and the browser still answer from this computer alone.
- **2026-10-22** — **Exchange that stays signed in:** from an audit of a large on-premises
  mailbox's log (110 folders, 550,000 items). A connection lost to an error kept its NTLM sign-in
  noted apart from it, so the next request went unsigned on a new connection and its 401 asked
  for a password that was right; the sign-in now lives with its connection. A kept connection
  idle past 45 s isn't reused (IIS drops them at 120 s, proxies sooner), and one the server closed
  before reading the request sends it again, once, on a new one. A chunked answer's last empty
  line is read with it (left behind, the next answer failed as "bad status line"), and empty lines
  before a status line are skipped. Exchange busy (`ErrorServerBusy`, HTTP 503) is waited out as
  it asks, then asked again; a SOAP fault refuses one request without dropping the connection
  and restarting the full sync. A reader connection's failure no longer asks for the password
  (its folder is read on the main connection); "Connecting…" doesn't stay up after a connection
  made for a message, a picture or the rules; the Inbox is polled on a deadline that work in the
  window no longer pushes back. Rules: Exchange refused every rule sent from here (`<t:Delete/>`,
  an empty boolean, and tests out of the schema's order), and the refused rule was sent again,
  and the rules read again, after every folder; Create rule from a message belongs to the
  message's account, moves to a folder to choose and goes to the server, and a colleague's mail
  carries its SMTP address (`PidTagSenderSmtpAddress`). Drag-out works on XWayland again.
- **2026-10-21** — **v0.21**: started again, Quatrelle brings its window forward (minimized or
  behind other windows) instead of opening a second one on the same data, as PdfCraft does;
  another data folder (`--data`, `--sample`, a portable copy) still opens its own.
- **2026-10-21** — **v0.21**, issues #1 and #4 again, reproduced with the app against fake IMAP
  (1,500 messages) and Exchange servers first: a shared mailbox in the folder pane and the
  calendar as a mailbox of its own, moved like an account; a Google invitation's notes read with
  their links, and every dialog within the window (its title and buttons were off a 2160-pixel
  screen); Show more at the end of a folder and Search the server at the end of a search, for
  mail past what's kept (IMAP); online only explained again, its password asked for at start,
  and a warning before closing would lose unsent mail or changes.
- **2026-10-21** — **v0.21**, the gaps audited again (craft-fonts still has no Hebrew or Korean
  font, the `.pst` crates no new release): gap #7 closed, Search the server for JMAP, Microsoft
  Graph and EWS accounts, their finds kept while the server has them; the browser searches its
  server and shows more; JMAP folders keep more than one `Email/get` (they kept 100 to 500); the
  reading pane's header fits a narrow pane. Checked against fake Exchange and JMAP servers, in
  the app and the browser.
- **2026-10-21** — **v0.21**, a reliability round, each problem first reproduced on the fake
  servers or in the app: edits made mid-sync are now persisted on the spot and replayed
  over refreshed folders (they could be undone on screen, or lost on exit); Graph `$batch` entries
  now request immutable ids (mismatched ids caused endless re-downloads and dropped server-search
  hits), are capped at 4 and retried on 429, and large first syncs read summaries from the listing;
  sibling message windows no longer drag each other around; non-recurring appointments sharing a
  UID stay separate; the 1604 placeholder year is read as "no year"; hidden Exchange folders are
  skipped; Online is shown once mail is current; dead IPv6 routes cost 3 s, not 20 s.
- **2026-10-22** — **v0.22**, Outlook's everyday tools, from a side-by-side look at Outlook's
  ribbon and calendar: calendar reminders (VALARM, mapped to EWS and Graph) with a Reminders
  window, snooze, dismiss and a desktop notification; blocked and safe senders applied as mail
  arrives (and to Junk, whose high-water mark is now set as soon as it's known: against a real
  IMAP server, mail filed there before Junk first changed was never seen arriving); Clean Up
  (messages a later reply quotes in full, conservative by design); Search Folders (with
  `larger:`/`smaller:` and `from:a,b` in the search language, sent to IMAP and JMAP servers too);
  Quick Steps with Ctrl+Shift+1…9 (digit shortcuts now read by key position, so Shift or AZERTY
  don't break them); the Work Week view; Insert date & time and Word count while writing. Found
  on the way: settings were written only on a clean close, so a crash or a forced quit lost
  every change (now kept every few seconds when changed); the log's terminal echo panicked once
  standard error went away (Quatrelle ended at the next log line), and the CLI did the same
  writing into a closed pipe; the ribbon's last buttons were cut off at 1336 px (labels now drop
  from the end as needed); a big calendar's expansion was quadratic. Tested against a GreenMail
  IMAP server in Docker as well as the sample mailbox.
- **2026-10-22** — **v0.22**, test builds update themselves (`apps/quatrelle/src/self_update.rs`),
  modelled on how the Craft launcher replaces its own copy and on Office's install-on-close: a
  check a minute after start and then about hourly with jitter (`If-None-Match`, so an unchanged
  list costs no quota), never an older or the same build, the installer for how this copy was
  installed, refused unless its size and SHA-256 match GitHub's per-file digest (or the release's
  `SHA256SUMS.txt`), installed by a detached helper after Quatrelle exits (logged to
  `updates/install.log`), and a day-long marker against reinstall loops. Only
  `release-wip.yml` turns it on (`QUATRELLE_UPDATE_CHANNEL=test`); `release.yml` is unchanged.
  Checked on Windows from a portable copy posing as 0.20.0: test build 18 downloaded, verified,
  unzipped over the folder (data kept) and reopened.
