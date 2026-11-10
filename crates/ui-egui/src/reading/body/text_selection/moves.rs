// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Moving a selection's end: the run and letter nearest a place on the screen, a word or a line
//! taken whole (a double- or triple-click), and the text's keys (Ctrl+A, Shift with the arrows,
//! Home and End, Escape, a copy), which go on across runs.

use super::*;

impl Doc {
    /// The run, and the letter in it, nearest `p` among `among`, of the rows `keep` keeps: the
    /// row nearest up or down, then across.
    pub(super) fn nearest(&self, among: Range<usize>, p: Pos2, keep: impl Fn(Rect) -> bool) -> Option<(usize, CCursor)> {
        let gap = |v: f32, r: egui::Rangef| if v < r.min { r.min - v } else { (v - r.max).max(0.0) };
        let mut best: Option<((f32, f32), usize, Rect)> = None;
        for i in among {
            let Some(r) = self.runs.get(i) else { continue };
            for row in r.rows().filter(|row| keep(*row)) {
                let score = (gap(p.y, row.y_range()), gap(p.x, row.x_range()));
                if best.is_none_or(|(b, ..)| score < b) {
                    best = Some((score, i, row));
                }
            }
        }
        let (_, i, row) = best?;
        let r = self.runs.get(i)?;
        let at = pos2(p.x.clamp(row.left() - 1.0, row.right() + 1.0), row.center().y);
        Some((i, r.galley.cursor_from_pos(at - r.pos)))
    }

    /// Where `m` is among the runs laid out: its own, or the first after it (a mark past them all
    /// at the end of the last).
    fn resolve(&self, m: Mark) -> Option<(usize, CCursor)> {
        let last = self.runs.len().checked_sub(1)?;
        for (i, r) in self.runs.iter().enumerate() {
            let k = (r.part, r.n);
            if k == m.run_key() {
                let at = if m.at.index.0 > r.len() { r.galley.end() } else { m.at };
                return Some((i, at));
            }
            if k > m.run_key() {
                return Some((i, CCursor::default()));
            }
        }
        self.runs.get(last).map(|r| (last, r.galley.end()))
    }

    /// The runs on the line of run `i` (line breaks apart).
    fn line_of(&self, i: usize) -> Range<usize> {
        let starts = |j: usize| self.runs.get(j).is_none_or(|r| r.breaks > 0 || j == 0);
        let mut s = i;
        while !starts(s) {
            s -= 1;
        }
        let mut e = i.saturating_add(1);
        while e < self.runs.len() && !starts(e) {
            e += 1;
        }
        s..e
    }

    /// A double-click: the word at `at` in run `i`; a triple-click: its line (a paragraph).
    pub(super) fn select_unit(&mut self, i: usize, at: CCursor, line: bool) {
        let Some(r) = self.runs.get(i) else { return };
        let chars: Vec<char> = r.galley.text().chars().collect();
        let at = at.index.0.min(chars.len());
        let (from, to) = if line {
            let in_run = |c: &char| *c == '\n';
            let s = chars.get(..at).and_then(|b| b.iter().rposition(in_run)).map(|p| (i, p + 1));
            let e = chars.get(at..).and_then(|b| b.iter().position(in_run)).map(|p| (i, at + p));
            let l = self.line_of(i);
            let first = l.start;
            let last = l.end.saturating_sub(1);
            (s.unwrap_or((first, 0)), e.unwrap_or_else(|| (last, self.runs.get(last).map_or(0, Run::len))))
        } else {
            let word = |c: &char| is_word_char(*c);
            let here = chars.get(at).filter(|c| word(c)).or_else(|| at.checked_sub(1).and_then(|p| chars.get(p)).filter(|c| word(c)));
            if here.is_none() {
                return;
            }
            let s = chars.get(..at).and_then(|b| b.iter().rposition(|c| !word(c))).map_or(0, |p| p + 1);
            let e = chars.get(at..).and_then(|b| b.iter().position(|c| !word(c))).map_or(chars.len(), |p| at + p);
            ((i, s), (i, e))
        };
        self.anchor = self.mark(from.0, CCursor::new(from.1));
        self.head = self.mark(to.0, CCursor::new(to.1));
    }

