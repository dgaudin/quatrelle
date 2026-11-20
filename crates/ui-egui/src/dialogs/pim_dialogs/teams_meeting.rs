// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An appointment's Teams meeting, in its form: the Teams Meeting button of the window's ribbon
//! and its menu (add, remove, the options page), what the window asks before one can be made (the
//! permission, a new sign-in), and why Teams didn't make it; the block written at the end of its
//! notes (the link to join, the meeting ID and passcode, the phone, the options page), found again
//! whatever language the interface had when it was written (D10); the place, Join and the
//! conference the meeting brings; what removing it takes out, and what the window leaves to delete
//! when it goes without saving, or its calendar can't hold it any more.
//!
//! What is hostile here: the notes, which anyone may have written (an invitation pasted in, a
//! block edited by hand). A block starts only at a line that is a join line's words followed by
//! nothing but a link of Teams' own ([`teams_meeting_url`]); it goes on for the lines that start
//! with the words of the block's other lines, [`BLOCK_LINES`] at most, and an options link is
//! taken from it only when it is Teams' own too (and checked again as it's opened). The meeting's
//! values come from the engine, which keeps only plain ones and Teams' own links. The button
//! asks one meeting at a time (T5), and only for an appointment the user can keep one on.

use std::sync::OnceLock;

#[cfg(not(target_arch = "wasm32"))]
use quatrelle_engine::TeamsMeetingPath;
use quatrelle_engine::fill;
use quatrelle_engine::model::ical::Conference;
#[cfg(not(target_arch = "wasm32"))]
use quatrelle_engine::model::ical::{is_webex_link, links_in};
use quatrelle_engine::model::pim::teams::{Status, TEAMS_CONFERENCE_LABEL, TeamsMeeting, TeamsMeetingError, teams_meeting_url};

use super::*;

/// Why Add a Teams meeting is off, as the catalogs key it: the appointment holds Webex; it isn't
/// the user's to keep a meeting on.
#[cfg(not(target_arch = "wasm32"))]
const WEBEX_FIRST: &str = "This appointment has a Webex meeting or room: remove it first";
#[cfg(not(target_arch = "wasm32"))]
const NOT_YOURS: &str = "A Teams meeting can only be added to an appointment you organize";

/// What the Teams Meeting button offers.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TeamsButton {
    /// Add a Teams meeting.
    Add,
    /// A creation is on its way: nothing more is asked meanwhile.
    Working,
    /// The appointment holds its meeting: Remove the Teams meeting, and its options page.
    Has,
    /// Nothing can be added: why, as the catalogs key it (`""`: nothing to say).
    Off(&'static str),
}

/// What the button offers for the form, its calendar taking a meeting by `path`. Someone else's
/// event may carry any id: it's never the user's to remove, nor to add to.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn button_state(f: &EventForm, path: TeamsMeetingPath) -> TeamsButton {
    if f.teams_wait.is_some() {
        return TeamsButton::Working;
    }
    if !f.can_invite || !f.teams_allowed {
        return TeamsButton::Off(NOT_YOURS);
    }
    if f.teams_meeting_id.is_some() {
        return TeamsButton::Has;
    }
    match path {
        TeamsMeetingPath::Unavailable => TeamsButton::Off(NOT_YOURS),
        // D13: Exchange Online is to make its own there, at step 2.
        TeamsMeetingPath::Mailbox => TeamsButton::Off(""),
        TeamsMeetingPath::Standalone if has_webex(f) => TeamsButton::Off(WEBEX_FIRST),
        TeamsMeetingPath::Standalone => TeamsButton::Add,
    }
}

/// Whether the appointment holds Webex: its scheduled meeting, or a Webex link (a room's, as its
/// block put it) in its place or notes. Erased from both, a room's gives Teams back.
#[cfg(not(target_arch = "wasm32"))]
fn has_webex(f: &EventForm) -> bool {
    f.webex_id.is_some() || [&f.location, &f.description].into_iter().any(|text| links_in(text).iter().any(|l| is_webex_link(l)))
}

