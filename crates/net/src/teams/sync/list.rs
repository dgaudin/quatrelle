// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The list pass: the chat list read until a chat older than the newest activity known (to its
//! end at the hourly pass), a page a step. Each chat is kept in the copy with its read state; one
//! new to the copy, or whose `lastUpdatedDateTime` moved (a rename, a member change), has its
//! members read; one whose last message is newer than its watermark is read. At the hourly pass
//! the chats active in the last 30 days have their members and messages read, older ones once a
//! week, and a chat of the copy the whole list no longer has is left. A chat Graph won't serve
//! has its new last message said as news ([`news`]).

use super::reads::looked;
use super::*;
use crate::teams::graph::{Page, chat_of};
use crate::teams::store::chat_hash;
use quatrelle_model::pim::teams::{CHAT_PAGES, News};

/// The chats older than the month refreshed at the hourly pass: once a week.
const WEEK_MS: i64 = 7 * DAY_MS;

/// The longest a listing (the list's pages) is waited for: one held longer (a page that keeps
/// failing, a 5xx or a 429 on its link) is given up and begun again.
const LISTING_MAX: Duration = Duration::from_secs(600);
/// The chat list refused (403, an account without Teams): asked again an hour later.
const LIST_RETRY: Duration = Duration::from_secs(3600);
const NO_CHATS: &str = "Teams didn't give this account's chats (403): sign in with the account you use Teams with";

/// A list pass: its pages, then the chats it found that need reading.
pub(super) struct Pass {
    /// When it began ([`LISTING_MAX`]).
    begun: Instant,
    /// The hourly pass: the whole list, the month's chats refreshed, the absent ones left.
    whole: bool,
    /// The newest activity known before it: a page that reaches it is the last.
    known: Option<i64>,
    /// Its pages are read: the chats' work goes on.
    listed: bool,
    next: Option<String>,
    pages: usize,
    /// The links followed and the chats seen, by keyed hashes: a link met twice, or a page with
    /// nobody new, ends the listing.
    links: HashSet<u64>,
    seen: HashSet<u64>,
    /// The hash each chat listed is kept under in the copy (the hourly pass only).
    hashes: HashSet<String>,
    /// What the last whole list that came to its end by itself lacked, taken as this whole one
    /// begins: what it lacks too is left; cut short, it leaves nothing lacked for the next.
    lacked: HashSet<String>,
    /// The list came to its end by itself: a chat the copy has and it hasn't is no longer the
    /// user's.
    ended: bool,
    pub(super) round: Round,
    /// The hourly refresh found by the whole list's pages.
    refresh: VecDeque<Task>,
}

/// Whether the chat list is refused (an account-wide 403) at `now`. Meanwhile no chat is read,
/// the history neither: a refusal then is the account's, and says nothing of a chat (none is
/// marked for it). The cadences are passed, so that they don't hold the loop's deadline.
pub(super) fn list_refused(w: &mut Worker, now: Instant) -> bool {
    let Some(until) = w.sync.list_refused else { return false };
    if now >= until {
        w.sync.list_refused = None;
        return false;
    }
    for c in [Cadence::Open, Cadence::List, Cadence::Active, Cadence::Hourly] {
        if w.limiter.due(c, now) {
            w.limiter.ran(c, now);
        }
    }
    let s = &mut w.sync;
    (s.open, s.active, s.hourly) = (None, None, None);
    s.reads.clear();
    true
}

