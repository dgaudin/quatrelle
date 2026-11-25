# Microsoft Teams chats: registering the app and the step 0 check

Quatrelle reads and writes Teams chats through Microsoft Graph, signed in as the person using it.
Before any of that is built, the organisation's administrator registers an app for it in
Microsoft Entra (once), then runs `quatrelle-cli teams check` on their own account: it says, in
numbers only, whether Graph serves the account's chats, which reaction forms come back, whether
the writes Quatrelle needs work, and how often Graph throttles the polling Quatrelle plans. Those
numbers decide what is built next.

The check signs in in the browser, keeps the tokens in memory only (nothing is written to disk or
to the keychain), and prints counts, kinds of chat as Graph names them, yes or no, HTTP statuses
with Graph's error codes, and durations: never a name, an address, a chat's topic, a message's
text or an id. Its output can be pasted back as it is.

## 1. Registering the app (once)

Done by an administrator of the Microsoft 365 tenant, in the
[Microsoft Entra admin center](https://entra.microsoft.com).

1. **Entra ID › App registrations › New registration**.
   - *Name*: `Quatrelle` (anything; people see it on the consent page).
   - *Supported account types*: **Single tenant only - \<your tenant\>** (also written
     *Accounts in this organizational directory only*).
   - *Redirect URI*: platform **Public client/native (mobile & desktop)**, value
     `http://localhost`.
   - **Register**.
2. The loopback addresses, and public client flows. Quatrelle listens for the sign-in's answer on
   `http://127.0.0.1:<port>/`: Microsoft ignores the port of a loopback redirect but matches its
   host and path. The other two addresses can be added in the same form; if the form refuses
   them, add them in the manifest instead:
   - **Manifest** (in the app's menu, the Microsoft Graph app manifest): the `publicClient`
     entry becomes:

     ```json
     "publicClient": {
         "redirectUris": [
             "http://localhost",
             "http://127.0.0.1",
             "http://127.0.0.1/"
         ]
     },
     ```

     **Save**.
   - **Authentication** › *Advanced settings* › **Allow public client flows**: the toggle
     **Enable the following mobile and desktop flows: Yes**. **Save**. (In the manifest it is
     `"isFallbackPublicClient": true`.)
   - No client secret and no certificate: **Certificates & secrets** stays empty.

   These are the same settings as [docs/oauth.md](oauth.md), step 2, for mail.
3. **API permissions › Add a permission › Microsoft Graph › Delegated permissions**, and tick:

   | Permission | Why |
   |---|---|
   | `User.Read` | who signed in |
   | `User.ReadBasic.All` | the names and pictures of the people in a chat |
   | `Chat.ReadWrite` | reading the chats, posting, editing, reacting, deleting one's own messages, marking a chat read |
   | `Presence.Read.All` | people's availability |
   | `Files.ReadWrite` | sending a file in a chat (it goes to the sender's OneDrive) |
   | `Files.Read.All` | opening the files others shared in a chat |
   | `offline_access` | staying signed in (a refresh token) |
   | `openid`, `email` | who signed in (an ID token), under *OpenId permissions* |

   **Add permissions**. Quatrelle asks for exactly these at sign-in, nothing more.

   **Teams meetings (optional).** To let Quatrelle make Teams meetings from the calendar (section
   8, Teams meetings), also add the delegated permission `OnlineMeetings.ReadWrite`, and give
   admin consent for it (step 4) when the organisation requires it. Quatrelle never asks for it
   with the rest: it is asked for only when someone clicks **Authorize Teams meetings**, never
   when signing in for the conversations. Without it everything above works as before.
4. Optional: **Grant admin consent for \<your organisation\>** on the same page, then **Yes**.
   Microsoft documents `Chat.ReadWrite` as not needing admin consent; the others normally don't
   either, so without this step each person approves them once on Microsoft's consent page. If
   the check says consent is needed, step 4 grants it for everyone. It is **required** when the
   tenant doesn't let users consent to apps (**Entra ID › Enterprise apps › Consent and
   permissions**). If the check prints
   `Microsoft asks for consent to these permissions: see docs/teams.md, step 4`, this is the step
   to do.
5. **Overview**: copy the **Application (client) ID** and the **Directory (tenant) ID**. They are
   not secrets. They are what `--client` and `--tenant` take below; later, they go into Quatrelle's
   options, or into a build made on your own computer (a published build never carries them).

## 2. Running the check

`quatrelle-cli` comes with Quatrelle's packages: the Linux packages (`.deb`, `.rpm`, the `.tar`
archive, Flatpak), the macOS CLI zip, FreeBSD's, and on Windows both the installer and the portable zip.
The Windows installer puts `quatrelle-cli.exe` beside `quatrelle.exe` in
`C:\Program Files\Quatrelle\` but not on the `PATH`: run it from that folder
(`& "C:\Program Files\Quatrelle\quatrelle-cli.exe" teams check …` in PowerShell). From the
sources it is `cargo run --release -p quatrelle-cli -- teams check …`.

```sh
quatrelle-cli teams check --client <Application (client) ID> --tenant <Directory (tenant) ID>
```

`--tenant` also takes the tenant's domain (`contoso.onmicrosoft.com`). Instead of the two flags,
the environment variables `QUATRELLE_TEAMS_CLIENT_ID` and `QUATRELLE_TEAMS_TENANT` can be set (or
built in, set while building).

It prints `Open this address to sign in: https://login.microsoftonline.com/…` and opens the
browser on it (if the browser doesn't open, copy the address into one). Sign in with your own
account and approve the permissions if asked. The sign-in must be done within 5 minutes. Then the
check runs, in a few seconds without `--write` or `--hour`.

The whole check, with everything below:

```sh
quatrelle-cli teams check --client … --tenant … --write "<chat link>" --files "<chat link>" --hour --call "<chat link>"
```

## 3. What each line means

A typical run (with `--write`, `--files` and `--call`):

```text
chats: 3 (one-to-one 1, group 1, meeting 0, other 1)
chats of other kinds: unknownFutureValue 1
chats of other kinds, by id: 19:...unq.gbl.spaces 1
chats of other kinds, last message from: app bot 1
chats made in another organisation: 1
messages, the 3 newest chats: read 2, refused 1
messages refused, by kind: one-to-one 0, group 0, meeting 0, other 1
messages refused, made in another organisation: 1
messages refused, why: 403 Forbidden 1
messages in the newest chat read: 4
reactions: 2 on 1 messages (by name 1, as emoji 1, other 0)
file attachments: 1, the first downloads: yes, 9 bytes
write, send (1/2): yes
write, react: yes
write, the reaction moves lastModifiedDateTime: yes
write, unset the reaction: yes
write, edit: yes
write, soft delete by /users: yes
write, send (2/2): yes
write, soft delete by /me: yes
write, mark the chat read: yes
write, test messages left undeleted: 0
upload session: yes (host: …sharepoint.com)
upload: yes
item: yes (attachment id: yes, on your OneDrive: yes)
shared with the others: 2 of 2
message with the file: yes
listed as a file: yes
download: yes (host: …sharepoint.com)
cleaned up: yes
in all: requests 27, throttled 0
call: opened
```

| Line | Meaning |
|---|---|
| `chats: N (one-to-one …, group …, meeting …, other …)` | Graph lists your chats, by kind. `at least N`: there were more than 10 pages (500 chats); only those were counted. `chats: no (403 …)`: Graph refused (a permission is missing: step 3 or 4; or the account has no Teams: sign in with the one you use Teams with, not an administrator's account). |
| `chats of other kinds: …` | Only when some are `other`: their kinds as Graph names them (`none` when it gives none). |
| `chats of other kinds, by id: …` | The shapes of their ids, never the ids: the number before the colon and the domain Teams ends them with (`19:...thread.v2`, `19:...unq.gbl.spaces`; `another` for any other), or one of Teams' own threads (`48:notes`). |
| `chats of other kinds, last message from: …` | Who wrote their last message, by Graph's kind of identity: `app bot` (an app), `person aadUser` (someone of an organisation), `person personalMicrosoftAccountUser` and the like, `nobody` (an event, such as a call that ended), `no message`. |
| `chats made in another organisation: N` | Chats another organisation's member started (a meeting of theirs, for one). `not known (no tenant)`: the sign-in didn't say which organisation is yours. |
| `messages, the N newest chats: read R, refused F` | The 20 chats with the latest messages (fewer if you have fewer): a page of messages asked of each. |
| `messages refused, …` | Only when some were refused: those chats by kind, how many were made in another organisation, and why (Graph's status and error code, with how many times). |
| `messages in the newest chat read: N` | Every message of the newest chat Graph let be read (`at least`: more than 10 pages). `0 (no chat read)`: none could be. |
| `reactions: N on M messages (by name …, as emoji …, other …)` | How Graph writes reactions in the chats read: as names (`like`, `heart`…), as emoji, or otherwise. |
| `file attachments: N, the first downloads: yes, B bytes` | Files shared in the chats read, and whether the first one can be downloaded (its bytes are counted, not kept). `none to try`, or `no (403 …)` and the like. |
| `write, …` | Only with `--write`: each write Quatrelle needs, `yes`, `no (<HTTP status> <code>)`, or `not tried` when an earlier step failed. See below. |
| `write, stray test messages swept: N` | Only when a send's answer was lost: the test messages found and deleted anyway. |
| `write, test messages left undeleted: N` | Should be `0`. Otherwise delete the test messages by hand in that chat. |
| `upload session: …`, `upload: …`, `item: …`, `shared with the others: …`, `message with the file: …`, `listed as a file: …`, `download: …`, `cleaned up: …` | Only with `--files`: each step of sending a small file and opening it back, `yes`, `no (HTTP <status> <code>)` or `not tried` when an earlier step failed. See section 7. |
| `hour, after N min: requests …, throttled …, failed …` | Only with `--hour`, every 10 minutes: how it is going. |
| `hour, S s: requests R, throttled T, failed F, 429 rate X %` | The hour's totals: `throttled` is the number of 429 (Too Many Requests) answers. It may end with `, stopped (401 …)` (the sign-in ran out and couldn't be renewed), `, cut short at the end`, or `, sign-in renewed N` (the access token was renewed in the hour, as expected after 60 to 90 minutes). |
| `in all: requests R, throttled T` | Every request of the run, and the 429 answers among them. |
| `call: opened` | Only with `--call`: the call link was opened. `call: no (…)` says why. |

A status in brackets is Graph's HTTP status, followed by its error code when it gives one
(`403 Forbidden`): `401` the sign-in was refused, `403` a permission is missing or the action
isn't allowed, `404` not found, `429` throttled, `5xx` Graph's own trouble.

## 4. `--write <chat>`: the writes

```sh
quatrelle-cli teams check --client … --tenant … --write "https://teams.microsoft.com/l/message/19%3A…/…?context=…"
```

It tries, in the chat given, each write Quatrelle needs, about one a second:

1. posts a first test message, `Quatrelle check, please ignore (1/2)`;
2. reacts to it with 👍, checks that the reaction changes the message's modification time, then
   takes the reaction off;
3. edits it to `Quatrelle check, please ignore (1/2, edited)`;
4. deletes it (Teams' soft delete, the way the Teams app does);
5. posts a second, `Quatrelle check, please ignore (2/2)`, and deletes it the other way Graph
   offers;
6. marks the chat read.

It posts two short messages and deletes each at once; whatever fails in between, every test
message it posted is deleted before it ends (the last `write` line says how many couldn't be).
**The others in the chat may still see them come and go, and get a notification**: pick a
one-to-one chat with a colleague who knows. Nothing else in the chat is touched. (The chat with
yourself won't do: its id isn't a `19:…` one, and the check refuses it.)

To get the chat's link: in Teams, point to any message in the chat, **… (More options) › Copy
link**. That link (`https://teams.microsoft.com/l/message/19%3A…/…`) carries the chat's id, which
is what the check takes from it. Paste it in quotes. The chat's id itself (`19:…@thread.v2` or
`19:…@unq.gbl.spaces`) is taken too: in Teams on the web, it is in the address bar after
`/conversations/`. A link or id that isn't one is refused before anything is sent.

## 5. `--hour`: the throttling rate

```sh
quatrelle-cli teams check --client … --tenant … --hour
```

After the rest of the check, it polls for **an hour** at the pace Quatrelle plans: the chat list
every 10 seconds, the newest chat it could read every 3 seconds, and the chats active in the last
7 days (20 at most) every minute, leaving out the chats Graph refused to read. It reads only, and
prints a line every 10 minutes and a total at the end: the `429 rate` is what this is for. Keep
the computer awake (no sleep) during that hour; the access token is renewed in memory when it
runs out.

## 6. `--call <chat>`: a call

```sh
quatrelle-cli teams check --client … --tenant … --call "<chat link>"
```

It reads the chat's members and opens Teams' call link (`https://teams.microsoft.com/l/call/0/0?users=…`)
with everyone in it but you: Teams (the app or the web) then offers to start the call. Nothing is
called until you confirm there. It prints `call: opened`, or why not; it never prints the members.

## 7. `--files <chat>`: a file sent and opened

```sh
quatrelle-cli teams check --client … --tenant … --files "<chat link>"
```

It checks, on the chat given, that sending a file and opening one work on your account. It sends
**one small text file** (30 bytes, `Quatrelle check, please ignore`) to your OneDrive, shares it with
the chat's other members, posts it in the chat with the message `Quatrelle check, please ignore
(file)`, reads the message back and downloads the file the way another member's Teams does, then
**removes the message and the file**. It runs after the writes of `--write` when both are given,
and takes a chat the same way (section 4). **The others in the chat may see the message come and
go, and get a notification**: pick a one-to-one chat with a colleague who knows.

Each step prints one line, `yes`, `no (HTTP <status> <code>)`, or `not tried` when an earlier step
failed. Nothing is ever printed that names the chat, a member, the file or an address.

| Line | Meaning |
|---|---|
| `upload session: yes (host: …sharepoint.com)` | OneDrive opened an upload for the file. The host is where the bytes go, said by its last two labels only (the address itself carries its own authorisation and is never printed). `no (HTTP 507 quotaLimitReached)`: the drive is full. |
| `upload: yes` | The 30 bytes went up in one piece, on a connection of their own to that host and without your token (the upload's address is its own authorisation). |
| `item: yes (attachment id: yes, on your OneDrive: yes)` | OneDrive gave the file's description a message needs: the id Teams uses for the attachment, and whether the file's address is on your own OneDrive's host (`no`, or `not known` when Graph didn't say which one is yours). |
| `shared with the others: N of M` | The file was shared, to read, with the chat's members but you, M in all and N granted, with no mail sent. `no (HTTP 403 Forbidden)`: your organisation doesn't let you share files this way. `0 of 0`: nobody else is in the chat. |
| `message with the file: yes` | The message was posted with the file attached. |
| `listed as a file: yes` | Read back, the message lists the file as a file (not as a card or an image). |
| `download: yes (host: …sharepoint.com)` | The file's address, asked of Graph, led to a download of the same 30 bytes, without your token. `no (…)` says what stopped it, or how many bytes came. |
| `cleaned up: yes` | The message and the file are deleted. `no (the message is left)`, `no (the file is left)`, `no (a message may be left)` or `no (a file may be left)` say what to delete by hand: the message in the chat, the file `Quatrelle check.txt` (perhaps numbered, as `Quatrelle check 1.txt`) in the OneDrive folder `Microsoft Teams Chat Files`. |

## 8. Using Teams in Quatrelle

Once your organization has registered the application (section 1), Quatrelle reads your Teams
chats from a copy kept on your computer, as Outlook's Cached Exchange Mode keeps mail, and you
can reply in them, react to messages, mention people, and edit or delete your own messages.

### Options › Teams

Open File › Options › Teams.

- **Use Teams in Quatrelle** is the switch. Off, nothing is asked of Teams and the Conversations
  module is not shown.
- **Application (client) ID** and **Directory (tenant) ID** are the two ids from the registration
  (the tenant may also be written as the organization's domain). Each is saved when you leave its
  box.
- **Sign in** opens Microsoft's sign-in page in the browser chosen in File › Options ›
  Applications for Teams meetings, in a window of its own and the profile chosen there (the
  default browser when that is the choice, or when none is found). It needs the switch on and
  both ids filled in. While Quatrelle waits, "A Microsoft sign-in window has opened" comes with
  **Reopen the page** (the window may be behind another one, or closed) and **Copy the address**
  (to paste it in a browser yourself), here and above the conversations list. Only Microsoft's
  own sign-in page is ever opened this way.
- Once signed in, the page shows who you are, and where the copy stands: copying ("Copying your
  conversations… 12 of 340 conversations"), up to date, offline, or what to do (sign in again; a
  permission your administrator must grant, with the permission's name). When signing in again
  fails or lacks a permission, **Sign in** stays there to try again, here and above the list.
- **Sign out** asks whether the copy of your conversations on this computer goes too: **Keep the
  copy**, **Delete the copy**, or Cancel. Kept, it is used again when the same account signs back
  in. Messages you sent that Teams doesn't have yet are deleted either way, and the question says
  how many.
- The sign-in is remembered in the system keychain. Without one, Quatrelle says "Your sign-in won't
  be remembered: no keychain is available", and signing in is asked again at each start.
- A copy made for another account is never read: it is set aside, and the page offers **Delete that
  copy**.
- **Announce new messages** and **Only messages sent to me alone or naming me** are the two
  switches of the announcements (see Announcements below).
- **Show conversations that can't be read** lists the chats Graph won't serve (off at first; see
  The chats Graph won't serve below).

Signing in needs a browser on the same computer: the web app does not offer Teams.

### The Conversations module

With Teams on and signed in, **Conversations** appears in the module rail. If you sign out or
switch Teams off while it is open, Quatrelle returns to Mail.

- **The list** shows your chats, the most recently active first; a chat with unread messages is
  in bold (only the rail's **Conversations** shows a count). Chats with someone outside your
  organization carry an **External** tag, as does a chat whose members Teams hasn't given yet;
  group chats show how many members they have. At most 20,000 conversations are kept; the search box finds a chat by its
  name and, once the copy is indexed, by the words of its messages. A chat found by its words
  opens at the message found, tinted for a moment; when newer messages are left out, **Go to the
  newest messages** under the last one opens the chat at its newest. Conversations you are no
  longer in say so, in the list and under the conversation's name.
- **The chats Graph won't serve.** Microsoft does not let an application read some chats: those
  it lists without a name and as of a kind it doesn't say (most likely chats around meetings;
  the chats of meetings themselves are read as any other; most are old chats you are no longer
  in). They are left out of the list unless **Show conversations that can't be read** is on in
  File › Options › Teams; shown, they are marked **Can't be read here**, and opening one says so
  and offers **Open in Teams**.
- **Reading a conversation.** Messages are grouped by day (Today, Yesterday, then the date), edited
  ones say "(edited)", deleted ones say so, and an app's messages carry an **App** tag. A sender
  outside your organization is tagged **External** on each message: before the name on the first
  of their messages in a row, beside the time on the next ones. Anyone who isn't among the chat's
  members is too, whatever their message says (a guest who left, or anyone while Teams hasn't
  given the members yet). **Members** lists who is in the chat. Reactions are chips under their
  message (see Reacting, mentioning, editing and deleting). A message shows 500 paragraphs at
  most, then "…" (open it in Teams to read the rest).
- **Links** in a message open in your browser on a click. Pointing at one shows where it goes:
  with **Open links without their tracking** on (File › Options), the address inside a redirect
  it's wrapped in, as in mail: the page that opens (through your organization's link check when
  the link goes through one). A link that couldn't be handed to a browser as it is, or out of its
  redirect (not a web address, too long, or a host that isn't what it reads as, such as
  `bank.example@other.example`), is plain text.
- **Open in Teams**, beside every conversation's name, opens that conversation in Teams itself, in
  the browser window Teams meetings open in (else the default browser); the chats Graph won't
  serve and the content Quatrelle can't show (cards) offer it too. Quatrelle makes the
  conversation's link itself, in Teams' own form for a conversation, from the chat's id and your
  organization's; for a chat whose id doesn't have the shape of a Teams chat, it uses the link
  Microsoft gives, and the button shows only when there is one. Should Teams not know the chat,
  Teams opens at its home. Only links to Teams chats open this way, never a link from a message.
- **Pictures** kept with a message are fetched when the message is first drawn, a few per message,
  and shown in place; a click shows one big. A picture from elsewhere on the web is never fetched
  by itself: its place says **Show picture**, and only that click loads it (an `https` address, a
  size limit, a minute at most, however slowly its server answers). One that can't be had (refused, too big, out of reach) says "Picture
  not available"; Quatrelle remembers it (a thousand such pictures at most) and doesn't ask for it
  again until it starts again.
- **Files** shared in a chat appear as cards with the file's name and the host its link goes to
  (out of a redirect, as links are). A link to your own OneDrive or your organization's SharePoint
  is shown as such; any other host is written on the card, "Link to another site", as is a link
  that goes through a link check (a click opens the check's address, whatever it names). Nothing opens until you click it; **Open** and
  **Open in browser** are on the card (see Pictures and files).

### Replying

The box at the foot of a conversation sends to it (there is none in a conversation you are no
longer in, or one Graph won't serve).

- **Enter** sends; **Shift+Enter** starts a new line. **Ctrl+B** and **Ctrl+I** make the
  selection bold or italic, and the emoji button opens the mail editor's emoji picker. In
  Conversations, wherever the keyboard is, the keys that send the mail being written (Ctrl+Enter,
  Alt+S) do nothing: a mail goes from Mail, or from its own window.
- What is sent is the text as typed, its lines, bold and italic, and the people mentioned (see
  below): what looks like markup goes as it was written, and a web address typed isn't made a
  link. Pictures and files go with a message (see Pictures and files below).
- A message longer than Teams takes (80 KB once written for Teams) stays in the box, which says
  "This message is too long for Teams". With 50 messages waiting in the conversation, or 500 in
  all, it stays there too: "Too many messages are waiting".
- Each conversation keeps what its box holds while Quatrelle runs (20 conversations at most);
  it's gone when Quatrelle closes.

**Messages going out.** A message sent shows at once on your side, dimmed, with "Sending…",
and takes its place among the others once Teams has it. Quatrelle sends them one at a time, in
the order written in each conversation, a second apart at least; a message waiting in one
conversation doesn't hold back the others', though everything pauses while Teams asks to wait or
after it fails. Once the outbox has taken them, they are kept in the copy until Teams has them:
without the network, in Work offline, or while Teams asks to wait, they wait (and still show
"Sending…" after a restart: a message of words alone; one with files or pictures, see Pictures and
files), then go. A message waiting its turn offers **Delete**; one being sent
offers nothing, since it may be in Teams already.

When the network drops during a send, Quatrelle can't tell whether Teams got the message: before
sending it again, it looks for it in the conversation. That avoids nearly every duplicate, but
can't rule one out: Teams may list a message only after Quatrelle looked. One posted three times
that Teams never confirms is marked Not sent. Quatrelle looks through 200 changes of the
conversation at most: when more came since (a busy conversation while the computer slept), or
when looking fails five times running (over about eight minutes), the message is marked Not sent
rather than sent again, and you decide.

**Not sent.** A message that couldn't go stays in its conversation marked **Not sent**, with
why: "Teams refused this message" (with Teams' code), "Too long for Teams", "Too many messages
are waiting", "You're no longer in this conversation", or "Quatrelle couldn't confirm it was
sent". **Retry** sends it again; **Delete** removes it. A message not sent is never sent again by
itself. One the outbox didn't take (too many waiting, a conversation left) is kept only while
Quatrelle runs. When its conversation isn't on the screen, a notice says "A message to Support
team wasn't sent". Switching Teams off keeps the messages waiting: they go when it's on again.

### Pictures and files

**Sending.** The paperclip beside the box attaches files (the system's file dialog), and a
picture on the clipboard (a screenshot, a picture copied from another program) is added with Ctrl+V.
Each shows above the box as a small chip with its name and size and a cross that takes it away.
They go with the text, the pictures first and then the files, each in the order added.

- **At most 10 files and pictures go with a message, and each file at most 100 MB.** One that is
  empty, can't be read, is already there, or is over the limit isn't added, and the box says why
  under it. A message may be only files, with no text.
- A picture pasted that is 4 MB (4 MiB) or less goes inside the message and shows in the conversation as
  any picture does. A bigger one goes as a file. Quatrelle first saves a picture pasted as a file
  of its own in a folder only you can read; that file is removed when you take the picture away with
  its cross, when the conversation's box lets its words go (a draft let go to make room for
  others), and when **Cancel** closes the New conversation pane. Switching to another
  conversation keeps what was typed, pictures included. Once the message is sent the file stays
  for Quatrelle to read it, perhaps after a restart, and Quatrelle removes it at its start when
  it is a week old.
- **Where a file goes.** It is put in your OneDrive, in the folder `Microsoft Teams Chat Files`,
  and shared **to read only** with the people who are in the conversation when the message is
  sent, nobody else (Quatrelle asks Teams who they are just before it shares, and checks that
  what OneDrive made is that file, in that folder); the message carries a link to it. A file already uploaded stays in that folder when
  its message is cancelled or isn't sent: delete it there if you don't want it.
- **Sending shows.** The message appears at once on your side, dimmed, with **Uploading…** and the
  share done as the files go up (a big file may take a while), then "Sending…" as for any message. Everything waits when
  Teams asks to wait or the network is away. **Work offline** stops an upload under way: it starts
  again from the beginning once you're back online. A file that still can't go up after three
  tries (each one taken up several times where it stopped) marks its message Not sent, "Quatrelle
  couldn't confirm it was sent".
- **Not sent.** Besides the reasons of Replying, a message with files can be marked Not sent for:
  "A file is no longer there, or has changed" (it is read again when its turn comes), "A file is
  bigger than Teams takes here (100 MB)", or "OneDrive didn't share the file with everyone in this
  conversation" (nothing goes half shared: if the file can't be shared with every other member,
  or a member has no address, the message isn't sent). A message
  that Quatrelle's outbox refused when it was sent, and that had files, has no **Retry**: it says
  "Attach the files again to send it" and only **Delete** is offered. A message the outbox took
  keeps its **Retry** and **Delete** while Quatrelle runs (a file that is gone or changed when it
  goes marks it Not sent, as above). **Delete** removes it.
- **After a restart** (or Teams switched off and on again), a message whose files weren't all
  shared yet, or that has pictures and wasn't sent yet, isn't sent from what Quatrelle had kept of
  it: nothing of it is read, uploaded or shared. It shows Not sent, "A file is no longer there,
  or has changed", says "Attach the files again to send it", and only **Delete** is offered. It
  stays, its words with it, from one start to the next until you delete it. A message of words
  alone goes on as before.

**Files others shared.** A file card shows its name, size and where its link goes.

- **Open** downloads the file, with your own sign-in and not by the link on the card, into a
  folder of Quatrelle's that only you can read, and opens it with your computer's program for
  that type of file; the card says "Downloading…" meanwhile. Files of 100 MB or less are
  downloaded: a bigger one says "It is bigger than Quatrelle downloads (100 MB): open it in the
  browser". When it fails the card says why: not enough room on the disk, Teams refused it (with
  Teams' code), Teams can't be reached, the download was too slow, too many files are being
  downloaded, or the file is no longer in the conversation. A file removed from that folder since
  it was downloaded is downloaded again at the next **Open**.
- **A file that could run** (a program, a script, a web page, a picture that holds scripts, a
  document with macros, or a Word, Excel or PowerPoint file of the formats before 2007 and a rich
  text file, which can hold them too), or of a type Quatrelle doesn't know (without an extension,
  or with one it doesn't know: your computer would go by what the file holds), isn't opened: it is
  shown in its folder, and the card says "This file could run a program: it is shown in its
  folder, not opened." You decide what to do with it. On Windows every file downloaded is marked
  as coming from the internet, as browsers do: Office opens it in Protected View, wherever you
  open it from.
- **Open in browser** opens the file's link in your default browser; pointing at it shows where it
  goes. It is offered for any web link of the card a browser may be handed (see Links above),
  including the files over 100 MB.

### Starting a conversation

**New conversation**, beside the search box of the list, or Ctrl+N in Conversations, opens a pane
where the conversation is written, in the place of the one open.

- **To:** holds the people you pick, each a chip (tagged **External** for someone outside your
  organization) with a cross, or Backspace in the empty field for the last one. Type a name or an
  address: after a short pause Quatrelle looks it up in your organization's directory, and lists
  the people found under the field (**Searching…**, **No one found**, or "The directory couldn't
  be searched"). ↑ and ↓ choose and Enter or a click picks. Only people the directory gave can be
  picked; an address typed that the directory doesn't know isn't added.
- **At most 20 people** besides you. With two or more, **Group name (optional)** is offered.
- At the foot is the box of a conversation, with its bold, italic, emoji, files and pictures
  (without the @ mentions, which need the conversation's members). **Enter creates the
  conversation and sends the first message**; nothing is made before.
- **It needs the network.** While Teams makes it ("Creating the conversation…") everything
  waits, **Cancel** too: the request has gone, and its answer settles it. When Teams can't be
  reached ("Teams can't be reached: try again") or refuses ("Teams didn't create the
  conversation", with its code), the pane stays as it was, with the people,
  the name, the words and the files, to try again. If the conversation is made and the first
  message is refused (too long, too many waiting), the pane also stays with what was typed, a
  notice says the message wasn't sent, and the conversation is in the list: sending again opens
  it rather than making another.
- **A one-to-one conversation already there** with that person is opened, and the message goes to
  it: Teams keeps one conversation for two people.
- Esc first closes a popup over the pane, then empties **To:** when people are listed under it,
  then sets the pane aside, as opening a conversation from the list does: **New conversation**
  (or Ctrl+N) brings it back as it was. Only **Cancel** closes it: the people, the name and the
  words typed are let go, and so are the files of the pictures pasted (files attached are only
  dropped from the pane, the originals untouched).

### Calling

**Call**, in the header of a conversation, beside Open in Teams, offers **Audio call** and
**Video call**. It is there in a one-to-one conversation and in a group chat, when every other
person has an address Teams gave and there are at most 20 of them; not in the chat of a meeting,
a conversation you have left, one whose members aren't known yet, or one where someone has no
address.

Choosing one opens Teams' call page for the other people of the conversation in the browser window
Teams meetings open in (else the default browser): Teams, the app or the web, then asks you to
confirm, and **nothing is called until you do there**. Quatrelle itself neither places nor hears
a call. The link is made from the members' addresses and checked again before it opens: only a
Teams call page on `teams.microsoft.com` ever opens, and the log names no one.

### Reacting, mentioning, editing and deleting

- **The bar.** With the pointer on a message, a bar shows over its top edge: six quick reactions
  (👍 ❤️ 😆 😮 😢 😡), **Copy**, and on your own messages **Edit** and **Delete**. Copy puts the
  message's words on the clipboard as plain text. In a conversation you're no longer in, the bar
  offers Copy alone. System lines (someone added, the conversation renamed), deleted messages,
  messages that can't be shown and a message being deleted have no bar.
- **Reactions.** Under a message, each kind of reaction is a chip with how many reacted so; yours
  are in the accent colour. A click on a chip adds your reaction of that kind, or takes it back;
  hovering it lists who reacted, by the conversation's members' names. A reaction Teams names in
  words rather than as an emoji (other than the six quick ones) is shown but can't be clicked.
  Your reaction counts at once and goes to Teams in its turn; one clicked and taken back before
  it went sends nothing. One Teams doesn't take is said by a notice ("Teams didn't take your
  reaction in Support team"), and the chip goes back to what Teams has.
- **Mentions.** Typing "@" in the box offers the conversation's members from your organization
  (eight at most; a guest of your organization is tagged **External**), found by the start of their name, of a word of it, or of their address: ↑ and
  ↓ choose, Enter, Tab or a click picks, Esc closes the list. The name goes in highlighted and
  the person is mentioned in Teams; changing its letters afterwards makes it plain text. People
  from other organizations can't be mentioned from Quatrelle (their name can still be typed). A
  message mentions 50 people at most.
- **Editing.** ↑ in an empty box edits your last message in the conversation (not while newer
  messages are left out, nor while one you sent is still going out), and **Edit** on the bar any
  of yours. Its words come into the box under "Editing message", and what you were typing is set
  aside until the edit is sent or cancelled. **Edit** on the message being edited keeps what you
  typed; on another of yours, that one is edited instead, and the changes typed in the first are
  let go. Enter sends it; Esc or Cancel cancels it. An edit that changes nothing sends nothing;
  one that leaves nothing isn't sent ("To remove a message, delete it"). Only a message the box
  can hold whole can be edited here: its text, bold, italic, links written as their own address,
  and mentions. A message with a list, a quote, code, underlining, a picture or a file has no Edit
  (↑ says "Your last message can't be edited here"): edit it in Teams.
- **Deleting.** **Delete** on the bar asks "Delete this message?" first. Deleted, the message is
  gone for everyone in the conversation, and Quatrelle can't bring it back. Esc or Cancel keeps it.
- **Changes going out.** An edit shows its new words at once, dimmed, with "Editing…" under the
  message; a deletion dims the message and says "Deleting…". While one waits its turn, **Cancel**
  lets it go. Without a network a change waits for it, however long, as a message does, and goes
  once the network is back. One Teams doesn't make says "Not edited" or "Not deleted" and why
  ("Teams refused this change", with Teams' code, or "Quatrelle couldn't confirm the change was
  made", after three tries that Teams answered with trouble of its own, or that never got through
  while everything else did), with **Retry** and **Cancel**; when its conversation isn't on the
  screen, a notice says so ("Your edit in Support team wasn't made"). A change of a message the
  conversation doesn't show at the moment (an older one) is listed after the messages going out.
  Edits, deletions and reactions go in the order made, with the messages sent, and count in the
  same limits (50 waiting in a conversation, 500 in all). A message deleted in Teams meanwhile
  takes its changes still waiting with it, a deletion Teams refused among them.

### Marked read

A conversation is marked read in Teams when you look at it: when you open it, when a message
comes into it while it's on the screen, and when you come back to it (to the window, to
Conversations, or from the File page). On the screen means open in Conversations, in Quatrelle's
window in front: not minimized, not hidden in the notification area, not behind another
application, and not under the File page. Its row stays in bold until Teams says it's read, about
a second. A conversation you're no longer in, or one Graph won't serve, isn't marked.

### Announcements

With **Announce new messages** on, each new message from someone else is a desktop notification.
Its title says who sent it, as the conversation's members name them: "Ana Exemple", "Ana Exemple in
Agenda" for a group or a meeting's chat, "Ana Exemple mentioned you in Agenda" when it names you;
"(External)" follows a sender outside your organization (or not shown to be of it: anyone who isn't
among the conversation's members), "(App)" an
application. Its text is the message's first line ("New message" when it has none), as plain
text: what looks like markup in a message is shown as written.

- **Only messages sent to me alone or naming me** keeps the one-to-one chats and the messages
  that mention you.
- The conversation on the screen isn't announced: it's marked read instead (when it can be). It is
  announced when the window isn't in front.
- Hidden chats aren't announced, nor your own messages, edits, reactions, or what the first copy
  brings.
- A conversation's next notification waits five seconds; what comes meanwhile goes as one ("3
  new messages"). At most four notifications go every five seconds; the rest are summed up in one
  ("12 new messages in 5 conversations").
- For a chat Graph won't serve, its new last line, as the list gives it, is announced when those
  chats are shown; left out of the list, they aren't announced.

### The local copy

The copy is one data file, `Teams.quatrelle`, in the same folder as your accounts' data files
(see `docs/architecture.md`). It holds the chats, their messages, the pictures the screen has
asked for and the messages going out, and is readable by you only. It also holds the search's
index of the messages' words, each conversation's in parts of about a thousand messages, so a new
message rewrites only the part it goes in. A file that is damaged is set aside beside it and a new
one begun; a copy of another account is set aside as `Teams-<8 letters>.quatrelle.old`.

To delete it: Sign out › Delete the copy, or Delete that copy for one set aside. Closing
Quatrelle and removing the file by hand does the same; the next sign-in copies again.

The first copy reads the chat list, then each chat's history backwards, newest chats first,
a page at a time. It stops while the disk is nearly full and goes on when there is room again.

### How often Quatrelle asks Teams

Quatrelle keeps well under Microsoft's limits, with one worker that makes one request at a time:

| What | How often |
|---|---|
| A message you send, edit, delete or react to | as soon as it may: at most one a second in each conversation |
| Marking a conversation read | when you look at it |
| The chat open on the screen | every 3 seconds |
| The list of chats | every 10 seconds |
| Chats active in the last 7 days | every minute |
| Presence of the people on the screen | every minute |
| Chats active in the last 30 days (the others weekly) | every hour |
| History of older messages | in the gaps, when nothing above is due |

After a "too many requests" answer (429 or 503), nothing is asked until Microsoft's wait is over,
then Quatrelle goes slower, and returns to its pace after an hour without one. Each request ends
within two minutes (a server that answers slowly is taken for the network gone: Offline, tried
again later). Switching Teams off or signing out stops all of it, once what was under way has
ended: a step may make a few requests in a row (reading a long list of members, signing in), each
ending within two minutes.

### Teams meetings

A **Teams Meeting** button in the appointment window makes a Teams meeting through Microsoft
Graph (`/me/onlineMeetings`), for an appointment on a calendar that is not a Microsoft 365
mailbox (Exchange on premises, CalDAV, a local calendar). The meeting is made at the click, so the
invitation can be read and changed before it is sent. The button is in the desktop app only. On
the calendar of a Microsoft 365 mailbox, the appointment itself becomes the meeting (see **In a
Microsoft 365 mailbox's calendar**, below).

**The button.** It sits beside **Join** and **Webex Meeting**; a plain click does the first
entry of its menu (**Teams Meeting menu**):

- **Add a Teams meeting** needs a title and a start ("A meeting needs a title and a start"
  otherwise, and nothing is sent). While the request is out the button shows
  "Creating the Teams meeting…", and Save and Send wait for the answer. Only one request goes at
  a time from a window.
- **Remove the Teams meeting** takes the block and the place (when it is the meeting's link) out
  of the appointment at once; the meeting itself is deleted when the appointment is saved.
- **Meeting options** opens the meeting's own options page in the browser, once the meeting
  exists and its address passed the checks below. The options are set there, not in Quatrelle.

**What the answer writes.** The place becomes the link to join, unless a place was typed. The
notes get a block of up to four lines, in the language of the interface, each value as Teams gave
it (a line with no value is left out):

```text
Join the Teams meeting: <link>
Meeting ID: <id>   Passcode: <passcode>
By phone: <number>, conference ID <conference>
Meeting options: <link>
```

The lines already in the notes stay. Adding a meeting again replaces the block, found whatever
language the interface had when it was written; only the button writes it. **Join** works on the
meeting: it takes the appointment's conference first, then a meeting link in the place or the
notes, and a Teams link opens in the browser, as other meeting links do. The conference is
labelled "Microsoft Teams".

**Permission, apart.** Quatrelle signs in for the conversations without the meetings permission.
With Teams off or signed out, the button opens File › Options › Teams. Signed in but without the
permission, the window says "Teams meetings need your permission" with **Authorize Teams meetings**,
which opens Microsoft's sign-in page again with `OnlineMeetings.ReadWrite` added (File › Options ›
Teams has the same button on its **Teams meetings** line, which reads "Teams meetings are allowed"
afterwards). While that page is open, the conversations pause. Refused there, they go on at once
from the sign-in already there, without signing in again; a page closed or left unanswered keeps
them paused until its five minutes are up, then they go on the same way. The permission is forgotten
when you sign out, when Microsoft refuses it at a renewal (the conversations then carry on with
their own permissions), when Graph answers 403 saying it is missing, and when a sign-in that asks
for it is refused with no sign-in to go on from (the next **Sign in** asks for the conversations'
permissions alone); the next click asks again.

**The option.** File › Options › Calendar has **Make every meeting with guests a Teams meeting**
(off at first; note: "An appointment you organize gets a Teams meeting with its first guest"). On,
an appointment you organize gets its meeting once it has a title, a start and a guest and nothing
has been typed for a second, or when Send is pressed, which then waits for the answer. It is not
added when the appointment already has a Teams meeting, a Webex meeting or room, was opened with
guests already, belongs to someone else's calendar, or when the meeting was removed in this window;
each window tries once. In a Microsoft 365 mailbox's calendar the appointment is marked, as below,
Teams signed in or not, and Send goes at once. Elsewhere, with Teams off, signed out or without the
permission, nothing is sent and the window says once "Teams meeting not added: sign in to Teams",
with the button to do so; choosing a Microsoft 365 mailbox's calendar after that takes the words
away, and the appointment is marked there.

**In a Microsoft 365 mailbox's calendar.** There Exchange Online makes the meeting, not Quatrelle:
**Add a Teams meeting** asks nothing of Teams, and needs neither Teams signed in nor the meetings
permission. It marks the appointment, the window says "A Teams meeting will be added when the
appointment is saved", and Save and Send don't wait. The appointment is saved with Graph's
`isOnlineMeeting` set and `onlineMeetingProvider` `teamsForBusiness`, sent again with each change of
an appointment you organize, never with a change to your copy of someone else's meeting. Exchange
Online makes the meeting and writes its own text in the notes, and the link to join it comes back
with the appointment: **Join** takes that link first. Exchange Online follows the appointment
itself: changing, moving or deleting it asks nothing of Teams. Before the first save, **Remove the
Teams meeting** takes the mark off. After it, Microsoft 365 keeps the meeting: the entry is grey
("Microsoft 365 keeps an online meeting once it is made"), and Quatrelle never sends
`isOnlineMeeting` as false. Choosing a calendar that isn't a Microsoft 365 mailbox's before the
first save takes the mark off ("The Teams meeting was removed: add it again for this calendar"). The
mark is never written in iCalendar, and never goes with a meeting Quatrelle asked Teams for.

**What follows the appointment.** Only for an appointment you organize (or one with no
organizer, no guests and no answer of yours): an event someone else organized never makes
Quatrelle touch a meeting, whatever id it carries.

- Saving a changed title, start or end sends what changed (a `PATCH`). A time zone changed alone
  sends nothing unless it moves the start or the end: the meeting holds only those two instants, in
  UTC. A new link in the answer replaces the old one in the notes, the place and the conference.
- What the meeting was last sent is kept until Quatrelle closes. A change saved while Teams couldn't
  take it (signed out, switched off, offline) goes with the next save; after a restart that change,
  or one still waiting to be sent when Quatrelle closed, is no longer known, and the meeting keeps
  its old title or times until they change again (its link still works).
- Deleting or cancelling the appointment, or saving it with the meeting removed, deletes the
  meeting. A meeting already gone (404) counts as deleted.
- A series has one meeting, for all its occurrences, with the times of the first one: moving or
  deleting an occurrence sends nothing.
- A window closed without saving deletes the meeting it made (when the answer is still on its way,
  as it arrives). Quitting Quatrelle with such a window open does the same, waiting 3 seconds at
  most.
- A meeting can be left in Teams that no appointment in Quatrelle shows: after a crash; when Teams
  refuses the deletion; when Teams is signed out, switched off or offline as the appointment is
  deleted (a notice says so, and nothing tries again later); when the appointment, or its whole
  calendar, is deleted from another program (Outlook, a phone); and when Teams is switched off while
  the meeting is being made.
- Guests get only the calendar's invitation; Teams sends them nothing.

**Where the id lives.**

| Kept in | As |
|---|---|
| iCalendar (CalDAV, the local cache) | `X-QUATRELLE-TEAMS-ID` |
| Exchange, through EWS | an extended property named `TeamsMeetingId`, in Quatrelle's own property set beside Webex's, read with the appointment and written on create and update; taken off only when the meeting is taken out of an appointment that had one (nothing is sent about it for one that never had one) |

An invitation never carries it, nor does an appointment attached to a message; one in an invitation
received is never read, and on an event you don't organize it is never used or written. A value that
is not a plausible id (1 to 512 bytes of letters, digits, `-`, `_`, `=`, `+`, `/`) is dropped; in a
request's address it is encoded as one segment.

**With Webex.** An appointment has a Teams meeting or a Webex meeting or room, not both: a Teams
meeting (or the mark Exchange Online makes one by) greys the Webex button and rooms, a Webex meeting
or room greys **Add a Teams meeting** ("This appointment has a Webex meeting or room: remove it
first", "This appointment has a Teams meeting: remove it first"), and removing one frees the other.
Nothing falls back to Webex by itself (see [docs/webex.md](webex.md)). An appointment of someone
else's calendar greys the button with "A Teams meeting can only be added to an appointment you
organize". Moving the appointment to a calendar that can't hold a meeting takes the meeting out
("The Teams meeting was removed: add it again for this calendar").

**When it fails.** The window says why and the appointment is left as it was:

| Teams | The window says |
|---|---|
| signed out, or Teams off | "Teams is signed out" (the button opens Options › Teams) |
| 403 `Forbidden` or `InsufficientPrivileges` without the permission | "Teams meetings need your permission" |
| 403 though the token holds the permission, or another 403 (licence, policy) | "Your Teams account can't create meetings" |
| 401 after a renewal | "Sign in to Teams again" |
| a link Quatrelle doesn't trust | "Teams gave a link Quatrelle doesn't trust" (the meeting is deleted again) |
| no network | "The Teams meeting couldn't be created: no connection" |
| any other status | "Teams answered 400: BadRequest", Graph's status and its code, never its message, which may quote content |

A failed update or deletion is a notice, "The Teams meeting couldn't be updated: …" or "… deleted:
…". Requests wait in one queue of 32, served before anything else the worker does, one at a time;
the 33rd answers "no connection" without a request, as does one that waited two minutes and all of
them when Work offline turns on. At most 16 requests for a new meeting wait at once. A 429 is waited
out and sent again, twice at most (three sends in all); a 503 is sent again for an update or a
deletion, never for a creation, so none is made twice. Once Graph says the permission is missing,
the requests waiting, and those that come until it is given again, get the same answer without
asking Graph. If the part of Quatrelle that talks to Teams can't start, every request is answered
"no connection". Nothing is hidden: no failure ever becomes a Webex meeting or a room.

**What is checked in Teams' answer.** It comes from a server, so it is read as hostile:

- A link to join or to the options is kept only if it is `https` on port 443, with no `@` before
  the path, no space, control, bidirectional or `\` character, at most 2,048 bytes, and a
  host that is `teams.microsoft.com` or ends in `.teams.microsoft.com`, or one of the government
  clouds' (`teams.microsoft.us`, `gov.teams.microsoft.us`, `dod.teams.microsoft.us`):
  `teams.microsoft.com.contoso.example` and `evilteams.microsoft.com` are refused. A bad link to
  join refuses the whole answer; a bad options link too, when one is given.
- The meeting ID, passcode, phone number and conference ID are kept only as at most 64 letters,
  digits, spaces and `+ - ( )`; any other stays out of the block. Teams' own invitation text is
  never read.
- The meeting's own id must be a plausible one, or the answer is refused.
- The title is sent as one line of 255 characters at most (control and bidirectional characters
  out); an end at or before the start becomes half an hour after it. The passcode is never
  written to a log or a file; the log names no title, link or id.

**Not tried on a real account yet** (see `docs/gaps.md`): creating, changing and deleting a meeting,
the `TeamsMeetingId` property on a real on-premises Exchange (set, and taken off an appointment
whose meeting is taken out), the hosts of the links Teams gives, and a meeting Exchange Online makes
in a Microsoft 365 mailbox's calendar, with the mark sent again at each change (tried against a fake
Graph only).

### Not yet

Channels, teams, the options of a meeting set from Quatrelle (its lobby, who may present),
co-organizers and delegates, starting a conversation with someone outside the directory (an address
the directory doesn't know), and pictures placed between two paragraphs of a message (they go with
the message, not in its text) are not there yet. For those, use Open in Teams.

## 9. Sending the result back

Everything printed on the standard output is numbers, kinds of chat, yes or no, HTTP statuses with
Graph's error codes, and durations: copy it as it is and paste it back. The sign-in address printed on the error output (`Open this address
to sign in: …`) carries the app's client id and no secret, and needn't be sent.
