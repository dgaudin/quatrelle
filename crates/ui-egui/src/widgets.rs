//! Small shared widgets: toolbar buttons, avatars, the welcome screen.

use egui::{Color32, CornerRadius, Response, RichText, Sense, Stroke, Ui, vec2};

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog};

/// A ribbon button: icon over nothing, label to the right.
pub fn tool_button(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, enabled: bool) -> Response {
    let label = tl!(label);
    let font = theme::regular(13.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, t.text);
    let size = vec2(galley.size().x + 36.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let color = if enabled { t.text } else { t.faint };
    if enabled && (resp.hovered() || resp.is_pointer_button_down_on()) {
        ui.painter().rect_filled(rect, 6.0, if resp.is_pointer_button_down_on() { t.accent_soft } else { t.hover });
    }
    let ir = egui::Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(16.0, 16.0));
    icons::paint(ui.painter(), ir, icon, if enabled { t.accent } else { t.faint });
    ui.painter().galley_with_override_text_color(rect.min + vec2(30.0, (30.0 - galley.size().y) / 2.0), galley, color);
    resp
}

/// A ribbon button that stays lit while it's on (a search term in the box, say).
pub fn tool_toggle(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, on: bool) -> Response {
    let label = tl!(label);
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::regular(13.0), t.text);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 36.0, 30.0), Sense::click());
    if on {
        ui.painter().rect(rect, 6.0, t.accent_soft, Stroke::new(1.0, t.accent.gamma_multiply(0.5)), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, t.hover);
    }
    icons::paint(ui.painter(), egui::Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(16.0, 16.0)), icon, t.accent);
    ui.painter().galley_with_override_text_color(rect.min + vec2(30.0, (30.0 - galley.size().y) / 2.0), galley, t.text);
    resp
}

/// A "⋯" button in the style of [`pill_button`], for a menu of more actions.
pub fn more_pill(ui: &mut Ui, tip: &str, t: &Tokens) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 28.0), Sense::click());
    let fill = if resp.hovered() { t.hover } else { t.surface };
    ui.painter().rect(rect, 6.0, fill, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    for dx in [-6.0, 0.0, 6.0] {
        ui.painter().circle_filled(rect.center() + vec2(dx, 0.0), 1.6, t.dim);
    }
    resp.on_hover_text(tl!(tip))
}

/// The filled "New mail" button.
pub fn accent_button(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, enabled: bool) -> Response {
    let label = tl!(label);
    let font = theme::bold(13.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, t.on_accent);
    let size = vec2(galley.size().x + 40.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let fill = if !enabled {
        t.faint
    } else if resp.hovered() {
        t.accent.gamma_multiply(0.88)
    } else {
        t.accent
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    let ir = egui::Rect::from_min_size(rect.min + vec2(10.0, 7.0), vec2(16.0, 16.0));
    icons::paint(ui.painter(), ir, icon, t.on_accent);
    ui.painter().galley_with_override_text_color(rect.min + vec2(32.0, (30.0 - galley.size().y) / 2.0), galley, t.on_accent);
    resp
}

/// A dialog's main button (OK, Connect, Remove…): filled in the app colour, words only.
pub fn primary_button(ui: &mut Ui, label: &str, t: &Tokens, enabled: bool) -> Response {
    let label = tl!(label);
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::bold(13.0), t.on_accent);
    let size = vec2((galley.size().x + 28.0).max(76.0), 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let fill = if !enabled {
        t.faint
    } else if resp.hovered() {
        t.accent.gamma_multiply(0.88)
    } else {
        t.accent
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, t.on_accent);
    resp
}

/// An icon button on the coloured title bar.
pub fn bar_icon_button(ui: &mut Ui, icon: Icon, tip: &str, t: &Tokens) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    icons::paint(ui.painter(), icons::centered(rect, 16.0), icon, if resp.hovered() { t.text } else { t.dim });
    resp.on_hover_text(tl!(tip))
}

/// A small icon button inside panes.
pub fn tiny_icon_button(ui: &mut Ui, icon: Icon, tip: &str, t: &Tokens) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    icons::paint(ui.painter(), icons::centered(rect, 13.0), icon, if resp.hovered() { t.text } else { t.dim });
    resp.on_hover_text(tl!(tip))
}

