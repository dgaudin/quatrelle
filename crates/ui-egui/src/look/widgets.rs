//! Small shared widgets: toolbar buttons, avatars, the welcome screen.

use egui::{Color32, CornerRadius, Response, RichText, Sense, Stroke, Ui, vec2};

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog};

/// A ribbon button: icon over nothing, label to the right.
/// The ribbon's buttons as it's drawn: those from `icons_from` on are icons alone, their labels
/// tooltips (it didn't fit the window); how wide each label is, and how far right they reached.
#[derive(Clone, Debug, Default)]
pub struct RibbonFit {
    pub active: bool,
    pub icons_from: usize,
    pub count: usize,
    pub labels: Vec<f32>,
    pub right: f32,
}

impl RibbonFit {
    /// The first button to draw without its label so that the ribbon fits `room_right`: labels
    /// go from the end, as few as it takes.
    pub fn fit(&self, room_right: f32) -> usize {
        let hidden: f32 = self.labels.iter().skip(self.icons_from).sum();
        let need = self.right + hidden - room_right;
        let mut from = self.labels.len();
        let mut saved = 0.0;
        while need > 0.5 && saved < need && from > 0 {
            from -= 1;
            saved += self.labels.get(from).copied().unwrap_or(0.0);
        }
        from
    }
}

pub fn ribbon_fit_id() -> egui::Id {
    egui::Id::unique("ribbon-fit")
}

/// While the ribbon is drawn: note a button (its label's width) and whether it goes without it.
fn ribbon_button(ui: &Ui, label_width: f32) -> bool {
    ui.ctx().data_mut(|d| {
        let f = d.get_temp_mut_or_default::<RibbonFit>(ribbon_fit_id());
        if !f.active {
            return false;
        }
        let i = f.count;
        f.count += 1;
        f.labels.push(label_width + 4.0);
        i >= f.icons_from
    })
}

fn reached(ui: &Ui, rect: egui::Rect) {
    ui.ctx().data_mut(|d| {
        let f = d.get_temp_mut_or_default::<RibbonFit>(ribbon_fit_id());
        if f.active {
            f.right = f.right.max(rect.right());
        }
    });
}

/// An icon alone, its label as the tooltip (a ribbon too wide for the window).
/// A button's background as the pointer comes and goes: the hover fades in and out (quickly),
/// a press shows at once.
fn hover_fill(ui: &Ui, resp: &Response, rect: egui::Rect, t: &Tokens, enabled: bool) {
    let k = crate::motion::toward(ui.ctx(), resp.id.with("hover"), enabled && resp.hovered(), crate::motion::FAST);
    if enabled && resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, 6.0, t.accent_soft);
    } else if k > 0.0 {
        ui.painter().rect_filled(rect, 6.0, t.hover.gamma_multiply(k));
    }
}

/// What a drawn button is to a screen reader: a button (or a toggle, `on` given) named `label`
/// (translated), as egui's own buttons say; without it, AccessKit reads an unnamed button.
pub(crate) fn named(ui: &Ui, resp: &Response, label: &str, enabled: bool, on: Option<bool>) {
    let enabled = enabled && ui.is_enabled();
    resp.widget_info(|| match on {
        Some(on) => egui::WidgetInfo::selected(egui::Role::Button, enabled, on, label),
        None => egui::WidgetInfo::labeled(egui::Role::Button, enabled, label),
    });
}

fn icon_button(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, enabled: bool, on: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 30.0), if enabled { Sense::click() } else { Sense::hover() });
    reached(ui, rect);
    if on {
        ui.painter().rect(rect, 6.0, t.accent_soft, Stroke::new(1.0, t.accent.gamma_multiply(0.5)), egui::StrokeKind::Inside);
    } else {
        hover_fill(ui, &resp, rect, t, enabled);
    }
    let ir = egui::Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(16.0, 16.0));
    icons::paint(ui.painter(), ir, icon, if enabled { t.accent } else { t.faint });
    resp.on_hover_text(label)
}

