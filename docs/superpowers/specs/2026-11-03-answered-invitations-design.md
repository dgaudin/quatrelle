# Calendar: an answered invitation leaves the Inbox

Status: the second of the four calendar features of upstream ZIDAN44 v0.31
(`~/mailcraft/merge-tools/survey-v0.31.md`, « Décidé le 02/11 »), issue #31. Confirmed by the owner on
2026-11-02 as gap-filling, not a port; this design approved on 2026-11-03 with two decisions: **one
checkbox, as Outlook has it** (not upstream's three choices with a folder per account), and **a
cancellation removed from the calendar goes too**. Upstream's `484e4c2f`, `25c6997b` and `3c4febce` are
the reference; their code builds on `497a480a` (tentative invitations on arrival), left out here.

## Why

Once an invitation is answered, its message is only clutter. Outlook's option "delete meeting requests
and notifications from Inbox after responding" is on from the start. Quatrelle already does part of it,
always and with no option: an invitation answered **in the message** goes to Deleted Items, its window
closes, and the list goes on to the next message (`respond_to_invite`, through `move_selection`).
What's missing:

- an answer given **in the calendar** (the appointment's window, `event.respond`) leaves the invitation
  in the Inbox;
- the meeting's **earlier invitations** (the first one, then each update) stay in the Inbox; on IMAP
  nothing else takes them away;
- a **cancellation** stays after "Remove from calendar";
- there's **no option** to keep them.

## What the owner sees

- **File › Options › Calendar**: a new switch, "Move meeting invitations to Deleted Items once
  answered", on by default (today's behaviour), with the hint "Also a cancellation once its meeting is
  removed from the calendar, and the meeting's earlier invitations." Our own wording, not Outlook's.
- **Answered in the message** (Accept, Tentative, Decline in the reading pane or the message's window):
  as today, the invitation goes to Deleted Items, its window closes, the list shows the next message
  without marking it read. Now its earlier versions in the Inbox go with it.
- **Answered in the calendar**: the meeting's invitations in the Inbox go to Deleted Items. The message
  being read stays, unless it's one of them; then the list goes on to the next one.
- **"Remove from calendar"** on a cancellation: the cancellation and the meeting's invitations go to
  Deleted Items.
- **An update newer than the version answered stays in the Inbox**, to be seen and answered.
- With the switch off, nothing moves; the message's window still closes after an answer, as today.
- Ctrl+Z puts the messages back, as for any move.

## Rules

1. **Which messages.** In the account's Inbox: messages carrying the same meeting (`same_meeting` on the
   calendar's UID, so Exchange's `UID#start` matches), with method `REQUEST` (answer) or `REQUEST` and
   `CANCEL` (cancellation removed). Plus the message the action was taken in, wherever it is, except in
   Deleted Items, Sent Items, Drafts or the Outbox.
2. **Which occurrences.** An answer or cancellation of the whole meeting takes every invitation of it. One
   naming some occurrences only (`RECURRENCE-ID` without the series) takes only the invitations whose
   parts all name some of those occurrences.
3. **Which versions.** A message's part is compared with the same part of the version dealt with: the
   series with the series, an occurrence with the same occurrence, else with the series. It goes only if
   its `SEQUENCE` is no higher. Not by `DTSTAMP`: the calendar's copy is stamped when written to the
   server, and with the organizer's clock ahead it looked older than its own invitation (upstream's
   finding in `25c6997b`).
   - Answered in the message: the version dealt with is the message's calendar.
   - Answered in the calendar, or removed from it: the calendar's copy.
4. **Whose meetings.** Answered in the calendar, the user's own meetings are left alone: one whose
   organizer is one of the user's addresses. A meeting with no organizer known counts as someone else's.
   On Exchange a colleague's meeting has none here (the internal `EX` address isn't read as an address),
   and `own_organizer` already marks the user's own.
5. **Where to.** Each message goes to Deleted Items **beside it**, the way `move_selection` uses
   `folder_beside`: mail in another mailbox opened here goes to that mailbox's own Deleted Items.
6. **Next message.** When the message being read is among those moving, the list shows the one right
   after it that stays (else the one before), without marking it read. The anchor is the message read,
   not the lowest one moving: an earlier version further down doesn't take the list down there
   (`successor` today goes after the last one moving; upstream's `3c4febce`).

## Finding the invitations cheaply

Only messages already downloaded are looked at; nothing is fetched from the server for this. Upstream
reads the sources of the Inbox's newest 2000 messages, up to 64 MB, at every answer. On the owner's
mailbox that is a noticeable pause, so the summaries filter first:

- the Inbox's messages (newest first) whose subject contains the meeting's title (case aside, a title of
  at least 3 characters), or whose sender is the meeting's organizer;
- at most 200 such candidates, at most 16 MB of sources read, none bigger than 8 MB;
- a source without `text/calendar` or `.ics` in it isn't parsed.

A renamed meeting's older invitations are still found by their sender. They are missed only when the
organizer isn't known (an Exchange colleague's meeting, rule 4); they then stay, as today.

