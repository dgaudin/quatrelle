//! EWS calendars: appointments read as events (their recurrence in Graph's terms) and written back as fields.

use super::*;

/// An EWS `Recurrence` element as Graph's pattern and range.
pub fn recurrence_json(r: &El) -> Option<Value> {
    let p = r.kids.iter().find(|k| k.name.ends_with("Recurrence") && !k.name.contains("End") && !k.name.contains("Numbered"))?;
    let kind = match p.name.as_str() {
        "DailyRecurrence" => "daily",
        "WeeklyRecurrence" => "weekly",
        "AbsoluteMonthlyRecurrence" => "absoluteMonthly",
        "RelativeMonthlyRecurrence" => "relativeMonthly",
        "AbsoluteYearlyRecurrence" => "absoluteYearly",
        "RelativeYearlyRecurrence" => "relativeYearly",
        _ => return None,
    };
    let num = |n: &str| p.text_of(n).and_then(|v| v.trim().parse::<u64>().ok());
    let days: Vec<String> = p.text_of("DaysOfWeek").unwrap_or("").split_whitespace().map(str::to_ascii_lowercase).collect();
    let month = p.text_of("Month").and_then(|m| MONTHS.iter().position(|x| x.eq_ignore_ascii_case(m.trim()))).map(|i| i + 1);
    let pattern = json!({
        "type": kind,
        "interval": num("Interval").unwrap_or(1),
        "daysOfWeek": days,
        "index": p.text_of("DayOfWeekIndex").unwrap_or("first").trim().to_ascii_lowercase(),
        "month": month.unwrap_or(0),
        "dayOfMonth": num("DayOfMonth").unwrap_or(0),
    });
    let date = |e: &El, n: &str| e.text_of(n).map(|d| d.trim().get(..10).unwrap_or("").to_string());
    let range = if let Some(e) = r.child("EndDateRecurrence") {
        json!({"type": "endDate", "startDate": date(e, "StartDate"), "endDate": date(e, "EndDate")})
    } else if let Some(e) = r.child("NumberedRecurrence") {
        json!({"type": "numbered", "startDate": date(e, "StartDate"), "numberOfOccurrences": e.text_of("NumberOfOccurrences").and_then(|n| n.trim().parse::<u64>().ok()).unwrap_or(1)})
    } else {
        json!({"type": "noEnd"})
    };
    Some(json!({"pattern": pattern, "range": range}))
}

/// An RRULE as an EWS `Recurrence` element (starting on the event's day).
pub fn recurrence_xml(rule: &str, start: i64) -> Option<String> {
    let r = recurrence_of(rule, start)?;
    let p = &r["pattern"];
    let interval = p["interval"].as_u64().unwrap_or(1).max(1);
    let day = |d: &Value| d.as_str().and_then(|n| DAYS.iter().find(|x| x.eq_ignore_ascii_case(n))).copied();
    let days = p["daysOfWeek"].as_array().map(|a| a.iter().filter_map(day).collect::<Vec<_>>().join(" ")).unwrap_or_default();
    let index = p["index"].as_str().map(|i| {
        let mut c = i.chars();
        c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
    });
    let month = p["month"].as_u64().and_then(|m| MONTHS.get(usize::try_from(m).ok()?.checked_sub(1)?)).copied().unwrap_or("January");
    let mday = p["dayOfMonth"].as_u64().unwrap_or(1);
    let pattern = match p["type"].as_str()? {
        "daily" => format!("<t:DailyRecurrence><t:Interval>{interval}</t:Interval></t:DailyRecurrence>"),
        "weekly" => format!("<t:WeeklyRecurrence><t:Interval>{interval}</t:Interval><t:DaysOfWeek>{days}</t:DaysOfWeek></t:WeeklyRecurrence>"),
        "absoluteMonthly" => format!(
            "<t:AbsoluteMonthlyRecurrence><t:Interval>{interval}</t:Interval><t:DayOfMonth>{mday}</t:DayOfMonth></t:AbsoluteMonthlyRecurrence>"
        ),
        "relativeMonthly" => format!(
            "<t:RelativeMonthlyRecurrence><t:Interval>{interval}</t:Interval><t:DaysOfWeek>{days}</t:DaysOfWeek><t:DayOfWeekIndex>{}</t:DayOfWeekIndex></t:RelativeMonthlyRecurrence>",
            index.unwrap_or_else(|| "First".into())
        ),
        "absoluteYearly" => {
            format!("<t:AbsoluteYearlyRecurrence><t:DayOfMonth>{mday}</t:DayOfMonth><t:Month>{month}</t:Month></t:AbsoluteYearlyRecurrence>")
        }
        "relativeYearly" => format!(
            "<t:RelativeYearlyRecurrence><t:DaysOfWeek>{days}</t:DaysOfWeek><t:DayOfWeekIndex>{}</t:DayOfWeekIndex><t:Month>{month}</t:Month></t:RelativeYearlyRecurrence>",
            index.unwrap_or_else(|| "First".into())
        ),
        _ => return None,
    };
    let rg = &r["range"];
    let first = rg["startDate"].as_str().unwrap_or("");
    let range = match rg["type"].as_str() {
        Some("endDate") => format!(
            "<t:EndDateRecurrence><t:StartDate>{first}</t:StartDate><t:EndDate>{}</t:EndDate></t:EndDateRecurrence>",
            rg["endDate"].as_str().unwrap_or(first)
        ),
        Some("numbered") => format!(
            "<t:NumberedRecurrence><t:StartDate>{first}</t:StartDate><t:NumberOfOccurrences>{}</t:NumberOfOccurrences></t:NumberedRecurrence>",
            rg["numberOfOccurrences"].as_u64().unwrap_or(1)
        ),
        _ => format!("<t:NoEndRecurrence><t:StartDate>{first}</t:StartDate></t:NoEndRecurrence>"),
    };
    Some(format!("<t:Recurrence>{pattern}{range}</t:Recurrence>"))
}