pub fn tool_button(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, enabled: bool) -> Response {
    let label = tl!(label);
    let font = theme::regular(13.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, t.text);
    if ribbon_button(ui, galley.size().x) {
        let resp = icon_button(ui, icon, label, t, enabled, false);
        named(ui, &resp, label, enabled, None);
        return resp;
    }
    let size = vec2(galley.size().x + 36.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    named(ui, &resp, label, enabled, None);
    reached(ui, rect);
    let color = if enabled { t.text } else { t.faint };
    hover_fill(ui, &resp, rect, t, enabled);
    let ir = egui::Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(16.0, 16.0));
    icons::paint(ui.painter(), ir, icon, if enabled { t.accent } else { t.faint });
    ui.painter().galley_with_override_text_color(rect.min + vec2(30.0, (30.0 - galley.size().y) / 2.0), galley, color);
    resp
}

/// A "⋯" button in the style of [`pill_button`], for a menu of more actions.
pub fn more_pill(ui: &mut Ui, tip: &str, t: &Tokens) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 28.0), Sense::click());
    named(ui, &resp, tl!(tip), true, None);
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
    named(ui, &resp, label, enabled, None);
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
    named(ui, &resp, label, enabled, None);
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

/// Width of the labels and label buttons left of a form's fields (the message's To, Cc…).
pub(crate) const FIELD_LABEL_W: f32 = 76.0;
/// Height of a form's row: a field and the label button naming it.
pub(crate) const FIELD_ROW_H: f32 = 30.0;

/// Margin left and right of a field button's label.
const FIELD_BUTTON_MARGIN: f32 = 8.0;

/// Width of the button [`field_button`] draws for `label` (translated here): [`FIELD_LABEL_W`]
/// less its gap, or wider when the label as it reads in the language shown needs it
/// ("Calendrier ->").
pub(crate) fn field_button_w(ui: &Ui, label: &str, menu: bool) -> f32 {
    let text = ui.painter().layout_no_wrap(tl!(label).to_string(), theme::regular(13.0), egui::Color32::PLACEHOLDER).size().x;
    let chevron = if menu { 14.0 } else { 0.0 };
    (text + chevron + 2.0 * FIELD_BUTTON_MARGIN).max(FIELD_LABEL_W - 8.0)
}

/// A small framed button naming a field (To, Cc…), as in Outlook's message window; with `menu`, a
/// small drawn chevron after its label, the two centred together. `label` is translated here.
pub(crate) fn field_button(ui: &mut Ui, t: &Tokens, label: &str, enabled: bool, menu: bool) -> Response {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(field_button_w(ui, label, menu), FIELD_ROW_H - 4.0), if enabled { Sense::click() } else { Sense::hover() });
    named(ui, &resp, tl!(label), enabled, None);
    let fill = if resp.hovered() && enabled { t.hover } else { t.surface };
    ui.painter().rect(rect, 3.0, fill, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let color = if enabled { t.text } else { t.dim };
    let galley = ui.painter().layout_no_wrap(tl!(label).to_string(), theme::regular(13.0), color);
    let chevron = if menu { 14.0 } else { 0.0 };
    let left = rect.center().x - (galley.size().x + chevron) / 2.0;
    let text_pos = egui::pos2(left, rect.center().y - galley.size().y / 2.0);
    let text_w = galley.size().x;
    ui.painter().galley(text_pos, galley, color);
    if menu {
        let r = egui::Rect::from_center_size(egui::pos2(left + text_w + 4.0 + 5.0, rect.center().y + 0.5), vec2(10.0, 10.0));
        icons::paint(ui.painter(), r, Icon::ChevronDown, t.dim);
    }
    ui.add_space(8.0);
    resp
}

/// An icon button on the coloured title bar.
pub fn bar_icon_button(ui: &mut Ui, icon: Icon, tip: &str, t: &Tokens) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    named(ui, &resp, tl!(tip), true, None);
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    icons::paint(ui.painter(), icons::centered(rect, 16.0), icon, if resp.hovered() { t.text } else { t.dim });
    resp.on_hover_text(tl!(tip))
}

/// A small icon button inside panes.
pub fn tiny_icon_button(ui: &mut Ui, icon: Icon, tip: &str, t: &Tokens) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    named(ui, &resp, tl!(tip), true, None);
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
    named(ui, &resp, label, true, None);
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
/// A compact group's rows: three small buttons stacked in the height of a large one.
const SMALL_H: f32 = 22.0;
/// Between a compact group's columns.
const COLUMN_GAP: f32 = 4.0;
/// Items stacked in a compact group's column.
const ROWS: usize = 3;
/// What a group's end takes beyond its buttons: the row's spacing, the line, and the space
/// either side of it ([`Groups::end`]).
const GROUP_SEP: f32 = 11.0;

