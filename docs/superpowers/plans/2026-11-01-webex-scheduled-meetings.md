# Webex scheduled meetings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The appointment window's Webex Meeting button creates a real scheduled meeting through Webex's public REST API, keeps its id on the event, and follows the appointment when it changes or goes; the personal room stays as an explicit choice.

**Architecture:** `net` gets a Webex client (`webex.rs`) and a single worker thread that runs its jobs in order. The engine stays sans-I/O: commands push `Job::Webex…`, and `Event::Webex…` comes back into a `WebexState` on the session. `event.create`, `event.update`, `event.move` and `event.delete` follow the meeting from the engine, so a drag in the calendar and the CLI behave as the window does. The window only asks for the creation, reads the answer and deletes what it leaves unsaved.

**Tech Stack:** Rust, egui, the existing `http::send`, `oauth`, `Credentials` and fake-server patterns. No new dependency.

**Spec:** `docs/superpowers/specs/2026-11-01-webex-scheduled-meetings-design.md`

## Global Constraints

- No `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!`, `unimplemented!` or `unsafe` outside tests.
- At most 1000 lines per Rust file (split before ~800) and 16 `.rs` per folder. These are **full**: `crates/model/src/*.rs`, `crates/engine/src/session/*.rs`, `crates/engine/src/tests/*.rs`, `crates/ui-egui/src/tests/*.rs`. New files go where this plan says.
- Interface text through `tl!`/`tf!`, with a real translation in all 13 catalogs `crates/ui-egui/src/i18n/*.tsv`.
- Nothing about any Webex organisation is built in: no client id, no secret, no site. `https://webexapis.com/v1` is the only fixed address.
- The client secret and refresh token live only in the keychain (`Credentials::set_secret`, under `webex::keyring()`). The access token lives only in memory. A meeting's password goes only into `Secret` and the notes: never into a log.
- The default path never builds a `/meet/` link, and no API error ever falls back to the personal room.
- Everything Webex answers is hostile: capped, cleaned of control characters, its id checked by `valid_webex_id`.
- Every cargo command starts with `mkdir -p ~/.tq`, then `TMPDIR=$HOME/.tq CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=4 CARGO_TARGET_DIR=$PWD/target`. One heavy cargo at a time (`pgrep -a cargo` first). Stable clippy uses `CARGO_TARGET_DIR=$PWD/target-stable`.
- Every commit ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Work on branch `webex-meetings` in the main checkout. Never push.
- Each task's gate before its commit: `cargo fmt --all`, the tests it names, and `cargo xtask ci`.

## Review Focus

1. **A stranger's invitation carrying `X-QUATRELLE-WEBEX-ID`:** deleting or changing it sends nothing to Webex. Test: `a_strangers_webex_id_is_never_acted_on` (Task 6).
2. **"Delete this occurrence", or dragging one occurrence, of a recurring meeting:** the series' Webex meeting stays as it was. Test: `one_occurrence_leaves_the_series_meeting_alone` (Task 6).
3. **Save pressed, by mouse or keyboard, while the meeting is being created:** held until the answer. Closing meanwhile deletes the meeting when it arrives. Tests: `save_waits_for_the_webex_answer` (Task 8), `a_meeting_answered_after_its_window_closed_is_deleted` (Task 5).
4. **The title or time changed between the click and the first save:** the first save sends `PATCH`. A save that changes nothing sends nothing. Tests: `the_first_save_sends_what_changed_since_the_click`, `a_save_that_changes_nothing_sends_nothing` (Task 6).
5. **An Exchange appointment whose zone is a Windows name (`Romance Standard Time`):** Webex gets `Europe/Paris`, and a zone with no IANA name gets `UTC` with the same instants. Test: `fields_take_the_zone_by_its_iana_name` (Task 2).

---

### Task 1: No calendar search at Google for a password account

Done before this plan ran, on `main` as b35f191f, for the owner's test build. Nothing to do here.

---

### Task 2: The Webex id on the event, and the shared types

**Files:**
- Modify: `crates/model/src/ical.rs` (`CalEvent`, `write`), `crates/model/src/ical/meeting.rs`, `crates/model/src/ical/tests.rs`, `crates/model/src/job.rs`
- Create: `crates/model/src/job/webex.rs`

**Interfaces:**
- Produces, exported as `Event` is:
  - `CalEvent.webex_id: Option<String>` (`#[serde(default, skip_serializing_if = "Option::is_none")]`)
  - `pub fn valid_webex_id(id: &str) -> bool`: 1 to 200 characters of ASCII letters, digits, `_` and `-`
  - `pub const WEBEX_REDIRECT_PORT: u16 = 53683;`, `pub const WEBEX_REDIRECT_URI: &str = "http://127.0.0.1:53683/";`
  - `pub struct WebexFields { pub title: String, pub start: i64, pub end: i64, pub timezone: String, pub recurrence: Option<String> }` with `pub fn of(e: &CalEvent) -> WebexFields`
  - `pub struct WebexMeeting { pub id: String, pub web_link: String, pub meeting_number: String, pub password: Secret, pub sip_address: String }`
  - `pub enum WebexError { SignIn, Forbidden, Refused { status: u16, message: String }, Failed(String) }`, with `Display` in English: "Sign in to Webex again", "Your Webex account can't schedule meetings on this site", "Webex answered {status}: {message}", the text itself

