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
        if self.occurrence_deleted(master, index) {
            return None;
        }
        let (folder, el) = self.pim.iter().find(|(i, _, _)| *i == master).map(|(_, f, e)| (f.clone(), e.clone()))?;
        let (start, end) = occurrence_times(&el, index)?;
        let copied: String = el
            .kids
            .iter()
            .filter(|k| matches!(k.name.as_str(), "Subject" | "Body" | "Location" | "UID" | "IsAllDayEvent" | "Organizer" | "RequiredAttendees"))
            .map(xml)
            .collect();
        let occurrence = parse(&format!(
            "<t:CalendarItem>{copied}<t:Start>{0}</t:Start><t:End>{1}</t:End><t:OriginalStart>{0}</t:OriginalStart>\
             <t:CalendarItemType>Occurrence</t:CalendarItemType></t:CalendarItem>",
            when(start),
            when(end)
        ));
        self.pim.push((id.clone(), folder, occurrence.kids.first()?.clone()));
        Some(id)
    }

    /// Whether occurrence `index` of series `master` was deleted (its series lists it among its
    /// `DeletedOccurrences`).
    pub(super) fn occurrence_deleted(&self, master: &str, index: usize) -> bool {
        let Some((_, _, el)) = self.pim.iter().find(|(i, _, _)| *i == master) else { return false };
        let Some((start, _)) = occurrence_times(el, index) else { return false };
        el.child("DeletedOccurrences")
            .is_some_and(|d| d.children("DeletedOccurrence").any(|o| o.text_of("Start").and_then(quatrelle_jmap::utc_date) == Some(start)))
    }

    /// An occurrence item deleted (`id`, `master:occN`): its series lists it among its
    /// `DeletedOccurrences` from now on, as Exchange's does.
    pub(super) fn delete_occurrence(&mut self, id: &str) {
        let Some((master, index)) = id.split_once(":occ").and_then(|(m, n)| Some((m.to_string(), n.parse::<usize>().ok()?))) else { return };
        let Some((_, _, el)) = self.pim.iter_mut().find(|(i, _, _)| *i == master) else { return };
        let Some((start, _)) = occurrence_times(el, index) else { return };
        let gone = parse(&format!("<t:DeletedOccurrence><t:Start>{}</t:Start></t:DeletedOccurrence>", when(start))).kids;
        match el.kids.iter_mut().find(|k| k.name == "DeletedOccurrences") {
            Some(list) => list.kids.extend(gone),
            None => el.kids.push(El { name: "DeletedOccurrences".into(), kids: gone, ..Default::default() }),
        }
    }
}

/// When occurrence `index` (from 1) of series `el` starts and ends, from its pattern (daily or
/// weekly, with its interval).
fn occurrence_times(el: &El, index: usize) -> Option<(i64, i64)> {
    let rec = el.child("Recurrence")?;
    let interval = rec.find("Interval").and_then(|i| i.text.trim().parse::<i64>().ok()).unwrap_or(1).max(1);
    let step = if rec.child("DailyRecurrence").is_some() { interval } else { interval * 7 } * 86_400;
    let start = quatrelle_jmap::utc_date(el.text_of("Start")?)?;
    let end = quatrelle_jmap::utc_date(el.text_of("End")?).unwrap_or(start + 3600);
    let shift = i64::try_from(index.checked_sub(1)?).ok()? * step;
    Some((start + shift, end + shift))
}

/// A time as EWS writes it.
fn when(t: i64) -> String {
    let (y, m, d) = quatrelle_model::dates::civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

impl Data {
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
