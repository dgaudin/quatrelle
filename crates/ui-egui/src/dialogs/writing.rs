//! Dialogs: while writing (Address Book, saving changes, no subject, word count).

use super::*;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, people: &mut people_field::PeopleFields, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::SelectNames(f) => names::select_names(ui, f, s, people, t, acts),
        Dialog::AttachItem(f) => attach_item(ui, f, s, t, acts),
        Dialog::SaveChanges => {
            ui.set_width(380.0);
            title(ui, t, "Want to save your changes?");
            ui.label(RichText::new(tl!("The message is kept in Drafts if you save it.")).color(t.dim));
            ui.add_space(10.0);
            let mut keep = true;
            button_row(ui, |ui| {
                if ui.button(tl!("Cancel")).clicked() {
                    keep = false;
                }
                if ui.button(tl!("No")).clicked() {
                    acts.push(Action::Run("compose.close", json!({"save": false})));
                    keep = false;
                }
                if widgets::accent_button(ui, crate::icons::Icon::Drafts, "Yes", t, true).clicked() {
                    acts.push(Action::Run("compose.close", json!({"save": true})));
                    keep = false;
                }
            });
            keep
        }
        Dialog::EmptySubject => {
            ui.set_width(360.0);
            title(ui, t, "Send without a subject?");
            ui.label(RichText::new(tl!("Messages without a subject are easy to overlook.")).color(t.dim));
            let (ok, cancel) = buttons(ui, t, "Send anyway", true);
            if ok {
                acts.push(Action::Run("compose.send", json!({"allowEmptySubject": true})));
                return false;
            }
            !cancel
        }
        Dialog::Spelling(f) => {
            ui.set_width(460.0);
            title(ui, t, "Spelling");
            let mut keep = true;
            if crate::spellcheck::ready().is_none() {
                ui.label(RichText::new(tl!("The dictionaries are still being read.")).color(t.dim));
            } else if let Some(c) = s.compose.as_ref() {
                // The body first, then the subject (Outlook's order); both are read again on
                // every frame (from memory, paragraph by paragraph), so a change moves straight
                // on to the next word. Neither the signature nor the original under a reply is
                // checked, and the words let be in the message aren't.
                let mut words: Vec<(&'static str, std::ops::Range<usize>, String)> = Vec::new();
                for (field, subject) in [("body", false), ("subject", true)] {
                    words.extend(crate::rich_edit::misspelt_in(c, subject).into_iter().map(|(r, w)| (field, r, w)));
                }
                words.retain(|(field, r, _)| !f.passed.contains(&(field, r.start)));
                if words.is_empty() {
                    ui.label(RichText::new(tl!("No misspelt words.")).color(t.dim));
                } else {
                    // The first word left, with what could take its place (worked out once
                    // for it: the dictionaries take a while over it).
                    let (field, range, word) = (words[0].0, words[0].1.clone(), words[0].2.clone());
                    if f.shown != word {
                        f.shown = word.clone();
                        f.chosen = None;
                        f.suggestions = crate::spellcheck::suggest_in(&word, &crate::rich_edit::check_for(c).langs, 10);
                    }
                    ui.label(RichText::new(tf!("Not in the dictionary: {word}", word = word)).strong());
                    ui.add_space(6.0);
                    ui.label(RichText::new(tl!("Suggestions")).color(t.dim).small());
                    if f.suggestions.is_empty() {
                        ui.label(RichText::new(tl!("No suggestions.")).color(t.faint).small());
                    }
                    let suggestions = f.suggestions.clone();
                    egui::ScrollArea::vertical().max_height(170.0).auto_shrink([false, true]).show(ui, |ui| {
                        for sug in &suggestions {
                            let picked = f.chosen.as_deref() == Some(sug.as_str());
                            if ui.selectable_label(picked, sug).clicked() {
                                f.chosen = Some(sug.clone());
                            }
                        }
                    });
                    ui.add_space(10.0);
                    button_row(ui, |ui| {
                        if widgets::primary_button(ui, "Change", t, f.chosen.is_some()).clicked()
                            && let Some(with) = f.chosen.clone()
                        {
                            acts.push(Action::Run(
                                "compose.replaceText",
                                json!({"from": range.start, "to": range.end, "text": with, "field": field}),
                            ));
                            f.chosen = None;
                        }
                        if ui.button(tl!("Ignore once")).clicked() {
                            f.passed.push((field, range.start));
                            f.chosen = None;
                        }
                        if ui.button(tl!("Ignore all")).on_hover_text(tl!("Let this word be, here and elsewhere in this message")).clicked() {
                            crate::spellcheck::ignore_all(c.id, &word);
                            f.chosen = None;
                        }
                        if ui.button(tl!("Add to dictionary")).on_hover_text(tl!("Stop underlining this word, here and in other messages")).clicked()
                        {
                            acts.push(Action::Run("spelling.add", json!({"word": word.clone()})));
                            crate::spellcheck::add_word(&word);
                            f.chosen = None;
                        }
                    });
                }
            } else {
                ui.label(RichText::new(tl!("There's no message to check.")).color(t.dim));
            }
            ui.add_space(10.0);
            if ui.button(tl!("Close")).clicked() {
                keep = false;
            }
            keep
        }
        Dialog::SendCheck(found) => {
            use quatrelle_engine::SendCheck;
            ui.set_width(440.0);
            title(ui, t, "Check before sending");
            let mut exposed = false;
            for f in found.iter() {
                let line = match f {
                    SendCheck::Lookalike { address, like } => {
                        tf!("{address} looks like an address at {like}, but isn't one. Is it the right one?", address = address, like = like)
                    }
                    SendCheck::Exposed { count } => {
                        exposed = true;
                        tf!("{n} people who may not know each other will all see each other's addresses.", n = count)
                    }
                };
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
                    crate::icons::paint(ui.painter(), r, crate::icons::Icon::Warning, t.warn);
                    ui.add(egui::Label::new(RichText::new(line).color(t.text)).wrap());
                });
                ui.add_space(4.0);
            }
            if exposed {
                ui.add_space(4.0);
                if ui
                    .button(tl!("Put everyone in Bcc"))
                    .on_hover_text(tl!("Each of them gets the message without seeing who else did; you're in To"))
                    .clicked()
                {
                    acts.push(Action::Run("compose.hideRecipients", json!({})));
                    return false;
                }
            }
            let (ok, cancel) = buttons(ui, t, "Send anyway", true);
            if ok {
                acts.push(Action::Run("compose.send", json!({"allowEmptySubject": true, "checked": true})));
                return false;
            }
            !cancel
        }
        Dialog::WordCount(n, selection) => {
            ui.set_width(320.0);
            title(ui, t, "Word count");
            if *selection {
                ui.label(RichText::new(tl!("The selected text")).color(t.dim).small());
                ui.add_space(4.0);
            }
            egui::Grid::new("word-count").num_columns(2).spacing(vec2(24.0, 6.0)).show(ui, |ui| {
                for (label, value) in [
                    (tl!("Words"), n.words),
                    (tl!("Characters (no spaces)"), n.chars_no_spaces),
                    (tl!("Characters (with spaces)"), n.chars),
                    (tl!("Paragraphs"), n.paragraphs),
                    (tl!("Lines"), n.lines),
                ] {
                    ui.label(RichText::new(label).color(t.dim));
                    ui.label(value.to_string());
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            close_row(ui)
        }
        // Every other kind is shown by its own group (`body`).
        _ => true,
    }
}

/// Attach Item's choice: what kind of item, from where, and the ones picked.
#[derive(Clone, Debug)]
pub struct ItemForm {
    pub kind: quatrelle_engine::ItemKind,
    /// The folder messages are listed from, and the folders to choose from: (account, path,
    /// name).
    pub account: String,
    pub folder: String,
    pub folders: Vec<(String, String, String)>,
    pub query: String,
    /// The items listed (filled in after the frame, when `stale`), and the ids picked.
    pub rows: Vec<quatrelle_engine::ItemChoice>,
    pub picked: Vec<String>,
    pub stale: bool,
}

impl Default for ItemForm {
    fn default() -> Self {
        ItemForm {
            kind: quatrelle_engine::ItemKind::Message,
            account: String::new(),
            folder: String::new(),
            folders: Vec::new(),
            query: String::new(),
            rows: Vec::new(),
            picked: Vec::new(),
            stale: true,
        }
    }
}

/// Attach Item: a kind of item, a folder for messages, words to look for; a click picks an item
/// (and another click leaves it), a double-click attaches that one at once.
fn attach_item(ui: &mut Ui, f: &mut ItemForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    use quatrelle_engine::ItemKind;
    ui.set_width(640.0);
    title(ui, t, "Attach Item");
    ui.horizontal(|ui| {
        for (kind, label) in [(ItemKind::Message, tl!("Messages")), (ItemKind::Contact, tl!("Contacts")), (ItemKind::Event, tl!("Calendar"))] {
            if ui.selectable_label(f.kind == kind, label).clicked() && f.kind != kind {
                f.kind = kind;
                f.picked.clear();
                f.stale = true;
            }
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if f.kind == ItemKind::Message {
            ui.label(RichText::new(tl!("Folder")).strong());
            let shown = f.folders.iter().find(|(a, p, _)| *a == f.account && *p == f.folder).map(|(_, _, n)| n.clone()).unwrap_or_default();
            egui::ComboBox::from_id_salt("attach-item-folder").width(240.0).selected_text(shown).show_ui(ui, |ui| {
                for (a, p, name) in &f.folders {
                    if ui.selectable_label(f.account == *a && f.folder == *p, name).clicked() && (f.account != *a || f.folder != *p) {
                        (f.account, f.folder) = (a.clone(), p.clone());
                        f.picked.clear();
                        f.stale = true;
                    }
                }
            });
            ui.add_space(12.0);
        }
        let r = ui.add(egui::TextEdit::singleline(&mut f.query).desired_width(220.0).hint_text(tl!("Search")));
        if ui.memory(|m| m.focused().is_none()) {
            r.request_focus();
        }
        if r.changed() {
            f.stale = true;
        }
    });
    ui.add_space(6.0);
    let mut now: Option<String> = None;
    egui::ScrollArea::vertical().max_height(360.0).min_scrolled_height(360.0).auto_shrink([false, false]).show(ui, |ui| {
        if f.rows.is_empty() && !f.stale {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| ui.label(RichText::new(tl!("Nothing to attach here.")).color(t.dim)));
        }
        for row in &f.rows {
            let picked = f.picked.contains(&row.id);
            let when = match (row.when, f.kind) {
                (0, _) => String::new(),
                (d, ItemKind::Event) => crate::i18n::date(&quatrelle_engine::model::dates::full_time(d, &s.clock)),
                (d, _) => crate::i18n::date(&quatrelle_engine::model::dates::list_time(d, &s.clock)),
            };
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
            if picked || resp.hovered() {
                ui.painter().rect_filled(rect, 4.0, if picked { t.accent_soft } else { t.hover_row });
            }
            let p = ui.painter_at(rect.shrink2(vec2(8.0, 0.0)));
            let title = if row.title.trim().is_empty() { tl!("(no subject)").to_string() } else { row.title.clone() };
            p.text(rect.left_top() + vec2(8.0, 4.0), egui::Align2::LEFT_TOP, title, theme::regular(13.0), t.text);
            p.text(rect.left_bottom() + vec2(8.0, -4.0), egui::Align2::LEFT_BOTTOM, &row.detail, theme::regular(12.0), t.dim);
            p.text(rect.right_top() + vec2(-8.0, 4.0), egui::Align2::RIGHT_TOP, when, theme::regular(12.0), t.dim);
            if resp.double_clicked() {
                now = Some(row.id.clone());
            } else if resp.clicked() {
                if picked {
                    f.picked.retain(|id| *id != row.id);
                } else {
                    f.picked.push(row.id.clone());
                }
            }
        }
    });
    let (ok, cancel) = buttons(ui, t, "Attach", !f.picked.is_empty());
    let chosen = match now {
        Some(id) => vec![id],
        None if ok => f.picked.clone(),
        None => Vec::new(),
    };
    for id in &chosen {
        acts.push(Action::Run("compose.attachItem", json!({"kind": f.kind.code(), "id": id})));
    }
    chosen.is_empty() && !cancel
}
