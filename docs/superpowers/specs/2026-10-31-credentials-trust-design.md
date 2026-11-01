# Credentials go only to the servers an account trusts

Status: the owner decided the rule on 2026-10-31 ("1: oui"). It follows his decision of the same day on
Autodiscover ("Demander, comme Outlook"). This is the written spec. It builds on branch
`jeton-redirection`, which is not merged.

## Why

The shared HTTP client followed redirects itself and sent every header again, the access token
included, to whatever host the `Location` named. CalDAV/CardDAV did the same with the Basic password.
Branch `jeton-redirection` drops `Authorization`, `Proxy-Authorization` and `Cookie` once a redirect
leaves the first URL's origin. Its review (2026-10-31) found that this is not enough, and that it breaks
something:

1. **DAV reaches the other host on the next request.** `Dav::request` returns the final URL. The callers
   resolve the hrefs the other host answers against it (`home_from`, `collections`, `etags`, `multiget`,
   `crates/net/src/protocols/dav.rs`). The next request then sends the password there. The homes it
   finds are saved (`Pim.caldav_home`), so this lasts.
2. **JMAP does the same with its session's URLs.** `apiUrl`, `downloadUrl`, `uploadUrl` and
   `eventSourceUrl` come from the server's JSON and may name any https host (`crates/jmap/src/lib.rs:206-221`).
   Every later request sends the token there.
3. **Discovery to a provider's host now fails.** `.well-known/caldav` and `.well-known/jmap` (RFC 6764,
   RFC 8620 §2.2) commonly redirect from the address's domain to the provider's host. With the branch,
   the request arrives there without credentials and gets a 401. JMAP fails. DAV concludes "no calendar
   here" without a word (`discover` swallows errors).
4. **Exchange Autodiscover sends the password to another domain.** A `RedirectAddr` answer makes Quatrelle
   query that address's domain and answer its 401 with Basic or NTLM, without asking anyone
   (`crates/net/src/exchange/ews/autodiscover.rs:63, 90-116`). Outlook asks first.

## The owner's rule

- **The account's own servers, and any host of the same registrable domain, get the credentials.**
  `fastmail.com` → `api.fastmail.com` keeps them.
- **A host of another domain gets nothing until the user says yes.** The question reads: « Le serveur
  propose de configurer ce compte avec <domaine>. Autoriser ? ». The answer is remembered for this
  account.
- **Credentials are bound to each account's trusted servers**, not to a redirect chain. A URL handed out
  by a server that isn't trusted never gets them.
- **A refused or failed discovery says so**; nothing fails in silence.

"Credentials" means the Basic password, an NTLM or Negotiate answer, an OAuth bearer, and cookies a
server set.

## What the owner sees

