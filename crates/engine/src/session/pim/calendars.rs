// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A calendar's own menu in the side pane: its colour and its place kept on this computer, one
//! calendar shown alone, one removed from the list here (never on the server) and brought back,
//! and, where the account's server allows it, a calendar made, renamed or deleted there (through
//! the account's journal, the calendars read again after).

use quatrelle_model::pim::{CalendarInfo, MAX_CALENDAR_NAME, plain_name};
use serde_json::json;

use super::*;

/// The colours a calendar can be given on this computer, the palette's.
pub const PALETTE_COLORS: u8 = 16;
/// A calendar colour from this one on is the palette's (`PALETTE_COLOR + i`, `i` below
/// [`PALETTE_COLORS`]), as [`Session::calendar_color`] gives it: past every colour a server's
/// calendar or an account's has.
pub const PALETTE_COLOR: u8 = 64;

/// Why a calendar can't be made, renamed or deleted from here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalendarRefusal {
    /// The account's server keeps none Quatrelle changes ([`AccountConfig::makes_calendars`]).
    Server,
    /// The account's main calendar.
    Main,
    /// Not one of the account's own: someone else's, a public folder's, one subscribed to by its
    /// link, one kept on this computer, or one the account can only read.
    NotOwn,
    /// The server hasn't said yet which calendar is the account's main one (kept from before it
    /// was asked).
    NotRead,
}

impl CalendarRefusal {
    /// Why, for a command that's refused.
    pub fn reason(self) -> &'static str {
        match self {
            CalendarRefusal::Server => "this account's server doesn't allow it",
            CalendarRefusal::Main => "the account's main calendar stays as it is",
            CalendarRefusal::NotOwn => "only the account's own calendars can be changed",
            CalendarRefusal::NotRead => "the server hasn't listed the calendars yet",
        }
    }
}

/// Why a calendar can't be removed from the list on this computer ([`Session::calendar_removable`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemovalRefusal {
    /// The account's main calendar, or the one kept here for an account without calendars.
    Main,
    /// Opened by its owner's address, or subscribed to by its link: its own Remove takes it away.
    OwnRemove,
}

impl RemovalRefusal {
    /// Why, for a command that's refused.
    pub fn reason(self) -> &'static str {
        match self {
            RemovalRefusal::Main => "the account's main calendar stays in the list",
            RemovalRefusal::OwnRemove => "one opened by its owner's address or subscribed to by its link goes by its own Remove",
        }
    }
}

/// Why a calendar group can't be made, renamed, deleted or moved from here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupRefusal {
    /// The account's server keeps no calendar groups Quatrelle changes (an IMAP, CalDAV or local
    /// account).
    Server,
    /// The server keeps the groups but not an order of them (Microsoft 365).
    Order,
    /// The server can't move an existing calendar from one group to another (Microsoft 365).
    Move,
    /// The group was made by Outlook (or another client), not here: [MS-OXOCFG] gives no
    /// well-known id to tell its own headers from ours, so only an empty one may go.
    Outlook,
    /// The group still holds calendars: deleting it would take them with it.
    NotEmpty,
}

impl GroupRefusal {
    /// Why, for a command that's refused.
    pub fn reason(self) -> &'static str {
        match self {
            GroupRefusal::Server => "this account's server doesn't keep calendar groups",
            GroupRefusal::Order => "Microsoft 365 keeps no order for calendar groups",
            GroupRefusal::Move => "Microsoft 365 can't move a calendar between groups",
            GroupRefusal::Outlook => "Outlook's own calendar group can't be changed here",
            GroupRefusal::NotEmpty => "a calendar group that still holds calendars can't be deleted here",
        }
    }
}

/// Whether two of an account's calendars are listed together in the side pane, so one moves past
/// the other: in the same group, and both someone else's (by address) or neither.
fn listed_together(a: &CalendarInfo, b: &CalendarInfo) -> bool {
    a.group == b.group && quatrelle_model::pim::shared_calendar_owner(&a.id).is_some() == quatrelle_model::pim::shared_calendar_owner(&b.id).is_some()
}

