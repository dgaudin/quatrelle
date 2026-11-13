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

/// How long after a paste the release of the Ctrl+V that brought it may come (egui sends the
/// paste when the key goes down, and only the key's release).
const SHORTCUT_HELD: std::time::Duration = std::time::Duration::from_secs(3);

/// What the editor keeps of pastes, for none to be lost and the log to say where each went.
#[derive(Default)]
pub(crate) struct Pastes {
    /// A paste asked for (by a shortcut or the menu) that hasn't come: the frame, and what asked.
    asked: Option<(u64, &'static str)>,
    /// When the last paste came.
    last: Option<web_time::Instant>,
    /// V is down, its press seen (a `v` typed): its release isn't a Ctrl+V's.
    v_held: bool,
    /// Pastes asked for that never came (the clipboard couldn't be read), told in the log.
    pub(crate) missed: u32,
    /// A paste taken for a picture copied (HTML of a picture alone, the picture read off the
    /// clipboard after the frame): its text, pasted as text next frame when no picture came.
    pub(crate) unpasted: Option<String>,
}

impl Pastes {
    /// A paste was asked for (`what`: Shift+Insert, the menu's Paste): told when none comes in
    /// this frame or the next.
    pub(crate) fn ask(&mut self, ctx: &egui::Context, what: &'static str) {
        self.asked = Some((ctx.cumulative_frame_nr(), what));
        ctx.request_repaint();
    }
}

/// A paste into the message this frame, its text having the keyboard: with its look where it
/// has one ([`paste_formatted`]), else left to the text box; each told in the log (never its
/// text), and a Ctrl+V that brought none ([`watch`]).
pub(super) fn pasted(ui: &mut Ui, ed: &mut Editor, c: &mut Compose, id: egui::Id) {
    let came = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Paste(s) => Some(s.chars().count()),
            _ => None,
        })
    });
    watch(ui, ed, came.is_some());
    let Some(chars) = came else { return };
    if c.plain {
        log::info!("paste into the message: {chars} characters; a plain-text message: as text, by the text box");
        return;
    }
    paste_formatted(ui, ed, c, id, chars);
}

/// A Ctrl+V (⌘V) in the message that brings no paste (the system's clipboard couldn't be read),
/// or a paste asked for otherwise ([`Pastes::ask`]) that doesn't come in that frame or the next:
/// told in the log, and counted. egui keeps a clipboard shortcut's press for itself (sending the
/// paste then) and sends its release alone: a release of V whose press wasn't seen is that
/// shortcut, which brought its paste when one came a moment before.
fn watch(ui: &Ui, ed: &mut Editor, came: bool) {
    let frame = ui.ctx().cumulative_frame_nr();
    let p = &mut ed.pastes;
    if came {
        p.asked = None;
        p.last = Some(web_time::Instant::now());
    } else if let Some((asked, what)) = p.asked
        && frame > asked
    {
        p.asked = None;
        p.missed = p.missed.saturating_add(1);
        log::info!("paste into the message: {what} brought no paste in that frame or the next (the clipboard couldn't be read)");
    }
    let recent = p.last.is_some_and(|t| t.elapsed() < SHORTCUT_HELD);
    let mut asked = false;
    ui.input(|i| {
        for e in &i.events {
            if let egui::Event::Key { key: Key::V, pressed, modifiers, .. } = e {
                if *pressed {
                    asked |= modifiers.command && !p.v_held;
                    p.v_held = true;
                } else {
                    asked |= !p.v_held && !recent;
                    p.v_held = false;
                }
            }
        }
    });
    if asked && !came {
        p.ask(ui.ctx(), "Ctrl+V");
    }
}

