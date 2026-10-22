//! Dialogs: categories, rules, Search Folders and Quick Steps.

use super::*;
use quatrelle_engine::model::organize::ForeignRule;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::NewCategory { name, color, error } => {
            ui.set_width(340.0);
            title(ui, t, "New category");
            let r = ui.add(egui::TextEdit::singleline(name).hint_text(tl!("Name")).desired_width(f32::INFINITY));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            ui.horizontal(|ui| {
                for c in 0..8u8 {
                    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
                    if resp.hovered() {
                        ui.painter().circle_filled(rect.center(), 11.0, t.hover);
                    }
                    ui.painter().circle_filled(rect.center(), 8.0, theme::category_color(c, t.dark));
                    if *color == c {
                        ui.painter().circle_stroke(rect.center(), 10.0, egui::Stroke::new(2.0, t.text));
                    }
                    if resp.clicked() {
                        *color = c;
                    }
                }
            });
            if let Some(e) = error {
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            let ready = !name.trim().is_empty();
            let (ok, cancel) = buttons(ui, t, "Create", ready);
            if (ok || enter) && ready {
                *error = None;
                acts.push(Action::Submit("category.create", json!({"name": name.trim(), "color": color})));
            }
            !cancel
        }
        Dialog::Rules => {
            ui.set_width(560.0);
            title(ui, t, "Rules");
            ui.label(RichText::new(tl!("Rules run on new mail as it arrives in an Inbox, in this order.")).color(t.dim).small());
            ui.add_space(6.0);
            // Many rules: a box to find one by its name or what it does.
            let find_id = egui::Id::unique("rules-find");
            let mut find: String = ui.ctx().data(|d| d.get_temp(find_id)).unwrap_or_default();
            if s.rules.len() > 6 {
                ui.add(egui::TextEdit::singleline(&mut find).hint_text(tl!("Find a rule")).desired_width(f32::INFINITY));
                ui.ctx().data_mut(|d| d.insert_temp(find_id, find.clone()));
                ui.add_space(4.0);
            }
            let needle = find.trim().to_lowercase();
            if s.rules.is_empty() {
                ui.label(RichText::new(tl!("No rules yet.")).color(t.faint));
            }
            let mut rules = s.rules.clone();
            let mut changed = false;
            let mut remove = None;
            let mut shown = 0usize;
            // One line a rule (its switch, name, what it does, and its buttons as icons), in a
            // list that scrolls past a dozen or so.
            egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).show(ui, |ui| {
                // With the find box, a steady height, so the dialog doesn't jump while typing.
                let steady = s.rules.len() > 6;
                egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, !steady]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (i, r) in rules.iter_mut().enumerate() {
                        let what = describe(s, r);
                        if !needle.is_empty() && !r.name.to_lowercase().contains(&needle) && !what.to_lowercase().contains(&needle) {
                            continue;
                        }
                        let (row, hover) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
                        if hover.hovered() {
                            ui.painter().rect_filled(row, 0.0, t.hover_row);
                        } else if shown % 2 == 1 {
                            ui.painter().rect_filled(row, 0.0, t.chrome);
                        }
                        shown += 1;
                        let mut inner = ui.new_child(
                            egui::UiBuilder::new().max_rect(row.shrink2(vec2(8.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        let ui = &mut inner;
                        changed |= ui.checkbox(&mut r.enabled, "").on_hover_text(tl!("On / off")).changed();
                        // The account's server can run it while Quatrelle is closed (Exchange's
                        // inbox rules, or an IMAP account's Sieve script; the dialog says when it
                        // can't).
                        let can_server = r.account.as_deref().is_some_and(|a| s.server_kind(a).is_some());
                        if can_server {
                            changed |= ui.checkbox(&mut r.server, "").on_hover_text(tl!("Also run on the server")).changed();
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if widgets::tiny_icon_button(ui, Icon::Delete, "Delete", t).clicked() {
                                remove = Some(i);
                            }
                            if widgets::tiny_icon_button(ui, Icon::Drafts, "Edit", t).clicked() {
                                acts.push(Action::Dialog(Dialog::EditRule(Box::new(RuleForm::from_rule(i, r)))));
                            }
                            if widgets::tiny_icon_button(ui, Icon::Sync, "Run now", t).clicked() {
                                acts.push(Action::Run("rules.run", json!({"index": i, "inbox": true})));
                            }
                            ui.add_space(6.0);
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                let mut job = egui::text::LayoutJob::default();
                                let strong = egui::TextFormat {
                                    font_id: theme::bold(13.0),
                                    color: if r.enabled { t.text } else { t.faint },
                                    ..Default::default()
                                };
                                job.append(&r.name, 0.0, strong);
                                let dim = egui::TextFormat { font_id: theme::regular(12.0), color: t.dim, ..Default::default() };
                                job.append(&what, 10.0, dim);
                                job.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width());
                                ui.add(egui::Label::new(job).selectable(false)).on_hover_text(format!("{}\n{}", r.name, what));
                            });
                        });
                    }
                    if shown == 0 && !s.rules.is_empty() {
                        ui.add_space(8.0);
                        ui.label(RichText::new(tl!("No rule matches.")).color(t.faint));
                        ui.add_space(8.0);
                    }
                });
            });
            if let Some(i) = remove {
                acts.push(Action::Run("rules.delete", json!({"index": i})));
            } else if changed {
                acts.push(Action::Run("rules.set", json!({"rules": rules})));
            }
            // The rules the accounts' servers run that Quatrelle didn't make (Outlook's own, a
            // server's web page, a Sieve script): shown, never changed here — whoever made them
            // owns them.
            if !s.foreign_rules.is_empty() {
                ui.add_space(10.0);
                ui.label(RichText::new(tl!("On the server, made elsewhere")).color(t.dim).small().strong());
                ui.add_space(2.0);
                // A mailbox can keep dozens (a web page's rules, Outlook's own): they scroll
                // rather than pushing the dialog's buttons off the screen.
                egui::ScrollArea::vertical().max_height(150.0).auto_shrink([false, true]).id_salt("server-rules").show(ui, |ui| {
                    for f in &s.foreign_rules {
                        ui.horizontal(|ui| {
                            // The server's own rule: it runs whether Quatrelle is open or not, so
                            // its switch acts there, not here.
                            let mut on = f.enabled;
                            if ui.checkbox(&mut on, "").on_hover_text(tl!("On / off on the server")).changed() {
                                acts.push(Action::Run("rules.server.enabled", json!({"account": f.account, "id": f.id, "enabled": on})));
                            }
                            let who = if s.accounts.len() > 1 {
                                s.account(&f.account).map(|a| a.config.email.clone()).unwrap_or_default()
                            } else {
                                String::new()
                            };
                            let text = if who.is_empty() { f.name.clone() } else { format!("{} — {}", f.name, who) };
                            ui.label(RichText::new(text).color(if f.enabled { t.text } else { t.faint }).small()).on_hover_text(&f.summary);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                if widgets::tiny_icon_button(ui, Icon::Delete, "Delete", t).clicked() {
                                    acts.push(Action::Run("rules.server.delete", json!({"account": f.account, "id": f.id})));
                                }
                                if f.runnable && widgets::tiny_icon_button(ui, Icon::Sync, "Run now", t).clicked() {
                                    acts.push(Action::Run("rules.server.run", json!({"account": f.account, "id": f.id})));
                                }
                                // A rule every part of which Quatrelle knows can be changed here
                                // (its place among the server's rules stays); the others are
                                // shown with why, and stay as the program that made them has them.
                                if f.runnable {
                                    if widgets::tiny_icon_button(ui, Icon::Drafts, "Edit", t).clicked()
                                        && let Some(form) = RuleForm::from_server(&f.account, f)
                                    {
                                        acts.push(Action::Dialog(Dialog::EditRule(Box::new(form))));
                                    }
                                } else {
                                    ui.label(RichText::new(tl!("Quatrelle doesn't know every part of this rule yet")).color(t.faint).small());
                                }
                            });
                        });
                    }
                });
                ui.label(
                    RichText::new(tl!("These rules run on the server. Turn them off, run them, change them or remove them here."))
                        .color(t.faint)
                        .small(),
                );
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tl!("New rule…")).clicked() {
                    acts.push(Action::Dialog(Dialog::rule_from("", None, false)));
                }
                let any_on = s.rules.iter().any(|r| r.enabled);
                if ui
                    .add_enabled(any_on, egui::Button::new(tl!("Run all")))
                    .on_hover_text(tl!("Run every rule that's on, on the mail already in the Inbox"))
                    .clicked()
                {
                    acts.push(Action::Run("rules.run", json!({"inbox": true})));
                }
                // As Outlook's Rules options: the rules to a file, or rules from one.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(!s.rules.is_empty(), egui::Button::new(tl!("Export…"))).clicked() {
                        acts.push(Action::ExportRules);
                    }
                    if ui.button(tl!("Import…")).clicked() {
                        acts.push(Action::ImportRules);
                    }
                });
            });
            ui.add_space(6.0);
            close_row(ui)
        }
        Dialog::EditRule(f) => {
            ui.set_width(540.0);
            let on_server = f.remote.clone();
            title(
                ui,
                t,
                if on_server.is_some() {
                    "Change the rule"
                } else if f.index.is_some() {
                    "Edit rule"
                } else {
                    "New rule"
                },
            );
            ui.horizontal(|ui| {
                ui.label(tl!("Name"));
                ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(400.0));
            });
            ui.add_space(8.0);
            // What the rule looks at: as many tests as needed, all of them or any one.
            ui.label(RichText::new(tl!("When a message")).strong());
            if f.conditions.len() > 1 {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut f.match_all, true, tl!("matches all of them"));
                    ui.radio_value(&mut f.match_all, false, tl!("matches any of them"));
                });
            }
            let mut remove_cond = None;
            for (i, c) in f.conditions.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(("rule-test", i)).width(230.0).selected_text(c.test.label()).show_ui(ui, |ui| {
                        for x in RuleTest::ALL {
                            ui.selectable_value(&mut c.test, x, x.label());
                        }
                    });
                    if c.test.needs_text() {
                        ui.add(egui::TextEdit::singleline(&mut c.text).hint_text(tl!("Text or address")).desired_width(220.0));
                    }
                    if widgets::tiny_icon_button(ui, Icon::Delete, "Remove", t).clicked() {
                        remove_cond = Some(i);
                    }
                });
            }
            if let Some(i) = remove_cond {
                f.conditions.remove(i);
            }
            if ui.button(tl!(if f.conditions.is_empty() { "+ Add a condition" } else { "+ Add another condition" })).clicked() {
                f.conditions.push(RuleCond { test: RuleTest::From, text: String::new() });
            }
            ui.add_space(8.0);
            // What it does: as many actions as needed.
            ui.label(RichText::new(tl!("Then")).strong());
            let mut account = f.account.clone();
            let mut remove_act = None;
            for (i, a) in f.actions.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(("rule-do", i)).width(200.0).selected_text(a.action.label()).show_ui(ui, |ui| {
                        for x in RuleDo::ALL {
                            if ui.selectable_value(&mut a.action, x, x.label()).changed() {
                                a.target.clear();
                            }
                        }
                    });
                    match a.action {
                        RuleDo::MoveTo => {
                            let shown = account
                                .as_deref()
                                .and_then(|id| s.folder(id, &a.target))
                                .map(|x| {
                                    format!(
                                        "{} ({})",
                                        crate::i18n::folder(x),
                                        s.account(&x.account).map(|a| a.config.email.as_str()).unwrap_or_default()
                                    )
                                })
                                .unwrap_or_else(|| tl!("Choose…").into());
                            egui::ComboBox::from_id_salt(("rule-folder", i)).width(240.0).selected_text(shown).show_ui(ui, |ui| {
                                for acc in &s.accounts {
                                    ui.label(RichText::new(&acc.config.email).small().color(t.faint));
                                    for x in acc.folders.iter().filter(|x| x.selectable) {
                                        let on = account.as_deref() == Some(acc.config.id.as_str()) && a.target == x.path;
                                        let label = format!("{}{}", "    ".repeat(x.depth()), crate::i18n::folder(x));
                                        if ui.selectable_label(on, label).clicked() {
                                            account = Some(acc.config.id.clone());
                                            a.target = x.path.clone();
                                        }
                                    }
                                }
                            });
                        }
                        RuleDo::Categorize => {
                            let shown = if a.target.is_empty() { tl!("Choose…").to_string() } else { tl!(&a.target).to_string() };
                            egui::ComboBox::from_id_salt(("rule-cat", i)).width(200.0).selected_text(shown).show_ui(ui, |ui| {
                                for c in &s.prefs.categories {
                                    ui.selectable_value(&mut a.target, c.name.clone(), &c.name);
                                }
                            });
                        }
                        _ => {}
                    }
                    if widgets::tiny_icon_button(ui, Icon::Delete, "Remove", t).clicked() {
                        remove_act = Some(i);
                    }
                });
            }
            f.account = account;
            if let Some(i) = remove_act {
                f.actions.remove(i);
            }
            if ui.button(tl!(if f.actions.is_empty() { "+ Add an action" } else { "+ Add another action" })).clicked() {
                f.actions.push(RuleAct { action: RuleDo::MoveTo, target: String::new() });
            }
            ui.add_space(6.0);
            ui.checkbox(&mut f.stop, tl!("Don't run later rules on mail this one matches"));
            // Outlook's "on this server": the server files the mail while Quatrelle is closed,
            // as far as it can express the rule (Exchange's inbox rules, a Sieve script). A rule
            // the server keeps already runs there.
            if on_server.is_none() {
                let server_account = f
                    .account
                    .clone()
                    .filter(|a| s.server_kind(a).is_some())
                    .or_else(|| s.accounts.iter().find(|a| s.server_kind(&a.config.id).is_some()).map(|a| a.config.id.clone()));
                if let Some(id) = server_account {
                    let mut on = f.server;
                    if ui.checkbox(&mut on, tl!("Also run on the server")).changed() {
                        f.server = on;
                        if on && f.account.is_none() {
                            f.account = Some(id.clone());
                        }
                    }
                }
            }
            if f.server || on_server.is_some() {
                // What this account's server (Exchange's rules, or Sieve) can't express stays
                // here, and the dialog says why.
                let kind = f.account.clone().or_else(|| on_server.as_ref().map(|(a, _)| a.clone())).and_then(|a| s.server_kind(&a));
                let blockers = f.to_rule().map(|r| kind.map(|k| r.server_blockers_for(k)).unwrap_or_default()).unwrap_or_default();
                if !blockers.is_empty() {
                    ui.label(
                        RichText::new(tf!("The server can't run this rule: {why}. It stays on this computer.", why = blockers.join(", ")))
                            .color(t.warn)
                            .small(),
                    );
                }
            }
            if on_server.is_none() {
                ui.checkbox(&mut f.run_now, tl!("Run it now on the mail already in the Inbox"));
            }
            if let Some(e) = &f.error {
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let (ok, cancel) = buttons(ui, t, "Save", true);
            if ok {
                match f.to_rule() {
                    Ok(rule) => match &on_server {
                        // The server's own rule: only its parts go back, at the place it keeps.
                        Some((account, id)) => {
                            f.error = None;
                            acts.push(Action::Submit("rules.server.save", json!({"account": account, "id": id, "rule": rule})));
                        }
                        None => {
                            let mut rules = s.rules.clone();
                            let at = match f.index.filter(|i| *i < rules.len()) {
                                Some(i) => {
                                    // Keep what the server knows of it: its id, and whether it runs.
                                    let (enabled, remote_id, priority) = match rules.get(i) {
                                        Some(r) => (r.enabled, r.remote_id.clone(), r.priority),
                                        None => (true, None, None),
                                    };
                                    if let Some(slot) = rules.get_mut(i) {
                                        *slot = Rule { enabled, remote_id, priority, ..rule };
                                    }
                                    i
                                }
                                None => {
                                    rules.push(rule);
                                    rules.len() - 1
                                }
                            };
                            f.error = None;
                            acts.push(Action::Submit("rules.set", json!({"rules": rules})));
                            if f.run_now {
                                acts.push(Action::Run("rules.run", json!({"index": at, "inbox": true})));
                            }
                        }
                    },
                    Err(e) => f.error = Some(e),
                }
            }
            !cancel
        }
        Dialog::SearchFolder(f) => {
            ui.set_width(460.0);
            title(ui, t, if f.original.is_some() { "Edit Search Folder" } else { "New Search Folder" });
            ui.add(
                egui::Label::new(RichText::new(tl!("Shows the mail it finds in every folder, except Deleted Items and Junk Email.")).color(t.dim))
                    .wrap(),
            );
            ui.add_space(8.0);
            ui.label(RichText::new(tl!("Show")).color(t.dim).small());
            let before = f.preset;
            egui::ComboBox::from_id_salt("search-folder-preset")
                .width(ui.available_width())
                .selected_text(tl!(SEARCH_PRESETS.get(f.preset).map_or("Custom", |p| p.0)))
                .show_ui(ui, |ui| {
                    for (i, (label, _)) in SEARCH_PRESETS.iter().enumerate() {
                        ui.selectable_value(&mut f.preset, i, tl!(label));
                    }
                });
            if f.preset != before {
                f.apply_preset(before);
            }
            let (needs_people, custom) = f.needs();
            if needs_people {
                ui.add_space(6.0);
                ui.label(RichText::new(tl!("From these people (addresses or names, separated by commas)")).color(t.dim).small());
                ui.add(egui::TextEdit::singleline(&mut f.people).hint_text("ana@example.com, Jun").desired_width(f32::INFINITY));
            }
            if custom {
                ui.add_space(6.0);
                ui.label(RichText::new(tl!("Search for")).color(t.dim).small());
                ui.add(egui::TextEdit::singleline(&mut f.query).hint_text("from:ana subject:invoice has:attachment").desired_width(f32::INFINITY));
            }
            ui.add_space(6.0);
            ui.label(RichText::new(tl!("Name")).color(t.dim).small());
            let r = ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(f32::INFINITY));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if let Some(e) = &f.error {
                ui.add_space(4.0);
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let query = f.final_query();
            let ready = !f.name.trim().is_empty() && !query.trim().is_empty();
            let (ok, cancel) = buttons(ui, t, if f.original.is_some() { "Save" } else { "Create" }, ready);
            if (ok || enter) && ready {
                let p = match &f.original {
                    Some(old) => json!({"name": old, "newName": f.name.trim(), "query": query}),
                    None => json!({"name": f.name.trim(), "query": query}),
                };
                acts.push(Action::Submit(if f.original.is_some() { "searchFolder.edit" } else { "searchFolder.create" }, p));
            }
            !cancel
        }
        Dialog::QuickStep(f) => {
            ui.set_width(440.0);
            title(ui, t, if f.original.is_some() { "Edit Quick Step" } else { "New Quick Step" });
            ui.add(
                egui::Label::new(
                    RichText::new(tl!("Done to the selected mail in one click; the first nine also with Ctrl+Shift+1 to 9.")).color(t.dim),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            ui.label(RichText::new(tl!("Name")).color(t.dim).small());
            let r = ui.add(egui::TextEdit::singleline(&mut f.name).hint_text(tl!("Done")).desired_width(f32::INFINITY));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(6.0);
            ui.checkbox(&mut f.read, tl!("Mark as read"));
            ui.checkbox(&mut f.flag, tl!("Flag"));
            ui.checkbox(&mut f.pin, tl!("Pin"));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(tl!("Categorize"));
                let shown = if f.category.is_empty() { tl!("None").to_string() } else { f.category.clone() };
                egui::ComboBox::from_id_salt("quick-step-category").width(220.0).selected_text(shown).show_ui(ui, |ui| {
                    ui.selectable_value(&mut f.category, String::new(), tl!("None"));
                    for c in &s.prefs.categories {
                        ui.selectable_value(&mut f.category, c.name.clone(), c.name.as_str());
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.label(tl!("Then move to"));
                egui::ComboBox::from_id_salt("quick-step-move").width(220.0).selected_text(QuickStepForm::move_label(&f.move_to)).show_ui(ui, |ui| {
                    let mut choices = vec![
                        StepMove::Stay,
                        StepMove::Role(FolderRole::Archive),
                        StepMove::Role(FolderRole::Trash),
                        StepMove::Role(FolderRole::Junk),
                    ];
                    choices.extend(f.folders.iter().map(|p| StepMove::Folder(p.clone())));
                    for c in choices {
                        let label = QuickStepForm::move_label(&c);
                        ui.selectable_value(&mut f.move_to, c, label);
                    }
                });
            });
            if let Some(e) = &f.error {
                ui.add_space(4.0);
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let ready = !f.name.trim().is_empty() && !f.actions().is_empty();
            if let Some(old) = f.original.clone() {
                ui.add_space(6.0);
                if ui.button(RichText::new(tl!("Delete this Quick Step")).color(t.danger)).clicked() {
                    acts.push(Action::Submit("quickStep.delete", json!({"name": old})));
                }
            }
            let (ok, cancel) = buttons(ui, t, if f.original.is_some() { "Save" } else { "Create" }, ready);
            if (ok || enter) && ready {
                let p = match &f.original {
                    Some(old) => json!({"name": old, "newName": f.name.trim(), "actions": f.actions()}),
                    None => json!({"name": f.name.trim(), "actions": f.actions()}),
                };
                acts.push(Action::Submit(if f.original.is_some() { "quickStep.edit" } else { "quickStep.create" }, p));
            }
            !cancel
        }
        // Every other kind is shown by its own group (`body`).
        _ => true,
    }
}

/// What the rule editor offers: one test and one action (rules made elsewhere can have more).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleTest {
    From,
    To,
    Subject,
    AnyText,
    HasAttachment,
    Bulk,
    /// A list of senders, any of which matches (Exchange's "from these people").
    FromAny,
    /// A list of recipients.
    ToAny,
    /// A list of subjects.
    SubjectAny,
}

impl RuleTest {
    pub(super) const ALL: [RuleTest; 9] = [
        RuleTest::From,
        RuleTest::To,
        RuleTest::Subject,
        RuleTest::AnyText,
        RuleTest::HasAttachment,
        RuleTest::Bulk,
        RuleTest::FromAny,
        RuleTest::ToAny,
        RuleTest::SubjectAny,
    ];
    pub(super) fn label(self) -> &'static str {
        tl!(match self {
            RuleTest::From => "From contains",
            RuleTest::To => "To or Cc contains",
            RuleTest::Subject => "Subject contains",
            RuleTest::AnyText => "Sender, recipients or subject contain",
            RuleTest::HasAttachment => "Has attachments",
            RuleTest::Bulk => "Is a newsletter or list mail",
            RuleTest::FromAny => "From one of these (comma-separated)",
            RuleTest::ToAny => "To or Cc one of these (comma-separated)",
            RuleTest::SubjectAny => "Subject has one of these (comma-separated)",
        })
    }
    pub(super) fn needs_text(self) -> bool {
        !matches!(self, RuleTest::HasAttachment | RuleTest::Bulk)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleDo {
    MoveTo,
    Archive,
    Delete,
    Categorize,
    MarkRead,
    Flag,
    Pin,
}

impl RuleDo {
    pub(super) const ALL: [RuleDo; 7] =
        [RuleDo::MoveTo, RuleDo::Archive, RuleDo::Delete, RuleDo::Categorize, RuleDo::MarkRead, RuleDo::Flag, RuleDo::Pin];
    pub(super) fn label(self) -> &'static str {
        tl!(match self {
            RuleDo::MoveTo => "Move to folder",
            RuleDo::Archive => "Archive",
            RuleDo::Delete => "Delete",
            RuleDo::Categorize => "Categorize",
            RuleDo::MarkRead => "Mark as read",
            RuleDo::Flag => "Flag",
            RuleDo::Pin => "Pin",
        })
    }
}

/// The Search Folders offered, as Outlook's New Search Folder lists them: a name and what it
/// searches for. "Mail from specific people" asks for them; "Custom" for the search.
pub const SEARCH_PRESETS: [(&str, &str); 9] = [
    ("Unread mail", "is:unread"),
    ("Mail flagged for follow up", "is:flagged"),
    ("Important mail", "is:important"),
    ("Categorized mail", "is:categorized"),
    ("Mail with attachments", "has:attachment"),
    ("Large mail (over 100 KB)", "larger:100KB"),
    ("Pinned mail", "is:pinned"),
    ("Mail from specific people", "from:"),
    ("Custom", ""),
];

pub(super) const PRESET_PEOPLE: usize = 7;

pub(super) const PRESET_CUSTOM: usize = 8;

#[derive(Clone, Debug, Default)]
pub struct SearchFolderForm {
    /// The name it had (`None`: a new one).
    pub original: Option<String>,
    /// Which of [`SEARCH_PRESETS`].
    pub preset: usize,
    pub name: String,
    /// For "Mail from specific people": addresses or names, separated by commas.
    pub people: String,
    /// For "Custom": the search, as typed in the search box.
    pub query: String,
    pub error: Option<String>,
}

impl SearchFolderForm {
    pub fn new() -> SearchFolderForm {
        SearchFolderForm { name: tl!("Unread mail").to_string(), ..Default::default() }
    }

    /// An existing one, to change: its search shown as written.
    pub fn edit(f: &quatrelle_engine::SearchFolder) -> SearchFolderForm {
        let preset = SEARCH_PRESETS.iter().position(|p| !p.1.is_empty() && p.1 != "from:" && p.1 == f.query).unwrap_or(PRESET_CUSTOM);
        SearchFolderForm { original: Some(f.name.clone()), preset, name: f.name.clone(), query: f.query.clone(), ..Default::default() }
    }

    /// Another kind picked: its name follows, unless one was typed.
    pub fn apply_preset(&mut self, before: usize) {
        let was = SEARCH_PRESETS.get(before).map(|p| tl!(p.0).to_string()).unwrap_or_default();
        if self.name.trim().is_empty() || self.name == was {
            self.name = SEARCH_PRESETS.get(self.preset).map(|p| tl!(p.0).to_string()).unwrap_or_default();
        }
        if self.preset == PRESET_CUSTOM && self.query.trim().is_empty() {
            self.query = SEARCH_PRESETS.get(before).map(|p| p.1.to_string()).filter(|q| q != "from:").unwrap_or_default();
        }
    }

    /// Whether it asks for people, and for a search of its own.
    pub fn needs(&self) -> (bool, bool) {
        (self.preset == PRESET_PEOPLE, self.preset == PRESET_CUSTOM)
    }

    /// What it searches for: `from:ana,"zoe park"` for people.
    pub fn final_query(&self) -> String {
        match self.preset {
            PRESET_PEOPLE => {
                let people: Vec<String> = self
                    .people
                    .split([',', ';'])
                    .map(|p| p.trim().replace('"', ""))
                    .filter(|p| !p.is_empty())
                    .map(|p| if p.contains(char::is_whitespace) { format!("\"{p}\"") } else { p })
                    .collect();
                if people.is_empty() { String::new() } else { format!("from:{}", people.join(",")) }
            }
            PRESET_CUSTOM => self.query.trim().to_string(),
            i => SEARCH_PRESETS.get(i).map(|p| p.1.to_string()).unwrap_or_default(),
        }
    }
}

/// Where a Quick Step moves mail: nowhere, a folder with a role, or a folder by its path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum StepMove {
    #[default]
    Stay,
    Role(FolderRole),
    Folder(String),
}

