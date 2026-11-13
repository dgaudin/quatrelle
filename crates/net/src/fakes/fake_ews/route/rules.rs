// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The fake EWS server's inbox rules: listed, and changed (made, set, deleted), refused while
//! Outlook's own copy of them is kept.

use super::*;

impl Data {
    /// An inbox rules operation answered; `None` for any other, which `route` answers.
    pub(super) fn rules_route(&mut self, op: &El) -> Option<Vec<String>> {
        let ok = |inner: String| format!("<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>{inner}</m:R>");
        let err =
            |code: &str| format!("<m:R ResponseClass=\"Error\"><m:MessageText>{code}</m:MessageText><m:ResponseCode>{code}</m:ResponseCode></m:R>");
        let mut out = Vec::new();
        match op.name.as_str() {
            "GetInboxRules" => {
                let list: String = self
                    .rules
                    .iter()
                    .enumerate()
                    .map(|(i, (id, name))| {
                        format!(
                            "<t:Rule><t:RuleId Id=\"{}\"/><t:DisplayName>{}</t:DisplayName><t:Priority>{}</t:Priority><t:IsEnabled>true</t:IsEnabled></t:Rule>",
                            esc(id),
                            esc(name),
                            i + 1
                        )
                    })
                    .collect();
                out.push(ok(format!("<m:OutlookRuleBlobExists>false</m:OutlookRuleBlobExists><m:InboxRules>{list}</m:InboxRules>")));
            }
            "UpdateInboxRules" if self.outlook_rules && op.text_of("RemoveOutlookRuleBlob") != Some("true") => {
                out.push(err("ErrorOutlookRuleBlobExists"));
            }
            "UpdateInboxRules" => {
                self.outlook_rules = false;
                let mut problem = None;
                if let Some(ops) = op.child("Operations") {
                    for o in &ops.kids {
                        match o.name.as_str() {
                            "CreateRuleOperation" => {
                                let Some(rule) = o.child("Rule") else { continue };
                                let name = rule.text_of("DisplayName").unwrap_or_default().to_string();
                                let id = self.id("R");
                                self.rules.push((id, name));
                            }
                            "SetRuleOperation" => {
                                let Some(rule) = o.child("Rule") else { continue };
                                let id = rule.child("RuleId").and_then(|r| r.attr("Id")).unwrap_or_default().to_string();
                                let name = rule.text_of("DisplayName").unwrap_or_default().to_string();
                                match self.rules.iter_mut().find(|(i, _)| *i == id) {
                                    Some(r) => r.1 = name,
                                    None => problem = Some("ErrorRuleNotFound"),
                                }
                            }
                            "DeleteRuleOperation" => {
                                let id = o.child("RuleId").and_then(|r| r.attr("Id")).unwrap_or_default().to_string();
                                self.rules.retain(|(i, _)| *i != id);
                            }
                            _ => {}
                        }
                    }
                }
                out.push(match problem {
                    Some(code) => err(code),
                    None => ok(String::new()),
                });
            }
            _ => return None,
        }
        Some(out)
    }
}
