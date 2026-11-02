# Exchange text first: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Opening a big Exchange message that isn't on this computer shows its text at once; the whole message follows, and ends up in the cache as if downloaded normally.

**Architecture:** The engine asks for `Job::FetchTextFirst` when a message qualifies. The main connection fetches the body and the attachment list in one `GetItem`, builds a MIME message with empty attachment parts, emits `Event::TextFirst`, and hands the whole download to a thread of its own with a copy of the connection. That thread writes the source only and emits the usual `Event::Body`.

**Tech Stack:** Rust, EWS over SOAP, Microsoft Graph, `mail-builder`/`mail-parser`, egui. Fakes: `crates/net/src/fakes/fake_ews*`, `fake_graph*`.

**Spec:** `docs/superpowers/specs/2026-11-02-exchange-text-first-design.md`. Read it first: it holds the reasons, and this plan doesn't repeat them.

**Upstream's code to adapt:** `git show 59515345` and `git show 42c45848` on the `zidan` remote (`https://github.com/ZIDAN44/mailcraft.git`). Adapted by hand, never applied whole: upstream's names are `mailcraft_*`, ours `quatrelle_*`, and the worker part is replaced.

## Global Constraints

- Branch `zidan-v0.31`, in the main checkout. Agents run one at a time: never two compiling at once.
- Focused tests: `CARGO_BUILD_JOBS=2 cargo test -p <crate> <filter>`.
- Before every commit: `CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=4 cargo xtask ci`. If it fails, don't commit.
- No `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!` or `unsafe` outside tests. Everything from the server is hostile: `get()`, checked arithmetic, char-boundary slicing, caps.
- At most 1000 lines per Rust file, 16 Rust files per folder. `crates/engine/src/tests` and `crates/ui-egui/src/tests` are full: put tests in existing files or their subfolders, declared by the existing parent.
- Portable crates (`model`, `mime`, `store`, `jmap`, `engine`, `ui-egui`) stay wasm-clean: no clock, no temp files. Nothing in `engine` depends on `net`.
- No GPL code. No Outlook wording.
- Interface text through `tl!`/`tf!`, in all 13 catalogs of `crates/ui-egui/src/i18n/` (`ar de es fr he it ja ko nl pt-br ru zh-hans zh-hant`). Each is a real translation, not a copy of the English.
- Never test against a real account. Fake servers only.
- Commit messages follow the repo's style (`Exchange: a message's text first…`) and end with:
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`

Exact values, fixed by the spec:

- `TEXT_FIRST_OVER: u32 = 1 << 20`, in `quatrelle_model`, because the engine can't see `net`.
- At most 500 attachments listed.
- `STILL_COMING = "its attachments are still downloading: a moment"`, in `quatrelle_engine`.
- Strings: `Downloading the attachments…` and `The attachments couldn't be downloaded: {error}`.
- Fields: `HEADER_FIELDS` plus `item:Body`, `item:Attachments`, `item:ItemClass`, `item:Importance`. No `BodyType`. No `message:InReplyTo`, `item:InReplyTo` or `message:References`.

## Review Focus

Likely to bite and not in the spec's test list. Each has a test in the task named.

1. **Arrowing down through many big messages:** every text comes, each key downloads once, the last opened goes first, and the main connection doesn't stall (Task 4).
2. **The whole message arrives while something else is open:** the open message isn't disturbed, and the arrived one is still indexed (Task 6).
3. **Odd attachment lists:** no name, size missing or absurd, more than 500, an `ItemAttachment` without a type, a `ReferenceAttachment` (Tasks 1 and 3).
4. **Reopening a message while it downloads:** its text comes again, and no second download starts (Tasks 4 and 6).
5. **The account stops mid-download, or the message is deleted on the server:** no spinner left for ever, no file written back (Task 4).

---

### Task 1: `mime`: a message from its text and a list of files

**Suggested model:** Sonnet, medium.

**Files:**
- Create: `crates/mime/src/attachments/text_first.rs`
- Modify: `crates/mime/src/attachments.rs` (`mod text_first;`, `pub use`, `shown_size`, used for `AttachmentInfo.size` in `attached()`)
- Modify: `crates/mime/src/lib.rs` (re-exports)
- Modify: `crates/mime/Cargo.toml` (`proptest = "1"` in dev-dependencies, as in `crates/model`)

**Interfaces:**
- Produces, re-exported from `quatrelle_mime`:
  - `pub struct PendingFile { pub name: String, pub mime: String, pub size: u64, pub content_id: Option<String>, pub inline: bool }` (`Clone, Debug, Default, PartialEq, Eq`)
  - `pub fn text_first(d: &Draft, from: &EmailAddress, date: i64, message_id: &str, files: &[PendingFile]) -> Result<Vec<u8>, String>`
  - `pub fn is_protected_file(mime: &str, name: &str) -> bool`: `mime` starts, ignoring case, with `application/pkcs7-mime`, `application/x-pkcs7-mime` or `multipart/signed`, or `name` is `smime.p7m` ignoring case.
  - `pub const MOST_PENDING_FILES: usize = 500;`

