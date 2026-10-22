//! EWS scheduling: the rooms a server offers (`GetRoomLists`, then `GetRooms` for each list,
//! as Outlook's Room Finder fills its lists) and when people and rooms are busy
//! (`GetUserAvailability`, which the Scheduling Assistant draws).
//!
//! Times are asked for and answered in a time zone with no bias at all, so everything is UTC:
//! the request's `TimeZone` is the fixed one that means UTC, and the answer's times, written
//! without a zone, are read as UTC.

use quatrelle_model::scheduling::{Availability, BusyKind, BusySlot, MAX_PEOPLE, MAX_ROOMS, MAX_SLOTS, MAX_WINDOW, Room, RoomList, merge_slots};

use super::*;

/// The merged view's step, in minutes (a digit per step).
const MERGED_STEP: i64 = 30;

/// A `GetRoomLists`/`GetRooms`/`GetUserAvailability` answer: the response element itself
/// carries the result (and its refusal), not a list of response messages.
fn answer<'a>(doc: &'a El, element: &str) -> Result<&'a El> {
    let m = crate::ews::operation(doc, element)?;
    crate::ews::check_message(m)?;
    // Some servers put the refusal in the code alone, without the class.
    match m.text_of("ResponseCode").map(str::trim) {
        Some(code) if code != "NoError" && !code.is_empty() => Err(NetError::No(m.text_of("MessageText").unwrap_or(code).trim().to_string())),
        _ => Ok(m),
    }
}

/// A mailbox element (`Name`, `EmailAddress`) as a room.
fn room_of(e: &El, list: &str) -> Option<Room> {
    let email = text(e, "EmailAddress").to_ascii_lowercase();
    if !email.contains('@') || email.len() > 320 {
        return None;
    }
    let name = text(e, "Name").chars().take(200).collect::<String>();
    Some(Room { name: if name.is_empty() { email.clone() } else { name }, email, list: list.to_string() })
}

/// The room lists the organisation set up: none where it set none up (many on-premises
/// servers), which isn't an error.
pub fn room_lists(c: &Client) -> Result<Vec<RoomList>> {
    let doc = c.call("<m:GetRoomLists/>")?;
    let m = answer(&doc, "GetRoomListsResponse")?;
    let mut out = Vec::new();
    for a in m.child("RoomLists").map(|l| l.children("Address").collect::<Vec<_>>()).unwrap_or_default().into_iter().take(MAX_ROOMS) {
        if let Some(r) = room_of(a, "") {
            out.push(RoomList { email: r.email, name: r.name });
        }
    }
    Ok(out)
}

/// The rooms in one room list.
pub fn rooms_in(c: &Client, list: &RoomList) -> Result<Vec<Room>> {
    let doc = c.call(&format!("<m:GetRooms><m:RoomList><t:EmailAddress>{}</t:EmailAddress></m:RoomList></m:GetRooms>", esc(&list.email)))?;
    let m = answer(&doc, "GetRoomsResponse")?;
    let label = if list.name.is_empty() { list.email.as_str() } else { list.name.as_str() };
    let mut out = Vec::new();
    for r in m.child("Rooms").map(|l| l.children("Room").collect::<Vec<_>>()).unwrap_or_default().into_iter().take(MAX_ROOMS) {
        if let Some(room) = r.child("Id").and_then(|id| room_of(id, label)) {
            out.push(room);
        }
    }
    Ok(out)
}

/// Every room list and every room in them, as the Room Finder lists them. A list that won't
/// open is left out (the others still count); no lists at all is an empty answer.
pub fn rooms(c: &Client) -> Result<(Vec<RoomList>, Vec<Room>)> {
    let lists = room_lists(c)?;
    let mut out: Vec<Room> = Vec::new();
    for l in &lists {
        match rooms_in(c, l) {
            Ok(found) => {
                for r in found {
                    if !out.iter().any(|x| x.email == r.email) && out.len() < MAX_ROOMS {
                        out.push(r);
                    }
                }
            }
            Err(e) => log::info!("room list {}: {e}", l.email),
        }
    }
    Ok((lists, out))
}

/// `2026-10-22T09:00:00`, as the request's time zone (UTC) has it: no zone suffix.
fn plain(t: i64) -> String {
    utc(t).trim_end_matches('Z').to_string()
}

