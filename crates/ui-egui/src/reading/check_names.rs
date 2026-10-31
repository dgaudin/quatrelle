//! Check Names' choice, as Outlook's: several people fit a name typed in a contacts field (To,
//! Cc, Bcc, a dialog's row…), and the one meant is picked from them (or the name is left as typed).

use quatrelle_engine::NameChoice;

use super::*;

/// What the choice dialog came to this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Still asked.
    Pending,
    /// A person picked (`people.choose` runs).
    Picked,
    /// The name left as typed (Cancel, Escape).
    Cancelled,
}

/// The dialog over the field's window while a name waits for its person: the choice is answered
/// to its field (`people.choose`).
pub(crate) fn choice_dialog(ctx: &egui::Context, t: &Tokens, choice: &NameChoice, acts: &mut Vec<Action>) -> Answer {
    let id = egui::Id::unique(("check-names", choice.field.window, choice.field.name));
    let picked_id = id.with(("picked", choice.token.as_str()));
    let mut picked: Option<usize> = ctx.data(|d| d.get_temp(picked_id)).flatten();
    // `Some(None)`: left as typed.
    let mut done: Option<Option<usize>> = None;
    egui::Modal::new(id).frame(crate::dialogs::dialog_frame(t)).show(ctx, |ui| {
        ui.set_width(540.0);
        ui.label(RichText::new(tl!("Check Names")).font(theme::bold(18.0)).color(t.text));
        ui.add_space(4.0);
        ui.label(RichText::new(tf!("More than one person fits “{name}”. Choose who you meant:", name = choice.token)).color(t.text));
        ui.add_space(6.0);
        egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(4).inner_margin(Margin::same(2)).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt(id.with("people")).max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
                for (i, p) in choice.people.iter().enumerate() {
                    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                    let on = picked == Some(i);
                    if on || r.hovered() {
                        ui.painter().rect_filled(rect, 3.0, if on { t.accent_soft } else { t.hover_row });
                    }
                    let name = p.display();
                    let mid = rect.center().y;
                    let name_clip = Rect::from_min_max(rect.min, egui::pos2(rect.left() + 240.0, rect.bottom()));
                    ui.painter().with_clip_rect(name_clip).text(
                        egui::pos2(rect.left() + 8.0, mid),
                        egui::Align2::LEFT_CENTER,
                        name,
                        theme::regular(13.5),
                        t.text,
                    );
                    ui.painter().text(egui::pos2(rect.left() + 250.0, mid), egui::Align2::LEFT_CENTER, &p.address, theme::regular(13.0), t.dim);
                    if r.clicked() {
                        picked = Some(i);
                    }
                    if r.double_clicked() {
                        done = Some(Some(i));
                    }
                }
            });
        });
        let (ok, cancel) = crate::dialogs::buttons(ui, t, "OK", picked.is_some());
        if ok && picked.is_some() {
            done = Some(picked);
        } else if cancel {
            done = Some(None);
        }
    });
    if done.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        done = Some(None);
    }
    ctx.data_mut(|d| d.insert_temp(picked_id, picked));
    let field = json!({"window": choice.field.window, "field": choice.field.name, "token": choice.token});
    match done {
        Some(Some(i)) => {
            if let Some(p) = choice.people.get(i) {
                let mut person = field;
                person["name"] = json!(p.name);
                person["kind"] = json!(p.kind.code());
                // A contact group has no address: it's chosen by its item id.
                if !p.address.is_empty() {
                    person["email"] = json!(p.address);
                }
                if !p.item_id.is_empty() {
                    person["itemId"] = json!(p.item_id);
                }
                acts.push(Action::Run("people.choose", person));
            }
            Answer::Picked
        }
        Some(None) => {
            acts.push(Action::Run("people.choose", field));
            Answer::Cancelled
        }
        None => Answer::Pending,
    }
}
