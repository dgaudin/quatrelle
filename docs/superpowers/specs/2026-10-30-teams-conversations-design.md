# Teams conversations drawn by Quatrelle (1.1, part A2.1)

Status: approved in conversation on 2026-10-30, section by section; reviewed independently the
same day (Graph facts checked against Microsoft's v1.0 documentation). This revision takes the
review in and records the owner's ruling on polling.

## Why

The owner works on Linux, where Microsoft no longer ships a Teams desktop client. Meetings and
calls already open in a Chrome app window (part A3, `2026-10-29-meetings-in-chrome-design.md`).
What is missing is everyday Teams messaging. The owner's organisation (Proginov) is on
Microsoft 365 and the owner is its administrator, so an application can be registered in its
Entra ID.

Teams natively (A2) comes in four parts, each with its own spec, plan and test builds, in the
owner's order (2026-10-30):

1. **Conversations** (this spec): one-to-one, group and meeting chats, with the presence of the
   people shown in the list.
2. Team channels.
3. Setting one's own presence.
4. Team files.

WhatsApp (A1) comes after A2.

## Owner's decisions (2026-10-30)

- The first version of conversations already has, beyond listing, reading, replying and a
  notification per new message:
  - new conversation;
  - images and files (see and send);
  - reactions and @mentions;
  - editing and deleting one's own messages.
- **A full local copy** of the chats, as Outlook's Cached Exchange Mode does for mail: instant
  opening, reading offline, and searchable in Quatrelle.
- **Organisation 1**: Teams is a service of Quatrelle of its own, not tied to any mail account. It
  has one Microsoft 365 sign-in, a card in File › Options, its own data file and its own module in
  the left rail. The owner's mail is on-premises Exchange (EWS, NTLM), unrelated to the Microsoft
  365 sign-in. One Teams identity at a time (a second organisation is out of scope).
- The design sections below (architecture and connection; synchronisation; the screen; errors,
  privacy, tests and delivery) were each approved as presented.
- **Polling.** The owner was told that Microsoft's Teams API rules allow polling a resource once a
  day, and that the compliant alternative, change notifications, needs a public endpoint or
  Azure. Offered delivery through Azure Event Grid to a storage queue, the owner kept the
  approved cadences (every 10 s, and so on): there is no Azure subscription. The owner accepts
  the risk; see Risks › Polling, accepted by the owner.

## What the owner sees

### File › Options › Teams (desktop only)

A new card, after Applications. It holds:
- "Use Teams in Quatrelle", a switch (off at first). Off, nothing of Teams runs: no request, no
  polling, no module.
- The application's two identifiers:
  - "Application (client) ID";
  - "Directory (tenant) ID", a GUID or the tenant's verified domain.
  Both are pre-filled when the build carries defaults (repository secrets
  `QUATRELLE_TEAMS_CLIENT_ID` and `QUATRELLE_TEAMS_TENANT`, passed by `release-wip.yml` as the
  Microsoft mail ones are) and editable otherwise.
- "Sign in": it opens the browser on Microsoft's sign-in (see Connection). Once signed in, the
  card shows the account's name and address, and a "Sign out" button.
- Notifications: "Notify me of new messages" (on), "Only when I'm mentioned or in a one-to-one
  chat" (off).
- A link to the registration guide (`docs/teams.md`, below).

### The Conversations module

- The left rail gains "Conversations", with the Fluent `chat_20_regular` icon (added to
  `assets/icons/fluent/` with its `ATTRIBUTION.md` row, like every Fluent icon). It carries a
  badge with the count of unread messages (99+ past 99), hidden chats not counted.
- The module is shown only when Teams is switched on. `Module` (in
  `crates/engine/src/session/pim.rs`) gains `Conversations`; if the remembered module is
  `Conversations` while Teams is off or signed out, Quatrelle opens on Mail instead of an empty
  screen.
- Shortcuts while the module is open:
  - Ctrl+N: new conversation;
  - Ctrl+E: search the conversations, as the mail module's search box does (Outlook's key);
  - Alt+↑/↓: previous or next conversation.
  Ctrl+F stays Forward in mail and is not used in the module.