/// How the ribbon's groups fit the window: from `compact_from` on, each group is drawn compact
/// — its buttons small, stacked three to a column — as Outlook's ribbon shrinks its groups
/// from the right when the window is too narrow for the large ones. Measured in one frame
/// (every item's width drawn either way) for the next: [`GroupFit::fit`].
#[derive(Clone, Debug, Default)]
pub struct GroupFit {
    pub active: bool,
    /// Groups from this index on are compact (the first never is: New mail stays large).
    pub compact_from: usize,
    /// Each compact group's column widths, from the frame before (by group index).
    pub columns: Vec<Vec<f32>>,
    /// Measured this frame, per group: its width drawn large, drawn compact, and its columns
    /// when compact.
    pub groups: Vec<(f32, f32, Vec<f32>)>,
    /// Where the first group starts, and how far right the last one reached.
    pub origin: f32,
    pub right: f32,
    /// The group being drawn: its index, and its items so far (width drawn large, drawn
    /// compact; `true` for a menu arrow, which joins the item before it when compact).
    group: usize,
    items: Vec<(f32, f32, bool)>,
    /// When it's drawn compact: its rect and column widths.
    compact: Option<(egui::Rect, Vec<f32>)>,
}

impl GroupFit {
    /// The fit a ribbon is drawn with: the groups compact from `compact_from` on, with the
    /// columns measured the frame before.
    pub fn start(compact_from: usize, columns: Vec<Vec<f32>>) -> GroupFit {
        GroupFit { active: true, compact_from, columns, ..Default::default() }
    }

    /// The items as stacked: the compact width of each column, menu arrows joined to their item.
    fn columns_of(items: &[(f32, f32, bool)]) -> Vec<f32> {
        let mut stacked: Vec<f32> = Vec::new();
        for &(_, compact, arrow) in items {
            match (arrow, stacked.last_mut()) {
                (true, Some(prev)) => *prev += compact,
                (true, None) => {}
                (false, _) => stacked.push(compact),
            }
        }
        stacked.chunks(ROWS).map(|c| c.iter().copied().fold(0.0, f32::max)).collect()
    }

    /// A group's width drawn large and drawn compact (its name under it can widen either).
    fn widths(items: &[(f32, f32, bool)], name_w: f32) -> (f32, f32, Vec<f32>) {
        let spacing = 2.0;
        let large: f32 = items.iter().map(|i| i.0 + spacing).sum();
        let columns = Self::columns_of(items);
        let compact = columns.iter().sum::<f32>() + COLUMN_GAP * columns.len().saturating_sub(1) as f32 + spacing;
        (large.max(name_w + 8.0), compact.max(name_w + 8.0), columns)
    }

    /// The first group to draw compact so that the ribbon fits `room_right`: from the end, as
    /// few as it takes; all of them when even that is too wide (the row then scrolls).
    pub fn fit(&self, room_right: f32) -> usize {
        let n = self.groups.len();
        let room = room_right - self.origin - 4.0;
        let mut width: f32 = self.groups.iter().map(|g| g.0).sum::<f32>() + GROUP_SEP * n.saturating_sub(1) as f32;
        let mut from = n;
        while width > room && from > 1 {
            from -= 1;
            if let Some(g) = self.groups.get(from) {
                width -= g.0 - g.1;
            }
        }
        from
    }
}

pub fn group_fit_id() -> egui::Id {
    egui::Id::unique("group-fit")
}

/// A ribbon item's measure as it's drawn (`large`: a large button's width; `compact`: a small
/// one's; `arrow`: a menu arrow, drawn beside the item before it when compact). Its rect when
/// the group is drawn compact.
fn group_item(ui: &Ui, large: f32, compact: f32, arrow: bool) -> Option<egui::Rect> {
    ui.ctx().data_mut(|d| {
        let f = d.get_temp_mut_or_default::<GroupFit>(group_fit_id());
        if !f.active {
            return None;
        }
        f.items.push((large, compact, arrow));
        let (rect, cols) = f.compact.as_ref()?;
        // Where it goes: the arrow at the right of the item before it; the item in its column.
        let stacked: Vec<&(f32, f32, bool)> = f.items.iter().filter(|i| !i.2).collect();
        let k = stacked.len().checked_sub(1)?;
        let (c, r) = (k / ROWS, k % ROWS);
        let w = *cols.get(c)?;
        let x = rect.left() + cols.iter().take(c).sum::<f32>() + COLUMN_GAP * c as f32;
        let y = rect.top() + (SMALL_H + 1.0) * r as f32;
        let row = egui::Rect::from_min_size(egui::pos2(x, y), vec2(w, SMALL_H));
        Some(if arrow { egui::Rect::from_min_size(egui::pos2(row.right() - compact, y), vec2(compact, SMALL_H)) } else { row })
    })
}

