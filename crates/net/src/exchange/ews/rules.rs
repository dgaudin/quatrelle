//! EWS: the Inbox rules Exchange runs (`GetInboxRules`, `UpdateInboxRules`), read as the server keeps them and written from Quatrelle's.

use super::*;

/// A rule's id: the element's attribute in the schema, its text from a server that keeps the
/// older shape (Exchange 2016 does).
fn rule_id(r: &El) -> String {
    r.child("RuleId").map(|x| x.attr("Id").map(str::to_string).unwrap_or_else(|| x.text.trim().to_string())).unwrap_or_default()
}

/// A rule as the server keeps it, in Quatrelle's terms when every part of it can be said there.
pub(super) fn server_rule(r: &El) -> quatrelle_model::organize::ForeignRule {
    use quatrelle_model::organize::{Condition, ForeignRule, Rule, RuleAction};
    let id = rule_id(r);
    let name = r.text_of("DisplayName").unwrap_or("").trim().to_string();
    let enabled = r.text_of("IsEnabled") != Some("false");
    let strings = |el: &El| -> Vec<String> { el.children("String").map(|s| s.text.trim().to_string()).filter(|s| !s.is_empty()).collect() };
    let quoted = |v: &[String]| v.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(", ");
    let mut conditions = Vec::new();
    let mut actions = Vec::new();
    let mut stop = false;
    let mut runnable = true;
    let (mut when, mut then) = (Vec::new(), Vec::new());
    if let Some(c) = r.child("Conditions") {
        for k in &c.kids {
            match k.name.as_str() {
                "ContainsSubjectStrings" => {
                    let v = strings(k);
                    when.push(format!("subject {}", quoted(&v)));
                    match v.len() {
                        1 => conditions.push(Condition::Subject(v[0].clone())),
                        0 => runnable = false,
                        _ => conditions.push(Condition::SubjectAny(v)),
                    }
                }
                "ContainsSenderStrings" => {
                    let v = strings(k);
                    when.push(format!("sender {}", quoted(&v)));
                    match v.len() {
                        1 => conditions.push(Condition::From(v[0].clone())),
                        0 => runnable = false,
                        _ => conditions.push(Condition::FromAny(v)),
                    }
                }
                "ContainsRecipientStrings" => {
                    let v = strings(k);
                    when.push(format!("recipient {}", quoted(&v)));
                    match v.len() {
                        1 => conditions.push(Condition::To(v[0].clone())),
                        0 => runnable = false,
                        _ => conditions.push(Condition::ToAny(v)),
                    }
                }
                // "From these people" / "sent to these people": addresses as Exchange keeps
                // them, which Quatrelle reads by their address.
                "FromAddresses" | "SentToAddresses" => {
                    let who = k.name == "FromAddresses";
                    let people: Vec<String> = k
                        .children("Address")
                        .filter_map(|a| a.text_of("EmailAddress").map(|e| e.trim().to_string()))
                        .filter(|e| e.contains('@'))
                        .collect();
                    when.push(format!("{} {}", if who { "from" } else { "sent to" }, quoted(&people)));
                    match (who, people.len()) {
                        (true, 1) => conditions.push(Condition::From(people[0].clone())),
                        (false, 1) => conditions.push(Condition::To(people[0].clone())),
                        (true, 0) | (false, 0) => runnable = false,
                        (true, _) => conditions.push(Condition::FromAny(people)),
                        (false, _) => conditions.push(Condition::ToAny(people)),
                    }
                }
                "HasAttachments" if k.text.trim() == "true" => {
                    conditions.push(Condition::HasAttachment);
                    when.push("an attachment".into());
                }
                "ContainsBodyStrings" => {
                    // Quatrelle's rules look at the subject and the addresses, not the body.
                    when.push(format!("body {}", quoted(&strings(k))));
                    runnable = false;
                }
                other => {
                    when.push(other.to_string());
                    runnable = false;
                }
            }
        }
    }
    if let Some(a) = r.child("Actions") {
        for k in &a.kids {
            match k.name.as_str() {
                "Delete" if k.text.trim() == "true" => {
                    actions.push(RuleAction::Delete);
                    then.push("delete".to_string());
                }
                "MarkAsRead" if k.text.trim() == "true" => {
                    actions.push(RuleAction::MarkRead);
                    then.push("mark as read".to_string());
                }
                "AssignCategories" => {
                    for c in strings(k) {
                        actions.push(RuleAction::Categorize(c.clone()));
                        then.push(format!("categorise \"{c}\""));
                    }
                }
                "MoveToFolder" => {
                    // The folder by the id the server knows it by; the account's folders turn it
                    // into a path on the way in (see `worker::path_of_folder_id`).
                    let id = k.child("FolderId").and_then(|f| f.attr("Id")).unwrap_or("").to_string();
                    if id.is_empty() {
                        runnable = false;
                        then.push("move".to_string());
                    } else {
                        actions.push(RuleAction::MoveTo(format!("id:{id}")));
                        then.push("move to one of its folders".to_string());
                    }
                }
                "StopProcessingRules" if k.text.trim() == "true" => stop = true,
                other => {
                    then.push(other.to_string());
                    runnable = false;
                }
            }
        }
    }
    let when = if when.is_empty() { "every message".to_string() } else { when.join(" + ") };
    let then = if then.is_empty() { "nothing".to_string() } else { then.join(", ") };
    let summary = if stop { format!("{when} → {then}, then stops") } else { format!("{when} → {then}") };
    let rule = runnable.then(|| Rule {
        name: name.clone(),
        enabled,
        account: None,
        conditions,
        match_all: true,
        actions,
        stop,
        server: true,
        remote_id: Some(id.clone()),
        priority: r.text_of("Priority").and_then(|p| p.trim().parse().ok()),
        refused: None,
    });
    ForeignRule { account: String::new(), id, name, enabled, runnable, rule, summary }
}

