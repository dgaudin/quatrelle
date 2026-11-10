// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A plain-text message as it reads: selected with the rest of the message's text, as one text
//! ([`text_selection`], adapted from upstream 0eeb5d6a: it was a text box that couldn't be
//! typed in), the web and mail addresses written in it (`https://…`, `www.…`, `name@host`) shown
//! as links and opened with a click, as Outlook makes them links.

use super::*;

/// A plain-text body, in the reader's font, its addresses as links, the find bar's finds (`lit`)
/// lit; where the current one was drawn.
pub(super) fn plain_body(st: &mut UiState, ui: &mut Ui, text: &str, lit: Option<&find::Lit>, acts: &mut Vec<Action>) -> Option<Rect> {
    // Where the addresses laid out start, for the tests to point at.
    #[cfg(test)]
    ui.ctx().data_mut(|d| d.remove::<Vec<Rect>>(egui::Id::unique("plain-links")));
    let marks = lit.map(|l| l.marks_in(0..text.len())).unwrap_or_default();
    linked_run(st, ui, text, &theme::regular(14.5), lit.map(|l| (l, marks.as_slice())), "plain-links", acts)
}

/// Plain text `s` (a short message's, or a long one's chunk), a run of the message's text, its web
/// and mail addresses links: a hand and the address under the pointer, opened on a click (a drag
/// still selects). The find bar's finds in it (`found`) lit; where the current one was drawn.
/// Where its addresses start is kept for the tests under `links`.
pub(super) fn linked_run(
    st: &mut UiState,
    ui: &mut Ui,
    s: &str,
    font: &egui::FontId,
    found: Option<(&find::Lit, &[find::Mark])>,
    #[cfg_attr(not(test), allow(unused_variables))] links_at: &str,
    acts: &mut Vec<Action>,
) -> Option<Rect> {
    let t = st.tokens;
    let links = mime::find_links(s);
    let plain = egui::TextFormat { font_id: font.clone(), color: t.text, ..Default::default() };
    let link = egui::TextFormat { font_id: font.clone(), color: t.accent, underline: egui::Stroke::new(1.0, t.accent), ..Default::default() };
    let mut job = egui::text::LayoutJob::default();
    let mut at = 0;
    // Each address in characters too (the galley counts characters, the text's ranges bytes).
    let mut chars: Vec<std::ops::Range<usize>> = Vec::with_capacity(links.len());
    let mut count = 0usize;
    for (r, _) in &links {
        if let (Some(before), Some(address)) = (s.get(at..r.start), s.get(r.clone())) {
            job.append(before, 0.0, plain.clone());
            job.append(address, 0.0, link.clone());
            count = count.saturating_add(before.chars().count());
            let len = address.chars().count();
            chars.push(count..count.saturating_add(len));
            count = count.saturating_add(len);
            at = r.end;
        }
    }
    job.append(s.get(at..).unwrap_or(""), 0.0, plain);
    if let Some((l, marks)) = found {
        l.light(&mut job, marks);
    }
    job.wrap.max_width = ui.available_width();
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let r = text_selection::run(st, ui, galley.clone(), None);
    let hit = found.and_then(|(_, marks)| marks.iter().find(|m| m.current)).and_then(|m| {
        let at = s.get(..m.range.start)?.chars().count();
        Some(galley.pos_from_cursor(egui::text::CCursor::new(at)).translate(r.rect.min.to_vec2()))
    });
    if links.is_empty() {
        return hit;
    }
    #[cfg(test)]
    {
        let at: Vec<Rect> = chars.iter().map(|c| galley.pos_from_cursor(egui::text::CCursor::new(c.start)).translate(r.rect.min.to_vec2())).collect();
        ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<Vec<Rect>>(egui::Id::unique(links_at)).extend(at));
    }
    let under = |p: egui::Pos2| {
        let at = galley.cursor_from_pos(p - r.rect.min).index.0;
        chars.iter().position(|c| c.contains(&at)).and_then(|i| links.get(i)).map(|(_, url)| url.clone())
    };
    if let Some(url) = r.hover_pos().and_then(under) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        text_selection::pointing(st);
        egui::Response::clone(&r).on_hover_text_at_pointer(url);
    }
    if r.clicked()
        && let Some(url) = r.interact_pointer_pos().and_then(under)
    {
        acts.push(Action::OpenUrl(url));
    }
    hit
}