/// Paste what another program put on the clipboard as HTML (a signature copied from Outlook,
/// part of a web page) with its look, in place of the selection; text that is plainly Markdown,
/// formatted ([`paste_markdown`]), when there is no HTML or the HTML is only a code block of it
/// (copied from a page or an editor that showed it as code). Else (or when the HTML holds no
/// text) the text box pastes the text. `chars`: the paste's length, for the log.
fn paste_formatted(ui: &mut Ui, ed: &mut Editor, c: &mut Compose, id: egui::Id, chars: usize) {
    let raw = pasted_text(ui);
    let markdown = if raw.as_deref().is_some_and(quatrelle_engine::mime::looks_like_markdown) { "yes" } else { "no" };
    let tell = |html: &str, went: &str| {
        log::info!("paste into the message: {chars} characters; clipboard HTML: {html}; seen as Markdown: {markdown}; went in {went}");
    };
    // Keep text only: the text box pastes the text as it is.
    if std::mem::take(&mut ed.paste_plain) {
        tell("not read", "as text, by the text box (Keep text only)");
        return;
    }
    let began = web_time::Instant::now();
    let html = crate::clipboard_html();
    let ms = began.elapsed().as_millis();
    let read = html.as_ref().map_or_else(|| format!("none, asked in {ms} ms"), |h| format!("{} bytes, read in {ms} ms", h.len()));
    let went = match html {
        None => {
            if raw.is_some_and(|raw| paste_markdown(ui, ed, c, id, raw, false)) {
                "formatted, from the Markdown"
            } else {
                "as text, by the text box"
            }
        }
        Some(html) => {
            if let Some(text) = &raw
                && let Some(md) = quatrelle_engine::mime::code_block_markdown(&html, text)
                && paste_markdown(ui, ed, c, id, md, false)
            {
                "formatted, from the Markdown (the HTML is a code block of it)"
            } else if paste_html(ui, ed, c, id, &html) {
                "formatted, from the HTML"
            } else {
                "as text, by the text box (the HTML holds no text)"
            }
        }
    };
    tell(&read, went);
}

/// HTML pasted with its look ([`pasted_html_with_pictures`](quatrelle_engine::mime::pasted_html_with_pictures)),
/// in place of the selection. `false` when it holds no text (a picture alone is pasted as a
/// picture), for the text box to paste the text.
fn paste_html(ui: &mut Ui, ed: &mut Editor, c: &mut Compose, id: egui::Id, html: &str) -> bool {
    let (mut add, mut add_styles, pictures) = quatrelle_engine::mime::pasted_html_with_pictures(html);
    if nothing(&add) {
        return false;
    }
    // The pictures that came along (embedded in the HTML) become the message's; one named by
    // `cid:` stays where the message has it already (text copied from the editor), and any
    // other picture goes.
    let known: Vec<String> = c.inline.iter().chain(&pictures).map(|p| p.cid.clone()).collect();
    for _ in 0..10_000 {
        let unknown =
            add.chars().enumerate().find(|(i, ch)| *ch == PICTURE && add_styles.style(*i).picture.as_ref().is_none_or(|cid| !known.contains(cid)));
        let Some((at, _)) = unknown else { break };
        rich::splice(&mut add, &mut add_styles, at..at + 1, "", &Styles::plain(""));
    }
    c.styles.fit(&c.body);
    let n = c.body.chars().count();
    let sel = ed.sel.start.min(n)..ed.sel.end.min(n);
    let typing = ed.typing.clone().unwrap_or_else(|| typing_style(&c.body, &c.styles, sel.start));
    let kept = !nothing(&add);
    let (add, add_styles) = or_as_pasted((add, add_styles), &pasted_whole(ui), &typing);
    if kept {
        for p in pictures {
            if !c.inline.iter().any(|x| x.cid == p.cid) {
                c.inline.push(p);
            }
        }
    }
    take_pastes(ui, false);
    snap(ed, c.body.clone(), c.styles.clone(), ed.sel.start);
    put_in(ui, ed, c, id, sel, &add, &add_styles);
    true
}

/// Whether what was made of a paste holds nothing to put in: no text but white space, pictures
/// aside.
fn nothing(text: &str) -> bool {
    text.replace(PICTURE, "").trim().is_empty()
}

/// What the HTML or the Markdown path made of a paste (`made`), the paste being taken from the
/// text box; or, when that holds nothing ([`nothing`]), the text pasted (`raw`) in the look
/// typed at the cursor (`typing`), as the text box would have put it in: a paste is never lost.
pub(crate) fn or_as_pasted(made: (String, Styles), raw: &str, typing: &Style) -> (String, Styles) {
    if !nothing(&made.0) || raw.is_empty() {
        return made;
    }
    let mut styles = Styles::plain(raw);
    let look = styles.intern(typing.clone());
    styles.chars.fill(look);
    (raw.to_string(), styles)
}

/// The paste's events taken out of this frame (the middle click's, `middle`, or the clipboard's),
/// once what goes in instead is known: the text box would paste them again.
fn take_pastes(ui: &mut Ui, middle: bool) {
    ui.input_mut(|i| i.events.retain(|e| !matches!((e, middle), (egui::Event::Paste(_), false) | (egui::Event::MiddleClickPaste { .. }, true))));
}

