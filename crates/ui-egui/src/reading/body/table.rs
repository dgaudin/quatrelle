//! Tables in a message: their width and place, their columns' widths, and their rows' cells side
//! by side (or one under the other when a layout's few columns can't fit). A table of figures is
//! laid out as a browser lays it out, its columns as wide as their content, and scrolls sideways
//! when it's wider than the pane, as Outlook shows it.

use super::*;

/// A layout's few columns squeezed narrower than this go one under the other.
pub(super) const MIN_COLUMN: f32 = 140.0;
/// Most columns a table is read with.
const MAX_COLUMNS: usize = 64;
/// The rows a table of figures is sized by (a report of thousands, by its first ones).
const SIZED_BY: usize = 60;

impl Body<'_> {
    /// A table: as wide as it asks (or as its content, for a box of text alone), placed as it
    /// says, its rows one under the other in its frame.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn table(
        &mut self,
        st: &mut UiState,
        ui: &mut Ui,
        table: &mime::layout::Table,
        width: f32,
        page: [u8; 3],
        align: mime::layout::Align,
        depth: usize,
    ) {
        use mime::layout::{Align as A, Width};
        let mut columns = column_widths(&table.rows);
        let asked = |w: Option<Width>| match w {
            Some(Width::Px(px)) => Some(px as f32 * self.zoom),
            Some(Width::Percent(pc)) => Some(width * f32::from(pc) / 100.0),
            None => None,
        };
        // A table of figures, sized as a browser sizes one: each column as wide as its content
        // on one line when they all fit (or the table asks for more: widened alike), else
        // between that and its longest word, its text wrapping; when even its words can't fit,
        // it scrolls sideways.
        let figures = figure_columns(ui, table, &columns, self.zoom);
        let tw = match &figures {
            Some(f) => {
                let (least, most) = f.iter().fold((0.0f32, 0.0f32), |(a, b), (lo, hi)| (a + lo, b + hi));
                let room = asked(table.width);
                let target = room.unwrap_or(width).max(1.0);
                let widths: Vec<f32> = if most <= target {
                    let grow = if room.is_some() { target / most.max(1.0) } else { 1.0 };
                    f.iter().map(|(_, hi)| hi * grow).collect()
                } else if least >= target {
                    f.iter().map(|(lo, _)| *lo).collect()
                } else {
                    let share = (target - least) / (most - least).max(1.0);
                    f.iter().map(|(lo, hi)| lo + (hi - lo) * share).collect()
                };
                columns = widths.iter().map(|w| Some(Width::Px(w.ceil() as u32))).collect();
                widths.iter().sum()
            }
            None => match table.width {
                Some(Width::Px(px)) => (px as f32 * self.zoom).min(width),
                Some(Width::Percent(pc)) => width * f32::from(pc) / 100.0,
                None => shrunk(ui, table, self.zoom).map_or(width, |w| w.min(width)),
            },
        }
        .max(1.0);
        if tw > width + 0.5 {
            egui::ScrollArea::horizontal().id_salt(("message-table", self.key.to_id(), self.cells)).auto_shrink([false, true]).show(ui, |ui| {
                self.table_box(st, ui, table, &columns, tw, 0.0, page, depth);
            });
            return;
        }
        // Where it sits: as it says, else as the box around it lines up its content
        // (a button's table in a centred cell, which browsers centre).
        let indent = match if table.align == A::Left { align } else { table.align } {
            A::Center => (width - tw) / 2.0,
            A::Right => width - tw,
            A::Left => 0.0,
        }
        .max(0.0);
        self.table_box(st, ui, table, &columns, tw, indent, page, depth);
    }

    /// A table `tw` wide, `indent` from the left, in its frame: its rows one under the other.
    #[allow(clippy::too_many_arguments)]
    fn table_box(
        &mut self,
        st: &mut UiState,
        ui: &mut Ui,
        table: &mime::layout::Table,
        columns: &[Option<mime::layout::Width>],
        tw: f32,
        indent: f32,
        page: [u8; 3],
        depth: usize,
    ) {
        let tbg = theme::box_color(table.background, page, &st.tokens);
        let page = tbg.unwrap_or(page);
        let behind = table.background_image.as_deref().and_then(|src| self.background(st, ui.ctx(), src));
        // Rows meet when its cells draw borders (a receipt's grid); otherwise a
        // little air between them.
        let bordered = table.rows.iter().flatten().any(|c| !c.borders.is_none());
        ui.horizontal_top(|ui| {
            ui.add_space(indent);
            ui.allocate_ui_with_layout(vec2(tw, 0.0), egui::Layout::top_down(Align::Min), |ui| {
                ui.set_width(tw);
                let fill = tbg.map_or(Color32::TRANSPARENT, |c| Color32::from_rgb(c[0], c[1], c[2]));
                let (frame, round) = framed_box(fill, &table.borders, table.radius, page, &st.tokens);
                let framed = frame.show(ui, |ui| {
                    // Its place in the drawing: above the fill, under the content.
                    let slot = ui.painter().add(egui::Shape::Noop);
                    ui.spacing_mut().item_spacing = vec2(0.0, if bordered { 0.0 } else { 4.0 });
                    for row in &table.rows {
                        self.row(st, ui, row, columns, tw, page, depth);
                    }
                    slot
                });
                if let Some(tex) = &behind {
                    ui.painter().set(framed.inner, cover(tex, framed.response.rect));
                }
                // Its borders (a signature's rule down its left edge, a grid's outer line).
                if !round {
                    edges(ui, framed.response.rect, &table.borders, page, &st.tokens);
                }
            });
        });
    }

    /// A table row: its cells side by side, or one under the other when a layout's two or three
    /// are squeezed narrower than [`MIN_COLUMN`] (a table of figures stays a table).
    /// `columns`: the widths the table's columns ask for (a width set in one row is the whole
    /// column's, as HTML sizes columns). Each cell's content sits at the top, the middle or the
    /// bottom of the row as the cell asks ([`mime::layout::VAlign`]): the row is as tall as its
    /// tallest cell as measured the frame before ([`CellHeights`]), and the others are moved
    /// down in it (their backgrounds and borders span the row).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn row(
        &mut self,
        st: &mut UiState,
        ui: &mut Ui,
        row: &[mime::layout::Cell],
        columns: &[Option<mime::layout::Width>],
        width: f32,
        page: [u8; 3],
        depth: usize,
    ) {
        use mime::layout::{VAlign, Width};
        let px = |w: Width| match w {
            Width::Px(px) => px as f32 * self.zoom,
            Width::Percent(pc) => width * f32::from(pc) / 100.0,
        };
        let pads: Vec<Margin> = row.iter().map(padding_of).collect();
        // Widths: what cells (or their columns) ask for, the rest shared by the others.
        let mut col = 0usize;
        let asked: Vec<Option<f32>> = row
            .iter()
            .zip(&pads)
            .map(|(c, pad)| {
                let span = usize::from(c.span.max(1));
                let from_columns = || {
                    let parts = columns.get(col..col + span)?;
                    (parts.len() == span && parts.iter().all(Option::is_some)).then(|| parts.iter().flatten().map(|w| px(*w)).sum::<f32>())
                };
                // A floated picture's box (a signature's logo): as wide as the picture, padding added.
                let w = c.width.map(px).or_else(from_columns).or_else(|| c.fit.map(|f| f as f32 * self.zoom + pad.sum().x));
                col += span;
                w
            })
            .collect();
        let fixed: f32 = asked.iter().flatten().sum();
        let free = asked.iter().filter(|a| a.is_none()).count();
        let mut widths: Vec<f32> = if fixed > width || (free > 0 && width - fixed < MIN_COLUMN * free as f32) {
            // Asked for more than there is: shared out in proportion.
            let total: f32 = asked.iter().map(|a| a.unwrap_or(MIN_COLUMN)).sum::<f32>().max(1.0);
            asked.iter().map(|a| a.unwrap_or(MIN_COLUMN) / total * width).collect()
        } else {
            let each = if free > 0 { (width - fixed) / free as f32 } else { 0.0 };
            asked.iter().map(|a| a.unwrap_or(each)).collect()
        };
        // Cells that ask for nothing extra (spacers) next to wider ones.
        let used: f32 = widths.iter().sum();
        if used < width
            && let Some(last) = widths.last_mut()
        {
            *last += width - used;
        }
        // A column the message asks narrow (a signature's logo) isn't squeezed: only one given
        // less than it needs is.
        let squeezed = fixed > width || (free > 0 && width - fixed < MIN_COLUMN * free as f32);
        let stack = (2..=3).contains(&row.len()) && squeezed && widths.iter().zip(row).any(|(w, c)| *w < MIN_COLUMN && !c.blocks.is_empty());
        // The cells' numbers (theirs before any table inside them), and their heights as last
        // drawn; the row's height when every one is known and some cell sits below the top.
        let first = self.cells;
        self.cells = self.cells.saturating_add(row.len());
        let heights: Vec<Option<f32>> = (0..row.len()).map(|i| st.cell_heights.get(self.key, first.saturating_add(i))).collect();
        let aligned = !stack && row.len() > 1 && row.iter().any(|c| c.valign != VAlign::Top && !c.blocks.is_empty());
        let row_h = (aligned && heights.iter().all(Option::is_some)).then(|| heights.iter().flatten().fold(0.0f32, |a, h| a.max(*h)));
        if aligned && row_h.is_none() && !ui.is_sizing_pass() {
            // Measured this frame: drawn aligned on the next, right away.
            ui.ctx().request_repaint();
        }
        let cell = |this: &mut Self, st: &mut UiState, ui: &mut Ui, c: &mime::layout::Cell, w: f32, pad: Margin, n: usize, h: Option<f32>| {
            let bg = theme::box_color(c.background, page, &st.tokens);
            let fill = bg.map_or(Color32::TRANSPARENT, |b| Color32::from_rgb(b[0], b[1], b[2]));
            let behind = c.background_image.as_deref().and_then(|src| this.background(st, ui.ctx(), src));
            // How far down the row its content goes.
            let offset = match (c.valign, h, row_h) {
                (VAlign::Middle, Some(h), Some(rh)) => ((rh - h) / 2.0).max(0.0),
                (VAlign::Bottom, Some(h), Some(rh)) => (rh - h).max(0.0),
                _ => 0.0,
            };
            ui.allocate_ui_with_layout(vec2(w, 0.0), egui::Layout::top_down(Align::Min), |ui| {
                ui.set_width(w);
                let (frame, round) = framed_box(fill, &c.borders, c.radius, page, &st.tokens);
                let framed = frame.inner_margin(pad).show(ui, |ui| {
                    let slot = ui.painter().add(egui::Shape::Noop);
                    let inner = (w - pad.sum().x).max(1.0);
                    ui.set_width(inner);
                    // The cell starts here even when its content is pushed down, and is at
                    // least as tall as its row.
                    let start = ui.cursor().top();
                    ui.expand_to_include_y(start);
                    if let Some(rh) = row_h {
                        ui.set_min_height((rh - pad.sum().y).max(0.0));
                    }
                    if offset > 0.0 {
                        ui.add_space(offset);
                    }
                    let before = ui.cursor().top();
                    this.blocks(st, ui, &c.blocks, inner, bg.unwrap_or(page), c.align, depth + 1);
                    // Its content's height (the cursor rests a spacing below the last piece).
                    let moved = ui.cursor().top() - before;
                    let content = if moved > 0.0 { (moved - ui.spacing().item_spacing.y).max(0.0) } else { 0.0 };
                    (slot, content + pad.sum().y)
                });
                let (slot, natural) = framed.inner;
                if !ui.is_sizing_pass() {
                    st.cell_heights.set(this.key, n, natural);
                }
                let rect = framed.response.rect;
                if let Some(tex) = &behind {
                    ui.painter().set(slot, cover(tex, rect));
                }
                if !round {
                    edges(ui, rect, &c.borders, page, &st.tokens);
                }
                // Boxes placed over it; the cell grows to hold any that reach past its end.
                let mut bottom = rect.bottom();
                for o in &c.overlays {
                    bottom = bottom.max(this.overlay(st, ui, o, rect, bg.unwrap_or(page), depth));
                }
                if bottom > rect.bottom() {
                    ui.add_space(bottom - rect.bottom());
                }
            });
        };
        if stack {
            for (i, c) in row.iter().enumerate().filter(|(_, c)| !c.blocks.is_empty()) {
                cell(self, st, ui, c, width, pads.get(i).copied().unwrap_or_default(), first.saturating_add(i), None);
            }
        } else {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (i, (c, w)) in row.iter().zip(&widths).enumerate() {
                    let h = heights.get(i).copied().flatten();
                    cell(self, st, ui, c, *w, pads.get(i).copied().unwrap_or_default(), first.saturating_add(i), h);
                }
            });
        }
    }
}

