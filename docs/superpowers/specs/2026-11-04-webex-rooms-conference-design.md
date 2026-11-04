# Calendar: several Webex rooms, and CONFERENCE read and written

Status: the fourth of the four calendar features of upstream ZIDAN44 v0.31
(`~/mailcraft/merge-tools/survey-v0.31.md`, « Décidé le 02/11 »), issue #25, in the 1.1 scope the owner
set on 2026-11-04. Upstream's `e838af15` is the reference for `CONFERENCE`; `15d37985`, `f85d124f` and
`f98b31db` (upstream's generic rooms, form and Options card) are the reference, not ported. Designed
with the owner on 2026-11-04: **several Webex rooms**, each **a name, its link and free lines**; the
rooms in the Webex Meeting button's menu, kept in Options › Calendar; `CONFERENCE` read and written.

## Why

- Quatrelle keeps one Webex personal room (`Prefs::webex_room`): someone who hosts in more than one
  (their own, a team's, a person's they organize for, another site's) types the others by hand.
- Choosing the personal room on an appointment that already has a scheduled Webex meeting keeps both
  (noted on 2026-11-02 after the scheduled meetings).
- Quatrelle neither reads nor writes RFC 7986's `CONFERENCE` (§5.11): an invitation whose meeting is
  only there (another calendar app, a CalDAV server) has no Join, and Quatrelle's own meetings on
  CalDAV, local calendars and invitations sent from an IMAP account don't tell other apps where to join.
- The calendar search (feature 1) was promised the `CONFERENCE` lines.

## What the owner sees

**File › Options › Calendar.** « Salles Webex » replaces « Salle personnelle Webex »: one row per room
with its name, its link, a box for its free lines and a ×, then « Ajouter une salle ». A room is:

- a name, at most 80 characters, required for a new room; the room carried over from the single field
  has none and shows as « Ma salle personnelle » until it's named;
- its link, a Webex personal room (`https://<site>.webex.com/meet/<name>`, `ical::is_webex_link`, as
  today), at most 2 048 bytes; two rooms never share a link;
- free lines (meeting number, video address, dial-in numbers…), at most 20 lines of 200 characters,
  copied as they are.

At most 20 rooms. A wrong link says why under its row and stays to be corrected, as today.

**The Webex Meeting button's menu** (appointment and meeting windows):

- « Nouvelle réunion Webex » (the scheduled meeting, as today);
- a separator, then one line per room, its name (its link on hover);
- « Ajouter une salle… »: the name and the link asked in the menu, as the first room is asked today;
  the free lines are added in Options.

While the appointment has a scheduled Webex meeting (`webex_id`), the rooms are greyed, their hover
« Ce rendez-vous a déjà sa réunion Webex ». A click on the button itself still schedules a meeting. In
the browser (no scheduled meetings), a click with exactly one room adds it; with none or several it
opens the menu (the rooms and « Ajouter une salle… »).

**What a room adds:** its link as the place when the place is empty (a place typed is kept); at the end
of the notes « Rejoindre la réunion Webex : », the link, then its free lines; Join shows. Choosing
another room replaces the previous room's lines rather than adding a second block. The previous block
is found as Quatrelle wrote it: the « Rejoindre la réunion Webex : » line (in any of the catalogs'
languages, the interface's may have changed since), the room's link on the next line, then the lines
after it that are still among that room's free lines; a line that matches none of these (the room's
lines edited in Options since, text the user typed) stays. The appointment gets the room as its
`CONFERENCE` (below).

**Join** takes, in order: the server's own link (Exchange's, a known meeting service); the
`CONFERENCE` link, any web link, a self-hosted one included, since the calendar says it joins; the
first known meeting link in the place or the notes, as today.

## CONFERENCE

**Read** (every iCalendar Quatrelle reads: CalDAV, local calendars, invitations by mail, `.ics` files):
the first `CONFERENCE` line whose value is a web link Quatrelle may open (`launchable_link`: http or
https, a plain host, no user name, at most 2 048 bytes) and whose `FEATURE`, when given, isn't only
`PHONE`, `CHAT` or `FEED` becomes the event's conference, with its `LABEL` (at most 80 characters,
its quotes taken off, control characters left out) and its features. Parameters in any order (Stalwart reorders them), `VALUE=URI` or none.
Every other `CONFERENCE` line (a `tel:` one, chat, a second one) stays in the event's kept lines and is
written back as it came.

