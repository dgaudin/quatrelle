//! The message list: date headings, rows with avatar, sender, subject and preview, unread and
//! flag markers, hover actions, multi-select (Ctrl/Shift), drag to a folder, right-click menu.
//! Only visible rows are laid out, so long folders scroll smoothly.

use egui::{Color32, Rect, RichText, Sense, Ui, pos2, vec2};
use quatrelle_engine::model::{FolderRole, MessageKey, Summary, addr, dates};
use quatrelle_engine::{ListFilter, Row, SearchScope, Session};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme;
use crate::widgets;
use crate::{Action, Dialog, UiState};

/// The list's rows (date headings and messages) and their tops, made again only when the list
/// changes (its revision), the day turns or the row height changes: a big folder's hundreds of
/// thousands of rows, each with its heading's label, were made again every frame.
pub(crate) struct ListRows {
    key: (u64, i64, u32),
    rows: Vec<Row>,
    tops: Vec<f32>,
    total: f32,
}

/// What a dragged row carries.
pub struct DragKeys(pub Vec<MessageKey>);

const HEADER_H: f32 = 30.0;

fn row_height(s: &Session) -> f32 {
    match (s.prefs.compact, s.prefs.show_preview) {
        (true, _) => 50.0,
        (false, true) => 80.0,
        (false, false) => 62.0,
    }
}

