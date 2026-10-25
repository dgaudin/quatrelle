//! The picked task, written on in place as Outlook's task form has it: its subject, the day it
//! starts and the day it's due, its status, priority and % complete, its reminder, Private, and
//! its notes. A choice is saved as it's made; text once it's left.

use quatrelle_engine::model::pim::{Task, TaskStatus};
use serde_json::Value;

use super::*;
use crate::pim_dialogs::pickers;

/// The form as typed.
#[derive(Clone, Debug, Default)]
pub(super) struct Form {
    uid: String,
    /// When the task was last changed, as the form was filled in from it.
    modified: i64,
    subject: String,
    notes: String,
    start: String,
    due: String,
    /// Its reminder: on, and the day and time.
    remind: bool,
    remind_day: String,
    remind_time: String,
}

impl Form {
    fn of(t: &Task, s: &Session) -> Form {
        let today = dates::today(&s.clock);
        let (remind_day, remind_time) = match t.reminder {
            Some(at) => {
                let c = dates::civil(at, &s.clock);
                (format!("{:04}-{:02}-{:02}", c.year, c.month, c.day), format!("{:02}:{:02}", c.hour, c.minute))
            }
            // Outlook offers the due day at 8:00.
            None => (pickers::text_of(t.due.unwrap_or(today)), "08:00".to_string()),
        };
        Form {
            uid: t.uid.clone(),
            modified: t.modified,
            subject: t.subject.clone(),
            notes: t.notes.clone(),
            start: t.start.map(pickers::text_of).unwrap_or_default(),
            due: t.due.map(pickers::text_of).unwrap_or_default(),
            remind: t.reminder.is_some(),
            remind_day,
            remind_time,
        }
    }

    /// The reminder as typed, for `task.update`: `None` while it doesn't read.
    fn reminder(&self) -> Option<Value> {
        if !self.remind {
            return Some(Value::Null);
        }
        let day = pickers::day_of(&self.remind_day)?;
        let (h, m) = self.remind_time.trim().split_once(':')?;
        let (h, m) = (h.parse::<u32>().ok().filter(|h| *h < 24)?, m.parse::<u32>().ok().filter(|m| *m < 60)?);
        Some(json!(format!("{}T{h:02}:{m:02}", pickers::text_of(day))))
    }
}

/// What % Complete offers, as Outlook's arrows step it.
const PERCENTS: [u8; 5] = [0, 25, 50, 75, 100];