/// A calendar's name as typed, made plain ([`plain_name`]); refused empty or longer than
/// [`MAX_CALENDAR_NAME`].
fn typed_name(name: &str) -> Result<String, CmdError> {
    let out = plain_name(name);
    if out.is_empty() {
        return Err(CmdError::Params("a calendar needs a name".into()));
    }
    if out.chars().count() > MAX_CALENDAR_NAME {
        return Err(CmdError::Params(format!("a calendar's name is at most {MAX_CALENDAR_NAME} characters")));
    }
    Ok(out)
}

impl Session {
    /// Whether a calendar can be removed from the list on this computer: not the account's main
    /// one (nor the one kept here for an account without calendars); one opened by its owner's
    /// address or subscribed to by its link goes by its own Remove. Decided here alone: the menu
    /// greys what this refuses. (The side pane keeps the heading of an account with removed
    /// calendars, whatever is left listed: they're brought back from there.)
    pub fn calendar_removable(&self, account: &str, calendar: &str) -> Result<(), RemovalRefusal> {
        let p = self.pim.get(account);
        let c = p.and_then(|p| p.calendars.iter().find(|c| c.id == calendar));
        if calendar == LOCAL || c.is_some_and(|c| c.main) {
            return Err(RemovalRefusal::Main);
        }
        let opened = quatrelle_model::pim::shared_calendar_owner(calendar)
            .is_some_and(|owner| p.is_some_and(|p| p.shared.iter().any(|x| x.eq_ignore_ascii_case(owner))));
        if opened || quatrelle_model::pim::is_web_calendar(calendar) {
            return Err(RemovalRefusal::OwnRemove);
        }
        Ok(())
    }

    /// Remove a calendar from the list on this computer, never on the server: out of the side
    /// pane, the calendar, the search, the reminders and the calendars an appointment is put in,
    /// and no longer read by the sync. What this computer has of it stays as it was, so it's
    /// brought back ([`Session::restore_calendar`]) going on from there.
    pub fn remove_calendar(&mut self, account: &str, calendar: &str) -> CmdResult {
        self.calendar_named(account, calendar)?;
        self.calendar_removable(account, calendar).map_err(|r| CmdError::Disabled(r.reason().into()))?;
        self.prefs.removed_calendars.insert(Session::calendar_key(account, calendar));
        self.pim_revision = self.pim_revision.wrapping_add(1);
        Ok(json!({"account": account, "calendar": calendar, "removed": true}))
    }

    /// Bring a calendar removed from the list back: listed, shown or hidden as it was, and read
    /// again (at once in the calendar, else at the next sync).
    pub fn restore_calendar(&mut self, account: &str, calendar: &str) -> CmdResult {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        let restored = self.prefs.removed_calendars.remove(&Session::calendar_key(account, calendar));
        if restored {
            self.pim_revision = self.pim_revision.wrapping_add(1);
        }
        Ok(json!({"account": account, "calendar": calendar, "restored": restored}))
    }

    /// An account's calendars removed from the list, as the server lists them.
    pub fn removed_calendars(&self, account: &str) -> Vec<&CalendarInfo> {
        let Some(p) = self.pim.get(account) else { return Vec::new() };
        p.calendars.iter().filter(|c| self.calendar_removed(account, &c.id)).collect()
    }

    /// An account's calendars in the order the side pane lists them: as the user placed them on
    /// this computer, then those not placed, as the server lists them; none removed from the list.
    pub fn ordered_calendars(&self, account: &str) -> Vec<&CalendarInfo> {
        let Some(p) = self.pim.get(account) else { return Vec::new() };
        let mut list: Vec<&CalendarInfo> = p.calendars.iter().filter(|c| !self.calendar_removed(account, &c.id)).collect();
        if let Some(order) = self.prefs.calendar_order.get(account) {
            let at: HashMap<&str, usize> = order.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
            list.sort_by_key(|c| at.get(c.id.as_str()).copied().unwrap_or(usize::MAX));
        }
        list
    }