/// The `GetUserAvailability` request for these addresses between `from` and `to`, in the
/// schema's order: the time zone (UTC: no bias, standard and daylight alike), the mailboxes,
/// then the view wanted (a detailed one where the person allows it, the merged digits
/// everywhere).
pub(crate) fn availability_request(emails: &[String], from: i64, to: i64) -> String {
    let zone = "<t:TimeZone><t:Bias>0</t:Bias>\
         <t:StandardTime><t:Bias>0</t:Bias><t:Time>02:00:00</t:Time><t:DayOrder>5</t:DayOrder><t:Month>10</t:Month><t:DayOfWeek>Sunday</t:DayOfWeek></t:StandardTime>\
         <t:DaylightTime><t:Bias>0</t:Bias><t:Time>02:00:00</t:Time><t:DayOrder>1</t:DayOrder><t:Month>4</t:Month><t:DayOfWeek>Sunday</t:DayOfWeek></t:DaylightTime>\
         </t:TimeZone>";
    let boxes: String = emails
        .iter()
        .take(MAX_PEOPLE)
        .map(|e| {
            format!(
                "<t:MailboxData><t:Email><t:Address>{}</t:Address></t:Email><t:AttendeeType>Required</t:AttendeeType>\
                 <t:ExcludeConflicts>false</t:ExcludeConflicts></t:MailboxData>",
                esc(e.trim())
            )
        })
        .collect();
    format!(
        "<m:GetUserAvailabilityRequest>{zone}<m:MailboxDataArray>{boxes}</m:MailboxDataArray>\
         <t:FreeBusyViewOptions><t:TimeWindow><t:StartTime>{}</t:StartTime><t:EndTime>{}</t:EndTime></t:TimeWindow>\
         <t:MergedFreeBusyIntervalInMinutes>{MERGED_STEP}</t:MergedFreeBusyIntervalInMinutes><t:RequestedView>DetailedMerged</t:RequestedView>\
         </t:FreeBusyViewOptions></m:GetUserAvailabilityRequest>",
        plain(from),
        plain(to)
    )
}

/// A `GetUserAvailability` answer, one entry per address asked for, in order: the calendar
/// events the server listed (its detailed view), else the merged digits laid over the window;
/// a mailbox refused keeps the reason.
pub(crate) fn parse_availability(doc: &El, emails: &[String], from: i64) -> Vec<Availability> {
    let responses: Vec<&El> = doc.find("FreeBusyResponseArray").map(|a| a.children("FreeBusyResponse").collect()).unwrap_or_default();
    emails
        .iter()
        .take(MAX_PEOPLE)
        .enumerate()
        .map(|(i, email)| {
            let mut a = Availability { email: email.trim().to_ascii_lowercase(), slots: Vec::new(), error: None };
            let Some(r) = responses.get(i) else {
                a.error = Some("the server gave no answer for this mailbox".into());
                return a;
            };
            if let Some(m) = r.child("ResponseMessage")
                && let Err(e) = crate::ews::check_message(m)
            {
                a.error = Some(e.to_string());
                return a;
            }
            let Some(view) = r.child("FreeBusyView") else {
                a.error = Some("no free/busy for this mailbox".into());
                return a;
            };
            let mut slots: Vec<BusySlot> = Vec::new();
            if let Some(events) = view.child("CalendarEventArray") {
                for e in events.children("CalendarEvent").take(MAX_SLOTS) {
                    let (Some(start), Some(end)) = (quatrelle_jmap::utc_date(&text(e, "StartTime")), quatrelle_jmap::utc_date(&text(e, "EndTime")))
                    else {
                        continue;
                    };
                    let subject = e.child("CalendarEventDetails").map(|d| text(d, "Subject").chars().take(200).collect()).unwrap_or_default();
                    slots.push(BusySlot { start, end, kind: BusyKind::parse(&text(e, "BusyType")), subject });
                }
            } else if let Some(merged) = view.text_of("MergedFreeBusy") {
                // A digit per step, from the window's start.
                for (n, d) in merged.trim().chars().take(MAX_SLOTS).enumerate() {
                    let kind = BusyKind::parse(&d.to_string());
                    if kind == BusyKind::Free {
                        continue;
                    }
                    let start = from.saturating_add(i64::try_from(n).unwrap_or(0).saturating_mul(MERGED_STEP * 60));
                    slots.push(BusySlot { start, end: start.saturating_add(MERGED_STEP * 60), kind, subject: String::new() });
                }
            } else if view.text_of("FreeBusyViewType").is_some_and(|v| v.trim() == "None") {
                a.error = Some("no free/busy for this mailbox".into());
            }
            a.slots = merge_slots(slots);
            a
        })
        .collect()
}

