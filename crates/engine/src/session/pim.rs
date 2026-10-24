//! Calendar and People: each account's events and contacts, kept in its cache (`pim.json`).
//! Accounts whose server offers CalDAV / CardDAV sync them (see `net::dav`); the others, and the
//! sample mailbox, keep a calendar and an address book on this computer.

use quatrelle_model::dates::{civil_from_days, days_from_civil};
use quatrelle_model::ical::{self, CalEvent};
use quatrelle_model::pim::{LOCAL, Pim, StoredCard, StoredEvent, Tombstone};
use quatrelle_model::vcard::Card;

use super::*;

pub use quatrelle_model::pim::PIM_DOC;

mod deleting;
mod moving;
mod notes;
mod parking;
mod people;

/// Whether a calendar is the user's own: not a colleague's shared one, nor a public folder's.
pub(super) fn own_calendar(id: &str) -> bool {
    quatrelle_model::pim::shared_calendar_owner(id).is_none() && !id.starts_with("public:")
}
pub use notes::NoteRef;
pub use people::Person;

/// The part of the app on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Module {
    #[default]
    Mail,
    Calendar,
    People,
    /// Outlook's Notes.
    Notes,
}

/// How much of the calendar is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CalMode {
    Day,
    /// Monday to Friday (Outlook's Work Week).
    WorkWeek,
    #[default]
    Week,
    Month,
}

/// A reminder whose time has come (File › Reminders, as Outlook's Reminders window lists them).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DueReminder {
    /// Names it for snoozing and dismissing: `<account>|<uid>|<occurrence>`.
    pub key: String,
    pub account: String,
    pub uid: String,
    pub occurrence: i64,
    pub summary: String,
    pub location: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    /// When it was due (its start, less its reminder; or the end of a snooze).
    pub due: i64,
}

/// How long after an event has ended its reminder still waits to be answered.
const REMINDER_KEPT_AFTER_END: i64 = 86_400;
/// Most reminders listed at once.
const REMINDERS_MAX: usize = 200;

/// One occurrence of an event, ready to draw. Times are Unix seconds; all-day events start at
/// local midnight of their first day.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgendaItem {
    pub account: String,
    pub calendar: String,
    pub uid: String,
    /// When this occurrence starts as the event's rule has it (names it for edits).
    pub occurrence: i64,
    pub recurring: bool,
    pub summary: String,
    pub location: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub color: u8,
    /// The colour of its first category (an index into the category palette), which Outlook
    /// draws an appointment in rather than its calendar's.
    pub category: Option<u8>,
    /// `TENTATIVE`, `CANCELLED`…
    pub status: String,
    /// The user's reply is awaited (`NEEDS-ACTION`).
    pub needs_reply: bool,
    /// Its reminder, in minutes before the start (a negative one after it); `None`: none.
    pub reminder: Option<i64>,
    /// It can be dragged to another time: its calendar can be changed, and it's the user's own
    /// (not a meeting someone else organizes).
    pub movable: bool,
    /// A meeting the user organizes, with guests to tell of a change.
    pub organized: bool,
}

/// A calendar colour of this or above is an account's colour ([`Session::calendar_color`]).
pub const ACCOUNT_COLOR: u8 = 16;

/// An event's reminder, in minutes before its start: its own field (`CalEvent::reminder`, which
/// its `VALARM` is read into and written from, and which Exchange reads and writes); or, for one
/// made elsewhere with an alarm still among its `extra` lines, that alarm.
fn reminder_of(e: &CalEvent) -> Option<i64> {
    e.reminder.or_else(|| ical::reminder_minutes(e))
}

impl Session {
    pub(crate) fn load_pim(&mut self) {
        let Some(store) = self.store.clone() else { return };
        for a in &self.accounts {
            let mut p: Pim = store.load_account_doc(&a.config.id, PIM_DOC).unwrap_or_default();
            p.ensure_defaults();
            // Other people's appointments not among the calendar's: set aside when it was last
            // left (brought back when it's shown; see `parking.rs`).
            if p.events.iter().all(|e| own_calendar(&e.calendar)) {
                self.parked.insert(a.config.id.clone());
            }
            self.pim.insert(a.config.id.clone(), p);
        }
        // Away from the calendar from the start: set aside at once.
        self.away_from_calendar = Some(self.clock.now.saturating_sub(parking::PARK_AFTER));
    }