/// The reader's own padding in a table cell, when the message sets none.
const PAD: i8 = 6;

/// A table without a width that is one box of text (a newsletter's button, a note in a frame,
/// a signature's name beside its rule): as wide as its text, padding added, as a browser
/// shrinks a table to fit its content; `None` for any other (it takes the whole width).
fn shrunk(ui: &Ui, table: &mime::layout::Table, zoom: f32) -> Option<f32> {
    let [row] = table.rows.as_slice() else { return None };
    let [cell] = row.as_slice() else { return None };
    let boxed = cell.background.is_some()
        || !cell.borders.is_none()
        || cell.radius > 0
        || table.background.is_some()
        || !table.borders.is_none()
        || table.radius > 0;
    if !boxed || cell.width.is_some() || cell.background_image.is_some() || table.background_image.is_some() || !cell.overlays.is_empty() {
        return None;
    }
    Some(natural_width(ui, &cell.blocks, zoom)? + padding_of(cell).sum().x + 2.0)
}

/// A cell's padding: the message's when it sets one (`padding`, `cellpadding`; the layout caps
/// it), else the reader's.
fn padding_of(c: &mime::layout::Cell) -> Margin {
    let side = |v: u8| i8::try_from(v).unwrap_or(i8::MAX);
    match c.padding {
        Some([top, right, bottom, left]) => Margin { left: side(left), right: side(right), top: side(top), bottom: side(bottom) },
        None => Margin::same(PAD),
    }
}

