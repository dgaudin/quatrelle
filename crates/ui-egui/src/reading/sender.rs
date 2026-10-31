//! The From row of a message being written, as Outlook's: the button offers each account's own
//! address, the mailboxes it opens (a shared one) and the other addresses it sent from, and
//! another address: a one-entry contacts field (a person, a shared mailbox or a list), whose name
//! is checked on Ctrl+K or as the field is left, never as it's typed.

use quatrelle_engine::model::Addressee;
use quatrelle_engine::{Compose, FieldId, Sender};

use super::*;
use crate::dialogs::people_field::{PeopleFields, accepts_of, egui_id};
use crate::look::people_field as widget;

/// The Other email address section's state between frames, by message.
#[derive(Clone, Default)]
struct Other {
    /// The section is open.
    open: bool,
    /// The field's text.
    text: String,
    /// The field is to take the keyboard (the section's just been opened).
    focus: bool,
    /// Send from it was asked as the field closed what was typed: the entry is taken at the next
    /// frame, once it's parsed.
    take: bool,
}

fn state_id(compose: u64) -> egui::Id {
    egui::Id::unique(("compose-other-sender", compose))
}

/// The other address's field in the message `compose` (the engine checks its names as it does
/// any field's).
pub(super) fn other_field(compose: u64) -> FieldId {
    FieldId { window: compose, name: "from" }
}

/// The one address `text` holds, if it holds just one.
fn one_address(text: &str) -> Option<EmailAddress> {
    addr::parse_list(text.trim()).ok().filter(|l| l.len() == 1).and_then(|l| l.into_iter().next())
}

/// Make the other address's field ready to draw while its section is open (its text parsed when
/// it changed, the names the engine settled taken, the suggestions for what's typed).
pub(super) fn prepare_other(people: &mut PeopleFields, s: &mut Session, ctx: &egui::Context, compose: u64, account: &str) {
    let o: Other = ctx.data_mut(|d| d.get_temp(state_id(compose))).unwrap_or_default();
    if o.open {
        let field = other_field(compose);
        people.prepare(s, field, Some(account), accepts_of(field.name), &o.text);
    }
}

/// The From button and who the message is from beside it. `choosing`: the choice dialog asks
/// which person a name in the other address means (the menu stays open under it).
pub(super) fn from_row(
    ui: &mut Ui,
    t: &Tokens,
    c: &mut Compose,
    senders: &[Sender],
    people: &mut PeopleFields,
    choosing: bool,
    acts: &mut Vec<Action>,
) {
    let own = senders.iter().find(|x| x.account == c.account && x.from.is_empty()).map(|x| x.label.clone()).unwrap_or_default();
    let b = label_button_menu(ui, t, "From", true, true).on_hover_text(tl!("Send from another account, a shared mailbox or another address"));
    // The field's suggestion list and card are drawn outside the menu: a click there, or in the
    // choice dialog, isn't one outside it.
    let field = egui_id(other_field(c.id));
    let on_field = ui.ctx().input(|i| i.pointer.interact_pos()).is_some_and(|p| widget::over_popups(ui.ctx(), field, p));
    let close = if choosing || on_field { egui::PopupCloseBehavior::IgnoreClicks } else { egui::PopupCloseBehavior::CloseOnClickOutside };
    egui::Popup::menu(&b).close_behavior(close).show(|ui| {
        for x in senders {
            let on = x.account == c.account && x.from.eq_ignore_ascii_case(&c.from);
            if ui.selectable_label(on, &x.label).clicked() {
                c.account = x.account.clone();
                c.from = x.from.clone();
                ui.close();
            }
        }
        ui.separator();
        if let Some(who) = other_address(ui, t, c, people, acts) {
            let mine = senders.iter().any(|x| x.account == c.account && x.from.is_empty() && x.email.eq_ignore_ascii_case(&who.address));
            c.from = if mine { String::new() } else { who.text() };
            ui.close();
        }
    });
    ui.allocate_ui_with_layout(vec2(ui.available_width(), ROW_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        if c.from.is_empty() {
            ui.add(egui::Label::new(RichText::new(&own).color(t.dim)).truncate());
        } else {
            // Another address: the account it goes through on hover.
            ui.add(egui::Label::new(RichText::new(&c.from).color(t.text)).truncate()).on_hover_text(tf!("Sent through {account}", account = own));
        }
    });
}

/// The one entry Send from it would take: one chip the field holds, with nothing typed after it
/// and nothing against it.
fn takeable(people: &PeopleFields, field: FieldId) -> Option<Addressee> {
    let mut who = people.addressees(field);
    let one = who.len() == 1 && !people.blocked(field) && people.typing(field).trim().is_empty();
    if one { who.pop() } else { None }
}

/// Outlook's Other E-mail Address: a one-entry contacts field under the menu's list, taken with
/// Send from it, or Enter with its suggestions closed. A name typed is checked on Ctrl+K or as
/// the field is left; several people fitting it are asked about in the choice dialog.
fn other_address(ui: &mut Ui, t: &Tokens, c: &Compose, people: &mut PeopleFields, acts: &mut Vec<Action>) -> Option<Addressee> {
    let id = state_id(c.id);
    let mut o: Other = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    if !o.open {
        if ui.selectable_label(false, tl!("Other email address…")).clicked() {
            (o.open, o.focus) = (true, true);
        }
        ui.data_mut(|d| d.insert_temp(id, o));
        return None;
    }
    let field = other_field(c.id);
    let fid = egui_id(field);
    // The heading names the field's input to a screen reader.
    let heading = ui.label(RichText::new(tl!("Other email address")).font(theme::bold(13.0)).color(t.text));
    ui.interact(heading.rect, widget::label_id(fid), Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, tl!("Other email address")));
    ui.label(RichText::new(tl!("Exchange sends from it when your account may send as it or on its behalf.")).color(t.dim).small());
    // As the field stood before this frame's keys and clicks: the click on Send from it may be
    // the one that closes what was typed.
    let can_take = takeable(people, field).is_some() || one_address(people.typing(field)).is_some();
    let had_focus = ui.memory(|m| m.has_focus(widget::input_id(fid)));
    ui.scope(|ui| {
        ui.set_width(300.0);
        let focus = std::mem::take(&mut o.focus);
        people.draw(ui, t, field, Some(&c.account), accepts_of(field.name), "", "Name or address", &mut o.text, focus, false, false, acts);
    });
    // Enter the suggestions didn't take (the list closed): Send from it.
    let enter = had_focus && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let clicked = ui.add_enabled(can_take, egui::Button::new(tl!("Send from it"))).clicked();
    let asked = clicked || enter;
    // What was typed is parsed at the next frame: asked as it closed, it's taken then.
    let taken = if asked || o.take { takeable(people, field) } else { None };
    o.take = asked && taken.is_none();
    if taken.is_some() {
        o = Other::default();
    }
    ui.data_mut(|d| d.insert_temp(id, o));
    taken
}
