# Sender check — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Warn when a message wears a known person's name, or a known signature, on someone else's
address; optionally show the sender's address in the list and tag mail from outside the organisation.

**Architecture:** The model's name comparison is picked from upstream (`lookalike/people.rs`,
`lookalike/external.rs`); the model gains `lookup_name` and a signature reader (`lookalike/signature.rs`).
The engine gathers who is known once per change of a stamp (accounts, PIM, address book revision,
organisation domains) in `session/opening/senders.rs`, asks Exchange's directory for an unknown
person's name when a message is opened, and sets the open messages' warnings. The UI draws the
banner, the list row's marks and the Options rows.

**Tech Stack:** Rust 2024, egui 0.37, serde_json, proptest, cargo-fuzz; upstream commits reachable in
this repository (`git show <commit>`; upstream says `mailcraft`, here `quatrelle`).

**Spec:** `docs/superpowers/specs/2026-11-04-sender-check-design.md`

## Global Constraints

- CLAUDE.md applies whole: no `unwrap`/`expect`/`panic!`/indexing that can panic in non-test code; `get()`, checked or saturating arithmetic, char-boundary slicing.
- A new `.rs` file starts with `// SPDX-License-Identifier: MIT OR Apache-2.0` then `// Copyright (c) 2026 Didier Gaudin`; a file holding upstream code has `// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors` before the Didier Gaudin line (`cargo xtask authors`).
- At most 1000 lines per Rust file, 16 Rust files per folder. Full folders: `crates/model/src`, `crates/engine/src/session`, `crates/engine/src/tests`, `crates/ui-egui/src/tests` — new files go only where the tasks say. `crates/ui-egui/src/tests/reading.rs` (816 lines) takes only a `mod` line; `crates/ui-egui/src/reading.rs` (781) shrinks before it changes.
- Interface text through `tl!`/`tf!`, a row in each of the 13 catalogs `crates/ui-egui/src/i18n/*.tsv`; French wording as the spec gives it.
- Examples in code and tests: invented people at `.example` domains and the big mail services; nothing about the owner's employer.
- `view.senderAddresses` defaults to **never** and `external_tag` to **false** (upstream's default for addresses is `strangers`: not here).
- Builds one at a time, `TMPDIR=$HOME/.tq`; never two committing agents in this checkout. Work on branch `sender-check`.
- Before each commit: `cargo test -p <crate>`, `cargo clippy -p <crate> --all-targets -- -D warnings`, `cargo fmt --check`; the last task runs `cargo xtask ci`.
- Commit messages in English, the repo's style ("Area: what changed"), ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Picks are committed by `~/mailcraft/merge-tools/run_picks.sh` / `commit_pick.sh` (upstream author kept).

## Review Focus

1. An outsider's Outlook reply quoting the user's own message, the user's signature (`www.studio.example`, `zoe@studio.example`) in the quote: no warning → Task 2 (own addresses never count) and Task 4 test on a parsed HTML reply.
2. Arrowing down through 20 messages from strangers: at most one sender question waits, and a Check Names question isn't held behind them → Task 4 test.
3. A 1 MB message on one line, 100 000 short lines, multi-byte text across the 64 KiB cut: bounded time, no panic → Task 2 test.
4. A forwarded message (Gmail's dashes, Apple's "Begin forwarded message:" / « Début du message réexpédié : ») carrying the original sender's signature: the forwarder isn't warned for it → Task 2 test.
5. The per-frame gathering never runs while the address book is being read, and never loads it → Task 3 test.

---

### Task 1: Picks — upstream's name check (model)

**Model:** sonnet, effort medium.

**Files:** whatever `14c0bee4` and `22796fda` touch (`crates/model/src/lookalike.rs`, new `crates/model/src/lookalike/{people,external}.rs`, `crates/model/src/contacts.rs`, `crates/model/tests/properties.rs`, `fuzz/fuzz_targets/search.rs`, `fuzz/README.md`, `crates/ui-egui/src/reading.rs`, the 13 catalogs); from `f421f17e` only `crates/mime/src/lib.rs`.

**Interfaces:**
- Produces: `quatrelle_model::lookalike::{SenderCheck, KnownPeople, name_key, is_free_mail, own_domains, clean_domain, is_external}`, `SenderWarning::NameOfKnownPerson { name, known, real }`, `SenderCheck::{new, domains, people, is_own, knows, check}`, `ContactBook::written()`, `mime::Parsed::bulk: bool`.

- [ ] **Step 1:** `git switch -c sender-check`, then `~/mailcraft/merge-tools/run_picks.sh 14c0bee4 22796fda`. On a conflict, resolve toward the fork (the fork's `lookalike.rs` is upstream's base, so conflicts should be in catalogs only), `git add`, `~/mailcraft/merge-tools/commit_pick.sh <commit>`.
- [ ] **Step 2:** `~/mailcraft/merge-tools/pick.sh f421f17e`, then keep only `crates/mime/src/lib.rs` (`git restore --staged --worktree` every other path); the engine part is rewritten in Task 3.
- [ ] **Step 3: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-model -p quatrelle-mime` and `cargo build -p quatrelle-engine -p quatrelle-ui-egui` — expect PASS (the engine still calls `check_sender`; keep it if the pick removed it, as a wrapper over a `SenderCheck` built from the known domains only).
- [ ] **Step 4: Commit** with `commit_pick.sh f421f17e "the mime part only (Parsed::bulk); the engine part is rewritten for the fork's address book"`.

### Task 2: Model — `lookup_name`, the signature reader and its rule

**Model:** opus, effort high (a parser of hostile text).

**Files:**
- Modify: `crates/model/src/lookalike/people.rs` (add `lookup_name`; add the Didier Gaudin header line), `crates/model/src/lookalike.rs` (`mod signature;`, re-exports, the new variant)
- Create: `crates/model/src/lookalike/signature.rs` (reader, rule, tests in its `#[cfg(test)] mod tests`)
- Modify: `crates/model/tests/properties.rs`, `fuzz/Cargo.toml`, `fuzz/README.md`; Create: `fuzz/fuzz_targets/signature.rs`
- Modify: `crates/ui-egui/src/reading.rs` (the new variant's text) and the 13 catalogs

**Interfaces:**
- Consumes: Task 1's `SenderCheck`, `name_key`, `is_free_mail`; `lookalike.rs`'s private `related` and `domain_of` (reachable from a child module).
- Produces: `pub fn lookup_name(name: &str) -> Option<String>`; `pub fn own_text(text: &str) -> &str`; `pub fn signature_shows(text: &str) -> Vec<Shown>`; `#[derive(Clone, Debug, PartialEq, Eq)] pub enum Shown { Address(String), Site(String) }` (lower case; `Site` holds the domain, `www.` dropped); `SenderWarning::SignatureShowsOther { shown: String, real: String }`; `impl SenderCheck { pub fn check_signature(&self, from: &EmailAddress, text: &str, listed: bool, org: &BTreeSet<String>) -> Option<SenderWarning> }`; all under `quatrelle_model::lookalike::`.

- [ ] **Step 1: Write the failing tests** in `signature.rs`'s tests (people `ana@studio.example`, org `{"studio.example"}`):
  - `lookup_name_as_a_person_types_it`: `"Dr. Dana Whitfield, CEO"` → `Some("Dana Whitfield")`; `"Ｄａｎａ Whitfield"`, `"𝐃𝐚𝐧𝐚 Whitfield"`, `"Dаna Whitfield"` (Cyrillic а), `"Dana\u{200B} Whitfield"`, `"Dana J. Whitfield"` → `Some("Dana Whitfield")`; `"Z0e Park"` → `Some("Zoe Park")`; `"Jérôme Dupré"` → `Some("Jérôme Dupré")`; `"Whitfield, Dana"` → `Some("Whitfield Dana")`; `"IT Support"`, `"Dana"`, `""` → `None`; a 300-letter two-word name → at most 100 chars.
  - `own_text_stops_at_each_quotation_mark`: for each of `-----Original Message-----`, `-----Message d'origine-----`, `---------- Forwarded message ---------`, 32 underscores, `From: Ana <a@b.example>\nSent: Monday`, `De : Ana\nEnvoyé : lundi`, `De\u{a0}: Ana\nEnvoyé\u{a0}: lundi`, `Von: Ana\nGesendet: Montag`, `On Mon, 3 Nov 2026, Ana <a@b.example> wrote:`, `Le lun. 3 nov. 2026 à 10:00, Ana <a@b.example> a écrit :`, `> quoted`, `Begin forwarded message:`, `Début du message réexpédié :` — `own_text("Hello\n\nDana\n" + mark + "\nquoted dana@x.example")` doesn't contain `quoted` and does contain `Dana` (Review Focus 4).
  - `a_from_line_alone_is_no_quote`: `"From: our team, with love\n\nDana\nwww.studio.example"` → `own_text` is the whole text.
  - `the_signature_is_after_dashes_or_the_last_lines`: `"Hi\n-- \nDana\ndana@corp.example"` → `[Address("dana@corp.example")]`; 30 lines with `x@one.example` on line 2 and `y@two.example` on line 29 → only `y@two.example`.
  - `addresses_and_sites_are_read`: `www.Studio.example`, `https://studio.example/about`, `mailto:Dana@Corp.example` → `Site("studio.example")` once, `Address("dana@corp.example")`; 25 addresses → 10 kept.
  - `case_a_the_organisation_from_outside`: from `("Direction", "dg.studio@gmail.com")`, text `"Please wire it today.\n\nDana Whitfield\nCEO\nwww.studio.example"` → `Some(SignatureShowsOther { shown: "studio.example", real: "dg.studio@gmail.com" })`; the same `listed = true` still warns (mail service).
  - `case_b_an_address_known_exactly`: check knowing `("Dana Whitfield", "dana@corp.example")`, from `("Direction Générale", "dg@corp-direction.example")`, signature `dana@corp.example` → shown `dana@corp.example`.
  - `case_c_the_senders_name_on_free_mail`: from `("Dana Whitfield", "dw.ceo@gmail.com")`, signature `dana.whitfield@unknown.example` → warns; from `("Dana Whitfield", "dw@other-corp.example")` → `None`.
  - `what_says_nothing`: sender known (`knows("dg.studio@gmail.com")`) → `None`; `listed = true` from `news@corp-news.example` → `None`; signature `someone@gmail.com` only → `None`; a third party's signature under `De : …\nEnvoyé : …` → `None`; from `ana@studio.example`, signature `www.mail.studio.example` → `None`; the user's own address `ana@studio.example` in the signature of an outsider's mail → `None` (Review Focus 1); from the user's own address → `None`.
  - `hostile_text_in_bounded_time`: 1 000 000 chars on one line, 100 000 lines of `x`, only `\u{200B}`, `"é"` × 40 000 (crosses the 64 KiB cut) → each `check_signature` returns, total < 1 s (Review Focus 3).
- [ ] **Step 2: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-model lookalike::signature` — expect compile failure.
- [ ] **Step 3: Implement.** Constants `READ_MOST = 64 * 1024`, `SIGNATURE_LINES = 15`, `MOST_SHOWN = 10`, `LOOKUP_MOST = 100`. `own_text` cuts at the first mark of the spec's list plus Apple's two lines (case-insensitive; a header block needs a second header line within four lines); the 64 KiB cut is on a char boundary. `check_signature` order: `None` for an empty address, `is_own(from)`, `knows(from)`, or `listed` from a non-mail-service domain; then for each `Shown` in order, skipping mail-service domains, domains `related` to the sender's and the user's own addresses: case a (domain in `org` or under one), case b (`knows(address)`), case c (the address's user part with `.`/`_`/`-` as spaces has the sender's `name_key`, sender on a mail service); the first match is the warning. `lookup_name` keeps accents (narrower than the skeleton's `plain`: other scripts' look-alikes, full-width and mathematical letters only).
- [ ] **Step 4:** In `reading.rs`'s `sender_warning`, the arm `SignatureShowsOther` → `tf!("Careful: the signature gives {shown}, but this comes from {real}.", …)`; fr « Attention : la signature donne {shown}, mais ceci vient de l'adresse {real}. », the 12 other catalogs translated.
- [ ] **Step 5:** `properties.rs`: `signature_ignores_what_follows_a_quote(a in ".{0,400}", b in ".{0,400}")` — `signature_shows(&format!("{a}\n-----Original Message-----\n{b}")) == signature_shows(&format!("{a}\n-----Original Message-----\n"))`; `lookup_name_is_short(s in ".{0,600}")` — `lookup_name(&s).map_or(0, |n| n.chars().count()) <= 100`. Fuzz target `signature`: bytes → lossy text split at the first two `\n` into name, address, text; run `lookup_name`, `signature_shows`, `check_signature` (one known person, org `{"studio.example"}`, `listed` = length is even); row in `fuzz/Cargo.toml` (`# the signature read against the sender`) and `fuzz/README.md`.
- [ ] **Step 6: Run** `cargo test -p quatrelle-model`, `cargo build -p quatrelle-ui-egui`, `cargo test -p quatrelle-ui-egui i18n` — expect PASS.
- [ ] **Step 7: Commit** — "Sender check: the signature read against the sender, and the name as the directory is asked it".

### Task 3: Engine — who is known, gathered once per change; marks, options, commands

**Model:** opus, effort high.

**Files:**
- Create: `crates/engine/src/session/opening/senders.rs`; Modify: `crates/engine/src/session/opening.rs` (`mod senders;`; `known_domains` and `sender_warning` move there), `crates/engine/src/session.rs` (field `senders: Option<Senders>` replaces `known_domains`), `crates/engine/src/session/prefs.rs`, `crates/engine/src/cmd/view.rs`, `crates/engine/src/cmd/navigation.rs`, `crates/engine/src/lib.rs`, `apps/quatrelle-cli/src/main.rs`
- Create: `crates/engine/src/tests/safety/senders.rs`; Modify: `crates/engine/src/tests/safety.rs` (`mod senders;`)
- Reference: `git show 3a87dfa4 f421f17e -- crates/engine apps` (upstream's version, on its own book: don't copy `FRESH_SECS`, `written` kept, `keep_senders_when`).

**Interfaces:**
- Consumes: Task 1's `SenderCheck::new(domains, people, own)`, `own_domains`, `clean_domain`, `is_external`; `ContactBook::written()`; `Session::contacts_revision()`, `contacts_whole()`, `contacts_book()`, `contacts_pending()`, `pim_revision`, the people index (`self.people.index` values, `Indexed::who: Addressee`).
- Produces: `pub enum SenderAddresses { Never, Strangers, Always }` (`code()`, `from_code()`, `Default` = `Never`), prefs `sender_addresses`, `external_tag: bool`, `org_domains: Vec<String>` (saved like `clean_links`); `#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)] pub struct SenderMarks { pub address: bool, pub warning: bool, pub external: bool }`; both re-exported from `quatrelle_engine`. On `Session`: `pub fn keep_senders(&mut self)` (gathers when the stamp changed; reads the whole book first when needed, as `written_domains` does today), `pub fn keep_senders_quietly(&mut self)` (only when the stamp changed, a book is loaded and `!contacts_pending()`; never loads it), `pub fn known_domains(&mut self) -> BTreeSet<String>`, `pub fn org_domains(&mut self) -> BTreeSet<String>`, `pub fn organisation(&self) -> Option<&BTreeSet<String>>`, `pub fn sender_marks(&self, m: &Summary) -> SenderMarks`, `pub fn is_external(&self, email: &str) -> bool`, `pub(crate) fn message_warning(&mut self, key: &MessageKey, parsed: &mime::Parsed) -> Option<SenderWarning>` (`listed` = `parsed.bulk` or the row's `bulk`). Each gathering sets `warning` on the reading pane and every open window again. Commands as upstream's: `view.senderAddresses {"show": "always"|"strangers"|"never"}`, `view.externalTag {"on"?: bool, "domains"?: [string] | string}` (at most 100, each through `clean_domain`), returning `{"on", "domains", "organisation"}`; `list.messages` rows gain `fromAddress`, `senderWarning`, `external`; `message.read` gains `external`.

- [ ] **Step 1: Write the failing tests** in `tests/safety/senders.rs` (account `zoe@studio.example` named "Zoe Park"):
  - `known_people_come_from_accounts_cards_written_and_directory`: a message from `"Ana Lima" <ana.lima.1984@gmail.com>` gives `NameOfKnownPerson { name: "Ana Lima", known: "ana@corp.example", real: "ana.lima.1984@gmail.com" }` when Ana is (a) a card, (b) written to (`contacts_sent`), (c) in an `Event::DirectoryPeople` answer — three sessions; with none, `None`. `"Zoe Park" <zoe.park@outlook.com>` warns with `known: "zoe@studio.example"`.
  - `saving_the_sender_as_a_contact_takes_the_warning_away`: open the warned message, add a card `"Ana Lima" ana.lima.1984@gmail.com`, `keep_senders()` → reading pane's `warning` is `None`; a window on it too.
  - `marks_follow_the_options`: rows from a contact, a named stranger at gmail.com, a colleague at studio.example, the user: default → no `address`; `strangers` → only the stranger's; `always` → all but the user's; `external` only after `view.externalTag {"on": true}` and only for the stranger; `{"domains": "corp.example, studio.example"}` → `domains` both; `{"show": "sometimes"}`, `{"domains": 5}`, 101 domains → `Err(CmdError::Params(_))`.
  - `list_and_read_say_the_marks`: `list.messages` rows carry the three fields; `message.read` carries `warning` and `external`.
  - `the_quiet_gathering_waits_for_the_book` (Review Focus 5): no book loaded → `keep_senders_quietly()` leaves `senders` `None`; with a folder queued (`contacts_pending()`), a changed stamp isn't gathered; once settled it is.
  - `gathering_is_quick`: 20 000 people written to and 30 000 directory people; three times: add a card (stamp changes), time `keep_senders()`; the best < 500 ms (debug build; the spec's 50 ms is for release, measured in Task 6).
- [ ] **Step 2: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-engine safety::senders` — expect compile failure.
- [ ] **Step 3: Implement** the stamp (accounts' email, user name and name; `pim_revision`; `contacts_revision()`; `prefs.org_domains`), the gathering (accounts, every account's cards under `display()`, `written()`, the people index's entries with an address; domains: accounts' plus `written_domains`; org: listed else `own_domains`), `sender_marks` and `is_external` as upstream's, the prefs, commands and CLI lines; `opening.rs`'s four `sender_warning(&p.from)` calls become `message_warning(key, parsed)`; `writing.rs` keeps using `known_domains()`.
- [ ] **Step 4: Run** `cargo test -p quatrelle-engine` (with `every_command_survives_junk_params`) and `cargo build -p quatrelle-cli` — expect PASS.
- [ ] **Step 5: Commit** — "Sender check: who is known, gathered once per change of the address book, the directory, contacts or accounts".

### Task 4: Engine — the directory asked on opening, the signature warning

**Model:** opus, effort high.

**Files:**
- Modify: `crates/engine/src/session/opening/senders.rs`, `crates/engine/src/session/mailboxes/directory.rs`, `crates/engine/src/tests/safety/senders.rs`

**Interfaces:**
- Consumes: Task 2's `lookup_name`, `check_signature`; Task 3's `message_warning`, `keep_senders_quietly`; `directory_account(Some(&key.account))`, `search_directory`.
- Produces: `Directory::waiting` entries become `(String, Asked)` with `pub(crate) enum Asked { Checked, Typed, Sender }` (`search_directory`'s `typed: bool` maps to `Typed`/`Checked`, unchanged for callers); `pub(crate) fn ask_directory_for_sender(&mut self, account: &str, name: &str)`: a waiting `Sender` entry is replaced, `Checked`/`Typed` ones are never displaced. `message_warning` returns the header warning, else `check_signature(&parsed.from, &parsed.text, listed, org)`; with neither and the spec's conditions met, it asks the directory with `lookup_name`. `apply_directory` ends with `keep_senders_quietly()`.

- [ ] **Step 1: Write the failing tests** (an Exchange account `zoe@studio.example`, as `tests/writing/names.rs` sets one up):
  - `an_unknown_persons_name_is_asked_once`: open `"Dr. Dana Whitfield, CEO" <dw.ceo@gmail.com>` → exactly one `Job::SearchDirectory { query: "dana whitfield", .. }`; open it again → none more.
  - `no_question_when_known_or_no_person_or_bulk_or_no_exchange`: a known sender, `"IT Support"`, `"Dana"`, a `List-Id` message from `news@corp-news.example`, an IMAP-only session → no `SearchDirectory`.
  - `the_answer_brings_the_warning`: then `Event::DirectoryPeople { query: "dana whitfield", people: [Dana Whitfield dana@corp.example], .. }` → reading pane `NameOfKnownPerson { name: "Dana Whitfield", known: "dana@corp.example", real: "dw.ceo@gmail.com" }`; with `error: Some("offline")` instead → `None`.
  - `strangers_in_a_row_ask_for_the_last` (Review Focus 2): a Check Names question waiting, then 20 strangers opened → `directory_searching` true for the 20th name and for the Check Names one, false for the 2nd to 19th.
  - `the_signature_warns_in_the_pane_and_a_window`: `"Direction" <dg.studio@gmail.com>` signed `www.studio.example` → `SignatureShowsOther { shown: "studio.example", .. }` in the pane and in a window.
  - `an_outlook_reply_quoting_the_user_doesnt_warn` (Review Focus 1): a parsed HTML message from `"Bo Chen" <bo@supplier.example>`: `<p>Thanks, done.</p><p>Bo</p><div style="border:none;border-top:solid #E1E1E1 1.0pt"><p><b>De :</b> Zoe Park &lt;zoe@studio.example&gt;<br><b>Envoyé :</b> lundi 3 novembre 2026 10:00<br><b>À :</b> Bo Chen</p></div><p>Hello Bo</p><p>Zoe Park<br>www.studio.example<br>zoe@studio.example</p>` → `None`; the same with `<hr>` in place of the `div`'s border, and Gmail's `Le lun. 3 nov. 2026 à 10:00, Zoe Park <zoe@studio.example> a écrit :` + `<blockquote>` → `None`.
- [ ] **Step 2: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-engine safety::senders` — expect the new tests to fail.
- [ ] **Step 3: Implement** as the Interfaces say; Check Names tests (`tests/writing/names.rs`) stay green.
- [ ] **Step 4: Run** `cargo test -p quatrelle-engine` — expect PASS.
- [ ] **Step 5: Commit** — "Sender check: an unknown person's name asked of the directory as the message opens, and the signature compared with the sender".

### Task 5: Interface — banner, list row, Options, per-frame gathering

**Model:** opus, effort high.

**Files:**
- Create: `crates/ui-egui/src/reading/sender_check.rs` (both header lines: `sender_warning` moves there from `reading.rs:501-528`, `pub(crate) use sender_check::sender_warning;` in `reading.rs` so `reading/header.rs:150` and `dialogs/other.rs:97` don't change; the new `external_tag`)
- Modify: `crates/ui-egui/src/reading/header.rs` (the tag beside the sender's name), `crates/ui-egui/src/panes/list.rs` (row), `crates/ui-egui/src/panes/backstage/options.rs` (Safety group, lines 377-407), `crates/ui-egui/src/app/frame.rs` (`self.session.keep_senders_quietly()` once per frame, after the address book step near line 192), the 13 catalogs
- Create: `crates/ui-egui/src/tests/reading/senders.rs`; Modify: `crates/ui-egui/src/tests/reading.rs` (`mod senders;` only)
- Reference: `git show 3a87dfa4 80fb57d9 06a623db -- crates/ui-egui` (upstream's row, tag and options; the fork's look wins).

**Interfaces:**
- Consumes: Task 3's `SenderMarks`, `sender_marks`, `is_external`, `organisation`, `SenderAddresses`, commands; Task 2's `SignatureShowsOther`.

- [ ] **Step 1: Write the failing tests** (headless frames, as `tests/reading.rs` builds them):
  - `each_banner_says_its_case`: the four `SenderWarning` variants show their sentence in the reading pane.
  - `a_warned_row_has_its_sign`: a row whose marks say `warning` has the accessible text « Vérifiez qui l'a envoyé » / "Check who sent it"; an unwarned row hasn't.
  - `the_address_is_cut_to_its_domain_and_the_name_keeps_its_share`: upstream `06a623db`'s test, adapted: in All Inboxes with External on, the name gets at least 9/20 of the line, the address reads `<…@example-billing.com>`.
  - `external_stays_on_one_line_in_a_narrow_pane`: upstream `80fb57d9`'s test, adapted.
  - `the_safety_rows`: Options shows the three rows' labels (the domains row only while the tag is on); choosing « Personnes que vous ne connaissez pas » dispatches `view.senderAddresses {"show": "strangers"}`.
- [ ] **Step 2: Run** `TMPDIR=$HOME/.tq cargo test -p quatrelle-ui-egui tests::reading::senders` — expect failure.
- [ ] **Step 3: Implement.** Row: ⚠ (`Icon::Warning`, `t.danger`, 15 px, name moved 19 px) before the name; with `address`, the dimmer address (12.5 px) after the name, cut to `<…@domain>` when it doesn't fit, the name keeping at least 9/20 of the room; External: framed `tl!("External")` before the time; the row's accessible words gain "Check who sent it" and "External". Options rows and texts as the spec (upstream's catalog rows exist for most). French as the spec.
- [ ] **Step 4: Run** `cargo test -p quatrelle-ui-egui` (catalog tests included) — expect PASS.
- [ ] **Step 5: Commit** — "Sender check: the warning in the list, the sender's address and the External tag as options, in File › Options › Safety".

### Task 6 (controller, not a subagent): checks, documents, review, test build

- [ ] `cargo xtask ci`; `cargo test --release -p quatrelle-engine gathering_is_quick -- --nocapture`: note the time against the spec's 50 ms (slices if over).
- [ ] The real window (`cargo run -p quatrelle -- --sample`, `QUATRELLE_SCREENSHOT`): Options › Safety, the list with addresses on « Personnes que vous ne connaissez pas » and External on; read the PNGs.
- [ ] `FORK.md`: what was taken (picks), rewritten (engine), added (directory question, signature rule); the survey's lot 5 marked done.
- [ ] Whole-branch review on Fable, effort high; one fix wave; merge into main; test build; memory and carnet.
