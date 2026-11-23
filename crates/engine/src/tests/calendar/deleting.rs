// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: an appointment deleted while the sync that makes it on the server is on its way
//! is held until the server says where it made it, then deleted there as any other, and never
//! comes back; one deleted before any sync read it never goes. On-premises Exchange (EWS),
//! Exchange Online (Graph) and CalDAV, whose syncs all make what's new, then read the calendar
//! back.

use quatrelle_model::dates::Clock;
use quatrelle_model::ical::CalEvent;
use quatrelle_model::pim::{CalendarInfo, DavSync, PIM_DOC, Pim, StoredEvent};

use super::*;

/// The kind of server the account's calendar is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Server {
    Ews,
    Graph,
    CalDav,
}

const DAV_CALENDAR: &str = "https://dav.example.org/cal/work/";

impl Server {
    /// The account's main calendar, as its sync names it.
    fn calendar(self) -> &'static str {
        match self {
            Server::Ews => "ews:calendar",
            Server::Graph => "AAMkMain=",
            Server::CalDav => DAV_CALENDAR,
        }
    }

    fn home(self) -> &'static str {
        match self {
            Server::Ews => "ews",
            Server::Graph => "graph",
            Server::CalDav => "https://dav.example.org/cal/",
        }
    }

    /// Where the server makes an appointment sent to it, and the UID it reads it back under:
    /// Exchange gives it one of its own (a single appointment's with its start after a `#`),
    /// CalDAV keeps the one sent.
    fn item(self, e: &CalEvent) -> (String, String) {
        let exchange = "040000008200E00074C5B7101A82E00800000000F1A9D3";
        match self {
            Server::Ews if e.rrule.is_none() => ("AAMkMade=".into(), format!("{exchange}#{}", e.start)),
            Server::Ews => ("AAMkMade=".into(), exchange.into()),
            Server::Graph => ("AAMkMadeEvent=".into(), exchange.into()),
            Server::CalDav => (format!("{DAV_CALENDAR}made.ics"), e.uid.clone()),
        }
    }
}

/// An account on `server`, its calendar read once (empty): where new appointments go.
fn session(name: &str, server: Server) -> (Session, String) {
    let mut s = Session::open(Some(temp(name)), Clock::new(NOW, 0));
    let add = match server {
        Server::CalDav => json!({"email": "zoe@contoso.example", "password": "pw"}),
        Server::Ews | Server::Graph => {
            json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"})
        }
    };
    let id = s.run("account.add", &add).unwrap()["account"].as_str().unwrap().to_string();
    if server == Server::Graph
        && let Some(a) = s.accounts.iter_mut().find(|a| a.config.id == id)
    {
        a.config.ews = None;
        a.config.graph = Some(quatrelle_model::GRAPH_URL.into());
    }
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(server, "state-1", Vec::new(), Vec::new(), Vec::new())) });
    s.take_jobs();
    (s, id)
}

/// A sync's result: the calendar read (up to `state`) with `read` in it, what the sync made on
/// the server (`made`), and what it deleted there.
fn result(server: Server, state: &str, read: Vec<StoredEvent>, made: Vec<StoredEvent>, deleted: Vec<String>) -> DavSync {
    let calendar = server.calendar();
    DavSync {
        caldav_home: Some(server.home().into()),
        calendars: Some(vec![CalendarInfo {
            id: calendar.into(),
            name: "Calendar".into(),
            ctag: state.into(),
            main: server != Server::CalDav,
            ..Default::default()
        }]),
        synced_calendars: vec![calendar.into()],
        events: read,
        pushed_events: made,
        deleted,
        checked: NOW,
        ..Default::default()
    }
}

/// The account's calendars as saved: what a sync that starts now reads.
fn as_saved(s: &Session, id: &str) -> Pim {
    s.store.as_ref().unwrap().load_account_doc(id, PIM_DOC).unwrap()
}