pub fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 10, top: 10, bottom: 6 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(crate::view_title(s)).font(theme::bold(18.0)).color(t.text));
        });
        ui.add_space(4.0);
        if s.focus_split() {
            ui.horizontal(|ui| {
                for (other, label) in [(false, "Focused"), (true, "Other")] {
                    let on = s.focus_other == other;
                    let text = RichText::new(tl!(label)).color(if on { t.text } else { t.dim }).font(if on {
                        theme::bold(14.0)
                    } else {
                        theme::regular(14.0)
                    });
                    let r = ui.add(egui::Button::new(text).frame_when_inactive(false));
                    if on {
                        ui.painter().line_segment(
                            [pos2(r.rect.left() + 2.0, r.rect.bottom() + 1.0), pos2(r.rect.right() - 2.0, r.rect.bottom() + 1.0)],
                            egui::Stroke::new(2.0, t.accent),
                        );
                    }
                    if r.clicked() && !on {
                        acts.push(Action::Run("list.focusTab", json!({"value": if other { "other" } else { "focused" }})));
                    }
                }
            });
            ui.add_space(2.0);
        }
        ui.horizontal(|ui| {
            // How the list is arranged, kept at the right (as Outlook's "By Date" above its list):
            // the tabs wrap in what's left, so long words (Dutch, German…) never run into it.
            let arrange = tf!("By {field} ⏷", field = tl!(s.prefs.arrange.label()));
            let right = if s.search.is_none() {
                ui.painter().layout_no_wrap(arrange.clone(), egui::TextStyle::Small.resolve(ui.style()), t.dim).size().x + 54.0
            } else {
                0.0
            };
            let left_w = (ui.available_width() - right).max(120.0);
            ui.allocate_ui_with_layout(vec2(left_w, 22.0), egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true), |ui| {
                // The tabs, and a filter chosen from the Home tab's Filter menu when it's another.
                let shown: Vec<ListFilter> = ListFilter::ALL.into_iter().chain((!ListFilter::ALL.contains(&s.filter)).then_some(s.filter)).collect();
                for f in shown {
                    let on = s.filter == f;
                    let text = RichText::new(tl!(f.label())).color(if on { t.accent } else { t.dim }).font(if on {
                        theme::bold(12.5)
                    } else {
                        theme::regular(12.5)
                    });
                    let r = ui.add(egui::Button::new(text).frame_when_inactive(false));
                    if on {
                        ui.painter().line_segment(
                            [pos2(r.rect.left() + 4.0, r.rect.bottom()), pos2(r.rect.right() - 4.0, r.rect.bottom())],
                            egui::Stroke::new(2.0, t.accent),
                        );
                    }
                    if r.clicked() {
                        acts.push(Action::Run("list.filter", json!({"value": f.code()})));
                    }
                }
                // A Search Folder always looks everywhere: no scope to pick.
                if let Some((q, current)) = s.search.as_ref().filter(|_| s.open_search_folder_name().is_none()) {
                    ui.add_space(8.0);
                    ui.menu_button(RichText::new(format!("{} ▾", crate::scope_label(*current))).small(), |ui| {
                        for scope in SearchScope::ALL {
                            if ui.selectable_label(scope == *current, crate::scope_label(scope)).clicked() {
                                acts.push(Action::Run("search.run", json!({"query": q, "scope": scope.code()})));
                                ui.close();
                            }
                        }
                    })
                    .response
                    .on_hover_text(tl!("Look in"));
                }
            });
            if s.search.is_none() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let rev = s.prefs.reverse_sort;
                    if ui
                        .add(egui::Button::new(RichText::new(if rev { "↓" } else { "↑" }).color(t.dim)).frame_when_inactive(false))
                        .on_hover_text(tl!("Reverse sort"))
                        .clicked()
                    {
                        acts.push(Action::Run("view.reverseSort", json!({})));
                    }
                    let r = ui.add(egui::Button::new(RichText::new(arrange).color(t.dim).small()).frame_when_inactive(false));
                    egui::Popup::menu(&r).show(|ui| arrange_menu(ui, s, acts));
                });
            }
        });
    });
    let sep = ui.available_rect_before_wrap();
    ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));
    let sent_view = matches!(s.view_role(), Some(FolderRole::Sent | FolderRole::Drafts)) || s.view == quatrelle_engine::View::Outbox;
    // A table where the list is wide (see `Session::list_as_table`): its headings stay put above
    // the rows, sized as the rows are (the scroll bar's width aside).
    let table = s.list_as_table();
    let cols = columns(ui.available_width() - ui.spacing().scroll.bar_width - 2.0, &st.list_columns);
    if table {
        table_header(s, &t, ui, &cols, sent_view, &mut st.list_columns, acts);
    }

    // The row height differs between the table and the list (the fork's table keeps its own
    // heights), so the row cache is keyed on it too.
    let mh = if table { table_row_height(s) } else { row_height(s) };
    let hh = if table { TABLE_HEADING_H } else { HEADER_H };
    let key = (s.revision, dates::today(&s.clock), mh.to_bits());
    if st.list_rows.as_ref().is_none_or(|c| c.key != key) {
        let rows = s.rows();
        let mut tops = Vec::with_capacity(rows.len());
        let mut y = 0.0;
        for r in &rows {
            tops.push(y);
            y += if matches!(r, Row::Header(_) | Row::Group(_)) { hh } else { mh };
        }
        st.list_rows = Some(ListRows { key, rows, tops, total: y });
    }
    let Some(cached) = st.list_rows.take() else { return };
    let ListRows { rows, tops, total, .. } = &cached;
    let (rows, tops, total) = (rows.as_slice(), tops.as_slice(), *total);
    if rows.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(60.0);
            let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
            icons::paint(ui.painter(), r, if s.search.is_some() { Icon::Search } else { Icon::Inbox }, t.faint);
            ui.add_space(8.0);
            let msg = if s.search.as_ref().is_some_and(|(_, scope)| *scope == SearchScope::All) {
                tl!("No results. Try fewer words.")
            } else if s.search.is_some() {
                tl!("No results. Try fewer words, or search all folders.")
            } else if s.filter != ListFilter::All {
                tl!("Nothing matches this filter.")
            } else {
                tl!("All caught up: nothing in this folder.")
            };
            ui.label(RichText::new(msg).color(t.dim));
            ui.add_space(10.0);
            footer(s, &t, ui, acts);
        });
        st.list_rows = Some(cached);
        return;
    }

    let focus_index = s.focus.as_ref().and_then(|k| s.list.iter().position(|m| &m.key == k));
    let scroll_to = if st.scroll_to_focus {
        st.scroll_to_focus = false;
        focus_index.and_then(|fi| rows.iter().position(|r| matches!(r, Row::Message { index, .. } if *index == fi)))
    } else {
        None
    };

    // After the last message: more on the server, or searching it (issue #4).
    let has_footer = s.more_on_server().is_some() || s.search.is_some();
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("list").show_viewport(ui, |ui, viewport| {
        ui.set_height(total + if has_footer { FOOTER_H } else { 0.0 });
        let origin = ui.min_rect().min;
        let width = ui.available_width();
        if let Some(ri) = scroll_to
            && let Some(&top) = tops.get(ri)
        {
            let r = Rect::from_min_size(origin + vec2(0.0, top), vec2(width, mh));
            ui.scroll_to_rect(r, None);
        }
        let first = tops.partition_point(|&top| top + mh < viewport.min.y);
        // Rows come without a preview when their source was downloaded ahead: worked out for
        // the rows drawn, after the frame.
        let previews = s.prefs.show_preview && !s.prefs.compact;
        for (i, row) in rows.iter().enumerate().skip(first) {
            let top = tops.get(i).copied().unwrap_or(0.0);
            if top > viewport.max.y {
                break;
            }
            match row {
                Row::Header(label) => {
                    let r = Rect::from_min_size(origin + vec2(0.0, top), vec2(width, hh));
                    heading(ui, &t, r, tl!(label), table);
                }
                Row::Group(name) => {
                    let r = Rect::from_min_size(origin + vec2(0.0, top), vec2(width, hh));
                    let (name, _) = crate::bidi::shown(name);
                    heading(ui, &t, r, name.as_ref(), table);
                }
                Row::Message { index, count, unread } => {
                    let Some(m) = s.list.get(*index) else { continue };
                    if previews && s.preview_wanted(m) {
                        st.want_previews.push(m.key.clone());
                    }
                    let r = Rect::from_min_size(origin + vec2(0.0, top), vec2(width, mh));
                    if table {
                        table_row(s, &t, ui, r, &cols, m, *count, *unread, sent_view, acts);
                    } else {
                        message_row(s, &t, ui, r, m, *count, *unread, sent_view, acts);
                    }
                }
            }
        }
        if has_footer && total < viewport.max.y {
            let r = Rect::from_min_size(origin + vec2(0.0, total), vec2(width, FOOTER_H));
            ui.scope_builder(egui::UiBuilder::new().max_rect(r.shrink2(vec2(14.0, 8.0))), |ui| {
                ui.vertical_centered(|ui| footer(s, &t, ui, acts));
            });
        }
    });
    st.list_rows = Some(cached);
}

/// The list's footer's height.
const FOOTER_H: f32 = 76.0;