fn person(m: &El) -> Option<Attendee> {
    let mb = if m.name == "Mailbox" { m } else { m.find("Mailbox")? };
    let email = text(mb, "EmailAddress");
    email.contains('@').then(|| Attendee { email, name: text(mb, "Name"), partstat: String::new(), rsvp: false })
}

/// An appointment as an event (occurrences and changed instances are left to their series).
pub fn event_of(it: &El) -> Option<CalEvent> {
    if matches!(it.text_of("CalendarItemType"), Some("Occurrence" | "Exception")) {
        return None;
    }
    let all_day = it.text_of("IsAllDayEvent").is_some_and(|v| v == "true");
    let start = quatrelle_jmap::utc_date(&text(it, "Start"))?;
    let end = quatrelle_jmap::utc_date(&text(it, "End")).unwrap_or(start + 3600).max(start);
    // All-day times are local midnights: the nearest UTC midnight is the day.
    let day = |t: i64| (t + 43_200).div_euclid(86_400) * 86_400;
    let (start, end) = if all_day { (day(start), day(end).max(day(start) + 86_400)) } else { (start, end) };
    let body = it.child("Body");
    let content = body.map(|b| b.text.clone()).unwrap_or_default();
    let html = body.and_then(|b| b.attr("BodyType")).is_some_and(|t| t.eq_ignore_ascii_case("HTML"));
    // A Teams or Zoom invitation's join link, whole, from the HTML (its text wraps long ones).
    let join_url = if html {
        quatrelle_mime::html_links(&content).into_iter().find(|l| quatrelle_model::ical::is_meeting_link(l)).unwrap_or_default()
    } else {
        String::new()
    };
    let description = if html { quatrelle_mime::html_to_text(&content) } else { content };
    // Outlook's reminder: set, and how long before the start.
    let reminder = (it.text_of("ReminderIsSet") == Some("true"))
        .then(|| it.text_of("ReminderMinutesBeforeStart").and_then(|m| m.trim().parse::<i64>().ok()))
        .flatten();
    let mut attendees = Vec::new();
    for list in ["RequiredAttendees", "OptionalAttendees"] {
        if let Some(l) = it.child(list) {
            for a in l.children("Attendee").take(500) {
                if let Some(mut p) = person(a) {
                    p.partstat = match a.text_of("ResponseType").unwrap_or("") {
                        "Accept" | "Organizer" => "ACCEPTED",
                        "Decline" => "DECLINED",
                        "Tentative" => "TENTATIVE",
                        _ => "NEEDS-ACTION",
                    }
                    .to_string();
                    p.rsvp = true;
                    attendees.push(p);
                }
            }
        }
    }
    let id = it.child("ItemId").and_then(|i| i.attr("Id")).unwrap_or("").to_string();
    let uid = [text(it, "UID"), id].into_iter().find(|u| !u.is_empty())?;
    // Single items can share a UID: some organizers' series reach Exchange as so many separate
    // appointments, all keeping the series' UID (an Exchange 2016 server does this), and a UID
    // kept alone would leave but one of them. The start makes each its own appointment — and
    // stays the same when the item moves to another calendar.
    let uid = if it.text_of("CalendarItemType") == Some("Single") { format!("{uid}#{start}") } else { uid };
    Some(CalEvent {
        uid,
        summary: text(it, "Subject"),
        description: description.trim().to_string(),
        join_url,
        reminder,
        location: text(it, "Location"),
        start,
        end,
        all_day,
        rrule: it.child("Recurrence").and_then(recurrence_json).as_ref().and_then(rrule_of),
        organizer: it.child("Organizer").and_then(person),
        attendees,
        status: if it.text_of("IsCancelled") == Some("true") { "CANCELLED".into() } else { String::new() },
        categories: it
            .child("Categories")
            .map(|c| c.children("String").map(|s| s.text.trim().to_string()).filter(|s| !s.is_empty()).take(20).collect())
            .unwrap_or_default(),
        ..Default::default()
    })
}

