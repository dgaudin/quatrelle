# Teams conversations drawn by Quatrelle (1.1, part A2.1)

Status: approved in conversation on 2026-10-30, section by section; this document is the written
spec to review.

## Why

The owner works on Linux, where Microsoft no longer ships a Teams desktop client. Meetings and
calls already open in a Chrome app window (part A3, `2026-10-29-meetings-in-chrome-design.md`).
What is missing is everyday Teams messaging. The owner's organisation (Proginov) is on
Microsoft 365 and the owner is its administrator, so an application can be registered in Entra ID
and given admin consent.

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

## What the owner sees

### File › Options › Teams (desktop only)

A new card, after Applications. It holds:
- "Use Teams in Quatrelle", a switch (off at first).
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
  badge with the count of unread messages (99+ past 99).
- The module is shown only when Teams is switched on.
- Shortcuts while the module is open:
  - Ctrl+N: new conversation;
  - Ctrl+F: search the conversations;
  - Alt+↑/↓: previous or next conversation.

**The list (left pane)**
- One row per chat:
  - picture (the person's photo, else coloured initials);
  - a presence dot (one-to-one chats);
  - the name (the other person; a group's topic, else its members' first names);
  - the last message ("Paul: it's fixed");
  - its time.
- Chats with unread messages are bold. Meeting chats carry a calendar icon. Hidden chats aren't
  listed. Newest activity first.
- At the top:
  - a search box: the chats' names and, on the local copy, their messages' text, answered off
    the UI thread (see Search);
  - "New conversation".
- An empty state ("No conversations yet") and a "Signing in…" / "Copying your conversations…
  N of M" state during the first copy.

**The conversation (right pane)**
- Header:
  - the name;
  - for one-to-one chats, the presence ("Available", "Busy", "Away", "Do not disturb",
    "Offline");
  - the members (a count, the list on click);
  - "Call": it opens the Teams call to the chat's members in the Chrome app window of part A3,
    with the profile chosen there, through a Teams deep link (`https://teams.microsoft.com/l/call/
    0/0?users=…`, checked with `is_teams_link`/`launchable_link` before launching).
- Messages:
  - grouped by day ("Today", "Yesterday", the date);
  - consecutive messages of one sender grouped under one name and picture;
  - one's own on the right on a light accent tint, others' on the left with name and picture;
  - each with its time.
- System messages (members added or removed, a meeting started or ended, the chat renamed) are
  one centred, dim line.
- An edited message shows "(edited)". A deleted one shows "This message was deleted".
- Reactions appear under a message as chips (emoji and count). A click adds or removes one's own
  reaction, and hovering lists who reacted.
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
  seen in Teams" with an "Open in Teams" link (its web link, in the Chrome app window).

**Writing**
- A box at the bottom of the conversation, several lines. Enter sends, Shift+Enter makes a new
  line, Ctrl+B and Ctrl+I make bold and italic.
- The emoji button reuses the picker of 1.1. The paperclip attaches a file. A pasted picture is
  added as an inline image.
- Typing "@" offers the chat's members.
- ↑ in an empty box edits one's last message. Esc cancels an edit.
- A message that couldn't be sent stays in the conversation marked "Not sent", with "Retry" and
  "Delete".

**New conversation**
- A dialog with "To:". It searches the organisation's directory (name or address) and adds people
  as chips, plus an optional topic for a group.
- If a one-to-one chat with that person (or a group with exactly those members) exists, it opens
  that chat instead of creating one.
- The chat is created on the first message sent.

**Notifications**
- A desktop notification per new message from someone else, through the existing
  `Services::notify`, titled with the sender and chat ("Marie Durand", or "Paul in Support
  team"), with the text's first line.
- No notification:
  - for the chat open in a focused window;
  - during the first copy or a history backfill;
  - for hidden chats;
  - when the setting says so.
- A mention is marked in the title ("Marie Durand mentioned you").

## Connection

- **Sign-in:** OAuth 2.0 authorization code with PKCE over a loopback redirect (RFC 8252 +
  RFC 7636), the same flow Quatrelle already uses for Microsoft mail (`crates/net/src/oauth.rs`,
  `authorize`). The authority is `https://login.microsoftonline.com/<tenant>/` with the tenant
  from Options.
- **Scopes** (delegated):
  - `openid profile offline_access User.Read`;
  - `User.ReadBasic.All` (directory search, names, photos);
  - `Chat.ReadWrite` (read chats and messages, mark read, edit and delete one's own, react,
    create chats);
  - `ChatMessage.Send`;
  - `Presence.Read.All`;
  - `Files.ReadWrite` (upload to one's OneDrive and share);
  - `Files.Read.All` (open files others shared).
  Part 1 asks for all of them at once, so later parts don't force a new consent. The exact list
  is confirmed by step 0.
- **Tokens:**
  - The refresh token lives only in the system keychain, through `Credentials::get_secret`/
    `set_secret` under a fixed Teams key (not an account's). Access tokens stay in memory.
  - Renewal happens under one mutex shared by every Teams request, so two renewals never race.
  - An access token refused once (401) is renewed and the request retried once, before any
    "sign in again". This closes the two gaps recorded on the carnet (item graph-jeton) for the
    Teams connection.
- **The owner's registration** (written up step by step in a new `docs/teams.md`, in English, with
  the French UI names in the conversation):
  1. Entra admin center › App registrations › New registration: single tenant.
  2. Platform "Mobile and desktop applications", redirect URI `http://localhost`.
  3. API permissions: the delegated scopes above.
  4. "Grant admin consent for Proginov".
  5. Copy the Application (client) ID and the Directory (tenant) ID into Options › Teams, or into
     the repository secrets.

## Data

- **Model** (`crates/model`, pure and wasm-safe):
  - `teams::Chat`: id, kind (one-to-one / group / meeting), topic, members, the user's
    last-read time, hidden, last activity, web URL;
  - `teams::Member`: user id, display name, address;
  - `teams::ChatMessage`: id, chat id, sender, created and modified times, deleted time, kind
    (message / system event), body as `ChatText`, mentions, reactions, attachments, hosted images;
  - `teams::Reaction`: emoji, user ids;
  - `teams::Attachment`: reference file or hosted image or unsupported card;
  - the outbox's `teams::Op`;
  - the jobs and events below.
- **Message text:** Teams sends HTML. A new parser in `crates/mime` (beside `html.rs`, pure)
  turns it into `ChatText`, Quatrelle's own runs-and-blocks representation. It keeps only the
  allowed subset (bold, italic, underline, strike, links with `http(s)` only, lists, quotes, code,
  `<at>` mentions, `<emoji>` as its Unicode alt text, `<img>` pointing at hosted contents). Every
  other tag is dropped with its text kept, and scripts and styles are dropped with their content.
  - It has caps on input size (256 KiB), depth and output size.
  - The writer does the reverse for sending: `ChatText` to Teams HTML, escaping text. Mentions
    become `<at id="n">` plus the `mentions` array.
  - Fuzz target and round-trip proptest.
- **Store:** a data file of its own, `Teams` in the data folder (beside the accounts' files, set
  aside and rebuilt from the server if damaged, like any cache). Its tables:
  - chats;
  - messages by chat, in time order;
  - sync watermarks and backfill cursors per chat;
  - the outbox;
  - hosted images and photos (an LRU cache capped at 1 GiB);
  - a search index of message text.
  It respects the existing disk guard: downloads pause when the disk runs low. Message bodies are
  capped at 256 KiB stored.
- **Sign-out** deletes the refresh token and asks whether to delete the local copy too.

## Synchronisation

All network work is in `crates/net` (a Teams worker with its own Graph connection), which emits
events. The engine applies them in bounded batches per frame. **No step on the UI thread grows
with the size of the copy.** That is the lesson of the search bug of 2026-10-30 (carnet item
`recherche-sans-fin`).

**Cadences (approved):**
- **The chat list, every 10 s:**
  - `GET /me/chats?$expand=lastMessagePreview&$orderby=lastMessagePreview/createdDateTime desc
    &$top=50`, reading pages only until the activity is older than what is known;
  - a chat new to Quatrelle, or whose `lastUpdatedDateTime` changed, has its members read
    (`$expand=members` on that chat);
  - every chat whose last message is newer than its watermark has its messages read
    incrementally.
- **The open chat, every 3 s** while it is on screen: its incremental message read.
- **Chats active in the last 7 days, every minute:** an incremental read, to catch edits, deletes
  and reactions on older messages.
  - If step 0 finds that a reaction does not move `lastModifiedDateTime`, this pass re-reads each
    such chat's latest 50 messages instead.
- **Hourly:** a full pass over every chat (members, read state, the latest messages), like the
  mail's full sync.
- These cadences continue while the window is minimised to the tray, so notifications come.
- They stop when Teams is switched off or signed out.
- A 429 or 503 waits for `Retry-After` (as the Graph mail code does) and then slows that cadence.

**Incremental read of a chat:**
- `GET /chats/{id}/messages?$top=50&$orderby=lastModifiedDateTime desc&$filter=
  lastModifiedDateTime gt {watermark - 2 s}`, paged.
- Messages are upserted by id, a deleted one keeps its id with `deletedDateTime`, and the
  watermark moves to the newest `lastModifiedDateTime` seen.

**First copy and history:**
1. All chats are listed first, newest activity first.
2. Then each chat's history is read backwards (`GET /chats/{id}/messages?$top=50`, newest first,
   following `@odata.nextLink`), newest chats first.
3. The `nextLink` is kept per chat, so a restart resumes where it stopped.

The module shows progress, and recent chats are usable at once.

**Read state:**
- A chat is unread when a message from someone else is newer than the chat's
  `viewpoint.lastMessageReadDateTime`.
- Opening a chat marks it read (`POST /chats/{id}/markChatReadForUser`) through the outbox.

**Presence:** for one-to-one chats in the visible list and the open chat's members, every 60 s,
`POST /communications/getPresencesByUserId` (at most 650 ids a call). It is kept in memory only.

**Photos:** `GET /users/{id}/photos/48x48/$value`, kept in the image cache. A missing one means
initials.

**Outbox** (persistent, in order per chat), with the Graph call for each operation:

| Operation | Graph call |
|---|---|
| send | `POST /chats/{id}/messages` |
| send with inline images | the same, with `hostedContents` (base64), each image at most 4 MiB after re-encoding |
| send with files | upload to `/me/drive/root:/Microsoft Teams Chat Files/{name}:/content` (resumable upload session above 4 MiB); `POST /me/drive/items/{id}/invite` to the chat's members (read, no e-mail); then the message with a `reference` attachment |
| edit | `PATCH /chats/{id}/messages/{msg}` |
| delete | `POST …/softDelete` |
| react | `POST …/setReaction`, `…/unsetReaction` |
| mark read | as above |
| new chat | `POST /chats` (`oneOnOne` or `group`, members bound by user id), then the first message |

- Offline, operations wait. A refused one is marked failed with Graph's reason, and the message
  shows "Not sent".
- A send whose outcome is unknown (connection lost after the request left) is not blindly
  retried. The chat's latest messages are read first, and a message from the user with the same
  text, created after it was queued, counts as sent. This guards against duplicates.

**Directory search** (new conversation): `GET /users?$search="displayName:{q}" OR "mail:{q}"`
with `ConsistencyLevel: eventual`, at most 25 results. Typing is debounced (300 ms).

**Search** of the conversations runs on the local copy:
- An inverted index of message text and sender names is kept in the Teams file and updated as
  messages arrive, in small bounded steps.
- The query is answered off the UI thread or in constant-cost slices.
- It never rebuilds from scratch on routine sync.

## Errors

- **Sign-in expired or revoked** (refused refresh): "Teams: sign in again" in the module and the
  Options card, with a button. The local copy stays readable. The browser never opens on its own.
- **Offline:** "Offline" in the module. The outbox waits, and everything resumes on return.
- **429/503:** wait as told, then resume.
- **A permission missing** (403 with a consent error, or a scope absent from the token): a message
  that names it and points to admin consent in Entra, instead of a raw error.
- **Removed from a chat** (403/404 on it): kept read-only, with "You're no longer in this
  conversation".
- **The data file damaged:** set aside, rebuilt from the server, never fatal.
- **Everything from Graph is hostile:** JSON size caps per response (8 MiB), message and field
  length caps, image decode limits (those of `ui-egui/src/reading.rs`), `http(s)` links only, file
  names made safe before saving. No panics: `Result` everywhere, as the rest of Quatrelle.

## Privacy and security

- The refresh token stays in the keychain. `Secret`'s `Debug` is redacted.
- The log never carries a message's text, a name, an address, a chat topic or an id in clear.
  It carries counts, durations, HTTP statuses and shortened hashes.
- Quatrelle connects only to `login.microsoftonline.com`, `graph.microsoft.com` and the
  OneDrive/SharePoint hosts Graph hands file links for. Certificate checks are always on
  (rustls, Mozilla roots).
- Remote images wait for a click. Hosted contents come through Graph, authenticated.
- Clean room: Microsoft's Teams client is never copied. The screens, wording and drawings are
  Quatrelle's own, beyond feature names and the MIT-licensed Fluent icons.

## Testing

- **The fake Graph server** (`crates/net/src/fakes/fake_graph*`) learns the Teams endpoints:
  - chats with `$expand`, `$orderby` and paging;
  - messages, including the `lastModifiedDateTime` filter and paging;
  - hosted contents;
  - set and unset reaction, PATCH, softDelete, markChatReadForUser;
  - POST /chats;
  - users search;
  - presence by ids;
  - OneDrive upload (simple and session) and invite;
  - photos;
  - 429 with `Retry-After`, 401 then renewal, 403 consent.
- **net:** each cadence and the incremental, backfill and resume logic against the fake, the
  outbox (including the unknown-outcome guard), the token renewal under concurrency.
- **model / mime:** the HTML subset parser and writer, its caps, a fuzz target in `fuzz/`, a
  round-trip proptest.
- **engine:** each `teams.*` command, including `every_command_survives_junk_params`; event
  application in bounded batches; unread counts; notifications' rules.
- **ui-egui:** headless frames of the module (empty, signing in, first copy, list, conversation
  with every kind of content, failed message, new-conversation dialog, Options card) in light and
  dark, and the 13 catalogues.
- **Scale:** a synthetic copy of 2,000 chats and 300,000 messages. Opening the module, scrolling
  a long chat and receiving a message each keep per-frame work flat (timed test, as for search).
- **Step 0, by the owner on the real account**, before building anything else:
  - a command `quatrelle-cli teams check`: sign in through the browser, then print only numbers
    (chats by kind; messages in the most recent chat; whether `setReaction` and
    `markChatReadForUser` answer; whether a reaction moves `lastModifiedDateTime`, on a chat the
    owner names);
  - the token is kept in memory only, and nothing is written to disk;
  - Claude never handles the credentials.

## Delivery

Four test builds, each usable on its own:
1. Options › Teams, sign-in, the first copy, the list and reading (images included, files as
   cards), presence dots, local search.
2. Replying, notifications, mark read, the outbox and "Not sent".
3. Reactions, @mentions, edit and delete.
4. Sending images and files, opening files, new conversation, "Call".

## Out of scope for this part

- Channels, setting one's own presence, browsing team files: parts 2–4.
- Threaded replies with quote, forwarding, pinning and muting chats.
- Typing indicators, per-member read receipts.
- GIF and sticker search, scheduling, translations.
- A second Teams identity or tenant.
- The web build (no keychain; desktop only, like Applications).

## Risks to check at step 0

- The owner's mailbox is on-premises. Teams chat for such users is stored in the cloud, and
  Graph's chat API should serve it; step 0 proves it before anything else is built.
- `setReaction`/`unsetReaction` and `markChatReadForUser` must be available on Graph v1.0 for
  delegated use. If one is beta-only, that feature waits and the spec is amended.
- Files shared by upload plus invite must open for the other members as they do from Teams.
  Checked in delivery 4 with the owner and a colleague.
- Graph throttling for Teams under these cadences, measured by step 0's timings and watched in
  the log (statuses only).
