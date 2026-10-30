//! The From row of a message being written, as Outlook's: the button offers each account's own
//! address, the mailboxes it opens (a shared one) and the other addresses it sent from, and
//! another address typed in, or found by a name typed (among the people known here and in the
//! organisation's directory).

use quatrelle_engine::{Compose, Sender};

use super::*;

/// The most people offered under the other address.
const MAX_CANDIDATES: usize = 10;

/// Who the other address typed may be: the people known here it begins the name or address of,
/// and those the directory has for it.
#[derive(Default)]
pub(super) struct Candidates {
    pub people: Vec<EmailAddress>,
    /// The directory is still being asked.
    pub searching: bool,
}

fn state_id(compose: u64) -> egui::Id {
    egui::Id::unique(("compose-other-sender", compose))
}

/// The box the other address is typed in (Ctrl+K there checks its name, not the message's).
pub(super) fn box_id(compose: u64) -> egui::Id {
    egui::Id::unique(("compose-other-sender-box", compose))
}

/// The one address `text` holds, if it holds just one.
fn one_address(text: &str) -> Option<EmailAddress> {
    addr::parse_list(text.trim()).ok().filter(|l| l.len() == 1).and_then(|l| l.into_iter().next())
}

/// [`Candidates`] for the other address being typed in the message `compose`; the directory of
/// its account (or the first Exchange one) is asked as it's typed, from three characters.
pub(super) fn candidates(s: &mut Session, ctx: &egui::Context, compose: u64, account: &str) -> Candidates {
    let (open, typed, _): (bool, String, bool) = ctx.data_mut(|d| d.get_temp(state_id(compose))).unwrap_or_default();
    let typed = typed.trim();
    if !open || typed.chars().count() < 2 || one_address(typed).is_some() {
        return Candidates::default();
    }
    let mut people: Vec<EmailAddress> = s.suggest_contacts_now(typed, 6).into_iter().map(|c| c.address).collect();
    let mut searching = false;
    if typed.chars().count() >= 3
        && let Some(gal) = s.directory_account(Some(account))
    {
        s.search_directory(&gal, typed, true);
        searching = s.directory_searching(&gal, typed);
        for p in s.directory_found(&gal, typed).map(|f| quatrelle_engine::directory_people(&f.people)).unwrap_or_default() {
            // Known here by its address alone: the directory's full name for it.
            match people.iter_mut().find(|x| x.email.eq_ignore_ascii_case(&p.email)) {
                Some(known) => quatrelle_engine::fill_name(known, &p),
                None => people.push(p),
            }
        }
    }
    people.truncate(MAX_CANDIDATES);
    Candidates { people, searching }
}

/// The From button and who the message is from beside it.
pub(super) fn from_row(ui: &mut Ui, t: &Tokens, c: &mut Compose, senders: &[Sender], cands: &Candidates) {
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
        if let Some(from) = other_address(ui, t, c.id, cands) {
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
/// its button once it's one address; a name typed offers the people it may be (Enter takes the
/// only one, Ctrl+K writes them out in the box).
fn other_address(ui: &mut Ui, t: &Tokens, compose: u64, cands: &Candidates) -> Option<EmailAddress> {
    let id = state_id(compose);
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
    let found = one_address(&typed);
    let only = match cands.people.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    };
    let mut taken = None;
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut typed).id(box_id(compose)).hint_text(tl!("Name or address")).desired_width(260.0));
        if std::mem::take(&mut focus) {
            r.request_focus();
        }
        // In a menu the box keeps the focus on Enter: Enter while typing in it.
        let enter = (r.has_focus() || r.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.add_enabled(found.is_some(), egui::Button::new(tl!("Send from it"))).clicked() || enter {
            taken = found.clone().or_else(|| only.clone().filter(|_| enter));
        }
        // Ctrl+K: the one person the name fits, written out.
        if r.has_focus()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K))
            && let Some(one) = &only
        {
            typed = one.to_string();
        }
    });
    for p in &cands.people {
        if ui.selectable_label(false, p.to_string()).clicked() {
            taken = Some(p.clone());
        }
    }
    if cands.searching {
        ui.label(RichText::new(tl!("Searching the directory…")).color(t.dim).small());
    } else if !typed.trim().is_empty() && found.is_none() && cands.people.is_empty() {
        ui.label(RichText::new(tf!("Not an address: {what}", what = typed.trim())).color(t.danger).small());
    }
    if taken.is_some() {
        (open, typed) = (false, String::new());
    }
    ui.data_mut(|d| d.insert_temp(id, (open, typed, focus)));
    taken
}