/// The list: a page a step, then the chats it found that need reading. Refused (403: an account
/// without Teams), it is said, and asked again an hour later. A page that follows a link refused
/// otherwise (a 400 on a link Graph no longer takes) ends the listing there, as a link elsewhere
/// does: what it found is read, and nothing is taken for gone. A listing still under way after
/// [`LISTING_MAX`] is begun again; the chats' work after it isn't held to that (each chat's fails
/// on its own and the next one goes).
pub(super) fn list_step(w: &mut Worker, now: Instant) -> bool {
    if w.sync.pass.as_ref().is_some_and(|p| !p.listed && now.saturating_duration_since(p.begun) > LISTING_MAX) {
        log::warn!("teams: the list of conversations was still being read after ten minutes; it is read again from its start");
        w.sync.pass = None;
    }
    if w.sync.pass.is_none() {
        let hourly = w.limiter.due(Cadence::Hourly, now);
        if !hourly && !w.limiter.due(Cadence::List, now) {
            return false;
        }
        let lacked = if hourly { std::mem::take(&mut w.sync.absent) } else { HashSet::new() };
        w.sync.pass = Some(Pass {
            begun: now,
            whole: hourly,
            known: w.sync.newest,
            listed: false,
            next: None,
            pages: 0,
            links: HashSet::new(),
            seen: HashSet::new(),
            hashes: HashSet::new(),
            lacked,
            ended: false,
            round: Round::default(),
            refresh: VecDeque::new(),
        });
    }
    if w.sync.pass.as_ref().is_some_and(|p| p.listed) {
        return run_round(w, Which::List, now);
    }
    if !w.may(Route::ChatList, now) {
        return true;
    }
    #[cfg(test)]
    w.trip_list();
    let next = w.sync.pass.as_ref().and_then(|p| p.next.clone());
    match w.request(Route::ChatList, now, |c| c.chats(next.as_deref())) {
        Ok(page) => listed(w, page, now),
        Err(e) if http_code(&e) == Some(403) => {
            log::warn!("teams: the list of conversations was refused ({}); asked again in an hour", kind(&e));
            w.sync.pass = None;
            w.sync.list_refused = now.checked_add(LIST_RETRY);
            w.limiter.ran(Cadence::List, now);
            w.limiter.ran(Cadence::Hourly, now);
            w.status(Status::Failed { why: NO_CHATS.into() });
        }
        Err(e) if next.is_some() && matches!(e, NetError::Protocol(_)) => {
            log::warn!("teams: a page of the list of conversations was refused ({}); the listing ends there", kind(&e));
            list_over(w, now);
        }
        Err(e) => w.failed(Cadence::List, "the list of conversations", &e, now),
    }
    true
}

/// A page of the list read: each chat kept, what it needs noted; the next page, or the listing's
/// end (the list is up to date, and at the hourly pass the chats it lacks are left).
fn listed(w: &mut Worker, page: Page, now: Instant) {
    let chats: Vec<Chat> = page.items.iter().take(MAX_PAGE_ITEMS).filter_map(chat_of).filter(|c| id_ok(&c.id)).collect();
    let (had_next, next_key) = (page.next.is_some(), page.next.as_deref().map(|n| w.key_of(n)));
    // Graph's own address only: a link elsewhere ends the listing (never followed with the token).
    let owned = page.next.as_deref().is_some_and(|n| w.client.as_ref().is_some_and(|c| c.own(n).is_ok()));
    if had_next && !owned {
        log::warn!("teams: a page of the list of conversations links outside Graph's address; the listing ends there");
    }
    // A page cut at the most items taken: what it had past them isn't known, so not its end.
    let (mut fresh, mut reached, mut capped) = (false, false, page.items.len() > MAX_PAGE_ITEMS);
    let (mut writes, mut rows, mut announced) = (Vec::new(), Vec::new(), Vec::new());
    for chat in chats {
        let key = w.key_of(&chat.id);
        let Some(pass) = w.sync.pass.as_mut() else { return };
        if pass.seen.len() >= MAX_CHATS {
            capped = true;
            break;
        }
        if !pass.seen.insert(key) {
            continue;
        }
        fresh = true;
        // Its link as the copy will keep it (the list's, else the one kept before).
        let url = chat.web_url.clone().or_else(|| w.copy.as_ref().and_then(|c| c.chat(&chat.id)).and_then(|r| r.chat.web_url));
        w.sync.links.note(key, &chat.kind, url.as_deref());
        let Some(pass) = w.sync.pass.as_mut() else { return };
        if pass.whole {
            pass.hashes.insert(chat_hash(&chat.id));
        }
        reached |= pass.known.is_some_and(|k| chat.last_activity.is_none_or(|at| at <= k));
        let whole = pass.whole;
        let s = &mut w.sync;
        if s.activity.len() < MAX_CHATS || s.activity.contains_key(&chat.id) {
            s.activity.insert(chat.id.clone(), chat.last_activity);
        }
        s.newest = s.newest.max(chat.last_activity);
        let (id, write, task, refresh, said) = take_chat(w, chat, whole);
        writes.extend(write);
        announced.extend(said);
        match (task, w.sync.pass.as_mut()) {
            // New activity: read now, refreshed with it when the hour asks.
            (Some(mut task), Some(pass)) => {
                if refresh {
                    task.members = true;
                    task.read = true;
                }
                pass.round.todo.push_back(task);
            }
            (None, Some(pass)) => {
                if refresh {
                    pass.refresh.push_back(Task { chat: id.clone(), members: true, updated: None, read: true, guarded: true, preview: false });
                }
                rows.push(id);
            }
            (_, None) => rows.push(id),
        }
    }
    // The page's chats kept in one go; then their rows (read back from the copy).
    if let Some(copy) = &w.copy
        && let Err(e) = copy.put_chats(&writes)
    {
        log::warn!("teams: a page of conversations couldn't be kept: {}", kind(&NetError::Store(e)));
    }
    news::say(w, announced);
    for id in rows {
        dirty(w, &id);
    }
    let Some(pass) = w.sync.pass.as_mut() else { return };
    pass.pages = pass.pages.saturating_add(1);
    let more = fresh && !capped && pass.pages < CHAT_PAGES && (pass.whole || !reached);
    pass.next = page.next.filter(|n| more && owned && n.len() <= MAX_LINK && next_key.is_some_and(|k| pass.links.insert(k)));
    if pass.next.is_some() {
        return;
    }
    pass.ended = !had_next && !capped && !pass.seen.is_empty();
    list_over(w, now);
}

