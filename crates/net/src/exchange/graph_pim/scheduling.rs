//! Microsoft 365 scheduling through Graph: the organisation's rooms (`/places`, or the older
//! `findRoomLists`/`findRooms` where the sign-in hasn't `Place.Read.All`) and when people and
//! rooms are busy (`getSchedule`, the Scheduling Assistant's free/busy).

use quatrelle_model::scheduling::{Availability, BusyKind, BusySlot, MAX_PEOPLE, MAX_ROOMS, MAX_SLOTS, MAX_WINDOW, Room, RoomList, merge_slots};

use super::*;

/// Schedules one `getSchedule` asks about at most (Graph's limit).
const SCHEDULES_PER_CALL: usize = 20;
/// The merged view's step, in minutes.
const MERGED_STEP: i64 = 30;

/// A room or list as Graph gives it: `/places` names them `emailAddress` and `displayName`,
/// `findRooms` `address` and `name`.
fn room_of(v: &Value, list: &str) -> Option<Room> {
    let email = [s(v, "emailAddress"), s(v, "address")].into_iter().find(|e| e.contains('@'))?.trim().to_ascii_lowercase();
    if email.len() > 320 {
        return None;
    }
    let name: String = [s(v, "displayName"), s(v, "name")].into_iter().find(|n| !n.trim().is_empty()).unwrap_or_else(|| email.clone());
    Some(Room { name: name.trim().chars().take(200).collect(), email, list: list.to_string() })
}

/// The rooms of every list, by whichever of Graph's two ways the sign-in allows: the Places
/// API first, then the older functions. No lists at all is an empty answer (the directory has
/// none, or the organisation set none up).
pub fn rooms(c: &Client) -> Result<(Vec<RoomList>, Vec<Room>)> {
    let places = c.all("/places/microsoft.graph.roomList?$top=100", MAX_ROOMS);
    let (lists, by_list): (Vec<Value>, fn(&str) -> String) = match places {
        Ok(lists) => (lists, |email| format!("/places/{}/microsoft.graph.roomList/rooms?$top=100", seg(email))),
        Err(e) => {
            log::info!("the Places API didn't answer ({e}): the older room functions are asked");
            (c.all("/me/findRoomLists", MAX_ROOMS)?, |email| format!("/me/findRooms(RoomList='{}')", seg(email)))
        }
    };
    let lists: Vec<RoomList> =
        lists.iter().filter_map(|v| room_of(v, "")).map(|r| RoomList { email: r.email, name: r.name }).take(MAX_ROOMS).collect();
    let mut out: Vec<Room> = Vec::new();
    for l in &lists {
        let label = if l.name.is_empty() { l.email.as_str() } else { l.name.as_str() };
        match c.all(&by_list(&l.email), MAX_ROOMS) {
            Ok(found) => {
                for r in found.iter().filter_map(|v| room_of(v, label)) {
                    if !out.iter().any(|x| x.email == r.email) && out.len() < MAX_ROOMS {
                        out.push(r);
                    }
                }
            }
            Err(e) => log::info!("room list {}: {e}", l.email),
        }
    }
    // No lists: every room the directory knows, when the older function lists them.
    if lists.is_empty()
        && let Ok(all) = c.all("/me/findRooms", MAX_ROOMS)
    {
        out.extend(all.iter().filter_map(|v| room_of(v, "")).take(MAX_ROOMS));
    }
    Ok((lists, out))
}

/// The `getSchedule` body for these addresses between `from` and `to` (UTC).
pub(crate) fn schedule_request(emails: &[String], from: i64, to: i64) -> Value {
    json!({
        "schedules": emails.iter().take(SCHEDULES_PER_CALL).map(|e| e.trim()).collect::<Vec<_>>(),
        "startTime": date_time(from),
        "endTime": date_time(to),
        "availabilityViewInterval": MERGED_STEP,
    })
}