/// When these people and rooms are busy between `from` and `to`: a day at least (Exchange
/// won't answer for less), two months at most.
pub fn availability(c: &Client, emails: &[String], from: i64, to: i64) -> Result<Vec<Availability>> {
    let to = to.max(from.saturating_add(86_400)).min(from.saturating_add(MAX_WINDOW));
    let doc = c.call(&availability_request(emails, from, to))?;
    // The whole request refused (a response element with an error and no array).
    if doc.find("FreeBusyResponseArray").is_none() {
        let m = crate::ews::operation(&doc, "GetUserAvailabilityResponse")?;
        crate::ews::check_message(m)?;
        if let Some(code) = m.text_of("ResponseCode").map(str::trim).filter(|c| *c != "NoError" && !c.is_empty()) {
            return Err(NetError::No(m.text_of("MessageText").unwrap_or(code).trim().to_string()));
        }
    }
    Ok(parse_availability(&doc, emails, from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ews::parse;

    /// The request keeps the schema's order (the server refuses any other): the time zone, the
    /// mailboxes, the view; everything in UTC, written without a zone.
    #[test]
    fn the_request_is_in_schema_order_and_utc() {
        let from = quatrelle_jmap::utc_date("2026-10-22T00:00:00Z").unwrap();
        let x = availability_request(&["Ana <ana@contoso.example>".into(), "room.lyon@contoso.example".into()], from, from + 86_400);
        let at = |needle: &str| x.find(needle).unwrap_or_else(|| panic!("no {needle} in {x}"));
        assert!(at("<t:TimeZone>") < at("<m:MailboxDataArray>") && at("<m:MailboxDataArray>") < at("<t:FreeBusyViewOptions>"));
        assert!(
            at("<t:TimeWindow>") < at("<t:MergedFreeBusyIntervalInMinutes>") && at("<t:MergedFreeBusyIntervalInMinutes>") < at("<t:RequestedView>")
        );
        assert!(x.contains("<t:StartTime>2026-10-22T00:00:00</t:StartTime><t:EndTime>2026-10-23T00:00:00</t:EndTime>"));
        assert!(x.contains("<t:Address>Ana &lt;ana@contoso.example&gt;</t:Address>"), "escaped as given");
        assert_eq!(x.matches("<t:MailboxData>").count(), 2);
    }

    /// Events come as the detailed view lists them, the merged digits where that's all the
    /// server gives, a refusal per mailbox, and an answer missing for one.
    #[test]
    fn answers_are_read_per_mailbox() {
        let doc = parse(
            "<s:Envelope xmlns:s=\"s\"><s:Body><m:GetUserAvailabilityResponse xmlns:m=\"m\" xmlns:t=\"t\"><m:FreeBusyResponseArray>\
             <m:FreeBusyResponse><m:ResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode></m:ResponseMessage>\
             <m:FreeBusyView><t:FreeBusyViewType>Detailed</t:FreeBusyViewType><t:CalendarEventArray>\
             <t:CalendarEvent><t:StartTime>2026-10-22T09:00:00</t:StartTime><t:EndTime>2026-10-22T10:00:00</t:EndTime><t:BusyType>Busy</t:BusyType>\
             <t:CalendarEventDetails><t:Subject>Budget</t:Subject></t:CalendarEventDetails></t:CalendarEvent>\
             <t:CalendarEvent><t:StartTime>2026-10-22T10:00:00</t:StartTime><t:EndTime>2026-10-22T11:00:00</t:EndTime><t:BusyType>Busy</t:BusyType>\
             <t:CalendarEventDetails><t:Subject>Budget</t:Subject></t:CalendarEventDetails></t:CalendarEvent>\
             <t:CalendarEvent><t:StartTime>junk</t:StartTime><t:EndTime>2026-10-22T11:00:00</t:EndTime><t:BusyType>Busy</t:BusyType></t:CalendarEvent>\
             <t:CalendarEvent><t:StartTime>2026-10-22T14:00:00</t:StartTime><t:EndTime>2026-10-22T15:00:00</t:EndTime><t:BusyType>OOF</t:BusyType></t:CalendarEvent>\
             </t:CalendarEventArray></m:FreeBusyView></m:FreeBusyResponse>\
             <m:FreeBusyResponse><m:ResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode></m:ResponseMessage>\
             <m:FreeBusyView><t:FreeBusyViewType>MergedOnly</t:FreeBusyViewType><t:MergedFreeBusy>0022x1</t:MergedFreeBusy></m:FreeBusyView></m:FreeBusyResponse>\
             <m:FreeBusyResponse><m:ResponseMessage ResponseClass=\"Error\"><m:MessageText>No free/busy access</m:MessageText>\
             <m:ResponseCode>ErrorNoFreeBusyAccess</m:ResponseCode></m:ResponseMessage></m:FreeBusyResponse>\
             </m:FreeBusyResponseArray></m:GetUserAvailabilityResponse></s:Body></s:Envelope>",
        );
        let from = quatrelle_jmap::utc_date("2026-10-22T00:00:00Z").unwrap();
        let emails =
            ["ana@contoso.example".to_string(), "room@contoso.example".into(), "secret@contoso.example".into(), "nobody@contoso.example".into()];
        let out = parse_availability(&doc, &emails, from);
        assert_eq!(out.len(), 4);
        let ana = &out[0];
        assert_eq!(ana.error, None);
        assert_eq!(ana.slots.len(), 2, "{:?}", ana.slots);
        assert_eq!(
            (ana.slots[0].start, ana.slots[0].end, ana.slots[0].kind, ana.slots[0].subject.as_str()),
            (from + 9 * 3600, from + 11 * 3600, BusyKind::Busy, "Budget")
        );
        assert_eq!((ana.slots[1].kind, ana.slots[1].start), (BusyKind::OutOfOffice, from + 14 * 3600));
        let room = &out[1];
        assert_eq!(room.slots.len(), 3, "{:?}", room.slots);
        assert_eq!((room.slots[0].start, room.slots[0].end, room.slots[0].kind), (from + 3600, from + 2 * 3600, BusyKind::Busy));
        assert_eq!((room.slots[1].kind, room.slots[2].kind), (BusyKind::NoData, BusyKind::Tentative));
        assert!(out[2].error.as_deref().is_some_and(|e| e.contains("free/busy")) && out[2].slots.is_empty());
        assert!(out[3].error.is_some(), "no answer for the fourth");
    }

    /// Hostile input: thousands of events are capped, and junk is no event.
    #[test]
    fn answers_are_capped() {
        let mut events = String::new();
        for i in 0..5000 {
            events.push_str(&format!(
                "<t:CalendarEvent><t:StartTime>2026-10-22T09:00:00</t:StartTime><t:EndTime>2026-10-22T10:00:00</t:EndTime><t:BusyType>Busy</t:BusyType>\
                 <t:CalendarEventDetails><t:Subject>M{i}</t:Subject></t:CalendarEventDetails></t:CalendarEvent>"
            ));
        }
        let doc = parse(&format!(
            "<s:Envelope xmlns:s=\"s\"><s:Body><m:GetUserAvailabilityResponse xmlns:m=\"m\" xmlns:t=\"t\"><m:FreeBusyResponseArray>\
             <m:FreeBusyResponse><m:FreeBusyView><t:CalendarEventArray>{events}</t:CalendarEventArray></m:FreeBusyView></m:FreeBusyResponse>\
             </m:FreeBusyResponseArray></m:GetUserAvailabilityResponse></s:Body></s:Envelope>"
        ));
        let out = parse_availability(&doc, &["a@x".into()], 0);
        assert!(out[0].slots.len() <= MAX_SLOTS && !out[0].slots.is_empty());
        let empty = parse_availability(&parse("<x/>"), &["a@x".into()], 0);
        assert!(empty[0].error.is_some());
    }
}