/// An element as EWS XML, its names in the `t:` namespace (what a rule sent back must look like).
fn xml_of(el: &El) -> String {
    let attrs: String = el.attrs.iter().filter(|(k, _)| *k != "xmlns" && !k.contains(':')).map(|(k, v)| format!(" {k}=\"{}\"", esc(v))).collect();
    let kids: String = el.kids.iter().map(xml_of).collect();
    format!("<t:{}{attrs}>{}{}</t:{}>", el.name, esc(&el.text), kids, el.name)
}

/// The most rules Exchange answers with (a mailbox's rule list is small; a hostile one isn't
/// unbounded).
const MAX_RULES: usize = 500;

/// A rule test's values, as Exchange writes a list of them.
fn rule_strings(element: &str, values: &[String]) -> String {
    let list: String = values.iter().filter(|v| !v.trim().is_empty()).map(|v| format!("<t:String>{}</t:String>", esc(v))).collect();
    if list.is_empty() { String::new() } else { format!("<t:{element}>{list}</t:{element}>") }
}

/// A rule as Exchange's `InboxRules` hold it, for `CreateRuleOperation` / `SetRuleOperation`.
/// Only the parts Exchange can express are written; the engine never sends the others.
///
/// Written as the schema has it (`RulePredicatesType`, `RuleActionsType`): each test and action
/// once, in the schema's order, and the yes-or-no ones saying `true`. Exchange refuses anything
/// else with the whole update: `<t:Delete/>` (an empty boolean) or a sender test after a subject
/// test was enough for every rule sent from here to be turned down. The same test twice (two
/// "from" tests) becomes one list, which Exchange matches when any of it does.
pub(super) fn inbox_rule_xml(r: &Rule, folder: Option<&str>, priority: usize, id: Option<&str>) -> String {
    let mut x = String::from("<t:Rule>");
    if let Some(id) = id {
        x.push_str(&format!("<t:RuleId Id=\"{0}\">{0}</t:RuleId>", esc(id)));
    }
    x.push_str(&format!("<t:DisplayName>{}</t:DisplayName>", esc(&r.name)));
    // Its place among the server's rules: the one it had, so that changing it leaves the others
    // where they are (a new rule goes last).
    x.push_str(&format!("<t:Priority>{}</t:Priority>", r.priority.unwrap_or(priority as u32)));
    x.push_str(&format!("<t:IsEnabled>{}</t:IsEnabled>", r.enabled));
    let (mut senders, mut recipients, mut subjects, mut attachments) = (Vec::new(), Vec::new(), Vec::new(), false);
    for c in &r.conditions {
        match c {
            Condition::From(t) => senders.push(t.clone()),
            Condition::To(t) => recipients.push(t.clone()),
            Condition::Subject(t) => subjects.push(t.clone()),
            Condition::HasAttachment => attachments = true,
            // A list: any of them matches (Exchange's own "from these people").
            Condition::FromAny(v) => senders.extend(v.iter().cloned()),
            Condition::ToAny(v) => recipients.extend(v.iter().cloned()),
            Condition::SubjectAny(v) => subjects.extend(v.iter().cloned()),
            // Exchange knows neither: the engine keeps those rules at home.
            Condition::AnyText(_) | Condition::Bulk => {}
        }
    }
    let unique = |v: &mut Vec<String>| {
        let mut seen = std::collections::HashSet::new();
        v.retain(|t| seen.insert(t.trim().to_lowercase()));
    };
    for v in [&mut senders, &mut recipients, &mut subjects] {
        unique(v);
    }
    x.push_str("<t:Conditions>");
    x.push_str(&rule_strings("ContainsRecipientStrings", &recipients));
    x.push_str(&rule_strings("ContainsSenderStrings", &senders));
    x.push_str(&rule_strings("ContainsSubjectStrings", &subjects));
    if attachments {
        x.push_str("<t:HasAttachments>true</t:HasAttachments>");
    }
    x.push_str("</t:Conditions><t:Actions>");
    let mut categories: Vec<String> = r
        .actions
        .iter()
        .filter_map(|a| match a {
            RuleAction::Categorize(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    unique(&mut categories);
    x.push_str(&rule_strings("AssignCategories", &categories));
    let has = |want: fn(&RuleAction) -> bool| r.actions.iter().any(want);
    if has(|a| matches!(a, RuleAction::Delete)) {
        x.push_str("<t:Delete>true</t:Delete>");
    }
    if has(|a| matches!(a, RuleAction::MarkRead)) {
        x.push_str("<t:MarkAsRead>true</t:MarkAsRead>");
    }
    if let Some(folder) = folder.filter(|_| has(|a| matches!(a, RuleAction::MoveTo(_) | RuleAction::MoveToRole(_)))) {
        x.push_str(&format!("<t:MoveToFolder><t:FolderId Id=\"{}\"/></t:MoveToFolder>", esc(folder)));
    }
    if r.stop {
        x.push_str("<t:StopProcessingRules>true</t:StopProcessingRules>");
    }
    x.push_str("</t:Actions></t:Rule>");
    x
}

impl Client {
    /// The ids of the rules Exchange keeps for the Inbox, by name.
    pub fn inbox_rule_ids(&self) -> Result<Vec<(String, String)>> {
        Ok(self.inbox_rules()?.into_iter().map(|r| (r.name, r.id)).collect())
    }

    /// The rules the Inbox keeps, as the server has them: Quatrelle's own and the ones made
    /// elsewhere (Outlook, a web page), each with what it does — and whether Quatrelle knows
    /// every part of it (it can then run it here).
    pub fn inbox_rules(&self) -> Result<Vec<quatrelle_model::organize::ForeignRule>> {
        let doc = self.call("<m:GetInboxRules/>")?;
        let m = operation(&doc, "GetInboxRulesResponse")?;
        check_message(m)?;
        let mut out = Vec::new();
        if let Some(list) = m.find("InboxRules") {
            for r in list.children("Rule").take(MAX_RULES) {
                out.push(server_rule(r));
            }
        }
        Ok(out)
    }

    /// Turn one of the server's own rules on or off: the rule is sent back as the server keeps
    /// it, with only `IsEnabled` changed (a SetRuleOperation replaces the whole rule).
    pub fn set_rule_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let mut rule = self.rule_element(id)?.ok_or_else(|| NetError::No("that rule is no longer on the server".into()))?;
        rule.kids.retain(|k| k.name != "IsEnabled");
        rule.kids.push(El { name: "IsEnabled".into(), text: if enabled { "true".into() } else { "false".into() }, ..Default::default() });
        self.update_rules(&format!("<t:SetRuleOperation>{}</t:SetRuleOperation>", xml_of(&rule)))
    }

    /// Remove one of the server's own rules.
    pub fn delete_rule(&self, id: &str) -> Result<()> {
        self.update_rules(&format!("<t:DeleteRuleOperation><t:RuleId Id=\"{0}\">{0}</t:RuleId></t:DeleteRuleOperation>", esc(id)))
    }

    /// The element a rule is kept as, found by its id.
    pub(super) fn rule_element(&self, id: &str) -> Result<Option<El>> {
        let doc = self.call("<m:GetInboxRules/>")?;
        let m = operation(&doc, "GetInboxRulesResponse")?;
        check_message(m)?;
        Ok(m.find("InboxRules").and_then(|l| l.children("Rule").find(|r| rule_id(r) == id)).cloned())
    }

    /// The next change of the rules removes the copy Outlook keeps of them, which makes
    /// Exchange refuse every change while it's there (`RemoveOutlookRuleBlob`): the user agreed
    /// to lose the rules Outlook runs only on its computer.
    pub fn replace_outlook_rules(&self) {
        self.replace_outlook_rules.set(true);
    }

    /// An `UpdateInboxRules` of these operations, removing Outlook's copy when asked to.
    pub(super) fn update_request(&self, operations: &str) -> String {
        let remove = if self.replace_outlook_rules.get() { "<m:RemoveOutlookRuleBlob>true</m:RemoveOutlookRuleBlob>" } else { "" };
        format!("<m:UpdateInboxRules>{remove}<m:Operations>{operations}</m:Operations></m:UpdateInboxRules>")
    }

    /// One `UpdateInboxRules`, its answer checked.
    pub(super) fn update_rules(&self, operations: &str) -> Result<()> {
        let doc = self.call(&self.update_request(operations))?;
        let messages = messages(&doc);
        if messages.is_empty() {
            check_message(operation(&doc, "UpdateInboxRulesResponse")?)?;
        }
        for m in messages {
            check_message(m)?;
        }
        // Done once: Outlook's copy is gone.
        self.replace_outlook_rules.set(false);
        Ok(())
    }

    /// Create, change and remove the Inbox rules Exchange runs, from Quatrelle's rules: each is
    /// paired with the id of the folder its action moves mail to (`None` for actions that need
    /// none). Answers the id every rule has on the server, by its name.
    ///
    /// Only what Exchange can express is sent ([`Rule::server_ready`]): the engine leaves the
    /// rest at home. A rule the server has never seen is created; one carrying a `remote_id` is
    /// changed; `delete` names the ids to remove, and only ever ones Quatrelle created.
    pub fn set_inbox_rules(&self, wanted: &[(Rule, Option<String>)], delete: &[String]) -> Result<Vec<(String, String)>> {
        if wanted.is_empty() && delete.is_empty() {
            return Ok(Vec::new());
        }
        // What the server has before: a rule it just created is the one it didn't have.
        let before: Vec<String> = self.inbox_rule_ids()?.into_iter().map(|(_, id)| id).collect();
        let mut ops = String::new();
        for (i, (r, folder)) in wanted.iter().enumerate() {
            let known = r.remote_id.as_deref().filter(|id| !id.is_empty());
            let rule = inbox_rule_xml(r, folder.as_deref(), i + 1, known);
            if known.is_some() {
                ops.push_str(&format!("<t:SetRuleOperation>{rule}</t:SetRuleOperation>"));
            } else {
                ops.push_str(&format!("<t:CreateRuleOperation>{rule}</t:CreateRuleOperation>"));
            }
        }
        for id in delete.iter().filter(|id| !id.is_empty()) {
            // The id travels both ways: as the attribute the schema names and as the text an
            // older answer shape uses (this Exchange 2016 writes the text).
            ops.push_str(&format!("<t:DeleteRuleOperation><t:RuleId Id=\"{0}\">{0}</t:RuleId></t:DeleteRuleOperation>", esc(id)));
        }
        let doc = self.call(&self.update_request(&ops))?;
        // As with GetInboxRules: a server may answer with the response element itself.
        let messages = messages(&doc);
        if messages.is_empty() {
            check_message(operation(&doc, "UpdateInboxRulesResponse")?)?;
        }
        for m in messages {
            check_message(m)?;
        }
        self.replace_outlook_rules.set(false);
        // Read the ids back: creating a rule answers none. A rule we sent without one is the
        // rule that appeared, under that name.
        let after = self.inbox_rule_ids()?;
        let mut out = Vec::new();
        for (r, _) in wanted {
            let id = match r.remote_id.as_deref().filter(|id| !id.is_empty()) {
                Some(id) => Some(id.to_string()),
                None => after.iter().find(|(name, id)| name == &r.name && !before.contains(id)).map(|(_, id)| id.clone()),
            };
            match id {
                Some(id) => out.push((r.name.clone(), id)),
                None => log::warn!("the rule {} was sent to Exchange but isn't listed there", r.name),
            }
        }
        Ok(out)
    }
}
