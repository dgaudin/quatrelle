//! The File page: Options › Assistant, the language model asked about mail.

use super::*;

/// How tall the list of models gets before it scrolls.
const LIST_HEIGHT: f32 = 300.0;
/// The model's row: as wide as the boxes above it, its two buttons included.
const WIDTH: f32 = 320.0;
const BUTTON: f32 = 28.0;
const GAP: f32 = 4.0;

/// An OpenAI-compatible server (on this computer, the organisation's, or a service), its key,
/// then the model: picked from the ones the server lists, or typed. Each field is saved once
/// it's left; the key goes to the keychain and is never shown again.
pub(super) fn assistant(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    // The server's models, asked for once when the page shows (again when its address or key
    // changes, by the engine).
    let url = s.prefs.llm_url.trim();
    if !url.is_empty() && s.llm_models.as_ref().is_none_or(|m| m.url != url) {
        acts.push(Action::Run("assistant.models", json!({})));
    }
    card(
        ui,
        t,
        tl!("Assistant"),
        Some(tl!("Summarize a message with a language model. The text of the messages you ask about is sent to this server: use one you trust.")),
        |ui| {
            for (first, field, label, hint) in
                [(true, "url", "Server address", "http://localhost:11434/v1"), (false, "key", "Key (if the server needs one)", "")]
            {
                setting_row(ui, t, first, name(t, tl!(label)), None, |ui| {
                    let id = egui::Id::unique(("options-llm", field));
                    let current = if field == "url" { s.prefs.llm_url.clone() } else { String::new() };
                    let mut text: String = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| current.clone());
                    let r = ui.add(egui::TextEdit::singleline(&mut text).hint_text(hint).password(field == "key").desired_width(320.0));
                    if r.lost_focus() && text.trim() != current {
                        let mut p = json!({"url": s.prefs.llm_url, "model": s.prefs.llm_model});
                        p[field] = json!(text.trim());
                        acts.push(Action::Run("assistant.settings", p));
                        if field == "key" {
                            text.clear();
                        }
                    }
                    ui.data_mut(|d| d.insert_temp(id, text));
                });
            }
            let status = s.llm_models.as_ref().filter(|m| m.url == url).map(|m| match (&m.error, m.asking) {
                (_, true) => tl!("Looking for the server's models…").to_string(),
                (Some(e), _) => tf!("The models couldn't be listed: {error}", error = e),
                (None, _) => tf!("Models on this server: {n}", n = m.names.len()),
            });
            setting_row(ui, t, false, name(t, tl!("Model")), status.as_deref().map(|a| note(t, a)), |ui| model(s, t, ui, acts));
        },
    );
}

/// The model's box: typed in, or picked from the server's list under its arrow; the button
/// beside asks the server again.
fn model(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    let id = egui::Id::unique(("options-llm", "model"));
    let current = s.prefs.llm_model.clone();
    let mut text: String = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| current.clone());
    let listed = s.llm_models.as_ref().filter(|m| m.url == s.prefs.llm_url.trim());
    let names = listed.map(|m| m.names.as_slice()).unwrap_or_default();
    let asking = listed.is_some_and(|m| m.asking);
    let mut picked = None;
    // As wide as the other boxes, read left to right (the row's controls are laid out from the
    // right edge).
    let row = vec2(WIDTH, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        let hint = if names.is_empty() { "llama3.1" } else { tl!("Choose or type a model") };
        let side = BUTTON + GAP;
        let field = ui.add(egui::TextEdit::singleline(&mut text).hint_text(hint).desired_width(WIDTH - 2.0 * side));
        if field.lost_focus() && text.trim() != current {
            picked = Some(text.trim().to_string());
        }
        let arrow =
            square_button(ui, t, Icon::ChevronDown, field.rect.height(), !names.is_empty()).on_hover_text(tl!("Choose from the server's models"));
        let under = field.rect.union(arrow.rect);
        egui::Popup::menu(&arrow).anchor(egui::PopupAnchor::ParentRect(under)).show(|ui| {
            ui.set_min_width(under.width() - 2.0 * ui.spacing().menu_margin.leftf());
            egui::ScrollArea::vertical().max_height(LIST_HEIGHT).show(ui, |ui| {
                for n in names {
                    if ui.selectable_label(*n == text.trim(), n).clicked() {
                        picked = Some(n.clone());
                    }
                }
            });
        });
        let again = square_button(ui, t, Icon::Sync, field.rect.height(), !asking && !s.prefs.llm_url.trim().is_empty());
        if again.on_hover_text(tl!("Load the server's models again")).clicked() {
            acts.push(Action::Run("assistant.models", json!({})));
        }
    });
    if let Some(m) = picked {
        text.clone_from(&m);
        if m != current {
            acts.push(Action::Run("assistant.settings", json!({"url": s.prefs.llm_url, "model": m})));
        }
    }
    ui.data_mut(|d| d.insert_temp(id, text));
}

/// A square button as tall as the box beside it, an icon in it.
fn square_button(ui: &mut Ui, t: &Tokens, icon: Icon, height: f32, enabled: bool) -> egui::Response {
    let side = height.clamp(20.0, BUTTON);
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if enabled && resp.hovered() { t.hover } else { t.surface };
    ui.painter().rect(rect, 4.0, fill, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    icons::paint(ui.painter(), icons::centered(rect, 14.0), icon, if enabled { t.dim } else { t.faint });
    resp
}