All three types derive `Clone, Debug, PartialEq, Eq, Serialize, Deserialize`, as `Event`'s contents do; `WebexFields` derives `Default` too.

- [ ] **Step 1: Write the failing tests**

In `ical/tests.rs`:
- `webex_id_round_trips`: an event with `webex_id: Some("abc_20261102T090000Z".into())` written with `ical::write(&[e], None, now)` holds `X-QUATRELLE-WEBEX-ID:abc_20261102T090000Z`, and reads back with the same id and an empty `extra`.
- `an_invitation_leaves_the_webex_id_out`: `ical::write(&[e], Some("REQUEST"), now)` and `Some("CANCEL")` don't contain `X-QUATRELLE-WEBEX-ID`.
- `a_junk_webex_id_is_not_read`: values `a/../b`, 201 `x`, and the empty value leave `webex_id` `None`.

In `job/webex.rs`, `fields_take_the_zone_by_its_iana_name`: `tzid` `Romance Standard Time` gives `Europe/Paris`; `Europe/Berlin` stays; `` and `+0530` give `UTC`. `title`, `start`, `end` and `recurrence` come from `summary`, `start`, `end`, `rrule`.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p quatrelle-model webex`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

- `ical/meeting.rs`: `read` takes `X-QUATRELLE-WEBEX-ID` through `valid_webex_id`; `lines` writes it. `valid_webex_id` lives here.
- `ical::write`: when `method` is `Some`, the events are written with `webex_id` `None`, on clones.
- `WebexFields::of`: the zone is `Zone::named(&e.tzid).and_then(|z| z.name())`, else `"UTC"`.

- [ ] **Step 4: Run the tests and `cargo xtask ci`**

Expected: PASS.

- [ ] **Step 5: Commit**

`git commit -m "Calendar: an event keeps the Webex meeting made for it"`

---

### Task 3: The id through Exchange and Graph

**Files:**
- Modify: `crates/net/src/exchange/ews_pim.rs` (the shape events are read with), `crates/net/src/exchange/ews_pim/events.rs`, `crates/net/src/exchange/ews_pim/push.rs`, `crates/net/src/exchange/ews_pim/tests.rs`
- Modify: `crates/net/src/exchange/graph_pim.rs`
- Modify: `crates/net/src/fakes/fake_ews/` and `crates/net/src/fakes/fake_graph/` where they hold appointments

**Interfaces:**
- Consumes: `CalEvent.webex_id`, `valid_webex_id` (Task 2).
- Produces: `const WEBEX_SET: &str = "5b7a1c3e-8f44-4f0e-9c1d-3e2a7d6b4c11";`, Quatrelle's own property set, in `exchange/ews_pim/events.rs` and used by Graph. The property is named `WebexId`, type String.

- [ ] **Step 1: Write the failing tests**

- `ews_pim/tests.rs`, `an_appointments_webex_id_round_trips`: create an event with `webex_id` on the fake EWS through the sync; read it back with the same id. Change its `summary`, sync: the id stays. An id the fake holds that fails `valid_webex_id` reads as `None`.
- `event_fields` order, `the_webex_id_comes_before_the_appointments_own_fields`: the `ExtendedProperty` comes after `ReminderIsSet` and before `<t:Start>` (EWS keeps the schema's order). None when `webex_id` is `None`.
- `graph_pim.rs`, in `events_and_contacts_both_ways`: `event_json` holds `singleValueExtendedProperties` `[{"id": "String {5b7a1c3e-8f44-4f0e-9c1d-3e2a7d6b4c11} Name WebexId", "value": id}]`, and `event_of` reads it back. A sync against the fake asks events with `$expand=singleValueExtendedProperties`.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p quatrelle-net webex_id`
Expected: FAIL.

- [ ] **Step 3: Implement**

- EWS: `WEBEX_ID` is `<t:ExtendedFieldURI PropertySetId="…" PropertyName="WebexId" PropertyType="String"/>`. Read it in the shape's `AdditionalProperties`, and in `appointment()` through a helper by `PropertyName` beside `extended_text`.
- `event_fields`: push it, as `tasks.rs` pushes `TASK_COMPLETE`, just before the `calendar:Start` block, only when `Some`.
- `update_item`: an entry whose key is an `ExtendedFieldURI` becomes `<t:SetItemField>{uri}<t:CalendarItem>…`, as tasks' update does. An event with no id leaves the property alone.
- Graph: add the `$expand` (percent-encoded, as `graph.rs:50` writes its own) to both `$select={EVENT_FIELDS}` lists; read in `event_of` through `valid_webex_id`; write in `event_json` only when `Some`.
- Fakes: keep the property from `CreateItem` and `UpdateItem` and answer it; keep Graph's on the event's JSON.

