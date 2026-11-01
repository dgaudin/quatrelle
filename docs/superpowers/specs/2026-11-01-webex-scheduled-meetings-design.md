# Webex: a scheduled meeting, not the personal room

Status: the owner decided the goal and three choices on 2026-10-31 ("2 ok on fait dans cet ordre"), and
approved this written spec on 2026-11-01 ("webex ok"). Facts about Webex were read in its public developer
documentation on 2026-11-01.

## Why

Quatrelle's Webex Meeting button only writes the personal room's link, typed once in Options, into the
appointment (`ui-egui/src/dialogs/pim_dialogs/meeting.rs`, `add_webex`; `prefs.webex_room`;
`calendar.webexRoom`). No request goes to Webex. So every meeting shares one room and one link, with no
meeting number or password of its own, and nothing ties it to the appointment's time.

Outlook's Cisco Webex Scheduler creates a real scheduled meeting on the user's site. Its endpoints
(`meetings-api.webex.com/wbxaddin`) are private to Cisco's add-in. The supported way for another client is
the public REST API.

## The owner's decisions (2026-10-31)

- **The default is a real scheduled meeting**, through `https://webexapis.com/v1/meetings`, with the
  appointment's title, times and time zone, on the configured site.
- **OAuth through an integration the owner registers** on developer.webex.com; the token in the system
  keychain; `siteUrl` from Options, never hard-coded.
- **The Webex meeting's id is kept on the event**, so that changing or cancelling the appointment in
  Quatrelle changes or deletes the meeting. A change made in Outlook or on a phone doesn't: only Quatrelle
  knows to do it.
- **The invitation carries what the API answers** (`webLink`, `meetingNumber`, `password`, `sipAddress`),
  never a link built by hand.
- **The personal room stays, as an explicit choice** in the button's menu. No API error ever falls back
  to it silently.

## What Webex's documentation settles

- **Sign-in:** `https://webexapis.com/v1/authorize`, then a form POST to
  `https://webexapis.com/v1/access_token`.
  - The token request needs `client_secret`, **with PKCE too** (Login with Webex, step 7).
  - The documentation says never to expose the secret in browser or mobile application code.
  - So nothing is built into Quatrelle: the owner types the integration's client id and secret into
    Options, and the secret goes to the system keychain. Each user or organisation has its own integration.
- **The redirect URI must match exactly** one registered with the integration. Quatrelle's sign-in today
  listens on a random port (`oauth::authorize`), so Webex needs a **fixed** one:
  `http://127.0.0.1:53683/`. Options shows it, with a copy button, for the owner to paste when registering.
- **Tokens:** use the expiry the server returns. Typically 14 days for the access token and 90 for the
  refresh token. A refresh may return a new refresh token, which replaces the old one.
- **Permissions (scopes):** `meeting:schedules_read` and `meeting:schedules_write` for the meetings, and
  `spark:people_read` for `GET /v1/people/me`, which names the account shown in Options.
- **Meetings:** `POST /v1/meetings`; `PATCH` and `PUT /v1/meetings/{id}`; `DELETE /v1/meetings/{id}`.
  `PUT` is to be retired. `PATCH` requires nothing but the id. `sendEmail` is optional and defaults to true.

## What the owner sees

- **Options › Calendar › Webex:** site (`<org>.webex.com`), client id, client secret, the redirect URI to
  register, and "Sign in to Webex" (then the account shown, and "Sign out"). The existing "Webex personal
  room" field stays.
- **In the appointment window**, the Webex Meeting button is a menu:
  - **"New Webex meeting"** (the default; a plain click does it);
  - **"Use my personal room"** (what the button does today).

  Not signed in, "New Webex meeting" opens Options at the Webex section.
- **Creating:** the button shows it's working. Then:
  - the place is set to `webLink` unless one was typed;
  - the notes gain, in the invitation's language:
    « Rejoindre la réunion Webex : {webLink} », « Numéro de réunion : {meetingNumber} »,
    « Mot de passe : {password} », « Vidéo : {sipAddress} »;
  - Join works, as for any meeting link.

  A meeting needs a title and a start: without them, the button says so and sends nothing.
- **Saving a changed appointment** (title, start, end, time zone, repeat) updates the Webex meeting. If the
  answer gives another `webLink`, the old one is replaced in the notes and the place. The meeting number,
  password and video address can't change this way: Quatrelle never sends them.