pub(super) fn show(s: &Session, tu: &mut TasksUi, t: &Tokens, ui: &mut Ui, r: &TaskRef, today: i64, acts: &mut Vec<Action>) {
    let task = &r.task;
    // Filled in again when another task is picked, or this one changed (here, or in Outlook)
    // while nothing is being typed.
    let typing = ui.ctx().memory(|m| m.focused().is_some());
    if tu.form.as_ref().is_none_or(|f| f.uid != task.uid || (f.modified != task.modified && !typing)) {
        tu.form = Some(Form::of(task, s));
    }
    let Some(f) = tu.form.as_mut() else { return };
    let uid = task.uid.clone();
    let update = |acts: &mut Vec<Action>, mut v: Value| {
        v["uid"] = json!(uid);
        acts.push(Action::Run("task.update", v));
    };
    egui::ScrollArea::vertical().auto_shrink(false).id_salt(("task-form", &task.uid)).show(ui, |ui| {
        let field = ui.add(egui::TextEdit::singleline(&mut f.subject).font(theme::bold(17.0)).hint_text(tl!("Subject")).desired_width(f32::INFINITY));
        if field.lost_focus() {
            let subject = f.subject.trim();
            if subject.is_empty() {
                f.subject = task.subject.clone();
            } else if subject != task.subject {
                update(acts, json!({"subject": subject}));
            }
        }
        // Where it stands, in a line: late, due when, or done when.
        let said = match (task.completed, task.due) {
            (Some(d), _) if task.is_complete() => tf!("Completed on {date}", date = crate::calendar::long_date(d)),
            (_, Some(d)) if task.is_overdue(today) => tf!("Overdue: due {date}", date = crate::calendar::long_date(d)),
            (_, Some(d)) if d == today => tl!("Due today").to_string(),
            (_, Some(d)) => tf!("Due {date}", date = crate::calendar::long_date(d)),
            _ => tl!("No due date").to_string(),
        };
        ui.add_space(2.0);
        ui.label(RichText::new(said).color(if task.is_overdue(today) { t.danger } else { t.dim }).small());
        ui.add_space(12.0);
        egui::Grid::new(("task-fields", &task.uid)).num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
            for (label, text, key, id) in [("Start date", &mut f.start, "start", "task-start"), ("Due date", &mut f.due, "due", "task-due")] {
                ui.label(RichText::new(tl!(label)).color(t.dim));
                ui.horizontal(|ui| {
                    if pickers::date(ui, t, id, text, today)
                        && let Some(d) = pickers::day_of(text)
                    {
                        update(acts, json!({ key: pickers::text_of(d) }));
                    }
                    if !text.is_empty() && widgets::tiny_icon_button(ui, Icon::Close, "None", t).clicked() {
                        text.clear();
                        update(acts, json!({ key: null }));
                    }
                });
                ui.end_row();
            }
            ui.label(RichText::new(tl!("Status")).color(t.dim));
            egui::ComboBox::from_id_salt(("task-status", &task.uid)).width(190.0).selected_text(tl!(task.status.label())).show_ui(ui, |ui| {
                for st in TaskStatus::ALL {
                    if ui.selectable_label(task.status == st, tl!(st.label())).clicked() && task.status != st {
                        update(acts, json!({"status": st.name()}));
                    }
                }
            });
            ui.end_row();
            ui.label(RichText::new(tl!("Priority")).color(t.dim));
            let name = |i: i8| match i {
                1.. => tl!("High"),
                ..0 => tl!("Low"),
                0 => tl!("Normal"),
            };
            egui::ComboBox::from_id_salt(("task-priority", &task.uid)).width(190.0).selected_text(name(task.importance)).show_ui(ui, |ui| {
                for i in [-1, 0, 1] {
                    if ui.selectable_label(task.importance == i, name(i)).clicked() && task.importance != i {
                        update(acts, json!({"importance": i}));
                    }
                }
            });
            ui.end_row();
            ui.label(RichText::new(tl!("% Complete")).color(t.dim));
            egui::ComboBox::from_id_salt(("task-percent", &task.uid)).width(190.0).selected_text(format!("{} %", task.percent)).show_ui(ui, |ui| {
                for p in PERCENTS {
                    if ui.selectable_label(task.percent == p, format!("{p} %")).clicked() && task.percent != p {
                        update(acts, json!({"percent": p}));
                    }
                }
            });
            ui.end_row();
            ui.label(RichText::new(tl!("Reminder")).color(t.dim));
            ui.horizontal(|ui| {
                let mut changed = ui.checkbox(&mut f.remind, "").changed();
                if f.remind {
                    changed |= pickers::date(ui, t, "task-remind", &mut f.remind_day, today);
                    changed |= pickers::time(ui, t, &mut f.remind_time);
                }
                if changed && let Some(v) = f.reminder() {
                    update(acts, json!({"reminder": v}));
                }
            });
            ui.end_row();
            ui.label(RichText::new(tl!("Private")).color(t.dim));
            let mut private = task.private;
            if ui.checkbox(&mut private, "").changed() {
                update(acts, json!({"private": private}));
            }
            ui.end_row();
            if !task.categories.is_empty() {
                ui.label(RichText::new(tl!("Categories")).color(t.dim));
                ui.horizontal_wrapped(|ui| {
                    for c in &task.categories {
                        let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 5.0, theme::category_color(s.category_color_of(c), t.dark));
                        ui.label(c);
                    }
                });
                ui.end_row();
            }
        });
        ui.add_space(12.0);
        let notes = ui.add(egui::TextEdit::multiline(&mut f.notes).hint_text(tl!("Notes")).desired_rows(10).desired_width(f32::INFINITY));
        if notes.lost_focus() && f.notes != task.notes {
            update(acts, json!({"notes": f.notes}));
        }
        ui.add_space(8.0);
        ui.label(RichText::new(tf!("In {account}", account = r.account)).color(t.faint).small());
    });
}