/// A small button in a compact group: its icon and name on one line, in the row it was given.
fn small(ui: &mut Ui, icon: Icon, galley: std::sync::Arc<egui::Galley>, t: &Tokens, kind: Big, rect: egui::Rect) -> Response {
    let enabled = kind != Big::Off;
    let id = ui.scope_id().with(("ribbon-small", rect.min.x as i32, rect.min.y as i32));
    let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    named(ui, &resp, galley.text(), enabled, (kind == Big::On).then_some(true));
    let pressed = enabled && resp.is_pointer_button_down_on();
    if kind == Big::On {
        ui.painter().rect(rect, 4.0, t.accent_soft, Stroke::new(1.0, t.accent.gamma_multiply(0.5)), egui::StrokeKind::Inside);
    } else if enabled && (resp.hovered() || pressed) {
        ui.painter().rect_filled(rect, 4.0, if pressed { t.accent_soft } else { t.hover });
    }
    let icon_color = match kind {
        Big::Off => t.faint,
        Big::Accent => t.accent,
        _ => t.text,
    };
    icons::paint(ui.painter(), egui::Rect::from_min_size(rect.min + vec2(4.0, 3.0), vec2(16.0, 16.0)), icon, icon_color);
    let text = if enabled { t.text } else { t.faint };
    ui.painter().galley_with_override_text_color(rect.min + vec2(24.0, (SMALL_H - galley.size().y) / 2.0), galley, text);
    resp
}

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
    // In a group drawn compact (the window too narrow for the large buttons): small, stacked.
    let line = ui.painter().layout_no_wrap(label.to_string(), theme::regular(12.5), text);
    if let Some(rect) = group_item(ui, w, line.size().x + 30.0, false) {
        return small(ui, icon, line, t, kind, rect);
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(w, RIBBON_BUTTON_H), if enabled { Sense::click() } else { Sense::hover() });
    named(ui, &resp, label, enabled, (kind == Big::On).then_some(true));
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
    // Compact: at the right of the small button before it.
    if let Some(rect) = group_item(ui, 16.0, 14.0, true) {
        let id = ui.scope_id().with(("ribbon-small-more", rect.min.x as i32, rect.min.y as i32));
        let resp = ui.interact(rect, id, Sense::click());
        named(ui, &resp, tl!(tip), true, None);
        if resp.hovered() {
            ui.painter().rect_filled(rect, 4.0, t.hover);
        }
        icons::paint(ui.painter(), icons::centered(rect, 10.0), Icon::ChevronDown, if resp.hovered() { t.text } else { t.dim });
        return resp.on_hover_text(tl!(tip));
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(16.0, RIBBON_BUTTON_H), Sense::click());
    named(ui, &resp, tl!(tip), true, None);
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    let ir = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.min.y + 42.0), vec2(12.0, 12.0));
    icons::paint(ui.painter(), ir, Icon::ChevronDown, if resp.hovered() { t.text } else { t.dim });
    resp.on_hover_text(tl!(tip))
}

/// The ribbon's groups, laid out left to right: each ends with its name centred under its buttons
/// and a line after it, as Outlook's ribbon names its groups (New, Delete, Respond…). A group
/// the window has no room for is drawn compact ([`GroupFit`]).
pub struct Groups {
    start: f32,
    top: f32,
    index: usize,
}

impl Groups {
    /// Start the first group where the ribbon's row starts.
    pub fn new(ui: &mut Ui) -> Groups {
        let mut g = Groups { start: ui.cursor().min.x, top: ui.cursor().min.y, index: 0 };
        ui.ctx().data_mut(|d| {
            let f = d.get_temp_mut_or_default::<GroupFit>(group_fit_id());
            f.origin = g.start;
        });
        g.begin(ui);
        g
    }