Adapt upstream's `crates/mime/src/attachments/text_first.rs` (commit `59515345`) with these differences:

- No `TEXT_FIRST` header, no `is_text_first`, no `In-Reply-To` handling.
- An address with neither name nor email is dropped from the lists. One with a name alone is kept: the engine reads an empty `email` as "address not known yet".
- A media type is kept only if it has one `/`, is under 128 bytes and holds no control character, `;` or space. Otherwise `application/octet-stream`.

- [ ] **Step 1: Write the failing tests** in `attachments/text_first.rs`

  Port upstream's `its_text_now_its_files_named_and_sized`, without `is_text_first`. Then add:

  ```rust
  #[test]
  fn a_name_alone_stays_a_name() {
      let d = Draft { to: vec![EmailAddress::new("Alex Martin", "")], subject: "s".into(), body: "b".into(), ..Default::default() };
      let raw = text_first(&d, &EmailAddress::new("Sam", "sam@x.example"), 1, "", &[]).unwrap();
      let p = parse(&raw).unwrap();
      assert_eq!((p.to[0].name.as_str(), p.to[0].email.as_str()), ("Alex Martin", ""));
  }

  #[test]
  fn files_without_a_name_or_a_size_and_too_many() {
      let many = vec![PendingFile { mime: "application/pdf".into(), ..Default::default() }; MOST_PENDING_FILES + 20];
      let raw = text_first(&Draft::default(), &EmailAddress::default(), 0, "", &many).unwrap();
      assert_eq!(parse(&raw).unwrap().attachments.len(), MOST_PENDING_FILES);
  }

  #[test]
  fn what_says_it_is_signed_or_encrypted() {
      assert!(is_protected_file("application/pkcs7-mime; smime-type=enveloped-data", "x.bin"));
      assert!(is_protected_file("Multipart/Signed", ""));
      assert!(is_protected_file("application/octet-stream", "SMIME.P7M"));
      assert!(!is_protected_file("application/pdf", "report.pdf"));
  }

  proptest! {
      /// Nothing the server says adds a header: every header name is one the builder writes.
      #[test]
      fn hostile_text_adds_no_header(subject in ".{0,80}", name in ".{0,80}", kind in ".{0,40}", cid in ".{0,40}", who in ".{0,40}") {
          let to = vec![EmailAddress::new(format!("{who}\r\nEvil: 1"), "a@b.example")];
          let d = Draft { to, subject: format!("{subject}\r\nEvil: 1"), body: "x".into(), ..Default::default() };
          let file = PendingFile { name: format!("{name}\r\nEvil: 1"), mime: format!("{kind}\r\nEvil: 1"), size: 1, content_id: Some(format!("{cid}\r\nEvil: 1")), inline: true };
          let raw = text_first(&d, &EmailAddress::default(), 0, "", &[file]).unwrap();
          let text = String::from_utf8_lossy(&raw);
          prop_assert!(!text.contains("\nEvil:"));
      }
  }
  ```

- [ ] **Step 2: Run them:** `CARGO_BUILD_JOBS=2 cargo test -p quatrelle-mime text_first`. Expected: doesn't compile.
- [ ] **Step 3: Implement** the file, `shown_size`, and the re-exports.
- [ ] **Step 4: Run them again.** Expected: all pass.
- [ ] **Step 5: Commit** after `cargo xtask ci`: `Mail: a message from its text and a list of files to come (from upstream 5951534)`.

---

### Task 2: `model`: the job, the event, and the queue's rules

**Suggested model:** Sonnet, medium.

**Files:**
- Modify: `crates/model/src/job.rs` (variant, `account()`, `TEXT_FIRST_OVER`)
- Modify: `crates/model/src/job/events.rs` (variant)
- Modify: `crates/net/src/worker.rs` (`push_job`, `between_folders`, `jumps_queue`, `log_event`)
- Modify: `crates/net/src/worker/changes.rs` (arm: `Job::FetchTextFirst { key } => self.fetch(key)` for now)
- Modify: `crates/net/src/worker/pool.rs` (offline arm, l.388: an or-pattern with `Job::FetchBody`)
- Modify: `apps/quatrelle-web/src/backend.rs` (l.398 and l.822: handled as `Job::FetchBody`)
- Modify: `crates/engine/src/session/events.rs` (arm for `Event::TextFirst`: ignored for now, Task 6 fills it)
- Test: `crates/net/src/worker/tests.rs`

**Interfaces:**
- Produces:
  - `Job::FetchTextFirst { key: MessageKey }`
  - `Event::TextFirst { key: MessageKey, raw: Vec<u8> }`
  - `pub const TEXT_FIRST_OVER: u32 = 1 << 20;` in `quatrelle_model`

