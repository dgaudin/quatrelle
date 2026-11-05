// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What drawing the message reads from the find bar: where the words were found, which one is
//! the current one, and the colours they're lit in; and text laid out with them lit.

use std::ops::Range;
use std::sync::Arc;

use egui::{Color32, Rect};
use quatrelle_engine::mime;

use super::doc::Doc;

/// A piece of a drawn text lit by the find bar: its bytes in that text, and whether it's (part
/// of) the current find.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Mark {
    pub(crate) range: Range<usize>,
    pub(crate) current: bool,
}

/// The finds a message is drawn with.
#[derive(Clone)]
pub(crate) struct Lit {
    pub(super) doc: Arc<Doc>,
    pub(super) hits: Arc<Vec<Range<usize>>>,
    pub(super) current: usize,
    /// The part of a long message to lay out whatever the view: the current find's, while it's
    /// being brought into view.
    pub(crate) target: Option<usize>,
    /// The finds' fill, the current one's, and the current one's text.
    pub(super) all: Color32,
    pub(super) now: Color32,
    pub(super) ink: Color32,
}

impl Lit {
    /// The finds in `range` of the text looked in, in bytes of that range.
    pub(crate) fn marks_in(&self, range: Range<usize>) -> Vec<Mark> {
        let first = self.hits.partition_point(|h| h.end <= range.start);
        let mut out = Vec::new();
        for (i, h) in self.hits.iter().enumerate().skip(first) {
            if h.start >= range.end {
                break;
            }
            let (s, e) = (h.start.max(range.start), h.end.min(range.end));
            if s < e {
                out.push(Mark { range: s - range.start..e - range.start, current: i == self.current });
            }
        }
        out
    }

    /// The finds in a formatted span's text (none for a span that isn't looked in).
    pub(crate) fn marks_of(&self, sp: &mime::Span) -> Vec<Mark> {
        match self.doc.span(sp) {
            Some((start, len)) => self.marks_in(start..start.saturating_add(len)),
            None => Vec::new(),
        }
    }

    /// The part of a long message the current find is in.
    pub(crate) fn current_part(&self) -> Option<usize> {
        self.hits.get(self.current).and_then(|h| self.doc.part_of(h.start))
    }

    /// `job` with `marks` lit (bytes of its text): the finds on their fill, in the text's own
    /// colour then, so they read on it whatever colour the message gave them.
    pub(crate) fn light(&self, job: &mut egui::text::LayoutJob, marks: &[Mark]) {
        if marks.is_empty() {
            return;
        }
        let mut out = Vec::with_capacity(job.sections.len() + 2 * marks.len());
        let range = |a: usize, b: usize| egui::text::ByteIndex(a)..egui::text::ByteIndex(b);
        for sec in std::mem::take(&mut job.sections) {
            let (start, end): (usize, usize) = (sec.byte_range.start.into(), sec.byte_range.end.into());
            let mut at = start;
            for m in marks.iter().filter(|m| m.range.start < end && m.range.end > start) {
                let (s, e) = (m.range.start.max(start), m.range.end.min(end));
                if s > at {
                    out.push(egui::text::LayoutSection { byte_range: range(at, s), ..sec.clone() });
                }
                let mut format = sec.format.clone();
                format.background = if m.current { self.now } else { self.all };
                if m.current {
                    format.color = self.ink;
                }
                out.push(egui::text::LayoutSection { byte_range: range(s, e), format, ..sec.clone() });
                at = e;
            }
            if at < end {
                out.push(egui::text::LayoutSection { byte_range: range(at, end), ..sec });
            }
        }
        job.sections = out;
    }

    /// `text` laid out as a label lays it out, with `marks` lit.
    pub(crate) fn lit_text(&self, ui: &egui::Ui, text: egui::RichText, marks: &[Mark]) -> egui::WidgetText {
        if marks.is_empty() {
            return text.into();
        }
        let job = egui::WidgetText::from(text).into_layout_job(ui.style(), egui::FontSelection::Default, ui.text_valign());
        let mut job = Arc::unwrap_or_clone(job);
        self.light(&mut job, marks);
        job.into()
    }
}

/// Where in a drawn text's `rect` the find starting at byte `at` of its `len` is: its whole rect
/// for a line or two, else a row's height at the place its share of the text puts it.
pub(crate) fn place(rect: Rect, at: usize, len: usize, row: f32) -> Rect {
    if rect.height() <= 2.0 * row || len == 0 {
        return rect;
    }
    let y = rect.top() + rect.height() * (at as f32 / len as f32);
    Rect::from_min_max(egui::pos2(rect.left(), y), egui::pos2(rect.right(), (y + row).min(rect.bottom())))
}
