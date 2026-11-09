// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
//! Quick Steps, as Outlook's: several things done to the selected mail in one click (or with
//! Ctrl+Shift+1…9): mark it read, flag, pin or categorize it, then move, archive or delete it.

use quatrelle_model::organize::RuleAction;

use super::*;

/// The most Quick Steps kept, and actions in one.
pub const QUICK_STEPS_MAX: usize = 50;
const ACTIONS_MAX: usize = 10;
const NAME_MAX: usize = 100;
/// How many ready-made steps there are.
const SEEDED: usize = 4;

/// A named set of actions done to the selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickStep {
    pub name: String,
    /// In the order given, except that a move (to a folder, Archive, Deleted Items) comes last.
    pub actions: Vec<RuleAction>,
    /// Follow-up done, as Mark Complete (after the marks above, before the move).
    #[serde(default)]
    pub complete: bool,
    /// A message to write, opened for the person to finish (nothing is sent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<StepWrite>,
    /// A ready-made step not set up yet: it asks for its address or folder at the first click.
    #[serde(default)]
    pub unset: bool,
}

/// The message a Quick Step opens, in a window of its own, filled in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepWrite {
    Reply,
    ReplyAll,
    /// Forward the message to these addresses (as typed in a To box).
    Forward {
        to: String,
    },
    /// A new message to these addresses, from the message's account.
    NewMessage {
        to: String,
    },
}

impl Session {
    fn quick_step_at(&self, name: &str) -> Option<usize> {
        let name = name.trim();
        self.prefs.quick_steps.iter().position(|q| q.name.eq_ignore_ascii_case(name))
    }