/// Same-height chips in columns that share the width and wrap onto more rows, as Outlook lays out
/// attachments; past `max_rows` rows they scroll. `chip(ui, i, width)` draws chip `i`.
pub fn chip_grid(ui: &mut Ui, id: egui::Id, n: usize, min_w: f32, max_w: f32, h: f32, max_rows: usize, mut chip: impl FnMut(&mut Ui, usize, f32)) {
    if n == 0 {
        return;
    }
    let gap = 6.0;
    let fits = |avail: f32| {
        let mut cols = 1usize;
        while (cols + 1) as f32 * min_w + cols as f32 * gap <= avail {
            cols += 1;
        }
        cols
    };
    let mut avail = ui.available_width().max(min_w);
    let max_rows = max_rows.max(1);
    // Leave room for the scroll bar when the chips will scroll.
    if n.div_ceil(fits(avail)) > max_rows {
        avail = (avail - egui::style::ScrollStyle::solid().bar_width - 6.0).max(min_w);
    }
    let cols = fits(avail);
    let w = ((avail - (cols - 1) as f32 * gap) / cols as f32).clamp(min_w.min(avail), max_w);
    let rows = n.div_ceil(cols);
    ui.scope(|ui| {
        // A solid scroll bar, always shown, so it's plain that there are more.
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        egui::ScrollArea::vertical().id_salt(id).max_height(max_rows as f32 * (h + gap) - gap).auto_shrink([false, true]).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(gap, gap);
            for r in 0..rows {
                ui.horizontal(|ui| {
                    for i in r * cols..((r + 1) * cols).min(n) {
                        chip(ui, i, w);
                    }
                });
            }
        });
    });
}

/// Text cut to one line of `width`, ending in "…" when it doesn't fit.
pub fn one_line(ui: &Ui, text: &str, font: egui::FontId, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping { max_width: width.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.painter().layout_job(job)
}

/// A labelled icon button for the reading pane's action row.
pub fn pill_button(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens) -> Response {
    let label = tl!(label);
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::regular(13.0), t.text);
    let size = vec2(galley.size().x + 34.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() { t.hover } else { t.surface };
    ui.painter().rect(rect, 6.0, fill, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    icons::paint(ui.painter(), egui::Rect::from_min_size(rect.min + vec2(9.0, 6.0), vec2(16.0, 16.0)), icon, t.accent);
    ui.painter().galley_with_override_text_color(rect.min + vec2(29.0, (28.0 - galley.size().y) / 2.0), galley, t.text);
    resp
}

/// How tall the ribbon's buttons are (a group's name goes under them).
pub const RIBBON_BUTTON_H: f32 = 68.0;
/// The space for a group's name under its buttons.
const GROUP_NAME_H: f32 = 16.0;

/// A ribbon button as Outlook's: a large icon with its name under it (on two lines when long),
/// greyed out when it can't be used.
pub fn big_button(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, enabled: bool) -> Response {
    big(ui, icon, label, t, if enabled { Big::Plain } else { Big::Off })
}

/// A ribbon button that stays lit while it's on.
pub fn big_toggle(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, on: bool) -> Response {
    big(ui, icon, label, t, if on { Big::On } else { Big::Plain })
}

/// The ribbon's first button (New mail, New event…): its icon in the app colour.
pub fn big_accent(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, enabled: bool) -> Response {
    big(ui, icon, label, t, if enabled { Big::Accent } else { Big::Off })
}

#[derive(Clone, Copy, PartialEq)]
enum Big {
    Plain,
    On,
    Accent,
    Off,
}

fn big(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, kind: Big) -> Response {
    let label = tl!(label);
    let enabled = kind != Big::Off;
    let text = if enabled { t.text } else { t.faint };
    // Long names break between words onto a second line, as Outlook's do.
    let mut galley = ui.painter().layout(label.to_string(), theme::regular(12.5), text, 76.0);
    // Two lines at most: a longer name makes the button wider instead.
    if galley.rows.len() > 2 {
        galley = ui.painter().layout(label.to_string(), theme::regular(12.5), text, 128.0);
    }
    let w = (galley.size().x + 14.0).max(46.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, RIBBON_BUTTON_H), if enabled { Sense::click() } else { Sense::hover() });
    let pressed = enabled && resp.is_pointer_button_down_on();
    if kind == Big::On {
        ui.painter().rect(rect, 4.0, t.accent_soft, Stroke::new(1.0, t.accent.gamma_multiply(0.5)), egui::StrokeKind::Inside);
    } else if enabled && (resp.hovered() || pressed) {
        ui.painter().rect_filled(rect, 4.0, if pressed { t.accent_soft } else { t.hover });
    }
    let ir = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.min.y + 18.0), vec2(26.0, 26.0));
    let icon_color = match kind {
        Big::Off => t.faint,
        Big::Accent => t.accent,
        _ => t.text,
    };
    icons::paint(ui.painter(), ir, icon, icon_color);
    let at = egui::pos2(rect.center().x - galley.size().x / 2.0, rect.min.y + 35.0);
    ui.painter().galley_with_override_text_color(at, galley, text);
    resp
}

