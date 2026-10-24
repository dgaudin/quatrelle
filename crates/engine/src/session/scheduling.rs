//! Scheduling a meeting: the rooms to pick from (Outlook's Room Finder) and when people and
//! rooms are busy (its Scheduling Assistant). An Exchange or Microsoft 365 account asks its
//! server ([`Job::GetRooms`], [`Job::GetAvailability`], answered by events while the form stays
//! responsive); every other account is answered at once from the calendars kept here. What
//! was asked is kept, so the form draws from it frame after frame and asks once per window.

use quatrelle_model::ical::{self, CalEvent};
use quatrelle_model::pim::{StoredEvent, shared_calendar_id, shared_calendar_owner};
use quatrelle_model::scheduling::{
    Availability, BusyKind, BusySlot, MAX_PEOPLE, MAX_ROOMS, MAX_SLOTS, MAX_WINDOW, Room, RoomList, is_rooms_group, merge_slots,
};

use super::*;

/// Something asked of the server: on its way, here, or refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetched<T> {
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Fetched<T> {
    /// Its name in commands' answers.
    pub fn state(&self) -> &'static str {
        match self {
            Fetched::Loading => "loading",
            Fetched::Ready(_) => "ready",
            Fetched::Failed(_) => "failed",
        }
    }
}

/// The rooms an account offers: the server's lists and their rooms, with those the calendar
/// knows after them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomFinder {
    pub lists: Vec<RoomList>,
    pub rooms: Vec<Room>,
    /// The server offered no room lists (none set up, or no server): the rooms are those the
    /// calendar knows.
    pub from_calendar: bool,
}

/// Free/busy windows kept at most; past that they're all let go (asked again as needed).
const FREE_BUSY_MAX: usize = 400;

fn key(account: &str, email: &str, from: i64, to: i64) -> (String, String, i64, i64) {
    (account.to_string(), email.trim().to_ascii_lowercase(), from, to)
}

impl Session {
    /// Whether an account's server offers rooms and free/busy (Exchange, Microsoft 365).
    pub fn server_schedules(&self, account: &str) -> bool {
        self.account(account).is_some_and(|a| !a.is_local() && (a.config.ews.is_some() || a.config.graph.is_some()))
    }

    /// The rooms an account knows from its calendars alone: the mailboxes of the calendars
    /// Outlook keeps in its rooms group ("Rooms", or "Salles" in a French Outlook), and every
    /// room or resource booked in a meeting kept here. What a server with no room lists set up
    /// still has to offer.
    pub fn calendar_rooms(&self, account: &str) -> Vec<Room> {
        let Some(p) = self.pim.get(account) else { return Vec::new() };
        let mut out: Vec<Room> = Vec::new();
        let mut add = |email: &str, name: &str| {
            let email = email.trim().to_ascii_lowercase();
            if email.contains('@') && !out.iter().any(|r| r.email == email) && out.len() < MAX_ROOMS {
                let name = if name.trim().is_empty() { email.clone() } else { name.trim().to_string() };
                out.push(Room { email, name, list: String::new() });
            }
        };
        for c in p.calendars.iter().filter(|c| is_rooms_group(&c.group)) {
            if let Some(email) = shared_calendar_owner(&c.id) {
                add(email, &c.name);
            }
        }
        for a in p.events.iter().flat_map(|e| e.event.attendees.iter()).filter(|a| a.is_resource()) {
            add(&a.email, &a.name);
        }
        out.sort_by_cached_key(|r| r.name.to_lowercase());
        out
    }

    /// What's known of an account's rooms.
    pub fn rooms_now(&self, account: &str) -> Option<&Fetched<RoomFinder>> {
        self.rooms.get(account)
    }

    /// Ask for an account's rooms (`refresh`: again): the server's, with the calendar's after
    /// them once they come; an account with no server for it has the calendar's at once.
    pub fn request_rooms(&mut self, account: &str, refresh: bool) -> CmdResult {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        if refresh || !self.rooms.contains_key(account) {
            if self.server_schedules(account) {
                self.rooms.insert(account.to_string(), Fetched::Loading);
                self.jobs.push(Job::GetRooms { account: account.to_string() });
            } else {
                let rooms = self.calendar_rooms(account);
                self.rooms.insert(account.to_string(), Fetched::Ready(RoomFinder { lists: Vec::new(), rooms, from_calendar: true }));
            }
        }
        Ok(self.rooms_json(account))
    }