    pub(crate) fn save_pim(&mut self, account: &str) {
        self.forget_contacts();
        self.pim_revision = self.pim_revision.wrapping_add(1);
        let (Some(store), Some(p)) = (&self.store, self.pim.get(account)) else { return };
        if let Err(e) = store.save_account_doc(account, PIM_DOC, p) {
            self.notice(format!("calendar and contacts could not be saved: {e}"), true);
        }
    }

    /// An account's calendars and address books (made on first use).
    pub fn pim_mut(&mut self, account: &str) -> &mut Pim {
        self.pim.entry(account.to_string()).or_insert_with(|| {
            let mut p = Pim::default();
            p.ensure_defaults();
            p
        })
    }

    /// The account new events and people go to: the one on screen, else the first.
    pub(super) fn pim_account(&self, wanted: Option<&str>) -> Result<String, CmdError> {
        match wanted {
            Some(a) if self.account(a).is_some() => Ok(a.to_string()),
            Some(a) => Err(CmdError::Params(format!("no account `{a}`"))),
            None => self.current_account().map(|a| a.config.id.clone()).ok_or_else(|| CmdError::Disabled("add an account first".into())),
        }
    }

    // ----- the calendar view -----

    /// The local day (days since the epoch) the calendar shows.
    pub fn cal_day(&self) -> i64 {
        self.cal_day.unwrap_or_else(|| quatrelle_model::dates::today(&self.clock))
    }

    /// The local days on screen: a day, the working days or the whole week (Monday first), or the
    /// weeks of the month.
    pub fn cal_days(&self) -> (i64, i64) {
        let d = self.cal_day();
        let monday = |x: i64| x - (x + 3).rem_euclid(7);
        match self.cal_mode {
            CalMode::Day => (d, d + 1),
            CalMode::WorkWeek => (monday(d), monday(d) + 5),
            CalMode::Week => (monday(d), monday(d) + 7),
            CalMode::Month => {
                let (y, m, _) = civil_from_days(d);
                let first = monday(days_from_civil(y, m, 1));
                (first, first + 42)
            }
        }
    }

    /// The Unix time a local day starts.
    pub fn local_midnight(&self, day: i64) -> i64 {
        self.clock.midnight(day)
    }

    /// Move the calendar: to a date, today, or by `step` days, weeks or months (its mode).
    pub fn cal_goto(&mut self, date: Option<&str>, step: i64) -> CmdResult {
        let mut d = self.cal_day();
        match date.map(str::trim) {
            Some("today") | Some("") => d = quatrelle_model::dates::today(&self.clock),
            Some(s) => {
                d = parse_local(s, &Clock::default())
                    .map(|(t, _)| t.div_euclid(86_400))
                    .ok_or_else(|| CmdError::Params(format!("`{s}` is not a date (YYYY-MM-DD)")))?;
            }
            None => {}
        }
        let step = step.clamp(-1200, 1200);
        d = match self.cal_mode {
            CalMode::Day => d + step,
            CalMode::Week | CalMode::WorkWeek => d + 7 * step,
            CalMode::Month => {
                let (y, m, day) = civil_from_days(d);
                let total = i64::from(m) - 1 + step;
                let (yy, mm) = (y + total.div_euclid(12), u32::try_from(total.rem_euclid(12) + 1).unwrap_or(1));
                days_from_civil(yy, mm, day.min(28))
            }
        };
        self.cal_day = Some(d);
        let (y, m, dd) = civil_from_days(d);
        Ok(serde_json::json!({"date": format!("{y:04}-{m:02}-{dd:02}"), "mode": self.cal_mode}))
    }

    /// The key a calendar is shown or hidden under (`Prefs::hidden_calendars`).
    pub fn calendar_key(account: &str, calendar: &str) -> String {
        format!("{account}/{calendar}")
    }

    /// Whether a calendar is shown (ticked) in the calendar.
    pub fn calendar_shown(&self, account: &str, calendar: &str) -> bool {
        !self.prefs.hidden_calendars.contains(&Session::calendar_key(account, calendar))
    }

