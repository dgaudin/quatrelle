//! Dialogs: categories, rules, Search Folders and Quick Steps.

use super::*;

mod rule_form;

pub use rule_form::*;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::NewCategory { name, color, error } => {
            ui.set_width(340.0);
            title(ui, t, "New category");
            let r = ui.add(egui::TextEdit::singleline(name).hint_text(tl!("Name")).desired_width(f32::INFINITY));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            ui.horizontal_wrapped(|ui| {
                for c in 0..quatrelle_engine::model::organize::CATEGORY_COLORS {
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
        Dialog::ReplaceOutlookRules { account, email } => {
            ui.set_width(440.0);
            title(ui, t, "Replace Outlook's copy of the rules?");
            ui.label(RichText::new(email.as_str()).color(t.dim));
            ui.add_space(4.0);
            ui.label(RichText::new(tl!(
                "Outlook keeps a copy of this mailbox's rules, and Exchange won't change them while it's there. Replacing it lets Quatrelle change them. The rules Outlook runs only while it's open on a computer will be lost; the rules on the server stay."
            )));
            let (ok, cancel) = buttons(ui, t, "Replace", true);
            if ok {
                acts.push(Action::Run("rules.replaceOutlookCopy", json!({"account": account})));
            }
            if ok || cancel {
                acts.push(Action::Dialog(Dialog::Rules));
            }
            !(ok || cancel)
        }
        Dialog::Rules => {
            ui.set_width(560.0);
            title(ui, t, "Rules");
            ui.label(RichText::new(tl!("Rules run on new mail as it arrives in an Inbox, in this order.")).color(t.dim).small());
            ui.add_space(6.0);
            // Exchange refuses every change while Outlook keeps its own copy of the rules: said,
            // with the way out.
            for a in s.accounts.iter().filter(|a| s.outlook_keeps_rules(&a.config.id)) {
                egui::Frame::new().fill(t.warn.gamma_multiply(0.15)).corner_radius(6).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let text = tf!(
                        "Exchange won't change the rules of the account {email} while Outlook keeps its own copy of them.",
                        email = a.config.email
                    );
                    ui.label(RichText::new(text).color(t.text));
                    if ui.button(tl!("Replace Outlook's copy…")).clicked() {
                        acts.push(Action::Dialog(Dialog::ReplaceOutlookRules { account: a.config.id.clone(), email: a.config.email.clone() }));
                    }
                });
                ui.add_space(6.0);
            }
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
                            // The server wouldn't take it: said here with why, until it's
                            // changed (then it's sent again).
                            if let Some(why) = &r.refused {
                                ui.label(RichText::new(tl!("Refused by the server")).color(t.warn).small()).on_hover_text(why);
                            }
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
                                // Cut at the buttons: a label in a row otherwise runs on as far as
                                // its text goes, whatever its job says (a rule with a long sender
                                // ran under its buttons and out of the dialog, 29/10).
                                ui.add(egui::Label::new(job).truncate().selectable(false)).on_hover_text(format!("{}\n{}", r.name, what));
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
                            // Its buttons first, at the right; its name in what's left, cut there.
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
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    let name = RichText::new(text).color(if f.enabled { t.text } else { t.faint }).small();
                                    ui.add(egui::Label::new(name).truncate()).on_hover_text(&f.summary);
                                });
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
                    acts.push(Action::Dialog(Dialog::rule_from("", None, false, None)));
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
        Dialog::EditRule(f) => rule_form::edit_rule(ui, f, s, t, acts),
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

/// The Search Folders offered, as Outlook's New Search Folder lists them: a name and what it
/// searches for. "Mail from specific people" asks for them; "Custom" for the search.
pub const SEARCH_PRESETS: [(&str, &str); 10] = [
    ("Unread mail", "is:unread"),
    ("Mail flagged for follow up", "is:flagged"),
    ("Important mail", "is:important"),
    ("Categorized mail", "is:categorized"),
    ("Mail with attachments", "has:attachment"),
    ("Large mail (over 100 KB)", "larger:100KB"),
    ("Pinned mail", "is:pinned"),
    // Not one of Outlook's: what you sent that nobody has answered (two to thirty days old).
    ("Sent mail waiting for a reply", "is:awaiting"),
    ("Mail from specific people", "from:"),
    ("Custom", ""),
];

pub(super) const PRESET_PEOPLE: usize = 8;

pub(super) const PRESET_CUSTOM: usize = 9;

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
