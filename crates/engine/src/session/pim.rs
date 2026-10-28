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
mod reminders;
mod syncing;
mod tasks;
pub(crate) mod web;

/// Whether a calendar is the user's own: not a colleague's shared one, nor a public folder's,
/// nor one subscribed to by its link (a holiday list is someone else's, as Outlook has it: set
/// aside away from the calendar, its alarms not reminders).
pub(super) fn own_calendar(id: &str) -> bool {
    quatrelle_model::pim::shared_calendar_owner(id).is_none() && !id.starts_with("public:") && !quatrelle_model::pim::is_web_calendar(id)
}
pub use notes::NoteRef;
pub use people::Person;
pub use reminders::DueReminder;
pub use tasks::{TaskChange, TaskRef};

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
    /// Outlook's Tasks.
    Tasks,
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

impl Session {
    pub(crate) fn load_pim(&mut self) {
        let Some(store) = self.store.clone() else { return };
        for a in &self.accounts {
            let mut p: Pim = store.load_account_doc(&a.config.id, PIM_DOC).unwrap_or_default();
            p.read_as(store.load_account_doc(&a.config.id, quatrelle_model::pim::PIM_READING_DOC).unwrap_or_default());
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
        self.write_pim(account);
    }

    /// [`Session::save_pim`] when the contacts didn't change: the address book isn't worked out
    /// again.
    pub(crate) fn write_pim(&mut self, account: &str) {
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
            // Opening the calendar reads every calendar ticked; one just ticked is read alone (a
            // colleague's), or with the rest (a public folder's), and one subscribed to by its
            // link isn't the server's to read.
            let opened = p.own_only && !own_only;
            let ticked: Vec<String> = if own_only {
                Vec::new()
            } else {
                p.not_read.iter().filter(|c| !not_read.contains(c) && !quatrelle_model::pim::is_web_calendar(c)).cloned().collect()
            };
            let p = self.pim_mut(&a);
            p.own_only = own_only;
            p.not_read = not_read;
            // In a document of its own: the calendar isn't written again for it (nor the address
            // book worked out again), at each switch between the calendar and the mail.
            let reading = p.reading();
            if let Some(store) = &self.store
                && let Err(e) = store.save_account_doc(&a, quatrelle_model::pim::PIM_READING_DOC, &reading)
            {
                log::warn!("{a}: which calendars to read couldn't be kept: {e}");
            }
            if opened || ticked.iter().any(|c| quatrelle_model::pim::shared_calendar_owner(c).is_none()) {
                log::info!("{a}: the calendars to read now (the calendar opened, or a public folder's ticked)");
                self.jobs.push(Job::SyncCalendars { account: a.clone(), calendars: Vec::new() });
            } else if !ticked.is_empty() {
                log::info!("{a}: {} calendars ticked, read alone", ticked.len());
                self.jobs.push(Job::SyncCalendars { account: a.clone(), calendars: ticked });
            }
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
            log::info!("{account}: a change to send to the calendars, contacts or tasks");
            self.jobs.push(Job::SyncDav { account: account.to_string() });
        }
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