/// What's past the list (issue #4): a folder's older mail on its server, to show more of; a
/// search's, to ask the servers for.
fn footer(s: &Session, t: &theme::Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    if let Some((account, folder, n)) = s.more_on_server() {
        ui.label(RichText::new(tf!("{n} more on the server", n = n)).color(t.dim).small());
        if s.showing_more.as_ref().is_some_and(|(a, f)| *a == account && *f == folder) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tl!("Getting them from the server…")).color(t.dim).small());
            });
        } else if ui.button(tl!("Show more")).clicked() {
            acts.push(Action::Run("folder.more", json!({"account": account, "folder": folder})));
        }
        return;
    }
    if s.search.is_none() || s.accounts.iter().all(|a| a.is_local() || a.config.pop.is_some()) {
        return;
    }
    match &s.server_search {
        Some(ss) if Some(&ss.query) == s.search.as_ref().map(|(q, _)| q) => {
            if ss.pending > 0 {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(tl!("Searching the server…")).color(t.dim).small());
                });
            } else {
                ui.label(RichText::new(tf!("The server found {n}", n = ss.found.len())).color(t.dim).small());
            }
            for e in ss.errors.iter().take(3) {
                ui.add(egui::Label::new(RichText::new(e).color(t.warn).small()).wrap());
            }
        }
        _ => {
            ui.label(RichText::new(tl!("Mail older than what's kept here is only on the server.")).color(t.dim).small());
            if ui.button(tl!("Search the server")).clicked() {
                acts.push(Action::Run("search.server", json!({})));
            }
        }
    }
}

/// A date (or other) heading between the rows.
fn heading(ui: &Ui, t: &theme::Tokens, r: Rect, text: &str, table: bool) {
    if table {
        ui.painter().text(r.left_center() + vec2(10.0, 0.0), egui::Align2::LEFT_CENTER, text, theme::bold(12.0), t.dim);
        ui.painter().line_segment([r.left_bottom(), r.right_bottom()], egui::Stroke::new(1.0, t.border.gamma_multiply(0.6)));
    } else {
        ui.painter().text(r.left_bottom() + vec2(14.0, -8.0), egui::Align2::LEFT_BOTTOM, text, theme::bold(12.5), t.dim);
    }
}

/// The table's headings, and its date headings.
const TABLE_HEADER_H: f32 = 24.0;
const TABLE_HEADING_H: f32 = 24.0;

/// A table row: one line, or two with the preview under the sender and subject.
fn table_row_height(s: &Session) -> f32 {
    if s.prefs.show_preview && !s.prefs.compact { 40.0 } else { 26.0 }
}

/// The table's columns, as Outlook's: importance, attachment, sender (recipients in Sent),
/// subject, received, size, categories, flag.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Col {
    Importance,
    Attach,
    Who,
    Subject,
    Received,
    Size,
    Categories,
    Flag,
}

/// The widths the table's columns were dragged to (`None`: as wide as it sets them), kept with
/// the app's settings. The subject takes what the others leave.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ColumnWidths {
    pub who: Option<f32>,
    pub received: Option<f32>,
    pub size: Option<f32>,
    pub categories: Option<f32>,
}

impl ColumnWidths {
    const WHO: (f32, f32) = (60.0, 600.0);
    const RECEIVED: (f32, f32) = (70.0, 320.0);
    const SIZE: (f32, f32) = (40.0, 160.0);
    const CATEGORIES: (f32, f32) = (50.0, 320.0);

    /// Within reason (a hand-edited or damaged setting can't hide a column).
    pub fn clamped(self) -> ColumnWidths {
        let c = |v: Option<f32>, (lo, hi): (f32, f32)| v.filter(|w| w.is_finite()).map(|w| w.clamp(lo, hi));
        ColumnWidths {
            who: c(self.who, Self::WHO),
            received: c(self.received, Self::RECEIVED),
            size: c(self.size, Self::SIZE),
            categories: c(self.categories, Self::CATEGORIES),
        }
    }

    /// The width a column's drag handle changes: the sender's by its right edge, the columns
    /// after the subject by their left edge.
    fn slot(&mut self, c: Col) -> Option<(&mut Option<f32>, (f32, f32), f32)> {
        match c {
            Col::Who => Some((&mut self.who, Self::WHO, 1.0)),
            Col::Received => Some((&mut self.received, Self::RECEIVED, -1.0)),
            Col::Size => Some((&mut self.size, Self::SIZE, -1.0)),
            Col::Categories => Some((&mut self.categories, Self::CATEGORIES, -1.0)),
            _ => None,
        }
    }
}

/// Each column's span from the row's left edge: the narrow and dated ones fixed (or as wide as
/// they were dragged), the sender and subject sharing the rest; size and categories go first
/// when the list is narrow.
fn columns(width: f32, w: &ColumnWidths) -> Vec<(Col, f32, f32)> {
    let mut fixed_right = vec![(Col::Received, w.received.unwrap_or(156.0))];
    if width >= 620.0 {
        fixed_right.push((Col::Size, w.size.unwrap_or(64.0)));
    }
    if width >= 760.0 {
        fixed_right.push((Col::Categories, w.categories.unwrap_or(110.0)));
    }
    fixed_right.push((Col::Flag, 26.0));
    let left = 8.0 + 18.0 + 20.0;
    let right: f32 = fixed_right.iter().map(|(_, w)| w).sum();
    let rest = (width - left - right - 6.0).max(120.0);
    // The subject keeps 80 points whatever the sender's width.
    let who = w.who.unwrap_or((rest * 0.34).clamp(90.0, 260.0)).min((rest - 80.0).max(60.0));
    let mut out =
        vec![(Col::Importance, 8.0, 26.0), (Col::Attach, 26.0, 46.0), (Col::Who, left, left + who), (Col::Subject, left + who, left + rest)];
    let mut x = left + rest;
    for (c, w) in fixed_right {
        out.push((c, x, x + w));
        x += w;
    }
    out
}