/// A listing is over (by its end, or cut short): the list is up to date, the hour's refresh is
/// noted, and when the whole list came to its end by itself, the chats it lacks are looked at.
fn list_over(w: &mut Worker, now: Instant) {
    let Some(pass) = w.sync.pass.as_mut() else { return };
    pass.next = None;
    pass.listed = true;
    let (whole, ended, count, pages) = (pass.whole, pass.ended, pass.seen.len(), pass.pages);
    let refresh = std::mem::take(&mut pass.refresh);
    if w.sync.listed_said != Some((count, pages)) {
        w.sync.listed_said = Some((count, pages));
        log::info!("teams: {count} conversations listed in {pages} pages");
    }
    if let Some(line) = w.sync.links.changed() {
        log::info!("{line}");
    }
    w.up_to_date();
    if whole {
        // The hour's pass: its refresh goes after the other cadences, a request a step.
        w.limiter.ran(Cadence::Hourly, now);
        match &mut w.sync.hourly {
            Some(_) => log::info!("teams: the last hour's refresh isn't over; {} conversations wait for the next", refresh.len()),
            None if !refresh.is_empty() => w.sync.hourly = Some(Round { todo: refresh }),
            None => {}
        }
        if ended {
            left_behind(w);
        }
    }
    flush(w);
}

/// A chat as the list gives it: the list's fields (read state, hidden, topic, preview) over what
/// the copy has, whose members, `lastUpdatedDateTime` (until the members are read again) and
/// refused or left marks stay. Its id, its record when that changed (to be written), what it
/// needs now (members, a read; none: only its row), whether the hour's refresh is its (the
/// month's chats, older ones once a week), and its new last message's news (a chat Graph won't
/// serve; from a sender the members don't have, said once they're read again).
fn take_chat(w: &Worker, listed: Chat, whole: bool) -> (String, Option<ChatRecord>, Option<Task>, bool, Option<News>) {
    let server = server_ms(w);
    let stored = w.copy.as_ref().and_then(|c| c.chat(&listed.id));
    let updated = listed.updated;
    let mut r = match &stored {
        Some(s) => ChatRecord { chat: merged(&s.chat, listed), ..s.clone() },
        None => ChatRecord::new(Chat { members: Vec::new(), left: false, unreadable: None, updated: None, ..listed }),
    };
    if let Some(s) = &stored
        && s.chat.last_read != r.chat.last_read
    {
        // Read elsewhere (or marked unread): what came before it no longer counts.
        let after = r.chat.last_read.unwrap_or(i64::MIN);
        r.unread.retain(|(_, created)| *created > after);
    }
    let before = stored.as_ref().map(|s| &s.chat);
    let moved = before.is_none_or(|s| s.updated != updated);
    let stranger = match r.chat.preview.as_ref().map(|p| &p.sender) {
        Some(Sender::User { id, .. }) => !r.chat.members.iter().any(|m| m.user_id.eq_ignore_ascii_case(id)) && !looked(w, &r.chat.id, id),
        _ => false,
    };
    let members = moved || stranger;
    let new = stored.as_ref().is_some_and(|s| news::new_preview(w, s, &r.chat));
    // One marked left that the whole list has again is read: served, it is back.
    let read = wants_read(&r, before, server) || (whole && r.chat.left && server.is_some());
    let refresh = whole
        && !r.chat.left
        && r.chat.unreadable.is_none()
        && server.is_some()
        && (recent(r.chat.last_activity, server, MONTH_MS) || r.read_at.is_none_or(|t| !recent(Some(t), server, WEEK_MS)));
    let task = (members || read).then(|| Task {
        chat: r.chat.id.clone(),
        members,
        updated: moved.then_some(updated),
        read,
        guarded: false,
        preview: new && stranger,
    });
    let said = if new && !stranger { news::of_preview(w, &r.chat) } else { None };
    let write = (stored.as_ref() != Some(&r)).then(|| r.clone());
    (r.chat.id, write, task, refresh, said)
}