    fn rooms_json(&self, account: &str) -> Value {
        let mut v = serde_json::json!({"account": account, "state": self.rooms.get(account).map_or("none", Fetched::state)});
        match self.rooms.get(account) {
            Some(Fetched::Failed(e)) => v["error"] = serde_json::json!(e),
            Some(Fetched::Ready(f)) => {
                v["fromCalendar"] = serde_json::json!(f.from_calendar);
                v["lists"] = serde_json::json!(f.lists.iter().map(|l| serde_json::json!({"name": l.name, "email": l.email})).collect::<Vec<_>>());
                v["rooms"] = serde_json::json!(
                    f.rooms.iter().map(|r| serde_json::json!({"name": r.name, "email": r.email, "list": r.list})).collect::<Vec<_>>()
                );
            }
            _ => {}
        }
        v
    }

    /// The server's answer about rooms, the calendar's added after them: a server with no
    /// lists set up (or one that refused) still offers what the calendar knows.
    pub(crate) fn rooms_answered(&mut self, account: &str, lists: Vec<RoomList>, mut rooms: Vec<Room>, error: Option<String>) {
        let known = self.calendar_rooms(account);
        if let Some(e) = &error {
            if rooms.is_empty() && known.is_empty() {
                self.rooms.insert(account.to_string(), Fetched::Failed(e.clone()));
                return;
            }
            log::info!("{account}: rooms from the server: {e}; the calendar's are offered");
        }
        let from_calendar = rooms.is_empty();
        for r in known {
            if !rooms.iter().any(|x| x.email == r.email) && rooms.len() < MAX_ROOMS {
                rooms.push(r);
            }
        }
        self.rooms.insert(account.to_string(), Fetched::Ready(RoomFinder { lists, rooms, from_calendar }));
    }

    /// What's known of when someone (or a room) is busy over a window.
    pub fn availability_now(&self, account: &str, email: &str, from: i64, to: i64) -> Option<&Fetched<Availability>> {
        self.free_busy.get(&key(account, email, from, to))
    }

    /// Ask when these people and rooms are busy between `from` and `to`: once per window
    /// (`refresh`: again); what's known already answers at once, as does an account with no
    /// server for it, from the calendars kept here.
    pub fn request_availability(&mut self, account: &str, emails: &[String], from: i64, to: i64, refresh: bool) -> CmdResult {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        if to <= from {
            return Err(CmdError::Params("the window ends before it starts".into()));
        }
        if to - from > MAX_WINDOW {
            return Err(CmdError::Params("availability is read two months at a time at most".into()));
        }
        let mut emails: Vec<String> = emails.iter().map(|e| e.trim().to_ascii_lowercase()).filter(|e| e.contains('@') && e.len() < 320).collect();
        emails.dedup();
        emails.truncate(MAX_PEOPLE);
        if emails.is_empty() {
            return Err(CmdError::Params("`emails` is needed: whose availability".into()));
        }
        let wanted: Vec<String> = emails.iter().filter(|e| refresh || !self.free_busy.contains_key(&key(account, e, from, to))).cloned().collect();
        if !wanted.is_empty() {
            if self.free_busy.len() + wanted.len() > FREE_BUSY_MAX {
                self.free_busy.clear();
            }
            if self.server_schedules(account) {
                for e in &wanted {
                    self.free_busy.insert(key(account, e, from, to), Fetched::Loading);
                }
                self.jobs.push(Job::GetAvailability { account: account.to_string(), emails: wanted, from, to });
            } else {
                // Other people's calendars, set aside away from the calendar: read for this.
                let aside = self.set_aside(account);
                for e in &wanted {
                    let a = self.local_availability(account, e, from, to, &aside);
                    self.free_busy.insert(key(account, e, from, to), Fetched::Ready(a));
                }
            }
        }
        let people: Vec<Value> = emails.iter().map(|e| self.availability_json(account, e, from, to)).collect();
        Ok(serde_json::json!({"account": account, "from": from, "to": to, "people": people}))
    }

    fn availability_json(&self, account: &str, email: &str, from: i64, to: i64) -> Value {
        let known = self.free_busy.get(&key(account, email, from, to));
        let mut v = serde_json::json!({"email": email, "state": known.map_or("none", Fetched::state)});
        match known {
            Some(Fetched::Failed(e)) => v["error"] = serde_json::json!(e),
            Some(Fetched::Ready(a)) => {
                if let Some(e) = &a.error {
                    v["error"] = serde_json::json!(e);
                }
                v["busy"] = serde_json::json!(
                    a.slots
                        .iter()
                        .map(|s| serde_json::json!({"start": s.start, "end": s.end, "kind": s.kind.code(), "subject": s.subject}))
                        .collect::<Vec<_>>()
                );
            }
            _ => {}
        }
        v
    }

