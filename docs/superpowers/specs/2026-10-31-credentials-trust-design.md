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
  "Remove" (« Autres serveurs autorisés », en: "Other servers allowed"). Removing one restarts the
  account.
- **Existing accounts keep working with nothing to answer.** Every server already in their settings is
  trusted. The EWS URL an earlier Autodiscover found (`ews.json`) counts too.

## The rule, precisely

**A site** is a host's registrable domain: `api.fastmail.com` → `fastmail.com`,
`mail.example.co.uk` → `example.co.uk`.
- It comes from the Public Suffix List, through the `psl` crate (MIT OR Apache-2.0, pure Rust, no I/O,
  builds for wasm). `cargo deny check` must accept it.
- If it doesn't, the existing heuristic `autoconfig::base_domain` is moved to the model and used instead.
  The cost then: shared-hosting suffixes (`github.io`) read as one site.
- Host names are compared in lower case, without a trailing dot.
- An IP literal (`127.0.0.1`, `[::1]`) is its own site; `localhost` is its own site.

**An account's trusted sites** are the sites of:
- the account's address domain (`email`);
- every configured server: IMAP, SMTP, POP, JMAP, Graph, EWS (the URL typed, or the one Autodiscover
  found and saved in `ews.json`);
- each domain in a new field `AccountConfig.trusted_domains: Vec<String>`. It is saved in `accounts.json`
  with `#[serde(default, skip_serializing_if = "Vec::is_empty")]`, so old files load. It holds at most 20
  entries; a 21st replaces nothing and says "at most 20 other servers".

The model computes this as `Trust::of(&AccountConfig, extra: &[&str]) -> Trust`, with
`Trust::allows(host) -> bool` and `Trust::site(host) -> String`. `extra` is for URLs known only to the
worker (the saved EWS URL).

**Where it applies:**
1. **The generic HTTP redirect chain** (`http/redirect.rs`).
   - A redirect keeps the credentials when the next host's site is the first URL's site, or is in the
     request's trusted sites (a new `Request::trusting(Vec<String>)`, empty by default). Otherwise they
     are dropped for the rest of the chain, as the branch does.
   - https→http stays refused.
   - The response says where it ended: a new `Response.url: Option<String>`, the final URL after
     redirects.
2. **DAV** (`Dav`). `Dav::new` takes the account's `Trust`. Every request — PROPFIND, REPORT, PUT,
   DELETE, and those built from hrefs — carries `Authorization` only when its host is trusted.
   - A discovery that ends on a host that isn't trusted returns `NetError::Untrusted { site }` instead of
     "not here".
   - A home, a collection or an item href on an untrusted host is never requested with credentials. The
     sync of that calendar stops with `Untrusted`.
3. **JMAP** (`crates/jmap`, portable). `Client::connect` takes the trusted sites.
   - The session GET goes with them (`Request::trusting`).
   - A 401 whose final URL is on a site that isn't trusted gives `Untrusted { site }`.
   - The session's `apiUrl`, `downloadUrl`, `uploadUrl` and `eventSourceUrl` must each be on a trusted
     site, else `connect` fails with `Untrusted { site }` before sending anything there.
   - The web app (`apps/quatrelle-web`) passes the same list. There, a refusal shows as the account's
     error. The question dialog is the native app's; in the browser, the error's text names the domain.
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

## How the question travels (the password prompt's pattern)

- Net: a new `NetError::Untrusted { site: String }`. At connect, it does what `NeedsPassword` does: the
  account's worker stops retrying and emits `Event::Status` with a new
  `SyncStatus::NeedsTrust { site: String }`. Raised by DAV during a sync, it emits a new
  `Event::TrustAsked { account, site }`, and the mail sync goes on.
- Engine:
  - both raise one entry in `Session.ask_trust: Vec<(String, String)>` (account, site), at most once per
    pair per run (`asked_trust: HashSet`);
  - a `NeedsTrust` status shows in the account's status as « {domain} attend votre accord » (en:
    "{domain} is waiting for your answer").
- UI: `MailApp::announce` drains `ask_trust` as it drains `ask_password`, into
  `Dialog::TrustDomain { account, site }`.
- Commands (CLI and scripts too):
  - `account.trustDomain {account, domain}` adds `site(domain)` to `trusted_domains`, saves, and restarts
    the account (`Job::Start`);
  - `account.distrustDomain {account, domain}` removes it, saves, and restarts;
  - "Don't allow" runs `account.refuseDomain {account, domain}`, which only gives the notice and records
    the refusal for this run.
- `update_account`'s list of fields that restart the account gains `trusted_domains`. So do the two
  places that list every `AccountConfig` field (`cmd/accounts.rs` add, `AccountConfig::guess`).

## Errors and limits

- An empty or invalid domain (not a host name) is refused by the commands with "not a domain: {d}".
- `Untrusted` is not an authentication failure: it never opens the password prompt and never counts
  towards back-off. Once it is answered, the account starts again.
- The log carries the site, never an address or a credential.
- Hostile input stays capped as today (DAV hrefs, JMAP session sizes). The new checks are pure string
  work on host names already parsed.

## Testing

- **Model:**
  - `site()`: case, trailing dot, `co.uk`, IP literals, `localhost`, IDN as given;
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
  - a principal or home href on another site after a same-site start: never requested with credentials.

  This is the review's multi-request case.
- **JMAP:**
  - the session on the same site via a redirect;
  - `apiUrl` on another site: `Untrusted` before anything is sent there;
  - after the site is trusted: connects.
- **Autodiscover:** a `RedirectAddr` to another domain is not followed (the fake EWS records nothing
  there); an `EwsUrl` on another site gives `Untrusted`; both work once trusted.
- **Engine:**
  - `NeedsTrust` and `TrustAsked` raise one question per pair per run;
  - `account.trustDomain` saves and restarts;
  - `account.refuseDomain` gives the notice and no second question;
  - `account.distrustDomain` removes and restarts;
  - `every_command_survives_junk_params`.
- **UI:**
  - the dialog draws in both themes and in French;
  - Allow and Don't allow run their commands;
  - the dialog never opens over another one;
  - Server settings list and remove domains.
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
