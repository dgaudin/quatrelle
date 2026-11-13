// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The fake EWS server's occurrences of a series: one made an item of its own when it's asked
//! for, and those listed by their series.

use super::*;

impl Data {
    /// The item of one occurrence of a series (`OccurrenceItemId`: the series' item and the
    /// occurrence's place in it, from 1), made the first time it's asked for, as Exchange makes
    /// an occurrence its own item once it's opened: its day worked out from the series' pattern
    /// (daily or weekly, with its interval), its other fields the series'.
    pub(super) fn occurrence_item(&mut self, master: &str, index: usize) -> Option<String> {
        let id = format!("{master}:occ{index}");
        if self.pim.iter().any(|(i, _, _)| *i == id) {
            return Some(id);
        }
        let (folder, el) = self.pim.iter().find(|(i, _, _)| *i == master).map(|(_, f, e)| (f.clone(), e.clone()))?;
        let rec = el.child("Recurrence")?;
        let interval = rec.find("Interval").and_then(|i| i.text.trim().parse::<i64>().ok()).unwrap_or(1).max(1);
        let step = if rec.child("DailyRecurrence").is_some() { interval } else { interval * 7 } * 86_400;
        let start = quatrelle_jmap::utc_date(el.text_of("Start")?)?;
        let end = quatrelle_jmap::utc_date(el.text_of("End")?).unwrap_or(start + 3600);
        let shift = i64::try_from(index.checked_sub(1)?).ok()? * step;
        let when = |t: i64| {
            let (y, m, d) = quatrelle_model::dates::civil_from_days(t.div_euclid(86_400));
            let s = t.rem_euclid(86_400);
            format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
        };
        let copied: String = el
            .kids
            .iter()
            .filter(|k| matches!(k.name.as_str(), "Subject" | "Body" | "Location" | "UID" | "IsAllDayEvent" | "Organizer" | "RequiredAttendees"))
            .map(xml)
            .collect();
        let occurrence = parse(&format!(
            "<t:CalendarItem>{copied}<t:Start>{0}</t:Start><t:End>{1}</t:End><t:OriginalStart>{0}</t:OriginalStart>\
             <t:CalendarItemType>Occurrence</t:CalendarItemType></t:CalendarItem>",
            when(start + shift),
            when(end + shift)
        ));
        self.pim.push((id.clone(), folder, occurrence.kids.first()?.clone()));
        Some(id)
    }

    /// A series' occurrences that are items of their own (opened, moved), as the series lists
    /// them (`ModifiedOccurrences`).
    pub(super) fn modified_occurrences(&self, master: &str) -> String {
        let prefix = format!("{master}:occ");
        let list: String = self
            .pim
            .iter()
            .filter(|(i, _, _)| i.starts_with(&prefix))
            .map(|(i, _, e)| {
                format!(
                    "<t:Occurrence><t:ItemId Id=\"{i}\" ChangeKey=\"ck2\"/><t:Start>{}</t:Start><t:End>{}</t:End><t:OriginalStart>{}</t:OriginalStart></t:Occurrence>",
                    e.text_of("Start").unwrap_or_default(),
                    e.text_of("End").unwrap_or_default(),
                    e.text_of("OriginalStart").unwrap_or_default()
                )
            })
            .collect();
        if list.is_empty() { String::new() } else { format!("<t:ModifiedOccurrences>{list}</t:ModifiedOccurrences>") }
    }
}
