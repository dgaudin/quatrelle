// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Pasting into the message with a look: HTML another program put on the clipboard, and text
//! that is plainly Markdown (copied from a text editor, a chat, a terminal), each in place of
//! the selection, as one step to undo.

use super::*;

/// A step to undo: the message as `text` and `styles`, the cursor at `cursor`.
fn snap(ed: &mut Editor, text: String, styles: Styles, cursor: usize) {
    ed.undo.push(Snap { text, styles, cursor });
    if ed.undo.len() > UNDO_MAX {
        ed.undo.remove(0);
    }
    ed.redo.clear();
}

/// Paste what another program put on the clipboard as HTML (a signature copied from Outlook,
/// part of a web page) with its look, in place of the selection; with no HTML there, text that
/// is plainly Markdown, formatted ([`paste_markdown`]). `false` when there is neither, or the
/// HTML holds no text (a picture alone is pasted as a picture).
pub(super) fn paste_formatted(ui: &mut Ui, ed: &mut Editor, c: &mut Compose, id: egui::Id) -> bool {
    // Keep text only: the text box pastes the text as it is.
    if std::mem::take(&mut ed.paste_plain) {
        return false;
    }
    let Some(html) = crate::clipboard_html() else { return paste_markdown(ui, ed, c, id) };
    let (mut add, mut add_styles, pictures) = quatrelle_engine::mime::pasted_html_with_pictures(&html);
    if add.replace(PICTURE, "").trim().is_empty() {
        return false;
    }
    ui.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Paste(_))));
    c.styles.fit(&c.body);
    snap(ed, c.body.clone(), c.styles.clone(), ed.sel.start);
    // The pictures that came along (embedded in the HTML) become the message's; one named by
    // `cid:` stays where the message has it already (text copied from the editor), and any
    // other picture goes.
    for p in pictures {
        if !c.inline.iter().any(|x| x.cid == p.cid) {
            c.inline.push(p);
        }
    }
    let known: Vec<String> = c.inline.iter().map(|p| p.cid.clone()).collect();
    for _ in 0..10_000 {
        let unknown =
            add.chars().enumerate().find(|(i, ch)| *ch == PICTURE && add_styles.style(*i).picture.as_ref().is_none_or(|cid| !known.contains(cid)));
        let Some((at, _)) = unknown else { break };
        rich::splice(&mut add, &mut add_styles, at..at + 1, "", &Styles::plain(""));
    }
    let n = c.body.chars().count();
    let sel = ed.sel.start.min(n)..ed.sel.end.min(n);
    rich::splice(&mut c.body, &mut c.styles, sel.clone(), &add, &add_styles);
    let end = sel.start + add.chars().count();
    set_selection(ui.ctx(), id, end..end);
    ed.sel = end..end;
    ed.typing = None;
    ui.ctx().request_repaint();
    true
}

/// Text pasted with no HTML on the clipboard that is plainly Markdown
/// ([`mime::pasted_markdown`](quatrelle_engine::mime::pasted_markdown)): formatted, in place of
/// the selection. Undo right after gives back the text as it was pasted (as the text box would
/// have put it in), and once more the message as it was before. `false` when it isn't Markdown,
/// for the text box to paste it as it is.
fn paste_markdown(ui: &mut Ui, ed: &mut Editor, c: &mut Compose, id: egui::Id) -> bool {
    let raw = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Paste(s) => Some(s.replace("\r\n", "\n").replace('\r', "\n")),
            _ => None,
        })
    });
    let Some(raw) = raw else { return false };
    let Some((add, add_styles)) = quatrelle_engine::mime::pasted_markdown(&raw) else { return false };
    ui.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Paste(_))));
    c.styles.fit(&c.body);
    let n = c.body.chars().count();
    let sel = ed.sel.start.min(n)..ed.sel.end.min(n);
    // The text as pasted, in the look typed at the cursor.
    let typing = ed.typing.clone().unwrap_or_else(|| typing_style(&c.body, &c.styles, sel.start));
    let as_text: String = c.body.chars().take(sel.start).chain(raw.chars()).chain(c.body.chars().skip(sel.end)).collect();
    let typed_end = sel.start + raw.chars().count();
    let mut as_text_styles = c.styles.clone();
    as_text_styles.edit_at(&c.body, &as_text, &typing, Some(typed_end));
    snap(ed, c.body.clone(), c.styles.clone(), sel.start);
    snap(ed, as_text, as_text_styles, typed_end);
    rich::splice(&mut c.body, &mut c.styles, sel.clone(), &add, &add_styles);
    let end = sel.start + add.chars().count();
    set_selection(ui.ctx(), id, end..end);
    ed.sel = end..end;
    ed.typing = None;
    ui.ctx().request_repaint();
    true
}