/// A `getSchedule` answer, one entry per address asked for (matched by `scheduleId`, else by
/// order): the items listed, else the availability digits laid over the window; a refusal
/// per address kept.
pub(crate) fn parse_schedule(v: &Value, emails: &[String], from: i64) -> Vec<Availability> {
    let answers: Vec<&Value> = v.get("value").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    emails
        .iter()
        .take(SCHEDULES_PER_CALL)
        .enumerate()
        .map(|(i, email)| {
            let email = email.trim().to_ascii_lowercase();
            let mut a = Availability { email: email.clone(), slots: Vec::new(), error: None };
            let found = answers.iter().find(|x| s(x, "scheduleId").trim().eq_ignore_ascii_case(&email)).or_else(|| answers.get(i)).copied();
            let Some(r) = found else {
                a.error = Some("the server gave no answer for this mailbox".into());
                return a;
            };
            if let Some(e) = r.get("error").filter(|e| !e.is_null()) {
                let why = [s(e, "message"), s(e, "responseCode")].into_iter().find(|x| !x.is_empty()).unwrap_or_else(|| "refused".into());
                a.error = Some(why.chars().take(300).collect());
                return a;
            }
            let mut slots = Vec::new();
            match r.get("scheduleItems").and_then(Value::as_array) {
                Some(items) => {
                    for it in items.iter().take(MAX_SLOTS) {
                        let (Some(start), Some(end)) = (when(it.get("start")), when(it.get("end"))) else { continue };
                        let subject = if it.get("isPrivate").and_then(Value::as_bool) == Some(true) { String::new() } else { s(it, "subject") };
                        slots.push(BusySlot { start, end, kind: BusyKind::parse(&s(it, "status")), subject: subject.chars().take(200).collect() });
                    }
                }
                None => {
                    for (n, d) in s(r, "availabilityView").trim().chars().take(MAX_SLOTS).enumerate() {
                        let kind = BusyKind::parse(&d.to_string());
                        if kind == BusyKind::Free {
                            continue;
                        }
                        let start = from.saturating_add(i64::try_from(n).unwrap_or(0).saturating_mul(MERGED_STEP * 60));
                        slots.push(BusySlot { start, end: start.saturating_add(MERGED_STEP * 60), kind, subject: String::new() });
                    }
                }
            }
            a.slots = merge_slots(slots);
            a
        })
        .collect()
}

/// When these people and rooms are busy between `from` and `to`, twenty at a time.
pub fn availability(c: &Client, emails: &[String], from: i64, to: i64) -> Result<Vec<Availability>> {
    let to = to.max(from.saturating_add(3600)).min(from.saturating_add(MAX_WINDOW));
    let mut out = Vec::new();
    for chunk in emails.iter().take(MAX_PEOPLE).cloned().collect::<Vec<_>>().chunks(SCHEDULES_PER_CALL) {
        let v = c.post("/me/calendar/getSchedule", &schedule_request(chunk, from, to))?;
        out.extend(parse_schedule(&v, chunk, from));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_names_everyone_in_utc() {
        let from = quatrelle_jmap::utc_date("2026-10-22T00:00:00Z").unwrap();
        let v = schedule_request(&["ana@contoso.example".into(), " room@contoso.example ".into()], from, from + 86_400);
        assert_eq!(v["schedules"], json!(["ana@contoso.example", "room@contoso.example"]));
        assert_eq!(v["startTime"], json!({"dateTime": "2026-10-22T00:00:00", "timeZone": "UTC"}));
        assert_eq!(v["endTime"]["dateTime"], "2026-10-23T00:00:00");
        assert_eq!(v["availabilityViewInterval"], 30);
    }

    #[test]
    fn answers_are_read_per_schedule() {
        let from = quatrelle_jmap::utc_date("2026-10-22T00:00:00Z").unwrap();
        let v = json!({"value": [
            {"scheduleId": "room@contoso.example", "availabilityView": "0022"},
            {"scheduleId": "ana@contoso.example", "scheduleItems": [
                {"status": "busy", "start": {"dateTime": "2026-10-22T09:00:00.0000000", "timeZone": "UTC"}, "end": {"dateTime": "2026-10-22T10:00:00.0000000", "timeZone": "UTC"}, "subject": "Budget"},
                {"status": "tentative", "start": {"dateTime": "2026-10-22T11:00:00", "timeZone": "UTC"}, "end": {"dateTime": "2026-10-22T12:00:00", "timeZone": "UTC"}, "subject": "Secret", "isPrivate": true},
                {"status": "busy", "start": {"dateTime": "junk"}, "end": {"dateTime": "junk"}},
            ]},
            {"scheduleId": "secret@contoso.example", "error": {"message": "No access.", "responseCode": "ErrorNoFreeBusyAccess"}},
        ]});
        let out = parse_schedule(
            &v,
            &["ana@contoso.example".into(), "ROOM@contoso.example".into(), "secret@contoso.example".into(), "nobody@x".into()],
            from,
        );
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].slots.len(), 2, "{:?}", out[0].slots);
        assert_eq!((out[0].slots[0].start, out[0].slots[0].kind, out[0].slots[0].subject.as_str()), (from + 9 * 3600, BusyKind::Busy, "Budget"));
        assert_eq!((out[0].slots[1].kind, out[0].slots[1].subject.as_str()), (BusyKind::Tentative, ""), "private: no subject");
        assert_eq!(out[1].email, "room@contoso.example");
        assert_eq!((out[1].slots.len(), out[1].slots[0].start, out[1].slots[0].end), (1, from + 3600, from + 2 * 3600));
        assert_eq!(out[2].error.as_deref(), Some("No access."));
        assert!(out[3].error.is_some());
        assert!(parse_schedule(&json!("junk"), &["a@x".into()], from)[0].error.is_some());
    }
}