    /// Calendars come ticked when they're an account's own or opened here by address; others'
    /// that come with Outlook's calendar groups, and public folders', come unticked the first
    /// time they're seen (a dozen colleagues' weeks at once are slow to draw and hard to read),
    /// as Outlook leaves them until they're ticked. Run after a sync and once the settings are
    /// read (the calendars kept on this computer were seen before the settings came).
    pub fn note_new_calendars(&mut self) {
        let seen: Vec<(String, bool)> = self
            .pim
            .iter()
            .flat_map(|(a, p)| p.calendars.iter().map(move |c| (Session::calendar_key(a, &c.id), !c.group.is_empty() || c.id.starts_with("public:"))))
            .collect();
        for (key, others) in seen {
            if self.prefs.known_calendars.insert(key.clone()) && others {
                self.prefs.hidden_calendars.insert(key);
            }
        }
    }

    /// What each Exchange account's next calendar sync reads, as Outlook does: away from the
    /// calendar, its own calendar only; in it, the calendars ticked. Opening the calendar, or
    /// ticking one, reads what wasn't read at once.
    pub(crate) fn update_calendar_reading(&mut self) {
        self.park_calendars();
        let own_only = self.module != crate::Module::Calendar;
        let accounts: Vec<String> =
            self.accounts.iter().filter(|a| a.config.ews.is_some() || a.config.graph.is_some()).map(|a| a.config.id.clone()).collect();
        for a in accounts {
            let Some(p) = self.pim.get(&a) else { continue };
            let not_read: Vec<String> = p.calendars.iter().filter(|c| !self.calendar_shown(&a, &c.id)).map(|c| c.id.clone()).collect();
            if p.own_only == own_only && p.not_read == not_read {
                continue;
            }
            let more = (p.not_read.iter().any(|c| !not_read.contains(c)) || p.own_only) && !own_only;
            let p = self.pim_mut(&a);
            p.own_only = own_only;
            p.not_read = not_read;
            self.after_pim_change(&a, more);
        }
    }

    /// The calendars shown, in the order the side pane lists them: (account, id, name, colour).
    pub fn shown_calendars(&self) -> Vec<(String, String, String, u8)> {
        self.accounts
            .iter()
            .filter_map(|a| self.pim.get(&a.config.id).map(|p| (a, p)))
            .flat_map(|(a, p)| p.calendars.iter().map(move |c| (a.config.id.clone(), c.id.clone(), c.name.clone(), Session::calendar_color(a, c))))
            .filter(|(a, c, _, _)| self.calendar_shown(a, c))
            .collect()
    }

    /// The colour a calendar is drawn in (with its events): an account's own calendar in the
    /// account's colour, as its mail shows it, so each account's calendar can be told apart
    /// (they used to share one); its others in theirs. [`ACCOUNT_COLOR`] and above stand for
    /// an account's colour (`ACCOUNT_COLOR + AccountConfig::color`).
    pub fn calendar_color(a: &AccountState, c: &quatrelle_model::pim::CalendarInfo) -> u8 {
        if c.color == 0 { ACCOUNT_COLOR.saturating_add(a.config.color) } else { c.color }
    }

    /// The occurrences overlapping `[from, to)`, by start, from every calendar that's shown.
    /// (Each with its calendar's colour: [`Session::calendar_color`].)
    pub fn agenda(&self, from: i64, to: i64) -> Vec<AgendaItem> {
        let mine: Vec<String> =
            self.accounts.iter().flat_map(|a| [a.config.email.to_ascii_lowercase(), a.config.username.to_ascii_lowercase()]).collect();
        let mut out = Vec::new();
        for (account, p) in &self.pim {
            let Some(acc) = self.account(account) else { continue };
            for cal in p.calendars.iter().filter(|c| self.calendar_shown(account, &c.id)) {
                let stored: Vec<&StoredEvent> = p.events.iter().filter(|e| e.calendar == cal.id).collect();
                let events: Vec<CalEvent> = stored.iter().map(|e| e.event.clone()).collect();
                // All-day events are dates, not times: widen the window by a day each way.
                for o in ical::occurrences(&events, from - 86_400, to + 86_400) {
                    let Some(e) = events.get(o.event) else { continue };
                    let (start, end) = if e.all_day { (self.clock.utc(o.start), self.clock.utc(o.end)) } else { (o.start, o.end) };
                    if start >= to || end.max(start + 1) <= from {
                        continue;
                    }
                    let needs_reply = e.attendees.iter().any(|a| mine.contains(&a.email.to_ascii_lowercase()) && a.partstat == "NEEDS-ACTION");
                    let organizer_is_me = e.organizer.as_ref().is_some_and(|o| mine.contains(&o.email.to_ascii_lowercase()));
                    out.push(AgendaItem {
                        movable: !cal.read_only && (e.attendees.is_empty() || e.organizer.is_none() || organizer_is_me),
                        organized: !e.attendees.is_empty() && organizer_is_me,
                        account: account.clone(),
                        calendar: cal.id.clone(),
                        uid: e.uid.clone(),
                        occurrence: e.recurrence_id.unwrap_or(o.start),
                        recurring: e.rrule.is_some() || e.recurrence_id.is_some(),
                        summary: e.summary.clone(),
                        location: e.location.clone(),
                        start,
                        end,
                        all_day: e.all_day,
                        color: Session::calendar_color(acc, cal),
                        category: e.categories.first().map(|c| self.category_color_of(c)),
                        status: e.status.clone(),
                        needs_reply,
                        reminder: e.reminder,
                    });
                }
            }
        }
        out.sort_by(|a, b| (b.all_day, a.start, &a.summary).cmp(&(a.all_day, b.start, &b.summary)));
        out
    }