#[derive(Clone, Debug, Default)]
pub struct QuickStepForm {
    /// The name it had (`None`: a new one).
    pub original: Option<String>,
    pub name: String,
    pub read: bool,
    pub flag: bool,
    pub pin: bool,
    /// A category to add (empty: none).
    pub category: String,
    pub move_to: StepMove,
    /// The folders to offer: each account's own, by path, once.
    pub folders: Vec<String>,
    pub error: Option<String>,
}

impl QuickStepForm {
    pub(super) fn folders(s: &Session) -> Vec<String> {
        let mut v: Vec<String> = s
            .accounts
            .iter()
            .flat_map(|a| a.folders.iter())
            .filter(|f| matches!(f.role, FolderRole::Other | FolderRole::Inbox) && f.mailbox.is_none())
            .map(|f| f.path.clone())
            .collect();
        v.sort();
        v.dedup();
        v.truncate(500);
        v
    }

    /// A new one: marked read and moved to Archive, as Outlook's "Done".
    pub fn new(s: &Session) -> QuickStepForm {
        QuickStepForm { read: true, move_to: StepMove::Role(FolderRole::Archive), folders: Self::folders(s), ..Default::default() }
    }

    pub fn edit(s: &Session, q: &quatrelle_engine::QuickStep) -> QuickStepForm {
        use quatrelle_engine::model::organize::RuleAction;
        let mut f = QuickStepForm { original: Some(q.name.clone()), name: q.name.clone(), folders: Self::folders(s), ..Default::default() };
        for a in &q.actions {
            match a {
                RuleAction::MarkRead => f.read = true,
                RuleAction::Flag => f.flag = true,
                RuleAction::Pin => f.pin = true,
                RuleAction::Categorize(c) => f.category = c.clone(),
                RuleAction::MoveTo(p) => f.move_to = StepMove::Folder(p.clone()),
                RuleAction::MoveToRole(r) => f.move_to = StepMove::Role(*r),
                RuleAction::Delete => f.move_to = StepMove::Role(FolderRole::Trash),
            }
        }
        f
    }