- [ ] **Step 4: Run the tests and `cargo xtask ci`**

Expected: PASS, and the fake's schema check accepts the new element.

- [ ] **Step 5: Commit**

`git commit -m "Exchange and Graph: the Webex id travels with the appointment"`

---

### Task 4: The Webex client

**Files:**
- Create: `crates/net/src/webex.rs`, `crates/net/src/webex/tests.rs`, `crates/net/src/fakes/fake_webex.rs`
- Modify: `crates/net/src/oauth.rs`, `crates/net/src/lib.rs`, `crates/net/src/fakes.rs`, and `MemoryCredentials` in `crates/net/src/worker.rs` if it keeps no `set_secret` values

**Interfaces:**
- Consumes: Task 2's types.
- Produces in `crates/net/src/webex.rs`:

```rust
pub const API: &str = "https://webexapis.com/v1";
pub const AUTHORIZE_URL: &str = "https://webexapis.com/v1/authorize";
pub const TOKEN_URL: &str = "https://webexapis.com/v1/access_token";
pub const SCOPE: &str = "meeting:schedules_read meeting:schedules_write spark:people_read";
pub const REFRESH_KEY: &str = "webex-refresh";
pub const SECRET_KEY: &str = "webex-secret";
pub fn keyring() -> AccountConfig; // id and email "webex"
pub fn provider(client_id: &str, client_secret: &str) -> Option<Provider>; // id "webex", name "Webex"
pub struct Webex { /* api, token_url, client_id, creds, the access token and the refresh token it came from */ }
impl Webex {
    pub fn new(client_id: &str, creds: Arc<dyn Credentials>) -> Webex;
    pub fn create(&mut self, site: &str, fields: &WebexFields, quiet: bool) -> Result<WebexMeeting, WebexError>;
    pub fn update(&mut self, id: &str, before: &WebexFields, after: &WebexFields, quiet: bool) -> Result<WebexMeeting, WebexError>;
    pub fn delete(&mut self, id: &str, quiet: bool) -> Result<(), WebexError>;
    pub fn me(&mut self) -> Option<String>;
}
```

- Produces in `oauth.rs`: `pub fn authorize_on(p: &Provider, port: u16, login_hint: &str, open: &mut dyn FnMut(&str) -> std::result::Result<(), String>, timeout: Duration) -> Result<Tokens>`. `authorize` calls it with port 0.

- [ ] **Step 1: Write the failing tests**

`fake_webex.rs` follows `fake_graph.rs`: on localhost, `TOKEN`, `Data { meetings, calls: Vec<String> /* "METHOD path" with query */, bodies: Vec<Value>, refuse: VecDeque<(u16, Value)>, refreshes: usize, rotate: bool }`. It serves `POST /v1/meetings`, `PATCH` and `DELETE /v1/meetings/{id}`, `GET /v1/people/me`, and the form `POST /v1/access_token`.

In `webex/tests.rs`, over the fake, with `api` and `token_url` set to it:
- `creating_posts_the_appointment_to_the_site`: `calls == ["POST /v1/meetings"]`; the body has `title`, `start: "2026-11-02T09:00:00Z"`, `end`, `timezone: "Europe/Paris"`, `siteUrl: "acme.webex.com"`, `sendEmail: false` when quiet and `true` when not, `recurrence` only when set. The result holds the answer's `id`, `webLink`, `meetingNumber`, `password`, `sipAddress`.
- `no_personal_room_link_is_ever_built`: no call's path and no body contains `/meet/`.
- `changing_the_time_sends_only_what_changed`: `PATCH /v1/meetings/m1` whose body has `start`, `end`, `timezone`, `sendEmail`, and no `title`, `password` or `siteUrl`. A title change alone sends `title` and `sendEmail`.
- `deleting_asks_webex_to_mail_no_one`: quiet gives `DELETE /v1/meetings/m1?sendEmail=false`; not quiet, no query.
- `a_meeting_already_gone_is_deleted`: a 404 on `delete` is `Ok(())`.
- `an_expired_token_is_refreshed_once_and_the_request_tried_again`: 401 then success gives `["POST /v1/meetings", "POST /v1/access_token", "POST /v1/meetings"]`; with `rotate`, the keychain holds the new refresh token.
- `a_second_401_asks_to_sign_in_again`: `Err(WebexError::SignIn)`.
- `no_refresh_token_asks_to_sign_in`: `Err(WebexError::SignIn)`, and `calls` is empty.
- `a_403_says_the_account_cant_schedule`: `Err(WebexError::Forbidden)`.
- `other_refusals_carry_the_status_and_webexs_words`: a 400 with `{"message":"x","errors":[{"description":"y"}]}` gives `Refused { status: 400, message: "y" }`; a 502 with no JSON gives `Refused { status: 502, .. }` and no panic.
- `hostile_answers_are_refused_or_cleaned`: a newline in `password` is cleaned; `webLink` `javascript:x` or `http://x` is refused; no `id`, or an `id` failing `valid_webex_id`, is refused; a 2 MB body is an `Err`, not a panic.