    /// The reminders due at `now`, soonest event first: occurrences of the user's own calendars
    /// (shown or hidden; not others' shared with them) whose reminder time has passed, that
    /// haven't ended a day ago, weren't dismissed, and aren't snoozed past `now`.
    pub fn due_reminders(&self, now: i64) -> Vec<DueReminder> {
        let ahead = ical::REMINDER_MAX_MINUTES * 60;
        let mut out = Vec::new();
        for (account, p) in &self.pim {
            if self.account(account).is_none() {
                continue;
            }
            for cal in p.calendars.iter().filter(|c| quatrelle_model::pim::shared_calendar_owner(&c.id).is_none()) {
                // Only the series with a reminder somewhere (an exception may have its own).
                let with: std::collections::BTreeSet<&str> =
                    p.events.iter().filter(|e| e.calendar == cal.id && reminder_of(&e.event).is_some()).map(|e| e.event.uid.as_str()).collect();
                if with.is_empty() {
                    continue;
                }
                let events: Vec<CalEvent> =
                    p.events.iter().filter(|e| e.calendar == cal.id && with.contains(e.event.uid.as_str())).map(|e| e.event.clone()).collect();
                for o in ical::occurrences(&events, now - REMINDER_KEPT_AFTER_END - 86_400, now + ahead + 86_400) {
                    let Some(e) = events.get(o.event) else { continue };
                    let Some(minutes) = reminder_of(e) else { continue };
                    if e.status.eq_ignore_ascii_case("CANCELLED") {
                        continue;
                    }
                    let (start, end) = if e.all_day { (self.clock.utc(o.start), self.clock.utc(o.end)) } else { (o.start, o.end) };
                    let occurrence = e.recurrence_id.unwrap_or(o.start);
                    let key = format!("{account}|{}|{occurrence}", e.uid);
                    let due = match self.prefs.reminders.get(&key) {
                        Some(0) => continue,
                        Some(until) => *until,
                        None => start - minutes * 60,
                    };
                    if due > now || end.max(start) < now - REMINDER_KEPT_AFTER_END {
                        continue;
                    }
                    out.push(DueReminder {
                        key,
                        account: account.clone(),
                        uid: e.uid.clone(),
                        occurrence,
                        summary: e.summary.clone(),
                        location: e.location.clone(),
                        start,
                        end,
                        all_day: e.all_day,
                        due,
                    });
                }
            }
        }
        out.sort_by(|a, b| (a.start, &a.summary, &a.key).cmp(&(b.start, &b.summary, &b.key)));
        out.dedup_by(|a, b| a.key == b.key);
        out.truncate(REMINDERS_MAX);
        out
    }

    /// Dismiss reminders: the one named by `key`, or every one due at `now` (`None`).
    pub fn dismiss_reminders(&mut self, key: Option<&str>, now: i64) -> usize {
        let keys: Vec<String> = match key {
            Some(k) => vec![k.to_string()],
            None => self.due_reminders(now).into_iter().map(|r| r.key).collect(),
        };
        for k in &keys {
            self.prefs.reminders.insert(k.clone(), 0);
        }
        self.forget_old_reminders(now);
        keys.len()
    }

