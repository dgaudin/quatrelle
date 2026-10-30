# Microsoft Teams chats: registering the app and the step 0 check

Quatrelle reads and writes Teams chats through Microsoft Graph, signed in as the person using it.
Before any of that is built, the organisation's administrator registers an app for it in
Microsoft Entra (once), then runs `quatrelle-cli teams check` on their own account: it says, in
numbers only, whether Graph serves the account's chats, which reaction forms come back, whether
the writes Quatrelle needs work, and how often Graph throttles the polling Quatrelle plans. Those
numbers decide what is built next.

The check signs in in the browser, keeps the tokens in memory only (nothing is written to disk or
to the keychain), and prints counts, yes or no, HTTP status numbers and durations: never a name,
an address, a chat's topic, a message's text or an id. Its output can be pasted back as it is.

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
   host and path, and the portal's form refuses an `http` address other than `localhost`, so the
   other two go in through the manifest.
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

   **Add permissions**. Quatrelle asks for exactly these at sign-in, nothing more.
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
   options or the repository's build secrets.

## 2. Running the check

`quatrelle-cli` comes with Quatrelle's Linux packages (`.deb`, `.rpm`) and the macOS CLI zip; from
the sources it is `cargo run --release -p quatrelle-cli -- teams check …`.

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
quatrelle-cli teams check --client … --tenant … --write "<chat link>" --hour --call "<chat link>"
```

## 3. What each line means

A typical run (with `--write` and `--call`):

```text
chats: 2 (one-to-one 1, group 1, meeting 0, other 0)
messages in the newest chat: 4
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
in all: requests 15, throttled 0
call: opened
```

| Line | Meaning |
|---|---|
| `chats: N (one-to-one …, group …, meeting …, other …)` | Graph lists your chats, by kind. `at least N`: there were more than 10 pages (500 chats); only those were counted. `chats: no (403)`: Graph refused (a permission is missing: step 3 or 4). |
| `messages in the newest chat: N` | The messages Graph gives for the chat with the latest message (`at least`: more than 10 pages). `0 (no chat)`: the account has none. |
| `reactions: N on M messages (by name …, as emoji …, other …)` | How Graph writes reactions in that chat: as names (`like`, `heart`…), as emoji, or otherwise. |
| `file attachments: N, the first downloads: yes, B bytes` | Files shared in that chat, and whether the first one can be downloaded (its bytes are counted, not kept). `none to try`, or `no (403)` and the like. |
| `write, …` | Only with `--write`: each write Quatrelle needs, `yes`, `no (<HTTP status>)`, or `not tried` when an earlier step failed. See below. |
| `write, stray test messages swept: N` | Only when a send's answer was lost: the test messages found and deleted anyway. |
| `write, test messages left undeleted: N` | Should be `0`. Otherwise delete the test messages by hand in that chat. |
| `hour, after N min: requests …, throttled …, failed …` | Only with `--hour`, every 10 minutes: how it is going. |
| `hour, S s: requests R, throttled T, failed F, 429 rate X %` | The hour's totals: `throttled` is the number of 429 (Too Many Requests) answers. It may end with `, stopped (401)` (the sign-in ran out and couldn't be renewed), `, cut short at the end`, or `, sign-in renewed N` (the access token was renewed in the hour, as expected after 60 to 90 minutes). |
| `in all: requests R, throttled T` | Every request of the run, and the 429 answers among them. |
| `call: opened` | Only with `--call`: the call link was opened. `call: no (…)` says why. |

A status in brackets is Graph's HTTP status: `401` the sign-in was refused, `403` a permission is
missing or the action isn't allowed, `404` not found, `429` throttled, `5xx` Graph's own trouble.

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
every 10 seconds, the newest chat every 3 seconds, and the chats active in the last 7 days (20 at
most) every minute. It reads only, and prints a line every 10 minutes and a total at the end: the
`429 rate` is what this is for. Keep the computer awake (no sleep) during that hour; the access
token is renewed in memory when it runs out.

## 6. `--call <chat>`: a call

```sh
quatrelle-cli teams check --client … --tenant … --call "<chat link>"
```

It reads the chat's members and opens Teams' call link (`https://teams.microsoft.com/l/call/0/0?users=…`)
with everyone in it but you: Teams (the app or the web) then offers to start the call. Nothing is
called until you confirm there. It prints `call: opened`, or why not; it never prints the members.

## 7. Sending the result back

Everything printed on the standard output is numbers, yes or no, HTTP statuses and durations: copy
it as it is and paste it back. The sign-in address printed on the error output (`Open this address
to sign in: …`) carries the app's client id and no secret, and needn't be sent.
