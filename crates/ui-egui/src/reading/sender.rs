//! The From row of a message being written, as Outlook's: the button offers each account's own
//! address, the mailboxes it opens (a shared one) and the other addresses it sent from, and
//! another address typed in.

use quatrelle_engine::{Compose, Sender};

use super::*;

/// The From button and who the message is from beside it.
pub(super) fn from_row(ui: &mut Ui, t: &Tokens, c: &mut Compose, senders: &[Sender]) {
    let own = senders.iter().find(|x| x.account == c.account && x.from.is_empty()).map(|x| x.label.clone()).unwrap_or_default();
    let b = label_button_menu(ui, t, "From", true, true).on_hover_text(tl!("Send from another account, a shared mailbox or another address"));
    egui::Popup::menu(&b).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        for x in senders {
            let on = x.account == c.account && x.from.eq_ignore_ascii_case(&c.from);
            if ui.selectable_label(on, &x.label).clicked() {
                c.account = x.account.clone();
                c.from = x.from.clone();
                ui.close();
            }
        }
        ui.separator();
        if let Some(from) = other_address(ui, t, c.id) {
            let mine = senders.iter().any(|x| x.account == c.account && x.from.is_empty() && x.email.eq_ignore_ascii_case(&from.email));
            c.from = if mine { String::new() } else { from.to_string() };
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

/// Outlook's Other E-mail Address: an address typed under the menu's list, taken with Enter or
/// its button once it's one address.
fn other_address(ui: &mut Ui, t: &Tokens, compose: u64) -> Option<EmailAddress> {
    let id = egui::Id::unique(("compose-other-sender", compose));
    // Open, what's typed, and the box to take the keyboard (it's just been opened).
    let (mut open, mut typed, mut focus): (bool, String, bool) = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    if !open {
        if ui.selectable_label(false, tl!("Other email address…")).clicked() {
            (open, focus) = (true, true);
        }
        ui.data_mut(|d| d.insert_temp(id, (open, typed, focus)));
        return None;
    }
    ui.label(RichText::new(tl!("Other email address")).font(theme::bold(13.0)).color(t.text));
    ui.label(RichText::new(tl!("Exchange sends from it when your account may send as it or on its behalf.")).color(t.dim).small());
    let found = addr::parse_list(typed.trim()).ok().filter(|l| l.len() == 1).and_then(|l| l.into_iter().next());
    let mut taken = None;
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut typed).hint_text(tl!("Name <address>")).desired_width(260.0));
        if std::mem::take(&mut focus) {
            r.request_focus();
        }
        // In a menu the box keeps the focus on Enter: Enter while typing in it.
        let enter = (r.has_focus() || r.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.add_enabled(found.is_some(), egui::Button::new(tl!("Send from it"))).clicked() || enter {
            taken = found.clone();
        }
    });
    if !typed.trim().is_empty() && found.is_none() {
        ui.label(RichText::new(tf!("Not an address: {what}", what = typed.trim())).color(t.danger).small());
    }
    if taken.is_some() {
        (open, typed) = (false, String::new());
    }
    ui.data_mut(|d| d.insert_temp(id, (open, typed, focus)));
    taken
}