    /// Snooze a reminder until `until` (Unix seconds).
    pub fn snooze_reminder(&mut self, key: &str, until: i64) {
        self.prefs.reminders.insert(key.to_string(), until.max(1));
        self.forget_old_reminders(now_or(until));
    }

    /// Answers for occurrences long past go (and the list is kept small).
    fn forget_old_reminders(&mut self, now: i64) {
        let cutoff = now - 30 * 86_400;
        self.prefs.reminders.retain(|k, _| k.rsplit('|').next().and_then(|t| t.parse::<i64>().ok()).is_none_or(|t| t >= cutoff));
        while self.prefs.reminders.len() > 5000 {
            let Some(first) = self.prefs.reminders.keys().next().cloned() else { break };
            self.prefs.reminders.remove(&first);
        }
    }

    /// A category's colour by its name: the one made with that name (mail and events share
    /// them), else one picked from the name (a category from the server Quatrelle has no colour
    /// for still gets the same one every time).
    pub fn category_color_of(&self, name: &str) -> u8 {
        if let Some(c) = self.prefs.categories.iter().find(|c| c.name.eq_ignore_ascii_case(name.trim())) {
            return c.color;
        }
        let h = name.trim().to_lowercase().bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
        u8::try_from(h % 8).unwrap_or(0)
    }

    /// The event (master, not an instance) with a uid, and its account.
    pub fn find_event(&self, uid: &str) -> Option<(&str, &StoredEvent)> {
        let (account, at) = self.find_event_index(uid)?;
        Some((account, self.pim.get(account)?.events.get(at)?))
    }

    /// Where the event a dialog names is: its account and its place in that account's list.
    ///
    /// An appointment read from Exchange carries its start after a `#` in its UID (several items
    /// can share a series' UID), and that start changes when the appointment moves. A dialog
    /// opened before a sync therefore still names the old one, and asking for it by that name
    /// gave "no event `…`" while the appointment was there all along: the name without its start
    /// finds it again, as long as only one appointment answers to it.
    ///
    /// A meeting is under one UID in the user's calendar and in each colleague's shared calendar
    /// that has it: the user's own copy is the one meant. The first one found was taken, a
    /// colleague's copy as often as not — moved, or deleted, in a calendar the user may change.
    pub(super) fn find_event_index(&self, uid: &str) -> Option<(&str, usize)> {
        let own = |e: &StoredEvent| own_calendar(&e.calendar);
        let exact = |e: &StoredEvent| e.event.uid == uid && e.event.recurrence_id.is_none();
        for mine in [true, false] {
            let hit = |e: &StoredEvent| exact(e) && own(e) == mine;
            if let Some(found) = self.pim.iter().find_map(|(a, p)| p.events.iter().position(hit).map(|at| (a.as_str(), at))) {
                return Some(found);
            }
        }
        // Only a name carrying its start (`…#<start>`) stands for an appointment whose start
        // may have changed: a series' own UID named every appointment read from it, and the
        // lone occurrence of a series gone found (and deleted) a colleague's copy of one.
        let (base, _) = uid.split_once('#')?;
        for mine in [true, false] {
            let mut found: Option<(&str, usize)> = None;
            for (account, p) in &self.pim {
                for (at, e) in p.events.iter().enumerate() {
                    if e.event.recurrence_id.is_some() || e.event.uid.split('#').next() != Some(base) || own(e) != mine {
                        continue;
                    }
                    // Two of them: which one the dialog meant can't be told, so nothing is guessed.
                    if found.is_some() {
                        return None;
                    }
                    found = Some((account.as_str(), at));
                }
            }
            if found.is_some() {
                return found;
            }
        }
        None
    }

    pub(super) fn new_uid(&mut self) -> String {
        let n = self.fresh_id();
        format!("{:016x}-{n}@quatrelle", self.seed)
    }

