//! The message window's ribbon, as Outlook's: the tabs Message, Insert, Options, Format Text
//! and Review, each a row of named groups of big buttons (the main window's widgets), with
//! Save, Pop out and Discard small at the right of the tabs.

use serde_json::json;

use super::*;
use crate::widgets::{self, Groups};

/// The ribbon's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ComposeTab {
    #[default]
    Message,
    Insert,
    Options,
    FormatText,
    Review,
}

impl ComposeTab {
    const ALL: [(ComposeTab, &'static str); 5] = [
        (ComposeTab::Message, "Message"),
        (ComposeTab::Insert, "Insert"),
        (ComposeTab::Options, "Options"),
        (ComposeTab::FormatText, "Format Text"),
        (ComposeTab::Review, "Review"),
    ];
}

/// What the ribbon needs to know of the message's account and window.
pub(crate) struct RibbonInput<'a> {
    /// The sending account's signature (Insert signature is offered when there is one).
    pub signature: &'a str,
    pub clock: &'a Clock,
    /// Delay Delivery (a server account), Voting Buttons (Exchange).
    pub can_delay: bool,
    pub can_vote: bool,
    /// The sending account has an S/MIME certificate: Encrypt and Sign are offered.
    pub has_cert: bool,
    /// Written in a window of its own; and windows can open at all (not the browser).
    pub in_window: bool,
    pub can_pop: bool,
    /// A language model is set in Options: Rewrite is offered.
    pub assistant: bool,
}

/// Typefaces offered (Outlook's usual ones); the first is the message's own.
pub(crate) const FONTS: [&str; 10] =
    ["Aptos", "Calibri", "Arial", "Segoe UI", "Tahoma", "Verdana", "Georgia", "Times New Roman", "Cambria", "Courier New"];

