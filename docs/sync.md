# How Quatrelle syncs

Quatrelle is local-first: the UI only ever reads the on-disk cache (`crates/store`), and per-account
worker threads (`crates/net/src/worker.rs`) keep that cache in step with the server. This page
describes the algorithm and the standards it follows.

## Per-folder state

For every folder the cache keeps (`FolderState`):

| Field | Meaning | Source |
|---|---|---|
| `validity` | UIDVALIDITY when the cache was filled | RFC 3501 §2.3.1.1 |
| `uid_next` | UIDNEXT at the last sync; UIDs at or above it are new | RFC 3501 §2.3.1.1 |
| `highest_modseq` | HIGHESTMODSEQ at the last sync (0 without CONDSTORE) | RFC 7162 §3.1 |
| `messages` | summaries (headers, flags, size, preview), newest first | |

Message sources live next to it as `<validity>/<uid>.eml`. A message is identified everywhere by
`(account, folder, UIDVALIDITY, UID)` (`MessageKey`), which RFC 3501 guarantees is never reused.

## A folder sync (`crates/net/src/sync.rs`)

1. **SELECT** the folder (with `(CONDSTORE)` when the server offers it). With QRESYNC enabled and a
   cached state, the SELECT carries `(QRESYNC (<uidvalidity> <modseq> <cached range>))` and the
   server answers steps 3 and 4 itself: `VANISHED (EARLIER)` lists the expunged UIDs and FETCH
   responses carry the changed flags (RFC 7162 §3.2.5), so only new mail is left to ask for.
2. **UIDVALIDITY check.** If it differs from the cache, every cached UID is meaningless: drop the
   folder's cache, including pending changes for it, and start over (RFC 4549 §4.1).
3. **Flag changes** for cached messages:
   - with CONDSTORE: `UID FETCH <cached range> (UID FLAGS) (CHANGEDSINCE <modseq>)` returns only what
     changed, and the fetch is skipped entirely when HIGHESTMODSEQ hasn't moved;
   - without: `UID FETCH <cached range> (UID FLAGS)` and compare.