    /// Add an event; returns its uid. Events in a server calendar are sent there.
    pub fn create_event(&mut self, account: Option<&str>, calendar: Option<&str>, mut e: CalEvent) -> CmdResult {
        let account = self.pim_account(account)?;
        check_event(&e)?;
        if e.uid.trim().is_empty() {
            e.uid = self.new_uid();
        }
        let p = self.pim_mut(&account);
        let cal = match calendar {
            Some(c) => p
                .calendars
                .iter()
                .find(|x| x.id == c || x.name.eq_ignore_ascii_case(c))
                .ok_or_else(|| CmdError::Params(format!("no calendar `{c}`")))?,
            None => p.default_calendar().ok_or_else(|| CmdError::Disabled("no calendar to add to".into()))?,
        };
        if cal.read_only {
            return Err(CmdError::Params(format!("{} can't be changed", cal.name)));
        }
        let id = cal.id.clone();
        let uid = e.uid.clone();
        p.events.retain(|x| x.event.uid != uid);
        p.events.push(StoredEvent {
            calendar: id.clone(),
            href: String::new(),
            etag: String::new(),
            event: e,
            dirty: id != LOCAL,
            notify: false,
            respond: None,
        });
        self.after_pim_change(&account, id != LOCAL);
        Ok(serde_json::json!({"uid": uid, "account": account}))
    }

    /// Change an event (all of its occurrences).
    pub fn update_event(&mut self, uid: &str, change: impl FnOnce(&mut CalEvent)) -> CmdResult {
        let (account, at, real) = self
            .find_event_index(uid)
            .and_then(|(a, at)| self.pim.get(a).and_then(|p| p.events.get(at)).map(|e| (a.to_string(), at, e.event.uid.clone())))
            .ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?;
        let p = self.pim_mut(&account);
        let Some(stored) = p.events.get_mut(at) else {
            return Err(CmdError::Params(format!("no event `{uid}`")));
        };
        let _ = &real;
        if p.calendars.iter().any(|c| c.id == stored.calendar && c.read_only) {
            return Err(CmdError::Params("this calendar can't be changed".into()));
        }
        let mut e = stored.event.clone();
        change(&mut e);
        check_event(&e)?;
        e.sequence = e.sequence.saturating_add(1);
        stored.event = e;
        let remote = stored.calendar != LOCAL;
        stored.dirty |= remote;
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": real}))
    }

    /// Save, and ask for a sync when the server has something to learn.
    pub(crate) fn after_pim_change(&mut self, account: &str, remote: bool) {
        self.save_pim(account);
        if remote && self.account(account).is_some_and(|a| !a.is_local()) {
            self.jobs.push(Job::SyncDav { account: account.to_string() });
        }
    }

    // ----- syncing with the server -----