/// The tabs, then the chosen tab's groups.
pub(crate) fn compose_ribbon(ui: &mut Ui, tokens: &Tokens, ed: &mut Editor, c: &mut Compose, input: &RibbonInput<'_>, acts: &mut Vec<Action>) {
    let t = *tokens;
    tabs(ui, t, ed, c, input, acts);
    ui.add_space(2.0);
    c.styles.fit(&c.body);
    let n = c.body.chars().count();
    let sel = ed.sel.start.min(n)..ed.sel.end.min(n);
    let mut ops: Vec<Op> = Vec::new();
    egui::ScrollArea::horizontal().id_salt(("compose-ribbon", c.id)).show(ui, |ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let mut groups = Groups::new(ui);
            match ed.tab {
                ComposeTab::Message => {
                    clipboard(ui, t, ed, c, &sel);
                    groups.end(ui, &t, "Clipboard");
                    font(ui, t, ed, c, &sel, &mut ops, true);
                    groups.end(ui, &t, "Basic Text");
                    names(ui, t, c, acts);
                    groups.end(ui, &t, "Names");
                    include(ui, t, ed, c, &sel, input, acts);
                    groups.end(ui, &t, "Include");
                    tags(ui, t, c);
                    groups.last(ui, &t, "Tags");
                }
                ComposeTab::Insert => {
                    include(ui, t, ed, c, &sel, input, acts);
                    groups.end(ui, &t, "Include");
                    if widgets::big_button(ui, Icon::Rule, "Horizontal line", &t, !c.sending && !c.plain).clicked() {
                        ops.push(Op::Rule);
                    }
                    // Today's date or the time at the cursor, in one of a few ways (Outlook's
                    // Date & Time).
                    let r = widgets::big_button(ui, Icon::Calendar, "Date & time", &t, !c.sending);
                    egui::Popup::menu(&r).show(|ui| {
                        for text in date_choices(input.clock) {
                            if ui.button(text.as_str()).clicked() {
                                ops.push(Op::Insert(text));
                                ui.close();
                            }
                        }
                    });
                    groups.last(ui, &t, "Text");
                }
                ComposeTab::Options => options(ui, t, c, input, &mut groups),
                ComposeTab::FormatText => {
                    clipboard(ui, t, ed, c, &sel);
                    groups.end(ui, &t, "Clipboard");
                    font(ui, t, ed, c, &sel, &mut ops, false);
                    groups.end(ui, &t, "Font");
                    ui.add_enabled_ui(!c.plain && !c.sending, |ui| {
                        ui.allocate_ui_with_layout(vec2(0.0, widgets::RIBBON_BUTTON_H), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.add_space(20.0);
                            ui.horizontal(|ui| paragraph(ui, t, c, &sel, &mut ops));
                        });
                    });
                    groups.end(ui, &t, "Paragraph");
                    format_choice(ui, c);
                    groups.last(ui, &t, "Format");
                }
                ComposeTab::Review => {
                    // The browser has no dictionaries: nothing to check with.
                    if !cfg!(target_arch = "wasm32") {
                        if widgets::big_button(ui, Icon::SpellCheck, "Spelling", &t, !c.sending)
                            .on_hover_text(tl!("Check the spelling of the message (F7)"))
                            .clicked()
                        {
                            acts.push(Action::Dialog(Dialog::spelling()));
                        }
                        // The language the spelling is checked in (Outlook's Review › Language).
                        let r = widgets::big_button(ui, Icon::Language, "Language", &t, !c.sending)
                            .on_hover_text(tl!("The language the spelling is checked in"));
                        egui::Popup::menu(&r).show(|ui| language_menu(ui, c, acts));
                    }
                    // How much is written: the selection's count, or the whole message's.
                    if widgets::big_button(ui, Icon::WordCount, "Word count", &t, true).clicked() {
                        let range = if sel.is_empty() { 0..n } else { sel.clone() };
                        let text: String = c.body.chars().skip(range.start).take(range.end - range.start).filter(|ch| *ch != rich::PICTURE).collect();
                        acts.push(Action::Dialog(Dialog::WordCount(count_words(&text), !sel.is_empty())));
                    }
                    if input.assistant {
                        groups.end(ui, &t, "Proofing");
                        // The text written, rewritten by the language model set in Options.
                        let busy = c.rewrite.as_ref().is_some_and(|r| r.answer.is_none());
                        let r = widgets::big_button(ui, Icon::Drafts, "Rewrite", &t, !c.sending && !busy)
                            .on_hover_text(tl!("Have the language model set in Options rewrite what you wrote"));
                        egui::Popup::menu(&r).show(|ui| {
                            for (how, label) in [
                                ("fix", "Correct spelling and grammar"),
                                ("shorter", "Shorter"),
                                ("formal", "More formal"),
                                ("friendly", "Friendlier"),
                                ("english", "In English"),
                            ] {
                                if ui.button(tl!(label)).clicked() {
                                    acts.push(Action::Run("assistant.rewrite", json!({"how": how})));
                                    ui.close();
                                }
                            }
                        });
                        groups.last(ui, &t, "Assistant");
                    } else {
                        groups.last(ui, &t, "Proofing");
                    }
                }
            }
        });
    });
    link_dialog(ui, &t, ed, c);
    let changed = !ops.is_empty();
    for op in ops {
        apply(ed, c, op);
    }
    if changed {
        ed.refocus = Some(ed.sel.clone());
    }
}

/// The tab strip: the tabs at the left; Save, Pop out (or in) and Discard small at the right.
fn tabs(ui: &mut Ui, t: Tokens, ed: &mut Editor, c: &Compose, input: &RibbonInput<'_>, acts: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        for (tab, label) in ComposeTab::ALL {
            let on = ed.tab == tab;
            let r = ui.add(
                egui::Button::new(RichText::new(tl!(label)).color(if on { t.accent } else { t.dim }).font(if on {
                    theme::bold(13.5)
                } else {
                    theme::regular(13.5)
                }))
                .frame_when_inactive(false),
            );
            if on {
                // The underline glides from the tab left to the one chosen.
                let rect = r.rect;
                let ctx = ui.ctx().clone();
                let left = crate::motion::glide(&ctx, egui::Id::unique(("compose-tab-left", c.id)), rect.left() + 6.0, crate::motion::NORMAL);
                let right = crate::motion::glide(&ctx, egui::Id::unique(("compose-tab-right", c.id)), rect.right() - 6.0, crate::motion::NORMAL);
                ui.painter()
                    .line_segment([egui::pos2(left, rect.bottom() + 1.0), egui::pos2(right, rect.bottom() + 1.0)], Stroke::new(2.0, t.accent));
            }
            if r.clicked() {
                ed.tab = tab;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::tiny_icon_button(ui, Icon::Delete, "Discard", &t).clicked() {
                acts.push(Action::Run("compose.discard", json!({})));
            }
            if input.can_pop {
                let (icon, tip, id) = if input.in_window {
                    (Icon::PopIn, "Back to the reading pane", "compose.popIn")
                } else {
                    (Icon::PopOut, "Write it in a window of its own", "compose.popOut")
                };
                if widgets::tiny_icon_button(ui, icon, tip, &t).clicked() {
                    acts.push(Action::Run(id, json!({})));
                }
            }
            if !c.sending && widgets::tiny_icon_button(ui, Icon::Drafts, "Save draft (Ctrl+S)", &t).clicked() {
                acts.push(Action::Run("compose.saveDraft", json!({})));
            }
        });
    });
}