/// What a sync that read `saved` does: each appointment never sent made on the server, and read
/// back from the calendar as the server keeps it. (What it made, what it read.)
fn made(server: Server, saved: &Pim) -> (Vec<StoredEvent>, Vec<StoredEvent>) {
    let mut made = Vec::new();
    let mut read = Vec::new();
    for e in saved.events.iter().filter(|e| e.dirty && e.href.is_empty() && e.event.recurrence_id.is_none()) {
        let (href, uid) = server.item(&e.event);
        let calendar = server.calendar().to_string();
        made.push(StoredEvent { calendar: calendar.clone(), href: href.clone(), etag: "ck-1".into(), dirty: false, notify: false, ..e.clone() });
        read.push(StoredEvent { calendar, href, etag: "ck-1".into(), event: CalEvent { uid, ..e.event.clone() }, ..Default::default() });
    }
    (made, read)
}

/// What the session asked of the network since it was last looked at: how many syncs of the
/// account, and the calendars mailed (each one's method).
fn asked(s: &mut Session, id: &str) -> (usize, Vec<String>) {
    let jobs = s.take_jobs();
    let syncs = jobs.iter().filter(|j| matches!(j, Job::SyncDav { account } if account == id)).count();
    let mailed = jobs
        .iter()
        .filter_map(|j| match j {
            Job::Send { raw, .. } => {
                let text = String::from_utf8_lossy(raw);
                ["REQUEST", "CANCEL"].into_iter().find(|m| text.contains(&format!("method={m}"))).map(str::to_string)
            }
            _ => None,
        })
        .collect();
    (syncs, mailed)
}

/// A meeting the account's user organizes, with a guest (`repeat`: a series); its UID.
fn meeting(s: &mut Session, id: &str, repeat: Option<&str>) -> String {
    let mut params =
        json!({"summary": "Budget", "start": "2026-10-21T15:00", "end": "2026-10-21T15:30", "attendees": "Ana <ana@contoso.example>", "account": id});
    if let Some(r) = repeat {
        params["repeat"] = json!(r);
    }
    s.run("event.create", &params).unwrap()["uid"].as_str().unwrap().to_string()
}

/// The copies of the meeting the account keeps: by its item on the server, or by its title.
fn kept(s: &Session, id: &str, href: &str) -> usize {
    s.pim[id].events.iter().filter(|e| e.href == href || e.event.summary == "Budget").count()
}

/// A meeting made from half-hours chosen in the calendar, then deleted at once: the sync that
/// made it on the server (inviting its guests) was on its way. It went, came back with that
/// sync's result (the server's copy, read back with the calendar), and went for good only when
/// deleted a second time, which is what deleted it on the server. Held until the server says
/// where it made it, it's deleted there at the next sync as any other, and never comes back;
/// nothing is made twice, and the guests are told once.
fn deleted_while_it_went_up(server: Server, name: &str) {
    let (mut s, id) = session(name, server);
    let uid = meeting(&mut s, &id, None);
    let (syncs, mut mailed) = asked(&mut s, &id);
    assert_eq!(syncs, 1, "{server:?}: the meeting goes to the server");
    // The sync starts: the meeting, as saved, made on the server and read back from it.
    let (made, read) = made(server, &as_saved(&s, &id));
    let href = made.first().map(|e| e.href.clone()).unwrap();
    // Deleted meanwhile.
    s.run("event.delete", &json!({"uid": uid})).unwrap();
    assert_eq!(kept(&s, &id, &href), 0);
    let (_, m) = asked(&mut s, &id);
    mailed.extend(m);
    // The sync's result comes in.
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(server, "state-2", read.clone(), made.clone(), Vec::new())) });
    assert_eq!(kept(&s, &id, &href), 0, "{server:?}: the meeting deleted here doesn't come back");
    assert!(s.agenda(NOW, NOW + 30 * 86_400).iter().all(|a| a.summary != "Budget"), "{server:?}: not in the calendar");
    // Nor with the result of another sync that read it before it was deleted (and, given the item
    // made, sent it to that item again).
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(server, "state-2", read, made, Vec::new())) });
    assert_eq!(kept(&s, &id, &href), 0, "{server:?}: nor with a later result");
    // Deleted on the server by the next sync, as a meeting deleted here is.
    let (syncs, m) = asked(&mut s, &id);
    mailed.extend(m);
    assert_eq!(syncs, 1, "{server:?}: a sync deletes it on the server");
    let saved = as_saved(&s, &id);
    assert_eq!(saved.deleted.iter().map(|t| t.href.as_str()).collect::<Vec<_>>(), [href.as_str()], "{server:?}: its deletion goes");
    assert!(saved.events.iter().all(|e| !e.dirty && e.href != href), "{server:?}: nothing made again");
    // That sync's result: deleted there, the calendar read without it.
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(server, "state-3", Vec::new(), Vec::new(), vec![href.clone()])) });
    assert_eq!(kept(&s, &id, &href), 0, "{server:?}");
    assert!(s.pim[&id].deleted.is_empty(), "{server:?}: deleted once");
    let (syncs, m) = asked(&mut s, &id);
    mailed.extend(m);
    assert_eq!(syncs, 0, "{server:?}: nothing more to send");
    // The guests: invited once (by the server where it does that, else by mail), and told once
    // that it's off.
    let expected: &[&str] = if server == Server::CalDav { &["REQUEST", "CANCEL"] } else { &["CANCEL"] };
    assert_eq!(mailed, expected, "{server:?}");
    done(s);
}