/// `add` in place of `sel`, the cursor after it.
fn put_in(ui: &Ui, ed: &mut Editor, c: &mut Compose, id: egui::Id, sel: Range<usize>, add: &str, add_styles: &Styles) {
    rich::splice(&mut c.body, &mut c.styles, sel.clone(), add, add_styles);
    let end = sel.start + add.chars().count();
    set_selection(ui.ctx(), id, end..end);
    ed.sel = end..end;
    ed.typing = None;
    ui.ctx().request_repaint();
}

/// The text pasted this frame, its line ends made `\n`: `None` without one, or when it is
/// longer than Markdown is read (it is pasted as it is then, and isn't copied for nothing).
fn pasted_text(ui: &Ui) -> Option<String> {
    ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Paste(s) if s.len() <= quatrelle_engine::mime::MARKDOWN_MAX => Some(s.replace("\r\n", "\n").replace('\r', "\n")),
            _ => None,
        })
    })
}

/// The text pasted this frame, whatever its length, as the text box would put it in.
fn pasted_whole(ui: &Ui) -> String {
    ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Paste(s) => Some(s.replace("\r\n", "\n")),
            _ => None,
        })
    })
    .unwrap_or_default()
}

/// Text pasted (`raw`; by a middle click when `middle`) that is plainly Markdown
/// ([`mime::pasted_markdown`](quatrelle_engine::mime::pasted_markdown)): formatted, in place of
/// the selection. Undo right after gives back the text as it was pasted (as the text box would
/// have put it in), and once more the message as it was before. `false` when it isn't Markdown,
/// for the text box to paste it as it is.
fn paste_markdown(ui: &mut Ui, ed: &mut Editor, c: &mut Compose, id: egui::Id, raw: String, middle: bool) -> bool {
    let Some(made) = quatrelle_engine::mime::pasted_markdown(&raw) else { return false };
    c.styles.fit(&c.body);
    let n = c.body.chars().count();
    let sel = ed.sel.start.min(n)..ed.sel.end.min(n);
    // The text as pasted, in the look typed at the cursor.
    let typing = ed.typing.clone().unwrap_or_else(|| typing_style(&c.body, &c.styles, sel.start));
    let (add, add_styles) = or_as_pasted(made, &raw, &typing);
    take_pastes(ui, middle);
    let as_text: String = c.body.chars().take(sel.start).chain(raw.chars()).chain(c.body.chars().skip(sel.end)).collect();
    let typed_end = sel.start + raw.chars().count();
    let mut as_text_styles = c.styles.clone();
    as_text_styles.edit_at(&c.body, &as_text, &typing, Some(typed_end));
    snap(ed, c.body.clone(), c.styles.clone(), sel.start);
    snap(ed, as_text, as_text_styles, typed_end);
    put_in(ui, ed, c, id, sel, &add, &add_styles);
    true
}

/// A middle click on the text, which pastes the X11/Wayland selection where it lands (egui's
/// text box puts it there, keyboard or not): text that is plainly Markdown goes in formatted
/// there instead, as a paste does ([`paste_markdown`]: Undo right after gives back the text as
/// it was). Anything else is left to the text box.
pub(super) fn middle_click_markdown(ui: &mut Ui, ed: &mut Editor, c: &mut Compose, id: egui::Id) -> bool {
    let Some(drawn) = ed.drawn.clone() else { return false };
    // Where the click was, in the text's layer (as the text box reads it).
    let layer = ui.ctx().layer_transform_from_global(ui.layer_id());
    let clicked = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::MiddleClickPaste { pos, text } if text.len() <= quatrelle_engine::mime::MARKDOWN_MAX => {
                let pos = layer.map_or(*pos, |t| t * *pos);
                drawn.rect.contains(pos).then(|| (pos, text.replace("\r\n", "\n").replace('\r', "\n")))
            }
            _ => None,
        })
    });
    let Some((pos, text)) = clicked else { return false };
    let n = c.body.chars().count();
    let at = drawn.galley.cursor_from_pos(pos - drawn.pos).index.0.min(n);
    let before = std::mem::replace(&mut ed.sel, at..at);
    let (chars, markdown) = (text.chars().count(), quatrelle_engine::mime::looks_like_markdown(&text));
    let formatted = paste_markdown(ui, ed, c, id, text, true);
    log::info!(
        "middle-click paste into the message: {chars} characters; seen as Markdown: {}; went in {}",
        if markdown { "yes" } else { "no" },
        if formatted { "formatted, from the Markdown" } else { "as text, by the text box" }
    );
    if !formatted {
        ed.sel = before;
        return false;
    }
    ui.memory_mut(|m| m.request_focus(id));
    true
}