In `oauth.rs`'s tests:
- `webexs_token_request_carries_the_secret_and_no_scope`: the form posted by `refresh` with a `webex` provider has `client_secret` and no `scope`.
- `sign_in_listens_on_the_port_asked_for`: `authorize_on` on a free port puts `redirect_uri=http%3A%2F%2F127.0.0.1%3A{port}%2F` in the URL.
- `a_port_in_use_says_so`: with that port taken, the error names the port.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p quatrelle-net webex`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

- `oauth.rs`: `authorize_on` binds `127.0.0.1:{port}`; `AddrInUse` on a fixed port becomes `NetError::Auth("port {port} is in use: a sign-in may still be waiting in the browser")`. `token_request` sends no `scope` for `webex` as for `google`.
- The `expires_in` clamp in `token_request` stays. It keeps an access token at most a day, and refreshing daily is harmless.
- `Webex`: every request goes through one private function that takes the token, sends, and on a 401 drops it, takes a fresh one and sends once more; a second 401 is `SignIn`.
- Taking a token: read `REFRESH_KEY` and `SECRET_KEY` from the keychain (none: `SignIn`). A cached token stays good only while the refresh token is the one it came from and it hasn't `expired()`. Otherwise `oauth::refresh`: `NetError::Auth` is `SignIn`, any other error is `Failed`. A rotated refresh token is saved.
- Bodies: `start` and `end` as `YYYY-MM-DDTHH:MM:SSZ` with the formatter EWS uses. `create` always sends `title`, `start`, `end`, `timezone`, `siteUrl`, `sendEmail: !quiet`. `update` sends `title` if changed; `start`, `end` and `timezone` together if any of them changed; `recurrence` if changed; always `sendEmail`.
- Answers: read with `max = 1 << 20`. Clean `meetingNumber`, `password`, `sipAddress`: no control characters, trimmed, at most 300 characters. `webLink` must be `https://`, at most 2048 characters, with no control character. `id` is percent-encoded with `http::pct` in the URL.
- `refusal`: `errors[0].description`, else `message`, else the status alone, on one line, at most 300 characters.
- `me`: `GET /people/me`, the first of `emails`. Any failure is `None`.
- `Request` has `pub method`: build `PATCH` and `DELETE` from `Request::post` and `Request::get`.

- [ ] **Step 4: Run the tests and `cargo xtask ci`**

Expected: PASS.

- [ ] **Step 5: Commit**

`git commit -m "Net: Webex's meetings through its public API, with a sign-in on a fixed port"`

---

### Task 5: Jobs, the worker thread and the session's state

**Files:**
- Create: `crates/net/src/worker/webex.rs`, `crates/engine/src/session/pim/webex.rs`
- Create: `crates/engine/src/tests/calendar/webex.rs` (`mod webex;` in `tests/calendar.rs`)
- Modify: `crates/model/src/job.rs`, `crates/model/src/job/events.rs`, `crates/net/src/worker/pool.rs`, `crates/net/src/worker/changes.rs`, `crates/engine/src/session.rs`, `crates/engine/src/session/events.rs`, `crates/engine/src/session/prefs.rs`, `apps/quatrelle-web/src/backend.rs`

**Interfaces:**
- Consumes: Tasks 2 and 4.
- Produces:

```rust
// Job
WebexSignIn { client_id: String },
WebexSignOut,
SetWebexSecret { secret: Secret },
WebexCreate { ask: u64, client_id: String, site: String, fields: WebexFields, quiet: bool },
WebexUpdate { client_id: String, id: String, before: WebexFields, after: WebexFields, quiet: bool },
WebexDelete { client_id: String, id: String, quiet: bool },
// Event
WebexSignedIn { user: Option<String>, error: Option<WebexError> },
WebexCreated { ask: u64, meeting: Option<WebexMeeting>, error: Option<WebexError> },
WebexUpdated { id: String, meeting: Option<WebexMeeting>, error: Option<WebexError> },
WebexDeleted { id: String, error: Option<WebexError> },
// Prefs (serde skipped when empty or false)
webex_site: String, webex_client_id: String, webex_user: String, webex_signed_in: bool,
// Session.webex: WebexState
pub struct WebexAsk { pub fields: WebexFields, pub quiet: bool, pub abandoned: bool }
#[derive(Default)]
pub struct WebexState {
    pub signing_in: bool,
    pub sign_in_error: Option<WebexError>,
    /// Creations on their way, by `ask`.
    pub asks: HashMap<u64, WebexAsk>,
    /// Their answers, until the window that asked takes them.
    pub answers: HashMap<u64, Result<WebexMeeting, WebexError>>,
    /// What each meeting made in this run was last sent with.
    pub sent: HashMap<String, WebexFields>,
}
```