/// Clipboard: Paste (and its menu), with Cut and Copy stacked beside it.
fn clipboard(ui: &mut Ui, t: Tokens, ed: &mut Editor, c: &mut Compose, sel: &Range<usize>) {
    let some = !sel.is_empty();
    if widgets::big_button(ui, Icon::Paste, "Paste", &t, !c.sending).on_hover_text("Ctrl+V").clicked() {
        ed.paste = true;
        ed.refocus = Some(sel.clone());
    }
    let r = widgets::big_more(ui, "Paste options", &t);
    egui::Popup::menu(&r).show(|ui| {
        if ui.button(tl!("Paste")).clicked() {
            ed.paste = true;
            ed.refocus = Some(sel.clone());
            ui.close();
        }
        if ui.button(tl!("Keep text only")).on_hover_text(tl!("Paste without the look it has where it was copied")).clicked() {
            ed.paste = true;
            ed.paste_plain = true;
            ed.refocus = Some(sel.clone());
            ui.close();
        }
    });
    widgets::stack(ui, |ui| {
        if ui.add_enabled(some && !c.sending, egui::Button::new(tl!("Cut"))).on_hover_text("Ctrl+X").clicked() {
            cut(ui, ed, c, sel);
        }
        if ui.add_enabled(some, egui::Button::new(tl!("Copy"))).on_hover_text("Ctrl+C").clicked() {
            copy_selection(ui, c, sel);
            ed.refocus = Some(sel.clone());
        }
    });
}

/// Font, on two rows: typeface, size, bigger, smaller, clear; bold, italic, underline,
/// strikethrough, highlight, colour, and (`with_paragraph`, the Message tab) lists, indents
/// and alignment after them.
fn font(ui: &mut Ui, t: Tokens, ed: &Editor, c: &Compose, sel: &Range<usize>, ops: &mut Vec<Op>, with_paragraph: bool) {
    let cur =
        ed.typing.clone().unwrap_or_else(|| if sel.is_empty() { typing_style(&c.body, &c.styles, sel.start) } else { c.styles.style(sel.start) });
    let letter = |bold: bool, italic: bool, under: bool, strike: bool| {
        let line = Stroke::new(1.2, t.text);
        TextFormat {
            font_id: if bold { theme::bold(15.0) } else { theme::regular(15.0) },
            color: t.text,
            italics: italic,
            underline: if under { line } else { Stroke::NONE },
            strikethrough: if strike { line } else { Stroke::NONE },
            ..Default::default()
        }
    };
    ui.add_enabled_ui(!c.plain && !c.sending, |ui| {
        ui.allocate_ui_with_layout(vec2(0.0, widgets::RIBBON_BUTTON_H), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
            ui.horizontal(|ui| {
                let shown = cur.font.clone().unwrap_or_else(|| FONTS[0].to_string());
                egui::ComboBox::from_id_salt(("font-name", c.id)).width(118.0).selected_text(shown.as_str()).show_ui(ui, |ui| {
                    for (i, f) in FONTS.iter().enumerate() {
                        if ui.selectable_label(shown == *f, *f).clicked() {
                            ops.push(Op::Font((i > 0).then(|| f.to_string())));
                        }
                    }
                });
                egui::ComboBox::from_id_salt(("font-size", c.id)).width(48.0).selected_text(cur.size().to_string()).show_ui(ui, |ui| {
                    for z in rich::SIZES {
                        if ui.selectable_label(cur.size() == z, z.to_string()).clicked() {
                            ops.push(Op::Size(z));
                        }
                    }
                });
                for (ic, tip, op) in [
                    (Icon::FontGrow, "Increase font size (Ctrl+])", Op::Grow),
                    (Icon::FontShrink, "Decrease font size (Ctrl+[)", Op::Shrink),
                    (Icon::ClearFormat, "Clear all formatting (Ctrl+Space)", Op::Clear),
                ] {
                    if fmt_button(ui, &t, Face::Icon(ic), false, tip).clicked() {
                        ops.push(op);
                    }
                }
            });
            ui.horizontal(|ui| {
                for (s, face, on, tip, op) in [
                    ("B", letter(true, false, false, false), cur.bold, "Bold (Ctrl+B)", Op::Bold),
                    ("I", letter(false, true, false, false), cur.italic, "Italic (Ctrl+I)", Op::Italic),
                    ("U", letter(false, false, true, false), cur.underline, "Underline (Ctrl+U)", Op::Underline),
                    ("ab", letter(false, false, false, true), cur.strike, "Strikethrough", Op::Strike),
                ] {
                    if fmt_button(ui, &t, Face::Letter(s, face), on, tip).clicked() {
                        ops.push(op);
                    }
                }
                if let Some(h) = color_button(ui, &t, "ab", cur.highlight, &HIGHLIGHTS, "No color", "Text highlight color") {
                    ops.push(Op::Highlight(h));
                }
                if let Some(col) = color_button(ui, &t, "A", cur.color, &COLORS, "Automatic", "Font color") {
                    ops.push(Op::Color(col));
                }
                if with_paragraph {
                    ui.add_space(4.0);
                    paragraph(ui, t, c, sel, ops);
                }
            });
        });
    });
}