**The list (left pane)**
- One row per chat:
  - picture (the person's photo, else coloured initials);
  - a presence dot (one-to-one chats);
  - the name (the other person; a group's topic, else its members' first names);
  - the last message ("Paul: it's fixed");
  - its time.
- Chats with unread messages are bold. Meeting chats carry a calendar icon. Newest activity first.
- Hidden chats are synced like the others, so they reappear at once when Teams unhides them on a
  new message, but they are neither listed nor announced.
- A chat the user has left, or that was deleted, stays listed read-only (see Synchronisation).
- At the top:
  - a search box: the chats' names and, on the local copy, their messages' text, answered by the
    Teams worker (see Search);
  - "New conversation".
- An empty state ("No conversations yet") and a "Signing in…" / "Copying your conversations… N of
  M conversations" state during the first copy.

**The conversation (right pane)**
- Header:
  - the name;
  - for one-to-one chats, the presence ("Available", "Busy", "Away", "Do not disturb",
    "Offline");
  - the members (a count, the list on click);
  - "Call" (see Calling).
- Messages:
  - grouped by day ("Today", "Yesterday", the date);
  - consecutive messages of one sender grouped under one name and picture;
  - one's own on the right on a light accent tint, others' on the left with name and picture;
  - each with its time.
- System messages (members added or removed, a meeting started or ended, the chat renamed) are
  one centred, dim line.
- An edited message (Graph's `lastEditedDateTime` set) shows "(edited)". `lastModifiedDateTime`
  is not used for this: a reaction moves it too. A deleted one (`deletedDateTime` set) shows
  "This message was deleted".
- Reactions appear under a message as chips (emoji and count). A click adds or removes one's own
  reaction, and hovering lists who reacted (by name, from the chat's members).
- Hovering a message offers:
  - six quick reactions (👍 ❤️ 😆 😮 😢 😡);
  - "Copy";
  - on one's own messages, "Edit" and "Delete" (the latter after a confirmation).
- Rich text, from a filtered HTML subset (see Message text): bold, italic, underline,
  strikethrough, links, bulleted and numbered lists, quotes, inline code and code blocks, emoji.
  @mentions are highlighted, more strongly when they name the user.
- Images:
  - images sent in Teams (hosted contents) show as thumbnails, fetched when the message is first
    shown, then kept, and open full size in Quatrelle's picture viewer;
  - an image from an outside site (e.g. a GIF) waits for a click, under the same privacy rule as
    mail: never load remote content without an explicit user action.
- Files: a card per file (icon by type, name, size when known). A click downloads the file
  through Graph to Quatrelle's attachments folder and opens it with the system's app; "Open in
  browser" opens its web link.
- Content Quatrelle doesn't draw (cards, polls, Loop components) shows as "This content can be
  seen in Teams" with an "Open in Teams" link: the chat's `webUrl` (a chat message's own `webUrl`
  is null), opened in the Chrome app window of part A3.

**Calling**
- "Call" builds `https://teams.microsoft.com/l/call/0/0?users=<address>,<address>…` with the
  other members' addresses (their `email` from the member list), percent-encoded.
- A new `is_teams_call_link(url)` sits beside `is_teams_link` in
  `crates/model/src/ical/notes.rs`: a link on `teams.microsoft.com` whose path starts with
  `l/call/0/0` and whose query has `users=`. The link is checked with `is_teams_call_link` and
  `launchable_link`, then opened in the Chrome app window of part A3 with the profile chosen
  there.
- The A3 launcher accepts either a meeting link (`is_teams_link`) or a call link
  (`is_teams_call_link`). `is_meeting_link` is unchanged: a call link isn't a meeting.
- Step 0 confirms the deep-link format before delivery 4 builds on it.

**Writing**
- A box at the bottom of the conversation, several lines. Enter sends, Shift+Enter makes a new
  line, Ctrl+B and Ctrl+I make bold and italic.
- The emoji button reuses the picker of 1.1. The paperclip attaches a file (at most 10 per
  message, 100 MB each, Teams' limits). A pasted picture is added as an inline image.
- Typing "@" offers the chat's members.
- ↑ in an empty box edits one's last message. Esc cancels an edit.
- A message too long for Teams (the HTML written is capped at 80 KB, images apart; Teams takes
  about 100 KB) isn't sent: the box says so and keeps the text.
- A message that couldn't be sent stays in the conversation marked "Not sent", with "Retry" and
  "Delete".

**New conversation**
- A dialog with "To:". It searches the organisation's directory (name or address) and adds people
  as chips, plus an optional topic for a group.
- If a one-to-one chat with that person exists, it opens that chat instead of creating one (Graph
  itself returns the existing one-to-one chat on `POST /chats`). For a group, Quatrelle looks in
  its local copy for a group chat whose members are exactly those people plus the user, and opens
  it if found: Graph doesn't do that for groups.
- The chat is created on the first message sent.

**Notifications**
- A desktop notification per new message from someone else, titled with the sender and chat
  ("Marie Durand", or "Paul in Support team"), with the text's first line.
- No notification:
  - for the chat open in a focused window;
  - during the first copy or a history backfill;
  - for hidden chats;
  - when the setting says so.
- A mention is marked in the title ("Marie Durand mentioned you").
- As for mail, the engine decides and the UI shows: the engine applies these rules and queues the
  announcements; the UI drains the queue each frame into `Services::notify`
  (`crates/ui-egui/src/services.rs`), which the engine can't reach.

## Connection

- **Sign-in:** OAuth 2.0 authorization code with PKCE over a loopback redirect (RFC 8252 +
  RFC 7636), the same flow Quatrelle already uses for Microsoft mail (`crates/net/src/oauth.rs`,
  `authorize`, which listens on `http://127.0.0.1:<port>/`). The authority is
  `https://login.microsoftonline.com/<tenant>/` with the tenant from Options. If the browser
  doesn't open, the sign-in address is shown (printed by the CLI) to be opened by hand.
- **Provider:** a new `pub fn teams_provider(client_id: &str, tenant: &str) -> Provider` in
  `oauth.rs` (beside the private `microsoft`, which mail builds from an account through
  `provider_for_account`).
- **Scopes** (delegated), written in full in the provider's `scope` as `GRAPH_SCOPE` is:
  `https://graph.microsoft.com/User.Read https://graph.microsoft.com/User.ReadBasic.All
  https://graph.microsoft.com/Chat.ReadWrite https://graph.microsoft.com/Presence.Read.All
  https://graph.microsoft.com/Files.ReadWrite https://graph.microsoft.com/Files.Read.All
  offline_access`; `openid email` is added by `identity_scope`.
  - `User.Read`: the user's own identity (`GET /me`);
  - `User.ReadBasic.All`: directory search, names, photos;
  - `Chat.ReadWrite`: read chats, members and messages, send, mark read, edit and delete one's
    own, react, create chats (it covers `ChatMessage.Send` and `Chat.Create`, so neither is
    asked);
  - `Presence.Read.All`;
  - `Files.ReadWrite`: upload to one's OneDrive and share;
  - `Files.Read.All`: open files others shared.
  Part 1 asks for all of them at once, so later parts don't force a new consent. A refresh asks
  for the whole list again; a scope added later means a new consent (as `docs/oauth.md` says for
  mail). Step 0 confirms the list. None of these permissions needs admin consent.
- **Identity:** right after sign-in, `GET /me?$select=id,displayName,mail,userPrincipalName`
  gives the user's object id and names. The tenant id comes from the ID token's `tid` claim (a new
  `id_token_claim(token, "tid")` beside `id_token_user`), else from any chat's `tenantId`. Both
  are kept with the Teams settings (they aren't secrets) and are used by `markChatReadForUser`,
  `softDelete`, "one's own message" (`from.user.id` equal to the object id) and "mentions me".
- **Tokens:**
  - The refresh token lives only in the system keychain, through `Credentials::get_secret`/
    `set_secret` on a fixed pseudo-account, as `llm::keyring()` does for the language model: a new
    `teams::keyring()` in `net` makes an `AccountConfig` with id `teams`, entry `refresh`. Access
    tokens stay in memory.
  - Every Teams request (chats, messages, presence, photos, files) goes through the one Teams
    worker, and renewal happens under its one mutex, as the mail worker's `auth` and
    `renew_token` do, so two renewals never race.
  - An access token refused once (401) is renewed and the request retried once, before any
    "sign in again". This closes the two gaps recorded on the carnet (item graph-jeton) for the
    Teams connection.
  - A missing scope is seen from the token response's `granted` (`Tokens.granted`), without
    decoding the token.
- **The owner's registration** (written up step by step in a new `docs/teams.md`, in English, with
  the French UI names in the conversation):
  1. Entra admin center › App registrations › New registration: single tenant.
  2. Platform "Mobile and desktop applications", redirect URI `http://localhost`; then, in the
     manifest (`replyUrlsWithType`), also `http://127.0.0.1` and `http://127.0.0.1/`. Quatrelle
     listens on `http://127.0.0.1:<port>/`; Microsoft ignores the port but matches host and path,
     and the portal refuses an `http` loopback address, hence the manifest. Turn on "Allow public
     client flows". No client secret. Same as `docs/oauth.md` step 2.
  3. API permissions: the delegated scopes above.
  4. Optional: "Grant admin consent for Proginov". None of these permissions needs it; it spares
     every user the consent page, and is required only if the tenant forbids user consent.
  5. Copy the Application (client) ID and the Directory (tenant) ID into Options › Teams, or into
     the repository secrets.

## Data

- **Model** (`crates/model/src/pim/teams.rs`, pure and wasm-safe, no clock: times come from
  `Session::clock`; written `teams::` below):
  - `teams::Chat`: id, kind (one-to-one / group / meeting), topic, members, the user's last-read
    time (`viewpoint.lastMessageReadDateTime`; `0001-01-01T00:00:00Z` means never read), hidden
    (`viewpoint.isHidden`), left (no longer in the list), last activity, web URL, tenant id;
  - `teams::Member`: user id, display name, address;
  - `teams::ChatMessage`: id, chat id, sender (a user, an application such as a bot, or none),
    created time, modified time (`lastModifiedDateTime`, the sync watermark, moved by reactions
    too), edited time (`lastEditedDateTime`, shows "(edited)"), deleted time, kind (message /
    system event), body as `ChatText`, mentions, reactions, attachments, hosted images;
  - `teams::Reaction`: emoji, user ids. Graph reads a reaction as a name (`like`, `heart`,
    `laugh`, `surprised`, `sad`, `angry`) or as Unicode; both map to the six quick reactions, and
    any other is kept as it came. Step 0 prints which forms it sees.
  - `teams::Attachment`: reference file or hosted image or unsupported card;
  - `teams::ChatText`: the runs-and-blocks text (it lives in `model`, which `mime` depends on);
  - `teams::ChatRow`: what the list shows of a chat (id, name, kind, last line and time, unread
    count, hidden, left);
  - `teams::Op`: an outbox operation;
  - `teams::Presence`: available, busy, away, do not disturb, offline, unknown;
  - `teams::Picture`: a hosted image (chat id, message id, hosted content id) or a user's photo
    (user id);
  - `teams::Status`: signed out, signing in, copying (N of M conversations), up to date, offline,
    sign in again, permission missing (which one);
  - the payloads of the jobs and events below.
- **Keys:** a message is known by (chat id, message id) everywhere: store, search index, outbox
  references, reactions. Graph's message ids are unique only within a chat. A hosted image is
  known by (chat id, message id, hosted content id).
- **Message text:** Teams sends `body.contentType` `html` or `text`. A new parser in
  `crates/mime/src/teams_html.rs` (beside `html.rs`, pure) turns HTML into `ChatText`; `text` is
  taken as plain text. It keeps only the allowed subset (bold, italic, underline, strike, links
  with `http(s)` only, lists, quotes, code, `<at>` mentions, `<emoji>` as its Unicode alt text,
  `<img>` pointing at hosted contents, `<attachment id>` placeholders). Every other tag is dropped
  with its text kept, and scripts and styles are dropped with their content.
  - It has caps on input size (256 KiB), depth and output size.
  - The writer does the reverse for sending: `ChatText` to Teams HTML, escaping text, capped at
    80 KB. Mentions become `<at id="n">name</at>` plus the `mentions` array.
  - A message with `eventDetail` set is a system message, whatever its `messageType` says.
  - Fuzz target and round-trip proptest.
- **Store:** a data file of its own, through the existing store under a pseudo-account `teams`:
  `<files>/Teams.quatrelle`, beside the accounts' files, set aside and rebuilt from the server if
  damaged, like any cache. The store is a key → bytes file with one write transaction at a time;
  its keys:
  - `teams/chats/<chat-hash>`: the chat, its members, its watermark, backfill cursor and the
    first creation time of each of its pages;
  - `teams/msgs/<chat-hash>/<page>`: messages in pages of up to 100, in creation order, newest
    page last; a full page splits in two. An edited or reacted-to old message finds its page by
    its creation time, so no per-message map is kept;
  - `teams/outbox/<sequence>`: the outbox;
  - `teams/img/<hash>`: hosted images and photos, an LRU cache capped at 1 GiB, counted in the
    file's size (freed pages are reused by the file);
  - `teams/index/<segment>`: the search index of message text and sender names.
  Opening a chat reads its last two pages; scrolling up reads one more. Message bodies are capped
  at 256 KiB stored.
- **Disk guard:** the Teams worker reuses the free function `enough_room`
  (`crates/net/src/worker/ahead.rs`, behind the mail worker's `room_to_download`) on
  `store.data_dir()`. Short of room, history backfill, hosted
  images, photos and file downloads pause (told once, looked at again after ten minutes);
  incremental reads of new messages go on.
- **Sign-out** deletes the refresh token and asks whether to delete the local copy too. If the
  outbox holds unsent messages, the question says how many will be lost. Deleting stops the
  worker, closes the file, then deletes it and any set-aside `.damaged` copy.

## Where the code goes

`cargo xtask sizes` allows at most 16 Rust files per folder and 1000 lines per file (split before
800). `crates/model/src` and `crates/engine/src/session` already hold 16 files each, so nothing new
goes directly in them. Counts below are `.rs` files in the folder now → after.

| Code | Place | Folder count |
|---|---|---|
| Teams model types | `crates/model/src/pim/teams.rs` (+ `pim/teams/` for `ChatText` and the ops), re-exported by `pim.rs` | `model/src/pim` 2 → 3; `model/src` stays 16 |
| Job and event variants | `crates/model/src/job.rs` (536 lines) and `job/events.rs` (316 lines); payloads in `pim/teams` | unchanged |
| `is_teams_call_link` | `crates/model/src/ical/notes.rs` (238 lines) | unchanged (6) |
| HTML subset parser and writer | `crates/mime/src/teams_html.rs` (+ `teams_html/` for the writer and tests) | `mime/src` 15 → 16, then full |
| Teams worker, Graph calls, limiter, sync, backfill, outbox, files, search | `crates/net/src/teams.rs` + `net/src/teams/{auth,graph,limits,sync,backfill,outbox,files,search}.rs` | `net/src` 14 → 15 |
| `teams_provider`, `id_token_claim` | `crates/net/src/oauth.rs` (480 lines) | unchanged |
| Fake Graph's Teams routes | `crates/net/src/fakes/fake_graph/teams.rs` | `fake_graph` 2 → 3 |
| Session state and event application | `crates/engine/src/session/pim/teams.rs` (+ `session/pim/teams/`) | `session/pim` 9 → 10; `session` stays 16 |
| `Module::Conversations` | `crates/engine/src/session/pim.rs` (572 lines) | unchanged |
| `teams.*` commands | `crates/engine/src/cmd/teams.rs` | `engine/src/cmd` 10 → 11 |
| The module's panes | `crates/ui-egui/src/panes/conversations.rs` + `panes/conversations/{list,chat,writing}.rs` | `ui-egui/src/panes` 9 → 10 |
| New conversation dialog | `crates/ui-egui/src/dialogs/pim_dialogs/new_conversation.rs` | `pim_dialogs` 8 → 9 |
| Options › Teams card | `crates/ui-egui/src/panes/backstage/options/teams.rs` | `options` 1 → 2 |
| Teams event queue and notification drain | `crates/ui-egui/src/app/teams.rs`; `lib.rs` (916 lines) only calls it | `ui-egui/src/app` 11 → 12 |
| `quatrelle-cli teams check` | `apps/quatrelle-cli/src/teams.rs` (`main.rs` has 471 lines) | `quatrelle-cli/src` 1 → 2 |
| Fuzz target | `fuzz/fuzz_targets/teams_html.rs` | 9 → 10 |
| Icon | `assets/icons/fluent/chat_20_regular.svg`, its `ATTRIBUTION.md` row, `ui-egui/src/look/icons.rs` | |
| Registration guide | `docs/teams.md` | |

Layers hold: `model::pim::teams` is pure and wasm-safe; the parser is in `mime`; only `net`
talks to Graph; the engine only sends `Job`s and applies `Event`s. On the web build the module is
compiled but never shown, so `cargo xtask wasm` keeps passing.

## Synchronisation

**Ownership.** Only the Teams worker (in `crates/net`, with its own Graph connection) opens and
writes the `Teams` data file. The engine never writes it: a send, edit, reaction or mark-read is
a `Job::TeamsOp(teams::Op)` handed to the worker, which persists it in the outbox and answers with
an event. The engine keeps in memory only bounded windows:
- the chat-list rows (`teams::ChatRow`, one small row per listed chat);
- the open chat's last messages, at most 200 (two pages); scrolling up sends `Job::TeamsPage`
  for the previous page, and the window keeps at most 500 messages, dropping the far end.
Nothing in the engine grows with the number of messages in the copy.

**Bounded application.** Events carry at most 50 messages or chat rows each; their other lists are
capped as the table below says. The UI takes the Teams events out of what `Backend::poll` returns,
hands the engine at most 4 of them per frame, keeps the rest queued
(`crates/ui-egui/src/app/teams.rs`) and requests a repaint; per-frame work is thus at most 4 × 50
upserts into the in-memory windows, which ignore anything off-screen. Mail events keep their
current path.
This is the lesson of the search bug of 2026-10-30 (carnet item `recherche-sans-fin`): no step on
the UI thread grows with the size of the copy.

**Jobs** (engine → worker):

| Job | What it asks |
|---|---|
| `Job::TeamsSignIn` / `Job::TeamsSignOut { delete_copy }` | sign in through the browser; sign out, optionally deleting the copy |
| `Job::TeamsOpen { chat }` | the chat now on screen (or none): its last two pages, and the 3 s cadence |
| `Job::TeamsPage { chat, before }` | the page before a creation time, when scrolling up |
| `Job::TeamsVisible { users }` | the people whose presence is on screen |
| `Job::TeamsOp(teams::Op)` | an outbox operation |
| `Job::TeamsSearch { query }` | a search of the local copy |
| `Job::TeamsDirectory { query }` | a directory search (new conversation) |
| `Job::TeamsPicture(teams::Picture)` | a hosted image or a photo |
| `Job::TeamsFile { chat, message, attachment }` | download a file and save it |

**Events** (worker → engine):

| Event | What it carries |
|---|---|
| `Event::TeamsStatus(teams::Status)` | sign-in, copy progress, offline, errors |
| `Event::TeamsChats(Vec<teams::ChatRow>)` | changed rows, at most 50 |
| `Event::TeamsMessages { chat, messages, fresh }` | at most 50 messages; `fresh` when read by an incremental read after the first copy (only those may be announced) |
| `Event::TeamsOpDone { op, outcome }` | an outbox operation done or failed, with Graph's reason |
| `Event::TeamsPresence(Vec<(String, teams::Presence)>)` | presences by user id, at most 650 |
| `Event::TeamsFound { query, hits }` | search hits, at most 200 |
| `Event::TeamsPeople { query, people }` | directory results, at most 25 |
| `Event::TeamsPicture { picture, bytes }` | a picture, decoded by the UI under the limits of `ui-egui/src/reading.rs` |
| `Event::TeamsFileSaved { path }` | a downloaded file, for the UI to open with the system's app |

They have no mail account, so they are variants of their own, not an account's `Event::Notice`.

**Why chat by chat.** The delegated API has no delta for chats: `getAllMessages` and its `delta`
are application-only. Reading each chat's messages, filtered on `lastModifiedDateTime`, is the
only delegated way. (Change notifications are the alternative; see Risks.)

**Server time only.** Every age below ("active in the last 7 days", "30 days") is measured on
Graph's times (`lastMessagePreview.createdDateTime`) against server time, taken from the newest
HTTP `Date` header Graph sent, with a one-hour tolerance. The local clock is never compared with
a server time.

**Limits respected** (Microsoft's Teams service limits,
https://learn.microsoft.com/en-us/graph/throttling-limits#microsoft-teams-service-limits). A
limiter in the worker keeps:
- `GET /me/chats` to at most 1 request per second (the per-user limit);
- each chat's message reads to at most 1 per second, and never two reads of the same chat in
  flight;
- message sends to 1 per second per chat; hosted-content reads to 1 per second per chat;
- all Teams requests together under 16 per second (about 83 % of the 20 per app per tenant, the
  sustained figure Microsoft evaluates).
Regular cadences come before history backfill.

**Cadences (approved):**
- **The chat list, every 10 s:**
  - `GET /me/chats?$expand=lastMessagePreview&$orderby=lastMessagePreview/createdDateTime desc
    &$top=50`, following `@odata.nextLink` only until the activity is older than what is known.
    Read state and hidden state come from each chat's `viewpoint`, which only this call returns;
  - a chat new to Quatrelle, or whose `lastUpdatedDateTime` changed (it moves on a rename or a
    member change), has its members read: `GET /chats/{id}/members`, following `@odata.nextLink`;
  - every chat whose last message is newer than its watermark has its messages read
    incrementally.
- **The open chat, every 3 s** while it is on screen: its incremental message read.
- **Chats active in the last 7 days, every minute:** an incremental read, to catch edits, deletes
  and reactions on older messages. Microsoft documents that adding or removing a reaction moves
  `lastModifiedDateTime`; step 0 checks it. If it didn't, this pass would instead re-read each such
  chat's messages modified in the last 7 days.
- **Hourly:**
  - the whole chat list is read (all pages), with read state and `isHidden`;
  - a chat known locally but absent from the list is marked left: read-only, "You're no longer
    in this conversation", its pending outbox operations failed;
  - chats with activity in the last 30 days have their members and an incremental read, like the
    mail's full sync. Older chats are refreshed only when the list shows new activity, when
    opened, or once a week.
- These cadences continue while the window is minimised to the tray, so notifications come.
- They stop when "Use Teams in Quatrelle" is switched off or on sign-out.
- A 429 or 503 waits for `Retry-After` (reusing `backoff_secs` and `BUSY_ROUNDS` of
  `crates/net/src/exchange/graph/requests.rs`), then runs that cadence at twice its interval,
  doubling again on each new 429, up to 16 times; after an hour without one it returns to its
  approved value.

**Incremental read of a chat:**
- `GET /chats/{id}/messages?$top=50&$orderby=lastModifiedDateTime desc&$filter=
  lastModifiedDateTime gt {watermark − 2 s}`, following `@odata.nextLink`, with the header
  `Prefer: include-unknown-enum-members` (without it, system messages come as
  `unknownFutureValue`). The filter works only with `$orderby` on the same property, as here.
- Messages are upserted by (chat id, message id); a deleted one keeps its id with
  `deletedDateTime`, and the watermark moves to the newest `lastModifiedDateTime` seen.

**First copy and history:**
1. All chats are listed first, newest activity first.
2. Then each chat's history is read backwards: `GET /chats/{id}/messages?$top=50&$orderby=
   lastModifiedDateTime desc&$filter=lastModifiedDateTime lt {S}`, with the same `Prefer` header,
   following `@odata.nextLink`. `S` is the server time (`Date` header) when the chat's copy began;
   the chat's watermark starts at `S`, so the incremental reads take over from there. Every read
   of messages thus carries a date range.
3. Recent chats come first, meeting chats last. One history read is in flight at a time.
4. The `nextLink` is kept per chat, so a restart resumes where it stopped. If Graph refuses a kept
   `nextLink`, the read starts again with `lt` the oldest `lastModifiedDateTime` already copied.

The module shows progress ("N of M conversations") and recent chats are usable at once.

**Read state:**
- A chat is unread when a message from someone else is newer than the chat's
  `viewpoint.lastMessageReadDateTime`.
- The worker keeps each chat's unread count incrementally (on each message arriving and each
  viewpoint change), never by scanning the copy; the rows carry it and the badge sums the rows.
- Opening a chat marks it read through the outbox (below).

**Presence:** for one-to-one chats in the visible list and the open chat's members, every 60 s,
`POST /communications/getPresencesByUserId` with `{"ids":["<user id>", …]}` (at most 650 ids a
call). It is kept in memory only.

**Photos:** `GET /users/{id}/photos/48x48/$value`, kept in the image cache. A 404 means initials
and is remembered for a day, so it isn't asked again at every pass; colleagues whose mailbox is
on-premises may often have none.

**Hosted images:** `GET /chats/{chat}/messages/{msg}/hostedContents/{id}/$value`, with
`ConsistencyLevel: eventual` for a message that was edited or deleted.

**Outbox** (persistent in the Teams file, written by the worker only, in order per chat), with the
Graph call for each operation. `{me}` is the user's object id, `{tid}` the tenant id.

| Operation | Graph call |
|---|---|
| send | `POST /chats/{id}/messages` with `{"body":{"contentType":"html","content":"…"},"mentions":[…]}` |
| send with inline images | the same, plus `"hostedContents":[{"@microsoft.graph.temporaryId":"1","contentBytes":"<base64>","contentType":"image/png"}]`, the body pointing at `<img src="../hostedContents/1/$value">`; each image at most 4 MB after re-encoding |
| send with files | see below |
| edit | `PATCH /chats/{id}/messages/{msg}` with `{"body":{"contentType":"html","content":"…"},"mentions":[…]}`; Graph refuses it if a member isn't a Teams user, and the edit then shows "Not edited" with the reason |
| delete | `POST /users/{me}/chats/{id}/messages/{msg}/softDelete` (the documented form; step 0 also tries `/me/chats/…`) |
| react | `POST /chats/{id}/messages/{msg}/setReaction` and `…/unsetReaction`, with `{"reactionType":"👍"}` (Unicode) |
| mark read | `POST /chats/{id}/markChatReadForUser` with `{"user":{"id":"{me}","tenantId":"{tid}"}}` |
| new chat | `POST /chats` with `{"chatType":"oneOnOne"}` or `{"chatType":"group","topic":"…"}` and `"members"`: for the user and each person, `{"@odata.type":"#microsoft.graph.aadUserConversationMember","roles":["owner"],"user@odata.bind":"https://graph.microsoft.com/v1.0/users('{user id}')"}`; then the first message |

Sending files:
1. Upload to OneDrive: `PUT /me/drive/root:/Microsoft Teams Chat Files/{name}:/content?@microsoft.graph.conflictBehavior=rename`
   for a file up to 4 MB (Quatrelle's choice: Graph takes up to 250 MB this way). Above, an upload
   session: `POST /me/drive/root:/Microsoft Teams Chat Files/{name}:/createUploadSession` with
   `{"item":{"@microsoft.graph.conflictBehavior":"rename"}}`, then the file in ranges to the
   session's URL. `{name}` is made safe and percent-encoded.
2. `GET /me/drive/items/{item}?$select=id,eTag,webDavUrl,webUrl,name,size`.
3. Share with the chat's other members: `POST /me/drive/items/{item}/invite` with
   `{"recipients":[{"email":"…"} or {"objectId":"<user id>"} when a member has no address],
   "roles":["read"],"requireSignIn":true,"sendInvitation":false}`. A 207 answer (some recipients
   failed) marks the message "Not sent", naming the cause.
4. The message: `"attachments":[{"id":"<GUID inside the item's eTag>","contentType":"reference",
   "contentUrl":"<webDavUrl>","name":"<name>"}]`, and in the body
   `<attachment id="<same GUID>"></attachment>`.
- A 401 on the simple `PUT` renews the token and retries once. An upload session's URL carries
  its own authorisation and expires: after a restart it is resumed by asking the session URL
  which ranges it has; an expired one is cancelled (`DELETE` on its URL) and the file uploaded
  again from the start.

Opening a file others shared: `GET /shares/u!{base64url(contentUrl)}/driveItem/content` (to
confirm at step 0), saved to the attachments folder; "Open in browser" uses the attachment's
link.

- Offline, operations wait. A refused one is marked failed with Graph's reason, and the message
  shows "Not sent".
- A send whose outcome is unknown (connection lost after the request left) is not blindly
  retried. At queue time the operation records the newest `createdDateTime` known in that chat (a
  server time). After an unknown outcome the chat's latest messages are read, and a message from
  the user (`from.user.id` equal to the object id), with the same text, created after that
  recorded time, counts as sent. Local time is never compared with server time. This guards
  against duplicates.

**Directory search** (new conversation): `GET /users?$search="displayName:{q}" OR "mail:{q}"
&$count=true&$top=25&$select=id,displayName,mail,userPrincipalName` with the header
`ConsistencyLevel: eventual` (Graph requires both the header and `$count` with `$search`). `{q}`
loses its quotes and backslashes and is percent-encoded. Typing is debounced (300 ms).

**Search** of the conversations runs on the local copy, in the worker:
- An inverted index of message text and sender names is kept in the Teams file and updated as
  pages are written, in small bounded steps. It never rebuilds from scratch on routine sync. It is
  new infrastructure: the mail's index (`model::search::Index`) isn't persistent.
- The worker loads the index at start in slices, so the window never waits for it. Until it is
  loaded, a search answers "Indexing…".
- The UI sends `Job::TeamsSearch { query }` and gets `Event::TeamsFound` (chat and message ids, at
  most 200). The UI thread does no search work.

## Errors

- **Sign-in expired or revoked** (refused refresh): "Teams: sign in again" in the module and the
  Options card, with a button. The local copy stays readable. The browser never opens on its own.
- **Offline:** "Offline" in the module. The outbox waits, and everything resumes on return.
- **429/503:** wait as told, then the slower cadence (see Synchronisation).
- **A permission missing** (403 with a consent error, or a scope absent from `Tokens.granted`): a
  message that names it and says how to consent: sign in again to accept it, or, if the tenant
  forbids user consent, "Grant admin consent" in Entra. Never a raw error.
- **Removed from a chat** (403/404 on it, or absent from the hourly list): kept read-only, with
  "You're no longer in this conversation".
- **The data file damaged:** set aside, rebuilt from the server, never fatal.
- **Everything from Graph is hostile:** JSON size caps per response (8 MiB), message and field
  length caps, image decode limits (those of `ui-egui/src/reading.rs`), `http(s)` links only, file
  names made safe before saving. No panics: `Result` everywhere, as the rest of Quatrelle.

## Privacy and security

- The refresh token stays in the keychain. `Secret`'s `Debug` is redacted.
- The log never carries a message's text, a name, an address, a chat topic or an id in clear.
  It carries counts, durations, HTTP statuses and shortened hashes.
- Quatrelle connects only to `login.microsoftonline.com`, `graph.microsoft.com` and the
  OneDrive/SharePoint hosts Graph hands file and upload-session links for. Certificate checks are
  always on (rustls, Mozilla roots).
- Remote images wait for a click. Hosted contents come through Graph, authenticated.
- Clean room: Microsoft's Teams client is never copied. The screens, wording and drawings are
  Quatrelle's own, beyond feature names and the MIT-licensed Fluent icons.

## Testing

- **The fake Graph server** (`crates/net/src/fakes/fake_graph/teams.rs`, beside `route.rs`,
  which already knows `me`, `users`, photos and 429) learns the Teams endpoints:
  - chats with `$expand`, `$orderby`, `viewpoint` and paging; chat members with paging;
  - messages, including the `lastModifiedDateTime` filters (`gt`, `lt`), paging, and system
    messages as `unknownFutureValue` without the `Prefer` header;
  - hosted contents;
  - set and unset reaction (reactions read back by name and by Unicode), PATCH, softDelete,
    markChatReadForUser (refused without its body);
  - POST /chats (returning the existing one-to-one chat);
  - users search (refused without `$count=true`);
  - presence by ids;
  - OneDrive upload (simple and session, session expiry), item read, invite (200 and 207),
    download through `/shares`;
  - photos (with 404);
  - 429 with `Retry-After`, 401 then renewal, 403 consent.
- **net:** each cadence and the incremental, backfill and resume logic against the fake; the
  limiter (1 per second per chat, never two reads of one chat in flight, the global ceiling); the
  429 slow-down and its return; left chats; hidden chats synced; the outbox (including the
  unknown-outcome guard and the 207 invite); token renewal under concurrency; the disk guard.
- **model / mime:** the HTML subset parser and writer, its caps (256 KiB in, 80 KB out), a fuzz
  target in `fuzz/`, a round-trip proptest; the reaction mapping; `is_teams_call_link` (call links
  accepted, meeting and other links refused) and `is_meeting_link` unchanged.
- **engine:** each `teams.*` command, including `every_command_survives_junk_params`; event
  application in bounded batches (4 events of 50 per frame); the bounded windows; unread counts;
  notifications' rules; `Module::Conversations` falling back to Mail when Teams is off.
- **ui-egui:** headless frames of the module (empty, signing in, first copy, list, conversation
  with every kind of content, failed message, new-conversation dialog, Options card) in light and
  dark, and the 13 catalogues.
- **Scale:** a synthetic copy of 2,000 chats and 300,000 messages. Opening the module, scrolling
  a long chat and receiving a message each keep per-frame work flat (timed test, as for search).
- **Step 0, by the owner on the real account**, before building anything else:
  - a command `quatrelle-cli teams check`: sign in through the browser (the address printed if
    no browser opens), then print only numbers and yes/no answers:
    - chats by kind; messages in the most recent chat;
    - whether `setReaction`, `markChatReadForUser` and `softDelete` (both path forms) answer, on a
      chat the owner names;
    - whether a reaction moves `lastModifiedDateTime` (documented: yes);
    - which forms of `reactionType` come back;
    - whether a file attachment of the named chat downloads through `/shares`;
    - with `--hour`, the 429 rate over an hour at the approved cadences;
  - with `--call <chat>`, it opens the built call link in the browser, for the owner to see that
    Teams offers to call the right people (nothing printed);
  - the token is kept in memory only, and nothing is written to disk;
  - Claude never handles the credentials.

## Delivery

Four test builds, each usable on its own:
1. Options › Teams, sign-in (with `GET /me`), the first copy, the list and reading (images
   included, files as cards), presence dots, local search, the limiter and the 429 slow-down.
2. Replying, notifications, mark read, the outbox and "Not sent".
3. Reactions, @mentions, edit and delete.
4. Sending images and files, opening files, new conversation, "Call" (with
   `is_teams_call_link` and the A3 launcher taking call links).

## Out of scope for this part

- Channels, setting one's own presence, browsing team files: parts 2–4.
- Threaded replies with quote, forwarding, pinning and muting chats.
- Typing indicators, per-member read receipts.
- GIF and sticker search, scheduling, translations.
- A second Teams identity or tenant.
- The web build (no keychain; desktop only, like Applications).
- Change notifications (see Risks › Design note).

## Risks

**Polling, accepted by the owner.** Microsoft's Teams API overview says, under "Polling
requirements" (https://learn.microsoft.com/en-us/graph/api/resources/teams-api-overview?view=graph-rest-1.0#polling-requirements):
"If your app polls to see whether a resource has changed, you can only do that once per day. […]
When polling for new messages, you must specify a date range where supported. […] Apps that don't
follow these polling requirements will be considered in violation of the Microsoft APIs Terms of
Use." The approved cadences (every 10 s, 3 s, a minute, an hour) are polling in that sense. The
compliant alternative, change notifications, needs a public HTTPS endpoint, which a desktop app
doesn't have, or Azure (Event Grid or Event Hubs). On 2026-10-30 the owner was told this and
offered delivery through Azure Event Grid to a storage queue; the owner declined (there is no
Azure subscription) and accepted the risk of throttling, or of a Terms-of-Use objection, for the
application registered in the owner's own tenant. The mitigations:
- every message read carries the `lastModifiedDateTime` date range;
- never two reads of the same chat in flight;
- at most 1 request per second per user on `/me/chats` and 1 per second per chat on messages;
- a 429 waits `Retry-After` and then doubles that cadence until clean (an hour without a 429);
- step 0 measures the 429 rate over an hour at these cadences;
- "Use Teams in Quatrelle" off stops all polling.

**Design note: change notifications.** If Azure ever becomes available, Event Grid delivery to a
storage queue is the documented compliant path: a subscription with `notificationUrl`
`EventGrid:?azuresubscriptionid=<id>&resourcegroup=<group>&partnertopic=<topic>&location=<region>`
on the resource `/users/{id}/chats/getAllMessages`, with delegated `Chat.ReadWrite`; Quatrelle
would read the queue and then the chats it names. Out of scope now.

**To check at step 0:**
- The owner's mailbox is on-premises. Microsoft says such users can use Teams chat, whose
  messages live in a cloud "shadow" mailbox, but promises nothing about Graph's chat API for
  them; step 0 proves it before anything else is built.
- `setReaction`/`unsetReaction`, `markChatReadForUser` and `softDelete` are documented on Graph
  v1.0 for delegated use; step 0 checks that they answer for the owner. If one doesn't, that
  feature waits and the spec is amended.
- The call deep-link format (`l/call/0/0?users=…`).
- Files shared by upload plus invite must open for the other members as they do from Teams.
  Checked in delivery 4 with the owner and a colleague.