    /// Its actions as `quickStep.create` takes them.
    pub fn actions(&self) -> Vec<serde_json::Value> {
        let mut v = Vec::new();
        if self.read {
            v.push(json!({"do": "markRead"}));
        }
        if self.flag {
            v.push(json!({"do": "flag"}));
        }
        if self.pin {
            v.push(json!({"do": "pin"}));
        }
        if !self.category.is_empty() {
            v.push(json!({"do": "categorize", "value": self.category}));
        }
        match &self.move_to {
            StepMove::Stay => {}
            StepMove::Role(FolderRole::Trash) => v.push(json!({"do": "delete"})),
            StepMove::Role(r) => v.push(json!({"do": "moveToRole", "value": r.code()})),
            StepMove::Folder(p) => v.push(json!({"do": "moveTo", "value": p})),
        }
        v
    }

    pub(super) fn move_label(m: &StepMove) -> String {
        match m {
            StepMove::Stay => tl!("Don't move").to_string(),
            StepMove::Role(FolderRole::Archive) => tl!("Archive").to_string(),
            StepMove::Role(FolderRole::Trash) => tl!("Deleted Items").to_string(),
            StepMove::Role(FolderRole::Junk) => tl!("Junk Email").to_string(),
            StepMove::Role(r) => r.label().unwrap_or("").to_string(),
            StepMove::Folder(p) => p.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RuleForm {
    /// The rule being changed (`None`: a new one).
    pub index: Option<usize>,
    /// A rule of the server's own being changed: its account, and the id it has there.
    pub remote: Option<(String, String)>,
    pub name: String,
    pub enabled: bool,
    /// Every condition must match (`false`: any one).
    pub match_all: bool,
    pub conditions: Vec<RuleCond>,
    pub actions: Vec<RuleAct>,
    /// The account whose folders `MoveTo` offers (and the only one it applies to).
    pub account: Option<String>,
    pub stop: bool,
    /// Run it on the server too (Exchange's inbox rules, an IMAP account's Sieve script), when
    /// the server can express it.
    pub server: bool,
    /// Run it on the mail already in the Inbox once saved (Outlook's "run this rule now").
    pub run_now: bool,
    pub error: Option<String>,
}

impl RuleForm {
    pub(super) fn to_rule(&self) -> Result<Rule, String> {
        if self.name.trim().is_empty() {
            return Err(tl!("Give the rule a name").into());
        }
        let mut conditions = Vec::new();
        for c in &self.conditions {
            let t = c.text.trim().to_string();
            if c.test.needs_text() && t.is_empty() {
                return Err(tl!("Say what to look for").into());
            }
            let several = || t.split([',', ';']).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect::<Vec<_>>();
            conditions.push(match c.test {
                RuleTest::From => Condition::From(t),
                RuleTest::To => Condition::To(t),
                RuleTest::Subject => Condition::Subject(t),
                RuleTest::AnyText => Condition::AnyText(t),
                RuleTest::HasAttachment => Condition::HasAttachment,
                RuleTest::Bulk => Condition::Bulk,
                RuleTest::FromAny => Condition::FromAny(several()),
                RuleTest::ToAny => Condition::ToAny(several()),
                RuleTest::SubjectAny => Condition::SubjectAny(several()),
            });
        }
        if conditions.is_empty() {
            return Err(tl!("Add what the rule looks at").into());
        }
        let mut actions = Vec::new();
        for a in &self.actions {
            actions.push(match a.action {
                RuleDo::MoveTo if a.target.is_empty() => return Err(tl!("Pick a folder").into()),
                RuleDo::MoveTo => RuleAction::MoveTo(a.target.clone()),
                RuleDo::Archive => RuleAction::MoveToRole(FolderRole::Archive),
                RuleDo::Delete => RuleAction::Delete,
                RuleDo::Categorize if a.target.is_empty() => return Err(tl!("Pick a category").into()),
                RuleDo::Categorize => RuleAction::Categorize(a.target.clone()),
                RuleDo::MarkRead => RuleAction::MarkRead,
                RuleDo::Flag => RuleAction::Flag,
                RuleDo::Pin => RuleAction::Pin,
            });
        }
        if actions.is_empty() {
            return Err(tl!("Add what the rule does").into());
        }
        let moves = self.actions.iter().any(|a| a.action == RuleDo::MoveTo);
        Ok(Rule {
            name: self.name.trim().to_string(),
            enabled: self.enabled,
            account: if moves || self.server || self.remote.is_some() { self.account.clone() } else { None },
            conditions,
            match_all: self.match_all,
            actions,
            stop: self.stop,
            server: self.server || self.remote.is_some(),
            remote_id: self.remote.as_ref().map(|(_, id)| id.clone()),
            priority: None,
        })
    }

    pub(super) fn from_rule(index: usize, r: &Rule) -> RuleForm {
        RuleForm {
            index: Some(index),
            remote: None,
            name: r.name.clone(),
            enabled: r.enabled,
            match_all: r.match_all,
            conditions: r.conditions.iter().map(RuleCond::of).collect(),
            actions: r.actions.iter().map(RuleAct::of).collect(),
            account: r.account.clone(),
            stop: r.stop,
            server: r.server,
            run_now: false,
            error: None,
        }
    }

    /// A rule the server keeps (made in Outlook, on a web page, in a Sieve script Quatrelle
    /// doesn't own): shown and changed from its parts, never re-ordered.
    pub(super) fn from_server(account: &str, f: &ForeignRule) -> Option<RuleForm> {
        let r = f.rule.as_ref()?;
        Some(RuleForm {
            remote: Some((account.to_string(), f.id.clone())),
            account: Some(account.to_string()),
            ..RuleForm::from_rule(0, r).without_index()
        })
    }

    pub(super) fn without_index(mut self) -> RuleForm {
        self.index = None;
        self
    }
}

/// One line describing a rule.
/// A folder as the folder pane names it, with the folders it's in: `Inbox › gzamer`.
pub(super) fn folder_path_label(s: &Session, path: &str) -> String {
    // The folder's own delimiter; "/" (the commonest) for one not synced yet.
    let delimiter = s.accounts.iter().flat_map(|a| &a.folders).find(|f| f.path == path).map_or(Some('/'), |f| f.delimiter);
    let decoded = quatrelle_engine::model::utf7::decode(path);
    let parts: Vec<&str> = match delimiter {
        Some(d) => decoded.split(d).collect(),
        None => vec![decoded.as_str()],
    };
    parts
        .iter()
        .enumerate()
        .map(|(i, p)| if i == 0 && p.eq_ignore_ascii_case("INBOX") { tl!("Inbox").to_string() } else { crate::i18n::tc("folder", p).to_string() })
        .collect::<Vec<_>>()
        .join(" › ")
}

pub(super) fn describe(s: &Session, r: &Rule) -> String {
    let cond = r
        .conditions
        .iter()
        .map(|c| match c {
            Condition::From(t) => tf!("from “{text}”", text = t),
            Condition::To(t) => tf!("to “{text}”", text = t),
            Condition::Subject(t) => tf!("subject has “{text}”", text = t),
            Condition::AnyText(t) => tf!("mentions “{text}”", text = t),
            Condition::HasAttachment => tl!("has attachments").to_string(),
            Condition::Bulk => tl!("is list mail").to_string(),
            Condition::FromAny(v) => tf!("from “{text}”", text = v.join(", ")),
            Condition::ToAny(v) => tf!("to “{text}”", text = v.join(", ")),
            Condition::SubjectAny(v) => tf!("subject has “{text}”", text = v.join(", ")),
        })
        .collect::<Vec<_>>()
        .join(&format!(" {} ", if r.match_all { tl!("and") } else { tl!("or") }));
    let act = r
        .actions
        .iter()
        .map(|a| match a {
            RuleAction::MoveTo(f) => tf!("move to {folder}", folder = folder_path_label(s, f)),
            RuleAction::MoveToRole(role) => tf!("move to {folder}", folder = tl!(role.label().unwrap_or("folder"))),
            RuleAction::MarkRead => tl!("mark read").to_string(),
            RuleAction::Flag => tl!("flag").to_string(),
            RuleAction::Pin => tl!("pin").to_string(),
            RuleAction::Categorize(c) => tf!("categorize {category}", category = c),
            RuleAction::Delete => tl!("delete").to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    tf!("If it {condition}: {action}", condition = cond, action = act)
}
