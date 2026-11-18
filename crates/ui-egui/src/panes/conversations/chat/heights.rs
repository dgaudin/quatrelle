// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Only the open chat's messages in view are laid out (security review 1.5, content n° 2): a
//! window holds up to 500 messages of up to `MAX_BLOCKS` blocks each, far more than a frame can
//! lay out. Each message's height is kept, by its id, from the last frame it was laid out; one out
//! of view takes that place without being laid out. A message whose height isn't known yet (new
//! to the window, or the pane's width changed) is laid out out of sight, at most
//! [`MEASURE_BLOCKS`] blocks a frame; past them it takes a guess's place ([`guess`]) until a later
//! frame lays it out, or it comes into view.
//!
//! The scroll keeps its ways (`chat.rs`): a height that became known above the view moves the
//! scroll by as much, so what's on screen stays where it is (the frame is laid out again before
//! it's drawn); a message gone to is scrolled to again until the heights around it are known.
//!
//! What is hostile here: the messages' size, the sender's. What it protects: a frame lays out the
//! messages in view and at most [`MEASURE_BLOCKS`] blocks more, whatever the window holds.

use std::collections::{HashMap, HashSet};

use egui::{Rect, Ui, pos2, vec2};
use quatrelle_engine::model::pim::teams::{ChatMessage, MAX_BLOCKS, WINDOW_MAX};

/// The most blocks laid out a frame for messages out of view whose height isn't known yet.
const MEASURE_BLOCKS: usize = 2_000;
/// The most frames a message gone to is scrolled to again while the heights around it settle.
const SETTLE_FRAMES: u8 = 4;
/// A height that moved by less than this (in points) hasn't changed.
const SAME: f32 = 0.5;

/// A message's height, for the way it's drawn (`first` of a row of one sender's, or not):
/// measured, or guessed until it is.
#[derive(Clone, Copy, Debug)]
struct Height {
    first: bool,
    h: f32,
    measured: bool,
}

/// What a message is this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Plan {
    /// In view: laid out and drawn.
    Draw,
    /// Out of view, its height not known yet: laid out out of sight.
    Measure,
    /// Out of view: its place, not laid out.
    Skip(f32),
}

/// The open chat's messages' heights (`ConversationsUi::heights`), and what the frame being laid
/// out learnt of them.
#[derive(Clone, Debug, Default)]
pub struct Heights {
    /// The width they were laid out at.
    width: f32,
    by_id: HashMap<String, Height>,
    /// Blocks left this frame for messages out of view.
    budget: usize,
    /// Blocks laid out this frame, all its passes (the last frame's, once it's over).
    pub(crate) laid_out: usize,
    /// How much messages above the view grew this frame, their heights becoming known.
    above: f32,
    /// A height laid out this frame wasn't the one assumed.
    changed: bool,
    /// A message out of view still takes a guess's place.
    guessed: bool,
    /// The frames the message gone to has been scrolled to.
    settling: u8,
}

impl Heights {
    /// A frame's start, the messages `width` wide: another width, and every height is to be
    /// measured again. Messages out of view are measured only when `measure` (the frame's first
    /// pass: a pass laid out again doesn't learn more, which would move what's on screen again).
    pub(super) fn begin(&mut self, width: f32, measure: bool) {
        if (width - self.width).abs() > SAME {
            self.width = width;
            self.by_id.clear();
        }
        self.budget = if measure { MEASURE_BLOCKS } else { 0 };
        // Over the frame's passes: a pass laid out again counts too.
        if measure {
            self.laid_out = 0;
        }
        self.above = 0.0;
        self.changed = false;
        self.guessed = false;
    }