- **The question**, the first time an account needs another domain. It is a small dialog in the main
  window:
  - title « Autoriser un autre serveur ? » (en: "Allow another server?");
  - text « Le serveur propose de configurer {account} avec {domain}. Votre mot de passe ou votre connexion
    y seront envoyés. Autoriser ? » (en: "The server proposes to set up {account} with {domain}. Your
    password or sign-in will be sent there. Allow it?");
  - buttons « Autoriser » and « Ne pas autoriser » ("Allow", "Don't allow").

  It opens as the password prompt does: never over another dialog or the File page, once per account and
  domain per run. Several waiting questions come one after the other.
- **Allow** keeps the domain for the account and starts the account again: whatever was waiting
  (Autodiscover, the calendar's discovery, JMAP's session) goes on.
- **Don't allow** leaves the account as it is and says so in the status bar: « {domain} n'a pas été
  autorisé : {account} ne l'utilisera pas » (en: "{domain} wasn't allowed: {account} won't use it").
  Nothing is asked again for that domain until Quatrelle starts again. If the account can't work without
  it (Exchange whose server is on that domain), its status says why, as a failed sign-in does today.
- **Server settings** (File › Account settings › the account) list the other domains allowed, each with
  "Remove", and a field with "Add" (« Autres serveurs autorisés », en: "Other servers allowed"). Add
  serves the web app, which has no question dialog (below), and a domain refused and then wanted after
  all. Adding or removing one restarts the account.
- **Existing accounts keep working, mostly with nothing to answer.**
  - Every server already in their settings is trusted.
  - **URLs already in service and saved** count as trusted too. These are the EWS URL an earlier
    Autodiscover found (`ews.json`), and the saved CalDAV and CardDAV homes (`Pim.caldav_home`,
    `carddav_home`). Credentials have already gone there, for months. This is the one named exception
    to "a URL handed out by a server needs the user's yes".
  - JMAP's session is fetched again at each connection and isn't saved. A JMAP account whose session
    names another domain asks once.
  - Cases known to ask:
    - Fastmail: its `downloadUrl` is on `fastmailusercontent.com`;
    - a new iCloud account outside `@icloud.com`: its homes are on `icloud.com`;
    - Exchange hybrid: a `RedirectAddr` to `onmicrosoft.com`, then an `EwsUrl` on `office365.com`.
      That is two questions, one after the other, each followed by a restart;
    - a new on-premises Exchange account whose `EwsUrl` is on an internal domain (`exch01.corp.local`)
      other than the address's.

    The owner's own Exchange account is covered by `ews.json`.

## The rule, precisely

**A site** is a host's registrable domain: `api.fastmail.com` → `fastmail.com`,
`mail.example.co.uk` → `example.co.uk`.
- It comes from the Public Suffix List, through the `psl` crate (MIT OR Apache-2.0, pure Rust, no I/O,
  builds for wasm). `cargo deny check` must accept it.
- If it doesn't, the existing heuristic `autoconfig::base_domain` is moved to the model and used instead.
  The cost then: shared-hosting suffixes (`github.io`) read as one site.
- Host names are compared in lower case, without a trailing dot, in the ASCII form given. There is no
  U-label/A-label conversion: two forms are two sites, which errs on the safe side.
- When `psl` gives no registrable domain, the site is the host itself, normalised as above. That
  covers a single label (`exchange01`, `localhost`), an IP literal (`127.0.0.1`, `[::1]`; two spellings
  of one IPv6 address are two sites), and a bare suffix. An unknown TLD (`.local`, `.corp`) follows
  PSL's `*` rule: `exch01.contoso.local` → `contoso.local`.
- `psl` generates a large `match`. The plan measures its build time and binary size first. If they are
  unreasonable, or `cargo deny` refuses it, the fallback heuristic is **blocking**: shared-hosting
  suffixes (`github.io`, `onmicrosoft.com`, `azurewebsites.net`) would read as one site. That must then
  be fixed (a small built-in list of such suffixes) before delivery, not treated as a detail.

**An account's trusted sites** are the sites of:
- the account's address domain (`email`);
- every configured server: IMAP, SMTP, POP, JMAP, Graph, EWS (the URL typed, or the one Autodiscover
  found and saved in `ews.json`);
- each domain in a new field `AccountConfig.trusted_domains: Vec<String>`. It is saved in `accounts.json`
  with `#[serde(default, skip_serializing_if = "Vec::is_empty")]`, so old files load.
  - The commands keep it at most 20 entries: a 21st is refused with "at most 20 other servers".
  - A hand-edited file with more is read as it is.
- the URLs in service and saved (the DAV homes), given as `extra`, unless their site was withdrawn.
  An unreadable or empty value is ignored. `ews.json` is used as it is, before any look at the trust,
  unless its site was withdrawn.

The model computes this as `Trust::of(&AccountConfig, extra: &[&str]) -> Trust`, with
`Trust::allows(host) -> bool`, `Trust::sites() -> Vec<String>` and a free function
`site(host: &str) -> String`. `extra` holds the URLs known only to the worker and the engine's PIM doc
(the saved EWS URL and DAV homes).

**Where it applies:**
1. **The generic HTTP redirect chain** (`http/redirect.rs`).
   - A redirect keeps the credentials when the next host's site is the first URL's site, or is in the
     request's trusted sites (a new `Request::trusting(Vec<String>)`, empty by default). Otherwise they
     are dropped for the rest of the chain, as the branch does.
   - https→http stays refused.
   - The response says where it ended: a new `Response.url: Option<String>`, the final URL after
     redirects.
2. **DAV** (`Dav`). `Dav::new` takes the account's `Trust`.
   - **Nothing goes to an untrusted host.** A request whose URL, or one of whose redirect hops, is on a
     site the trust doesn't allow is **not sent at all**, with or without credentials. It returns
     `NetError::Untrusted { site }`. That covers PROPFIND, REPORT, PUT, DELETE, and those built from
     hrefs (principal, home, collection, item).
     - The branch's "send it without `Authorization`" isn't enough: that gets a 401, read as a failed
       sign-in ("calendars and contacts: sign-in failed").
     - An href that goes down from https to http is refused (`Tls`), as JMAP's `resolve` refuses it.
   - **Discovery** (`Dav::discover`) tries every start it has: SRV, `https://{domain}`,
     `caldav.{domain}`, `caldav.<imap host>`, the IMAP host.
     - A start that ends on an untrusted site is noted and skipped.
     - If another start finds a home, it wins with no question. The address's website commonly
       redirects everything to a web host (`wix.com`, `squarespace.com`); that must not ask.
     - Only if no start finds a home and at least one was noted does `discover` return
       `Untrusted { site }` for the first noted.
     - An SRV target on another site counts as such a start (RFC 6764 makes it the legitimate
       equivalent of the well-known redirect).
   - **Sync loops** (`dav::sync`: deletions, PUTs, collections) propagate `Untrusted` as they propagate
     `Auth` (`Err(e @ NetError::Untrusted { .. }) => return Err(e)`). It is never collected into
     `out.errors`, where a tombstone would stay forever and the error come back at each sync.
   - `sync_dav` turns it into `Event::TrustAsked`. After a refusal, the PIM sync of that account does
     nothing until the next start, and says so once.
3. **JMAP** (`crates/jmap`, portable).
   - `Client::connect` takes the trusted sites.
   - `jmap::Request` gains `trusting`, `jmap::Response` gains `url: Option<String>` (the final URL), and
     `jmap::Error` gains `Untrusted { site }`, mapped both ways in `net/src/protocols/jmap.rs`.
     - Natively, `Response.url` comes from `http::Response.url`.
     - In the browser it comes from `web_sys::Response::url()`.
   - The session GET goes with the trusted sites (`Request::trusting`). A 401 whose final URL is on an
     untrusted site gives `Untrusted { site }`.
   - The session's `apiUrl`, `downloadUrl`, `uploadUrl` and `eventSourceUrl` must each be on a trusted
     site. Otherwise `connect` fails with `Untrusted { site }` before sending anything there.
   - **In the browser**, `fetch` follows redirects itself and drops `Authorization` at an origin change
     (the Fetch standard), so `trusting` can't be honoured there. A well-known redirect to the provider's
     host fails in the web app as it does today.
     - The error's text names the domain.
     - The fix is to type the provider's session URL, or the domain in Server settings' "Add". The web
       app's documentation says so.
4. **Exchange Autodiscover** (`autodiscover.rs`).
   - A `RedirectAddr` whose domain's site isn't trusted is not followed: `Untrusted { site }`.
   - An `EwsUrl` on a site that isn't trusted is not used: `Untrusted { site }`.
   - The address's own domain and the configured server stay as today.
   - `ask_autodiscover` (`ews/shared.rs`) only asks the account's own domain and server, so it is
     unchanged.
5. **Graph, Teams, LLM, OAuth** keep their own rules. Graph and Teams accept next pages only from their
   base, and Teams downloads without the token. The LLM is the URL the owner typed. OAuth's token URLs are
   fixed. They gain only what point 1 gives every `http::send` user.
6. **IMAP, POP, SMTP, ManageSieve** connect only to configured hosts and follow no redirects: unchanged.
7. **EWS requests and its notification stream** (SOAP, NTLM, and the stream's affinity cookie in
   `ews/listening.rs`) go through `http::Conn`. It follows no redirect and refuses a request for another
   server (`http.rs`, `Conn::reaches`): unchanged.
8. **The Autodiscover test filter** (`autodiscover_answer`) accepts `http://localhost` as well as
   `http://127.0.0.1` under `cfg(test)`, so tests can name a second site.

## How the question travels (the password prompt's pattern)

- Net: a new `NetError::Untrusted { site: String }`.
  - **At connect** (`connect_failed`), a dedicated arm sets `conn = None` and `auth_failed = true`, and
    emits `Event::Status` with a new `SyncStatus::NeedsTrust { site: String }`. There is no back-off.
    The worker waits for a restart, as for `NeedsPassword`.
  - `Untrusted` is retryable (`NetError::is_retryable`). A message waiting to go stays in the Outbox
    until the user answers; it is neither failed nor dropped.
  - **Raised by DAV during a sync**, it emits a new `Event::TrustAsked { account, site }`, and the mail
    sync goes on.
  - The engine treats `NeedsTrust` everywhere it treats `NeedsPassword`: `SyncStatus::is_online()`,
    the statuses that reset the rules' state (`events.rs`), and the status bar.
- Engine:
  - both raise one entry in `Session.ask_trust: Vec<(String, String)>` (account, site), at most once per
    pair per run (`asked_trust: HashSet`);
  - a `NeedsTrust` status shows in the account's status as « {domain} attend votre accord » (en:
    "{domain} is waiting for your answer").