4. **Deletions:** `UID SEARCH UID <cached range>` lists the survivors; everything else (and its cached
   source) is removed. With a time kept (`SyncOptions::since_days`, Outlook's Cached Exchange Mode
   "Mail to keep offline"), what the server received before it goes too, with its source
   (`Summary::received_at`, INTERNALDATE, not the message's own date), but for what a search of
   the server brought in.
5. **New mail:** `UID SEARCH UID <UIDNEXT>:*` (filtered, since `n:*` always returns the last message)
   then headers for those UIDs in chunks: `UID FETCH … (UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER])`.
   The first sync of a folder fetches only the newest `window` messages (1000 by default): of
   `UID SEARCH ALL`, or with a time kept of `UID SEARCH SINCE <day>` (what was received from the
   cutoff's day on; the few hours of that day before the cutoff aren't kept). A longer window, or
   a longer time, lists the folder again, once.
6. **Save** the new state atomically (temporary file + rename). Beside it, `kept.json` (IMAP's
   own, gone with the folder's cache) keeps the time the folder was listed with and the UIDs a
   search of the server brought in (newest 500), which stay however old while the server has them.
7. **Prefetch** the newest sources (40 by default, up to 2 MB each) with `BODY.PEEK[]` so they open
   instantly, can be searched, and fill in previews. Bigger or older messages download when opened.
   When every message kept is to be downloaded (`SyncOptions::full`), `sync::download_ahead`
   brings the rest, newest first, a budget at a time, any size.
8. **Counts:** `UID SEARCH UNSEEN` for the folder's unread count.

`PEEK` is used everywhere so syncing never marks mail as read.

## Download all (`account.downloadAll`)

Outlook's Download All: the engine lists the messages of an account (or one folder) that have no
source on this computer (`Store::has_body`), and asks the worker for them with `Job::FetchBody`,
32 at a time, sending one more as each answer comes back. That is the fetch already used to open a
message (`crates/net/src/worker.rs`), so every transport serves it: IMAP `BODY.PEEK[]`, JMAP and
Graph blobs, EWS `GetItem` with the MIME content, POP's copy. A source that arrives is indexed for
search as it is read (`Session::index_body`). `Session::download_progress` feeds the status bar,
messages already kept are never asked for again, and it stops by itself once the last one is here.

## The worker loop

- On start: connect (TLS, or STARTTLS with a fresh capability list), authenticate (`AUTHENTICATE PLAIN`
  with SASL-IR when offered, else `LOGIN`; non-ASCII credentials go as literals), `ENABLE QRESYNC`
  (or `ENABLE CONDSTORE` without it).
- Replay pending offline changes, then **sync all**: `LIST` (special-use roles from RFC 6154
  attributes, or the usual names), then every selectable folder, Inbox first. Between folders the
  worker serves body fetches and uploads queued edits, so the UI stays responsive during a long
  first sync. The status turns Online when mail is done; CalDAV/EWS calendars and contacts run
  afterwards.
- Then **wait for mail**: `IDLE` on the Inbox (RFC 2177) in half-second slices, so new jobs interrupt
  it at once; re-issued every 10 minutes (servers drop IDLE after 30). Without IDLE, poll every 2
  minutes. A full sync runs every 15 minutes.
- **Failures:** a broken connection is retried with exponential back-off (5 s → 5 min). A refused
  password stops retrying and asks the user (`NeedsPassword`). A refused command is reported and
  the cache is resynced.

### Reading folders on connections of their own

A full sync reads folders on up to `SyncOptions::parallel` connections of their own (the app asks
for 2, so three with the main one), each taking folders from a shared list. The main connection
stays free, so an opened message, a send or an automatic-reply change is answered while folders are
still being read; it also takes a folder of its own when only one is left.

- **Numbers (JMAP, Graph, EWS):** each reader gets `State::reserve` — its own copy with a block of
  message numbers kept for it — and `State::absorb`/`compact` give them back once it's done, so two
  connections never number two messages the same. Stored keys (folder states) stay valid.
- **Each folder is read with its own options** (`Worker::options_for`): one asked to keep more
  (Show more) keeps them on a reader too, rather than being cut back to the account's window.
- **A server that takes one connection** (or says it's busy): the reader that can't connect says so
  (`SyncDone::Refused`), the account finishes the round on its main connection, and `parallel` is
  turned off for that account until it's started again.
- **POP** reads its Inbox on one connection and never starts a reader.

## Offline changes (RFC 4549 "pending actions")

Flag changes, moves and deletes are applied to the cache immediately (the UI updates before the
server answers) and appended to `<account>/ops.json` by the pool on the UI side, not by the account
thread, which may be busy for minutes; a store-wide mutex guards the file's read-modify-write.
When connected, the worker replays them oldest first, deleting each entry once the server accepts
or rejects it, also between folders of a full sync. After a folder is re-read from the server,
queued ops that touch it are applied to the new cache again:

- flags: `UID STORE <set> +FLAGS.SILENT (…)` / `-FLAGS.SILENT`;
- move: `UID MOVE` (RFC 6851), else `UID COPY` + `\Deleted` + `UID EXPUNGE` (RFC 4315; plain
  `EXPUNGE` only as a last resort);
- delete permanently: `\Deleted` + `UID EXPUNGE`.

Keys whose UIDVALIDITY no longer matches are skipped rather than applied to whatever now has that
UID. If the server refuses a change, it is dropped, the folder's sync state is reset (forcing a full
flag comparison) and the user is told.

Folder changes (create, rename, move, delete) are pending actions too: `CREATE` + `SUBSCRIBE`,
`RENAME` (subfolders follow; the cache directories move with them), `DELETE`. Deleting a folder
first moves it into Deleted Items, like mail.

## Sending

Messages are built with `mail-builder` (Bcc never written into the message), submitted over SMTP
(implicit TLS on 465 preferred per RFC 8314, or STARTTLS on 587), then filed in Sent Items with
`APPEND (\Seen)` — except on Gmail, which files sent mail itself. Replies set `In-Reply-To` and
`References` (trimmed per RFC 5322 §3.6.4) and the original gets `\Answered` (or `$Forwarded`).

**Outbox.** A message that can't go out now (no network, refused connection, TLS failure, a
timeout, or an SMTP 4xx "try later") is written to `<account>/outbox/<id>.eml` with its envelope in
`<id>.json`, and the compose window closes as if sent. The worker tries again after connecting,
after every full sync and on *Send now*, oldest first, stopping at the first failure that may
clear up. A permanent refusal (5xx) keeps the message in the Outbox with the error until the user
edits or deletes it.

## JMAP accounts (`crates/jmap`)

An account with a JMAP session address (`AccountConfig::jmap`, RFC 8620/8621) syncs over HTTPS
instead of IMAP and SMTP, behind the same jobs and events. Each request is one POST of a batch of
method calls, signed in with Basic authentication or an OAuth bearer token.

- **Keys.** JMAP message ids are strings that stay the same across mailboxes. The worker numbers
  them as it first sees them (`<account>/jmap.json`, numbers never reused) and uses the number as
  the UID, with a constant UIDVALIDITY of 1, so the cache, the engine and offline changes are the
  same as for IMAP. Numbers no cached message or pending change uses are pruned after a full sync.
- **Folders** are named by their path of mailbox names joined by `/` (the Inbox is `INBOX`); the
  mailbox id behind each path is kept with the numbers. Roles come from the mailbox `role`.
- **A folder sync** is two requests: `Email/query` (newest first, up to the window) with an
  `Email/get` of the keywords by back-reference and the mailbox's counts, then the `headers` of
  the messages not cached yet. Mail that leaves the window leaves the cache. The newest sources
  are downloaded as blobs.
- **Changes** are `Email/set` patches (`keywords/$seen`, `mailboxIds/<id>`), destroys for
  permanent deletion, and `Mailbox/set` for folders.
- **Sending** uploads the message, imports it into Sent (`$seen`) and submits it with
  `EmailSubmission/set` from the identity for the sender; a refused submission removes the copy.
  Offline sends use the same Outbox. Drafts are imported with `$draft`.
- **Push.** Between jobs the worker holds the session's event stream open (RFC 8620 §7.3,
  `text/event-stream`), reading it in half-second slices so jobs never wait, the way IMAP holds
  IDLE; a `state` event syncs the Inbox. Servers without an event stream are polled every two
  minutes.