/// The ▾ beside a ribbon button, for its menu of more choices: as tall as the button, the
/// arrow level with its name.
pub fn big_more(ui: &mut Ui, tip: &str, t: &Tokens) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(16.0, RIBBON_BUTTON_H), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    let ir = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.min.y + 42.0), vec2(12.0, 12.0));
    icons::paint(ui.painter(), ir, Icon::ChevronDown, if resp.hovered() { t.text } else { t.dim });
    resp.on_hover_text(tl!(tip))
}

/// The ribbon's groups, laid out left to right: each ends with its name centred under its buttons
/// and a line after it, as Outlook's ribbon names its groups (New, Delete, Respond…).
pub struct Groups {
    start: f32,
    top: f32,
}

impl Groups {
    /// Start the first group where the ribbon's row starts.
    pub fn new(ui: &Ui) -> Groups {
        Groups { start: ui.cursor().min.x, top: ui.cursor().min.y }
    }

    /// End the group begun last, named `name`, and start the next one after a line.
    pub fn end(&mut self, ui: &mut Ui, t: &Tokens, name: &str) {
        self.name(ui, t, name);
        ui.add_space(3.0);
        let (r, _) = ui.allocate_exact_size(vec2(1.0, RIBBON_BUTTON_H + GROUP_NAME_H - 6.0), Sense::hover());
        ui.painter().rect_filled(r, 0.0, t.border);
        ui.add_space(3.0);
        self.start = ui.cursor().min.x;
    }

    /// End the last group, named `name`.
    pub fn last(&mut self, ui: &mut Ui, t: &Tokens, name: &str) {
        self.name(ui, t, name);
    }

    fn name(&mut self, ui: &mut Ui, t: &Tokens, name: &str) {
        let galley = ui.painter().layout_no_wrap(tl!(name).to_string(), theme::regular(11.5), t.dim);
        // A name wider than its buttons widens the group.
        let span = ui.cursor().min.x - self.start;
        if span < galley.size().x + 8.0 {
            ui.add_space(galley.size().x + 8.0 - span);
        }
        let right = ui.cursor().min.x;
        let at = egui::pos2((self.start + right - galley.size().x) / 2.0, self.top + RIBBON_BUTTON_H + 2.0);
        ui.painter().galley_with_override_text_color(at, galley, t.dim);
        // Keep the row tall enough for the names.
        ui.allocate_exact_size(vec2(0.0, RIBBON_BUTTON_H + GROUP_NAME_H), Sense::hover());
    }
}

/// Small ribbon items stacked in a column (up to three), as Outlook stacks its small buttons.
pub fn stack(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(vec2(0.0, RIBBON_BUTTON_H), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.spacing_mut().interact_size.y = 19.0;
        // The column is as wide as its widest line: a label never wraps in it (from a width of
        // nothing, it would go one letter a line).
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        add(ui);
    });
}

/// An on/off switch (a setting that applies at once). `label` (translated) is for screen readers
/// only; the row shows its own. Clicking it flips `on`; the response says it changed.
pub fn switch(ui: &mut Ui, t: &Tokens, on: &mut bool, label: &str) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(36.0, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let label = label.to_string();
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *on, &label));
    let k = ui.ctx().animate_bool_responsive(resp.id, *on);
    let enabled = ui.is_enabled();
    let track = if *on {
        t.accent
    } else if resp.hovered() {
        t.hover
    } else {
        t.surface
    };
    let track = if enabled { track } else { track.gamma_multiply(0.5) };
    let edge = if *on { t.accent } else { t.dim };
    ui.painter().rect(rect, 10.0, track, Stroke::new(1.0, if enabled { edge } else { t.border }), egui::StrokeKind::Inside);
    let x = egui::lerp(rect.left() + 10.0..=rect.right() - 10.0, k);
    let knob = if *on { t.on_accent } else { t.dim };
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), if resp.hovered() && enabled { 6.5 } else { 6.0 }, knob);
    if resp.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), 12.0, Stroke::new(1.5, t.accent), egui::StrokeKind::Outside);
    }
    resp
}

