// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The commands asked for at start-up (`--cmd <id>[=<json>]`): engine commands, and the `ui.*`
//! ones that open a page or a dialog of the window (see the usage in `main.rs`).

use quatrelle_ui_egui::MailApp;
use quatrelle_ui_egui::backstage::{Backstage, Page};

use crate::say;

/// Run each `--cmd` of `args`, in order.
pub(crate) fn run_start_commands(app: &mut MailApp, args: &[String]) {
    for c in args.windows(2).filter(|w| w.first().is_some_and(|a| a == "--cmd")).filter_map(|w| w.get(1)) {
        let (id, params) = c.split_once('=').unwrap_or((c.as_str(), "{}"));
        match id {
            "ui.addAccount" => {
                let p: serde_json::Value = serde_json::from_str(params).unwrap_or_default();
                let mut d = quatrelle_ui_egui::Dialog::add_account_with(
                    p.get("email").and_then(|v| v.as_str()).unwrap_or_default(),
                    p.get("advanced").and_then(|v| v.as_bool()).unwrap_or(false),
                );
                if let quatrelle_ui_egui::Dialog::AddAccount(f) = &mut d {
                    f.graph = p.get("graph").and_then(|v| v.as_bool()).unwrap_or(false);
                    f.pop = p.get("pop").and_then(|v| v.as_bool()).unwrap_or(false);
                }
                app.ui.dialog = Some(d);
            }
            "ui.file" | "ui.settings" => app.ui.backstage = Some(Backstage::new(Page::Info)),
            "ui.options" => app.ui.backstage = Some(Backstage::new(Page::Options)),
            "ui.openExport" => app.ui.backstage = Some(Backstage::new(Page::Open)),
            "ui.about" => app.ui.backstage = Some(Backstage::new(Page::About)),
            // Optionally `={"at":"YYYY-MM-DDTHH:MM","summary":…,"guests":"a@x, …","rooms":"Room <r@x>, …","scheduling":true,"finder":true,"room":"r@x" (its day shown in the Room Finder),"repeat":RRULE,"recurrence":true,"attendees":true (Select Attendees and Resources open)}`.
            "ui.newEvent" => {
                let p: serde_json::Value = serde_json::from_str(params).unwrap_or_default();
                let text = |k: &str| p.get(k).and_then(|v| v.as_str()).map(str::to_string);
                let at = text("at").and_then(|x| quatrelle_engine::parse_local(&x, &app.session.clock)).map(|(t, _)| t);
                let mut d = quatrelle_ui_egui::Dialog::new_event(&app.session, at);
                if let quatrelle_ui_egui::Dialog::Event(f) = &mut d {
                    if let Some(s) = text("summary") {
                        f.summary = s;
                    }
                    if let Some(g) = text("guests") {
                        f.guests = g;
                    }
                    for r in quatrelle_engine::model::addr::parse_list(&text("rooms").unwrap_or_default()).unwrap_or_default() {
                        f.add_room(&r.name, &r.email);
                    }
                    f.scheduling = p.get("scheduling").and_then(|v| v.as_bool()).unwrap_or(false);
                    if p.get("finder").and_then(|v| v.as_bool()).unwrap_or(false) {
                        f.finder = Some(Default::default());
                    }
                    if let Some(r) = text("room") {
                        let shown = (r.split('@').next().unwrap_or_default().to_string(), r);
                        f.finder.get_or_insert_with(Default::default).shown = Some(shown);
                    }
                    if let Some(r) = text("repeat") {
                        f.repeat = r;
                    }
                    if p.get("recurrence").and_then(|v| v.as_bool()).unwrap_or(false) {
                        f.open_recurrence();
                    }
                    if p.get("attendees").and_then(|v| v.as_bool()).unwrap_or(false) {
                        f.inviting = true;
                        f.select_attendees();
                    }
                }
                app.ui.dialog = Some(d);
            }
            "ui.newContact" => app.ui.dialog = Some(quatrelle_ui_egui::Dialog::new_contact()),
            "ui.calendarSearch" => {
                let p: serde_json::Value = serde_json::from_str(params).unwrap_or_default();
                let when = p.get("when").and_then(|v| v.as_str()).and_then(quatrelle_engine::EventWhen::from_code).unwrap_or_default();
                app.search_calendar(p.get("query").and_then(|v| v.as_str()).unwrap_or_default(), when);
            }
            "ui.folderSize" => {
                let p: serde_json::Value = serde_json::from_str(params).unwrap_or_default();
                let first = app.session.accounts.first().map(|a| a.config.id.clone());
                if let Some(id) = p.get("account").and_then(|v| v.as_str()).map(str::to_string).or(first) {
                    app.run("account.folderSizes", serde_json::json!({"account": id}));
                    app.ui.backstage = Some(Backstage::new(Page::Info));
                    app.ui.dialog = Some(quatrelle_ui_egui::Dialog::folder_size(&app.session, &id));
                }
            }
            "ui.rules" => app.ui.dialog = Some(quatrelle_ui_egui::Dialog::Rules),
            // Optionally `={"query":…}`.
            "ui.find" => {
                let p: serde_json::Value = serde_json::from_str(params).unwrap_or_default();
                app.find_in_message(p.get("query").and_then(|v| v.as_str()));
            }
            _ => match serde_json::from_str(params) {
                Ok(p) => app.run(id, p),
                Err(e) => say(&format!("--cmd {id}: bad JSON: {e}")),
            },
        }
    }
}