    /// Merge what a CalDAV / CardDAV sync found. Changes made here since stay (and go to the
    /// server next time); everything else follows the server.
    pub fn apply_dav(&mut self, account: &str, sync: &quatrelle_model::pim::DavSync) {
        self.read_while_parked(account, sync);
        let p = self.pim_mut(account);
        p.caldav_home = sync.caldav_home.clone().or_else(|| Some(String::new()));
        p.carddav_home = sync.carddav_home.clone().or_else(|| Some(String::new()));
        p.dav_checked = Some(sync.checked);
        p.deleted.retain(|t| !sync.deleted.contains(&t.href));
        // What was stored: its address and version, and no longer to send (unless it changed
        // again meanwhile).
        for x in &sync.pushed_events {
            for e in p.events.iter_mut().filter(|e| e.event.uid == x.event.uid && e.event.recurrence_id == x.event.recurrence_id) {
                e.href = x.href.clone();
                e.etag = x.etag.clone();
                // Stored in another calendar (a new event goes to the server's default one).
                e.calendar = x.calendar.clone();
                e.dirty &= e.event != x.event;
                // The server told the guests with that change; a later change asks again.
                e.notify &= e.dirty;
            }
        }
        // Notes stored on the server: their item there, and no longer to send.
        for x in &sync.pushed_notes {
            for n in p.notes.iter_mut().filter(|n| n.note.uid == x.note.uid) {
                n.href = x.href.clone();
                n.etag = x.etag.clone();
                n.dirty &= n.note != x.note;
            }
        }
        // The server's notes, by their item there; those changed here (and those never sent)
        // stay.
        if let Some(server) = &sync.notes {
            p.notes.retain(|n| n.dirty || n.href.is_empty());
            for x in server {
                if !p.notes.iter().any(|n| !n.href.is_empty() && n.href == x.href) {
                    p.notes.push(x.clone());
                }
            }
        }
        for x in &sync.pushed_cards {
            for c in p.cards.iter_mut().filter(|c| c.card.uid == x.card.uid) {
                c.href = x.href.clone();
                c.etag = x.etag.clone();
                c.book = x.book.clone();
                c.dirty &= c.card != x.card;
            }
        }
        if let Some(server) = &sync.calendars {
            // The calendar on this computer goes once the server has calendars, unless it holds
            // events (they stay there).
            let keep_local = server.is_empty() || p.events.iter().any(|e| e.calendar == LOCAL);
            let mut list: Vec<quatrelle_model::pim::CalendarInfo> = server.clone();
            if keep_local && let Some(l) = p.calendars.iter().find(|c| c.id == LOCAL) {
                list.push(l.clone());
            }
            // Only some looked at: the others stay, with their events.
            if sync.partial {
                let kept: Vec<_> = p.calendars.iter().filter(|c| c.id != LOCAL && !list.iter().any(|x| x.id == c.id)).cloned().collect();
                list.extend(kept);
            }
            p.events.retain(|e| e.calendar == LOCAL || list.iter().any(|c| c.id == e.calendar) || e.dirty);
            for cal in &sync.synced_calendars {
                p.events.retain(|e| &e.calendar != cal || e.dirty || e.href.is_empty() || sync.kept.contains(&e.href));
                for x in sync.events.iter().filter(|x| &x.calendar == cal) {
                    let edited_here = p.events.iter().any(|e| e.dirty && (e.href == x.href || (e.href.is_empty() && e.event.uid == x.event.uid)));
                    if !edited_here {
                        // The same item there (its address); and an appointment kept here that
                        // never went (no address). Two items may share a UID — some Exchange
                        // series arrive as separate items keeping the series' UID — and each
                        // is kept, since the server's address tells them apart. One CalDAV item
                        // holds a series and its moved occurrences: each is kept too.
                        p.events.retain(|e| x.href.is_empty() || e.href != x.href || e.event.recurrence_id != x.event.recurrence_id);
                        p.events.retain(|e| {
                            !(e.href.is_empty() && e.event.uid == x.event.uid && e.event.recurrence_id == x.event.recurrence_id && !e.dirty)
                        });
                        p.events.push(x.clone());
                    }
                }
            }
            p.calendars = list;
        }
        self.note_new_calendars();
        let p = self.pim_mut(account);
        if let Some(server) = &sync.books {
            let keep_local = server.is_empty() || p.cards.iter().any(|c| c.book == LOCAL);
            let mut list: Vec<quatrelle_model::pim::BookInfo> = server.clone();
            if keep_local && let Some(l) = p.books.iter().find(|b| b.id == LOCAL) {
                list.push(l.clone());
            }
            if sync.partial {
                let kept: Vec<_> = p.books.iter().filter(|b| b.id != LOCAL && !list.iter().any(|x| x.id == b.id)).cloned().collect();
                list.extend(kept);
            }
            p.cards.retain(|c| c.book == LOCAL || list.iter().any(|b| b.id == c.book) || c.dirty);
            for book in &sync.synced_books {
                p.cards.retain(|c| &c.book != book || c.dirty || c.href.is_empty() || sync.kept.contains(&c.href));
                for x in sync.cards.iter().filter(|x| &x.book == book) {
                    if !p.cards.iter().any(|c| c.dirty && c.card.uid == x.card.uid) {
                        p.cards.retain(|c| c.card.uid != x.card.uid);
                        p.cards.push(x.clone());
                    }
                }
            }
            p.books = list;
        }
        p.ensure_defaults();
        self.drop_lone_occurrences(account);
        for e in sync.errors.iter().take(3) {
            self.notice(format!("calendars and contacts: {e}"), true);
        }
        // Invitations the server answered, and copies of invitations to answer instead.
        if self.settle_answers(account, sync) {
            self.jobs.push(Job::SyncDav { account: account.to_string() });
        }
        self.save_pim(account);
    }
}

impl Session {
    // ----- other people's calendars -----

