//! The File page: About Quatrelle, and checking for updates.

use super::*;

/// Check for updates, and what it found: a newer release to download (its installer for this
/// system, else its page) and what's new in it. Nothing installs by itself.
fn updates(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    use quatrelle_engine::UpdateCheck;
    if s.jmap_only {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        let checking = s.update == UpdateCheck::Checking;
        if ui.add_enabled(!checking, egui::Button::new(tl!("Check for updates"))).clicked() {
            acts.push(Action::Run("app.checkUpdates", json!({})));
        }
        match &s.update {
            UpdateCheck::NotAsked => {}
            UpdateCheck::Checking => {
                ui.spinner();
                ui.label(RichText::new(tl!("Checking…")).color(t.dim));
            }
            UpdateCheck::UpToDate(_) => {
                ui.label(RichText::new(tl!("You have the newest version.")).color(t.dim));
            }
            UpdateCheck::Available(r) => {
                let name = r.tag.trim_start_matches('v').to_string();
                ui.label(RichText::new(tf!("Quatrelle {version} is available.", version = name)).color(t.text).strong());
                let target = if r.download.is_empty() { r.page.clone() } else { r.download.clone() };
                if widgets::accent_button(ui, Icon::Sync, "Download", t, true).on_hover_text(&target).clicked() {
                    acts.push(Action::OpenUrl(target));
                }
                if ui.link(tl!("What's new")).on_hover_text(&r.page).clicked() {
                    acts.push(Action::OpenUrl(r.page.clone()));
                }
            }
            UpdateCheck::Failed(e) => {
                ui.label(RichText::new(tf!("Couldn't check for updates: {error}", error = e)).color(t.danger));
            }
        }
    });
}

pub(super) fn about(s: &Session, t: &Tokens, update: &crate::UpdateState, ui: &mut Ui, acts: &mut Vec<Action>) {
    heading(ui, t, "About Quatrelle");
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
        ui.painter().rect_filled(r, 12.0, t.accent);
        icons::paint(ui.painter(), icons::centered(r, 34.0), Icon::Unread, egui::Color32::WHITE);
        ui.add_space(10.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(tf!("Quatrelle {version}", version = env!("CARGO_PKG_VERSION"))).font(theme::bold(18.0)).color(t.text));
            ui.label(RichText::new(tl!("An ArtCraft app · MIT OR Apache-2.0")).color(t.dim));
        });
    });
    ui.add_space(10.0);
    updates(s, t, ui, acts);
    // A test build that updates itself: its next version on the way, or downloaded.
    match update {
        crate::UpdateState::Ready { version, .. } => {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(tf!("Quatrelle {version} is downloaded. It installs when you close Quatrelle.", version = version)).color(t.text),
                );
                if widgets::accent_button(ui, Icon::Sync, "Restart to update", t, true).clicked() {
                    acts.push(Action::RestartToUpdate);
                }
            });
        }
        crate::UpdateState::Downloading { version, done, total } => {
            ui.add_space(6.0);
            let percent = if *total > 0 { (done.saturating_mul(100) / total).min(100) } else { 0 };
            ui.label(RichText::new(tf!("Downloading Quatrelle {version}… {percent}%", version = version, percent = percent)).color(t.dim));
        }
        crate::UpdateState::None => {}
    }
    ui.add_space(14.0);
    for line in [
        "Local-first: mail is cached on this computer for fast offline search. Passwords stay in the system keychain.",
        "Quatrelle talks only to your own mail servers, over encrypted connections. Pictures from the web load only when you ask.",
        "Built in the open from the email standards: no code, icons or assets from other mail apps.",
    ] {
        ui.label(RichText::new(tl!(line)).color(t.dim));
        ui.add_space(4.0);
    }
    ui.add_space(8.0);
    if ui.link(tl!("Source code and issues")).clicked() {
        acts.push(Action::OpenUrl("https://github.com/dgaudin/quatrelle".into()));
    }
    // Fonts built in from craft-fonts travel with their licences.
    if !crate::craft_fonts::CRAFT_FONTS.is_empty() {
        ui.add_space(12.0);
        ui.label(RichText::new(tl!("Fonts")).font(theme::bold(14.0)).color(t.text));
        for f in crate::craft_fonts::CRAFT_FONTS {
            egui::CollapsingHeader::new(RichText::new(format!("{} {} · {}", f.family, f.style, f.licence)).color(t.dim))
                .id_salt(("craft-font", f.family, f.style))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(220.0).id_salt(("craft-font-licence", f.family, f.style)).show(ui, |ui| {
                        ui.label(RichText::new(f.licence_text).small().color(t.dim));
                    });
                });
        }
    }
}
