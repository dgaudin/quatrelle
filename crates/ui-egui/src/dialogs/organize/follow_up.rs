// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Follow Up › Custom… and Add Reminder…: a flagged message's start and due days and the time
//! its reminder rings, as Outlook's small Follow Up window has them.

use quatrelle_engine::model::dates;
use serde_json::Value;

use super::*;
use crate::pim_dialogs::pickers;

/// The form as typed: days as `YYYY-MM-DD`, the reminder's time as `YYYY-MM-DDTHH:MM`.
#[derive(Clone, Debug, Default)]
pub struct FollowUpForm {
    pub start: String,
    pub due: String,
    pub remind: bool,
    pub remind_day: String,
    pub remind_time: String,
    /// Custom… (the dates and the reminder) or Add Reminder… (the reminder alone).
    pub dates: bool,
    pub error: Option<String>,
    /// The message the change is for when it isn't the list's selection: the To-Do List's row or
    /// the To-Do bar's entry the menu was opened on, by its key (`mail.flag`'s `key`).
    pub key: Option<String>,
}

impl FollowUpForm {
    /// The form filled in from the message's follow-up, as the server or this computer has it:
    /// the one `key` names (a To-Do List row), else the selected one.
    pub fn of(s: &Session, dates: bool, key: Option<&str>) -> FollowUpForm {
        let keyed = key.and_then(|k| s.message_by_id(k));
        let Some(m) = keyed.as_deref().or_else(|| s.first_selected()) else {
            return FollowUpForm { dates, key: key.map(str::to_string), ..Default::default() };
        };
        let day = |d: Option<i64>| d.map(pickers::text_of).unwrap_or_default();
        let (remind_day, remind_time) = match s.follow_up_reminder(m) {
            Some(at) => {
                let c = dates::civil(at, &s.clock);
                (format!("{:04}-{:02}-{:02}", c.year, c.month, c.day), format!("{:02}:{:02}", c.hour, c.minute))
            }
            None => (String::new(), String::new()),
        };
        FollowUpForm {
            start: day(s.follow_up_start(m)),
            due: day(s.follow_up_due(m)),
            remind: s.follow_up_reminder(m).is_some(),
            remind_day,
            remind_time,
            dates,
            error: None,
            key: key.map(str::to_string),
        }
    }

    /// What `mail.flag` is given: the message's `key` when the form was opened on a To-Do List
    /// row or a To-Do bar entry, the days (`none` for an empty one) and the reminder's time, or
    /// `false` for none.
    pub(crate) fn params(&self) -> Value {
        let day = |text: &str| {
            let text = text.trim();
            if text.is_empty() { json!("none") } else { json!(text) }
        };
        let reminder = if self.remind { json!(format!("{}T{}", self.remind_day.trim(), self.remind_time.trim())) } else { json!(false) };
        let mut p = if self.dates {
            json!({ "start": day(&self.start), "due": day(&self.due), "reminder": reminder })
        } else {
            json!({ "reminder": reminder })
        };
        if let (Some(k), Some(o)) = (&self.key, p.as_object_mut()) {
            o.insert("key".into(), json!(k));
        }
        p
    }

    /// Says what's wrong with the form, if anything: days as `YYYY-MM-DD`, a reminder's time as
    /// `HH:MM`.
    pub(crate) fn check(&self) -> Option<String> {
        if self.dates {
            for (name, text) in [("start", &self.start), ("due", &self.due)] {
                if !text.trim().is_empty() && pickers::day_of(text).is_none() {
                    return Some(tf!("`{name}` is a day, YYYY-MM-DD", name = name));
                }
            }
        }
        if self.remind {
            let time = self.remind_time.trim();
            let ok = time.len() == 5
                && time.get(2..3) == Some(":")
                && time.get(0..2).and_then(|h| h.parse::<u32>().ok()).is_some_and(|h| h <= 23)
                && time.get(3..5).and_then(|m| m.parse::<u32>().ok()).is_some_and(|m| m <= 59);
            if pickers::day_of(&self.remind_day).is_none() || !ok {
                return Some(tl!("The reminder is a day (YYYY-MM-DD) and a time (HH:MM)").to_string());
            }
        }
        None
    }
}

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    let Dialog::FollowUp(f) = d else { return true };
    ui.set_width(380.0);
    title(ui, t, if f.dates { "Custom Follow Up" } else { "Add Reminder" });
    let today = dates::today(&s.clock);
    if f.dates {
        egui::Grid::new("follow-up-dates").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
            for (label, text, id) in [("Start date", &mut f.start, "follow-up-start"), ("Due date", &mut f.due, "follow-up-due")] {
                ui.label(RichText::new(tl!(label)).color(t.dim));
                ui.horizontal(|ui| {
                    pickers::date(ui, t, id, text, today);
                    if !text.is_empty() && widgets::tiny_icon_button(ui, crate::icons::Icon::Close, "None", t).clicked() {
                        text.clear();
                    }
                });
                ui.end_row();
            }
        });
        ui.add_space(4.0);
    }
    ui.horizontal(|ui| {
        ui.checkbox(&mut f.remind, tl!("Reminder"));
        if f.remind {
            pickers::date(ui, t, "follow-up-remind", &mut f.remind_day, today);
            pickers::time(ui, t, &mut f.remind_time);
        }
    });
    if let Some(e) = &f.error {
        ui.label(RichText::new(e.as_str()).color(t.danger).small());
    }
    let (ok, cancel) = buttons(ui, t, "OK", true);
    if ok {
        match f.check() {
            Some(e) => f.error = Some(e),
            None => {
                f.error = None;
                acts.push(Action::Run("mail.flag", f.params()));
            }
        }
    }
    !(cancel || (ok && f.error.is_none()))
}