/// How the calendar chosen in the window takes a meeting. A saved appointment whose own calendar
/// isn't among those listed (another account's then chosen in its place) takes none.
#[cfg(not(target_arch = "wasm32"))]
fn chosen_path(f: &EventForm, s: &Session) -> TeamsMeetingPath {
    match f.calendars.get(f.calendar) {
        Some((account, calendar, _)) if *account == f.account_id() => s.teams_meeting_path(account, calendar),
        _ => TeamsMeetingPath::Unavailable,
    }
}

/// Teams Meeting, after Webex Meeting: a click adds a meeting, and the menu beside it offers that,
/// Remove the Teams meeting and its Meeting options (`busy`: names being checked or a meeting on
/// its way, and the button waits). A calendar chosen since that can't hold the meeting made here
/// takes it out first ([`calendar_changed`]). Not in the browser, where Teams isn't signed in to.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn teams_button(ui: &mut Ui, t: &Tokens, f: &mut EventForm, s: &Session, busy: bool, acts: &mut Vec<Action>) {
    let path = chosen_path(f, s);
    if let Some(p) = calendar_changed(f, path) {
        acts.push(Action::Run("event.teamsMeeting", p));
    }
    let state = button_state(f, path);
    let why = match state {
        TeamsButton::Off(key) if !key.is_empty() => Some(tl!(key)),
        _ => None,
    };
    let adds = !busy && state == TeamsButton::Add;
    let r = widgets::big_button(ui, Icon::Video, "Teams Meeting", t, adds);
    let r = match why {
        Some(w) => r.on_hover_text(w),
        None => r,
    };
    if r.clicked() {
        add(f, s, acts);
    }
    let r = ui.add_enabled_ui(!busy, |ui| widgets::big_more(ui, "Teams Meeting menu", t)).inner;
    egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        let r = ui.add_enabled(adds, egui::Button::new(tl!("Add a Teams meeting")));
        let r = match why {
            Some(w) => r.on_disabled_hover_text(w),
            None => r,
        };
        if r.clicked() {
            add(f, s, acts);
            ui.close();
        }
        if ui.add_enabled(state == TeamsButton::Has, egui::Button::new(tl!("Remove the Teams meeting"))).clicked() {
            if let Some(p) = remove(f) {
                acts.push(Action::Run("event.teamsMeeting", p));
            }
            ui.close();
        }
        // Only a page of Teams' own (T1), the link checked again as it's opened.
        let page = options_link(&f.description).filter(|l| teams_meeting_url(l));
        if ui.add_enabled(state == TeamsButton::Has && page.is_some(), egui::Button::new(tl!("Meeting options"))).clicked() {
            if let Some(page) = page {
                acts.push(Action::OpenUrl(page));
            }
            ui.close();
        }
    });
}

/// Add a Teams meeting, the button's click or its menu's: a title and a start first; then Teams
/// switched off or signed out opens Options › Teams, and the permission missing is asked for in
/// the window ([`refused`]); otherwise the window asks Teams for the meeting.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn add(f: &mut EventForm, s: &Session, acts: &mut Vec<Action>) {
    f.error = None;
    f.teams_prompt = None;
    if f.summary.trim().is_empty() || f.date.trim().is_empty() {
        f.error = Some(tl!("A meeting needs a title and a start").to_string());
        return;
    }
    match s.teams_meetings_usable() {
        Ok(()) => acts.push(Action::ForTeamsMeeting(f.window, add_params(f))),
        Err(e) => refused(f, &e, acts),
    }
}

/// `event.teamsMeeting`'s params for the appointment: its title and times, and always its account
/// and calendar (from `f.calendars.get(f.calendar)`), even for one saved already, whose save names
/// only its uid.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn add_params(f: &EventForm) -> Value {
    let p = f.params();
    let (account, calendar) = f.calendars.get(f.calendar).map(|(a, c, _)| (a.as_str(), c.as_str())).unwrap_or_default();
    json!({"add": {"summary": p["summary"], "start": p["start"], "end": p["end"], "allDay": p["allDay"], "account": account, "calendar": calendar}})
}