/// What a column's heading arranges the list by, when it does.
fn column_arrange(c: Col, sent_view: bool) -> Option<quatrelle_engine::Arrange> {
    use quatrelle_engine::Arrange as A;
    match c {
        Col::Attach => Some(A::Attachments),
        Col::Who => Some(if sent_view { A::To } else { A::From }),
        Col::Subject => Some(A::Subject),
        Col::Received => Some(A::Date),
        Col::Size => Some(A::Size),
        Col::Flag => Some(A::Flag),
        Col::Importance | Col::Categories => None,
    }
}

/// The table's headings: a click arranges the list by that column, again turns it round.
#[allow(clippy::too_many_arguments)]
fn table_header(
    s: &Session,
    t: &theme::Tokens,
    ui: &mut Ui,
    cols: &[(Col, f32, f32)],
    sent_view: bool,
    widths: &mut ColumnWidths,
    acts: &mut Vec<Action>,
) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), TABLE_HEADER_H), Sense::hover());
    let p = ui.painter_at(r);
    p.line_segment([r.left_bottom(), r.right_bottom()], egui::Stroke::new(1.0, t.border));
    for &(c, x0, x1) in cols {
        let cell = Rect::from_x_y_ranges(r.min.x + x0..=r.min.x + x1, r.y_range());
        let arrange = column_arrange(c, sent_view);
        let resp = ui.interact(cell, ui.scope_id().with(("col", c as u8)), if arrange.is_some() { Sense::click() } else { Sense::hover() });
        if arrange.is_some() && resp.hovered() {
            p.rect_filled(cell, 0.0, t.hover);
        }
        if x0 > 30.0 {
            p.line_segment([cell.left_top() + vec2(0.0, 5.0), cell.left_bottom() - vec2(0.0, 5.0)], egui::Stroke::new(1.0, t.border));
        }
        let active = arrange.is_some_and(|a| a == s.prefs.arrange);
        let label = match c {
            Col::Importance => "!".to_string(),
            Col::Attach | Col::Flag => String::new(),
            Col::Who => tl!(if sent_view { "To" } else { "From" }).to_string(),
            Col::Subject => tl!("Subject").to_string(),
            Col::Received => tl!("Received").to_string(),
            Col::Size => tl!("Size").to_string(),
            Col::Categories => tl!("Categories").to_string(),
        };
        let color = if active { t.text } else { t.dim };
        match c {
            Col::Attach => icons::paint(&p, icons::centered(cell, 13.0), Icon::Attach, color),
            Col::Flag => icons::paint(&p, icons::centered(cell, 13.0), Icon::Flag, color),
            Col::Importance => {
                p.text(cell.center(), egui::Align2::CENTER_CENTER, label, theme::bold(12.0), color);
            }
            _ => {
                let font = if active { theme::bold(12.0) } else { theme::regular(12.0) };
                let g = p.layout_no_wrap(label, font, color);
                let w = g.size().x;
                p.with_clip_rect(cell.shrink2(vec2(4.0, 0.0))).galley(egui::pos2(cell.min.x + 6.0, cell.center().y - g.size().y / 2.0), g, color);
                if active {
                    let arrow = if s.prefs.reverse_sort { "▲" } else { "▼" };
                    p.text(
                        egui::pos2((cell.min.x + 12.0 + w).min(cell.max.x - 8.0), cell.center().y),
                        egui::Align2::LEFT_CENTER,
                        arrow,
                        theme::regular(9.0),
                        t.dim,
                    );
                }
            }
        }
        if let Some(a) = arrange
            && resp.on_hover_text(tf!("Arrange by: {field}", field = tl!(a.label()))).clicked()
        {
            if active {
                acts.push(Action::Run("view.reverseSort", json!({})));
            } else {
                acts.push(Action::Run("view.arrange", json!({"by": a.code()})));
            }
        }
    }
    // Above the headings (registered last, so it wins the hover at an edge).
    // The edges between headings resize the columns, dragged (a double-click sets it back).
    for &(c, x0, x1) in cols {
        let current = x1 - x0;
        let Some((slot, (lo, hi), sign)) = widths.slot(c) else { continue };
        let edge = r.min.x + if sign > 0.0 { x1 } else { x0 };
        let grip = Rect::from_x_y_ranges(edge - 3.0..=edge + 3.0, r.y_range());
        let resp =
            ui.interact(grip, ui.scope_id().with(("col-edge", c as u8)), Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if resp.dragged() {
            let dx = resp.drag_delta().x * sign;
            *slot = Some((slot.unwrap_or(current) + dx).clamp(lo, hi));
        }
        if resp.double_clicked() {
            *slot = None;
        }
        if resp.hovered() || resp.dragged() {
            p.line_segment([egui::pos2(edge, r.min.y + 3.0), egui::pos2(edge, r.max.y - 3.0)], egui::Stroke::new(2.0, t.accent));
        }
    }
}