#[test]
fn a_meeting_deleted_while_exchange_made_it_stays_deleted_and_goes_from_the_server() {
    deleted_while_it_went_up(Server::Ews, "deleting-flight-ews");
}

#[test]
fn a_meeting_deleted_while_microsoft_365_made_it_stays_deleted_and_goes_from_the_server() {
    deleted_while_it_went_up(Server::Graph, "deleting-flight-graph");
}

#[test]
fn a_meeting_deleted_while_caldav_made_it_stays_deleted_and_goes_from_the_server() {
    deleted_while_it_went_up(Server::CalDav, "deleting-flight-caldav");
}

/// A meeting already on the server, deleted while a sync that read the calendar before was on its
/// way: that sync's result doesn't bring it back, and the next one deletes it there.
fn deleted_while_a_sync_read_it(server: Server, name: &str) {
    let (mut s, id) = session(name, server);
    meeting(&mut s, &id, None);
    let (made, read) = made(server, &as_saved(&s, &id));
    let href = made.first().map(|e| e.href.clone()).unwrap();
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(server, "state-2", read.clone(), made, Vec::new())) });
    let uid = s.pim[&id].events.iter().find(|e| e.href == href).map(|e| e.event.uid.clone()).unwrap();
    asked(&mut s, &id);
    // A sync starts and reads the calendar (the meeting changed there: a guest answered); deleted
    // meanwhile.
    s.run("event.delete", &json!({"uid": uid})).unwrap();
    assert_eq!(asked(&mut s, &id).0, 1, "{server:?}: its deletion goes");
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(server, "state-3", read, Vec::new(), Vec::new())) });
    assert_eq!(kept(&s, &id, &href), 0, "{server:?}: not brought back by what the sync read before");
    assert_eq!(as_saved(&s, &id).deleted.iter().map(|t| t.href.as_str()).collect::<Vec<_>>(), [href.as_str()], "{server:?}: still to delete");
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(server, "state-4", Vec::new(), Vec::new(), vec![href.clone()])) });
    assert!(kept(&s, &id, &href) == 0 && s.pim[&id].deleted.is_empty(), "{server:?}");
    done(s);
}

#[test]
fn a_meeting_exchange_has_deleted_while_a_sync_read_it_stays_deleted() {
    deleted_while_a_sync_read_it(Server::Ews, "deleting-read-ews");
}

#[test]
fn a_meeting_microsoft_365_has_deleted_while_a_sync_read_it_stays_deleted() {
    deleted_while_a_sync_read_it(Server::Graph, "deleting-read-graph");
}

#[test]
fn a_meeting_caldav_has_deleted_while_a_sync_read_it_stays_deleted() {
    deleted_while_a_sync_read_it(Server::CalDav, "deleting-read-caldav");
}