/// How wide formatted text is unwrapped (its widest line) at `zoom`; `None` for anything but
/// text.
pub(super) fn natural_width(ui: &Ui, blocks: &[mime::layout::Block], zoom: f32) -> Option<f32> {
    let mut widest: f32 = 0.0;
    let faces = crate::look::typefaces::loaded(ui.ctx());
    for b in blocks {
        let mime::layout::Block::Text(lines) = b else { return None };
        for line in lines {
            if line.iter().any(|sp| sp.image.is_some()) {
                return None;
            }
            let w: f32 = line
                .iter()
                .map(|sp| ui.fonts_mut(|f| f.layout_no_wrap(sp.text.clone(), span_font_in(sp, &faces, zoom), Color32::WHITE).size().x))
                .sum();
            widest = widest.max(w);
        }
    }
    Some(widest)
}

/// The width each column of a table asks for: the first cell of one column that sets one, in any
/// row (HTML sizes a whole column so; only the first row set it in a report whose other rows then
/// lined up elsewhere).
pub(super) fn column_widths(rows: &[Vec<mime::layout::Cell>]) -> Vec<Option<mime::layout::Width>> {
    let mut out: Vec<Option<mime::layout::Width>> = Vec::new();
    for row in rows {
        let mut col = 0usize;
        for c in row {
            let span = usize::from(c.span.max(1));
            if span == 1 && col < MAX_COLUMNS {
                if out.len() <= col {
                    out.resize(col + 1, None);
                }
                if let (Some(slot), Some(w)) = (out.get_mut(col), c.width)
                    && slot.is_none()
                {
                    *slot = Some(w);
                }
            }
            col = col.saturating_add(span);
        }
    }
    out
}