**Written** wherever Quatrelle writes iCalendar (CalDAV, local calendars, invitations from an IMAP
account, `.ics` export): `CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=<label>:<link>`, the label
in double quotes when it holds a `,`, `;` or `:` (a parameter's value can't be escaped with a
backslash, RFC 5545 §3.2: `\;` would cut the line for every reader), its own double quotes and
control characters left out, and the line folded as every other. A room or a scheduled Webex meeting
sets it, label `Webex`. A conference read keeps its own label and features.

**Exchange** (EWS and Graph) has no place for it: nothing changes there; the link lives in the place
and the notes, where Join already finds it.

**Taken out** when the user removes it: the conference's link was in the place or the notes when the
window opened and is in neither when it's saved → no conference. A conference whose link was never
there (another app's) stays whatever else is changed. `event.create` and `event.update` take
`"conference"?: {"url": string, "label"?: string} | null` (`null` takes it out).

**Search:** the calendar search reads the conference's label and link with the place
(`ical/find.rs`, beside `join_url`).

## Design

- **Model.** New `ical/conference.rs`: `pub struct Conference { pub url: String, pub label: String,
  pub features: Vec<String> }`; `CalEvent::conference: Option<Conference>` (serde default, skipped when
  none); reading a `CONFERENCE` line and writing one. `ical.rs` is at 828 lines: its text helpers
  (`escape`, `unescape`, `split_text_list`, `duration`) move first to `ical/text.rs`, re-exported, so
  the parse arm and the write call fit. `join_link` takes the order above. `find.rs` reads the label
  and link.
- **Engine.** `Prefs::webex_rooms: Vec<WebexRoom { name, url, details }>` (`details` the free lines);
  an old settings file's `webex_room` becomes the first room, unnamed, and isn't written again.
  `calendar.webexRoom {"url", "name"?, "details"?, "remove"?: bool}` adds a room, changes the one with
  that link, or removes it, and returns the rooms; an empty link is refused now (the Options rows
  remove rooms one by one). `event.create` / `event.update` carry `conference`.
- **Interface.** The menu's rooms, the inline add and the replacing of a room's lines in a new
  `dialogs/pim_dialogs/webex_rooms.rs` (`meeting.rs` is at 647 lines; `rooms.rs` is already the
  Room Finder, the server's meeting rooms); the scheduled meeting's
  `pim_dialogs/webex.rs` sets the conference too; the Options rows in a new
  `panes/backstage/options/webex_rooms.rs`. The window remembers the conference link it opened with,
  for the taking-out rule.
- **Left from upstream**: rooms of other services (BigBlueButton, Jitsi, OpenTalk), its
  `---- Online meeting ----` block with markers, the meeting hosts' list, « add an online meeting to all
  meetings with guests », its Online meetings card, its `meetings.*` commands, Teams through Graph.

## Robustness

A `CONFERENCE` line comes from strangers; rooms are typed by hand.

- Caps: link 2 048 bytes, name and label 80 characters, 20 free lines of 200 characters, 20 rooms; a
  hand-edited settings file is cut the same way when read.
- No panics; char-boundary cuts; the label's quoting, and folding as the rest of the writer.
- The `ical` fuzz target gets `CONFERENCE` lines (hostile values, odd parameters, `tel:`, `javascript:`,
  a user name in the host); a property test: a conference written then read is the same.
- `every_command_survives_junk_params` runs `calendar.webexRoom` and `event.*` with `conference`.

## Tests

- **Model:** reading (parameters in any order, `VALUE` missing, a label with a comma, `tel:`, chat and
  phone-only lines left in the kept lines, `javascript:` and `https://user@host` refused); writing
  (the label quoted, folding, kept lines written once); Join's order; the search finds the label; the text
  helpers' move changes nothing (the existing `ical` tests).
- **Engine:** `calendar.webexRoom` add, change, remove, 21st room, 81-character name, 21 lines, an empty
  or non-Webex link, junk; an old `webex_room` setting carried over; `event.update` with and without
  `conference`, `null`; a CalDAV round trip through `fakes/fake_dav.rs` keeps the conference; an
  invitation from an IMAP account carries the line; an Exchange event pushes no such thing.
- **Interface, headless:** the menu lists the rooms by name, greyed with a scheduled meeting; a room
  fills the place and the notes, another replaces its lines (also when the label was written in
  another language, and leaving a line the user typed or a free line edited since); the link removed from the notes takes the
  conference out on save, a foreign conference stays; the Options rows (add, edit, ×, the unnamed
  room's name).