/// Why Teams didn't make the meeting, or can't now, in the window: Teams switched off or signed
/// out opens Options › Teams; the permission missing, or the sign-in refused, is asked for with its
/// button ([`prompt`]); anything else is said. The appointment stays as it was.
pub(crate) fn refused(f: &mut EventForm, e: &TeamsMeetingError, acts: &mut Vec<Action>) {
    match e {
        TeamsMeetingError::NotSignedIn => acts.push(Action::TeamsOptions),
        TeamsMeetingError::Consent => f.teams_prompt = Some(TeamsPrompt::Consent),
        TeamsMeetingError::SignIn => f.teams_prompt = Some(TeamsPrompt::SignIn),
        e => f.error = Some(error_text(e)),
    }
}

/// Why Teams didn't make the meeting, in the interface's language: Graph's status and code as
/// they came (D3), never its message.
pub(crate) fn error_text(e: &TeamsMeetingError) -> String {
    match e {
        TeamsMeetingError::NotSignedIn => tl!("Teams is signed out").to_string(),
        TeamsMeetingError::Consent => tl!("Teams meetings need your permission").to_string(),
        TeamsMeetingError::SignIn => tl!("Sign in to Teams again").to_string(),
        TeamsMeetingError::NotAllowed => tl!("Your Teams account can't create meetings").to_string(),
        TeamsMeetingError::Offline => tl!("The Teams meeting couldn't be created: no connection").to_string(),
        TeamsMeetingError::Untrusted => tl!("Teams gave a link Quatrelle doesn't trust").to_string(),
        TeamsMeetingError::Refused { status, code } if code.is_empty() => tf!("Teams answered {status}", status = status),
        TeamsMeetingError::Refused { status, code } => tf!("Teams answered {status}: {code}", status = status, code = code),
    }
}

/// What the window asks before Teams can make its meeting, under its ribbon: the permission
/// (Authorize Teams meetings), said while it's missing, or a new sign-in (Sign in, then done
/// with). Both open Microsoft's sign-in, and wait while one is under way.
pub(super) fn prompt(ui: &mut Ui, t: &Tokens, f: &mut EventForm, s: &Session, acts: &mut Vec<Action>) {
    let (asked, button, meetings) = match f.teams_prompt {
        // D17: kept while the permission is missing, whichever of Teams' answer and the worker's
        // word that it went comes first.
        Some(TeamsPrompt::Consent) if !s.prefs.teams.meetings => (TeamsMeetingError::Consent, tl!("Authorize Teams meetings"), true),
        Some(TeamsPrompt::SignIn) => (TeamsMeetingError::SignIn, tl!("Sign in"), false),
        _ => return,
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(error_text(&asked)).color(t.danger));
        let signing_in = s.teams.status == Status::SigningIn;
        if ui.add_enabled(!signing_in, egui::Button::new(button)).clicked() {
            acts.push(Action::Run("teams.signIn", json!({"meetings": meetings})));
            if f.teams_prompt == Some(TeamsPrompt::SignIn) {
                f.teams_prompt = None;
            }
        }
    });
    ui.add_space(4.0);
}

/// The calendar chosen takes a meeting by another path than Quatrelle's own (`path`: a Microsoft
/// 365 mailbox's, or someone else's): the meeting made in this window, or the one on its way,
/// can't stay. It's taken out as [`remove`] does (the one on its way given up), and the window
/// says to add one again. `event.teamsMeeting`'s params for it; none when there's no such meeting,
/// or the calendar takes it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn calendar_changed(f: &mut EventForm, path: TeamsMeetingPath) -> Option<Value> {
    if path == TeamsMeetingPath::Standalone || !(f.teams_fresh || f.teams_wait.is_some()) {
        return None;
    }
    f.error = Some(tl!("The Teams meeting was removed: add it again for this calendar").to_string());
    match f.teams_wait.take() {
        Some(ask) => {
            f.teams_removed = true;
            Some(json!({"remove": {"ask": ask}}))
        }
        None => remove(f),
    }
}

/// The most lines a meeting's block takes: the link to join; the meeting ID and passcode; the
/// phone; the options page.
pub(crate) const BLOCK_LINES: usize = 4;

/// The join line, as the catalogs key it.
const JOIN: &str = "Join the Teams meeting: {link}";
/// The options line.
const OPTIONS: &str = "Meeting options: {link}";
/// Each of the lines that may follow the join line in a block (the options line among them).
const FOLLOWING: [&str; 5] =
    ["Meeting ID: {id}", "Passcode: {passcode}", "By phone: {number}, conference ID {conference}", "By phone: {number}", OPTIONS];