## Exchange

- The server is documented to take the invitation out of the Inbox itself when it's answered
  ([Microsoft, Exchange ActiveSync](https://learn.microsoft.com/en-us/previous-versions/office/developer/exchange-server-interoperability-guidance/hh428685(v=exchg.140)),
  from the message or the calendar), and the EWS Managed API describes `CalendarActionResults.MeetingRequest`
  as the request an accept moved to Deleted Items. Quatrelle answers on the calendar's item
  (`AcceptItem` … `ReferenceItemId`); whether EWS then moves the request too isn't documented. So the
  server may move a message Quatrelle is moving too.
- **A move of a message no longer on the server is not an error** on Exchange: `MoveItem` answering
  `ErrorItemNotFound` for some items moves the others and returns success, and the folder is synced
  again so the message shows where it went. Today it shows "The server refused a change: not on the
  server any more".
- **The calendar's copy gets its version**: `calendar:AppointmentSequenceNumber` is read with the other
  appointment fields (`APPOINTMENT_FIELDS`) into `CalEvent::sequence`. Exchange's calendar copies have
  `sequence` 0 here today, which makes every update look newer than the calendar (rule 3 would move
  nothing) and lets an older invitation be answered over a newer calendar. It is read-only: nothing
  writes it back. Appointments not read again since keep 0 until they change; their invitations then
  stay, as today.
- Microsoft Graph has no sequence for events: its copies keep 0, so from the calendar only first
  invitations (`SEQUENCE:0`) go. Accepted as is.
- Never tried on a real Exchange: to check on the owner's server.

## Code

- `engine/src/session/invite/tidy.rs` (new; `invite.rs` declares `mod tidy;`, the `session/` folder is
  at 16 files): `tidy_meeting_mail(account, uid, methods, only, upto, acted_on) -> usize` and its
  helpers (`carries_meeting`, the successor anchored at the message read), the constants above.
- `invite.rs`: `respond_to_invite` uses it in place of `move_selection` (and closes the window as
  today); `respond_to_event` calls it for someone else's meeting; `apply_invite`'s `CANCEL` calls it
  after the removal. Each returns `"moved": n` in its result.
- `prefs.rs`: `tidy_answered_invites: bool`, `#[serde(default = "yes")]`: on for new and saved settings
  alike, since today it's always on.
- A command for the switch, `view.tidyAnsweredInvites` (`{"on"?: bool}`, toggles without), beside
  `view.signatureDashes`.
- `ui-egui/src/panes/backstage/options.rs`: the switch in the Calendar group; the two strings in every
  catalog of `ui-egui/src/i18n/`.
- `net/src/exchange/ews/changes.rs`: the move tolerating items gone. `net/src/exchange/ews_pim.rs` and
  `ews_pim/events.rs`: the sequence read.

## Tests

Engine (`engine/src/tests/invites.rs`, or `tests/calendar/answered.rs` if it grows past ~800 lines):

1. Answered in the message: it and an earlier version go to Deleted Items; a newer update stays; the
   list shows the message after the one answered, not marked read.
2. Answered in the calendar (`event.respond`): the meeting's invitations go; the message being read
   stays when it isn't one of them; the user's own meeting moves nothing.
3. An occurrence answered: only invitations to that occurrence go.
4. "Remove from calendar" on a cancellation: the cancellation and the invitations go.
5. Switch off: nothing moves, the window still closes.
6. Undo puts them back.
7. The candidates filter: an invitation under another subject from the organizer is found; a message
   with neither the title nor the organizer isn't read.
8. The existing `a_group_invitation_shows_its_answer_and_goes` stays green.

Net: `fake_ews` — a move of two messages, one already gone, moves the other and is no error; an
appointment read carries its `AppointmentSequenceNumber`.

`every_command_survives_junk_params` covers the new command. `cargo xtask ci` before each commit.

## Left out

- Guests' answers to the user's own meetings (upstream's `answered_replies`, off by default there).
- A folder of the user's choosing per account (upstream's third choice).
- A rule condition "is a meeting invitation": rules see a message's header as it arrives, and an
  invitation shows only in its body (upstream's finding).
- Downloading a message's source just to know whether it is an invitation.