/// A listed chat over the copy's: the copy's members, refused or left marks and
/// `lastUpdatedDateTime` stay; a preview, a time or an address the list doesn't give stays too.
fn merged(stored: &Chat, listed: Chat) -> Chat {
    Chat {
        members: stored.members.clone(),
        left: stored.left,
        unreadable: stored.unreadable,
        updated: stored.updated,
        preview: listed.preview.or_else(|| stored.preview.clone()),
        last_activity: listed.last_activity.or(stored.last_activity),
        web_url: listed.web_url.or_else(|| stored.web_url.clone()),
        tenant_id: listed.tenant_id.or_else(|| stored.tenant_id.clone()),
        ..listed
    }
}

/// Whether the list says a chat is to be read: its last message newer than its watermark (held
/// to Graph's clock and an hour, whatever the copy says); one Graph doesn't serve, once a day by
/// the server's clock; one the user left, only on new activity (they may be back).
fn wants_read(r: &ChatRecord, before: Option<&Chat>, server: Option<i64>) -> bool {
    let Some(server) = server else { return false };
    let at = r.chat.preview.as_ref().map(|p| p.at);
    if r.chat.left {
        let was = before.and_then(|b| b.preview.as_ref()).map(|p| p.at);
        return at.is_some_and(|at| was.is_none_or(|was| at > was));
    }
    if let Some(tried) = r.chat.unreadable {
        return server.saturating_sub(tried) >= DAY_MS;
    }
    let cap = server.saturating_add(CLOCK_TOLERANCE_MS);
    at.is_some_and(|at| r.watermark.is_none_or(|w| at > w.min(cap)))
}

/// The hourly pass, the whole list read to its end: the chats of the copy it lacks, and the whole
/// list before it lacked too (ended by itself as well), are no longer the user's (lacked once, one
/// may have moved in the list while it was read). Kept, read-only, and said.
fn left_behind(w: &mut Worker) {
    let Some((hashes, before)) = w.sync.pass.as_mut().map(|p| (std::mem::take(&mut p.hashes), std::mem::take(&mut p.lacked))) else {
        return;
    };
    let gone: Vec<ChatRecord> = match &w.copy {
        Some(c) => c.absent(&hashes).into_iter().filter(|r| !r.chat.left).collect(),
        None => return,
    };
    w.sync.absent = gone.iter().map(|r| chat_hash(&r.chat.id)).take(MAX_CHATS).collect();
    let now_left: Vec<ChatRecord> = gone
        .iter()
        .filter(|r| before.contains(&chat_hash(&r.chat.id)))
        .map(|r| ChatRecord { chat: Chat { left: true, ..r.chat.clone() }, ..r.clone() })
        .collect();
    if let Some(copy) = &w.copy
        && let Err(e) = copy.put_chats(&now_left)
    {
        log::warn!("teams: the conversations no longer listed couldn't be kept: {}", kind(&NetError::Store(e)));
        return;
    }
    for r in &now_left {
        log::info!("teams: conversation {} is no longer in the list; kept to read", w.hashed(&r.chat.id));
        w.say(quatrelle_model::pim::teams::Event::Removed { chat: r.chat.id.clone() });
        dirty(w, &r.chat.id);
        crate::teams::outbox::left(w, &r.chat.id);
    }
}
