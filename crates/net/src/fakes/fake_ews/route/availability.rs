// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The fake EWS server's free/busy (`GetUserAvailability`).

use super::*;

impl Data {
    /// A `GetUserAvailability` answer: as Exchange checks the request (the schema's order, a
    /// time zone, a window of a day or more), then each mailbox's calendar between the dates —
    /// its own calendar items, another's folder here, and the busy stretches given — or its
    /// refusal.
    pub(super) fn availability(&mut self, op: &El) -> String {
        let names: Vec<&str> = op.kids.iter().map(|k| k.name.as_str()).collect();
        if names != ["TimeZone", "MailboxDataArray", "FreeBusyViewOptions"] {
            return "<m:ResponseCode>ErrorSchemaValidation</m:ResponseCode><m:MessageText>The request failed schema validation</m:MessageText>"
                .into();
        }
        let window = op.find("TimeWindow");
        let (start, end) = (
            window.and_then(|w| w.text_of("StartTime")).unwrap_or("").trim().to_string(),
            window.and_then(|w| w.text_of("EndTime")).unwrap_or("").trim().to_string(),
        );
        let (Some(from), Some(to)) = (quatrelle_jmap::utc_date(&start), quatrelle_jmap::utc_date(&end)) else {
            return "<m:ResponseCode>ErrorInvalidTimeInterval</m:ResponseCode><m:MessageText>The time window is invalid</m:MessageText>".into();
        };
        if to - from < 86_400 || to - from > 62 * 86_400 || start.ends_with('Z') {
            return "<m:ResponseCode>ErrorInvalidTimeInterval</m:ResponseCode><m:MessageText>The time window must be a day to two months</m:MessageText>".into();
        }
        let emails: Vec<String> = op
            .find("MailboxDataArray")
            .map(|a| a.children("MailboxData").filter_map(|m| m.find("Address").map(|x| x.text.trim().to_ascii_lowercase())).collect())
            .unwrap_or_default();
        self.availability_asked.push((emails.clone(), start, end));
        let own_calendar = self.folders.iter().find(|f| f.known.as_deref() == Some("calendar") && f.owner.is_empty()).map(|f| f.id.clone());
        let mut out = String::new();
        for email in &emails {
            if self.no_freebusy.iter().any(|e| e.eq_ignore_ascii_case(email)) {
                out.push_str(
                    "<m:FreeBusyResponse><m:ResponseMessage ResponseClass=\"Error\"><m:MessageText>No free/busy access</m:MessageText>\
                     <m:ResponseCode>ErrorNoFreeBusyAccess</m:ResponseCode></m:ResponseMessage></m:FreeBusyResponse>",
                );
                continue;
            }
            let folder = if email == "riley@contoso.example" { own_calendar.clone() } else { Some(format!("mailbox:{email}")) };
            let mut events: Vec<(String, String, String, String)> = self
                .pim
                .iter()
                .filter(|(_, f, el)| Some(f) == folder.as_ref() && el.name == "CalendarItem")
                .filter_map(|(_, _, el)| {
                    Some((
                        el.text_of("Start")?.trim().trim_end_matches('Z').to_string(),
                        el.text_of("End")?.trim().trim_end_matches('Z').to_string(),
                        el.text_of("LegacyFreeBusyStatus").unwrap_or("Busy").trim().to_string(),
                        el.text_of("Subject").unwrap_or("").to_string(),
                    ))
                })
                .collect();
            events.extend(
                self.taken.iter().filter(|(e, ..)| e.eq_ignore_ascii_case(email)).map(|(_, s, e, k, subject)| {
                    (s.trim_end_matches('Z').to_string(), e.trim_end_matches('Z').to_string(), k.clone(), subject.clone())
                }),
            );
            let items: String = events
                .iter()
                .filter(|(s, e, _, _)| quatrelle_jmap::utc_date(e).is_some_and(|e| e > from) && quatrelle_jmap::utc_date(s).is_some_and(|s| s < to))
                .map(|(s, e, kind, subject)| {
                    format!(
                        "<t:CalendarEvent><t:StartTime>{s}</t:StartTime><t:EndTime>{e}</t:EndTime><t:BusyType>{kind}</t:BusyType>\
                         <t:CalendarEventDetails><t:Subject>{}</t:Subject><t:IsMeeting>true</t:IsMeeting></t:CalendarEventDetails></t:CalendarEvent>",
                        esc(subject)
                    )
                })
                .collect();
            out.push_str(&format!(
                "<m:FreeBusyResponse><m:ResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode></m:ResponseMessage>\
                 <m:FreeBusyView><t:FreeBusyViewType>Detailed</t:FreeBusyViewType><t:CalendarEventArray>{items}</t:CalendarEventArray></m:FreeBusyView>\
                 </m:FreeBusyResponse>"
            ));
        }
        format!("<m:FreeBusyResponseArray>{out}</m:FreeBusyResponseArray>")
    }
}
