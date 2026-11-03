# Calendar: Private and Show As, seen and set where Outlook has them

Status: the third of the four calendar features of upstream ZIDAN44 v0.31
(`~/mailcraft/merge-tools/survey-v0.31.md`, « Décidé le 02/11 »), issue #32. Confirmed by the owner on
2026-11-02 as gap-filling, not a port: the fork already has Private (`CalEvent::private`,
`PrivateKind`), the five Show As states (`ShowAs`), both in the appointment's window and on EWS and
CalDAV. Upstream's `5fa58c50` (which would replace that model), `1ae7e345`, `418faa02` and `6d1ccb3c`
are the reference. The owner's two requests of 2026-11-02 evening come with it: **the lock in the
calendar's grid** and **a right-click menu on an appointment**. Design approved on 2026-11-03 with two
choices: the menu has every entry proposed, and the grid draws Show As as Outlook does.

## Why

- A private appointment shows its lock only in its own window: in the day, week and month views it
  looks like any other.
- Show As is set in the window and kept on the server, but the grid draws only "tentative" (hollow, for
  an unanswered invitation or a `STATUS:TENTATIVE`), so a free or out-of-office appointment looks busy.
- Nothing happens on a right click on an appointment; deleting, answering or making one private means
  opening it.
- Exchange Online (Microsoft Graph) neither reads nor writes either property (`graph_pim.rs`): an
  appointment made private here goes up public, and one made private in Outlook comes back public.
- An Outlook invitation answered here keeps showing as tentative (`1ae7e345`): Outlook sends the
  guest's copy with `X-MICROSOFT-CDO-BUSYSTATUS:TENTATIVE` and says beside it what the organizer meant
  (`X-MICROSOFT-CDO-INTENDEDSTATUS`, usually busy). `set_answer` changes only the user's answer, so on
  IMAP + CalDAV the meeting stays tentative for good, and on Exchange until the next sync.
- The answer sent to the organizer carries alarms (`6d1ccb3c`, here in another form): `send_reply`
  keeps the event's `reminder` (written out as a `VALARM`, which RFC 5546 doesn't allow in a `REPLY`)
  and takes only the `BEGIN:VALARM` line of an alarm kept among the extra lines, so its `TRIGGER`,
  `ACTION` and `END:VALARM` go to the organizer, a component ended that never began.