    /// A calendar of `account`, or why the params don't name one.
    fn calendar_named(&self, account: &str, calendar: &str) -> Result<&CalendarInfo, CmdError> {
        self.pim
            .get(account)
            .and_then(|p| p.calendars.iter().find(|c| c.id == calendar))
            .ok_or_else(|| CmdError::Params(format!("no calendar `{calendar}` in `{account}`")))
    }

    /// A calendar removed from the list is neither moved (it isn't among those it would move
    /// past) nor shown alone (it isn't shown): refused until it's brought back.
    fn listed(&self, account: &str, calendar: &str) -> Result<(), CmdError> {
        if self.calendar_removed(account, calendar) {
            return Err(CmdError::Disabled("it's removed from the list: bring it back first".into()));
        }
        Ok(())
    }

    /// Where a calendar is among those listed with it (`listed_together`), and how many they are.
    fn place_among(&self, account: &str, calendar: &str) -> Option<(usize, usize)> {
        let list = self.ordered_calendars(account);
        let c = list.iter().find(|c| c.id == calendar)?;
        let peers: Vec<&&CalendarInfo> = list.iter().filter(|x| listed_together(x, c)).collect();
        Some((peers.iter().position(|x| x.id == calendar)?, peers.len()))
    }

    /// Whether a calendar can move up (or down) among those listed with it: not at that end.
    pub fn can_move_calendar(&self, account: &str, calendar: &str, up: bool) -> bool {
        self.place_among(account, calendar).is_some_and(|(at, n)| if up { at > 0 } else { at + 1 < n })
    }

    /// Move a calendar up (or down) past the one before (after) it among those listed with it,
    /// the order kept on this computer.
    pub fn move_calendar(&mut self, account: &str, calendar: &str, up: bool) -> CmdResult {
        let c = self.calendar_named(account, calendar)?.clone();
        self.listed(account, calendar)?;
        let list = self.ordered_calendars(account);
        let peers: Vec<usize> = list.iter().enumerate().filter(|(_, x)| listed_together(x, &c)).map(|(i, _)| i).collect();
        let at = peers.iter().position(|&i| list.get(i).is_some_and(|x| x.id == calendar)).unwrap_or(0);
        let next = if up { at.checked_sub(1) } else { at.checked_add(1) };
        let (Some(&here), Some(&there)) = (peers.get(at), next.and_then(|n| peers.get(n))) else {
            return Err(CmdError::Disabled(if up { "it's the first already".into() } else { "it's the last already".into() }));
        };
        let mut ids: Vec<String> = list.iter().map(|x| x.id.clone()).collect();
        ids.swap(here, there);
        self.prefs.calendar_order.insert(account.to_string(), ids.clone());
        self.pim_revision = self.pim_revision.wrapping_add(1);
        Ok(json!({"account": account, "calendar": calendar, "order": ids}))
    }

    /// Show this calendar, and hide every other (of every account).
    pub fn show_only_calendar(&mut self, account: &str, calendar: &str) -> CmdResult {
        self.calendar_named(account, calendar)?;
        self.listed(account, calendar)?;
        let this = Session::calendar_key(account, calendar);
        let others: Vec<String> =
            self.pim.iter().flat_map(|(a, p)| p.calendars.iter().map(move |c| Session::calendar_key(a, &c.id))).filter(|k| *k != this).collect();
        let hidden = others.len();
        self.prefs.hidden_calendars.extend(others);
        self.prefs.hidden_calendars.remove(&this);
        self.pim_revision = self.pim_revision.wrapping_add(1);
        Ok(json!({"account": account, "calendar": calendar, "hidden": hidden}))
    }

    /// Give a calendar one of the palette's colours on this computer (`0..PALETTE_COLORS`), the
    /// account's ([`ACCOUNT_COLOR`]), or (`None`) let it go back to the server's.
    pub fn set_calendar_color(&mut self, account: &str, calendar: &str, color: Option<u8>) -> CmdResult {
        self.calendar_named(account, calendar)?;
        let key = Session::calendar_key(account, calendar);
        match color {
            Some(c) if c < PALETTE_COLORS || c == ACCOUNT_COLOR => {
                self.prefs.calendar_colors.insert(key, c);
            }
            Some(c) => return Err(CmdError::Params(format!("`color` is 0 to {}, {ACCOUNT_COLOR} or null, not {c}", PALETTE_COLORS - 1))),
            None => {
                self.prefs.calendar_colors.remove(&key);
            }
        }
        self.pim_revision = self.pim_revision.wrapping_add(1);
        Ok(json!({"account": account, "calendar": calendar, "color": color}))
    }