/// Lists, indents and alignment.
fn paragraph(ui: &mut Ui, t: Tokens, c: &Compose, sel: &Range<usize>, ops: &mut Vec<Op>) {
    let para = c.styles.para(rich::lines_of(&c.body, sel.start..sel.start).0);
    for (ic, on, tip, op) in [
        (Icon::ListBullet, para.list == List::Bullet, "Bullets (Ctrl+Shift+L)", Op::Bullets),
        (Icon::ListNumber, para.list == List::Number, "Numbering", Op::Numbers),
        (Icon::IndentLess, false, "Decrease indent (Ctrl+Shift+T)", Op::Outdent),
        (Icon::IndentMore, false, "Increase indent (Ctrl+T)", Op::Indent),
        (Icon::AlignLeft, para.align == Align::Left, "Align left (Ctrl+L)", Op::Align(Align::Left)),
        (Icon::AlignCenter, para.align == Align::Center, "Center (Ctrl+E)", Op::Align(Align::Center)),
        (Icon::AlignRight, para.align == Align::Right, "Align right (Ctrl+R)", Op::Align(Align::Right)),
    ] {
        if fmt_button(ui, &t, Face::Icon(ic), on, tip).clicked() {
            ops.push(op);
        }
    }
}

/// Names: the address book, and the names typed checked against it.
fn names(ui: &mut Ui, t: Tokens, c: &Compose, acts: &mut Vec<Action>) {
    if widgets::big_button(ui, Icon::AddressBook, "Address Book", &t, !c.sending).on_hover_text(tl!("Ctrl+Shift+B")).clicked() {
        acts.push(Action::Dialog(Dialog::select_names(c, "to")));
    }
    if widgets::big_button(ui, Icon::CheckNames, "Check Names", &t, !c.sending)
        .on_hover_text(tl!("Fill in the addresses of the names typed, from your contacts and the people you've written to"))
        .clicked()
    {
        acts.push(Action::Run("compose.checkNames", json!({})));
    }
}