/// A table of figures — four columns or more, text in every cell — sized as a browser sizes one:
/// each column's least width (its longest word) and its width on one line (its widest content
/// unwrapped), padding added; the width it asks for is both. `None` for any other table (a
/// layout's), whose columns share its width as before.
fn figure_columns(ui: &Ui, table: &mime::layout::Table, asked: &[Option<mime::layout::Width>], zoom: f32) -> Option<Vec<(f32, f32)>> {
    use mime::layout::Width;
    let columns = table.rows.iter().map(|r| r.iter().map(|c| usize::from(c.span.max(1))).sum::<usize>()).max()?;
    if !(4..=MAX_COLUMNS).contains(&columns) {
        return None;
    }
    let mut widths = vec![(0.0f32, 0.0f32); columns];
    for row in table.rows.iter().take(SIZED_BY) {
        let mut col = 0usize;
        for c in row {
            if c.background_image.is_some() || !c.overlays.is_empty() {
                return None;
            }
            let span = usize::from(c.span.max(1));
            if span == 1 && !c.blocks.is_empty() {
                // Padding, and a few pixels to spare: a word short of one is cut in two.
                let pad = padding_of(c).sum().x + 6.0;
                let (lo, hi) = (longest_word(ui, &c.blocks, zoom)? + pad, natural_width(ui, &c.blocks, zoom)? + pad);
                if let Some((a, b)) = widths.get_mut(col) {
                    (*a, *b) = (a.max(lo), b.max(hi));
                }
            }
            col = col.saturating_add(span);
        }
    }
    for (i, w) in widths.iter_mut().enumerate() {
        match asked.get(i).copied().flatten() {
            Some(Width::Px(px)) => *w = (px as f32 * zoom, px as f32 * zoom),
            Some(Width::Percent(_)) => return None,
            None => {
                let least = f32::from(PAD) * 2.0;
                *w = (w.0.max(least), w.1.max(least));
            }
        }
    }
    Some(widths)
}

/// How wide formatted text's longest word is (what it can't be narrower than, wrapped); `None`
/// for anything but text.
fn longest_word(ui: &Ui, blocks: &[mime::layout::Block], zoom: f32) -> Option<f32> {
    let faces = crate::look::typefaces::loaded(ui.ctx());
    let mut widest: f32 = 0.0;
    for b in blocks {
        let mime::layout::Block::Text(lines) = b else { return None };
        for sp in lines.iter().flatten() {
            // The longest few by letters, measured (`CPU` is wider than `(%)`).
            let mut words: Vec<&str> = sp.text.split_whitespace().collect();
            words.sort_by_key(|w| std::cmp::Reverse(w.chars().count()));
            for word in words.into_iter().take(3) {
                let w = ui.fonts_mut(|f| f.layout_no_wrap(word.to_string(), span_font_in(sp, &faces, zoom), Color32::WHITE).size().x);
                widest = widest.max(w);
            }
        }
    }
    Some(widest)
}
