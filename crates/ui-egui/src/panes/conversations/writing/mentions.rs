// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The people @mentioned in the box. An "@" typed at the text's start or after a space offers the
//! open chat's members who can be mentioned ([`offered`], decision 11: of the user's organisation,
//! guests included, never the user), at most [`OFFER_MAX`], in rows above the box ([`rows`]);
//! ↑ and ↓ choose, Enter, Tab or a click picks ([`pick`]): the "@" and what follows it give way to
//! the member's name on one line ([`one_line`], the form the engine and the writer take a
//! mention's name in) and a space. The name is then a [`BoxMention`]: drawn as a bubble draws a
//! mention, sent as a byte range of the text with the member's id. One whose characters the user
//! changes is text again ([`follow`]), never a mention over other words. Esc closes the offer for
//! its "@" (until that "@" goes).
//!
//! What is hostile here: the members' names and addresses (Graph's): offered and drawn on one
//! line, a name with nothing left on one line never offered, looked at no further than the query's
//! [`QUERY_CHARS`] characters when matched; a guest (not proven of the organisation:
//! [`is_internal`]) offered with "External" before their name, so they don't pass for the
//! colleague they're named like. What it costs is bounded: the offer worked out again
//! only when its "@" or its query changes, the mentions at [`MAX_MENTIONS`].

use egui::{Sense, pos2, vec2};
use quatrelle_engine::ChatMembers;
use quatrelle_engine::model::pim::teams::{is_internal, mentionable, one_line};

use super::*;

/// The most members offered at once.
const OFFER_MAX: usize = 8;
/// The most characters typed after an "@" that are looked up.
const QUERY_CHARS: usize = 40;
/// An offered member's row, and the widest the offer is.
const ROW_H: f32 = 26.0;
const OFFER_W: f32 = 360.0;

/// A person @mentioned in the box: the characters of the text that name them, and their user id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BoxMention {
    pub(crate) chars: Range<usize>,
    pub(crate) user: String,
}

/// The members offered after an "@": the "@"'s character, what's typed after it up to the caret,
/// those of the chat it finds ([`Offered`]), the one chosen, and
/// whether Esc closed it.
#[derive(Clone, Debug, Default)]
pub(super) struct Offer {
    at: usize,
    query: String,
    people: Vec<Offered>,
    chosen: usize,
    closed: bool,
}

impl ChatDraft {
    /// Whether members are offered: the box's keys are then the offer's.
    pub(super) fn offering(&self) -> bool {
        self.offer.as_ref().is_some_and(|o| !o.closed && !o.people.is_empty())
    }
}

/// `mentions` following a change of the text from `old` to `new`: what both start with and end
/// with is unchanged, the change is what's between. A mention before the change stays, one after
/// it moves with it, one the change touches is text again.
pub(super) fn follow(mentions: &mut Vec<BoxMention>, old: &str, new: &str) {
    if mentions.is_empty() || old == new {
        return;
    }
    let (n_old, n_new) = (old.chars().count(), new.chars().count());
    let prefix = old.chars().zip(new.chars()).take_while(|(x, y)| x == y).count();
    let most = n_old.min(n_new).saturating_sub(prefix);
    let suffix = old.chars().rev().zip(new.chars().rev()).take(most).take_while(|(x, y)| x == y).count();
    // The change, in `old`'s characters; what follows it moves by the difference in length.
    let changed = prefix..n_old.saturating_sub(suffix);
    let moved = |c: usize| c.saturating_add(n_new).saturating_sub(n_old);
    mentions.retain_mut(|m| {
        if m.chars.end <= changed.start {
            true
        } else if m.chars.start >= changed.end {
            m.chars = moved(m.chars.start)..moved(m.chars.end);
            true
        } else {
            false
        }
    });
}

/// What's being looked up at the caret (a character of `text`): an "@" at the text's start or
/// after white space, then at most [`QUERY_CHARS`] characters, none a line break, up to the
/// caret. The "@"'s character, and what follows it.
pub(super) fn query_at(text: &str, caret: usize) -> Option<(usize, String)> {
    let end = text.char_indices().map(|(b, _)| b).chain(std::iter::once(text.len())).nth(caret)?;
    let head = text.get(..end)?;
    // `typed`: how many characters there are between the caret and this one.
    for (typed, (b, c)) in head.char_indices().rev().enumerate() {
        if c == '@' {
            let before = head.get(..b)?.chars().next_back();
            if before.is_some_and(|p| !p.is_whitespace()) {
                return None;
            }
            let at = caret.checked_sub(typed)?.checked_sub(1)?;
            return Some((at, head.get(b + 1..)?.to_string()));
        }
        if c == '\n' || c == '\r' || typed >= QUERY_CHARS {
            return None;
        }
    }
    None
}