    /// Shift with an arrow, Home or End (Ctrl too): the selection's moving end goes on, across
    /// runs. Where it went, to keep in view.
    fn step(&mut self, os: egui::os::OperatingSystem, key: Key, m: Modifiers, text: Rect) -> Option<Rect> {
        let last = self.runs.len().checked_sub(1)?;
        if self.head.is_none() {
            self.anchor = self.mark(0, CCursor::default());
            self.head = self.anchor;
        }
        if m.command && matches!(key, Key::Home | Key::End) {
            self.h_x = None;
            let home = key == Key::Home;
            self.head = Some(if home { Mark::START } else { Mark::END });
            return Some(if home {
                Rect::from_min_size(text.min, vec2(1.0, 20.0))
            } else {
                Rect::from_min_size(text.left_bottom() - vec2(0.0, 20.0), vec2(1.0, 20.0))
            });
        }
        let (mut i, c) = self.resolve(self.head?)?;
        let run = self.runs.get(i)?;
        let caret = run.caret(c);
        let mut to;
        match key {
            Key::ArrowUp | Key::ArrowDown => {
                // By where it is on the screen: rows of this run and of others alike.
                let x = *self.h_x.get_or_insert(caret.center().x);
                let found = if key == Key::ArrowDown {
                    self.nearest(0..self.runs.len(), pos2(x, caret.bottom()), |row| row.center().y > caret.bottom())
                } else {
                    self.nearest(0..self.runs.len(), pos2(x, caret.top()), |row| row.center().y < caret.top())
                };
                (i, to) = found.unwrap_or_else(|| {
                    if key == Key::ArrowDown {
                        (last, self.runs.get(last).map_or_else(CCursor::default, |r| r.galley.end()))
                    } else {
                        (0, CCursor::default())
                    }
                });
            }
            // To the start or end of the line on the screen, whatever runs it's made of (a bold
            // word is one of its own).
            Key::Home | Key::End => {
                self.h_x = None;
                (i, to) = self.across(os, i, c, key, m).unwrap_or((i, c));
            }
            _ => {
                self.h_x = None;
                let mut r = CCursorRange::one(c);
                r.on_key_press(os, &run.galley, &m, key);
                to = r.primary;
                if to.index == c.index {
                    (i, to) = self.across(os, i, c, key, m).unwrap_or((i, c));
                }
            }
        }
        self.head = self.mark(i, to);
        self.runs.get(i).map(|r| r.caret(to))
    }

    /// A key that didn't move within run `i` (at its edge) goes on into the next run or the one
    /// before, or along the line for Home and End.
    fn across(&self, os: egui::os::OperatingSystem, i: usize, c: CCursor, key: Key, m: Modifiers) -> Option<(usize, CCursor)> {
        let again = |j: usize, at: CCursor| {
            let r = self.runs.get(j)?;
            let mut range = CCursorRange::one(at);
            range.on_key_press(os, &r.galley, &m, key);
            Some((j, range.primary))
        };
        // Left and Right go on the screen: in a right-to-left paragraph, Left goes on in the text.
        let rtl = self.runs.get(i)?.galley.rows.first().is_some_and(|r| r.row.rtl_paragraph);
        match key {
            Key::ArrowLeft | Key::ArrowRight if (key == Key::ArrowLeft) != rtl => {
                let j = i.checked_sub(1)?;
                let end = self.runs.get(j)?.galley.end();
                // On the same line, the step is a letter (or word) of that run; else the line break.
                if self.runs.get(i)?.breaks == 0 { again(j, end) } else { Some((j, end)) }
            }
            Key::ArrowLeft | Key::ArrowRight => {
                let j = i.checked_add(1).filter(|j| *j < self.runs.len())?;
                if self.runs.get(j)?.breaks == 0 { again(j, CCursor::default()) } else { Some((j, CCursor::default())) }
            }
            Key::Home | Key::End => {
                let y = self.runs.get(i)?.caret(c).center().y;
                let line = self.line_of(i);
                let on_row = |row: Rect| row.y_range().contains(y);
                let x = if key == Key::Home { f32::NEG_INFINITY } else { f32::INFINITY };
                let rows = line.clone().flat_map(|j| self.runs.get(j).into_iter().flat_map(|r| r.rows()));
                let edge = rows.filter(|r| on_row(*r)).map(|r| if key == Key::Home { r.left() } else { r.right() });
                let edge = if key == Key::Home { edge.fold(f32::INFINITY, f32::min) } else { edge.fold(f32::NEG_INFINITY, f32::max) };
                let p = pos2(if edge.is_finite() { edge } else { x }, y);
                self.nearest(line, p, on_row)
            }
            _ => None,
        }
    }
}

/// The text's keys: Ctrl+A, Shift with the arrows, Home and End, Escape, and a copy.
pub(super) fn keys(d: &mut Doc, ui: &mut Ui, rect: Rect, whole: Option<&str>) {
    let os = ui.ctx().os();
    let events = ui.input(|i| i.events.clone());
    let mut used = false;
    let mut show = None;
    for e in &events {
        match e {
            egui::Event::Key { key: Key::A, pressed: true, modifiers, .. } if modifiers.command && !modifiers.shift => {
                d.anchor = Some(Mark::START);
                d.head = Some(Mark::END);
                used = true;
            }
            egui::Event::Key { key, pressed: true, modifiers, .. }
                if modifiers.shift && matches!(key, Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown | Key::Home | Key::End) =>
            {
                show = d.step(os, *key, *modifiers, rect).or(show);
                used = true;
            }
            egui::Event::Key { key: Key::Escape, pressed: true, .. } => {
                d.anchor = None;
                d.head = None;
            }
            egui::Event::Copy | egui::Event::Cut if d.selection().is_some() => {
                ui.ctx().copy_text(d.text(whole));
                used = true;
            }
            _ => {}
        }
    }
    if used {
        ui.input_mut(|i| {
            i.events.retain(|e| match e {
                egui::Event::Key { key: Key::A, modifiers, .. } => !modifiers.command,
                egui::Event::Key { key, modifiers, .. } => {
                    !(modifiers.shift && matches!(key, Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown | Key::Home | Key::End))
                }
                egui::Event::Copy | egui::Event::Cut => false,
                _ => true,
            })
        });
    }
    if let Some(r) = show {
        ui.scroll_to_rect(r.expand(8.0), None);
    }
}