The transport is written once (`quatrelle-jmap`, async over an `Http` trait) and shared: the
desktop worker runs it over its own HTTP client, the browser app over `fetch`.

## Exchange Online through Microsoft Graph (`crates/net/src/exchange/graph.rs`)

An account with a Graph address (`AccountConfig::graph`) syncs with Exchange Online through the
Microsoft Graph REST API, signed in with an OAuth token that asks for Graph's mail permissions
(`Mail.ReadWrite`, `Mail.Send`, `MailboxSettings.ReadWrite`). It works where an organisation has
switched IMAP off.

- **Ids** are asked for in their immutable form (`Prefer: IdType="ImmutableId"`), so a message
  keeps its id when it moves; they are numbered like JMAP's, with the same folder paths.
- **Folders** come from `/me/mailFolders` and each one's `childFolders`, following
  `@odata.nextLink` pages on the same server only. The well-known folders (`inbox`, `sentitems`,
  `deleteditems`…) are looked up in one JSON batch for their roles; the Outbox and Exchange's own
  bookkeeping folders are left out.
- **A folder sync** pages through `/mailFolders/{id}/messages` with the summary fields
  (`LIST_FIELDS`; `internetMessageHeaders` excluded because it slows listings to the point of
  gateway timeouts). If at most 20 messages are new, each is then fetched with its internet
  headers via `$batch`, four entries per batch to stay within Exchange Online's per-mailbox
  concurrency, every entry carrying its own `Prefer: IdType="ImmutableId"` (a batch's outer
  headers don't apply to its entries). With more new messages the listing's fields are used as
  they are, and the References/List-Id headers are filled in when the source is downloaded.
  HTTP 429 answers, top-level or per entry, are retried after `Retry-After` (at most 4 attempts,
  20 s per wait). Sources come from `/messages/{id}/$value`; mail with attachments downloads when
  opened.
- **Changes** are `PATCH`es (`isRead`, `flag`), `move`, `DELETE` and folder calls, batched.
- **Sending** posts the MIME source to `sendMail`, which files the copy in Sent Items; recipients
  only on the envelope go in a `Bcc:` header, which Exchange strips. Drafts are made from MIME and
  moved where they belong.
- **Automatic replies** are the mailbox settings' `automaticRepliesSetting` (scheduled when the
  dates are set). Graph doesn't tell a mailbox's size to its owner, so there is no quota.