`418faa02` (a new event starts in the account's default calendar) has nothing to fix here: it builds
on upstream's chosen default calendar (#30), not taken; the fork's event form already starts on the
account's first writable calendar, which is where invitations and `event.create` go
(`Pim::default_calendar`).

## What the owner sees

**In the grid** (Day, Work week, Week, Month, the all-day strip included):

- The strip at the appointment's left says how its time shows, as Outlook draws it: **Busy** solid in
  its colour (today's look), **Tentative** hatched, **Free** white with its colour's outline, **Out of
  office** purple, **Working elsewhere** dotted. The body stays as today: in its colour, hollow for an
  unanswered invitation or a meeting whose own status is tentative (`STATUS:TENTATIVE`).
- A **private** appointment has the Fluent lock (`Icon::Lock`) in its top right corner, the title
  stopping short of it. One with no title (a colleague's private appointment, given without its
  subject) reads "Private appointment" in the grid and in its card, not a blank box.

**On a right click** on an appointment, anywhere in the grid, a menu (our own wording, translated):

1. **Open**; **Join meeting** when it has an online meeting link (`CalEvent::join_link`).
2. For a meeting someone else organizes, in a calendar the user can write: **Accept**, **Tentative**,
   **Decline**, the current answer ticked; choosing another changes the answer (`event.respond`).
3. **Show As ▸** the five states, in Outlook's order, the current one ticked; **Categorize ▸** the
   categories (as the window's Categorize), each ticked when the appointment has it; **Private**, a
   tick with the lock. These three don't tell the guests: `event.update` with `"send": false`. On a
   recurring appointment they change the series, as its window does.
4. **Delete**; on a series, a submenu **Delete this occurrence** / **Delete the series**, as the
   window has it (`event.delete`).

In a read-only calendar (a colleague's shared one, a subscribed one) the menu has only Open and Join.
A right click doesn't start a drag and doesn't open the appointment.

**Answering an invitation** (in its message, in its window, in the menu, or Exchange answering for
the user): accepted, it shows as the organizer meant it (`X-MICROSOFT-CDO-INTENDEDSTATUS`), else busy
when it was tentative, else as it was; tentatively accepted, tentative unless it's free; declined,
unchanged. The answer sent to the organizer has no alarm.

**Exchange Online**: an appointment's Private and Show As go up with every change and come back as
Outlook set them (Personal and Confidential kept, as on EWS).

## Design

### Model (`crates/model`)

- `ical/meeting.rs`: `ShowAs::graph()` gives Graph's names (`free`, `tentative`, `busy`, `oof`,
  `workingElsewhere`); `ShowAs::from_cdo` already reads them, any case (`unknown` reads as nothing,
  so busy). `PrivateKind::from_sensitivity` already reads Graph's lower-case names; Graph is written
  `sensitivity().to_ascii_lowercase()`, `normal` when not private.
- `ical/meeting.rs`: `pub fn intended_show_as(e: &CalEvent) -> Option<ShowAs>` reads the first
  `X-MICROSOFT-CDO-INTENDEDSTATUS` among `e.extra` (it stays there, kept as written);
  `pub fn show_as_answered(e: &mut CalEvent, partstat: &str)` applies the rule above to `e.show_as`.

### Engine (`crates/engine`)

- `session/invite.rs` `set_answer` ends with `ical::show_as_answered(e, stat)`: the one place every
  answer goes through (`answer_event`, `answer_by_server`, `settle_answers`, the invitation's own).
- `send_reply`: `ical::set_reminder(&mut reply, None)` in place of the `retain` (every alarm goes whole,
  the reminder too).
- `AgendaItem` gains `private: bool`, `show_as: ShowAs`, and `answer: Option<String>`: the user's
  answer (`ACCEPTED`, `TENTATIVE`, `DECLINED`, `NEEDS-ACTION`: `my_answer`, else the partstat of the
  user's own guest line) when it's a meeting someone else organizes in a calendar the user can write,
  `None` otherwise. `calendar.events` gives `private`, `showAs` and `answer`.
- `event.update` takes `"send"?: bool` (default true), as `event.move` does: false, the guests of the
  user's own meeting aren't told (`tell_guests(…, send)`).

### Exchange Online (`crates/net/src/exchange/graph_pim.rs`)

- `EVENT_FIELDS` adds `sensitivity,showAs`. `event_of` sets `private`, `private_kind` and `show_as`
  from them; `event_json` writes both on every create and update.

### Interface (`crates/ui-egui/src/panes/calendar*`)

- `calendar.rs` `chip`: the left strip by `item.show_as` (a small painter helper: solid, hatched with
  diagonal lines, white with an outline, purple, dots), the lock when `item.private`, and the
  "Private appointment" title for an empty private one (a helper the day, week and month views and
  the card share). The purple is the category palette's (`theme::category_color(4, t.dark)`).
- New `panes/calendar/menu.rs`: `pub(super) fn menu(ui, s, item, acts)`, attached in `chip` with
  `resp.context_menu` so every view has it. It reads the event's categories and join link with
  `Session::find_event` only while the menu is open. Its entries push `Action::Dialog` (Open),
  `Action::OpenMeeting` (Join) and `Action::Submit` / `Action::Run` with the commands above.
- Every new string through `tl!`, in every catalog.

## Testing

- Model: `show_as_answered` (accepted with and without an intended status, from tentative and from
  free; tentative; declined), `intended_show_as` with junk, Graph names both ways.
- Engine: an Outlook invitation (`BUSYSTATUS:TENTATIVE`, `INTENDEDSTATUS:BUSY`) accepted on an IMAP +
  CalDAV account shows as busy, and the event written back says so once; the reply sent to the
  organizer has no `VALARM`, with a reminder and with an unreadable alarm; `event.update` with
  `"send": false` on the user's own meeting sends nothing; `AgendaItem` carries `private`, `show_as` and
  `answer`; `every_command_survives_junk_params` stays green.
- Graph: on the fake Graph server, an event made private and out of office is stored with
  `sensitivity: private` and `showAs: oof`, then confidential and free after an update, and reads back
  the same.
- Interface: the headless frames render a private appointment and one of each Show As in the week and
  month views; a right click on an appointment opens the menu with the expected entries (fewer for a
  read-only calendar), and choosing Private submits `event.update` with `private` and `send: false`.
- Then the real window (`--sample`, light and dark), screenshot read, and a test build.

## Left out

- Graph sends every guest as required: an optional guest becomes required on Exchange Online
  (`event_json`). Seen on the way; not this feature.
- Forward, Print, Propose New Time in the menu: Quatrelle has no command for them on an appointment.
- Upstream's chosen default calendar (#30) and its `418faa02`.
