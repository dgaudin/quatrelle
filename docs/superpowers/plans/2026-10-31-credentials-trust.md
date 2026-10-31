# Credentials go only to the servers an account trusts — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Credentials (Basic password, NTLM/Negotiate answers, OAuth bearer, cookies) go only to an account's trusted sites:
- its own servers and their registrable domains;
- domains it already uses (`ews.json`, saved DAV homes);
- domains the user allowed when asked.

Nothing at all is sent to an untrusted host. The question « Le serveur propose de configurer {account} avec {domain}… Autoriser ? » asks once per account and domain, and the answer is remembered.

**Architecture:**
- A pure `Trust` in the model (`addr/trust.rs`, sites from the Public Suffix List) is built for each account.
- The net layer checks it where credentials leave:
  - the HTTP redirect chain keeps them within the first URL's site or the trusted sites;
  - DAV, JMAP and Autodiscover refuse an untrusted host with `NetError::Untrusted { site }`.
- The worker turns that into `SyncStatus::NeedsTrust` (at connect) or `Event::TrustAsked` (during a DAV sync).
- The engine queues one question per pair. The UI asks it as it asks for a password.
- Commands record the answer in `AccountConfig.trusted_domains` and restart the account.

**Tech Stack:** Rust, egui; new dependency `psl` (Public Suffix List), if `cargo deny` and the build cost allow (Task 1 decides).