- **Rooms and free/busy** (`graph_pim/scheduling.rs`): the room lists and their rooms from the
  Places API (`/places/microsoft.graph.roomList`, then each list's `/rooms`), or, where the
  sign-in hasn't `Place.Read.All`, from `/me/findRoomLists` and `/me/findRooms(RoomList=…)` (and
  `/me/findRooms` alone when there are no lists); when people and rooms are busy from
  `POST /me/calendar/getSchedule`, twenty schedules a call, each one's items or its refusal. A
  room booked goes as an attendee of `type: resource`.

## On-premises Exchange through EWS (`crates/net/src/exchange/ews.rs`)

An account with an EWS address (`AccountConfig::ews`, `https://<server>/EWS/Exchange.asmx`) syncs
with an organisation's own Exchange server (2010 SP2 and later) through Exchange Web Services, for
servers that don't offer IMAP. Every call is SOAP over HTTPS.

- **Signing in** follows what the server's first `401` offers: NTLMv2 (`crates/net/src/exchange/ntlm.rs`,
  as `NTLM` or `Negotiate`) on a connection kept open, since NTLM signs in the connection rather
  than each request; or Basic. The user name is `DOMAIN\user` or an address; the password stays
  in the keychain. Plain `http://` is refused except for local test servers.
- **Folders** come from one deep `FindFolder` under the message folder root, in pages; the
  distinguished folders (`inbox`, `sentitems`, `deleteditems`…) are looked up in one `GetFolder`
  for their roles, and the Outbox, Exchange's bookkeeping folders and calendar, contact and task
  folders are left out.
- **A folder sync** first asks the folder's counts and when anything in it last changed
  (`PR_LOCAL_COMMIT_TIME_MAX`) in one `GetFolder`. Found as it was when last read (same stamp,
  counts, window and download-ahead, within six hours), the folder is left alone. Read before,
  only its changes since are asked (`SyncFolderItems` from the state kept in the numbering's
  `marks`, 512 a page): new items (their details then with `GetItem`, fifty at a time), read and
  flag marks, deletions. Otherwise — the first read, a state refused, a server without stamps —
  the folder's state is taken, then its newest items are listed with their read, flag
  (`PidTagFlagStatus`) and draft (`PidTagMessageFlags`) state and those not cached yet get their
  details. Sources downloaded ahead go ten (8 MB) to a `GetItem` with the MIME content. Item ids
  are numbered like JMAP's; Exchange gives a moved item a new id, so it shows up as a new message
  in its new folder. The Inbox is looked at every thirty seconds (one `GetFolder` when nothing
  came). The numbering is written only when it changed.
  A folder too big for its state to be taken (more than 200 pages of changes) is listed without
  one, and the mark says so (`too_big`): it isn't asked again for a week (asked at every pass,
  two public folders of 100,000 reports each took a quarter of an hour).
- **Mail kept, as Outlook's Cached Exchange Mode** (the owner's rule): what came in the time
  chosen in Options (`Prefs::keep_days`, two years unless chosen; `SyncOptions::since_days`).
  The listing asks only for mail received since then (`FindItem` restricted on
  `item:DateTimeReceived`), and a folder followed by its changes lets go of what has grown
  older, with its source (`Summary::received_at`, the server's time, not the message's own
  date). A search of the server's finds stay while they're there. Every message kept is
  downloaded whole in the end (`SyncOptions::full`, no size limit): the newest few come with
  the folder's list, the rest twenty at a time while nothing else is asked (`ahead_step`,
  every kind of account), the Inbox looked at every half minute meanwhile (IDLE and push wait
  until it's done), only their sources written by Exchange (what of a folder is missing is
  worked out once, `ews::AheadTodo`, and trusted while `Store::folder_version` says its list
  is the same). Downloading ahead pauses while the disk the mail is kept on has less than the
  larger of 5 GB and a fiftieth of it free (`worker/ahead.rs`, told once, looked at again every
  ten minutes): two years of a support team's shared mailbox was 37 GB. What's older
  than the time kept is read on the server when asked for (Show more: `Job::Older`, a hundred
  at a time, `FindItem` restricted to before then), held in memory by the session
  (`Session::older`) and shown after the folder's own list; one opened is read for showing
  (`Job::ReadOnline`), not kept nor indexed.