/// A message as a table row: its cells in the columns, bold while unread.
#[allow(clippy::too_many_arguments)]
fn table_row(
    s: &Session,
    t: &theme::Tokens,
    ui: &mut Ui,
    r: Rect,
    cols: &[(Col, f32, f32)],
    m: &Summary,
    count: usize,
    unread_in_thread: usize,
    sent_view: bool,
    acts: &mut Vec<Action>,
) {
    let id = ui.make_persistent_id(("row", &m.key));
    let resp = ui.interact(r, id, Sense::click_and_drag());
    let hovered = ui.rect_contains_pointer(r) && !ui.ctx().is_being_dragged(id) || resp.hovered();
    let selected = s.selection.contains(&m.key);
    let unread = if count > 1 { unread_in_thread > 0 } else { !m.flags.seen };
    let p = ui.painter_at(r);
    let fill = if selected {
        t.accent_soft
    } else if hovered {
        t.hover_row
    } else {
        Color32::TRANSPARENT
    };
    p.rect_filled(r, 0.0, fill);
    if unread {
        p.rect_filled(Rect::from_min_size(r.min + vec2(1.0, 4.0), vec2(3.0, r.height() - 8.0)), 1.5, t.accent);
    }
    p.line_segment([r.left_bottom(), r.right_bottom()], egui::Stroke::new(1.0, t.border));
    let two_lines = r.height() > 30.0;
    let line1 = if two_lines { r.min.y + 11.0 } else { r.center().y };
    let font = |size: f32| if unread { theme::bold(size) } else { theme::regular(size) };
    let mut clicked_action = false;
    for &(c, x0, x1) in cols {
        let cell = Rect::from_x_y_ranges(r.min.x + x0..=r.min.x + x1, r.y_range());
        let text_cell = |p: &egui::Painter, text: String, color: Color32, size: f32| {
            let (text, rtl) = crate::bidi::shown(&text);
            let g = p.layout_no_wrap(text.into_owned(), font(size), color);
            let inner = cell.shrink2(vec2(6.0, 0.0));
            let x = if rtl { (inner.max.x - g.size().x).max(inner.min.x) } else { inner.min.x };
            p.with_clip_rect(inner).galley(egui::pos2(x, line1 - g.size().y / 2.0), g, color);
        };
        match c {
            Col::Importance if m.importance != 0 => {
                let (mark, color) = if m.importance > 0 { ("!", t.danger) } else { ("↓", t.accent) };
                p.text(egui::pos2(cell.center().x, line1), egui::Align2::CENTER_CENTER, mark, theme::bold(13.0), color);
            }
            Col::Attach if m.has_attachments => {
                icons::paint(&p, Rect::from_center_size(egui::pos2(cell.center().x, line1), vec2(13.0, 13.0)), Icon::Attach, t.dim);
            }
            Col::Who => {
                let who = if sent_view { addr::display_list(&m.to) } else { m.from.display().to_string() };
                let who = if count > 1 { format!("{who}  ({count})") } else { who };
                text_cell(&p, who, t.text, 13.0);
            }
            Col::Subject => {
                let subject = if m.subject.is_empty() { tl!("(no subject)").to_string() } else { m.subject.clone() };
                text_cell(&p, subject, if unread { t.accent } else { t.text }, 13.0);
            }
            Col::Received => {
                let when = crate::i18n::date(&dates::full_time(m.date, &s.clock));
                text_cell(&p, when, if unread { t.accent } else { t.dim }, 12.0);
            }
            Col::Size => text_cell(&p, widgets::human_size(m.size as usize), t.dim, 12.0),
            Col::Categories => {
                let cats = s.categories_of(m);
                let mut x = cell.min.x + 6.0;
                for c in cats.iter().take(3) {
                    p.circle_filled(egui::pos2(x + 5.0, line1), 5.0, theme::category_color(c.color, t.dark));
                    x += 13.0;
                }
                if let Some(first) = cats.first() {
                    let g = p.layout_no_wrap(first.name.clone(), theme::regular(12.0), t.dim);
                    p.with_clip_rect(cell.shrink2(vec2(4.0, 0.0))).galley(egui::pos2(x + 2.0, line1 - g.size().y / 2.0), g, t.dim);
                }
            }
            Col::Flag => {
                // Flag or unflag from the row, as Outlook's flag column.
                let fr = Rect::from_center_size(egui::pos2(cell.center().x, line1), vec2(20.0, 20.0));
                let fresp = ui.interact(fr, id.with("flag"), Sense::click()).on_hover_text(tl!(if m.flags.flagged { "Unflag" } else { "Flag" }));
                if m.flags.flagged || fresp.hovered() || hovered {
                    let color = if m.flags.flagged { t.flag } else { t.faint };
                    icons::paint(&p, icons::centered(fr, 13.0), Icon::Flag, color);
                }
                if fresp.clicked() {
                    clicked_action = true;
                    if !selected {
                        acts.push(Action::Run("list.select", json!({"key": m.key.to_id()})));
                    }
                    acts.push(Action::Run("mail.flag", json!({})));
                }
            }
            _ => {}
        }
    }
    // The preview under the sender and subject.
    let preview = s.preview_of(m);
    if two_lines && !preview.is_empty() {
        let span = cols
            .iter()
            .filter(|(c, _, _)| matches!(c, Col::Who | Col::Subject))
            .fold((f32::MAX, 0.0f32), |(a, b), (_, x0, x1)| (a.min(*x0), b.max(*x1)));
        let inner = Rect::from_x_y_ranges(r.min.x + span.0 + 6.0..=r.min.x + span.1 - 6.0, r.y_range());
        let (preview, _) = crate::bidi::shown(preview);
        let g = p.layout_no_wrap(preview.into_owned(), theme::regular(12.0), t.dim);
        p.with_clip_rect(inner).galley(egui::pos2(inner.min.x, r.min.y + 20.0), g, t.dim);
    }
    row_input(s, t, ui, &resp, id, m, selected, clicked_action, acts);
}