    /// The server's answer about a window: each address it names takes its entry (the one
    /// waiting, or a new one). Those it doesn't name are left as they are: two requests can be
    /// out for one window (a guest added while the first is on its way), each answered on its
    /// own. An answer that names no one and failed fails everyone still waiting on the window.
    pub(crate) fn availability_answered(&mut self, account: &str, from: i64, to: i64, people: Vec<Availability>, error: Option<String>) {
        if people.is_empty() {
            let Some(e) = error else { return };
            let waiting: Vec<(String, String, i64, i64)> = self
                .free_busy
                .iter()
                .filter(|(k, v)| k.0 == account && k.2 == from && k.3 == to && matches!(v, Fetched::Loading))
                .map(|(k, _)| k.clone())
                .collect();
            for k in waiting {
                self.free_busy.insert(k, Fetched::Failed(e.clone()));
            }
            return;
        }
        for p in people.into_iter().take(MAX_PEOPLE) {
            if !p.email.contains('@') {
                continue;
            }
            let k = key(account, &p.email, from, to);
            let v = match (&p.error, &error) {
                // Refused as a whole: a failure, to be asked again on purpose.
                (Some(_), Some(e)) => Fetched::Failed(e.clone()),
                _ => Fetched::Ready(p),
            };
            self.free_busy.insert(k, v);
        }
    }

    /// When someone is busy, from the calendars kept here: the account's own calendars for
    /// their owner, the calendar opened by their address for anyone else (a room's, a
    /// colleague's), and nothing for someone whose calendar isn't here.
    fn local_availability(&self, account: &str, email: &str, from: i64, to: i64, aside: &[StoredEvent]) -> Availability {
        let none = |why: &str| Availability { email: email.to_string(), slots: Vec::new(), error: Some(why.to_string()) };
        let Some(p) = self.pim.get(account) else { return none("no calendar on this computer") };
        let mine = self.account(account).is_some_and(|a| a.config.email.eq_ignore_ascii_case(email) || a.config.username.eq_ignore_ascii_case(email));
        let theirs = shared_calendar_id(email);
        let calendars: Vec<&str> = p
            .calendars
            .iter()
            .filter(|c| if mine { shared_calendar_owner(&c.id).is_none() && !c.id.starts_with("public:") } else { c.id == theirs })
            .map(|c| c.id.as_str())
            .collect();
        if calendars.is_empty() {
            return none("their calendar isn't on this computer");
        }
        let events: Vec<CalEvent> =
            p.events.iter().chain(aside).filter(|e| calendars.contains(&e.calendar.as_str())).map(|e| e.event.clone()).collect();
        let mut slots = Vec::new();
        for o in ical::occurrences(&events, from - 86_400, to + 86_400) {
            let Some(e) = events.get(o.event) else { continue };
            if e.status.eq_ignore_ascii_case("CANCELLED") {
                continue;
            }
            let (start, end) = if e.all_day { (self.clock.utc(o.start), self.clock.utc(o.end)) } else { (o.start, o.end) };
            if start >= to || end <= from {
                continue;
            }
            // Their own answer to a meeting: declined takes no time, tentative shows as such.
            let answer = e.attendees.iter().find(|a| a.email.eq_ignore_ascii_case(email)).map(|a| a.partstat.as_str()).unwrap_or("");
            if answer == "DECLINED" {
                continue;
            }
            // And how it shows (Outlook's Show As): free time isn't taken.
            let kind = match e.show_as {
                ical::ShowAs::Free => continue,
                ical::ShowAs::OutOfOffice => BusyKind::OutOfOffice,
                ical::ShowAs::WorkingElsewhere => BusyKind::WorkingElsewhere,
                _ if e.show_as == ical::ShowAs::Tentative || e.status.eq_ignore_ascii_case("TENTATIVE") || answer == "TENTATIVE" => {
                    BusyKind::Tentative
                }
                _ => BusyKind::Busy,
            };
            slots.push(BusySlot { start, end, kind, subject: e.summary.clone() });
            if slots.len() >= MAX_SLOTS {
                break;
            }
        }
        Availability { email: email.to_string(), slots: merge_slots(slots), error: None }
    }
}