- UI: `MailApp::announce` drains `ask_trust` as it drains `ask_password`, into
  `Dialog::TrustDomain { account, site }`.
  - Unlike the password, it does **not** look at the account's status. A question from DAV arrives
    while the account is online.
  - The dialog opens for every entry, one after the other.
- Commands (CLI and scripts too):
  - `account.trustDomain {account, domain}` adds `site(domain)` to `trusted_domains`, saves, and restarts
    the account (`Job::Start`);
  - `account.distrustDomain {account, domain}` removes it, forgets what was found there (`ews.json`
    if it is on that site, the DAV homes on it, and the time of the last look), saves, and restarts.
    The site stays in `AccountConfig.withdrawn_domains` until it is allowed again: a saved URL on a
    withdrawn site counts for nothing (`trust::withdrawn`), however late a sync on its way saves it.
    Without this the saved URL would put the site back through `extra`, unasked. A site the account
    trusts anyway is left as it is;
  - "Don't allow" runs `account.refuseDomain {account, domain}`, which only gives the notice and records
    the refusal for this run.
  - **`account.trustDomain` and `account.distrustDomain` take the pair out of `asked_trust`.** Otherwise
    a domain removed and asked again in the same run would never be asked: the account would stay silent.
- A refusal isn't kept across runs: the question comes back at the next start, as the password prompt
  does. If that turns out to bother the owner, a persisted `refused_domains` list is a small follow-up.