/// Arrange the list by…, as a menu (the list's own and the View tab's).
pub(crate) fn arrange_menu(ui: &mut Ui, s: &Session, acts: &mut Vec<Action>) {
    for a in quatrelle_engine::Arrange::ALL {
        if ui.selectable_label(s.prefs.arrange == a, tl!(a.label())).clicked() {
            acts.push(Action::Run("view.arrange", json!({"by": a.code()})));
            ui.close();
        }
    }
    ui.separator();
    if ui.selectable_label(s.prefs.reverse_sort, tl!("Reverse sort")).clicked() {
        acts.push(Action::Run("view.reverseSort", json!({})));
        ui.close();
    }
}

#[allow(clippy::too_many_arguments)]
fn message_row(
    s: &Session,
    t: &theme::Tokens,
    ui: &mut Ui,
    r: Rect,
    m: &Summary,
    count: usize,
    unread_in_thread: usize,
    sent_view: bool,
    acts: &mut Vec<Action>,
) {
    let id = ui.make_persistent_id(("row", &m.key));
    let resp = ui.interact(r, id, Sense::click_and_drag());
    // The pointer anywhere on the row, its own buttons included (egui hovers only the button
    // under the pointer, and the buttons must stay while they're pointed at).
    let hovered = ui.rect_contains_pointer(r) && !ui.ctx().is_being_dragged(id) || resp.hovered();
    let selected = s.selection.contains(&m.key);
    let unread = if count > 1 { unread_in_thread > 0 } else { !m.flags.seen };
    let p = ui.painter_at(r);
    let sel = crate::motion::toward(ui.ctx(), id.with("selected"), selected, crate::motion::NORMAL);
    let hover = crate::motion::toward(ui.ctx(), id.with("hover"), hovered, crate::motion::FAST);
    let fill = crate::motion::mix(t.hover_row.gamma_multiply(hover), t.accent_soft, sel);
    p.rect_filled(r.shrink2(vec2(4.0, 1.0)), 6.0, fill);
    if unread {
        p.rect_filled(Rect::from_min_size(r.min + vec2(4.0, 8.0), vec2(3.0, r.height() - 16.0)), 2.0, t.accent);
    }
    // A line between two messages, as Outlook's: light, but plain to see, and across the row.
    p.line_segment([r.left_bottom() + vec2(8.0, 0.0), r.right_bottom() - vec2(8.0, 0.0)], egui::Stroke::new(1.0, t.border));

    let compact = s.prefs.compact;
    let mut x = r.min.x + 14.0;
    let who_addr = if sent_view { m.to.first().cloned().unwrap_or_default() } else { m.from.clone() };
    if !compact {
        let c = egui::pos2(x + 17.0, r.min.y + 26.0);
        // A sender whose picture we have (the directory's) shows it; otherwise their initials.
        match crate::people::address_picture(ui, s, &who_addr.email) {
            Some(tex) => {
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(tex.id(), egui::Rect::from_center_size(c, vec2(34.0, 34.0)), uv, Color32::WHITE);
            }
            None => widgets::avatar(ui, c, 17.0, &who_addr.initials(), theme::avatar_color(&who_addr.email, t.dark)),
        }
        x += 44.0;
    }
    let right = r.max.x - 14.0;
    let line1 = r.min.y + if compact { 7.0 } else { 11.0 };
    // Sender (or recipients in Sent) and time.
    let time = crate::i18n::date(&dates::list_time(m.date, &s.clock));
    let time_g = p.layout_no_wrap(time, if unread { theme::bold(12.0) } else { theme::regular(12.0) }, if unread { t.accent } else { t.dim });
    let tw = time_g.size().x;
    p.galley(egui::pos2(right - tw, line1 + 1.0), time_g, t.dim);
    let who = if sent_view { tf!("To: {names}", names = addr::display_list(&m.to)) } else { m.from.display().to_string() };
    let who = if count > 1 { format!("{who}  ({count})") } else { who };
    let (who, who_rtl) = crate::bidi::shown(&who);
    let who_g = p.layout_no_wrap(who.into_owned(), if unread { theme::bold(14.0) } else { theme::regular(14.0) }, t.text);
    // Right-to-left text starts at the right, so that's where it goes when it doesn't fit.
    let who_x = if who_rtl { (right - tw - 10.0 - who_g.size().x).max(x) } else { x };
    p.with_clip_rect(Rect::from_x_y_ranges(x..=right - tw - 8.0, r.y_range())).galley(egui::pos2(who_x, line1), who_g, t.text);

    // Subject with markers.
    let line2 = line1 + 19.0;
    let mut icons_w = 0.0;
    let marker = |ic: Icon, color: Color32, w: &mut f32| {
        *w += 18.0;
        let ir = Rect::from_min_size(egui::pos2(right - *w + 3.0, line2 + 1.0), vec2(14.0, 14.0));
        icons::paint(&p, ir, ic, color);
    };
    if !hovered {
        if m.flags.pinned() {
            marker(Icon::Pin, t.accent, &mut icons_w);
        }
        if m.flags.flagged {
            marker(Icon::Flag, t.flag, &mut icons_w);
        }
        if m.has_attachments {
            marker(Icon::Attach, t.dim, &mut icons_w);
        }
        // Importance the sender set: a red ! for high, a ↓ for low.
        if m.importance != 0 {
            icons_w += 12.0;
            let (mark, color) = if m.importance > 0 { ("!", t.danger) } else { ("↓", t.accent) };
            p.text(egui::pos2(right - icons_w + 6.0, line2 + 8.0), egui::Align2::CENTER_CENTER, mark, theme::bold(13.0), color);
        }
        if m.flags.answered {
            marker(Icon::Reply, t.faint, &mut icons_w);
        } else if m.flags.forwarded {
            marker(Icon::Forward, t.faint, &mut icons_w);
        }
    }
    // Categories: a coloured dot each, before the markers.
    if !hovered {
        for c in s.categories_of(m).into_iter().rev().take(4) {
            icons_w += 13.0;
            p.circle_filled(egui::pos2(right - icons_w + 7.0, line2 + 8.0), 5.0, theme::category_color(c.color, t.dark));
        }
    }
    let subject = if m.subject.is_empty() { tl!("(no subject)").to_string() } else { m.subject.clone() };
    let subj_color = if unread { t.accent } else { t.text };
    let (subject, subj_rtl) = crate::bidi::shown(&subject);
    let subj_g = p.layout_no_wrap(subject.into_owned(), if unread { theme::bold(13.0) } else { theme::regular(13.0) }, subj_color);
    let subj_right = right - icons_w.max(if hovered { 70.0 } else { 0.0 }) - 4.0;
    let subj_x = if subj_rtl { (subj_right - subj_g.size().x).max(x) } else { x };
    p.with_clip_rect(Rect::from_x_y_ranges(x..=subj_right, r.y_range())).galley(egui::pos2(subj_x, line2), subj_g, subj_color);
    let preview = s.preview_of(m);
    if s.prefs.show_preview && !compact && !preview.is_empty() {
        let (preview, pv_rtl) = crate::bidi::shown(preview);
        let pv = p.layout_no_wrap(preview.into_owned(), theme::regular(12.5), t.dim);
        let pv_x = if pv_rtl { (right - pv.size().x).max(x) } else { x };
        p.with_clip_rect(Rect::from_x_y_ranges(x..=right, r.y_range())).galley(egui::pos2(pv_x, line2 + 19.0), pv, t.dim);
    }

    // Hover actions.
    let mut clicked_action = false;
    if hovered {
        let mut bx = right - 22.0;
        let actions: [(Icon, &str, &'static str, serde_json::Value); 3] = [
            (Icon::Delete, "Delete", "mail.delete", json!({})),
            (Icon::Flag, if m.flags.flagged { "Unflag" } else { "Flag" }, "mail.flag", json!({})),
            (
                if m.flags.seen { Icon::Unread } else { Icon::Read },
                tl!(if m.flags.seen { "Mark unread" } else { "Mark read" }),
                "mail.toggleRead",
                json!({}),
            ),
        ];
        for (ic, tip, cmd, params) in actions {
            let br = Rect::from_min_size(egui::pos2(bx, line2 - 3.0), vec2(22.0, 22.0));
            let bresp = ui.interact(br, id.with(cmd), Sense::click()).on_hover_text(tip);
            if bresp.hovered() {
                ui.painter().rect_filled(br, 4.0, t.border);
            }
            let color = if cmd == "mail.flag" && m.flags.flagged { t.flag } else { t.dim };
            icons::paint(ui.painter(), icons::centered(br, 14.0), ic, color);
            if bresp.clicked() {
                clicked_action = true;
                if !selected {
                    acts.push(Action::Run("list.select", json!({"key": m.key.to_id()})));
                }
                acts.push(Action::Run(cmd, params));
            }
            bx -= 24.0;
        }
    }

    row_input(s, t, ui, &resp, id, m, selected, clicked_action, acts);
}

/// What a row does with the pointer, in cards and in the table alike: a click opens it (Ctrl
/// adds it to the selection, Shift selects up to it), a double-click opens it in a window (a
/// draft carries on being written), a drag takes it to a folder, a right-click gives its menu.
#[allow(clippy::too_many_arguments)]
fn row_input(
    s: &Session,
    t: &theme::Tokens,
    ui: &mut Ui,
    resp: &egui::Response,
    id: egui::Id,
    m: &Summary,
    selected: bool,
    clicked_action: bool,
    acts: &mut Vec<Action>,
) {
    if resp.clicked() && !clicked_action {
        let mods = ui.input(|i| i.modifiers);
        if mods.command || mods.ctrl {
            acts.push(Action::Run("list.select", json!({"key": m.key.to_id(), "add": true})));
        } else if mods.shift {
            range_select(s, &m.key, acts);
        } else {
            acts.push(Action::Open(m.key.clone()));
        }
    }
    if resp.double_clicked() {
        // Drafts carry on being written; other mail opens in a window of its own.
        if s.view_role() == Some(FolderRole::Drafts) {
            acts.push(Action::Open(m.key.clone()));
        } else {
            acts.push(Action::Run("message.popOut", json!({"key": m.key.to_id()})));
        }
    }
    if resp.drag_started() {
        if !selected {
            acts.push(Action::Run("list.select", json!({"key": m.key.to_id()})));
        }
        let keys = if selected { s.selection.clone() } else { vec![m.key.clone()] };
        resp.dnd_set_drag_payload(DragKeys(keys));
    }
    if resp.dragged() {
        let n = if selected { s.selection.len() } else { 1 };
        egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), id.with("drag"), egui::PopupAnchor::Pointer).show(|ui| {
            ui.label(if n == 1 { tl!("Move 1 message").to_string() } else { tf!("Move {n} messages", n = n) });
        });
    }
    let key = m.key.clone();
    let flags = m.flags.clone();
    let (subject, sender) = (m.subject.clone(), m.from.email.clone());
    resp.context_menu(|ui| {
        if !s.selection.contains(&key) {
            acts.push(Action::Run("list.select", json!({"key": key.to_id()})));
        }
        let item = |ui: &mut Ui, acts: &mut Vec<Action>, label: &str, act: Action| {
            if ui.button(label).clicked() {
                acts.push(act);
                ui.close();
            }
        };
        item(ui, acts, tl!("Open in new window"), Action::Run("message.popOut", json!({"key": key.to_id()})));
        ui.separator();
        item(ui, acts, tl!("Reply"), Action::Run("mail.reply", json!({})));
        item(ui, acts, tl!("Reply all"), Action::Run("mail.replyAll", json!({})));
        item(ui, acts, tl!("Forward"), Action::Run("mail.forward", json!({})));
        item(ui, acts, tl!("Quick print"), Action::Print);
        item(ui, acts, tl!("Save as…"), Action::SaveSelected);
        ui.separator();
        item(
            ui,
            acts,
            tl!(if flags.seen { "Mark as unread" } else { "Mark as read" }),
            Action::Run(if flags.seen { "mail.markUnread" } else { "mail.markRead" }, json!({})),
        );
        ui.menu_button(tl!("Follow up"), |ui| crate::organize::follow_up_menu(ui, s, t, acts));
        ui.separator();
        item(ui, acts, tl!("Move to…"), Action::Dialog(Dialog::move_to()));
        item(ui, acts, tl!("Archive"), Action::Run("mail.archive", json!({})));
        item(ui, acts, tl!("Report junk"), Action::Run("mail.junk", json!({})));
        ui.menu_button(tl!("Junk"), |ui| crate::junk_menu(ui, s, acts));
        item(ui, acts, tl!("Ignore"), Action::Run("mail.ignore", json!({})));
        item(ui, acts, tl!("Delete"), Action::Run("mail.delete", json!({})));
        ui.separator();
        // Find related, as Outlook's: the conversation, or everything from the sender.
        ui.menu_button(tl!("Find related"), |ui| {
            let topic = quatrelle_engine::model::thread::base_subject(&subject);
            let searches = [
                ("Messages in this conversation", format!("subject:\"{}\"", topic.replace('"', ""))),
                ("Messages from sender", format!("from:{sender}")),
            ];
            for (label, query) in searches {
                if ui.button(tl!(label)).clicked() {
                    acts.push(Action::Search(query));
                    ui.close();
                }
            }
        });
        crate::organize::context_items(ui, s, t, acts, flags.pinned());
    });
    // Reply actions need the message open.
    if resp.secondary_clicked() && s.reading.as_ref().is_none_or(|r| r.key != m.key) {
        acts.push(Action::Open(m.key.clone()));
    }
}