    /// What `m` (a row's `first` or not) is this frame, its top at the cursor: laid out when it's
    /// in view or `forced` (the message gone to), else its place, measured out of sight first
    /// while this frame's budget lasts.
    pub(super) fn plan(&mut self, ui: &Ui, m: &ChatMessage, first: bool, forced: bool) -> Plan {
        let known = self.by_id.get(&m.id).filter(|k| k.first == first).copied();
        let h = known.map_or_else(|| guess(m, first, self.width), |k| k.h);
        let place = Rect::from_min_size(pos2(ui.max_rect().left(), ui.cursor().top()), vec2(ui.max_rect().width(), h.max(1.0)));
        if forced || ui.is_rect_visible(place) {
            return Plan::Draw;
        }
        if known.is_some_and(|k| k.measured) {
            return Plan::Skip(h);
        }
        if self.budget > 0 {
            self.budget = self.budget.saturating_sub(blocks(m));
            return Plan::Measure;
        }
        if known.is_none() {
            self.by_id.insert(m.id.clone(), Height { first, h, measured: false });
        }
        self.guessed = true;
        Plan::Skip(h)
    }

    /// `m` laid out from `top`, `h` tall, the view's top at `view_top`: its height kept; one that
    /// changed above the view moves the scroll by as much.
    pub(super) fn laid(&mut self, m: &ChatMessage, first: bool, top: f32, h: f32, view_top: f32) {
        self.laid_out = self.laid_out.saturating_add(blocks(m));
        let was = self.by_id.insert(m.id.clone(), Height { first, h, measured: true }).filter(|w| w.first == first);
        match was {
            Some(w) if (w.h - h).abs() <= SAME => {}
            Some(w) => {
                self.changed = true;
                if top < view_top {
                    self.above += h - w.h;
                }
            }
            None => self.changed = true,
        }
    }

    /// A frame's end: the heights of messages no longer in the `window` let go once they're many.
    pub(super) fn end<'a>(&mut self, window: impl Iterator<Item = &'a ChatMessage>) {
        if self.by_id.len() > 2 * WINDOW_MAX {
            let kept: HashSet<&str> = window.map(|m| m.id.as_str()).collect();
            self.by_id.retain(|id, _| kept.contains(id.as_str()));
        }
    }

    /// How much messages above the view grew this frame (points).
    pub(super) fn above(&self) -> f32 {
        if self.above.abs() > SAME { self.above } else { 0.0 }
    }

    /// Whether a message out of view still takes a guess's place: a later frame measures it.
    pub(super) fn guessed(&self) -> bool {
        self.guessed
    }

    /// No message is being gone to: the next one gone to settles from the start.
    pub(super) fn idle(&mut self) {
        self.settling = 0;
    }

    /// Whether the message gone to, scrolled to this frame, may be left where it is: no height
    /// around it changed (or it has been scrolled to long enough).
    pub(super) fn settled(&mut self) -> bool {
        if !self.changed || self.settling >= SETTLE_FRAMES {
            self.settling = 0;
            true
        } else {
            self.settling += 1;
            false
        }
    }
}

/// The blocks laying `m` out costs (at least one: its name and time).
fn blocks(m: &ChatMessage) -> usize {
    m.body.blocks.len().clamp(1, MAX_BLOCKS)
}

/// A guess at `m`'s height before it's laid out, `width` the pane's: a line a block and more for
/// a long one, room for its pictures, files and chips, and its name above when it's a row's
/// `first`.
fn guess(m: &ChatMessage, first: bool, width: f32) -> f32 {
    let per_line = ((width * 0.7).clamp(180.0, 620.0) - 20.0) / 7.5;
    let lines: f32 = if m.deleted.is_some() || m.unshowable {
        1.0
    } else {
        let line = |b: &quatrelle_engine::model::pim::teams::Block| {
            let n: usize = b.runs.iter().map(|r| r.text.len()).sum();
            (n as f32 / per_line).ceil().max(1.0)
        };
        m.body.blocks.iter().take(MAX_BLOCKS).map(line).sum::<f32>().max(1.0)
    };
    let runs = m.body.blocks.iter().take(MAX_BLOCKS).flat_map(|b| &b.runs);
    let pictures = runs.filter(|r| r.hosted.is_some() || r.outside.is_some()).take(40).count() as f32;
    let files = m.attachments.len().min(20) as f32;
    let chips = if m.reactions.is_empty() { 0.0 } else { 30.0 };
    (if first { 42.0 } else { 20.0 }) + lines * 19.0 + (pictures / 3.0).ceil() * 130.0 + files * 90.0 + chips
}