/// Include: a file, a picture, a link, the signature.
fn include(ui: &mut Ui, t: Tokens, ed: &mut Editor, c: &mut Compose, sel: &Range<usize>, input: &RibbonInput<'_>, acts: &mut Vec<Action>) {
    if widgets::big_button(ui, Icon::Attach, "Attach File", &t, !c.sending).clicked() {
        acts.push(Action::AttachFiles);
    }
    // A message, a contact or an appointment, as a file (Outlook's Attach Item).
    let r = widgets::big_button(ui, Icon::NewMail, "Attach Item", &t, !c.sending);
    egui::Popup::menu(&r).show(|ui| {
        use quatrelle_engine::ItemKind;
        for (kind, label) in [(ItemKind::Message, tl!("Message…")), (ItemKind::Contact, tl!("Contact…")), (ItemKind::Event, tl!("Appointment…"))]
        {
            if ui.button(label).clicked() {
                acts.push(Action::AttachItem(kind));
                ui.close();
            }
        }
    });
    if widgets::big_button(ui, Icon::Image, "Pictures", &t, !c.sending && !c.plain).clicked() {
        acts.push(Action::InsertPicture);
    }
    if widgets::big_button(ui, Icon::Link, "Link", &t, !c.sending && !c.plain).on_hover_text(tl!("Ctrl+K")).clicked() {
        ed.link = Some(LinkForm::open(c, sel.clone()));
    }
    // The account's signature at the cursor, or making and changing it, from the message
    // (Outlook's Signature menu).
    let r = widgets::big_button(ui, Icon::Signature, "Signature", &t, !c.sending);
    egui::Popup::menu(&r).show(|ui| {
        if ui.add_enabled(!input.signature.trim().is_empty(), egui::Button::new(tl!("Insert signature"))).clicked() {
            acts.push(Action::InsertSignature);
            ui.close();
        }
        if ui.button(tl!("Signatures…")).clicked() {
            acts.push(Action::EditSignature { account: c.account.clone(), insert: true });
            ui.close();
        }
    });
}

/// Tags: high and low importance.
fn tags(ui: &mut Ui, t: Tokens, c: &mut Compose) {
    if widgets::big_toggle(ui, Icon::Important, "High importance", &t, c.importance > 0).clicked() {
        c.importance = if c.importance > 0 { 0 } else { 1 };
    }
    if widgets::big_toggle(ui, Icon::LowImportance, "Low importance", &t, c.importance < 0).clicked() {
        c.importance = if c.importance < 0 { 0 } else { -1 };
    }
}

/// Options: Bcc, encrypting and signing (S/MIME), a read receipt, voting buttons, delay delivery,
/// where replies go, the format.
fn options(ui: &mut Ui, t: Tokens, c: &mut Compose, input: &RibbonInput<'_>, groups: &mut Groups) {
    if widgets::big_toggle(ui, Icon::Bcc, "Bcc", &t, c.show_cc).on_hover_text(tl!("Hidden from the other recipients")).clicked() {
        c.show_cc = !c.show_cc;
    }
    groups.end(ui, &t, "Show Fields");
    // Outlook's Encrypt group, for an account with a certificate.
    if input.has_cert {
        if widgets::big_toggle(ui, Icon::Lock, "Encrypt", &t, c.encrypt)
            .on_hover_text(tl!("Only the recipients can read it: each needs to have sent you a signed message"))
            .clicked()
        {
            c.encrypt = !c.encrypt;
        }
        if widgets::big_toggle(ui, Icon::Seal, "Sign", &t, c.sign)
            .on_hover_text(tl!("Signed with your certificate: the recipients can tell it's from you and unchanged"))
            .clicked()
        {
            c.sign = !c.sign;
        }
        groups.end(ui, &t, "Encrypt");
    }
    if widgets::big_toggle(ui, Icon::Read, "Read receipt", &t, c.read_receipt)
        .on_hover_text(tl!("Ask the recipients to confirm they've read it (their mail app may not, or may ask them)"))
        .clicked()
    {
        c.read_receipt = !c.read_receipt;
    }
    // Outlook's Use Voting Buttons (Exchange carries them).
    if input.can_vote {
        let label = if c.voting.is_empty() { tl!("Voting buttons").to_string() } else { c.voting.replace(';', " · ") };
        let r = widgets::big_toggle(ui, Icon::Vote, &label, &t, !c.voting.is_empty()).on_hover_text(tl!("Let the recipients answer with a button"));
        egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| crate::reading::voting_menu(ui, &t, c));
    }
    groups.end(ui, &t, "Tracking");
    // Outlook's Delay Delivery and Direct Replies To.
    if input.can_delay {
        let label = match c.send_after {
            Some(at) => tf!("Sends {when}", when = crate::i18n::date(&quatrelle_engine::model::dates::full_time(at, input.clock))),
            None => tl!("Delay delivery").to_string(),
        };
        let r = widgets::big_toggle(ui, Icon::DelayDelivery, &label, &t, c.send_after.is_some())
            .on_hover_text(tl!("Keep it in the Outbox and send it later (Quatrelle has to be running then)"));
        egui::Popup::menu(&r)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| crate::reading::delay_menu(ui, &t, c, input.clock));
    }
    if widgets::big_toggle(ui, Icon::Reply, "Replies to", &t, c.show_reply_to || !c.reply_to.is_empty())
        .on_hover_text(tl!("Have replies sent to another address"))
        .clicked()
    {
        c.show_reply_to = !c.show_reply_to;
    }
    groups.end(ui, &t, "More Options");
    format_choice(ui, c);
    groups.last(ui, &t, "Format");
}

