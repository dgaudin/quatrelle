// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A centred (or right-aligned) line too long for a row, with a link in it (a newsletter's
//! footer): cut where egui wraps it laid out whole, each row's pieces of its spans drawn side by
//! side and moved over, as a browser moves each row of a centred paragraph. Each piece is a run of
//! its own, so a link stays a link (a piece of it on each row it's on), the line is selected and
//! copied as one, and the finds in a span are lit in its pieces.

use super::*;

/// A piece of one of a line's spans, on one row: that span with the row's words of it, which of
/// the line's spans it is, and the byte its words start at in that span's text.
pub(super) struct Piece {
    span: mime::Span,
    of: usize,
    from: usize,
}

impl Body<'_> {
    /// `line` cut at the rows it wraps to, `width` wide, laid out whole.
    pub(super) fn rows_of(&self, t: &Tokens, ui: &Ui, line: &[mime::Span], width: f32, page: [u8; 3]) -> Vec<Vec<Piece>> {
        let (mut job, _) = self.line_job(t, line, page);
        job.wrap.max_width = width.max(1.0);
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        // Where each row ends, in characters of the line.
        let mut ends = Vec::with_capacity(galley.rows.len());
        let mut at = 0usize;
        for row in &galley.rows {
            at = at.saturating_add(row.char_count_including_newline().0);
            ends.push(at);
        }
        let last = ends.len().saturating_sub(1);
        let mut rows: Vec<Vec<Piece>> = (0..ends.len().max(1)).map(|_| Vec::new()).collect();
        let push = |rows: &mut Vec<Vec<Piece>>, row: usize, of: usize, sp: &mime::Span, from: usize, to: usize| {
            if let (Some(words), Some(r)) = (sp.text.get(from..to), rows.get_mut(row)) {
                r.push(Piece { span: mime::Span { text: words.to_string(), ..sp.clone() }, of, from });
            }
        };
        // The line's character each span starts at.
        let mut start = 0usize;
        for (of, sp) in line.iter().enumerate() {
            // The piece being gathered: its row, and its first and past-the-last bytes.
            let mut piece: Option<(usize, usize, usize)> = None;
            for (n, (b, c)) in sp.text.char_indices().enumerate() {
                let row = ends.partition_point(|e| *e <= start.saturating_add(n)).min(last);
                let end = b + c.len_utf8();
                piece = match piece {
                    Some((r, from, _)) if r == row => Some((r, from, end)),
                    Some((r, from, to)) => {
                        push(&mut rows, r, of, sp, from, to);
                        Some((row, b, end))
                    }
                    None => Some((row, b, end)),
                };
            }
            if let Some((r, from, to)) = piece {
                push(&mut rows, r, of, sp, from, to);
            }
            start = start.saturating_add(sp.text.chars().count());
        }
        rows
    }

    /// A row of [`Body::rows_of`]: its pieces side by side, moved over as `align` says (the space
    /// that ends the row not counted).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn row_of_pieces(
        &mut self,
        st: &mut UiState,
        ui: &mut Ui,
        row: &[Piece],
        line: &[mime::Span],
        width: f32,
        page: [u8; 3],
        align: Align,
    ) {
        let last = row.len().saturating_sub(1);
        let row_w: f32 = row
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let words = if i == last { p.span.text.trim_end() } else { p.span.text.as_str() };
                ui.fonts_mut(|f| f.layout_no_wrap(words.to_string(), span_font_in(&p.span, &self.faces, self.zoom), Color32::WHITE).size().x)
            })
            .sum();
        let space = if align == Align::Center { (width - row_w) / 2.0 } else { width - row_w };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            // Each piece as wide as its words: the row was cut to fit.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            ui.add_space(space.max(0.0));
            for p in row {
                self.span_in(st, ui, &p.span, line.get(p.of).map(|o| (o, p.from)), width, page);
            }
        });
    }
}