    /// The colour a calendar is drawn in (with its events): the one chosen on this computer;
    /// else an account's own calendar in the account's colour, as its mail shows it, so each
    /// account's calendar can be told apart (they used to share one), its others in theirs.
    /// [`ACCOUNT_COLOR`] and above stand for an account's colour (`ACCOUNT_COLOR +
    /// AccountConfig::color`), [`PALETTE_COLOR`] and above for the palette's.
    pub fn calendar_color(&self, a: &AccountState, c: &CalendarInfo) -> u8 {
        match self.chosen_color(&a.config.id, &c.id) {
            Some(ACCOUNT_COLOR) => ACCOUNT_COLOR.saturating_add(a.config.color),
            Some(p) => PALETTE_COLOR.saturating_add(p),
            None if c.color == 0 => ACCOUNT_COLOR.saturating_add(a.config.color),
            None => c.color,
        }
    }

    /// The colour chosen on this computer for a calendar, when there's one that's one of the
    /// menu's (a value kept that isn't, from a settings file written elsewhere, counts as none).
    pub fn chosen_color(&self, account: &str, calendar: &str) -> Option<u8> {
        self.prefs.calendar_colors.get(&Session::calendar_key(account, calendar)).copied().filter(|&v| v < PALETTE_COLORS || v == ACCOUNT_COLOR)
    }

    /// Whether a calendar can be made in `account` (`calendar` none), or that one renamed or
    /// deleted: the account's server allows it, and the calendar is one of the account's own,
    /// not its main one. Decided here alone: the menu greys what this refuses.
    pub fn calendar_change(&self, account: &str, calendar: Option<&str>) -> Result<(), CalendarRefusal> {
        if !self.account(account).is_some_and(|a| a.config.makes_calendars()) {
            return Err(CalendarRefusal::Server);
        }
        let Some(calendar) = calendar else { return Ok(()) };
        let Some(p) = self.pim.get(account) else { return Err(CalendarRefusal::NotRead) };
        let Some(c) = p.calendars.iter().find(|c| c.id == calendar) else { return Err(CalendarRefusal::NotOwn) };
        if c.main {
            return Err(CalendarRefusal::Main);
        }
        if c.read_only || c.not_own || !c.group.is_empty() || c.id == LOCAL || !own_calendar(&c.id) {
            return Err(CalendarRefusal::NotOwn);
        }
        // Which is the main one isn't known yet: none is touched.
        if !p.calendars.iter().any(|c| c.main) {
            return Err(CalendarRefusal::NotRead);
        }
        Ok(())
    }

    /// Make a calendar called `name` in the account's own mailbox: on the server, which the
    /// calendars read again after bring in.
    pub fn create_calendar(&mut self, account: &str, name: &str) -> CmdResult {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        self.calendar_change(account, None).map_err(|r| CmdError::Disabled(r.reason().into()))?;
        let name = typed_name(name)?;
        if self.pim.get(account).is_some_and(|p| p.calendars.iter().any(|c| own_calendar(&c.id) && c.name.eq_ignore_ascii_case(&name))) {
            return Err(CmdError::Params(format!("there's a calendar called {name} already")));
        }
        self.jobs.push(Job::Apply { account: account.to_string(), op: Op::CreateCalendar { name: name.clone() } });
        Ok(json!({"account": account, "name": name}))
    }