/// One choice among a few, as joined buttons (Theme: System, Light, Dark), labels translated.
/// The picked one is raised; returns the index clicked, if any.
pub fn segmented(ui: &mut Ui, t: &Tokens, id: &str, options: &[&str], selected: usize) -> Option<usize> {
    let font = theme::regular(13.0);
    let labels: Vec<String> = options.iter().map(|o| o.to_string()).collect();
    let widths: Vec<f32> = labels.iter().map(|l| ui.painter().layout_no_wrap(l.clone(), font.clone(), t.text).size().x + 24.0).collect();
    let total: f32 = widths.iter().sum::<f32>() + 4.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 28.0), Sense::hover());
    ui.painter().rect(rect, 6.0, t.bg, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let mut x = rect.left() + 2.0;
    let mut clicked = None;
    for (i, (label, w)) in labels.iter().zip(&widths).enumerate() {
        let r = egui::Rect::from_min_size(egui::pos2(x, rect.top() + 2.0), vec2(*w, 24.0));
        let resp = ui.interact(r, ui.scope_id().with(("segment", id, i)), Sense::click());
        let on = i == selected;
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, ui.is_enabled(), on, label));
        if on {
            ui.painter().rect(r, 4.0, t.surface, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 4.0, t.hover);
        }
        let color = if on { t.text } else { t.dim };
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, label, if on { theme::bold(13.0) } else { font.clone() }, color);
        if resp.clicked() && !on {
            clicked = Some(i);
        }
        x += w;
    }
    clicked
}

pub fn group_sep(ui: &mut Ui, t: &Tokens) {
    ui.add_space(4.0);
    let (r, _) = ui.allocate_exact_size(vec2(1.0, 26.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.border);
    ui.add_space(4.0);
}

/// A round avatar with initials.
pub fn avatar(ui: &Ui, center: egui::Pos2, radius: f32, initials: &str, color: Color32) {
    ui.painter().circle_filled(center, radius, color);
    ui.painter().text(center, egui::Align2::CENTER_CENTER, initials, theme::bold(radius * 0.8), Color32::WHITE);
}

pub fn human_size(bytes: usize) -> String {
    let b = bytes as f64;
    if b < 1024.0 {
        format!("{bytes} B")
    } else if b < 1024.0 * 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else {
        format!("{:.1} MB", b / 1024.0 / 1024.0)
    }
}

pub fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>()) }
}

/// First run: no accounts yet.
pub fn welcome(ui: &mut Ui, t: &Tokens, jmap_only: bool, acts: &mut Vec<Action>) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.18).max(20.0));
        egui::Frame::new().fill(t.surface).stroke(Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(12)).inner_margin(egui::Margin::same(32)).show(ui, |ui| {
            ui.set_max_width(460.0);
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                ui.painter().rect_filled(r, 14.0, t.accent);
                icons::paint(ui.painter(), icons::centered(r, 38.0), Icon::Unread, t.on_accent);
                ui.add_space(12.0);
                ui.label(RichText::new(tl!("Welcome to Quatrelle")).font(theme::bold(22.0)).color(t.text));
                ui.add_space(4.0);
                let about = if jmap_only {
                    "Your mail in the browser. Add a JMAP account (such as Fastmail): Quatrelle talks to your mail server directly, and your password stays in this tab only."
                } else {
                    "Your mail, on your computer. Add any IMAP account: messages are kept on this device so they're fast to search and readable offline. Your password stays in the system keychain."
                };
                ui.label(RichText::new(tl!(about)).color(t.dim));
                ui.add_space(18.0);
                if accent_button(ui, Icon::Plus, "Add an account", t, true).clicked() {
                    acts.push(Action::Dialog(Dialog::add_account()));
                }
            });
        });
    });
}
