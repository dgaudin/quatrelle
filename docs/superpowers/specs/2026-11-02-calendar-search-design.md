# Calendar: search every calendar

Status: the owner confirmed on 2026-11-02 that this feature is taken whole from upstream ZIDAN44
(issue #33: `9539f80e`, with `54f09098`'s join link), as the first of the four calendar features of
v0.31 (`~/mailcraft/merge-tools/survey-v0.31.md`, « Décidé le 02/11 »). It is adapted to this fork's
calendar (its look, its `private` / `show_as` fields), not picked: upstream's commit builds on its own
`Privacy` type and `ical/showing.rs`, which this fork doesn't have. `d9135bea` (results mirrored for
right-to-left languages) is left out with the rest of upstream's right-to-left work.

## Why

Finding an appointment means paging through weeks. Outlook's search box searches where you are: in the
Calendar it finds appointments, and lists them in place of the days. Quatrelle's box searches mail,
people, tasks and notes, but not the calendar: in the Calendar it searches mail.

## What the owner sees

In the Calendar, the search box at the top reads "Search calendar (location:, organizer:, after:…)".
Typing in it replaces the days with a list of the appointments found:

- a bar at the top: **All**, **Upcoming**, **Past**, and a menu of where to look: **All calendars** (every
  calendar ticked in the side pane, as on screen) or one calendar, any of them, ticked or not;
- under All, the upcoming ones first (soonest first) under an "Upcoming" heading, then the past ones (latest
  first) under "Past";
- each row: the calendar's colour (or its first category's) as a strip, the date, the time (or "All day"),
  the subject in bold, a lock when it is private, a repeat mark when it is a series; under the subject the
  place, the categories and the calendar's name;
- "No events match." with the calendar icon when nothing is found.

Clicking a row empties the box, goes to that day in the view the calendar was in (day, week, month), and
opens the appointment at that occurrence. Escape in the box, or its × button, empties it and brings the
days back. The query stays when the owner switches to Mail and back, as the People and Notes queries do.

A series is found once: at its next occurrence that hasn't ended, or its last one when it's over (under
Past). An occurrence changed on its own that matches when its series doesn't is found by itself.

## The query

As the mail search box reads it (the same tokenizer, `split_tokens_with`):

- bare words match anywhere as word starts (`budg` finds "Budget"); `"quoted phrases"` as written;
- fields: `subject:` (`title:`), `location:` (`where:`, `room:`), `notes:` (`body:`, `description:`),
  `organizer:` (`organiser:`, `from:`), `attendee:` (`attendees:`, `to:`, `cc:`, `with:`), `category:`
  (`categories:`); a list after a field, `organizer:ana,jun`, is any of them;
- `is:private` (`is:confidential`), `is:recurring` (`is:series`);
- `after:` and `before:` a date (`2026-10-22`, `2026/10/22`, `20261022`), on the owner's days: the
  occurrences starting on or after it, before it;
- a leading `-` turns any term round.

The place holds the appointment's join link too (`join_url`, Teams' link from Exchange or Graph), so
`teams.microsoft.com` finds a Teams meeting whose place is empty. `CONFERENCE` lines come with feature 4
(online rooms), which adds them to the place searched and ports `54f09098`'s test.

Private means the fork's `CalEvent::private` (any of its kinds). Others' private appointments are found by
what the server gave for them (Exchange hides their subject itself).

The notes searched are cut at 64 KiB per appointment; at most 64 terms and 50 list items are read; a date
that can't be read is looked for as words. Nothing in a query can panic.

## The command

`calendar.search {"query", "when"?: "all"|"upcoming"|"past", "account"?, "calendar"? (id or name),
"limit"?}`: the same search for the CLI and agents. It answers each appointment found: uid, account,
calendar, summary, location, start, end, allDay, occurrence, recurring, upcoming, categories, private,
showAs. At most 1,000.

## Speed

The search runs on the window's thread, over what is in memory: while the calendar is shown that includes
others' calendars (25,000 appointments in the owner's organisation, 27/10). It runs again only when the
query, the switches, the calendars (`pim_revision`) or the minute change; between those, the list is kept.
A series' occurrences are looked for a year ahead first, then twenty years; behind, a year, then twenty,
then all.

A test holds 25,000 appointments (200 daily series among them) and checks that one search answers in under
a second in a debug build. A search that takes 50 ms or more is said in the log (at most every ten
seconds), as a slow calendar frame is.

## Where it goes

| Layer | File | What |
|---|---|---|
| model | `ical/find.rs` (new) | `EventQuery`, `EventTerm`, `EventField`, `Needle`: parse and match |
| model | `search.rs` | `split_tokens_with` (the tokenizer, told which fields it knows) |
| engine | `session/pim/find.rs` (new) | `find_events`, `EventHit`, `EventWhen`, `EVENT_HITS_MAX` |
| engine | `cmd/pim.rs` | `calendar.search` |
| ui | `panes/calendar/side.rs` (new) | the side pane, moved out of `calendar.rs` first (807 lines) |
| ui | `panes/calendar/found.rs` (new) | `CalSearch`, the results, `open_hit` |
| ui | `app/bars.rs` | the box searches the Calendar there |
| ui | `lib.rs` | `UiState::cal_search` |
| ui | `i18n/*.tsv` | the new strings in all 13 catalogs |

`EventHit` carries `private: bool` and `show_as: ShowAs` (the fork's) instead of upstream's `Privacy`.

## Tests

- model (`ical/tests/find.rs`): upstream's, adapted: words, phrases, fields, lists, `is:`, dates, `-`,
  odd and junk queries, a 200,000-word note; the join link found in the place and not in the notes.
- engine (`tests/calendar/find.rs`): upstream's, adapted: a series once at its next occurrence, its last
  when over, a changed occurrence alone, the order under All, Upcoming and Past, `after:` / `before:` on
  the owner's days, hidden calendars left out unless asked for, one calendar by id or by name, the limit,
  the command's answer; and the 25,000-appointment timing.
- `every_command_survives_junk_params` runs `calendar.search` with junk.
- ui: the box in the Calendar searches the calendar, not mail; a result opens its appointment on its day
  and empties the box; Escape empties it; `pim_tour` draws results under each switch and "No events match."
- i18n: the catalogs test fails on an untranslated string.

## Left out

- Right-to-left mirroring of the results (`d9135bea`), with upstream's right-to-left work.
- Outlook's advanced calendar search form (fields to fill in): the query fields do it.
- Searching appointments on the server (Exchange's `FindItem` with a restriction): what is kept here is
  searched, as Outlook in Cached Exchange Mode does; the owner's own calendar is kept from 6 months back.
