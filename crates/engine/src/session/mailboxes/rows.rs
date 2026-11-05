// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The session: the rows the message list shows (date headings or groups, conversations,
//! pinned mail first), built with the list and kept ([`Session::rows`]).

use super::nav::NO_ROW;
use super::*;

/// What a message row stands for: one message of the list, or a conversation (an index into
/// [`Session::threads`]).
enum Stands {
    One(usize),
    Thread(usize),
}

impl Session {
    /// The list as rows with date headings (and conversations, when on); where each message row
    /// is among them; and each message's row, by its place in the list (its place among the
    /// message rows, [`NO_ROW`] for none).
    pub(crate) fn build_rows(&self) -> (Vec<Row>, Vec<u32>, Vec<u32>) {
        let mut out = Vec::with_capacity(self.list.len() + 8);
        let mut out_of = Vec::with_capacity(self.list.len());
        let mut last: Option<Row> = None;
        let arrange = self.prefs.arrange;
        let mut push = |out: &mut Vec<Row>, index: usize, count: usize, unread: usize, date: i64| {
            // The heading this message goes under, as the list is arranged.
            let m = self.list.get(index);
            let heading = match arrange {
                Arrange::Date => Row::Header(DateGroup::of(date, &self.clock).label()),
                Arrange::From => Row::Group(m.map(|m| person(Some(&m.from))).unwrap_or_default()),
                Arrange::To => Row::Group(m.map(|m| person(m.to.first())).unwrap_or_default()),
                Arrange::Subject => Row::Group(m.map(|m| m.base_subject()).unwrap_or_default()),
                Arrange::Size => Row::Header(size_group(m.map_or(0, |m| m.size))),
                Arrange::Flag => Row::Header(match m.map(|m| flag_rank(m)) {
                    Some(2) => "Flagged",
                    Some(1) => "Completed",
                    _ => "Not flagged",
                }),
                Arrange::Attachments => Row::Header(if m.is_some_and(|m| m.has_attachments) { "With attachments" } else { "Without attachments" }),
            };
            let heading = match heading {
                Row::Group(g) if g.trim().is_empty() => Row::Header(if arrange == Arrange::Subject { "(no subject)" } else { "(nobody)" }),
                h => h,
            };
            if last.as_ref() != Some(&heading) {
                out.push(heading.clone());
                last = Some(heading);
            }
            out.push(Row::Message { index, count, unread });
        };
        // Pinned mail first, under its own heading (searches keep plain date order).
        let pins = self.search.is_none();
        let pinned = |i: usize| pins && self.list.get(i).is_some_and(|m| m.flags.pinned());
        let (mut pinned_rows, mut pinned_of) = (Vec::new(), Vec::new());
        if self.prefs.conversations && !self.threads.is_empty() {
            for (ti, t) in self.threads.iter().enumerate() {
                let Some(&newest) = t.messages.iter().max_by_key(|&&i| self.list.get(i).map_or(0, |m| m.date)) else { continue };
                let unread = t.messages.iter().filter(|&&i| self.list.get(i).is_some_and(|m| !m.flags.seen)).count();
                if t.messages.iter().any(|&i| pinned(i)) {
                    pinned_rows.push(Row::Message { index: newest, count: t.messages.len(), unread });
                    pinned_of.push(Stands::Thread(ti));
                } else {
                    push(&mut out, newest, t.messages.len(), unread, t.latest);
                    out_of.push(Stands::Thread(ti));
                }
            }
        } else {
            for (i, m) in self.list.iter().enumerate() {
                if pinned(i) {
                    pinned_rows.push(Row::Message { index: i, count: 1, unread: usize::from(!m.flags.seen) });
                    pinned_of.push(Stands::One(i));
                } else {
                    push(&mut out, i, 1, usize::from(!m.flags.seen), m.date);
                    out_of.push(Stands::One(i));
                }
            }
        }
        let rows = if pinned_rows.is_empty() {
            out
        } else {
            pinned_rows.insert(0, Row::Header("Pinned"));
            pinned_rows.extend(out);
            pinned_of.extend(out_of);
            out_of = pinned_of;
            pinned_rows
        };
        // The message rows are in the order `out_of` was pushed, pinned ones first.
        let msg_rows: Vec<u32> =
            rows.iter().enumerate().filter(|(_, r)| matches!(r, Row::Message { .. })).map(|(r, _)| u32::try_from(r).unwrap_or(NO_ROW)).collect();
        let mut of_list = vec![NO_ROW; self.list.len()];
        let mut put = |i: usize, p: u32| {
            if let Some(slot) = of_list.get_mut(i) {
                *slot = p;
            }
        };
        for (p, stands) in out_of.iter().enumerate() {
            // Past u32: no row, looked for instead.
            let Ok(p) = u32::try_from(p) else { break };
            match stands {
                Stands::One(i) => put(*i, p),
                Stands::Thread(t) => self.threads.get(*t).into_iter().flat_map(|t| &t.messages).for_each(|&i| put(i, p)),
            }
        }
        (rows, msg_rows, of_list)
    }
}
