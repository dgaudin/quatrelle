//! The event and contact forms: what they share (their rows, fields and headings), the event
//! form in `event` (with its Room Finder in `rooms` and its Scheduling Assistant in
//! `scheduling`), the contact form in `contact`.

use egui::{Margin, RichText, Sense, Ui, vec2};
use quatrelle_engine::Session;
use quatrelle_engine::model::dates;
use serde_json::{Value, json};

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, widgets};

mod contact;
mod event;
mod meeting;
mod pickers;
mod rooms;
pub(crate) mod scheduling;

pub use contact::{ContactForm, contact};
pub use event::{EventForm, event};
pub use meeting::meeting;
pub use rooms::RoomFinderUi;
pub use scheduling::SchedDrag;

/// A new event's reminder (Outlook's default for appointments).
pub const DEFAULT_REMINDER: i64 = 15;
/// The reminders offered, in minutes before the start (Outlook's list).
pub const REMINDER_CHOICES: [i64; 18] = [0, 5, 10, 15, 30, 60, 120, 180, 240, 480, 720, 1080, 1440, 2880, 4320, 5760, 10080, 20160];

/// A reminder as the list shows it: `None`, `15 minutes`, `2 hours`, `1 day`, `2 weeks`.
pub fn reminder_label(m: Option<i64>) -> String {
    match m {
        None => tl!("None").to_string(),
        Some(m) if m >= 10080 && m % 10080 == 0 => {
            if m == 10080 {
                tl!("1 week").to_string()
            } else {
                tf!("{n} weeks", n = m / 10080)
            }
        }
        Some(m) if m >= 1440 && m % 1440 == 0 => {
            if m == 1440 {
                tl!("1 day").to_string()
            } else {
                tf!("{n} days", n = m / 1440)
            }
        }
        Some(m) if m >= 60 && m % 60 == 0 => {
            if m == 60 {
                tl!("1 hour").to_string()
            } else {
                tf!("{n} hours", n = m / 60)
            }
        }
        Some(m) => {
            if m == 1 {
                tl!("1 minute").to_string()
            } else {
                tf!("{n} minutes", n = m)
            }
        }
    }
}

pub(super) fn local_date(t: i64, s: &Session) -> String {
    let c = dates::civil(t, &s.clock);
    format!("{:04}-{:02}-{:02}", c.year, c.month, c.day)
}

pub(super) fn local_time(t: i64, s: &Session) -> String {
    let c = dates::civil(t, &s.clock);
    format!("{:02}:{:02}", c.hour, c.minute)
}

pub(super) fn writable_calendars(s: &Session) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for a in &s.accounts {
        if let Some(p) = s.pim.get(&a.config.id) {
            for c in p.calendars.iter().filter(|c| !c.read_only) {
                let name = if c.id == quatrelle_engine::model::pim::LOCAL { tl!("Calendar").to_string() } else { c.name.clone() };
                out.push((a.config.id.clone(), c.id.clone(), format!("{name} · {}", a.config.email)));
            }
        }
    }
    out
}

/// The categories to pick from (name, colour): the ones made in Quatrelle, and any an event
/// already has from elsewhere.
pub(super) fn all_categories(s: &Session, has: &[String]) -> Vec<(String, u8)> {
    let mut out: Vec<(String, u8)> = s.prefs.categories.iter().map(|c| (c.name.clone(), c.color)).collect();
    for c in has {
        if !out.iter().any(|(n, _)| n.eq_ignore_ascii_case(c)) {
            out.push((c.clone(), s.category_color_of(c)));
        }
    }
    out
}

pub(super) fn partstat_label(p: &str) -> &'static str {
    match p {
        "ACCEPTED" => "Accepted",
        "DECLINED" => "Declined",
        "TENTATIVE" => "Tentative",
        _ => "No answer yet",
    }
}

/// Width of the forms' label column.
pub(super) const LABEL_W: f32 = 104.0;
/// Height of a one-line field.
pub(super) const FIELD_H: f32 = 30.0;