/// A member offered after an "@": their user id, their name and address on one line, and whether
/// they're a guest (External: not proven of the organisation).
#[derive(Clone, Debug)]
pub(super) struct Offered {
    user: String,
    name: String,
    address: String,
    external: bool,
}

/// The open chat's `members` offered for `query`: those who can be @mentioned (of the
/// organisation `tenant`, guests included: [`mentionable`]), never the user `me`, with a name left
/// on one line and a usable id, whose name, a word of it, or address starts with the query
/// (whatever the capitals); in the members' order, at most [`OFFER_MAX`].
pub(super) fn offered(members: &ChatMembers, me: &str, tenant: &str, query: &str) -> Vec<Offered> {
    let mut out = Vec::new();
    for m in members.as_slice() {
        if out.len() >= OFFER_MAX {
            break;
        }
        let id = &m.user_id;
        if !mentionable(m, tenant) || id.is_empty() || id.len() > MAX_ID || id.chars().any(char::is_control) || id.eq_ignore_ascii_case(me) {
            continue;
        }
        let name = one_line(&m.name);
        let address = one_line(&m.address);
        if !name.is_empty() && (starts(&name, query) || name.split(' ').any(|w| starts(w, query)) || starts(&address, query)) {
            out.push(Offered { user: id.clone(), name, address, external: !is_internal(m, tenant) });
        }
    }
    out
}

/// Whether `text` starts with `query`, whatever the capitals (looked at no further than it).
fn starts(text: &str, query: &str) -> bool {
    let mut text = text.chars().flat_map(char::to_lowercase);
    query.chars().flat_map(char::to_lowercase).all(|q| text.next() == Some(q))
}

/// The offer as the text and the caret stand now (`focused`: the box has the keyboard, else none
/// is): worked out again when its "@" or its query changed, the first member chosen; one Esc
/// closed stays closed while its "@" is there. None past [`MAX_MENTIONS`].
pub(super) fn refresh(d: &mut ChatDraft, members: &ChatMembers, me: &str, tenant: &str, focused: bool) {
    let looked_up = focused && d.sel.is_empty() && d.mentions.len() < MAX_MENTIONS;
    let Some((at, query)) = looked_up.then(|| query_at(&d.text, d.sel.end)).flatten() else {
        d.offer = None;
        return;
    };
    match d.offer.as_mut() {
        Some(o) if o.at == at && (o.query == query || o.closed) => o.query = query,
        _ => d.offer = Some(Offer { at, people: offered(members, me, tenant, &query), query, chosen: 0, closed: false }),
    }
}

/// ↑ (`up`) or ↓ on the offer: the member above or below the one chosen.
pub(super) fn choose(d: &mut ChatDraft, up: bool) {
    if let Some(o) = &mut d.offer {
        let last = o.people.len().saturating_sub(1);
        o.chosen = if up { o.chosen.saturating_sub(1) } else { o.chosen.saturating_add(1).min(last) };
    }
}

/// Esc on the offer: closed, for its "@".
pub(super) fn close(d: &mut ChatDraft) {
    if let Some(o) = &mut d.offer {
        o.closed = true;
    }
}

/// Enter or Tab on the offer: the member chosen picked.
pub(super) fn pick_chosen(d: &mut ChatDraft) {
    let chosen = d.offer.as_ref().filter(|o| !o.closed).and_then(|o| o.people.get(o.chosen)).map(|o| (o.user.clone(), o.name.clone()));
    if let Some((user, name)) = chosen {
        pick(d, &user, &name);
    }
}