/// The fields of an appointment, in the order EWS wants them, each with the field URI that names
/// it in an update.
///
/// The URI's prefix is the type the property is declared on, and Exchange refuses the whole
/// `UpdateItem` with "The request is invalid." when one is wrong: `item:Subject`, `item:Body` and
/// `item:Categories` are on `ItemType`, while `calendar:Start`, `calendar:End`,
/// `calendar:IsAllDayEvent`, `calendar:Location` and `calendar:Recurrence` are on
/// `CalendarItemType`. A reminder sent as `calendar:ReminderIsSet` (it's `item:`) did just that,
/// and the change stayed on this computer, Outlook on the old day. Each field now carries its
/// URI instead of it being guessed from the field's name.
pub(super) fn event_fields(e: &CalEvent) -> Vec<(&'static str, String)> {
    let (start, end) = if e.all_day { (e.start, e.end.max(e.start + 86_400)) } else { (e.start, e.end.max(e.start)) };
    let mut f = vec![
        ("item:Subject", format!("<t:Subject>{}</t:Subject>", esc(&e.summary))),
        ("item:Body", format!("<t:Body BodyType=\"Text\">{}</t:Body>", esc(&e.description))),
    ];
    // Categories come between the item's fields and the appointment's (EWS keeps the schema's
    // order); none at all is left out (an empty list isn't allowed).
    if !e.categories.is_empty() {
        let list: String = e.categories.iter().map(|c| format!("<t:String>{}</t:String>", esc(c))).collect();
        f.push(("item:Categories", format!("<t:Categories>{list}</t:Categories>")));
    }
    f.extend([
        ("calendar:Start", format!("<t:Start>{}</t:Start>", utc(start))),
        ("calendar:End", format!("<t:End>{}</t:End>", utc(end))),
        ("calendar:IsAllDayEvent", format!("<t:IsAllDayEvent>{}</t:IsAllDayEvent>", e.all_day)),
        ("calendar:Location", format!("<t:Location>{}</t:Location>", esc(&e.location))),
    ]);
    if let Some(r) = e.rrule.as_deref().and_then(|r| recurrence_xml(r, e.start)) {
        f.push(("calendar:Recurrence", r));
    }
    // After Recurrence in EWS's schema, as Outlook writes it.
    f.push(("item:ReminderIsSet", format!("<t:ReminderIsSet>{}</t:ReminderIsSet>", e.reminder.is_some())));
    if let Some(m) = e.reminder {
        f.push(("item:ReminderMinutesBeforeStart", format!("<t:ReminderMinutesBeforeStart>{m}</t:ReminderMinutesBeforeStart>")));
    }
    f
}
