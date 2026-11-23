// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The right-click menu on the text being written.

use super::*;

/// The right-click menu on the text, as Outlook's: what could take a misspelt word; Cut, Copy,
/// Paste with its options and Select all; the link (insert, edit, open, remove); clearing the
/// formatting; and the lists. Each command that has one shows its icon before its label.
pub(super) fn text_menu(resp: &egui::Response, ed: &mut Editor, c: &mut Compose, acts: &mut Vec<Action>, at_pointer: Option<usize>) {
    let n = c.body.chars().count();
    let sel = ed.sel.start.min(n)..ed.sel.end.min(n);
    let link = c.styles.style(sel.start).link.filter(|_| n > 0);
    let opening = !ed.menu_open;
    let shown = resp.context_menu(|ui| {
        ui.set_min_width(190.0);
        // The word the menu was opened on, when it isn't spelt right, in characters: Outlook
        // puts what could take its place at the top of this menu, so a red-underlined word is
        // two clicks from mended instead of a trip through the spelling dialog. Worked out as
        // the menu opens (the suggestions take a while), kept while it's shown.
        if opening {
            ed.menu_word = misspelt_at(c, at_pointer.unwrap_or(ed.sel.start));
            // Ten, as Outlook lists a good handful: the closest words come first, but a likely
            // one isn't always first ("vai" → "vais"), and five cut it off.
            ed.suggestions = ed.menu_word.as_ref().map(|(_, w)| crate::spellcheck::suggest_in(w, &check_for(c).langs, 10)).unwrap_or_default();
        }
        if let Some((range, word)) = ed.menu_word.clone() {
            let mut picked: Option<String> = None;
            for sug in &ed.suggestions {
                if ui.add_enabled(!c.sending, egui::Button::new((crate::widgets::menu_icon(None), RichText::new(sug).strong()))).clicked() {
                    picked = Some(sug.clone());
                }
            }
            if ed.suggestions.is_empty() {
                ui.label(RichText::new(tl!("No suggestions.")).weak());
            }
            if let Some(sug) = picked {
                let style = typing_style(&c.body, &c.styles, range.start);
                replace(ed, c, range.clone(), &sug, style);
                let end = range.start + sug.chars().count();
                ed.refocus = Some(end..end);
                ui.close();
            }
            ui.separator();
            // Let be in this message, or the writer's own dictionary (this word in it, or back
            // out).
            if ui
                .button((crate::widgets::menu_icon(None), tl!("Ignore all")))
                .on_hover_text(tl!("Let this word be, here and elsewhere in this message"))
                .clicked()
            {
                crate::spellcheck::ignore_all(c.id, &word);
                ed.refocus = Some(sel.clone());
                ui.close();
            }
            if crate::spellcheck::added(&word) {
                if ui.button((crate::widgets::menu_icon(None), tf!("Remove “{word}” from the dictionary", word = word))).clicked() {
                    acts.push(Action::Run("spelling.remove", serde_json::json!({"word": word.clone()})));
                    crate::spellcheck::remove_word(&word);
                    ui.close();
                }
            } else if ui
                .button((crate::widgets::menu_icon(Some(Icon::BookAdd)), tl!("Add to dictionary")))
                .on_hover_text(tl!("Stop underlining this word, here and in other messages"))
                .clicked()
            {
                acts.push(Action::Run("spelling.add", serde_json::json!({"word": word.clone()})));
                crate::spellcheck::add_word(&word);
                ui.close();
            }
            ui.separator();
        }
        let item = |ui: &mut Ui, icon: Option<Icon>, label: &str, keys: &str, enabled: bool| -> bool {
            let b = egui::Button::new((crate::widgets::menu_icon(icon), tl!(label))).shortcut_text(keys);
            let clicked = ui.add_enabled(enabled, b).clicked();
            if clicked {
                ui.close();
            }
            clicked
        };
        let some = !sel.is_empty();
        if item(ui, Some(Icon::Cut), "Cut", "Ctrl+X", some && !c.sending) {
            cut(ui, ed, c, &sel);
        }
        if item(ui, Some(Icon::Copy), "Copy", "Ctrl+C", some) {
            copy_selection(ui, c, &sel);
            ed.refocus = Some(sel.clone());
        }
        if item(ui, Some(Icon::Paste), "Paste", "Ctrl+V", !c.sending) {
            ed.paste_here(sel.clone());
        }
        // The paste options, under Paste: with the look the clipboard has, or as text alone.
        if item(ui, None, "Keep source formatting", "", !c.sending) {
            ed.paste_here(sel.clone());
        }
        if item(ui, None, "Keep text only", "", !c.sending) {
            ed.paste_text_only(sel.clone());
        }
        ui.separator();
        if item(ui, Some(Icon::SelectAll), "Select all", "Ctrl+A", n > 0) {
            ed.refocus = Some(0..n);
        }
        if !c.plain {
            ui.separator();
            if item(
                ui,
                Some(if link.is_some() { Icon::LinkEdit } else { Icon::Link }),
                if link.is_some() { "Edit link…" } else { "Link…" },
                "Ctrl+K",
                !c.sending,
            ) {
                ed.link = Some(LinkForm::open(c, sel.clone()));
            }
            if let Some(url) = &link {
                if item(ui, Some(Icon::PopOut), "Open link", "", true) {
                    acts.push(Action::OpenUrl(url.clone()));
                }
                if item(ui, Some(Icon::LinkDismiss), "Remove link", "", !c.sending) {
                    apply(ed, c, Op::Unlink);
                    ed.refocus = Some(sel.clone());
                }
            }
            ui.separator();
            if item(ui, Some(Icon::ClearFormat), "Clear formatting", "Ctrl+Space", some && !c.sending) {
                apply(ed, c, Op::Clear);
                ed.refocus = Some(sel.clone());
            }
            ui.separator();
            // Lists over the selection's paragraphs, checked (a tick for the icon) while the
            // selection is already one.
            let (first, last) = rich::lines_of(&c.body, sel.clone());
            let bullets_on = (first..=last).all(|l| c.styles.para(l).list == List::Bullet);
            let numbers_on = (first..=last).all(|l| c.styles.para(l).list == List::Number);
            if item(ui, Some(if bullets_on { Icon::Done } else { Icon::ListBullet }), "Bullets", "Ctrl+Shift+L", !c.sending) {
                apply(ed, c, Op::Bullets);
                ed.refocus = Some(sel.clone());
            }
            if item(ui, Some(if numbers_on { Icon::Done } else { Icon::ListNumber }), "Numbering", "", !c.sending) {
                apply(ed, c, Op::Numbers);
                ed.refocus = Some(sel.clone());
            }
        }
    });
    // Closed without a choice (Escape, a click elsewhere in the window): the text gets the
    // keyboard back, with what was selected.
    let open = shown.is_some();
    if ed.menu_open && !open && ed.refocus.is_none() && ed.link.is_none() && !resp.ctx.input(|i| i.pointer.any_pressed() && !resp.hovered()) {
        ed.refocus = Some(sel.clone());
    }
    ed.menu_open = open;
}
