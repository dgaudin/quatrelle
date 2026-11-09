// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The Quick Step dialog: its marks, the follow-up it completes, the message it writes, where
//! it moves mail; and the setting of a ready-made step.

use quatrelle_engine::StepWrite;

use super::*;

pub(super) fn draw(ui: &mut Ui, f: &mut QuickStepForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(440.0);
    title(ui, t, if f.original.is_some() { "Edit Quick Step" } else { "New Quick Step" });
    let about = if f.set_up {
        tl!("Set up this Quick Step: it runs once saved.")
    } else {
        tl!("Done to the selected mail in one click; the first nine also with Ctrl+Shift+1 to 9.")
    };
    ui.add(egui::Label::new(RichText::new(about).color(t.dim)).wrap());
    ui.add_space(8.0);
    ui.label(RichText::new(tl!("Name")).color(t.dim).small());
    let r = ui.add(egui::TextEdit::singleline(&mut f.name).hint_text(tl!("Done")).desired_width(f32::INFINITY));
    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    ui.add_space(6.0);
    ui.checkbox(&mut f.read, tl!("Mark as read"));
    ui.checkbox(&mut f.flag, tl!("Flag"));
    ui.checkbox(&mut f.pin, tl!("Pin"));
    ui.checkbox(&mut f.complete, tl!("Mark complete"));
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
        ui.label(tl!("Then write"));
        egui::ComboBox::from_id_salt("quick-step-write").width(160.0).selected_text(f.write.label()).show_ui(ui, |ui| {
            for w in StepWriting::ALL {
                ui.selectable_value(&mut f.write, w, w.label());
            }
        });
        if f.write.needs_to() {
            ui.add(egui::TextEdit::singleline(&mut f.to).hint_text("name@example.com").desired_width(f32::INFINITY));
        }
    });
    ui.horizontal(|ui| {
        ui.label(tl!("Then move to"));
        egui::ComboBox::from_id_salt("quick-step-move").width(220.0).selected_text(QuickStepForm::move_label(&f.move_to)).show_ui(ui, |ui| {
            let mut choices =
                vec![StepMove::Stay, StepMove::Role(FolderRole::Archive), StepMove::Role(FolderRole::Trash), StepMove::Role(FolderRole::Junk)];
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
    let ready = f.ready();
    if let Some(old) = f.original.clone() {
        ui.add_space(6.0);
        if ui.button(RichText::new(tl!("Delete this Quick Step")).color(t.danger)).clicked() {
            acts.push(Action::Submit("quickStep.delete", json!({"name": old})));
        }
    }
    let (ok, cancel) = buttons(ui, t, if f.original.is_some() { "Save" } else { "Create" }, ready);
    if (ok || enter) && ready {
        acts.push(Action::Submit(if f.original.is_some() { "quickStep.edit" } else { "quickStep.create" }, f.params()));
    }
    !cancel
}

/// What a Quick Step writes after its marks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StepWriting {
    #[default]
    None,
    Reply,
    ReplyAll,
    Forward,
    NewMessage,
}

impl StepWriting {
    const ALL: [StepWriting; 5] = [Self::None, Self::Reply, Self::ReplyAll, Self::Forward, Self::NewMessage];

    fn label(self) -> &'static str {
        match self {
            Self::None => tl!("None"),
            Self::Reply => tl!("Reply"),
            Self::ReplyAll => tl!("Reply all"),
            Self::Forward => tl!("Forward to"),
            Self::NewMessage => tl!("New message to"),
        }
    }

    /// It goes to addresses typed beside it.
    fn needs_to(self) -> bool {
        matches!(self, Self::Forward | Self::NewMessage)
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
    /// Follow-up done.
    pub complete: bool,
    /// The message it opens, and (for a forward or a new message) whom it goes to.
    pub write: StepWriting,
    pub to: String,
    /// Opened to set up a ready-made step: once saved, it runs on the selection.
    pub set_up: bool,
    /// Setting it up asks where mail goes (a step that writes asks for the address instead).
    pub needs_folder: bool,
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
        f.complete = q.complete;
        match &q.write {
            None => {}
            Some(StepWrite::Reply) => f.write = StepWriting::Reply,
            Some(StepWrite::ReplyAll) => f.write = StepWriting::ReplyAll,
            Some(StepWrite::Forward { to }) => (f.write, f.to) = (StepWriting::Forward, to.clone()),
            Some(StepWrite::NewMessage { to }) => (f.write, f.to) = (StepWriting::NewMessage, to.clone()),
        }
        f
    }

    /// The setting of a ready-made step, at its first click: it asks for what it lacks (the
    /// address, or the folder) and runs once saved.
    pub fn set_up(s: &Session, q: &quatrelle_engine::QuickStep) -> QuickStepForm {
        let mut f = Self::edit(s, q);
        f.set_up = true;
        f.needs_folder = q.write.is_none();
        if f.needs_folder && f.move_to == StepMove::Stay {
            f.move_to = StepMove::Role(FolderRole::Archive);
        }
        f
    }

    /// Something to save: it does a thing, to whom it writes is known, the folder is chosen.
    pub fn ready(&self) -> bool {
        let does = !self.actions().is_empty() || self.complete || self.write != StepWriting::None;
        let to = !self.write.needs_to() || !self.to.trim().is_empty();
        let folder = !self.needs_folder || self.move_to != StepMove::Stay;
        !self.name.trim().is_empty() && does && to && folder
    }

    /// Its writing as the commands take it.
    fn writing(&self) -> Option<serde_json::Value> {
        let to = self.to.trim();
        match self.write {
            StepWriting::None => None,
            StepWriting::Reply => Some(json!("reply")),
            StepWriting::ReplyAll => Some(json!("replyAll")),
            StepWriting::Forward => Some(json!({"forward": {"to": to}})),
            StepWriting::NewMessage => Some(json!({"newMessage": {"to": to}})),
        }
    }

    /// The parameters of `quickStep.create`, or (when it has an `original`) `quickStep.edit`.
    pub fn params(&self) -> serde_json::Value {
        let mut p = json!({"name": self.name.trim(), "actions": self.actions(), "complete": self.complete});
        match &self.original {
            Some(old) => {
                p["name"] = json!(old);
                p["newName"] = json!(self.name.trim());
                p["write"] = self.writing().unwrap_or(serde_json::Value::Null);
            }
            None => {
                if let Some(w) = self.writing() {
                    p["write"] = w;
                }
            }
        }
        p
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
