//! The message list as a table, as Outlook's: its columns (dragged wider or narrower, kept
//! with the settings), their headings that sort it, and its rows.

use super::*;

/// The table's headings, and its date headings.
pub(super) const TABLE_HEADER_H: f32 = 24.0;
pub(super) const TABLE_HEADING_H: f32 = 24.0;

/// A table row: one line, or two with the preview under the sender and subject.
pub(super) fn table_row_height(s: &Session) -> f32 {
    if s.prefs.show_preview && !s.prefs.compact { 40.0 } else { 26.0 }
}

/// The table's columns, as Outlook's: importance, attachment, sender (recipients in Sent),
/// subject, received, size, categories, flag.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Col {
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
pub(super) fn columns(width: f32, w: &ColumnWidths) -> Vec<(Col, f32, f32)> {
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
pub(super) fn column_arrange(c: Col, sent_view: bool) -> Option<quatrelle_engine::Arrange> {
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
pub(super) fn table_header(
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
    bottom_line(&p, r, 0.0, t.border);
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
pub(super) fn table_row(
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
    row_line(&p, t, r, 0.0);
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
                let color = if unread { t.accent } else { t.text };
                if s.has_draft_reply(m) {
                    // A reply waiting in Drafts: `[Draft]` before the subject.
                    let inner = cell.shrink2(vec2(6.0, 0.0));
                    let g = draft_galley(&p, t, 12.5);
                    let rest = Rect::from_x_y_ranges(inner.min.x + g.size().x + 5.0..=inner.max.x, inner.y_range());
                    p.with_clip_rect(inner).galley(egui::pos2(inner.min.x, line1 - g.size().y / 2.0), g, t.danger);
                    let (text, _) = crate::bidi::shown(&subject);
                    let g = p.layout_no_wrap(text.into_owned(), font(13.0), color);
                    p.with_clip_rect(rest).galley(egui::pos2(rest.min.x, line1 - g.size().y / 2.0), g, color);
                } else {
                    text_cell(&p, subject, color, 13.0);
                }
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