    /// Open someone else's calendar from an Exchange account (Outlook's Open Calendar › From
    /// Address Book): read with the account's own, read-only, once the server shows it.
    pub fn open_shared_calendar(&mut self, account: Option<&str>, email: &str) -> CmdResult {
        let account = match account {
            Some(a) => self.pim_account(Some(a))?,
            None => self
                .accounts
                .iter()
                .find(|a| a.config.graph.is_some() || a.config.ews.is_some())
                .map(|a| a.config.id.clone())
                .ok_or_else(|| CmdError::Disabled("other people's calendars open from an Exchange account".into()))?,
        };
        if !self.account(&account).is_some_and(|a| a.config.graph.is_some() || a.config.ews.is_some()) {
            return Err(CmdError::Params("other people's calendars open from an Exchange account".into()));
        }
        let email = email.trim().to_ascii_lowercase();
        let ok =
            quatrelle_model::addr::parse_list(&email).is_ok_and(|l| l.len() == 1 && l.first().is_some_and(|a| a.email.eq_ignore_ascii_case(&email)));
        if !ok {
            return Err(CmdError::Params(format!("not an email address: {email}")));
        }
        let mine = self.account(&account).is_some_and(|a| a.config.email.eq_ignore_ascii_case(&email));
        if mine {
            return Err(CmdError::Params("that's this account's own calendar".into()));
        }
        let p = self.pim_mut(&account);
        if !p.shared.contains(&email) {
            if p.shared.len() >= 50 {
                return Err(CmdError::Params("at most 50 other calendars can be open".into()));
            }
            p.shared.push(email.clone());
        }
        // Asked for: shown, even when it came unticked with a calendar group.
        let id = quatrelle_model::pim::shared_calendar_id(&email);
        self.prefs.hidden_calendars.remove(&Session::calendar_key(&account, &id));
        self.after_pim_change(&account, true);
        Ok(serde_json::json!({"account": account, "calendar": id}))
    }

    /// Close someone else's calendar: it and its events go from this computer.
    pub fn close_shared_calendar(&mut self, email: &str) -> CmdResult {
        let email = email.trim().to_ascii_lowercase();
        let id = quatrelle_model::pim::shared_calendar_id(&email);
        let account = self.pim.iter().find(|(_, p)| p.shared.contains(&email) || p.calendars.iter().any(|c| c.id == id)).map(|(a, _)| a.clone());
        let Some(account) = account else { return Err(CmdError::Params(format!("no calendar of {email} is open"))) };
        let p = self.pim_mut(&account);
        p.shared.retain(|e| *e != email);
        p.calendars.retain(|c| c.id != id);
        p.events.retain(|e| e.calendar != id);
        self.after_pim_change(&account, false);
        Ok(serde_json::json!({"closed": email}))
    }
}

pub(super) fn check_event(e: &CalEvent) -> Result<(), CmdError> {
    if e.summary.trim().is_empty() {
        return Err(CmdError::Params("give the event a title".into()));
    }
    if e.end < e.start {
        return Err(CmdError::Params("the event ends before it starts".into()));
    }
    if e.end - e.start > 366 * 86_400 {
        return Err(CmdError::Params("an event can last a year at most".into()));
    }
    Ok(())
}

/// A local date (`YYYY-MM-DD`, all day) or date and time (`YYYY-MM-DDTHH:MM`, or with a space),
/// or Unix seconds, as (Unix seconds, all day). `offset` is the local UTC offset; all-day dates
/// come back at UTC midnight (as events keep them).
pub fn parse_local(s: &str, clock: &Clock) -> Option<(i64, bool)> {
    let s = s.trim();
    if let Ok(n) = s.parse::<i64>() {
        return (n.abs() < 400_000_000_000).then_some((n, false));
    }
    let num = |a: usize, b: usize| s.get(a..b).filter(|x| x.bytes().all(|c| c.is_ascii_digit())).and_then(|x| x.parse::<i64>().ok());
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    if s.get(4..5) != Some("-") || s.get(7..8) != Some("-") || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let day = days_from_civil(y, u32::try_from(m).ok()?, u32::try_from(d).ok()?);
    if s.len() == 10 {
        return Some((day * 86_400, true));
    }
    if !matches!(s.get(10..11), Some("T" | " ")) {
        return None;
    }
    let (h, mi) = (num(11, 13)?, num(14, 16)?);
    if h > 23 || mi > 59 || s.get(13..14) != Some(":") {
        return None;
    }
    Some((clock.utc(day * 86_400 + h * 3600 + mi * 60), false))
}

/// The time to judge old answers by: `t` when it's a real time.
fn now_or(t: i64) -> i64 {
    t.max(0)
}