    /// End the group begun last, named `name`, and start the next one after a line.
    pub fn end(&mut self, ui: &mut Ui, t: &Tokens, name: &str) {
        self.name(ui, t, name);
        ui.add_space(3.0);
        let (r, _) = ui.allocate_exact_size(vec2(1.0, RIBBON_BUTTON_H + GROUP_NAME_H - 6.0), Sense::hover());
        ui.painter().rect_filled(r, 0.0, t.border);
        ui.add_space(3.0);
        self.start = ui.cursor().min.x;
        self.index += 1;
        self.begin(ui);
    }

    /// End the last group, named `name`.
    pub fn last(&mut self, ui: &mut Ui, t: &Tokens, name: &str) {
        self.name(ui, t, name);
    }

    /// Begin a group: compact when the fit says so and its columns are known from the frame
    /// before (its whole rect taken now, its buttons placed in it as they come); large
    /// otherwise.
    fn begin(&mut self, ui: &mut Ui) {
        let columns = ui.ctx().data_mut(|d| {
            let f = d.get_temp_mut_or_default::<GroupFit>(group_fit_id());
            f.group = self.index;
            f.items.clear();
            f.compact = None;
            (f.active && self.index >= f.compact_from.max(1)).then(|| f.columns.get(self.index).cloned()).flatten().filter(|c| !c.is_empty())
        });
        if let Some(cols) = columns {
            let w = cols.iter().sum::<f32>() + COLUMN_GAP * cols.len().saturating_sub(1) as f32;
            let (rect, _) = ui.allocate_exact_size(vec2(w, RIBBON_BUTTON_H), Sense::hover());
            ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<GroupFit>(group_fit_id()).compact = Some((rect, cols)));
        }
    }

    fn name(&mut self, ui: &mut Ui, t: &Tokens, name: &str) {
        let galley = ui.painter().layout_no_wrap(tl!(name).to_string(), theme::regular(11.5), t.dim);
        let name_w = galley.size().x;
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
        // The group's measure, for the fit.
        ui.ctx().data_mut(|d| {
            let f = d.get_temp_mut_or_default::<GroupFit>(group_fit_id());
            if f.active {
                let (large, compact, columns) = GroupFit::widths(&f.items, name_w);
                f.groups.push((large, compact, columns));
                f.right = f.right.max(right);
            }
        });
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

/// How full something is, as a bar on its track (red past nine tenths): the mailbox on its
/// server.
pub fn meter(ui: &mut Ui, t: &Tokens, frac: f32, width: f32) -> Response {
    let frac = if frac.is_finite() { frac.clamp(0.0, 1.0) } else { 0.0 };
    let (r, resp) = ui.allocate_exact_size(vec2(width, 8.0), Sense::hover());
    ui.painter().rect_filled(r, 4.0, t.border);
    if frac > 0.0 {
        let full = egui::Rect::from_min_size(r.min, vec2((r.width() * frac).max(r.height()), r.height()));
        ui.painter().rect_filled(full, 4.0, if frac > 0.9 { t.danger } else { t.accent });
    }
    resp
}

/// A whole number with its thousands set apart, as the interface's language writes them
/// (`18,412`, `18 412`, `18.412`).
pub fn count_text(n: u64) -> String {
    let digits = n.to_string();
    let sep = crate::i18n::current().thousands();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 * sep.len_utf8());
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

pub fn human_size(bytes: usize) -> String {
    size_text(u64::try_from(bytes).unwrap_or(u64::MAX))
}

/// A size as people read it, in the interface's language: `12 KB`, `3.4 MB`, `41.2 GB` (`Ko`,
/// `Mo`, `Go` in French, with a decimal comma where the language writes one).
pub fn size_text(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    // Three figures at most: 2.8 GB, 96.0 MB, 830 MB.
    let tenths = |v: f64| {
        let s = if v < 100.0 { format!("{v:.1}") } else { format!("{v:.0}") };
        if crate::i18n::current().decimal_comma() { s.replace('.', ",") } else { s }
    };
    if b < K {
        tf!("{size} B", size = bytes)
    } else if b < K * K {
        tf!("{size} KB", size = format!("{:.0}", b / K))
    } else if b < K * K * K {
        tf!("{size} MB", size = tenths(b / K / K))
    } else {
        tf!("{size} GB", size = tenths(b / K / K / K))
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