fn range_select(s: &Session, to: &MessageKey, acts: &mut Vec<Action>) {
    let anchor = s.focus.clone().or_else(|| s.selection.first().cloned());
    let Some(anchor) = anchor else {
        acts.push(Action::Run("list.select", json!({"key": to.to_id()})));
        return;
    };
    let a = s.list.iter().position(|m| m.key == anchor);
    let b = s.list.iter().position(|m| &m.key == to);
    let (Some(a), Some(b)) = (a, b) else { return };
    let (lo, hi) = (a.min(b), a.max(b));
    for m in s.list.iter().take(hi + 1).skip(lo) {
        if !s.selection.contains(&m.key) {
            acts.push(Action::Run("list.select", json!({"key": m.key.to_id(), "add": true})));
        }
    }
}

#[cfg(test)]
mod column_tests {
    use super::*;

    /// Columns dragged keep their width; the subject takes the rest and keeps some; a damaged
    /// setting can't hide a column.
    #[test]
    fn dragged_columns_keep_their_width() {
        let width = |cols: &[(Col, f32, f32)], c: Col| cols.iter().find(|x| x.0 == c).map(|x| x.2 - x.1).unwrap_or(0.0);
        let plain = columns(900.0, &ColumnWidths::default());
        assert_eq!(width(&plain, Col::Received), 156.0);
        let dragged = ColumnWidths { who: Some(300.0), received: Some(100.0), ..Default::default() };
        let cols = columns(900.0, &dragged);
        assert_eq!(width(&cols, Col::Who), 300.0);
        assert_eq!(width(&cols, Col::Received), 100.0);
        assert!(width(&cols, Col::Subject) > width(&plain, Col::Subject) - 300.0);
        // Too wide a sender for the list: the subject keeps 80 points.
        let cols = columns(500.0, &ColumnWidths { who: Some(600.0), ..Default::default() });
        assert!(width(&cols, Col::Subject) >= 80.0);
        let bad = ColumnWidths { who: Some(-5.0), received: Some(f32::NAN), size: Some(1e9), categories: None }.clamped();
        assert_eq!((bad.who, bad.received, bad.size), (Some(60.0), None, Some(160.0)));
    }
}