- `update_account`'s list of fields that restart the account gains `trusted_domains`. So do the two
  places that list every `AccountConfig` field (`cmd/accounts.rs` add, `AccountConfig::guess`).

## Errors and limits

- A domain `url_host` can't read is refused by the commands with "not a domain: {d}". Whatever it reads is
  taken, a single label and an IP address too: a question always has an answer.
- `Untrusted` is not an authentication failure: it never opens the password prompt and never counts
  towards back-off. Once it is answered, the account starts again.
- The log carries the site, never an address or a credential.
- Hostile input stays capped as today (DAV hrefs, JMAP session sizes). The new checks are pure string
  work on host names already parsed.

## Testing

- **Model:**
  - `site()`: case, trailing dot, `co.uk`, IP literals, `localhost`, a single label, `.local`, `xn--`
    with capitals;
  - `Trust::of` from a config, every server kind, `trusted_domains`, the `extra` EWS URL;
  - the 20-entry cap.
- **HTTP chain:**
  - same site, another port: kept;
  - another site: dropped;
  - another site in `trusting`: kept;
  - A→B→A: dropped from B on;
  - `Response.url` is the final URL.
- **DAV:**
  - a well-known redirect to the same site with credentials;
  - to another site: `Untrusted`, and the other host never receives `Authorization`;
  - the same after `trust` adds the site: it works;
  - a principal or home href on another site after a same-site start: never requested at all,
    `Untrusted` (the review's multi-request case);
  - a first start redirected to `localhost` while a second start answers: the home is found, and no
    question is asked;
  - an http href under https: refused;
  - `Untrusted` in a PUT or DELETE loop: propagated, not collected into `errors`;
  - a saved home on another site (given as `extra`): still synced, with no question.
- **JMAP:**
  - the session on the same site via a redirect;
  - `apiUrl` on another site: `Untrusted` before anything is sent there;
  - a `downloadUrl` on another site (Fastmail's case): `Untrusted`, then it connects once trusted;
  - after the site is trusted: connects;
  - `Response.url` is filled natively.
- **Autodiscover:**
  - a `RedirectAddr` to another domain is not followed (the fake EWS records nothing there);
  - an `EwsUrl` on another site gives `Untrusted`;
  - both work once trusted;
  - the hybrid chain (a `RedirectAddr`, then an `EwsUrl` on a third site) gives two questions, then
    works.
- **Worker:**
  - `Untrusted` at connect gives `NeedsTrust`, with no back-off;
  - a message in the Outbox stays there until the domain is trusted, then goes.
- **Engine:**
  - `NeedsTrust` and `TrustAsked` raise one question per pair per run;
  - `account.trustDomain` saves, restarts, and lets the pair be asked again later in the run;
  - `account.refuseDomain` gives the notice and no second question;
  - `account.distrustDomain` removes and restarts;
  - `every_command_survives_junk_params`.
- **UI:**
  - the dialog draws in both themes and in French;
  - Allow and Don't allow run their commands;
  - the dialog never opens over another one, and opens for an online account;
  - Server settings list, add and remove domains.
- **Two hosts in tests:** the fakes bind `127.0.0.1` only. `127.0.0.1` and `localhost` are two sites that
  reach the same server, which gives tests their "other domain". Unit tests cover real names.

## Out of scope

- Asking about an IMAP, POP or SMTP host on another domain found by autoconfig at account setup. The user
  confirms those in the form already.
- Certificate pinning or a TLS trust prompt.
- Graph and Teams beyond the generic chain.
- Revoking a domain on the server side, and any sync of `trusted_domains` across computers.

## Delivery

One branch, continuing `jeton-redirection`, merged once its final review is clean. Then a test build.