- **Public folders and shared mailboxes** (`Worker::read_online`): a shared mailbox is kept as
  the account's own mail unless the account doesn't keep them (`AccountConfig::shared_online`,
  Outlook's "Download shared folders" off); a public folder (under `Public Folders`,
  `quatrelle_model::is_public_folder`) is kept only when it's a Favorite and the account keeps
  public Favorites (`AccountConfig::public_favorites_kept`, Outlook's "Download Public Folder
  Favorites", off unless chosen). The session writes the public folders followed, per account,
  as `public-followed.json` (only once Favorites were set, unless public Favorites aren't kept:
  then none); the worker reads it at each full pass and walks them last, after the mailbox's
  own folders and its shared mailboxes. A folder read online isn't walked: opening it asks for
  it (`Job::SyncFolder`), and it's listed only (`SyncOptions::list_only`: its newest 500, Show
  more for more, nothing downloaded ahead, no state asked for). What an earlier pass kept of
  one beyond that is let go at the next full pass, and its numbers with it.
- **Told of changes as they happen** (`worker/listening.rs`, `ews/listening.rs`), as Outlook's
  Cached Exchange Mode is: the account's main connection subscribes to every folder of its
  mailbox (`Subscribe`, `SubscribeToAllFolders`) and to the folders of each shared mailbox kept
  here (by id, anchored to that mailbox), then holds `GetStreamingEvents` open on a connection of
  its own (signed in for it; ten minutes an answer, asked again when the server ends it or after
  eleven silent minutes, of the server the subscription lives on: `X-BackEndOverrideCookie`).
  The answer is SOAP envelopes one after the other, read a slice at a time
  (`http::BodyStream`) between jobs, between two downloads ahead and between two folders of the
  full sync (seconds over a hundred folders). What came with a notification is taken with it,
  without waiting for more: what comes while the folders are read is read next. A message's
  event syncs its folder and the one it left; a folder's own `ModifiedEvent` syncs it; a folder
  made, deleted or moved where the account has folders relists them first. So new mail, mail a
  server rule filed elsewhere, a read mark set on the phone and a shared mailbox's mail come at
  once, not at the next full sync. The list is told of as soon as it's saved
  (`ews::sync_folder_then`), before the newest messages are downloaded whole: new mail shows
  once its header is in, three requests after the notification (`GetFolder`, `SyncFolderItems`,
  `GetItem`), its preview a moment later. The log says how long each read took once told
  (`told of changes in INBOX, read in 412 ms`, its lines to the millisecond). A change in the account's own calendar or contacts (their ids asked once with
  `GetFolder`) runs their sync instead — a meeting accepted on the phone, or one a colleague moved,
  is in the calendar seconds later: two seconds after it's told (accepting a meeting is several
  changes), once a minute at most (the pass reads every calendar ticked again, seconds on a big
  mailbox). On subscribing, the Inboxes are synced once (what came before isn't told).
  While listening, the Inbox is still looked at every five minutes in case the server went
  quiet. A server that refuses `Subscribe` (older than Exchange 2010 SP1) is polled every half
  minute as before; a stream that fails (dropped, a subscription forgotten, ended at once) is
  tried again after 5 s, doubling up to 15 minutes, polled meanwhile.
- **Changes** are `UpdateItem`, `MoveItem`, `DeleteItem` and the folder calls, fifty items a call.
- **Categories** are kept by name, as Outlook keeps them: every answer that carries an item's marks
  asks for `item:Categories` too, read as the name's keyword (`keyword_for`; a category of this
  computer, `Work` as `$label2`, is matched by its name as well). The server's list replaces the
  item's categories; Quatrelle's own keywords (`$Pinned`, `$Junk`…) stay. A category put on or
  taken off here reads each item's list and writes it back with the name in or out
  (`SetItemField`, or `DeleteItemField` for none). After each PIM pass, the master category list
  (`GetUserConfiguration` `CategoryList` on the Calendar, MS-OXOCFG) gives each category Outlook's
  colour and adds those not made here. A folder read before categories were (its mark's
  `categories` false) asks once for its categorized messages alone (`Exists item:Categories`)
  rather than being listed again. Graph accounts keep categories on this computer still.