/// Deleted before any sync read it: it never goes, nothing comes back from the syncs that began
/// before or after, and nothing is deleted on the server. Made again here under the same UID
/// (an invitation's file opened again), it's made on the server, and kept, as any other.
fn deleted_before_it_went(server: Server, name: &str) {
    let (mut s, id) = session(name, server);
    let uid = meeting(&mut s, &id, None);
    s.run("event.delete", &json!({"uid": uid})).unwrap();
    let (syncs, _) = asked(&mut s, &id);
    assert_eq!(syncs, 1, "{server:?}: the sync asked for by the meeting, still to start");
    let saved = as_saved(&s, &id);
    assert!(saved.events.iter().all(|e| e.event.uid != uid) && saved.deleted.is_empty(), "{server:?}: nothing for that sync to make or delete");
    // The results of a sync that began before the meeting was made, and of the one it asked for.
    for state in ["state-2", "state-3"] {
        s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(server, state, Vec::new(), Vec::new(), Vec::new())) });
        assert!(s.find_event(&uid).is_none() && s.pim[&id].deleted.is_empty(), "{server:?}");
        assert_eq!(asked(&mut s, &id).0, 0, "{server:?}: nothing to send");
    }
    // Made again under the same UID: made there, and kept.
    let again = CalEvent { uid: uid.clone(), summary: "Budget".into(), start: NOW + 86_400, end: NOW + 88_200, ..Default::default() };
    s.create_event(Some(&id), None, again).unwrap();
    let (made, read) = made(server, &as_saved(&s, &id));
    let href = made.first().map(|e| e.href.clone()).unwrap();
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(server, "state-4", read, made, Vec::new())) });
    assert_eq!(kept(&s, &id, &href), 1, "{server:?}: the one made again stays");
    assert!(s.pim[&id].events.iter().any(|e| e.href == href && !e.dirty), "{server:?}: made on the server");
    assert!(s.pim[&id].deleted.is_empty(), "{server:?}: and not deleted there");
    done(s);
}

#[test]
fn an_appointment_deleted_before_exchange_read_it_never_goes() {
    deleted_before_it_went(Server::Ews, "deleting-before-ews");
}

#[test]
fn an_appointment_deleted_before_microsoft_365_read_it_never_goes() {
    deleted_before_it_went(Server::Graph, "deleting-before-graph");
}

#[test]
fn an_appointment_deleted_before_caldav_read_it_never_goes() {
    deleted_before_it_went(Server::CalDav, "deleting-before-caldav");
}

/// One occurrence of a new weekly meeting deleted while the sync made the series: the series
/// takes its item on the server, keeps the occurrence out, and the change goes to the server's
/// series; the guests are told of that occurrence once.
fn occurrence_deleted_while_it_went_up(server: Server, name: &str) {
    let (mut s, id) = session(name, server);
    let uid = meeting(&mut s, &id, Some("weekly"));
    let first = s.find_event(&uid).unwrap().1.event.start;
    let (_, mut mailed) = asked(&mut s, &id);
    let (made, read) = made(server, &as_saved(&s, &id));
    let href = made.first().map(|e| e.href.clone()).unwrap();
    s.run("event.delete", &json!({"uid": uid, "occurrence": first + 7 * 86_400})).unwrap();
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(server, "state-2", read, made, Vec::new())) });
    let starts: Vec<i64> = s.agenda(first - 1, first + 21 * 86_400).into_iter().filter(|a| a.summary == "Budget").map(|a| a.start).collect();
    assert_eq!(starts, [first, first + 14 * 86_400], "{server:?}: the occurrence stays out");
    let (_, master) = s.find_event(&uid).unwrap();
    assert!(master.href == href && master.dirty && master.event.exdates == [first + 7 * 86_400], "{server:?}: {master:?}");
    let (syncs, m) = asked(&mut s, &id);
    mailed.extend(m);
    assert!(syncs >= 1, "{server:?}: the change goes to the server's series");
    let expected: &[&str] = if server == Server::CalDav { &["REQUEST", "CANCEL"] } else { &["CANCEL"] };
    assert_eq!(mailed, expected, "{server:?}");
    done(s);
}

#[test]
fn an_occurrence_deleted_while_exchange_made_its_series_stays_out() {
    occurrence_deleted_while_it_went_up(Server::Ews, "deleting-occurrence-ews");
}

#[test]
fn an_occurrence_deleted_while_microsoft_365_made_its_series_stays_out() {
    occurrence_deleted_while_it_went_up(Server::Graph, "deleting-occurrence-graph");
}

#[test]
fn an_occurrence_deleted_while_caldav_made_its_series_stays_out() {
    occurrence_deleted_while_it_went_up(Server::CalDav, "deleting-occurrence-caldav");
}
