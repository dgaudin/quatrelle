//! The Room Finder: the rooms the account's server offers (its room lists, as Outlook's Room
//! Finder lists them), or those its calendar knows where the server has no lists set up, to
//! book one for the meeting; and any room by its address.

use quatrelle_engine::Fetched;

use super::*;

/// The Room Finder's state while it's open.
#[derive(Clone, Debug, Default)]
pub struct RoomFinderUi {
    /// What's typed to narrow the rooms.
    pub filter: String,
    /// The room list chosen (its address); `None`: every room.
    pub list: Option<String>,
    /// An address typed in by hand, for a room no list offers.
    pub address: String,
}

/// The rooms row: each room booked with its ×, and the Room Finder's button.
pub(super) fn row(ui: &mut Ui, f: &mut EventForm, t: &Tokens) {
    ui.horizontal_wrapped(|ui| {
        let mut remove = None;
        for (name, email) in &f.rooms {
            egui::Frame::new().fill(t.accent_soft).corner_radius(10.0).inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
                ui.label(RichText::new(name).color(t.text)).on_hover_text(email);
                if widgets::tiny_icon_button(ui, Icon::Close, "Remove", t).clicked() {
                    remove = Some(email.clone());
                }
            });
        }
        if let Some(e) = remove {
            f.remove_room(&e);
        }
        let open = f.finder.is_some();
        if ui.selectable_label(open, tl!("Room Finder")).clicked() {
            f.finder = if open { None } else { Some(RoomFinderUi::default()) };
        }
    });
}

/// The Room Finder under its row: the list to choose from (when the server has lists), a
/// filter, the rooms (ticked when booked: a click books or frees one), and an address for a
/// room no list offers. The rooms are asked for once, the first time it opens.
pub(super) fn finder(ui: &mut Ui, f: &mut EventForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) {
    let account = f.account_id();
    let known = s.rooms_now(&account);
    if known.is_none() {
        acts.push(Action::Run("calendar.rooms", json!({"account": account})));
    }
    let mut st = f.finder.take().unwrap_or_default();
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6.0).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        match known {
            None | Some(Fetched::Loading) => {
                ui.label(RichText::new(tl!("Looking for rooms…")).color(t.dim));
            }
            Some(Fetched::Failed(e)) => {
                ui.label(RichText::new(tf!("The rooms couldn't be read: {error}", error = e)).color(t.danger));
                if ui.small_button(tl!("Try again")).clicked() {
                    acts.push(Action::Run("calendar.rooms", json!({"account": account, "refresh": true})));
                }
            }
            Some(Fetched::Ready(finder)) => {
                ui.horizontal(|ui| {
                    if !finder.lists.is_empty() {
                        let shown = st
                            .list
                            .as_ref()
                            .and_then(|l| finder.lists.iter().find(|x| x.email == *l))
                            .map_or_else(|| tl!("All rooms").to_string(), |l| l.name.clone());
                        egui::ComboBox::from_id_salt("room-list").width(180.0).selected_text(shown).show_ui(ui, |ui| {
                            ui.selectable_value(&mut st.list, None, tl!("All rooms"));
                            for l in &finder.lists {
                                ui.selectable_value(&mut st.list, Some(l.email.clone()), &l.name);
                            }
                        });
                    }
                    field(ui, &mut st.filter, tl!("Filter rooms"), f32::INFINITY);
                });
                ui.add_space(4.0);
                let q = st.filter.trim().to_lowercase();
                let chosen = st.list.as_ref().and_then(|l| finder.lists.iter().find(|x| x.email == *l));
                let rooms: Vec<&quatrelle_engine::model::scheduling::Room> = finder
                    .rooms
                    .iter()
                    .filter(|r| chosen.is_none_or(|l| r.list == l.name || r.list == l.email))
                    .filter(|r| q.is_empty() || r.name.to_lowercase().contains(&q) || r.email.contains(&q) || r.list.to_lowercase().contains(&q))
                    .collect();
                if finder.rooms.is_empty() {
                    ui.label(RichText::new(tl!("No rooms are offered here yet: a room booked in one of your meetings shows here.")).color(t.dim));
                } else if rooms.is_empty() {
                    ui.label(RichText::new(tl!("No room matches")).color(t.dim));
                } else {
                    egui::ScrollArea::vertical().id_salt("room-finder").max_height(160.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for r in rooms {
                            let booked = f.rooms.iter().any(|(_, e)| *e == r.email);
                            let label = if r.list.is_empty() || chosen.is_some() { r.name.clone() } else { format!("{} · {}", r.name, r.list) };
                            if ui.selectable_label(booked, label).on_hover_text(&r.email).clicked() {
                                if booked {
                                    f.remove_room(&r.email);
                                } else {
                                    f.add_room(&r.name, &r.email);
                                }
                            }
                        }
                    });
                    if finder.from_calendar {
                        ui.label(
                            RichText::new(tl!("From your calendar: the rooms booked in your meetings and those in Outlook's Rooms group."))
                                .color(t.faint)
                                .small(),
                        );
                    }
                }
            }
        }
        // A room no list offers, by its address.
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let r = field(ui, &mut st.address, tl!("A room's address"), 280.0);
            let ok = st.address.contains('@');
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.add_enabled(ok, egui::Button::new(tl!("Add"))).clicked() || (enter && ok))
                && let Some(a) = quatrelle_engine::model::addr::parse_list(&st.address).ok().and_then(|l| l.into_iter().next())
            {
                f.add_room(&a.name, &a.email);
                st.address.clear();
            }
        });
    });
    f.finder = Some(st);
}
