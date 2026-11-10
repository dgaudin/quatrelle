# Gmail in Quatrelle

Google no longer accepts an account's own password from a mail program. Quatrelle reaches Gmail in
one of two ways:

- **Sign in with Google**, through the browser (OAuth 2.0), in a copy of Quatrelle built with a
  Google client ([oauth.md](oauth.md)). The published builds carry none, so they don't offer it.
- **An app password**, in any copy: a password Google makes for one program, used instead of your
  own. This page is about that.

## Make an app password

1. Turn on **2-Step Verification** for the Google account (*Google Account › Security*). Google
   offers app passwords only once it is on.
2. Open <https://myaccount.google.com/apppasswords>, give the password a name (`Quatrelle`), and
   create it. Google shows sixteen letters, once: copy them.

Google's own pages change from time to time; its help page "Sign in with app passwords" has the
steps as they are today.

## Add the account

*File › Add account…*: your Gmail address, and the sixteen letters as the password (not your own
password; the spaces Google shows between the groups of letters don't matter). The servers are
found from the address. Typed by hand, they are:

| | Server | Port | Security | User name |
|---|---|---|---|---|
| Incoming (IMAP) | `imap.gmail.com` | 993 | SSL/TLS | the whole address |
| Outgoing (SMTP) | `smtp.gmail.com` | 465 | SSL/TLS (or 587, STARTTLS) | the whole address |

The password is kept in the system keychain, never in Quatrelle's files or logs. It can be taken
back at any time on the same Google page: Quatrelle then asks for a new one.

## When Google refuses

- **A work or school account (Google Workspace):** its administrator may have turned app passwords
  off. Ask them, or for IMAP access through OAuth with the organisation's own client.
- **Advanced Protection** on the account: Google makes no app password for it.
- **"Application-specific password required"** or **"Invalid credentials"** at sign-in: the
  account's own password was typed, or the app password was taken back. Make a new one.

## What to know about Gmail

- Gmail's labels are folders here; a message with several labels is in each of them.
- A Gmail mailbox with a small quota can be emptied onto this computer: *Archive on this computer*
  in *File › Info* (mail is copied, read back the same, then deleted on the server through Gmail's
  Trash).
- The scam check (*File › Options › Assistant*) can be ticked for a Gmail account; in a personal
  mailbox, everyone but you counts as outside.