    /// Rename one of the account's calendars: here at once, on the server through the journal.
    pub fn rename_calendar(&mut self, account: &str, calendar: &str, name: &str) -> CmdResult {
        let was = self.calendar_named(account, calendar)?.name.clone();
        self.calendar_change(account, Some(calendar)).map_err(|r| CmdError::Disabled(r.reason().into()))?;
        let name = typed_name(name)?;
        if name != was {
            if let Some(c) = self.pim_mut(account).calendars.iter_mut().find(|c| c.id == calendar) {
                c.name.clone_from(&name);
            }
            self.write_pim(account);
            self.jobs
                .push(Job::Apply { account: account.to_string(), op: Op::RenameCalendar { calendar: calendar.to_string(), name: name.clone() } });
        }
        Ok(json!({"account": account, "calendar": calendar, "name": name}))
    }

    /// Delete one of the account's calendars with its events: here at once (what this computer
    /// keeps of it goes too), on the server through the journal.
    pub fn delete_calendar(&mut self, account: &str, calendar: &str) -> CmdResult {
        self.calendar_named(account, calendar)?;
        self.calendar_change(account, Some(calendar)).map_err(|r| CmdError::Disabled(r.reason().into()))?;
        let p = self.pim_mut(account);
        p.calendars.retain(|c| c.id != calendar);
        let before = p.events.len();
        // The server deletes them with it: none goes on its own.
        p.events.retain(|e| e.calendar != calendar);
        let events = before.saturating_sub(p.events.len());
        p.not_read.retain(|c| c != calendar);
        let key = Session::calendar_key(account, calendar);
        self.prefs.hidden_calendars.remove(&key);
        self.prefs.known_calendars.remove(&key);
        self.prefs.calendar_colors.remove(&key);
        self.prefs.removed_calendars.remove(&key);
        if let Some(order) = self.prefs.calendar_order.get_mut(account) {
            order.retain(|c| c != calendar);
        }
        self.write_pim(account);
        self.jobs.push(Job::Apply { account: account.to_string(), op: Op::DeleteCalendar { calendar: calendar.to_string() } });
        Ok(json!({"account": account, "calendar": calendar, "events": events}))
    }

    /// Whether the account's server keeps calendar groups Quatrelle changes, in the order it
    /// keeps them, and whether it can move a calendar from one to another. Decided here alone:
    /// the menu greys what this refuses.
    pub fn calendar_groups_change(&self, account: &str) -> Result<(), GroupRefusal> {
        match self.account(account).map(|a| (&a.config.ews, &a.config.graph)) {
            Some((ews, _)) if ews.is_some() => Ok(()),
            Some((_, graph)) if graph.is_some() => Ok(()),
            _ => Err(GroupRefusal::Server),
        }
    }

    /// Whether `group` may be renamed here: only a header this session wrote (or, on Microsoft
    /// 365, a group the account keeps through Graph). Outlook's own headers have its save stamp,
    /// not ours ([MS-OXOCFG] 2.2.9.4).
    pub fn calendar_group_rename(&self, account: &str, group: &str) -> Result<(), GroupRefusal> {
        match self.account(account).map(|a| (&a.config.ews, &a.config.graph)) {
            Some((ews, _)) if ews.is_some() => {
                if self.own_group(account, group) {
                    Ok(())
                } else {
                    Err(GroupRefusal::Outlook)
                }
            }
            Some((_, graph)) if graph.is_some() => Ok(()),
            _ => Err(GroupRefusal::Server),
        }
    }

    /// Whether `group` may be deleted here: ours, or (the fallback when [MS-OXOCFG] can't tell
    /// Outlook's headers from ours) a group that holds no calendar. Microsoft 365 refuses a
    /// group that still holds calendars, as its own behaviour on a non-empty one isn't known.
    pub fn calendar_group_delete(&self, account: &str, group: &str) -> Result<(), GroupRefusal> {
        match self.account(account).map(|a| (&a.config.ews, &a.config.graph)) {
            Some((ews, _)) if ews.is_some() => {
                if self.own_group(account, group) || !self.group_has_calendars(account, group) {
                    Ok(())
                } else {
                    Err(GroupRefusal::NotEmpty)
                }
            }
            Some((_, graph)) if graph.is_some() => {
                if self.group_has_calendars(account, group) {
                    Err(GroupRefusal::NotEmpty)
                } else {
                    Ok(())
                }
            }
            _ => Err(GroupRefusal::Server),
        }
    }