/// HTML or plain text, as Outlook's Format group.
fn format_choice(ui: &mut Ui, c: &mut Compose) {
    widgets::stack(ui, |ui| {
        if ui.radio(!c.plain, tl!("HTML")).clicked() {
            c.plain = false;
        }
        if ui.radio(c.plain, tl!("Plain text")).on_hover_text(tl!("Send without formatting")).clicked() {
            c.plain = true;
        }
    });
}

/// The selection onto the clipboard: as text, and with its look as HTML where the app can put
/// both (Outlook and Word then paste it formatted).
pub(super) fn copy_selection(ui: &Ui, c: &Compose, sel: &Range<usize>) {
    let chosen: String = c.body.chars().skip(sel.start).take(sel.end.saturating_sub(sel.start)).collect();
    if !c.plain {
        let (part, styles) = rich::slice(&c.body, &c.styles, sel.clone());
        if crate::write_clipboard_html(&rich::to_html(&part, &styles), &chosen) {
            return;
        }
    }
    ui.ctx().copy_text(chosen);
}

/// The selection onto the clipboard, and out of the text.
pub(super) fn cut(ui: &Ui, ed: &mut Editor, c: &mut Compose, sel: &Range<usize>) {
    copy_selection(ui, c, sel);
    let style = typing_style(&c.body, &c.styles, sel.start);
    replace(ed, c, sel.clone(), "", style);
    ed.refocus = Some(ed.sel.clone());
}

/// The language the spelling is checked in: each dictionary's, or all of them.
pub(crate) fn language_menu(ui: &mut Ui, c: &Compose, acts: &mut Vec<Action>) {
    let current = effective_language(c);
    for code in crate::spellcheck::languages() {
        if ui.radio(current.as_deref() == Some(code.as_str()), language_name(&code)).clicked() {
            acts.push(Action::Run("compose.language", json!({"language": code})));
            ui.close();
        }
    }
    ui.separator();
    if ui.radio(current.is_none(), tl!("All languages")).clicked() {
        acts.push(Action::Run("compose.language", json!({"language": "all"})));
        ui.close();
    }
}

/// The dictionary a message's spelling is checked with: the one chosen for it, else the
/// interface's language when there's a dictionary for it; `None` for all of them.
pub(crate) fn effective_language(c: &Compose) -> Option<String> {
    match c.language.as_str() {
        "all" => None,
        "" => {
            let code = crate::i18n::current().code();
            let lang = code.split('-').next().unwrap_or(code);
            crate::spellcheck::has(lang).then(|| lang.to_string())
        }
        l => Some(l.to_string()),
    }
}

/// A language's name in that language (no translation needed), from its code.
pub(crate) fn language_name(code: &str) -> String {
    match code {
        "fr" => "Français",
        "en" => "English",
        "de" => "Deutsch",
        "es" => "Español",
        "it" => "Italiano",
        "pt" => "Português",
        "nl" => "Nederlands",
        "ru" => "Русский",
        "pl" => "Polski",
        "sv" => "Svenska",
        "da" => "Dansk",
        "nb" | "nn" | "no" => "Norsk",
        "fi" => "Suomi",
        "cs" => "Čeština",
        "sk" => "Slovenčina",
        "sl" => "Slovenščina",
        "hu" => "Magyar",
        "el" => "Ελληνικά",
        "tr" => "Türkçe",
        "uk" => "Українська",
        "bg" => "Български",
        "ro" => "Română",
        "hr" => "Hrvatski",
        "ca" => "Català",
        "gl" => "Galego",
        "eu" => "Euskara",
        "he" => "עברית",
        "ar" => "العربية",
        other => return other.to_uppercase(),
    }
    .to_string()
}
