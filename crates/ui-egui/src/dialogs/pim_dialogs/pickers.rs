//! Picking a date and a time, as Outlook's appointment window has them: the field can be typed
//! in, and beside it a month to click a day in, or the hours by the half hour.

use quatrelle_engine::model::dates::{civil_from_days, days_from_civil};

use super::*;

/// A day typed as `YYYY-MM-DD`, and the days since 1970 it is.
fn day_of(text: &str) -> Option<i64> {
    let mut it = text.trim().split('-');
    let (y, m, d) = (it.next()?.parse::<i64>().ok()?, it.next()?.parse::<u32>().ok()?, it.next()?.parse::<u32>().ok()?);
    ((1..=12).contains(&m) && (1..=31).contains(&d) && (1900..=2200).contains(&y)).then(|| days_from_civil(y, m, d))
}

fn text_of(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// A date: typed, or picked in a month (the one it's in, or `today`'s). `true` when it changed.
pub(crate) fn date(ui: &mut Ui, t: &Tokens, id: &str, text: &mut String, today: i64) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        changed |= field(ui, text, "YYYY-MM-DD", 104.0).changed();
        let r = widgets::tiny_icon_button(ui, Icon::Calendar, "Pick a day", t);
        let month_id = ui.make_persistent_id(("picker-month", id));
        if r.clicked() {
            let day = day_of(text).unwrap_or(today);
            let (y, m, _) = civil_from_days(day);
            ui.data_mut(|d| d.insert_temp(month_id, y * 12 + i64::from(m) - 1));
        }
        egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            if let Some(day) = month(ui, t, month_id, day_of(text), today) {
                *text = text_of(day);
                changed = true;
                ui.close();
            }
        });
    });
    changed
}

/// A month's days to click one in, the month changed with its arrows; the day clicked.
fn month(ui: &mut Ui, t: &Tokens, month_id: egui::Id, chosen: Option<i64>, today: i64) -> Option<i64> {
    let mut shown: i64 = ui.data(|d| d.get_temp(month_id)).unwrap_or_else(|| {
        let (y, m, _) = civil_from_days(chosen.unwrap_or(today));
        y * 12 + i64::from(m) - 1
    });
    let (y, m) = (shown.div_euclid(12), u32::try_from(shown.rem_euclid(12) + 1).unwrap_or(1));
    let mut picked = None;
    ui.horizontal(|ui| {
        if widgets::tiny_icon_button(ui, Icon::ChevronLeft, "Previous month", t).clicked() {
            shown -= 1;
        }
        let name = crate::calendar::MONTHS.get(m as usize - 1).copied().unwrap_or("");
        ui.label(RichText::new(tf!("{month} {year}", month = crate::i18n::tc("month", name), year = y)).font(theme::bold(13.0)).color(t.text));
        if widgets::tiny_icon_button(ui, Icon::ChevronRight, "Next month", t).clicked() {
            shown += 1;
        }
    });
    ui.data_mut(|d| d.insert_temp(month_id, shown));
    let w = 28.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for d in crate::calendar::DAYS {
            let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
            let letter = tl!(d).chars().next().map(String::from).unwrap_or_default();
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, letter, theme::regular(11.0), t.faint);
        }
    });
    let first = days_from_civil(y, m, 1);
    let start = first - (first + 3).rem_euclid(7);
    for week in 0..6 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for i in 0..7 {
                let day = start + week * 7 + i;
                let (r, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
                let (_, dm, dd) = civil_from_days(day);
                if Some(day) == chosen {
                    ui.painter().circle_filled(r.center(), 11.0, t.accent);
                } else if resp.hovered() {
                    ui.painter().circle_filled(r.center(), 11.0, t.hover);
                }
                let color = if Some(day) == chosen {
                    t.on_accent
                } else if dm != m {
                    t.faint
                } else if day == today {
                    t.accent
                } else {
                    t.text
                };
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, dd.to_string(), theme::regular(12.0), color);
                if resp.clicked() {
                    picked = Some(day);
                }
            }
        });
    }
    picked
}

/// A time of day: typed, or picked among the hours by the half hour. `true` when it changed.
pub(crate) fn time(ui: &mut Ui, t: &Tokens, text: &mut String) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        changed |= field(ui, text, "09:00", 58.0).changed();
        let r = widgets::tiny_icon_button(ui, Icon::ChevronDown, "Pick a time", t);
        egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                for half in 0..48 {
                    let at = format!("{:02}:{:02}", half / 2, (half % 2) * 30);
                    if ui.selectable_label(text.trim() == at, &at).clicked() {
                        *text = at;
                        changed = true;
                        ui.close();
                    }
                }
            });
        });
    });
    changed
}