    /// Whether the server orders the group (Microsoft 365 doesn't say one), and it is ours.
    pub fn calendar_group_order(&self, account: &str, group: &str) -> Result<(), GroupRefusal> {
        match self.account(account).map(|a| (&a.config.ews, &a.config.graph)) {
            Some((ews, _)) if ews.is_some() => {
                if self.own_group(account, group) {
                    Ok(())
                } else {
                    Err(GroupRefusal::Outlook)
                }
            }
            Some((_, graph)) if graph.is_some() => Err(GroupRefusal::Order),
            _ => Err(GroupRefusal::Server),
        }
    }

    /// Whether the server can move an existing calendar between groups (Graph v1.0 can't).
    pub fn calendar_group_move(&self, account: &str) -> Result<(), GroupRefusal> {
        self.calendar_groups_change(account)?;
        if self.account(account).is_some_and(|a| a.config.ews.is_some()) { Ok(()) } else { Err(GroupRefusal::Move) }
    }

    /// Whether `group` is a header Quatrelle wrote.
    fn own_group(&self, account: &str, group: &str) -> bool {
        self.pim.get(account).is_some_and(|p| p.own_calendar_groups.iter().any(|g| g.eq_ignore_ascii_case(group)))
    }

    /// Whether a calendar of the account is listed in `group`.
    fn group_has_calendars(&self, account: &str, group: &str) -> bool {
        self.pim.get(account).is_some_and(|p| p.calendars.iter().any(|c| c.group.eq_ignore_ascii_case(group)))
    }

    /// A group of `account` by its name, or why the params don't name one.
    fn calendar_group_named(&self, account: &str, group: &str) -> Result<(), CmdError> {
        if self.pim.get(account).is_some_and(|p| p.calendar_groups.iter().any(|g| g.eq_ignore_ascii_case(group))) {
            Ok(())
        } else {
            Err(CmdError::Params(format!("no calendar group `{group}` in `{account}`")))
        }
    }

    /// Make a calendar group called `name`: on the server, which the calendars read again after
    /// bring in.
    pub fn create_calendar_group(&mut self, account: &str, name: &str) -> CmdResult {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        self.calendar_groups_change(account).map_err(|r| CmdError::Disabled(r.reason().into()))?;
        let name = typed_name(name)?;
        if self.pim.get(account).is_some_and(|p| p.calendar_groups.iter().any(|g| g.eq_ignore_ascii_case(&name))) {
            return Err(CmdError::Params(format!("there's a calendar group called {name} already")));
        }
        if let Some(p) = self.pim.get_mut(account) {
            p.calendar_groups.push(name.clone());
            p.own_calendar_groups.push(name.clone());
        }
        self.pim_revision = self.pim_revision.wrapping_add(1);
        self.write_pim(account);
        self.jobs.push(Job::Apply { account: account.to_string(), op: Op::CreateCalendarGroup { name: name.clone() } });
        Ok(json!({"account": account, "group": name}))
    }

    /// Rename a calendar group: here at once, on the server through the journal (the next sync
    /// says what the server kept).
    pub fn rename_calendar_group(&mut self, account: &str, group: &str, name: &str) -> CmdResult {
        self.calendar_group_named(account, group)?;
        self.calendar_group_rename(account, group).map_err(|r| CmdError::Disabled(r.reason().into()))?;
        let name = typed_name(name)?;
        if name.eq_ignore_ascii_case(group) {
            return Ok(json!({"account": account, "group": group, "name": name}));
        }
        if self.pim.get(account).is_some_and(|p| p.calendar_groups.iter().any(|g| g.eq_ignore_ascii_case(&name))) {
            return Err(CmdError::Params(format!("there's a calendar group called {name} already")));
        }
        let p = self.pim_mut(account);
        for g in &mut p.calendar_groups {
            if g.eq_ignore_ascii_case(group) {
                g.clone_from(&name);
            }
        }
        for g in &mut p.own_calendar_groups {
            if g.eq_ignore_ascii_case(group) {
                g.clone_from(&name);
            }
        }
        for c in &mut p.calendars {
            if c.group.eq_ignore_ascii_case(group) {
                c.group.clone_from(&name);
            }
        }
        self.write_pim(account);
        self.jobs.push(Job::Apply { account: account.to_string(), op: Op::RenameCalendarGroup { group: group.to_string(), name: name.clone() } });
        Ok(json!({"account": account, "group": name, "was": group}))
    }