    /// A Quick Step checked: a name not taken by another (`replacing`), actions that can be done
    /// (its categories and folders exist now), at most one move.
    fn checked_quick_step(
        &self,
        name: &str,
        actions: Vec<RuleAction>,
        complete: bool,
        mut write: Option<StepWrite>,
        replacing: Option<usize>,
    ) -> Result<QuickStep, CmdError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > NAME_MAX || name.chars().any(char::is_control) {
            return Err(CmdError::Params(format!("a Quick Step needs a name (up to {NAME_MAX} characters)")));
        }
        if self.quick_step_at(name).is_some_and(|i| Some(i) != replacing) {
            return Err(CmdError::Params(format!("there's already a Quick Step named `{name}`")));
        }
        if actions.len() > ACTIONS_MAX || (actions.is_empty() && !complete && write.is_none()) {
            return Err(CmdError::Params(format!("a Quick Step does 1 to {ACTIONS_MAX} things")));
        }
        if let Some(StepWrite::Forward { to } | StepWrite::NewMessage { to }) = &mut write {
            let list = quatrelle_model::addr::parse_list(to.trim()).map_err(|bad| CmdError::Params(format!("not an email address: {bad}")))?;
            if list.is_empty() {
                return Err(CmdError::Params("a Quick Step that writes to someone needs their address".into()));
            }
            *to = quatrelle_model::addr::format_field(&list);
        }
        let moves = actions.iter().filter(|a| matches!(a, RuleAction::MoveTo(_) | RuleAction::MoveToRole(_) | RuleAction::Delete)).count();
        if moves > 1 {
            return Err(CmdError::Params("a Quick Step moves mail to one place at most".into()));
        }
        for a in &actions {
            match a {
                RuleAction::Categorize(c) if self.category(c).is_none() => return Err(CmdError::Params(format!("no category `{c}`"))),
                RuleAction::MoveTo(f) if !self.accounts.iter().any(|a| a.folders.iter().any(|x| x.path == *f)) => {
                    return Err(CmdError::Params(format!("no folder `{f}`")));
                }
                RuleAction::MoveToRole(r) if !self.accounts.iter().any(|a| a.folder(*r).is_some()) => {
                    return Err(CmdError::Params("no account has that folder".into()));
                }
                _ => {}
            }
        }
        // The same action twice is done once.
        let mut once: Vec<RuleAction> = Vec::with_capacity(actions.len());
        for a in actions {
            if !once.contains(&a) {
                once.push(a);
            }
        }
        Ok(QuickStep { name: name.to_string(), actions: once, complete, write, unset: false })
    }

    pub fn create_quick_step(&mut self, name: &str, actions: Vec<RuleAction>, complete: bool, write: Option<StepWrite>) -> CmdResult {
        if self.prefs.quick_steps.len() >= QUICK_STEPS_MAX {
            return Err(CmdError::Failed(format!("there can be up to {QUICK_STEPS_MAX} Quick Steps")));
        }
        let q = self.checked_quick_step(name, actions, complete, write, None)?;
        self.prefs.quick_steps.push(q.clone());
        Ok(serde_json::json!({ "name": q.name, "index": self.prefs.quick_steps.len() - 1 }))
    }

    /// `write`: `Some(None)` takes the writing off.
    pub fn edit_quick_step(
        &mut self,
        name: &str,
        new_name: Option<&str>,
        actions: Option<Vec<RuleAction>>,
        complete: Option<bool>,
        write: Option<Option<StepWrite>>,
    ) -> CmdResult {
        let at = self.quick_step_at(name).ok_or_else(|| CmdError::Params(format!("no Quick Step `{}`", name.trim())))?;
        let old = self.prefs.quick_steps.get(at).cloned().ok_or_else(|| CmdError::Params("no such Quick Step".into()))?;
        let q = self.checked_quick_step(
            new_name.unwrap_or(&old.name),
            actions.unwrap_or(old.actions),
            complete.unwrap_or(old.complete),
            write.unwrap_or(old.write),
            Some(at),
        )?;
        if let Some(slot) = self.prefs.quick_steps.get_mut(at) {
            *slot = q.clone();
        }
        Ok(serde_json::json!({ "name": q.name }))
    }

    pub fn delete_quick_step(&mut self, name: &str) -> CmdResult {
        let at = self.quick_step_at(name).ok_or_else(|| CmdError::Params(format!("no Quick Step `{}`", name.trim())))?;
        let gone = self.prefs.quick_steps.remove(at);
        Ok(serde_json::json!({ "deleted": gone.name }))
    }

    /// Do a Quick Step (by name, or its place in the list) to the selection: its marks first,
    /// then its move, which Undo brings back.
    pub fn run_quick_step(&mut self, name: Option<&str>, index: Option<usize>) -> CmdResult {
        let at = match (name, index) {
            (Some(n), _) => self.quick_step_at(n).ok_or_else(|| CmdError::Params(format!("no Quick Step `{}`", n.trim())))?,
            (None, Some(i)) if i < self.prefs.quick_steps.len() => i,
            (None, Some(i)) => return Err(CmdError::Disabled(format!("there's no Quick Step {}", i + 1))),
            (None, None) => return Err(CmdError::Params("`name` or `index` is required".into())),
        };
        let q = self.prefs.quick_steps.get(at).cloned().ok_or_else(|| CmdError::Params("no such Quick Step".into()))?;
        if self.selection.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        if q.unset {
            return Err(CmdError::Params("set up this Quick Step first".into()));
        }
        // Before any mark: a step that moves does nothing on the online archive's mail.
        if self.quick_step_refused(&q.actions) {
            return Err(CmdError::Disabled(crate::ARCHIVE_READ_ONLY.into()));
        }
        if q.write.is_some() && self.selection.len() > 1 {
            return Err(CmdError::Disabled("a Quick Step that writes works on one message".into()));
        }
        let mut target: Option<MoveTarget> = None;
        for a in &q.actions {
            match a {
                RuleAction::MarkRead => {
                    self.flag_selection(Flag::Seen, Some(true))?;
                }
                RuleAction::Flag => {
                    self.flag_selection(Flag::Flagged, Some(true))?;
                }
                RuleAction::Pin => {
                    self.pin_selection(Some(true))?;
                }
                // A category deleted since is passed over.
                RuleAction::Categorize(c) => {
                    if self.category(c).is_some() {
                        self.categorize_selection(c, Some(true))?;
                    }
                }
                RuleAction::MoveTo(f) => target = Some(MoveTarget::Folder(f.clone())),
                RuleAction::MoveToRole(r) => target = Some(MoveTarget::Role(*r)),
                RuleAction::Delete => target = Some(MoveTarget::Role(FolderRole::Trash)),
            }
        }
        if q.complete {
            self.follow_up_selection(None, true)?;
        }
        let n = self.selection.len();
        if let Some(w) = &q.write {
            self.quick_step_writes(w)?;
        }
        if let Some(t) = target {
            self.move_selection(t)?;
        }
        Ok(serde_json::json!({ "name": q.name, "messages": n }))
    }

    /// The window a Quick Step opens, on the one selected message (opened first if it isn't
    /// the one being read), with whom it goes to when the step says.
    fn quick_step_writes(&mut self, w: &StepWrite) -> Result<(), CmdError> {
        let key = self.selection.first().cloned().ok_or_else(|| CmdError::Disabled("no message selected".into()))?;
        if self.reading.as_ref().map(|r| &r.key) != Some(&key) {
            self.show_message(&key, false);
        }
        match w {
            StepWrite::Reply => self.reply_compose(ReplyMode::Reply).map(drop)?,
            StepWrite::ReplyAll => self.reply_compose(ReplyMode::ReplyAll).map(drop)?,
            StepWrite::Forward { to } => {
                self.reply_compose(ReplyMode::Forward)?;
                self.address_compose(to)?;
            }
            StepWrite::NewMessage { to } => {
                self.new_compose(Some(&key.account))?;
                self.address_compose(to)?;
            }
        }
        // In a window of its own: the step moves the original, and the pane would lose it.
        if let Some(c) = self.compose.as_mut() {
            c.window = true;
        }
        Ok(())
    }

    fn address_compose(&mut self, to: &str) -> Result<(), CmdError> {
        let list = quatrelle_model::addr::parse_list(to).map_err(|bad| CmdError::Params(format!("not an email address: {bad}")))?;
        let c = self.compose.as_mut().ok_or_else(|| CmdError::Failed("compose".into()))?;
        c.to = quatrelle_model::addr::format_field(&list);
        Ok(())
    }

    /// The ready-made steps, added once (named by `names`, in the interface's language): they
    /// wait, `unset`, for the address or folder each needs. One already named so is passed over.
    pub fn seed_quick_steps(&mut self, names: &[String]) -> CmdResult {
        let step = |name: &str, actions: Vec<RuleAction>, complete: bool, write: Option<StepWrite>| QuickStep {
            name: name.to_string(),
            actions,
            complete,
            write,
            unset: true,
        };
        if self.prefs.quick_steps_seeded {
            return Ok(serde_json::json!({ "added": 0 }));
        }
        let [a, b, c, d] = names else {
            return Err(CmdError::Params(format!("`names` is {SEEDED} names")));
        };
        let steps = [
            step(a, Vec::new(), false, Some(StepWrite::Forward { to: String::new() })),
            step(b, Vec::new(), false, Some(StepWrite::NewMessage { to: String::new() })),
            step(c, vec![RuleAction::MarkRead], true, None),
            step(d, vec![RuleAction::Delete], false, Some(StepWrite::Reply)),
        ];
        let mut added = 0;
        for q in steps {
            let name = q.name.trim().to_string();
            let bad = name.is_empty() || name.chars().count() > NAME_MAX || name.chars().any(char::is_control);
            if bad || self.quick_step_at(&name).is_some() || self.prefs.quick_steps.len() >= QUICK_STEPS_MAX {
                continue;
            }
            self.prefs.quick_steps.push(QuickStep { name, ..q });
            added += 1;
        }
        self.prefs.quick_steps_seeded = true;
        Ok(serde_json::json!({ "added": added }))
    }
}