/// A meeting's block found in the notes. Where it is, only the tests read: the window takes its
/// links, and [`without_block`] its lines.
pub(crate) struct Block {
    /// The line it starts at (the notes split at each `\n`).
    #[cfg(test)]
    pub at: usize,
    /// How many lines it takes.
    #[cfg(test)]
    pub lines: usize,
    /// The link to join the meeting, from its first line.
    pub join: String,
    /// The link to the meeting's options page, when its options line has one of Teams' own.
    #[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "the browser has no Teams Meeting button to open it from"))]
    pub options: Option<String>,
}

/// The block's lines for `m`, in the interface's language, each value as Teams gave it; a line
/// whose values are all missing is left out. Filled in as they are, not with `tf!`: its marks
/// around a value would go out in the invitation.
pub(crate) fn block_lines(m: &TeamsMeeting) -> Vec<String> {
    let mut lines = vec![fill(tl!("Join the Teams meeting: {link}"), &[("link", &m.join_url)])];
    let mut ids = Vec::new();
    if !m.meeting_id.is_empty() {
        ids.push(fill(tl!("Meeting ID: {id}"), &[("id", &m.meeting_id)]));
    }
    if !m.passcode.0.is_empty() {
        ids.push(fill(tl!("Passcode: {passcode}"), &[("passcode", &m.passcode.0)]));
    }
    if !ids.is_empty() {
        lines.push(ids.join("   "));
    }
    if !m.toll_number.is_empty() {
        lines.push(if m.conference_id.is_empty() {
            fill(tl!("By phone: {number}"), &[("number", &m.toll_number)])
        } else {
            fill(tl!("By phone: {number}, conference ID {conference}"), &[("number", &m.toll_number), ("conference", &m.conference_id)])
        });
    }
    if !m.options_url.is_empty() {
        lines.push(fill(tl!("Meeting options: {link}"), &[("link", &m.options_url)]));
    }
    lines
}

/// The first meeting's block in `notes`.
pub(crate) fn find_block(notes: &str) -> Option<Block> {
    let lines: Vec<&str> = notes.split('\n').collect();
    (0..lines.len()).find_map(|at| {
        let (n, join) = block_at(lines.get(at..).unwrap_or_default())?;
        let options = lines.get(at + 1..at + n).unwrap_or_default().iter().find_map(|l| link_after(l, &labels().options));
        Some(Block {
            #[cfg(test)]
            at,
            #[cfg(test)]
            lines: n,
            join: join.to_string(),
            options: options.map(str::to_string),
        })
    })
}

/// The notes without any meeting's block, the blank lines around it as for a Webex room's
/// ([`super::webex_rooms::without_blocks`]). Notes with no block come back as they were.
pub(crate) fn without_block(notes: &str) -> String {
    super::webex_rooms::without_blocks(notes, |lines| block_at(lines).map(|(n, _)| n))
}

/// The link to the options page of the meeting whose block is in `notes`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn options_link(notes: &str) -> Option<String> {
    find_block(notes).and_then(|b| b.options)
}

/// Teams' answer in the appointment: its block at the end of the notes, in place of any meeting's
/// block there; its link as the place when that's empty or the replaced block's link (one typed
/// stays); Join; its conference; its id, made in this window and not saved yet.
pub(crate) fn take_meeting(f: &mut EventForm, m: &TeamsMeeting) {
    let replaced = find_block(&f.description).map(|b| b.join);
    let notes = without_block(&f.description);
    let notes = notes.trim_end();
    let gap = if notes.is_empty() { "" } else { "\n\n" };
    f.description = format!("{notes}{gap}{}\n", block_lines(m).join("\n"));
    let place = f.location.trim();
    if place.is_empty() || replaced.as_deref() == Some(place) {
        f.location = m.join_url.clone();
    }
    f.join = Some(m.join_url.clone());
    if let Some(c) = Conference::new(&m.join_url, TEAMS_CONFERENCE_LABEL) {
        f.conference = Some(c);
        f.conference_shown = true;
    }
    f.teams_meeting_id = Some(m.id.clone());
    f.teams_fresh = true;
}