/// A form's heading: a round badge (an icon, or a person's initials) and the title.
pub(super) fn form_header(ui: &mut Ui, t: &Tokens, badge: Result<Icon, String>, title: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 20.0, t.accent_soft);
        match badge {
            Ok(icon) => icons::paint(ui.painter(), icons::centered(r, 20.0), icon, t.accent),
            Err(initials) => {
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, initials, theme::bold(15.0), t.accent);
            }
        }
        ui.add_space(6.0);
        ui.label(RichText::new(tl!(title)).font(theme::bold(19.0)).color(t.text));
    });
    ui.add_space(12.0);
}

/// A labelled row of a form: an optional icon and the label in a column of their own, the
/// field (or fields) after, lined up with the field's first line.
/// `icons_used`: the form puts icons before labels, so a row without one is indented to match.
pub(super) fn form_row(ui: &mut Ui, t: &Tokens, icons_used: bool, icon: Option<Icon>, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(LABEL_W, FIELD_H), Sense::hover());
        let mut x = r.left();
        if let Some(icon) = icon {
            icons::paint(ui.painter(), egui::Rect::from_center_size(egui::pos2(x + 8.0, r.center().y), vec2(16.0, 16.0)), icon, t.dim);
        }
        if icons_used {
            x += 24.0;
        }
        ui.painter().text(egui::pos2(x, r.center().y), egui::Align2::LEFT_CENTER, tl!(label), theme::regular(13.0), t.dim);
        add(ui);
    });
    ui.add_space(8.0);
}

/// A one-line field of a form, `width` wide (the rest of the row when infinite).
pub(super) fn field(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> egui::Response {
    let w = if width.is_finite() { width } else { ui.available_width() };
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(w)
            .min_size(vec2(w, FIELD_H))
            .margin(Margin::symmetric(8, 6))
            .align(egui::Align2::LEFT_CENTER),
    )
}

/// A field of several lines, the rest of the row wide.
pub(super) fn area(ui: &mut Ui, value: &mut String, hint: &str, rows: usize) -> egui::Response {
    let w = ui.available_width();
    ui.add(egui::TextEdit::multiline(value).hint_text(hint).desired_width(w).desired_rows(rows).margin(Margin::symmetric(8, 6)))
}

/// The tallest the notes are shown before they scroll.
pub(super) const NOTES_MAX_H: f32 = 320.0;

/// An event's notes to read (issue #1): an invitation reads as it does in Outlook, each link
/// under its words and opened from there, long addresses shortened, the whole scrolling when
/// it's tall.
pub(super) fn notes(ui: &mut Ui, t: &Tokens, text: &str, acts: &mut Vec<Action>) {
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6.0).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical().id_salt("event-notes-read").max_height(NOTES_MAX_H).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
            let mut blank = 0;
            for line in quatrelle_engine::model::ical::note_lines(text) {
                // Runs of blank lines as one gap.
                if line.iter().all(|p| matches!(p, quatrelle_engine::model::ical::NotePart::Text(s) if s.trim().is_empty())) {
                    blank += 1;
                    if blank == 1 {
                        ui.add_space(6.0);
                    }
                    continue;
                }
                blank = 0;
                ui.horizontal_wrapped(|ui| {
                    for part in line {
                        match part {
                            quatrelle_engine::model::ical::NotePart::Text(s) => {
                                ui.label(RichText::new(s).color(t.text));
                            }
                            quatrelle_engine::model::ical::NotePart::Link { label, url } => {
                                let shown =
                                    if label == url { widgets::shorten(url.split_once("://").map_or(url.as_str(), |(_, r)| r), 56) } else { label };
                                if ui.link(shown).on_hover_text(&url).clicked() {
                                    acts.push(Action::OpenUrl(url));
                                }
                            }
                        }
                    }
                });
            }
        });
    });
}

/// The line above a form's buttons.
pub(super) fn footer_rule(ui: &mut Ui, t: &Tokens) {
    ui.add_space(6.0);
    let r = ui.available_rect_before_wrap();
    ui.painter().line_segment([r.left_top(), r.right_top()], egui::Stroke::new(1.0, t.border));
    ui.add_space(10.0);
}