/// The member `user`, named `name`, picked: the "@" before the caret and what follows it give way
/// to the name on one line and a space, in the "@"'s look; the name a mention, the caret after the
/// space. Nothing past [`MAX_MENTIONS`], nor with nothing looked up at the caret.
pub(super) fn pick(d: &mut ChatDraft, user: &str, name: &str) {
    let name = one_line(name);
    if name.is_empty() || user.is_empty() || d.mentions.len() >= MAX_MENTIONS || !d.sel.is_empty() {
        return;
    }
    let caret = d.sel.end;
    let Some((at, _)) = query_at(&d.text, caret) else { return };
    d.styles.fit(&d.text);
    let (Some(head), Some(tail)) = (d.text.get(..byte_at(&d.text, at)), d.text.get(byte_at(&d.text, caret)..)) else { return };
    let new = format!("{head}{name} {tail}");
    let n = name.chars().count();
    let end = at.saturating_add(n).saturating_add(1);
    let look = rich_edit::typing_style(&d.text, &d.styles, at.saturating_add(1));
    d.styles.edit_at(&d.text, &new, &look, Some(end));
    follow(&mut d.mentions, &d.text, &new);
    let name_at = at..at.saturating_add(n);
    d.mentions.retain(|m| m.chars.end <= name_at.start || m.chars.start >= name_at.end);
    let k = d.mentions.partition_point(|m| m.chars.start < at);
    d.mentions.insert(k, BoxMention { chars: name_at, user: user.to_string() });
    d.text = new;
    d.sel = end..end;
    d.caret_to = Some(end..end);
    d.offer = None;
    d.changed();
}

/// The mentions of `text` given by byte ranges and user ids (a send or an edit's, a message read
/// back), by character: one off a character's boundary, empty, without a usable id or overlapping
/// one before it is left out; at most [`MAX_MENTIONS`], in the text's order.
pub(super) fn of_bytes(text: &str, given: &[(Range<usize>, String)]) -> Vec<BoxMention> {
    if given.is_empty() {
        return Vec::new();
    }
    let at = Chars::of(text);
    let mut out: Vec<BoxMention> = given
        .iter()
        .take(MAX_MENTIONS)
        .filter(|(_, user)| !user.is_empty() && user.len() <= MAX_ID && !user.chars().any(char::is_control))
        .filter_map(|(r, user)| Some(BoxMention { chars: at.range(r)?, user: user.clone() }))
        .filter(|m| !m.chars.is_empty())
        .collect();
    out.sort_by_key(|m| m.chars.start);
    let mut end = 0;
    out.retain(|m| {
        let apart = m.chars.start >= end;
        if apart {
            end = m.chars.end;
        }
        apart
    });
    out
}

/// The offer, drawn as rows above the box ("External" first for a guest, the name, then the
/// address dim; the member chosen on the accent's soft tint): the one clicked, when one is.
pub(super) fn rows(ui: &mut Ui, t: &Tokens, d: &ChatDraft) -> Option<(String, String)> {
    let o = d.offer.as_ref().filter(|_| d.offering())?;
    let mut clicked = None;
    let frame = egui::Frame::new()
        .fill(if t.dark { t.surface } else { egui::Color32::WHITE })
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(6)
        .inner_margin(egui::Margin::same(4));
    frame.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let w = ui.available_width().min(OFFER_W);
        for (k, Offered { user, name, address, external }) in o.people.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::click());
            let said = if *external { format!("{name}, {}", tl!("External")) } else { name.clone() };
            widgets::named(ui, &resp, &said, true, None);
            let on = k == o.chosen;
            if on || resp.hovered() {
                ui.painter().rect_filled(rect, 4.0, if on { t.accent_soft } else { t.hover_row });
            }
            let mut left = rect.left() + 8.0;
            if *external {
                // Whole, before the name the member chose.
                let (wide, _) = super::super::list::tag(ui.painter(), t, tl!("External"), left, rect.center().y - 8.5);
                left += wide + 6.0;
            }
            let room = (rect.right() - 8.0 - left).max(40.0);
            let name_g = widgets::one_line(ui, name, theme::regular(13.5), t.text, room);
            let after = left + name_g.size().x + 10.0;
            ui.painter().galley(pos2(left, rect.center().y - name_g.size().y / 2.0), name_g, t.text);
            let room = rect.right() - 8.0 - after;
            if !address.is_empty() && room > 24.0 {
                let address_g = widgets::one_line(ui, address, theme::regular(12.0), t.dim, room);
                ui.painter().galley(pos2(after, rect.center().y - address_g.size().y / 2.0), address_g, t.dim);
            }
            if resp.clicked() {
                clicked = Some((user.clone(), name.clone()));
            }
        }
    });
    ui.add_space(4.0);
    clicked
}
