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
    /// The room whose day is shown under the list (name, address): the one clicked.
    pub shown: Option<(String, String)>,
}

/// A Rooms row's content: `line` (the rooms) in the room the Room Finder's toggle leaves it, the
/// toggle at the row's end, on the line's first line. The toggle is laid out first, as wide as
/// its label reads in the language shown, so the line never runs under it.
pub(super) fn with_finder_toggle(ui: &mut Ui, f: &mut EventForm, line: impl FnOnce(&mut Ui)) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
            ui.add_space(((FIELD_H - ui.spacing().interact_size.y) / 2.0).max(0.0));
            let open = f.finder.is_some();
            if ui.selectable_label(open, tl!("Room Finder")).clicked() {
                f.finder = if open { None } else { Some(RoomFinderUi::default()) };
            }
        });
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), line);
    });
}

/// The Room Finder under its row: the list to choose from (when the server has lists), a
/// filter, the rooms (ticked when booked), and an address for a room no list offers. A click on
/// a room shows its day under the list, the meeting over it, with a button to book it; a
/// double-click books it (or frees it) at once. The rooms are asked for once, the first time it
/// opens.
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
                            let shown = st.shown.as_ref().is_some_and(|(_, e)| *e == r.email);
                            let label = if r.list.is_empty() || chosen.is_some() { r.name.clone() } else { format!("{} · {}", r.name, r.list) };
                            let row = room_row(ui, t, room_row_id(f.window, &r.email), &label, booked, shown).on_hover_text(&r.email);
                            if row.clicked() {
                                st.shown = Some((r.name.clone(), r.email.clone()));
                            }
                            if row.double_clicked() {
                                toggle(f, &r.name, &r.email);
                            }
                        }
                    });
                    ui.label(RichText::new(tl!("Click a room to see when it's free; double-click it to book it.")).color(t.faint).small());
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
        // The room clicked: its day, and its button.
        if let Some((name, email)) = st.shown.clone() {
            ui.add_space(6.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new(&name).font(theme::bold(13.0)).color(t.text));
                ui.label(RichText::new(&email).color(t.dim).small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::tiny_icon_button(ui, Icon::Close, "Hide its day", t).clicked() {
                        st.shown = None;
                    }
                    let booked = f.rooms.iter().any(|(_, e)| *e == email);
                    let label = if booked { "Remove this room" } else { "Book this room" };
                    if ui.button(tl!(label)).clicked() {
                        toggle(f, &name, &email);
                    }
                });
            });
            super::scheduling::room_day(ui, f, s, t, acts, &name, &email);
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

/// Book a room, or free it when it's booked.
fn toggle(f: &mut EventForm, name: &str, email: &str) {
    if f.rooms.iter().any(|(_, e)| e.eq_ignore_ascii_case(email)) {
        f.remove_room(email);
    } else {
        f.add_room(name, email);
    }
}

/// A room's row in the Room Finder of the window numbered `window`: the tests find it by it.
pub fn room_row_id(window: u64, email: &str) -> egui::Id {
    egui::Id::unique(("room-finder-room", window, email.to_ascii_lowercase()))
}

/// A room in the list (`id`): ticked when booked, highlighted when its day is the one shown.
fn room_row(ui: &mut Ui, t: &Tokens, id: egui::Id, label: &str, booked: bool, shown: bool) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let resp = ui.interact(rect, id, Sense::click());
    if shown || resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, if shown { t.accent_soft } else { t.hover_row });
    }
    let tick = egui::Rect::from_center_size(egui::pos2(rect.min.x + 13.0, rect.center().y), vec2(16.0, 16.0));
    if booked {
        icons::paint(ui.painter(), tick, Icon::Complete, t.accent);
    }
    let text = widgets::one_line(ui, label, theme::regular(13.0), t.text, rect.width() - 34.0);
    ui.painter().galley(egui::pos2(rect.min.x + 28.0, rect.center().y - text.size().y / 2.0), text, t.text);
    resp
}