**Spec:** `docs/superpowers/specs/2026-10-31-credentials-trust-design.md` (the owner's rule, 2026-10-31; reviewed). The branch `jeton-redirection` already holds 1a809b6e, f868d2d2 and 02e66fdf (credentials dropped at an origin change); this plan replaces their origin rule with the site rule and completes it.

## Global Constraints

**Code**
- Repo rules: `CLAUDE.md`.
  - No `unwrap`/`expect`/`panic!` outside tests, and no `unsafe`.
  - Hostile input is capped.
  - Portable crates (model, jmap) never read the clock or temp files, and wasm keeps building (`cargo xtask wasm` is part of ci).
- The log carries a site, never an address, a query or a credential.

**Sizes**
- At most 1000 lines per file and 16 `.rs` per folder; split before ~800, by topic, keeping visibility (CLAUDE.md "File size").
- Full folders: `model/src` (16), `engine/src/session` (16), `engine/src/tests` (16), `ui-egui/src/tests` (16). Use subfolders.
- New files, by task:
  - Task 1: `model/src/addr/trust.rs`;
  - Task 1: `engine/src/session/mailboxes/trust.rs`;
  - Task 1: `engine/src/tests/accounts/trust.rs`, declared from `tests/accounts.rs`;
  - Task 6: `ui-egui/src/tests/writing/trust.rs`, declared from `tests/writing.rs`, or a new `tests/accounts/` subfolder declared from `tests/accounts.rs`.
- Near the limit:
  - `net/src/protocols/dav.rs` (729): Task 3 first splits it by topic into `protocols/dav/` (discovery, sync loops), with no behaviour change, then adds to it.
  - `jmap/src/lib.rs` (729): Task 4 moves the session's connection code to `jmap/src/session.rs` first.
  - `model/src/lib.rs` (731): only the new field.

**Interface text**
- Through `tl!`/`tf!`, with a row in all 13 catalogs `crates/ui-egui/src/i18n/*.tsv`.
- New strings, English → French, verbatim:

| English | French |
|---|---|
| "Allow another server?" | « Autoriser un autre serveur ? » |
| "The server proposes to set up {account} with {domain}. Your password or sign-in will be sent there. Allow it?" | « Le serveur propose de configurer {account} avec {domain}. Votre mot de passe ou votre connexion y seront envoyés. Autoriser ? » |
| "Allow" | « Autoriser » |
| "Don't allow" | « Ne pas autoriser » |
| "{domain} wasn't allowed: {account} won't use it" | « {domain} n'a pas été autorisé : {account} ne l'utilisera pas » |
| "{domain} is waiting for your answer" | « {domain} attend votre accord » |
| "Other servers allowed" | « Autres serveurs autorisés » |
| "Add" | « Ajouter » (reuse it if it exists) |
| "Remove" | « Retirer » (reuse it if it exists) |

**Commands**
- `account.trustDomain`, `account.distrustDomain` and `account.refuseDomain` take `{"account": string, "domain": string}`.
- Errors:
  - "not a domain: {d}";
  - "at most 20 other servers";
  - "no account `{id}`".
- `every_command_survives_junk_params` stays green.

**Memory and gates, every cargo command**
- First `mkdir -p ~/.tq`, then `TMPDIR=$HOME/.tq CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=4 CARGO_TARGET_DIR=$PWD/target cargo …`, after checking that `pgrep -a cargo` shows no other build (one heavy build at a time on this machine).
- The stable clippy uses `CARGO_TARGET_DIR=$PWD/target-stable`.
- Gates before each task's last commit:
  - `cargo fmt --all`;
  - `cargo xtask ci`;
  - stable clippy `--workspace --all-targets -D warnings`;
  - `cargo deny check` when a dependency changes.
- Known flake: `a_big_folder_changes_all_at_once_quickly`; rerun it alone once.
- Temp files go only under the session scratchpad. Never touch `/usr/bin/quatrelle` or `~/.local/share/quatrelle`. Never test against a real account.

**Commits**
- English, `Area: what changed`, with a prose body.
- End with `Co-Authored-By: Claude <model name> <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_019dhnBJAQnyMdiQr8krNEX4`.
- Run each new test once against the old code to see it fail.

**Two sites in tests:** the fakes bind `127.0.0.1` only. `127.0.0.1` and `localhost` are two sites reaching the same server: "another domain" in integration tests. Real names are covered in unit tests.

## Review Focus

1. **The owner's on-prem Exchange (NTLM) must keep working with no question.** Its server and `ews.json` are trusted. Test: `an_exchange_account_found_by_autodiscover_before_keeps_working_unasked` (Task 5).
2. **A website that redirects everything elsewhere must not ask, when CalDAV is found at another start.** Test: `a_start_redirected_elsewhere_is_skipped_when_another_finds_the_home` (Task 3).
3. **Credentials never reach an untrusted host through a URL a server hands out**, whether a DAV href or a JMAP `apiUrl`. That is the review's multi-request case. Tests: `an_href_on_another_site_is_never_requested` (Task 3) and `a_session_url_on_another_site_is_refused_before_anything_is_sent` (Task 4).
4. **A question removed and asked again in the same run is asked again**, not swallowed by the dedup. Test: `a_domain_removed_is_asked_again_in_the_same_run` (Task 1/6).
5. **An outgoing message waiting on an untrusted domain stays in the Outbox and goes once allowed.** Test: `a_message_waits_in_the_outbox_for_the_answer` (Task 5).

---

### Task 1: Sites, `Trust`, the account's `trusted_domains` and the three commands — `sonnet`, effort `high`

**Files:**
- Create:
  - `crates/model/src/addr/trust.rs` (declared `pub mod trust;` in `addr.rs`);
  - `crates/engine/src/session/mailboxes/trust.rs`;
  - `crates/engine/src/tests/accounts/trust.rs`.
- Modify:
  - `crates/model/src/lib.rs` (`AccountConfig.trusted_domains`);
  - `crates/model/Cargo.toml` and the workspace `Cargo.toml` (`psl`);
  - `crates/engine/src/cmd/accounts.rs` (three commands; the add-account literal gains the field);
  - `crates/model/src/addr/providers.rs` (`AccountConfig::guess`, the field);
  - `crates/engine/src/session/accounts.rs` (`update_account`'s `reconnect` list gains `trusted_domains`);
  - `crates/engine/src/session.rs` (`ask_trust: Vec<(String, String)>`, `asked_trust: HashSet<(String, String)>`, `refused_trust: HashSet<(String, String)>`).

**Interfaces (produced):**
- `pub fn site(host: &str) -> String`.
  - Lower case, trailing dot removed, the ASCII form as given.
  - `psl::domain_str` gives the registrable domain; when it gives `None`, the site is the normalised host itself.
- `pub struct Trust { sites: BTreeSet<String> }`:
  - `pub fn of(acc: &AccountConfig, extra: &[&str]) -> Trust`: the sites of `acc.email`'s domain, the IMAP/SMTP/POP hosts, the hosts of the `jmap`/`graph`/`ews` URLs (when they are URLs), each `trusted_domains` entry, and each `extra` URL's or host's site (empty or unparsable entries are ignored);
  - `pub fn allows(&self, host: &str) -> bool`;
  - `pub fn sites(&self) -> Vec<String>`.

  A URL's host is read with the model's own parsing (the scheme, then the authority up to `/`, `?`, `#`; userinfo or a bracketed IPv6 handled). If none exists in the model, add a small `fn url_host(url: &str) -> Option<String>` here with tests.
- `AccountConfig.trusted_domains: Vec<String>`, `#[serde(default, skip_serializing_if = "Vec::is_empty")]`.
- In `Session`:
  - `pub fn trust_domain(&mut self, account: &str, domain: &str) -> CmdResult`:
    - validates the domain (a host name: letters, digits, `.`, `-`, at most 253 characters, at least one dot or `localhost`; else "not a domain: {d}");
    - stores `site(domain)` (no duplicate; a 21st entry is refused with "at most 20 other servers");
    - removes the pair from `asked_trust` and `refused_trust`, saves the accounts, pushes `Job::Start`.
  - `pub fn distrust_domain(&mut self, account: &str, domain: &str) -> CmdResult`: the reverse. It also removes the pair from `asked_trust` and restarts.
  - `pub fn refuse_domain(&mut self, account: &str, domain: &str) -> CmdResult`: records the pair in `refused_trust` and gives the notice "{domain} wasn't allowed: {account} won't use it", with the account's address.
  - `pub(crate) fn ask_trust_once(&mut self, account: &str, site: &str)`: queues the pair in `ask_trust` unless it is in `asked_trust` or `refused_trust`, and records it in `asked_trust`.

  Tasks 5 and 6 call it.
- Commands `account.trustDomain`, `account.distrustDomain` and `account.refuseDomain` (params doc as in Global Constraints).

- [ ] **Step 1: Measure `psl`.** Add it, then run `cargo deny check`, the release build time of `quatrelle-model` and the size of the `quatrelle` binary, before and after.
  - Unreasonable (more than about 60 s of build or 3 MB of binary) or refused: stop and report with the numbers (BLOCKED). The controller decides on the fallback.
  - Otherwise, write the numbers in the report.
- [ ] **Step 2: Failing tests.**
  - Model:
    - `sites_are_registrable_domains`: `api.fastmail.com`→`fastmail.com`; `mail.example.co.uk`→`example.co.uk`; `MAIL.Example.COM.`→`example.com`; `exch01.corp.local`→`corp.local`; `exchange01`→`exchange01`; `localhost`→`localhost`; `127.0.0.1`→`127.0.0.1`; `[::1]`→`[::1]`; `xn--bcher-kva.example`→`xn--bcher-kva.example`;
    - `an_account_trusts_its_servers_and_what_it_was_given`: a config with IMAP `imap.mail.me.com`, `trusted_domains ["icloud.com"]`, `extra ["https://caldav.icloud.com/123/"]`; it allows `p01-caldav.icloud.com` and `smtp.mail.me.com`, and not `evil.example`.
  - Engine, in `tests/accounts/trust.rs`:
    - `trusting_a_domain_saves_it_and_restarts_the_account`;
    - `twenty_other_servers_at_most`;
    - `not_a_domain_is_refused`;
    - `a_refused_domain_is_not_asked_again_this_run`;
    - `a_domain_removed_is_asked_again_in_the_same_run`: `ask_trust_once`, then `trust_domain`, then `distrust_domain`, then `ask_trust_once` again queues it.
- [ ] **Step 3: Implement**, as the Interfaces say.
- [ ] **Step 4: Gates.** Then commit (`Accounts: the servers an account trusts, by registrable domain`).

---

### Task 2: The HTTP chain keeps credentials within the site, or the trusted sites — `sonnet`, effort `high`

**Files:**
- Modify:
  - `crates/net/src/http/redirect.rs`;
  - `crates/net/src/http.rs` (`Request::trusting`, `Response.url`, `send_to` sets it);
  - `crates/net/src/http/streamed.rs` (`get_into` sets `url` too);
  - `crates/net/src/lib.rs` (`NetError::Untrusted { site: String }`, error text "{site} isn't allowed for this account"; `is_retryable` includes it; `is_fatal` doesn't).

**Interfaces:**
- Consumes: Task 1's `quatrelle_model::addr::trust::site`.
- Produces:
  - `Request::trusting(self, sites: Vec<String>) -> Request`; the field `trusted: Vec<String>` is empty by default;
  - `Response.url: Option<String>`, the final URL after redirects;
  - `NetError::Untrusted { site }`.
- The chain's rule: a hop keeps the credential headers when `site(next.host)` equals `site(first.host)` or is in `req.trusted`. Otherwise they are dropped for the rest of the chain, as now. https→http stays refused.

- [ ] **Step 1: Failing tests** (in `http/redirect.rs` tests, with `127.0.0.1` and `localhost` as two sites):
  - `another_port_of_the_same_host_keeps_the_credentials`. This replaces the branch's "another port drops"; same host, same site.
  - `another_site_drops_them_for_good`: `127.0.0.1` → `localhost` → `127.0.0.1`.
  - `a_trusted_site_keeps_them`: `localhost` in `trusting`.
  - `the_answer_says_where_it_ended`.
  - Adjust the branch's existing tests to the site rule. Their intent ("never to another origin") becomes "never to another site".
- [ ] **Step 2: Implement.**
- [ ] **Step 3: Gates.** Then commit (`HTTP: credentials stay within the first site or those the account trusts`).

---

### Task 3: DAV sends nothing to an untrusted host; discovery tries every start — `opus`, effort `high`

**Files:**
- Split first, with no behaviour change, as its own commit: `crates/net/src/protocols/dav.rs` (729) into `protocols/dav/` pieces by topic (for example `dav/discover.rs`, `dav/sync.rs`). The tests move with them.
- Then modify:
  - `Dav::new(user, password, trust: Trust)`;
  - `request`, `put`, `delete`, `discover`, `home_from`, `collections`, `etags`, `multiget`, `absolute`;
  - `dav::sync`'s loops;
  - `crates/net/src/worker/sync.rs` `sync_dav`: it builds the `Trust` with the saved homes as `extra`, and turns `Untrusted` into `Event::TrustAsked`;
  - the `Event` enum (`crates/model/src/job/events.rs`: `TrustAsked { account: String, site: String }`).

**Interfaces:**
- Consumes: Task 1's `Trust`, and Task 2's `NetError::Untrusted`, `Request::trusting` and `Response.url`.
- The rule, from the spec's point 2:
  - **nothing is sent to an untrusted host**: before each request and each redirect hop, check `trust.allows(host)`, else `Err(Untrusted { site })`;
  - an href from https down to http is `Err(Tls)`;
  - `discover` tries every start, notes the untrusted ones, lets a found home win with no question, and only if none is found returns `Untrusted` of the first noted;
  - an SRV target on another site is such a start;
  - the sync loops propagate `Untrusted` like `Auth`, never into `out.errors`;
  - after a refusal (no question answered), `sync_dav` does nothing more for the account in that run. It sends `TrustAsked` once; the engine dedups.
- `sync_dav`'s trust: `Trust::of(&acc, &[caldav_home, carddav_home])`. The homes come from the stored PIM doc the worker already reads for DAV. Name where in the report.

- [ ] **Step 1: The split commit, then failing tests** (with `fake_dav`, and `127.0.0.1` vs `localhost`):
  - `a_well_known_redirect_within_the_site_signs_in`;
  - `a_well_known_redirect_to_another_site_is_untrusted_and_sends_nothing_there`: the second server records no request at all;
  - `once_trusted_it_works`;
  - `an_href_on_another_site_is_never_requested` (Review Focus 3);
  - `a_start_redirected_elsewhere_is_skipped_when_another_finds_the_home` (Review Focus 2);
  - `an_http_href_under_https_is_refused`;
  - `untrusted_in_a_put_loop_is_propagated_not_collected`;
  - `a_saved_home_on_another_site_still_syncs_unasked`.
- [ ] **Step 2: Implement.**
- [ ] **Step 3: Gates.** Then commit (`CalDAV/CardDAV: nothing goes to a server the account doesn't trust; discovery asks only when nothing else answers`).

---

### Task 4: JMAP refuses an untrusted session URL; the web app's limit — `opus`, effort `medium`

**Files:**
- First: move the session/connect code out of `crates/jmap/src/lib.rs` (729) into `crates/jmap/src/session.rs`, with no behaviour change, as its own commit.
- Then modify:
  - `jmap::Request` (`trusting`), `jmap::Response` (`url: Option<String>`), `jmap::Error` (`Untrusted { site }`);
  - `Client::connect(…, trusted: &[String])`;
  - `crates/net/src/protocols/jmap.rs`: `Blocking` passes `trusting` and fills `url` from `http::Response.url`, and both `From` mappings handle `Untrusted`;
  - the worker's JMAP connect (`worker/connect.rs`, `worker.rs` `connect_for_sync`): it passes `Trust::of(&acc, &[]).sites()`;
  - `apps/quatrelle-web/src/fetch.rs`: fills `url` from `web_sys::Response::url()`. `trusting` can't be honoured there; say so in a comment;
  - the web app's docs (where its accounts are described), with one paragraph on typing the provider's session URL.

**Interfaces:**
- Consumes: Tasks 1 and 2.
- The rule, from the spec's point 3:
  - a 401 whose `url` is on an untrusted site gives `Untrusted`;
  - each of `apiUrl`, `downloadUrl`, `uploadUrl` and `eventSourceUrl` (resolved) must be on a trusted site, or `connect` fails with `Untrusted` before anything is sent there. The jmap crate is portable: it does its own `site` through `quatrelle_model::addr::trust::site`. Check that `jmap` may depend on `model`; if not, take the trusted sites in and compare with a `site` function given by the caller. Rule on the simplest, and report.

- [ ] **Step 1: The move commit, then failing tests** (`fake_jmap`, `127.0.0.1` vs `localhost`):
  - `a_session_within_the_site_connects`;
  - `a_session_url_on_another_site_is_refused_before_anything_is_sent` (Review Focus 3: the fake records no API call);
  - `a_download_url_on_another_site_is_untrusted_then_works_once_trusted` (Fastmail's case);
  - `the_final_url_is_reported_natively`.
- [ ] **Step 2: Implement.**
- [ ] **Step 3: Gates.** wasm is included in ci. Then commit (`JMAP: a session's URLs must be on a site the account trusts`).

---

### Task 5: Autodiscover asks before another domain; the worker waits for the answer — `opus`, effort `high`

**Files:**
- Modify:
  - `crates/net/src/exchange/ews/autodiscover.rs`: `autodiscover_as(…, trust: &Trust)`; `RedirectAddr` and `EwsUrl` are checked; the `cfg(test)` filter accepts `http://localhost` too;
  - `crates/net/src/worker/connect.rs`:
    - the trust for the account, built with `ews.json`'s saved URL as `extra`;
    - `connect_failed` gets an arm for `Untrusted`: `conn = None`, `auth_failed = true`, `status(SyncStatus::NeedsTrust { site })`, and no back-off;
  - `crates/net/src/worker.rs` (`connect_for_sync`, the same);
  - `crates/model/src/job.rs` (`SyncStatus::NeedsTrust { site: String }`, and `is_online()` treats it as `NeedsPassword` is treated);
  - the Outbox path: confirm a send failing with `Untrusted` stays queued (it is retryable from Task 2), and is sent after the restart.

**Interfaces:**
- Consumes: Tasks 1 and 2.
- Produces: `SyncStatus::NeedsTrust { site }`, which Task 6 handles in the engine.

- [ ] **Step 1: Failing tests** (`fake_ews`, `127.0.0.1` vs `localhost`):
  - `a_redirect_addr_to_another_domain_is_not_followed` (the second server records nothing);
  - `an_ews_url_on_another_site_is_untrusted`;
  - `both_work_once_trusted`;
  - `the_hybrid_chain_asks_twice_then_works`;
  - `an_exchange_account_found_by_autodiscover_before_keeps_working_unasked` (Review Focus 1: `ews.json` holds a URL on another site; the account connects, with no `NeedsTrust`);
  - `untrusted_at_connect_needs_trust_with_no_backoff`;
  - `a_message_waits_in_the_outbox_for_the_answer` (Review Focus 5).
- [ ] **Step 2: Implement.**
- [ ] **Step 3: Gates.** Then commit (`Exchange: Autodiscover asks before another domain, and the account waits for the answer`).

---

### Task 6: The question in the engine and the window; Server settings list the domains — `sonnet`, effort `high`

**Files:**
- Modify:
  - `crates/engine/src/session/events.rs`:
    - `Event::TrustAsked` → `ask_trust_once`;
    - `Event::Status` with `NeedsTrust` → `ask_trust_once`, plus the status bar's words "{domain} is waiting for your answer";
    - `NeedsTrust` is added wherever `NeedsPassword` resets the rules' state;
  - `crates/ui-egui/src/app/actions.rs` `announce`: drains `ask_trust` into `Dialog::TrustDomain { account, site }`, without looking at the account's status, only when no dialog or File page is open;
  - `crates/ui-egui/src/dialogs.rs` (the variant) and `dialogs/accounts.rs` (its drawing: title, text, Allow → `account.trustDomain`, Don't allow → `account.refuseDomain`);
  - the Server settings form (`dialogs/accounts.rs` `edit_account`): "Other servers allowed", with the list, Remove (`account.distrustDomain`), and a field with Add (`account.trustDomain`);
  - the 13 catalogs;
  - `tests.rs`'s dialog gallery.

**Interfaces:**
- Consumes: Task 1's `ask_trust_once`, `Session.ask_trust` and the three commands; Task 3's `Event::TrustAsked`; Task 5's `SyncStatus::NeedsTrust`.

- [ ] **Step 1: Failing tests.**
  - Engine:
    - `a_status_needing_trust_asks_once`;
    - `trust_asked_during_a_sync_asks_once`.
  - UI:
    - `the_question_opens_for_an_online_account_and_not_over_another_dialog`;
    - `allow_trusts_and_restarts`;
    - `dont_allow_says_so_and_asks_no_more`;
    - `server_settings_list_add_and_remove_domains`.
  - Gallery: the dialog in both themes.
- [ ] **Step 2: Implement.**
- [ ] **Step 3: Look at it.** Take a screenshot of the dialog in French (sample data in a fresh scratch folder; the Quatrelle window only), and read the PNG.
- [ ] **Step 4: Gates.** Then commit (`Accounts: Quatrelle asks before using another domain, and Server settings list those allowed`).

---

## After the tasks

- A final whole-branch review on `fable`, effort `xhigh`, against the spec and this plan, including the three branch commits from before it.
- One fix wave, then a scoped re-review.
- Merge to main and publish a test build.
- Update the carnet items `jeton-redirection` and `autodiscover-redirection`.
