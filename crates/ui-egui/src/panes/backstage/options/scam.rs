// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The File page: Options › Assistant › Scam check. The box, the accounts it applies to, where the
//! mail goes, the day's cap, the senders taken as safe, and the test on examples.

use quatrelle_engine::model::addr::trust::url_host;
use quatrelle_engine::model::scam::EXAMPLES;

use super::*;

/// The card under the assistant's server and model.
pub(super) fn scam(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    let have_model = !s.prefs.llm_url.trim().is_empty() && !s.prefs.llm_model.trim().is_empty();
    card(
        ui,
        t,
        tl!("Scam check"),
        Some(tl!("A language model reads the external mail that arrives and warns about what looks like a scam.")),
        |ui| {
            let about = (!have_model).then(|| note(t, tl!("Set a server and a model above first.")));
            setting_row(ui, t, true, name(t, tl!("Spot scams in the external mail that arrives")), about, |ui| {
                let mut on = s.prefs.scam_check;
                ui.add_enabled_ui(have_model || on, |ui| {
                    if widgets::switch(ui, t, &mut on, tl!("Spot scams in the external mail that arrives")).changed() {
                        acts.push(Action::Run("assistant.scamCheck", json!({"enabled": on})));
                    }
                });
            });
            if !have_model {
                return;
            }
            if let Some(host) = url_host(&s.prefs.llm_url) {
                ui.label(RichText::new(tf!("External mail from unknown senders is sent to {host} to be read.", host = host)).color(t.dim).size(12.5));
                ui.add_space(4.0);
            }
            ui.label(name(t, tl!("Accounts it applies to")));
            for a in &s.accounts {
                let id = a.config.id.clone();
                let exchange = s.is_exchange(&id);
                let mut ticked = exchange && s.prefs.scam_accounts.contains(&id);
                ui.horizontal(|ui| {
                    if ui.add_enabled(exchange, egui::Checkbox::new(&mut ticked, a.config.email.as_str())).changed() {
                        let mut list: Vec<String> = s.prefs.scam_accounts.iter().filter(|x| **x != id).cloned().collect();
                        if ticked {
                            list.push(id.clone());
                        }
                        acts.push(Action::Run("assistant.scamCheck", json!({"accounts": list})));
                    }
                    if !exchange {
                        ui.label(note(t, tl!("not for this kind of account yet")));
                    }
                });
            }
            ui.add_space(4.0);
            let cap = s.prefs.scam_daily_cap;
            // What it did today, from the verdicts kept: whether the ceiling is far or near.
            let today = s.scam_today();
            let done = note(t, &tf!("Today: {judged} messages checked, {warned} flagged", judged = today.judged, warned = today.warned));
            setting_row(ui, t, false, name(t, &tf!("At most {n} messages a day", n = cap)), Some(done), |ui| {
                let id = egui::Id::unique(("options-scam", "cap"));
                let mut shown: u32 = ui.data_mut(|d| d.get_temp::<u32>(id)).unwrap_or(cap);
                let r = ui.add(egui::DragValue::new(&mut shown).range(1..=2000));
                let mut send = None;
                if r.changed() {
                    ui.data_mut(|d| d.insert_temp(id, shown));
                    if !r.dragged() {
                        send = Some(shown);
                    }
                } else if r.drag_stopped() {
                    send = ui.data_mut(|d| d.get_temp::<u32>(id));
                }
                if let Some(n) = send {
                    ui.data_mut(|d| d.remove_temp::<u32>(id));
                    if n != cap {
                        acts.push(Action::Run("assistant.scamCheck", json!({"dailyCap": n})));
                    }
                }
            });
            let trusted = s.scam_trusted_all();
            if !trusted.is_empty() {
                ui.add_space(4.0);
                ui.label(name(t, tl!("Senders you said are not scams")));
                for address in trusted {
                    ui.horizontal(|ui| {
                        ui.label(address.as_str());
                        if ui.button(RichText::new(tl!("Remove")).color(t.danger)).clicked() {
                            acts.push(Action::Run("scam.untrust", json!({"address": address})));
                        }
                    });
                }
            }
            trial(s, t, ui, acts);
        },
    );
}

/// The button that tries the check on the twenty examples, and what it found.
fn trial(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    let running = s.scam_test().is_some_and(|r| r.running);
    ui.add_space(4.0);
    let about = note(t, &tf!("{n} messages written for Quatrelle go to the model one at a time; nothing of yours is sent.", n = EXAMPLES.len()));
    setting_row(ui, t, false, name(t, tl!("Test the model")), Some(about), |ui| {
        if ui.add_enabled(!running, egui::Button::new(tl!("Try it on examples"))).clicked() {
            acts.push(Action::Run("assistant.testScam", json!({})));
        }
    });
    let Some(r) = s.scam_test() else { return };
    if r.running {
        ui.label(tf!("Trying it on the examples: {done} of {total}", done = r.done, total = r.total));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        return;
    }
    ui.label(tf!(
        "{spotted} of {scams} scams spotted, {left} of {normals} normal messages left alone, {secs} s a message",
        spotted = r.spotted,
        scams = r.scams,
        left = r.left_alone,
        normals = r.normals,
        secs = r.secs_per_message()
    ));
    // Each one on its line, with the score the model gave: how far over (or under) the line it was.
    for m in &r.misses {
        ui.label(match (m.score, m.scam) {
            (Some(score), false) => tf!("Flagged by mistake: {subject} (score {score})", subject = m.subject, score = score),
            (Some(score), true) => tf!("Scam not spotted: {subject} (score {score})", subject = m.subject, score = score),
            (None, _) => tf!("No answer: {subject}", subject = m.subject),
        });
    }
}