    /// Delete a calendar group: here at once (its calendars go back to My Calendars, or leave the
    /// list), on the server through the journal.
    pub fn delete_calendar_group(&mut self, account: &str, group: &str) -> CmdResult {
        self.calendar_group_named(account, group)?;
        self.calendar_group_delete(account, group).map_err(|r| CmdError::Disabled(r.reason().into()))?;
        let p = self.pim_mut(account);
        p.calendar_groups.retain(|g| !g.eq_ignore_ascii_case(group));
        p.own_calendar_groups.retain(|g| !g.eq_ignore_ascii_case(group));
        for c in &mut p.calendars {
            if c.group.eq_ignore_ascii_case(group) {
                c.group.clear();
            }
        }
        self.prefs.folded_calendar_groups.remove(group);
        self.write_pim(account);
        self.jobs.push(Job::Apply { account: account.to_string(), op: Op::DeleteCalendarGroup { group: group.to_string() } });
        Ok(json!({"account": account, "group": group, "deleted": true}))
    }

    /// Move a calendar group up (or down) among the groups, on the server through the journal.
    pub fn move_calendar_group(&mut self, account: &str, group: &str, up: bool) -> CmdResult {
        self.calendar_group_named(account, group)?;
        self.calendar_group_order(account, group).map_err(|r| CmdError::Disabled(r.reason().into()))?;
        let p = self.pim_mut(account);
        let Some(at) = p.calendar_groups.iter().position(|g| g.eq_ignore_ascii_case(group)) else {
            return Err(CmdError::Params(format!("no calendar group `{group}` in `{account}`")));
        };
        let there = if up {
            at.checked_sub(1).ok_or_else(|| CmdError::Disabled("it's the first calendar group already".into()))?
        } else {
            at.checked_add(1)
                .filter(|n| *n < p.calendar_groups.len())
                .ok_or_else(|| CmdError::Disabled("it's the last calendar group already".into()))?
        };
        p.calendar_groups.swap(at, there);
        self.write_pim(account);
        self.jobs.push(Job::Apply { account: account.to_string(), op: Op::MoveCalendarGroup { group: group.to_string(), up } });
        Ok(json!({"account": account, "group": group, "up": up}))
    }

    /// Put a calendar in a group (`group` empty: My Calendars): here at once, on the server
    /// through the journal.
    pub fn move_calendar_to_group(&mut self, account: &str, calendar: &str, group: &str) -> CmdResult {
        let c = self.calendar_named(account, calendar)?.clone();
        self.calendar_group_move(account).map_err(|r| CmdError::Disabled(r.reason().into()))?;
        let to = if group.is_empty() {
            if c.main {
                return Ok(json!({"account": account, "calendar": calendar, "group": ""}));
            }
            let my = self.pim.get(account).map(|p| p.my_calendar_group.clone()).unwrap_or_default();
            if my.is_empty() {
                return Err(CmdError::Disabled("the server doesn't name the group My Calendars is in".into()));
            }
            my
        } else {
            self.calendar_group_named(account, group)?;
            group.to_string()
        };
        if c.main {
            return Err(CmdError::Disabled("the account's main calendar stays in My Calendars".into()));
        }
        if let Some(c) = self.pim_mut(account).calendars.iter_mut().find(|x| x.id == calendar) {
            c.group = if group.is_empty() { String::new() } else { group.to_string() };
        }
        self.pim_revision = self.pim_revision.wrapping_add(1);
        self.write_pim(account);
        self.jobs.push(Job::Apply {
            account: account.to_string(),
            op: Op::MoveCalendarToGroup { calendar: calendar.to_string(), name: c.name, group: to },
        });
        Ok(json!({"account": account, "calendar": calendar, "group": group}))
    }
}