- **Sending** is `CreateItem` with the MIME source and `SendAndSaveCopy` (the copy goes to Sent
  Items); envelope-only recipients go in a `Bcc:` header. Drafts are `CreateItem` with `SaveOnly`,
  marked unsent.
- **Automatic replies** are `Get`/`SetUserOofSettings`.
- **Outlook's copy of the rules:** while Outlook keeps its own copy of the Inbox rules in the
  mailbox, `UpdateInboxRules` answers `ErrorOutlookRuleBlobExists`. The Rules dialog then offers
  to replace it: once the user agrees (Outlook's client-only rules are lost), the account's rules
  go again with `RemoveOutlookRuleBlob`, on that change only.
- **Certificates from the address book:** a message to encrypt whose recipient has sent no
  signed mail asks `ResolveNames` (`ReturnFullContactData`, `ContactDataShape="AllProperties"`,
  the directory) once per address, as Outlook finds them in the GAL: `MSExchangeCertificate`
  (Active Directory's `userCertificate`, X.509) and `UserSMIMECertificate` (a PKCS #7 bundle).
  The one made out to that address, valid today and the last to run out, is kept like one from
  signed mail; the message is sent again once it's there.
- **Folder Size and the quota:** one `FindFolder` (deep, from `msgfolderroot`) asks each folder's
  `PidTagMessageSizeExtended` (0x0E08, its own items' bytes) with the hidden mark, so Exchange's
  own folders stay out as Outlook leaves them out; a `GetFolder` of `root` and `msgfolderroot`
  asks the mailbox's limits (`PidTagStorageQuotaLimit` 0x3FF5, `PidTagProhibitSendQuota` 0x666E,
  `PidTagProhibitReceiveQuota` 0x666A, in KB), which a server may keep on the mailbox alone. The
  sizes added up are the space used on File › Info, measured against the receiving limit (else
  sending's, else the warning). Exchange spells tags its own way (`0xe08`): they're compared as
  numbers.
- **Finding the server:** an account added without one asks Autodiscover (POX, MS-OXDSCLI) at the
  domain's `autodiscover.` host and the domain itself, signed in as the user, following one
  redirect; the EWS address found is kept with the account.
- **Calendars and contacts** (`crates/net/src/exchange/ews_pim.rs`) are the mailbox's Calendar and
  Contacts folders, synced like CalDAV / CardDAV: changes here first (appointments updated field
  by field, contacts made again, never any meeting mail), then both folders read in full.
  Recurrences go through Graph's pattern-and-range shape, so one RRULE mapping serves both.
  Away from the calendar, only the account's own calendar is read; other people's appointments
  are set aside five minutes after it's left (at once at the start), as Outlook reads a shared
  calendar from its offline file only to show it (`session/pim/parking.rs`): out of memory and
  of `pim.json`, into `pim-parked.json`, read back when the calendar is shown (a calendar read
  again meanwhile keeps what was read). One changed here and not yet sent stays. On the owner's
  mailbox, 25,390 of 25,761 appointments: the session holds 49 MB rather than 105 away from
  the calendar, and a save of the calendar and contacts writes 1.6 MB rather than 35.
- **Rooms and free/busy** (`ews_pim/scheduling.rs`), on the account's second connection so a
  long sync never holds them: `GetRoomLists`, then `GetRooms` for each list (an empty answer
  where the organisation set no lists up: the engine then offers the rooms the calendar knows);
  `GetUserAvailability` for the organizer, the guests and the rooms over the day shown, with a
  `TimeZone` of no bias (standard and daylight alike) so every time is UTC, the window a day at
  least (Exchange answers for nothing less) and two months at most, a `DetailedMerged` view
  (the events where the person allows it, the merged digits otherwise), each mailbox's refusal
  (`ErrorNoFreeBusyAccess`) read on its own. A room booked goes in `t:Resources`, after the
  guests as the schema orders them, and reads back as a resource attendee; a room taken off a
  meeting is taken off the item (`DeleteItemField calendar:Resources`).

## Calendars and address books (`crates/net/src/protocols/dav.rs`)

Password accounts sync calendars over CalDAV (RFC 4791) and address books over CardDAV
(RFC 6352). The homes are found as RFC 6764 describes: an SRV record, then the `/.well-known/`
addresses on the mail domain, its `caldav.`/`carddav.` hosts and the IMAP host, through the
user's principal; a server with none is asked again a day later. A sync first sends what changed
here (deletions with `If-Match`, edits guarded by the item's etag so a newer copy on the server is
never overwritten, new items with `If-None-Match: *`), then reads each collection whose `getctag`
moved: the etags by `PROPFIND`, and the new or changed items by `calendar-multiget` /
`addressbook-multiget`. The engine merges the result: what changed here meanwhile stays to be sent.

**Invitations** travel by mail (iTIP over iMIP, RFC 5546/6047): a `text/calendar` part with
`METHOD:REQUEST` shows as an invitation card (Accept, Tentative, Decline), which updates the
calendar and mails a `REPLY` to the organizer; replies and `CANCEL`s update the event; events with
guests send them requests when saved and cancellations when deleted.

## Automatic replies on IMAP servers (`crates/net/src/protocols/sieve.rs`)

On IMAP hosts, automatic replies are a Sieve script (RFC 5228) with a `vacation` action
(RFC 5230), limited to dates with `currentdate` (RFC 5260), kept over ManageSieve (RFC 5804). A
script the user already runs stays in charge of their other rules: ours `include`s it (RFC 6609).

## Outlook data files (`crates/net/src/data_files/pst.rs`)

*Open Outlook data file* creates an account with no server named after the file and reads the
`.pst` (MS-PST, Unicode or ANSI, with Microsoft's `outlook-pst` crate) on a thread of its own:
the mail folders under the top of the mailbox become folders, and each mail item is written
again as a standard message from its properties (addresses, dates, text and HTML bodies,
attachments, pictures shown by `cid:`, embedded messages, read and follow-up flags), keeping its
original internet headers when it came in by mail. The file is hostile input: the reader runs
behind `catch_unwind`, unreadable items are skipped and counted, and sizes are capped.

Outlook keeps its data files open, with byte-range locks, while it runs (in the notification area
too): opening one then says so instead of trying the older ANSI format. An `.ost` (client magic
`SO`) is Outlook's offline copy of a server account and is refused with that explanation. The RSS
Feeds folder (`IPF.Note.OutlookHomepage`) isn't mail, and an empty Outbox (the store's 0x35E2) is
skipped.

## POP accounts (`crates/net/src/protocols/pop.rs`)

A POP account (RFC 1939; SSL/TLS, `STLS` or none; `USER`/`PASS` or `AUTH XOAUTH2`) has the
folders Outlook gives one, all on this computer: Inbox, Drafts, Sent Items, Deleted Items and
Junk Email. Syncing the Inbox opens a session, lists `UIDL`, downloads (`RETR`) each message
not seen before into the cache, saving the seen ids (`pop-seen.json`) after every message, and
quits; with `popDelete` each one is then marked `DELE`, else a copy stays on the server. Lines
and messages are capped and nothing with CR or LF is ever sent. Every other change (flags,
moves, deletes, drafts, the sent copy) is made in the cache only; the worker carries out moves
itself, since no server copy comes back. `account.add` with `pst` holds the account's first
connection until that data file has come into it (Outlook's "use an existing data file").

## In the browser (`apps/quatrelle-web`)

The web build keeps the cache in memory, mirrored to IndexedDB, and runs each account's jobs in
order over JMAP and `fetch`. New mail comes by push: the session's event stream is read through
`fetch` (an `EventSource` can't send the `Authorization` header), a `state` event syncs the Inbox,
and the Inbox syncs once whenever the stream opens. Without push the Inbox is polled every two
minutes; everything syncs every eight checks either way. Passwords stay in the tab; after a
reload the account asks for its password again.