`log_event` logs `Event::TextFirst` with the key and the byte count, never the content. `Job::ReadOnline` and `Job::FetchTextFirst` both join `between_folders` and `jumps_queue`. In `push_job`, a `Job::FetchTextFirst` equal to one waiting isn't queued again.

- [ ] **Step 1: Write the failing tests** in `worker/tests.rs`

  ```rust
  #[test]
  fn what_someone_waits_on_jumps_the_queue() {
      let key = MessageKey::default();
      for j in [Job::FetchBody { key: key.clone() }, Job::FetchTextFirst { key: key.clone() }, Job::ReadOnline { key: key.clone() }] {
          assert!(jumps_queue(&j) && between_folders(&j), "{j:?}");
      }
  }

  #[test]
  fn a_text_asked_for_twice_is_asked_once() {
      let mut q = VecDeque::new();
      let j = Job::FetchTextFirst { key: MessageKey::default() };
      push_job(&mut q, j.clone());
      push_job(&mut q, j);
      assert_eq!(q.len(), 1);
  }
  ```

- [ ] **Step 2: Run them:** `CARGO_BUILD_JOBS=2 cargo test -p quatrelle-net worker::tests`. Expected: doesn't compile.
- [ ] **Step 3: Add the variants and the arms.** The compiler lists every match. No `_ =>` that swallows the new job silently.
- [ ] **Step 4: Run them again.** Expected: pass. Also `cargo xtask wasm`.
- [ ] **Step 5: Commit** after `cargo xtask ci`: `Worker: old mail opened during a sync goes ahead of it, and a job for a message's text first`.

---

### Task 3: EWS: the text from one `GetItem`

**Suggested model:** Opus, high. Exchange specifics and hostile input.