- [ ] **Step 1: Write the failing tests**

In `tests/calendar/webex.rs`, applying events to a session built as `tests/calendar.rs` builds one:
- `an_answer_waits_for_its_window`: with an ask in `asks`, `WebexCreated` with a meeting puts `Ok` in `answers`, takes the ask out, and records its fields in `sent`.
- `a_meeting_answered_after_its_window_closed_is_deleted`: an ask `abandoned`, `WebexCreated` with a meeting pushes `Job::WebexDelete` with that id and `quiet` from the ask, and leaves `answers` empty.
- `an_abandoned_ask_that_failed_deletes_nothing`.
- `a_failed_creation_is_told_to_its_window`: `answers` holds `Err`.
- `a_failed_update_or_delete_is_a_notice`: `WebexUpdated` and `WebexDeleted` with an error give a notice, and the event in the calendar is unchanged.
- `signing_in_keeps_who_and_stops_waiting`: `WebexSignedIn` sets `webex_user` and `webex_signed_in`, clears `signing_in`. With an error, `sign_in_error` holds it and `webex_signed_in` stays false.
- `every_webex_job_goes_to_no_account`: `Job::account()` is `""` for each.

In `crates/net/src/worker/webex.rs`, `jobs_run_in_order_on_one_thread`: over the fake, a create, an update and a delete submitted one after the other reach the fake in that order, and each emits its event.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p quatrelle-engine webex && cargo test -p quatrelle-net jobs_run_in_order`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

- `worker/webex.rs`: `impl Pool { pub(super) fn webex(&mut self, job: Job) }`. A lazily started thread `webex` takes jobs from a channel, as `fetch_url` does, so that they run in order and a refresh never races another. It keeps a `Webex`, replaced when `client_id` changes.
- `WebexSignIn` runs on a thread of its own (it blocks for up to five minutes): the secret from the keychain, `webex::provider`, `oauth::authorize_on(&p, WEBEX_REDIRECT_PORT, "", &mut open, 5 min)` with `open` emitting `Event::OpenUrl`, as `sign_in` does; `t.refresh` to the keychain; then `Webex::me`. No secret kept: `WebexError::Failed("the client secret isn't set")`. No refresh token in the answer: `WebexError::Failed("Webex gave no lasting sign-in; try again")`.
- `WebexSignOut` deletes `REFRESH_KEY` on the job thread. `SetWebexSecret` follows `SetLlmKey`.
- Pool arms group the six jobs as `smime` groups its own. `Job::account()` and `changes.rs` list them with `AskLlm`.
- Web backend: each answers `WebexError::Failed("Webex needs the desktop app")` through its own event. `WebexSignOut` and `SetWebexSecret` do nothing.
- `session/pim/webex.rs`: `webex_signed_in`, `webex_created`, `webex_updated`, `webex_deleted`, wired in `events.rs`. Notices: "The Webex meeting couldn't be updated: {error}" and "The Webex meeting couldn't be deleted: {error}", through `notice_with`, with catalog rows.
- `webex_updated` with a meeting: when the event with that `webex_id` has a `join_link()` other than `web_link`, replace the old link in `description` and `location` through `update_event`, then `tell_guests` as `event.update` does.

- [ ] **Step 4: Run the tests and `cargo xtask ci`**

Expected: PASS. Add `changing_the_link_reaches_the_notes` to the engine tests for the replacement.

- [ ] **Step 5: Commit**

`git commit -m "Engine: Webex jobs on one thread, their answers kept for the window"`

---

### Task 6: Commands, and the appointment followed

**Files:**
- Create: `crates/engine/src/cmd/webex.rs`
- Modify: `crates/engine/src/cmd.rs`, `crates/engine/src/cmd/pim.rs`, `crates/engine/src/session/pim/moving.rs`, `crates/engine/src/session/pim/webex.rs`, `crates/engine/src/tests/calendar/webex.rs`

**Interfaces:**
- Consumes: Task 5.
- Produces commands:
  - `webex.settings` `{"site"?: string, "clientId"?: string, "secret"?: string ("" forgets it)}`
  - `webex.signIn`, `webex.signOut`
  - `webex.createMeeting` `{"summary": string, "start": "YYYY-MM-DD[THH:MM]", "end"?: …, "allDay"?: bool, "repeat"?: …, "quiet"?: bool}`, returns `{"ask": number}`
  - `webex.updateMeeting` `{"id": string, … as createMeeting}`
  - `webex.deleteMeeting` `{"id": string, "quiet"?: bool} | {"ask": number}`
- Produces `event.create` and `event.update` param `"webexId"?: string`.
- Produces on `Session`:
  - `pub(crate) fn webex_follow(&mut self, before: Option<&CalEvent>, after: &CalEvent)`
  - `pub(crate) fn webex_gone(&mut self, e: &CalEvent, occurrence: Option<i64>)`
  - `pub fn webex_take_answer(&mut self, ask: u64) -> Option<Result<WebexMeeting, WebexError>>`

- [ ] **Step 1: Write the failing tests**

In `tests/calendar/webex.rs`, reading `s.jobs` after each command:
- `settings_keep_the_site_as_a_host`: `https://acme.webex.com/` is kept as `acme.webex.com`; `not a site` is a `Params` error; `secret` pushes `Job::SetWebexSecret`.
- `signing_in_needs_a_client_id_and_waits`: refused with none; with one it pushes `WebexSignIn`; a second call is refused while `signing_in`.
- `signing_out_forgets_who`.
- `creating_needs_a_title_a_start_a_site_and_a_sign_in`: each missing one is an `Err`, and no job.
- `creating_asks_webex_with_the_appointments_fields`: `Job::WebexCreate` has the title, times, `timezone`, `site`, and `quiet` as given.
- `the_first_save_sends_what_changed_since_the_click`: `sent` holds the click's fields; `event.create` with `webexId` and a later time pushes `WebexUpdate` with them as `before`.
- `a_save_that_changes_nothing_sends_nothing`: `event.create` and `event.update` with the same fields push no Webex job.
- `changing_an_appointment_updates_its_meeting`: `event.update` of title, and `event.move`, each push `WebexUpdate`.
- `undoing_a_move_updates_its_meeting`.
- `deleting_an_appointment_deletes_its_meeting`: `quiet` is true with guests, false without.
- `a_strangers_webex_id_is_never_acted_on`: an event whose `organizer` is another address: `event.update`, `event.move` and `event.delete` push no Webex job.
- `one_occurrence_leaves_the_series_meeting_alone`: `event.delete` and `event.move` with an `occurrence` of a recurring event push no Webex job; the series' own move does.
- `no_meeting_no_webex`: an event without `webex_id` pushes none.
- `closing_during_a_creation_marks_it_abandoned`: `webex.deleteMeeting {"ask": n}` sets `abandoned`, and pushes nothing.
- `a_foreign_id_is_refused`: `webexId` failing `valid_webex_id` is a `Params` error.
- `every_command_survives_junk_params` stays green.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p quatrelle-engine webex`
Expected: FAIL.

- [ ] **Step 3: Implement**

- `cmd/webex.rs` returns its `CommandSpec`s as `cmd/home.rs` does.
- `webex.settings`: the site goes through `url_host`, and must have a dot. The client id is 1 to 200 ASCII letters, digits, `-`, `_`, `.`.
- `createMeeting` and `updateMeeting` build the fields with `event_from(s, v, CalEvent::default())` and `WebexFields::of`. A blank title, or no `start` param, is `Params("a Webex meeting needs a title and a start")`. Not signed in or no site is `Disabled`, naming File › Options › Calendar.
- `updateMeeting`'s `before` is `sent.get(id)`, else `WebexFields::default()`.
- `event_from`: `webexId` through `valid_webex_id`.
- `webex_follow`: nothing unless `after.webex_id` is `Some`, `after.recurrence_id` is `None`, and it's the user's: `organizer` is `None` or one of the accounts' addresses. `last` is `sent.get(id)`, else `before`'s fields if `before` has the same id, else no job. When `WebexFields::of(after) != last`, push `WebexUpdate` and record `sent`. `quiet` is `!after.attendees.is_empty()`.
- `webex_gone`: the same test on the event, nothing with `occurrence`, else `WebexDelete`; drop `sent`.
- Hooks: `event.create` after `create_event`; `event.update` with `current` as `before`; `event.delete` before its guests' branch; `move_event_to` only where the whole event moved, so that Undo follows too.

- [ ] **Step 4: Run the tests and `cargo xtask ci`**

Expected: PASS.

- [ ] **Step 5: Commit**

`git commit -m "Commands: Webex meetings, and appointments that take theirs along"`

---

### Task 7: Options › Calendar › Webex

**Files:**
- Create: `crates/ui-egui/src/panes/backstage/options/webex.rs`
- Modify: `crates/ui-egui/src/panes/backstage/options.rs`, `crates/ui-egui/src/lib.rs` (`Action::WebexOptions`), `crates/ui-egui/src/app/actions.rs`, `crates/ui-egui/src/i18n/*.tsv`
- Test: `crates/ui-egui/src/tests/scheduling/webex.rs` (`mod webex;` in `tests/scheduling.rs`)

**Interfaces:**
- Consumes: `webex.settings`, `webex.signIn`, `webex.signOut`, `Prefs.webex_*`, `Session.webex`, `WEBEX_REDIRECT_URI`.
- Produces: `Action::WebexOptions`: File › Options, scrolled to the Webex card (a temp flag in `ctx.data`, read once by the card, which calls `scroll_to_me`). Task 8 uses it.

- [ ] **Step 1: Write the failing tests**

- `the_webex_card_draws_signed_out_and_in`: headless frames of Options in both states; no panic.
- `the_secret_is_never_shown_again`: after leaving the secret field, the temp text is cleared and `webex.settings` was run with `secret`.
- `the_redirect_uri_is_shown_to_copy`: the card's redirect text equals `WEBEX_REDIRECT_URI`.
- `webex_options_opens_the_card`: `Action::WebexOptions` opens `backstage::Page::Options`.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p quatrelle-ui-egui webex`
Expected: FAIL.

- [ ] **Step 3: Implement**

A card "Webex" after the Calendar group, drawn only `if !cfg!(target_arch = "wasm32")`, in the manner of `options/assistant.rs`. Rows:
- "Site" (hint `acme.webex.com`)
- "Client ID"
- "Client secret" (`password(true)`, saved once left, then cleared; a note says it's kept in the keychain)
- "Redirect URI" (the value, with a copy button; a note says to paste it when registering the integration)
- "Sign in to Webex" (disabled without a client id, a spinner with "Waiting for the browser…" while `signing_in`). Signed in, "Signed in as {user}", or "Signed in" with no user, then "Sign out".
- `sign_in_error` under it, in `t.danger`.

The `Webex personal room` row stays where it is. Its note changes to: "Use my personal room, under Webex Meeting in an appointment's window, puts it in."

- [ ] **Step 4: Run the tests and `cargo xtask ci`**

Expected: PASS.

- [ ] **Step 5: Commit**

`git commit -m "Options: Webex's site, integration and sign-in"`

---

### Task 8: The appointment window

**Files:**
- Create: `crates/ui-egui/src/dialogs/pim_dialogs/webex.rs`, `crates/ui-egui/src/app/webex.rs`
- Modify: `crates/ui-egui/src/dialogs/pim_dialogs/meeting.rs`, `crates/ui-egui/src/dialogs/pim_dialogs/event.rs`, `crates/ui-egui/src/dialogs/pim_dialogs.rs`, `crates/ui-egui/src/app/windows.rs`, `crates/ui-egui/src/app/sending.rs`, `crates/ui-egui/src/i18n/*.tsv`, `crates/ui-egui/src/tests/scheduling.rs`, `crates/ui-egui/src/tests/scheduling/webex.rs`

**Interfaces:**
- Consumes: `webex.createMeeting`, `webex.deleteMeeting`, `Session::webex_take_answer`, `Action::WebexOptions`.
- Produces on `EventForm`:
  - `webex_id: Option<String>` (from the event)
  - `webex_fresh: bool` (made from this window and not saved)
  - `webex_wait: Option<u64>` (the creation being waited for)
- `EventForm::params()` adds `"webexId"` when set.
- In `pim_dialogs/webex.rs`:
  - `pub(crate) fn webex_params(f: &EventForm) -> Value`: `summary`, `start`, `end`, `allDay`, `repeat`, and `quiet: f.inviting`
  - `pub(crate) fn take_meeting(f: &mut EventForm, m: &WebexMeeting)`
  - `pub(crate) fn error_text(e: &WebexError) -> String`
  - `pub(crate) fn leaving(f: &EventForm) -> Option<Value>`: the `webex.deleteMeeting` params for what the window leaves unsaved
- Produces `Action::ForWebex(u64, Value)`: window `w` asks for a creation.

- [ ] **Step 1: Write the failing tests**

In `tests/scheduling/webex.rs`:
- `the_plain_click_creates_a_meeting`: a signed-in session, a titled form: the click pushes `Job::WebexCreate`, and `webex_wait` is set.
- `no_title_says_so_and_sends_nothing`: `f.error` is set, no job.
- `signed_out_opens_options`: the click gives `Action::WebexOptions`, no job.
- `the_answer_goes_into_the_appointment`: `take_meeting` with a blank place sets `location` to `web_link`; a typed place stays. The notes hold "Join the Webex meeting:", the link, and the number, password and video lines; `join` is set; `webex_id` is set; `webex_fresh` is true.
- `empty_values_write_no_line`: no `password`, no password line.
- `a_refused_creation_shows_and_changes_nothing`: for each of `SignIn`, `Forbidden`, `Refused`, `Failed`: `f.error` holds its text, and `location`, `description` and `join` are as they were.
- `the_personal_room_stays_a_choice`: "Use my personal room" pushes no Webex job and puts the room in as before. Move the existing `tests/scheduling.rs` personal-room tests onto the menu entry.
- `save_waits_for_the_webex_answer`: with `webex_wait` set, `for_event` with `event.create` does nothing, and the ribbon's Save is off. It goes once the answer is taken.
- `saving_sends_the_id_and_keeps_the_meeting`: `params()` holds `webexId`; closing after a save pushes no `WebexDelete`.
- `closing_unsaved_deletes_the_meeting`: closing with `webex_fresh` runs `webex.deleteMeeting` with the id.
- `closing_during_the_creation_abandons_it`: closing with `webex_wait` runs `webex.deleteMeeting` with the ask.
- `deleting_the_event_with_a_fresh_meeting_leaves_nothing`: `event.delete` from the window with `webex_fresh` set also deletes the fresh meeting.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p quatrelle-ui-egui webex`
Expected: FAIL.

- [ ] **Step 3: Implement**

- **Ribbon (desktop):** `big_button` "Webex Meeting" (a click is New Webex meeting) and beside it a small `Icon::ChevronDown` button opening `egui::Popup::menu` with "New Webex meeting" and "Use my personal room". The latter keeps today's behaviour, `webex_room` when none is set.
- **Ribbon (wasm):** the button stays as it is today.
- **The click:** blank title or date sets `f.error` to `tl!("A meeting needs a title and a start")`. `!prefs.webex_signed_in` gives `Action::WebexOptions`. Otherwise `Action::ForWebex(wid, webex_params(f))`, which runs the command and sets `webex_wait` from its `ask`, or `f.error` from its refusal.
- **Busy:** while `webex_wait` is set, a spinner with `tl!("Creating the Webex meeting…")`, beside "Checking names…". `ribbon`'s `checking` argument is `checking || f.webex_wait.is_some()`, and `for_event` returns early on `event.create` and `event.update` then.
- **Answers:** `app/webex.rs`, called at the top of `event_windows`, takes each window's answer: `take_meeting`, or `f.error = Some(error_text(&e))`. `WebexError::SignIn` in `error_text` is "Sign in to Webex again".
- **Notes:** each on its own line, skipping empty values, after a blank line: `tl!("Join the Webex meeting:")`, the link, `tf!("Meeting number: {number}")`, `tf!("Password: {password}")`, `tf!("Video: {address}")`.
- **Leaving:** where `event_windows` collects `closed`, push `Action::Run("webex.deleteMeeting", …)` from `leaving(f)`. In `event_command`'s success path, a command other than `event.create` and `event.update` does the same. A save clears nothing to delete, because `event_command` drops the form.

- [ ] **Step 4: Run the tests and `cargo xtask ci`**

Expected: PASS.

- [ ] **Step 5: Look at it**

Build and run with `--sample --data <scratchpad folder>` and `QUATRELLE_SCREENSHOT`, `--cmd ui.options`, and an appointment window. Read the PNGs: the card and the button with its chevron, light and dark. Fix what looks wrong.

- [ ] **Step 6: Commit**

`git commit -m "Appointment window: Webex Meeting schedules a meeting, the personal room by choice"`

---

### Task 9: Documentation and the final gates

**Files:**
- Create: `docs/webex.md`
- Modify: `CLAUDE.md` (L3's list), `ROADMAP.md`, `docs/gaps.md`, `docs/oauth.md`

- [ ] **Step 1: Write `docs/webex.md`**

In the manner of `docs/teams.md`: what it does, then registering the integration on developer.webex.com (Redirect URI `http://127.0.0.1:53683/`; scopes `meeting:schedules_read`, `meeting:schedules_write`, `spark:people_read`; the secret is typed into Options, never kept in a file). Then what a real account must confirm: the sign-in, a create, an update, a delete, and that no second invitation goes out.

- [ ] **Step 2: Update the other documents**

`ROADMAP.md` and `docs/gaps.md` as earlier deliveries did; `docs/oauth.md` gains the fixed port and why; `CLAUDE.md` adds Webex to L3's list.

- [ ] **Step 3: Run the gates**

Run:
```bash
cargo fmt --all
cargo xtask ci
CARGO_TARGET_DIR=$PWD/target-stable cargo +stable clippy --workspace --all-targets -- -D warnings
```
Expected: all exit 0. The known flake `a_big_folder_changes_all_at_once_quickly` passes when run again alone.

- [ ] **Step 4: Commit**

`git commit -m "Docs: Webex, from registering the integration to what a real account confirms"`
