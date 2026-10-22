# Browser sign-in (OAuth)

Microsoft (Outlook.com, Hotmail, Live and Microsoft 365) and Google no longer accept passwords
from mail apps over IMAP, POP and SMTP: they want their own sign-in in the browser (RFC 8252 with
PKCE). Providers only give tokens to a registered app, and the registration belongs to whoever
ships Quatrelle, so a copy of Quatrelle offers *Sign in with Microsoft* / *Sign in with Google*
only when it was given a registration:

| Setting | What it is |
|---|---|
| `QUATRELLE_MICROSOFT_CLIENT_ID` | the Application (client) ID of a Microsoft Entra app registration |
| `QUATRELLE_GOOGLE_CLIENT_ID`, `QUATRELLE_GOOGLE_CLIENT_SECRET` | a Google "Desktop app" OAuth client (its secret isn't secret for installed apps) |

They're read when Quatrelle runs, or built in when they're set while building. The test release
workflow (`release-wip.yml`) passes the repository secrets of those names to the build; without
them the build offers passwords only, and an Outlook.com address can't be added (the Add account
dialog says why). The signed release (`release.yml`) runs the shared
`storytold/craft-common-actions` workflow, which accepts only the secrets it declares: to give it
sign-in, that workflow needs a pass-through for these settings, added by whoever ships it with
their own registration.

An account can also bring **its own Microsoft registration** — an organisation's single-tenant
one — which stands in for the built-in one (and works where none is built in): see
[Your organisation's own registration](#your-organisations-own-registration-single-tenant).

## Registering Quatrelle with Microsoft

1. In the Microsoft Entra admin centre (any Microsoft account will do), **App registrations ›
   New registration**: a name, and *Accounts in any organizational directory and personal
   Microsoft accounts*.
2. **Authentication › Add a platform › Mobile and desktop applications**, redirect URI
   `http://localhost`; also add `http://127.0.0.1` and `http://127.0.0.1/` (Quatrelle listens
   on `http://127.0.0.1:<port>/`: Microsoft lets loopback redirects use any port, but the path
   has to match). Turn on **Allow public client flows**. No client secret.
3. **API permissions:** nothing has to be added. Quatrelle asks for what it needs when someone
   signs in (IMAP and SMTP access at `outlook.office.com` with `offline_access` for an IMAP
   account; Graph's `Mail.ReadWrite`, `Mail.ReadWrite.Shared`, `Mail.Send`, `Mail.Send.Shared`,
   `MailboxSettings.ReadWrite`, `Calendars.ReadWrite`, `Calendars.ReadWrite.Shared` and
   `Contacts.ReadWrite` for Exchange Online through Graph — the `.Shared` ones open the shared
   mailboxes and calendars the person was given), and the person approves it on Microsoft's
   page. Listing them here as delegated permissions only shows them in advance.

   A permission added to that list (as the `.Shared` ones were) makes Microsoft ask everyone to
   approve the app **again** at their next sign-in: tokens granted before don't carry it, and
   Quatrelle asks for the whole list each time it refreshes one. Nothing is lost; the browser
   opens once more.
4. Copy the **Application (client) ID** into the repository secret
   `QUATRELLE_MICROSOFT_CLIENT_ID` (Settings › Secrets and variables › Actions), or set it in
   the environment before starting Quatrelle to try it.

Personal accounts (Outlook.com, Hotmail, Live) can approve an unverified app; work and school
accounts often can't until the publisher is verified (a Microsoft AI Cloud Partner Program ID
under **Branding & properties**), or their administrator approves Quatrelle for the organisation.

A personal Microsoft account made with another provider's address (a Gmail address, say) has a
hidden mailbox address (`outlook_…@outlook.com`): mail sent through Graph goes out from it, and
SMTP refuses the sign-in (535 5.7.3) even though it covers sending. An Outlook.com address added
to the account and made its **primary alias** (account.live.com › Manage how you sign in) fixes
both; then the account is added once, with that address.

Sign in once with a test account: if Microsoft says the redirect URI doesn't match
(`AADSTS50011`), check the two loopback addresses in step 2.

## Your organisation's own registration (single tenant)

Work and school accounts often can't approve an app the organisation hasn't admitted. Rather
than wait for the publisher of a Quatrelle build to be verified, an organisation can register
Quatrelle itself (steps 1 and 2 above, with *Accounts in this organizational directory only*,
and its administrator granting consent under API permissions), and give each account that
registration in **Add account › Server settings › Microsoft sign-in: your organisation's own app
registration** (also in the account's settings afterwards):

| Field | What goes there |
|---|---|
| Application (client) ID | the registration's Application (client) ID |
| Tenant | the directory's id or its domain (`contoso.onmicrosoft.com`); `organizations` or `consumers` restrict to work-and-school or personal accounts; empty means `common`, any account |

Both are kept with the account (`oauth_client_id`, `oauth_tenant` in `accounts.json`, no secret
among them), and the sign-in then goes to `https://login.microsoftonline.com/<tenant>/…` with
that client id, in place of the built-in `QUATRELLE_MICROSOFT_CLIENT_ID` and `common`. A copy of
Quatrelle built without any registration adds such an account all the same: the Add account
dialog says where to give one. Changing the registration of an account means signing in again
(the sign-in kept for the old one is no good for the new one; Quatrelle asks when it fails).

From the command line: `account.add` and `account.update` take `oauthClientId` and
`oauthTenant` (`null` or empty puts the built-in one back).

## Microsoft 365 domains are proposed through Graph

When the address typed into Add account belongs to a domain whose mail **Microsoft 365** hosts —
its mail exchanger is Exchange Online's (`<domain>.mail.protection.outlook.com`), or the domain's
`_autodiscover._tcp` record points at Microsoft — the dialog proposes **Exchange (Microsoft 365)**
through Graph rather than IMAP, which organisations often switch off there. The IMAP settings
found, if any, stay under Server settings for whoever picks IMAP and SMTP anyway. Outlook.com,
Hotmail and Live addresses aren't affected: those are known by their domain.

## When an Outlook.com or Hotmail account doesn't connect

- **"Microsoft doesn't accept passwords from mail apps":** the account was added with a password
  (an older Quatrelle, or a password typed in). Click **Sign in with Microsoft** in the status bar
  (or on the account's right-click menu): the browser opens, and the account keeps its mail.
  Quatrelle no longer offers a password for these addresses when it can sign in with Microsoft.
- **"Microsoft signed you in, but its mail server won't open this mailbox over IMAP"** (the
  server's words: *User is authenticated but not connected*): the token is good, but IMAP won't
  open the mailbox. Either IMAP is switched off for it (Outlook.com › Settings › Mail ›
  Forwarding and IMAP › *Let devices and apps use IMAP*, off by default on new accounts; for a
  work account, the organisation's administrator), or the browser signed in as **another
  account** than the one added (a browser already signed in to Microsoft picks that one). Quatrelle
  asks Microsoft who signed in, tries that address once, and otherwise names it in the message.
  Turn IMAP on, or sign in again with the right account, then choose Sync; or **Use Microsoft
  Graph**, which reaches the mailbox without IMAP (and brings its calendar and contacts): the
  account starts again through Graph.