**Files:**
- Create: `crates/net/src/exchange/ews/text_first.rs`
- Modify: `crates/net/src/exchange/ews.rs` (`mod text_first;`, `pub use text_first::{fetch_text, text_first};`)
- Modify: `crates/net/src/fakes/fake_ews.rs` (`item_xml`, `It.class`, `MICROSOFT_FIELDS`, `refused_text`)
- Modify: `crates/net/src/fakes/fake_ews/server.rs` (the field check in the `GetItem` block, l.287)
- Modify: `crates/net/src/fakes/fake_ews/route.rs` (pass what's asked to `item_xml`)
- Test: `crates/net/src/exchange/ews/tests.rs`, `crates/net/src/fakes/fake_ews/tests/mailbox.rs`

**Interfaces:**
- Consumes: Task 1's `quatrelle_mime::{text_first, PendingFile, is_protected_file, Draft}`.
- Produces:
  - `pub fn text_first(m: &El) -> Option<Vec<u8>>`: the built message, or `None` (no body, an empty body, no attachment, S/MIME, invitation). `m` is the response message element, as `call_one` returns it.
  - `impl Client { pub fn download_text(&self, id: &str) -> Result<Option<Vec<u8>>> }`
  - `pub fn fetch_text(c: &Client, st: &mut State, store: &Store, key: &MessageKey) -> Result<Option<(Vec<u8>, String)>>`: the built message and the item id. `None` if the store has the body or the key's `validity != VALIDITY`. The id comes from `server_id`.
  - On the fake: `Data.refuse_text: bool`, which refuses a `GetItem` naming `item:Body` with the same SOAP fault as `refuse_fields`.

How `text_first` reads the item (the first child of `Items`, whatever its element name):

- `ItemClass`, ignoring case, starting with `IPM.Note.SMIME` or `IPM.Schedule.Meeting`: `None`. Any attachment for which `is_protected_file` holds: `None`.
- `From`: `address()` as `header_block` does, including the `0x5D01` substitution. Share this rather than copy it: split a `fn sender(it: &El) -> Option<String>` out of `header_block`.
- To, Cc, Reply-To: `address()`, then split into `EmailAddress`. `"Name" <>` gives a name with an empty email.
- `ItemAttachment` gives `message/rfc822`. `ReferenceAttachment` and any other element under `Attachments` give `application/octet-stream`. A missing or unparsable `Size` gives 0.
- `inline` needs `IsInline` true and a non-empty `ContentId`.
- `Importance`: `High` 1, `Low` -1. Date: `DateTimeSent`, else `DateTimeReceived`.
- HTML unless `BodyType` is `Text`, ignoring case.

`MICROSOFT_FIELDS` is the fake's list of valid `item:` and `message:` names, from Microsoft's FieldURI page:

```text
item:ArchiveTag item:Attachments item:Body item:Categories item:ConversationId item:Culture item:DateTimeCreated
item:DateTimeReceived item:DateTimeSent item:DisplayCc item:DisplayTo item:EffectiveRights item:Flag item:HasAttachments
item:Importance item:InReplyTo item:InstanceKey item:InternetMessageHeaders item:IsAssociated item:IsDraft item:IsFromMe
item:IsResend item:IsSubmitted item:IsUnmodified item:ItemClass item:ItemId item:LastModifiedName item:LastModifiedTime
item:MimeContent item:NormalizedBody item:ParentFolderId item:PolicyTag item:Preview item:ReminderDueBy item:ReminderIsSet
item:ReminderMinutesBeforeStart item:ResponseObjects item:RetentionDate item:Sensitivity item:Size item:StoreEntryId
item:Subject item:TextBody item:UniqueBody
message:ApprovalRequestData message:BccRecipients message:CcRecipients message:ConversationIndex message:ConversationTopic
message:From message:InternetMessageId message:IsDeliveryReceiptRequested message:IsRead message:IsReadReceiptRequested
message:IsResponseRequested message:ReceivedBy message:ReceivedRepresenting message:References message:ReminderMessageData
message:ReplyTo message:Sender message:ToRecipients message:VotingInformation
```

Any `GetItem` naming an `item:` or `message:` field outside the list gets the `ErrorSchemaValidation` fault of `refuse_fields`. Other prefixes aren't checked. Before adding this, `grep -rn 'FieldURI=\\"\(item\|message\):' crates/net/src`: every name the code sends must be in the list.

`item_xml` gains an argument saying what was asked. With `item:Body` asked for, it adds `<t:Body BodyType="HTML|Text">`, `<t:Attachments>` from `mime::parse(&it.raw)`, `<t:ItemClass>` from `It.class` (default `IPM.Note`) and `<t:Importance>`. `<t:HasAttachments>` tells the truth in every case; check that no existing test leaned on the hard-coded `false`.

- [ ] **Step 1: Write the failing tests**

  In `ews/tests.rs`, `an_item_read_as_exchange_gives_it`: an `El` parsed from a realistic `GetItem` answer, with:

  ```text
  <t:ItemClass>IPM.Note</t:ItemClass>
  <t:From><t:Mailbox><t:Name>Sam Lee</t:Name><t:EmailAddress>/O=CONTOSO/OU=EXCHANGE/CN=RECIPIENTS/CN=SAM</t:EmailAddress><t:RoutingType>EX</t:RoutingType></t:Mailbox></t:From>
  <t:ExtendedProperty><t:ExtendedFieldURI PropertyTag="0x5D01" PropertyType="String"/><t:Value>sam@contoso.example</t:Value></t:ExtendedProperty>
  ToRecipients: one SMTP, one EX ("Alex Martin")
  Attachments: FileAttachment (Size 20971586); FileAttachment (IsInline true, ContentId still@x, image/png); ItemAttachment (Size 4096);
               ReferenceAttachment (no Size); FileAttachment with no Name
  ```

  Assert on `mime::parse(&text_first(&el).unwrap()).unwrap()`:
  - `from.email == "sam@contoso.example"`, `from.name == "Sam Lee"`;
  - `to` has the SMTP one, then Alex Martin with `email == ""`;
  - the attachments' `(mime, size, inline)`: `20_971_586` first, `(image/png, inline)`, `message/rfc822`, `application/octet-stream` with size 0;
  - the text is there.

  In the same file, `what_is_left_whole`: `None` for `IPM.Note.SMIME.MultipartSigned`, `IPM.Schedule.Meeting.Request`, a `smime.p7m` attachment under a plain `IPM.Note`, an empty `Body`, and no attachment.

  In `fake_ews/tests/mailbox.rs`:
  - `exchange_refuses_a_field_it_does_not_know`: a `GetItem` with `message:InReplyTo` through `Client::call_one` gives an error; the same request with `item:InReplyTo` doesn't;
  - `a_message_text_from_the_server`: deliver a message with an attachment and call `Client::download_text(id)` directly. The attachment is listed empty, with its size, and `item_shapes` shows no request with `MimeContent`.

- [ ] **Step 2: Run them:** `CARGO_BUILD_JOBS=2 cargo test -p quatrelle-net text`. Expected: doesn't compile.
- [ ] **Step 3: Implement** the fake's changes, then `ews/text_first.rs`.
- [ ] **Step 4: Run them again,** then the whole fake EWS suite: `CARGO_BUILD_JOBS=2 cargo test -p quatrelle-net fake_ews`. Expected: all pass.
- [ ] **Step 5: Commit** after `cargo xtask ci`: `Exchange: a message's text and the list of its files, from fields the server knows (from upstream 42c4584)`.

---

### Task 4: The worker: text on the main connection, the rest on its own thread

**Suggested model:** Opus, high. This is the core of the lot: threads, stopping, ordering.

**Files:**
- Create: `crates/net/src/worker/rest.rs`
- Modify: `crates/net/src/worker.rs` (`mod rest;`, `Worker.rest: rest::Rest`, `Worker.text_refused: bool`, built in `with_numbering`)
- Modify: `crates/net/src/worker/changes.rs` (the arm from Task 2 calls `self.fetch_text_first(key)`)
- Test: new tests in `crates/net/src/fakes/fake_ews/tests/mailbox.rs`; if it passes 800 lines, a new `fake_ews/tests/text_first.rs`, declared in `fake_ews/tests.rs`

**Interfaces:**
- Consumes: Task 2's job and event, Task 3's `crate::ews::fetch_text`.
- Produces, in `worker/rest.rs`:

  ```rust
  /// What a whole message comes over: a connection of its own, copied when it's asked for.
  pub(super) enum Whole { Ews(crate::ews::Client), Graph { client: crate::graph::Client, owner: Option<String> } }

  struct Waiting { key: MessageKey, id: String, how: Whole }

  /// Whole messages to bring, one at a time, the last asked for first.
  pub(super) struct Rest { /* Arc<Mutex<Queue>>, Arc<AtomicBool>, Store, Sink */ }

  impl Rest {
      pub(super) fn new(store: Store, sink: Sink) -> Rest;
      /// `false`: the thread can't start. The caller brings the message itself.
      pub(super) fn add(&self, key: MessageKey, id: String, how: Whole) -> bool;
  }
  impl Drop for Rest { /* flag set; each key waiting gets Event::BodyFailed */ }

  impl Worker { pub(super) fn fetch_text_first(&mut self, key: MessageKey); }
  ```

  Graph's arm is added in Task 5: here `Whole` has only `Ews`.

Decisions the code must keep:

- `Rest` ends with the `Worker` through `Drop`, so every exit path of `run` is covered. That covers quitting, working offline, an account removed.
- `add`: running key → `true`, nothing changes. Waiting key → moved to the front, with the new `Whole`. New key → front. Thread down → spawn, named `whole-{account}`, with `std::thread::Builder`. On a spawn failure the entry is taken back out and `add` returns `false`.
- The thread, on each entry: flag set → `Event::BodyFailed { error: "the account stopped" }` and go on draining. `store.has_body(&key)` → skip silently. Else download; flag set → `BodyFailed`, nothing written. `write_body` then `Event::Body`. Any error, `write_body`'s included → `BodyFailed`. `running` and `up` are cleared under the same lock that finds the queue empty, so an `add` can't fall between them.
- A poisoned lock: treated as an empty queue, no panic.
- The thread never calls `self.failed`, never touches the list, never emits `FolderChanged`, never holds a `Sender<Job>`.
- `fetch_text_first`, in order:
  1. `store.read_body` hit → `Event::Body`.
  2. `self.text_refused`, or not `Conn::Ews`/`Conn::Graph` after connecting → `self.fetch(key)`.
  3. `ews::fetch_text`. `Ok(None)` → log at info, `self.fetch(key)`.
  4. `Err(NetError::No(e))` with `e` containing one of `ews_pim::FIELD_REFUSED` (make that constant `pub(crate)`): the shape was refused. `text_refused = true`, one `log::warn!`, `self.fetch(key)`. Any other `NetError::No`, such as a message gone from the server, isn't this case.
  5. Any other `Err` → log, `self.fetch(key)`. `fetch` reports it as usual.
  6. Save the numbering if it changed, as `fetch` does.
  7. `Event::TextFirst`, then `rest.add(...)`; on `false`, `self.fetch(key)`.

Log lines, English, one each: `"{account}: a message's text came first: {bytes} bytes in {ms} ms"` and `"{account}: the rest of it came in {secs:.1} s ({bytes} bytes)"`.

- [ ] **Step 1: Write the failing tests**, on the fake EWS server through the `Pool`, with `slow_sources` set. Each ends with `drop(pool)` and removes its store.

  - `the_text_comes_before_the_whole_message`: `slow_sources` 2 s. `Event::TextFirst` for the key arrives while `!store.has_body(&key)`; then `Event::Body`, and `store.has_body(&key)`. The built text wasn't written: `store.read_body` gives the original source.
  - `another_message_opened_meanwhile_shows_at_once`: two messages, 2 s; submit both; both `Event::TextFirst` within 1 s of the second submit.
  - `the_last_opened_comes_first_and_each_once`: five messages, 1 s; submit all five, then the third again. `Event::Body` order has the third before the two still waiting; each key's `Event::Body` once; count of `GetItem` with `MimeContent` is 5. (Review Focus 1, 4.)
  - `sending_waits_for_nothing`: 3 s; `Job::FetchTextFirst`, then `Job::Send`; `Event::Sent` before the `Event::Body`.
  - `a_public_folder_and_a_shared_mailbox_work`: reuse the set-ups of `public_folders_list_under_their_own_folder_and_sync` and `shared_mailboxes_are_kept_unless_read_online`. A message in each: `Event::TextFirst`, then `Event::Body`, and `routed` shows the mailbox header on the whole download.
  - `a_refused_text_falls_back_and_is_not_asked_again`: `refuse_text`. Two messages: `Event::Body` for both, no `Event::TextFirst`, one `GetItem` naming `item:Body`.
  - `a_failed_download_is_told`: `unreadable` holds the id: `Event::TextFirst`, then `Event::BodyFailed`, and no body stored. (Review Focus 5.)
  - `nothing_is_written_after_the_stop`: 2 s; two keys asked for; after `Event::TextFirst` for both, `Job::WorkOffline { on: true }`, which stops the worker with the pool still there to listen. Each key gets `Event::BodyFailed`, and after 3 s neither has a body in the store. (Review Focus 5.)
  - `the_folder_list_stays_as_it_was`: `store.folder_version` is the same before and after.
  - In `worker/tests.rs`: `Rest` alone, no server, for `add`'s rules with `Whole::Ews` of a client pointing at nothing: same key twice stays one, new keys go first.

- [ ] **Step 2: Run them:** `CARGO_BUILD_JOBS=2 cargo test -p quatrelle-net text_first`. Expected: fail or don't compile.
- [ ] **Step 3: Implement** `worker/rest.rs`.
- [ ] **Step 4: Run them ten times** to catch timing faults: `for i in 1 2 3 4 5 6 7 8 9 10; do CARGO_BUILD_JOBS=2 cargo test -p quatrelle-net text_first -- --test-threads=2 || break; done`. Expected: ten passes.
- [ ] **Step 5: Commit** after `cargo xtask ci`: `Exchange: the text on the main connection, the whole message on a copy of it, never in the way of sending`.

---

### Task 5: Graph

**Suggested model:** Sonnet, high.

**Files:**
- Create: `crates/net/src/exchange/graph/text_first.rs`
- Modify: `crates/net/src/exchange/graph.rs` (`mod text_first;`, re-exports, `Client::sibling`)
- Modify: `crates/net/src/worker/rest.rs` (`Whole::Graph`, and the Graph arm of `fetch_text_first`)
- Modify: `crates/net/src/fakes/fake_graph.rs`, `crates/net/src/fakes/fake_graph/tests.rs`

**Interfaces:**
- Consumes: Task 1, Task 4's `Whole`.
- Produces:
  - `pub fn text_first(v: &serde_json::Value) -> Option<Vec<u8>>`
  - `impl Client { pub fn download_text(&self, id: &str, owner: Option<&str>) -> Result<Option<Vec<u8>>> }`
  - `pub fn fetch_text(c: &Client, st: &State, store: &Store, key: &MessageKey) -> Result<Option<(Vec<u8>, String)>>`, with `None` as Task 3's
  - `impl Client { pub(crate) fn sibling(&self) -> Client }`: `base` and `auth` copied

Adapt upstream's `graph/text_first.rs` with these differences:

- `who(owner)`, not `/me`.
- `address()` keeps a name alone, as on EWS.
- `@odata.type` containing `eventMessage` on the message: `None`. An attachment for which `is_protected_file` holds: `None`.
- `inline: isInline`, `content_id: None`.
- `.take(MOST_PENDING_FILES)`.
- `Whole::Graph` downloads with `client.download(&id, owner.as_deref())`.

Fake: upstream's `$expand=attachments` route (see commit `42c45848`), plus `/users/{owner}/messages/{id}` if the fake routes those, and `hasAttachments` telling the truth.

- [ ] **Step 1: Write the failing tests** in `fake_graph/tests.rs`: upstream's `a_message_text_first_from_graph` adapted to `Event::TextFirst`; `graph_leaves_an_invitation_and_a_signed_message_whole`; a `Value`-only test in the same style as Task 3's for a name-only address and an inline attachment.
- [ ] **Step 2: Run them:** `CARGO_BUILD_JOBS=2 cargo test -p quatrelle-net graph`. Expected: fail.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them again.** Expected: pass.
- [ ] **Step 5: Commit** after `cargo xtask ci`: `Microsoft Graph: a message's text first, its files after (from upstream 42c4584)`.

---

### Task 6: The engine

**Suggested model:** Opus, medium.

**Files:**
- Modify: `crates/engine/src/session.rs` (`Reading.text_first`, `STILL_COMING`)
- Modify: `crates/engine/src/lib.rs` (`pub use` of `STILL_COMING`)
- Modify: `crates/engine/src/session/opening.rs` (`fetch_job`, `text_first_wanted`, `show_text_first`, `show_body`, the two `Reading` literals)
- Modify: `crates/engine/src/session/events.rs` (`Event::TextFirst`, `Event::BodyFailed`)
- Modify: `crates/engine/src/session/writing.rs` (`reply_compose`, `forward_as_attachment`)
- Modify: `crates/engine/src/cmd/compose.rs` (`attachment_bytes`, `message_source`, `message_pdf`, `save_all_attachments`, `preview_attachment`, `message_headers`)
- Test: `crates/engine/src/tests/mail.rs`; if it passes 800 lines, a new file in a subfolder

**Interfaces:**
- Consumes: Task 2's job, event and constant; Task 1's `text_first` to build test messages.
- Produces:
  - `pub text_first: bool` on `Reading`
  - `pub const STILL_COMING: &str = "its attachments are still downloading: a moment";`
  - `pub fn text_first(&self, key: &MessageKey) -> bool`: the reading or a window on that key is in its text phase. The interface uses it.
  - `pub(crate) fn whole_here(&self) -> Result<(), CmdError>`: `Err(CmdError::Failed(STILL_COMING.into()))` when the open reading is in its text phase.

`text_first_wanted(key)` is the spec's "When it applies", read from the account's config and `self.summary(key)`. The Outbox is left out.

`reply_compose`, when `r.text_first`:
- `Forward`: refused.
- Refused too if `parsed.attachments` has an inline one, or `parsed.images` is non-empty and any names `cid:`.
- Refused if the address written to is empty: `reply_to` first, else `from`.
- `ReplyAll`: refused also if any `to` or `cc` has an empty `email`.

`Event::BodyFailed` keeps its behaviour, and also reaches a window with `text_first`. The reading stays in its text phase, so the commands keep refusing and "Try again" shows.

- [ ] **Step 1: Write the failing tests** in `tests/mail.rs`, on the sample session. Helper: `text_first_of(s, key) -> Vec<u8>`, building the message with `mime::text_first` from the sample's parsed source, one pending file.

  - `a_message_text_first_then_whole`: `Event::TextFirst`: `r.text_first`, `r.parsed` has the text. `Event::Body`: `!r.text_first`, the attachment's bytes come.
  - `the_text_never_replaces_a_whole_message`: open it whole, apply `Event::TextFirst`: `!r.text_first`.
  - `the_text_is_not_kept_nor_indexed`: after `Event::TextFirst`, the store's body is unchanged, a search for a word only in that text finds nothing. After `Event::Body`, it's found.
  - `a_window_follows_the_same_way`: `pop_out`, the same sequence, on `s.windows`; `Event::BodyFailed` sets its `error` and leaves `parsed`.
  - `the_whole_message_arriving_leaves_the_open_one_alone`: open A in its text phase, open B, `Event::Body` for A: the reading is still B, unchanged; A is indexed. (Review Focus 2.)
  - `opening_it_again_asks_for_the_text_again`: open, leave, open: a second `Job::FetchTextFirst` queued. (Review Focus 4.)
  - `what_needs_the_whole_message_says_so`: during the text phase each of `mail.forward`, `mail.forwardAsAttachment`, `attachment.save`, `attachment.saveAll`, `attachment.preview`, `message.saveAs`, `message.exportPdf`, `message.headers` gives the error `STILL_COMING`.
  - `a_reply_waits_only_for_what_it_needs`: `mail.reply` works with a plain text phase; refused with an inline file; refused with an empty sender address. `mail.replyAll` refused with a name-only `Cc`.
  - `which_messages_come_text_first`: the job queued for each of: EWS account, `size` 2 MiB with attachments → `FetchTextFirst`; `size` 2 MiB without → `FetchBody`; `size` 100 KB with → `FetchBody`; Graph, `size` 0 with attachments → `FetchTextFirst`; IMAP → `FetchBody`; an older message → `ReadOnline`; "Download all" → `FetchBody`.

- [ ] **Step 2: Run them:** `CARGO_BUILD_JOBS=2 cargo test -p quatrelle-engine text_first`. Expected: fail.
- [ ] **Step 3: Implement.** `show_text_first` shares `parse_for_reading`, the sender warning and `request_pictures` with `show_body`, and fills only where `parsed` is `None`. `show_body` also matches `w.text_first` in its `shown` test and in its window filter, and clears the flag.
- [ ] **Step 4: Run them again,** with `every_command_survives_junk_params`. Expected: pass.
- [ ] **Step 5: Commit** after `cargo xtask ci`: `Engine: a message's text shows at once, and what needs the whole of it says it's coming`.

---

### Task 7: The reading pane and the translations

**Suggested model:** Opus, medium.

**Files:**
- Modify: `crates/ui-egui/src/reading.rs` (the spinner line, the failure line)
- Modify: `crates/ui-egui/src/reading/attachments.rs` (`preview` l.14 and the two like it at l.47 and l.73; the chip's click, l.216)
- Modify: `crates/ui-egui/src/reading/pictures.rs` (`picture_texture`)
- Modify: `crates/ui-egui/src/i18n/*.tsv`, all 13
- Modify: `crates/ui-egui/src/i18n/tests.rs` (`notice_wordings`: `out.insert(quatrelle_engine::STILL_COMING.to_string());`, beside `NAMES_NOT_FOUND`)
- Test: `crates/ui-egui/src/tests/reading.rs`; subfolder `tests/reading/` if it passes 800 lines

**Interfaces:**
- Consumes: Task 6's `Session::text_first(&key)`, `Reading.text_first`, `STILL_COMING`.

Decisions:

- `r.text_first`: the spinner and `Downloading the attachments…` show whenever it's set, even when no chip is listed, for example when every attachment is a picture shown in place. "Save all…" doesn't show.
- `r.error` with `r.parsed` set, shown above the attachments: the warning icon, `The attachments couldn't be downloaded: {error}`, `Try again` (`Action::Open`). The spinner line doesn't show then.
- `picture_texture`: before `sent_with`, `if s.text_first(key) { return Pic::Loading; }` for anything not `http(s)`.
- `preview` and its two neighbours: `if s.text_first(key) { return None; }` first, before the cache, so no absence is kept.
- The chip: a plain click during the text phase sets no `attachment_view`. It pushes the action that gives the notice `STILL_COMING`. Adding an `Action` is allowed if none fits; `attachment_card` has `acts`. `st.attachment_selected` stays as it is.

- [ ] **Step 1: Write the failing tests** in `tests/reading.rs`, in the style of upstream's `a_message_text_first_says_its_files_are_coming`, with `Event::TextFirst`:

  - that test, adapted, including the whole message taking its place;
  - `a_picture_waits_for_the_whole_message`: a text first with an inline image `cid:still@x` in its HTML: a spinner and no `[name]` placeholder; after `Event::Body`, the picture's texture exists. The second half is the one that fails if an absence was cached.
  - `a_failure_keeps_the_text`: `Event::BodyFailed`: the text and `Try again` are in the labels, `Downloading the attachments` isn't.
  - `a_chip_says_it_is_coming`: click the chip: `st.attachment_view` is `None`, the notice is `STILL_COMING`.

- [ ] **Step 2: Run them:** `CARGO_BUILD_JOBS=2 cargo test -p quatrelle-ui-egui text_first`. Expected: fail.
- [ ] **Step 3: Implement,** then add the three strings to the 13 catalogs, keeping each file's sort order.
- [ ] **Step 4: Run them again,** and `CARGO_BUILD_JOBS=2 cargo test -p quatrelle-ui-egui i18n`. Expected: pass.
- [ ] **Step 5: Commit** after `cargo xtask ci`: `Reading: the text at once, a spinner where the files and pictures will be`.

---

### Task 8: Fuzz target and the record

**Suggested model:** Sonnet, medium.

**Files:**
- Create: `fuzz/fuzz_targets/text_first.rs`, `fuzz/seeds/text_first/getitem`
- Modify: `fuzz/Cargo.toml` (`{ name = "text_first", bench = false, test = false, doc = false, path = "fuzz_targets/text_first.rs" }, # Exchange's and Graph's answers, read as a message's text`)
- Modify: `fuzz/README.md`, `docs/sync.md`, `FORK.md`
- Modify: `~/mailcraft/merge-tools/survey-v0.31.md` (lot 2 marked done, with the commit)

**Interfaces:**
- Consumes: `quatrelle_net::ews::{parse, text_first}`, `quatrelle_net::graph::text_first`, `quatrelle_mime::parse`.

The target: the first byte picks EWS or Graph. The rest is `ews::parse` of the text, or `serde_json::from_slice`. Whatever is built goes through `mime::parse`. A hand-written valid input in `fuzz/seeds/text_first/`: the `GetItem` answer of Task 3's test, after the byte that picks EWS. No dictionary.

`FORK.md` follows the lot 1 entry's style: `59515345` and `42c45848` taken, adapted, with the four differences in a line each. `docs/sync.md` gets a short section on the second way a message arrives.

- [ ] **Step 1: Write the target.**
- [ ] **Step 2: Build it:** `cd fuzz && CARGO_BUILD_JOBS=4 cargo +nightly fuzz build text_first`, then run it for two minutes: `cargo +nightly fuzz run text_first -- -max_total_time=120`. Expected: no crash. If `cargo fuzz` or the nightly toolchain isn't there, `cargo +stable check --manifest-path fuzz/Cargo.toml` and say so.
- [ ] **Step 3: Write the docs.**
- [ ] **Step 4: Commit** after `cargo xtask ci`: `FORK.md: Exchange text first, and a fuzz target for what it reads`.

---

## After the tasks

A whole-branch review, Fable at high effort: threads and stopping in `worker/rest.rs`, hostile input in the two builders, and the spec against the code. Then merge into `main`, publish a test build (Linux and web, no approval needed), and check for real, with the owner, a big message and a public folder's message: the log line is the proof, and the look of the reading pane is seen there too, the sample mailbox having no Exchange account. Then look at whether older mail read online really can't open its attachments, as the spec says.