/// Takes the meeting out of the form: its block, the place when it's the meeting's link (one typed
/// stays), Join, its conference (the one the window opened with comes back, unless it was this
/// meeting's), its id; the user removed it. A meeting made in this window and never saved is
/// deleted at once: `event.teamsMeeting`'s params for it. The saved appointment's is deleted by
/// the engine when the appointment is saved without it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn remove(f: &mut EventForm) -> Option<Value> {
    let link = meeting_link(f);
    f.description = without_block(&f.description);
    if let Some(link) = &link {
        if f.location.trim() == link {
            f.location.clear();
        }
        if f.join.as_ref() == Some(link) {
            f.join = None;
        }
        if f.conference.as_ref().is_some_and(|c| c.url == *link) {
            f.conference = f.conference_opened.clone().filter(|c| c.url != *link);
            f.conference_shown = f.conference.as_ref().is_some_and(|c| super::event::link_shown(&c.url, &f.location, &f.description));
        }
    }
    f.teams_removed = true;
    let fresh = std::mem::take(&mut f.teams_fresh);
    f.teams_meeting_id.take().filter(|_| fresh).map(|id| json!({"remove": {"id": id}}))
}

/// `event.teamsMeeting`'s params for what the window leaves unsaved: the meeting made in it, or
/// the one on its way. Never the saved appointment's.
pub(crate) fn leaving(f: &EventForm) -> Option<Value> {
    match (&f.teams_meeting_id, f.teams_wait) {
        (Some(id), _) if f.teams_fresh => Some(json!({"remove": {"id": id}})),
        (_, Some(ask)) => Some(json!({"remove": {"ask": ask}})),
        _ => None,
    }
}

/// The link of the form's meeting: its block's, else its conference's when Quatrelle labelled it
/// Teams', else Join's; only one of Teams' own.
#[cfg(not(target_arch = "wasm32"))]
fn meeting_link(f: &EventForm) -> Option<String> {
    let conference = f.conference.as_ref().filter(|c| c.label == TEAMS_CONFERENCE_LABEL).map(|c| c.url.clone());
    find_block(&f.description).map(|b| b.join).or(conference).or_else(|| f.join.clone()).filter(|l| teams_meeting_url(l))
}

/// The words before each line's value, in every catalog's language.
struct Labels {
    join: Vec<String>,
    following: Vec<String>,
    options: Vec<String>,
}

/// The block's labels, read once: the catalogs don't change while Quatrelle runs.
fn labels() -> &'static Labels {
    static LABELS: OnceLock<Labels> = OnceLock::new();
    LABELS.get_or_init(|| Labels { join: words(JOIN), following: FOLLOWING.iter().flat_map(|k| words(k)).collect(), options: words(OPTIONS) })
}

/// What each translation of `key` says before its first value, the blanks after it taken off;
/// never empty (it would start every line).
fn words(key: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in crate::i18n::in_every_language(key) {
        let label = t.split('{').next().unwrap_or_default().trim_end();
        if !label.trim().is_empty() && !out.iter().any(|l| l == label) {
            out.push(label.to_string());
        }
    }
    out
}

/// How many lines a meeting's block takes at the start of `lines`, and its link to join, when one
/// starts there: a join line, then the lines that start as the block's others do, [`BLOCK_LINES`]
/// in all at most.
fn block_at<'a>(lines: &[&'a str]) -> Option<(usize, &'a str)> {
    let l = labels();
    let join = link_after(lines.first().copied()?, &l.join)?;
    let follows = lines.iter().skip(1).take(BLOCK_LINES - 1).take_while(|line| l.following.iter().any(|w| line.trim().starts_with(w.as_str())));
    Some((1 + follows.count(), join))
}

/// The link after one of `labels` on `line` (both trimmed), when it's all that follows and
/// Teams' own.
fn link_after<'a>(line: &'a str, labels: &[String]) -> Option<&'a str> {
    let line = line.trim();
    labels.iter().filter_map(|w| line.strip_prefix(w.as_str())).map(str::trim).find(|rest| teams_meeting_url(rest))
}
