//! The rule editor: what a rule looks at and does, where it runs, and the form behind it.

use super::*;
use crate::dialogs::people_field::{DIALOG_WINDOWS, PeopleFields, accepts_of, dialog_field};
use quatrelle_engine::FieldId;
use quatrelle_engine::model::addr::matched;
use quatrelle_engine::model::organize::ForeignRule;

/// How wide a condition's contacts field is; the row's × follows it.
const RULE_FIELD_W: f32 = 260.0;

/// The contacts field condition `i` of a rule shows for its `test`: one address for From and To,
/// several for the lists. Any other test keeps its plain box (`None`).
pub(crate) fn rule_field(i: usize, test: RuleTest) -> Option<FieldId> {
    match test {
        RuleTest::From | RuleTest::To => Some(dialog_field(i as u64, "rule-one")),
        RuleTest::FromAny | RuleTest::ToAny => Some(dialog_field(i as u64, "rule-any")),
        _ => None,
    }
}

/// The New rule / Edit rule dialog; whether it stays open.
pub(super) fn edit_rule(ui: &mut Ui, f: &mut RuleForm, s: &Session, people: &mut PeopleFields, t: &Tokens, acts: &mut Vec<Action>) -> bool {
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
    // The directory the names are looked up in, when an account has one (Ctrl+K and Expand).
    let directory = s.directory_account(f.account.as_deref());
    for (i, c) in f.conditions.iter_mut().enumerate() {
        ui.horizontal_top(|ui| {
            let before = c.test;
            egui::ComboBox::from_id_salt(("rule-test", i)).width(230.0).selected_text(c.test.label()).show_ui(ui, |ui| {
                for x in RuleTest::ALL {
                    ui.selectable_value(&mut c.test, x, x.label());
                }
            });
            if c.test != before {
                // Another test, another field (or none): made again from the text.
                people.reset_window(DIALOG_WINDOWS | i as u64);
            }
            if let Some(field) = rule_field(i, c.test) {
                ui.vertical(|ui| {
                    ui.set_width(RULE_FIELD_W);
                    let accepts = accepts_of(field.name);
                    people.draw(
                        ui,
                        t,
                        field,
                        directory.as_deref(),
                        accepts,
                        "",
                        tl!("Text or address"),
                        &mut c.text,
                        false,
                        directory.is_some(),
                        false,
                        acts,
                    );
                });
            } else if c.test.needs_text() {
                ui.add(egui::TextEdit::singleline(&mut c.text).hint_text(tl!("Text or address")).desired_width(220.0));
            }
            if widgets::tiny_icon_button(ui, Icon::Delete, "Remove", t).clicked() {
                remove_cond = Some(i);
            }
        });
    }
    if let Some(i) = remove_cond {
        // Each field after it is the next one's now: made again from its text.
        for j in i..f.conditions.len() {
            people.reset_window(DIALOG_WINDOWS | j as u64);
        }
        f.conditions.remove(i);
    }
    // Which person a name means, when several fit (Ctrl+K in a field).
    for (i, c) in f.conditions.iter().enumerate() {
        if let Some(field) = rule_field(i, c.test) {
            people.choice_dialog(ui.ctx(), t, s, field.window, &[field.name], acts);
        }
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
                            format!("{} ({})", crate::i18n::folder(x), s.account(&x.account).map(|a| a.config.email.as_str()).unwrap_or_default())
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
    // Outlook's "on this server": the server files the mail while Quatrelle is closed, as far
    // as it can express the rule (Exchange's inbox rules, a Sieve script). A rule the server
    // keeps already runs there.
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
        // What this account's server (Exchange's rules, or Sieve) can't express stays here, and
        // the dialog says why.
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
    if let Some(why) = f.refused.as_deref().filter(|_| f.server) {
        ui.add(
            egui::Label::new(
                RichText::new(tf!("The server refused this rule ({why}). It's sent there again once saved.", why = why)).color(t.warn).small(),
            )
            .wrap(),
        );
    }
    if on_server.is_none() {
        // Made from a message in another folder, the rule runs on that folder's mail now (the
        // Inbox alone, it seemed to do nothing there).
        match f.run_now_folder(s) {
            Some(folder) => ui.checkbox(&mut f.run_now, tf!("Run it now on the mail already in {folder}", folder = folder)),
            None => ui.checkbox(&mut f.run_now, tl!("Run it now on the mail already in the Inbox")),
        };
    }
    if let Some(e) = &f.error {
        ui.label(RichText::new(e.as_str()).color(t.danger).small());
    }
    // An entry its place refuses (a contact group has no address) stops the rule.
    let stopped = f.conditions.iter().enumerate().any(|(i, c)| rule_field(i, c.test).is_some_and(|field| people.blocked(field)));
    let (ok, cancel) = buttons(ui, t, "Save", !stopped);
    if ok {
        match f.save_actions(s) {
            Ok(a) => {
                f.error = None;
                acts.extend(a);
            }
            Err(e) => f.error = Some(e),
        }
    }
    !cancel
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
    pub(crate) const ALL: [RuleTest; 9] = [
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
    pub(crate) fn label(self) -> &'static str {
        tl!(match self {
            RuleTest::From => "From contains",
            RuleTest::To => "To or Cc contains",
            RuleTest::Subject => "Subject contains",
            RuleTest::AnyText => "Sender, recipients or subject contain",
            RuleTest::HasAttachment => "Has attachments",
            RuleTest::Bulk => "Is a newsletter or list mail",
            RuleTest::FromAny => "From one of these",
            RuleTest::ToAny => "To or Cc one of these",
            RuleTest::SubjectAny => "Subject has one of these (comma-separated)",
        })
    }
    pub(crate) fn needs_text(self) -> bool {
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
    pub(crate) const ALL: [RuleDo; 7] =
        [RuleDo::MoveTo, RuleDo::Archive, RuleDo::Delete, RuleDo::Categorize, RuleDo::MarkRead, RuleDo::Flag, RuleDo::Pin];
    pub(crate) fn label(self) -> &'static str {
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
    /// Run it once saved on the mail already there (Outlook's "run this rule now"): in
    /// `run_in`, or the Inbox.
    pub run_now: bool,
    /// The folder the rule was made from (a message's, by its path in `account`), to run it on
    /// now. `None`: the Inbox.
    pub run_in: Option<String>,
    /// Why the server refused the rule being changed, when it did (shown; saving sends it again).
    pub refused: Option<String>,
    pub error: Option<String>,
}

impl RuleForm {
    pub(crate) fn to_rule(&self) -> Result<Rule, String> {
        if self.name.trim().is_empty() {
            return Err(tl!("Give the rule a name").into());
        }
        let mut conditions = Vec::new();
        for c in &self.conditions {
            let t = c.text.trim().to_string();
            // The addresses alone, names as typed (`Doe, John <j@x.fr>` is `j@x.fr`): a text of
            // separators only says nothing either.
            let entries = matched(&c.text);
            let address_test = rule_field(0, c.test).is_some();
            if c.test.needs_text() && (t.is_empty() || (address_test && entries.is_empty())) {
                return Err(tl!("Say what to look for").into());
            }
            let several = || t.split([',', ';']).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect::<Vec<_>>();
            conditions.push(match c.test {
                RuleTest::From => Condition::From(entries.into_iter().next().unwrap_or_default()),
                RuleTest::To => Condition::To(entries.into_iter().next().unwrap_or_default()),
                RuleTest::Subject => Condition::Subject(t),
                RuleTest::AnyText => Condition::AnyText(t),
                RuleTest::HasAttachment => Condition::HasAttachment,
                RuleTest::Bulk => Condition::Bulk,
                RuleTest::FromAny => Condition::FromAny(entries),
                RuleTest::ToAny => Condition::ToAny(entries),
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
            // Changed here, it goes to the server again.
            refused: None,
        })
    }

    /// The folder the rule runs on now, named as the folder pane names it — when it's one other
    /// than the Inbox (`None`: the Inbox, or no folder given).
    pub(crate) fn run_now_folder(&self, s: &Session) -> Option<String> {
        let (account, path) = (self.account.as_deref()?, self.run_in.as_deref()?);
        let folder = s.folder(account, path)?;
        (folder.role != FolderRole::Inbox).then(|| folder_path_label(s, path))
    }

    /// Saving: the rules as they'll be (`rules.set`, or `rules.server.save` for a rule the
    /// server keeps), then the new one run on the mail already there when asked — in the folder
    /// it was made from, or the Inbox.
    pub(crate) fn save_actions(&self, s: &Session) -> Result<Vec<Action>, String> {
        let rule = self.to_rule()?;
        // The server's own rule: only its parts go back, at the place it keeps.
        if let Some((account, id)) = &self.remote {
            return Ok(vec![Action::Submit("rules.server.save", json!({"account": account, "id": id, "rule": rule}))]);
        }
        let mut rules = s.rules.clone();
        let at = match self.index.filter(|i| *i < rules.len()) {
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
        let mut acts = vec![Action::Submit("rules.set", json!({"rules": rules}))];
        if self.run_now {
            let run = match self.account.as_deref().zip(self.run_in.as_deref()).filter(|(a, f)| s.folder(a, f).is_some()) {
                Some((account, folder)) => json!({"index": at, "account": account, "folder": folder}),
                None => json!({"index": at, "inbox": true}),
            };
            acts.push(Action::Run("rules.run", run));
        }
        Ok(acts)
    }

    pub(crate) fn from_rule(index: usize, r: &Rule) -> RuleForm {
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
            run_in: None,
            refused: r.refused.clone(),
            error: None,
        }
    }

    /// A rule the server keeps (made in Outlook, on a web page, in a Sieve script Quatrelle
    /// doesn't own): shown and changed from its parts, never re-ordered.
    pub(crate) fn from_server(account: &str, f: &ForeignRule) -> Option<RuleForm> {
        let r = f.rule.as_ref()?;
        Some(RuleForm {
            remote: Some((account.to_string(), f.id.clone())),
            account: Some(account.to_string()),
            ..RuleForm::from_rule(0, r).without_index()
        })
    }

    pub(crate) fn without_index(mut self) -> RuleForm {
        self.index = None;
        self
    }
}

/// A folder as the folder pane names it, with the folders it's in: `Inbox › gzamer`.
pub(crate) fn folder_path_label(s: &Session, path: &str) -> String {
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

/// One line describing a rule.
pub(crate) fn describe(s: &Session, r: &Rule) -> String {
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