- **Deleting or cancelling the appointment** deletes the Webex meeting.
- **Closing the window without saving** deletes the meeting just created: nothing is left on the site.
- **Errors** show in the window and change nothing in the appointment:
  - 401: the token is refreshed once and the request tried again; if that fails, "Sign in to Webex again";
  - 403: "Your Webex account can't schedule meetings on this site";
  - other 4xx and 5xx: Webex's own message, with the status.

  A failed update or delete after the appointment was saved shows a notice, and the appointment stays
  saved: Webex isn't allowed to undo a change in the calendar.

## Decisions this spec makes

1. **`PATCH`, not `PUT`, for an update.** Webex offers both at the same URL. `PATCH` sends only what
   changed; `PUT` replaces the meeting and needs its password and full details sent back. *Cost if wrong:*
   one verb.
2. **Webex is a service of Quatrelle, not of an account**, like the LLM and Teams: its settings are in
   `Prefs`, its token in the keychain under a dummy keyring, and its requests are `Job`s with no account.
   The Webex id belongs to the event, whichever account holds it.
3. **The meeting is created at the click, as the Scheduler does**, so that the invitation can be read and
   edited before sending. Hence the delete on closing without saving. If Quatrelle crashes between the two,
   a meeting stays on the site: harmless, and visible there.
4. **A recurring appointment** passes its `RRULE` as `recurrence`. Webex doesn't take several days or
   dates in a monthly or yearly rule: it answers with an error, shown as such. There's no attempt to
   convert one.
5. **`sendEmail` is false on create, update and delete** when the appointment invites guests: Exchange's
   invitation is the one they get, and Webex shouldn't mail them a second.
6. **One occurrence of a series isn't the series.** Deleting or moving one occurrence sends nothing to
   Webex: the id is the whole series'. A change made there stays in the calendar alone.
7. **The Webex id is the organizer's.** It isn't written into an invitation, and an event organized by
   someone else never sends anything to Webex, whatever id it carries: a stranger's invitation mustn't be
   able to make the user's own token delete a meeting.

## Where the code goes

- **Model:** `CalEvent.webex_id: Option<String>`, in iCal as `X-QUATRELLE-WEBEX-ID`
  (`ical/meeting.rs` has the pattern). `Prefs.webex_site`, `webex_client_id`.
- **Exchange:** an extended property in a property set of Quatrelle's own, read with the appointment and
  written on create and update (`exchange/ews_pim.rs` has the pattern). Graph:
  `singleValueExtendedProperties`.
- **Net:** a new `crates/net/src/webex.rs` (the REST calls, hostile answers capped), and `oauth::authorize`
  takes an optional fixed port. `Job::Webex…` and `Event::Webex…`, as `AskLlm` does it. Tokens through
  `Credentials::set_secret`, under a `webex::keyring()`.
- **Engine:** commands `webex.signIn`, `webex.signOut`, `webex.createMeeting`, `webex.updateMeeting`,
  `webex.deleteMeeting`, and the settings. `calendar.webexRoom` stays.
- **UI:** the button's menu (`pim_dialogs/meeting.rs`), the Options section, the window's busy state and
  error. New strings in the 13 catalogs.
- **The credentials rule** (`http/redirect.rs`) applies as it does to all `http::send`: the base is fixed,
  so a redirect to another site loses the token.

## Testing

- A fake Webex server in `net/src/fakes`:
  - the default click sends `POST /v1/meetings` with the title, times, time zone and `siteUrl`;
  - no `/meet/` URL is ever built on that path;
  - the notes hold the answer's four values;
  - changing the time sends `PATCH`, deleting sends `DELETE`, closing unsaved sends `DELETE`;
  - "Use my personal room" sends nothing to Webex;
  - 401 refreshes once and retries; a second 401 asks to sign in; 403, 4xx and 5xx are shown, and the
    personal room is never used instead;
  - an unsaved window whose meeting creation is still running deletes the meeting when the answer comes.
- `X-QUATRELLE-WEBEX-ID` round-trips through iCal; the EWS property through the fake EWS.
- `every_command_survives_junk_params`; junk answers from the fake are refused without panic.

## Out of scope

- Reading a meeting back from Webex, or following a change made on Webex's site.
- Webex Calling, Teams-style chats, recordings.
- The `meetings-api.webex.com/wbxaddin` endpoints.
- Validation with a real account: the owner does that once the integration is registered.

## Delivery

One branch, one test build. The owner then registers the integration, signs in and tries it.
