// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The fake server's reminders (issue #40): `GetReminders` and `PerformReminderAction`, which
//! Exchange 2013 brought, answered in the response element itself as Exchange does; an older
//! server (it says it's version 14 in each answer's `ServerVersionInfo`) refuses them.

use super::*;

/// What the fake's reminders are and what was done to them.
#[derive(Default)]
pub struct Reminders {
    /// The reminders listed: (item id, its series' id or empty, its start as `2026-…Z`).
    pub listed: Vec<(String, String, String)>,
    /// Actions taken: (`Dismiss` or `Snooze`, item id, the new reminder time).
    pub done: Vec<(String, String, String)>,
    /// An Exchange older than 2013: both operations are refused.
    pub before_2013: bool,
    /// The `GetReminders` answer as given, in place of the listing (hostile answers).
    pub raw: Option<String>,
    /// The connection is dropped when they're asked (offline).
    pub unreachable: bool,
}

/// The reminders' operations.
pub(super) fn is_reminders(op: &str) -> bool {
    matches!(op, "GetReminders" | "PerformReminderAction")
}

impl Data {
    /// The `ServerVersionInfo` header every answer carries: Exchange 2016, or 2010.
    pub(super) fn version_header(&self) -> &'static str {
        if self.reminders.before_2013 {
            "<s:Header><h:ServerVersionInfo xmlns:h=\"t\" MajorVersion=\"14\" MinorVersion=\"3\" MajorBuildNumber=\"123\" MinorBuildNumber=\"3\"/></s:Header>"
        } else {
            "<s:Header><h:ServerVersionInfo xmlns:h=\"t\" MajorVersion=\"15\" MinorVersion=\"1\" MajorBuildNumber=\"2507\" MinorBuildNumber=\"6\" Version=\"V2017_07_11\"/></s:Header>"
        }
    }

    /// The answer to one of the reminders' operations under the version the request asked for:
    /// its HTTP status and body (`None`: the connection is dropped).
    pub(super) fn reminders_answer(&mut self, op: &El, version: &str) -> Option<(u16, String)> {
        if self.reminders.unreachable {
            return None;
        }
        let header = self.version_header();
        if self.reminders.before_2013 || !version.starts_with("Exchange2013") {
            return Some((
                500,
                format!(
                    "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\">{header}<s:Body><s:Fault>\
                     <faultcode>a:ErrorInvalidServerVersion</faultcode><faultstring>The specified server version is invalid.</faultstring>\
                     <detail><e:ResponseCode xmlns:e=\"e\">ErrorInvalidServerVersion</e:ResponseCode></detail></s:Fault></s:Body></s:Envelope>"
                ),
            ));
        }
        let (class, inner) = if op.name == "GetReminders" {
            let listed: String = self
                .reminders
                .listed
                .iter()
                .map(|(id, series, start)| {
                    let master = if series.is_empty() { String::new() } else { format!("<t:RecurringMasterItemId Id=\"{series}\" ChangeKey=\"m1\"/>") };
                    format!(
                        "<t:Reminder><t:Subject>Event</t:Subject><t:ReminderTime>{start}</t:ReminderTime><t:StartDate>{start}</t:StartDate>\
                         <t:EndDate>{start}</t:EndDate><t:ItemId Id=\"{id}\" ChangeKey=\"r1\"/>{master}<t:ReminderGroup>Calendar</t:ReminderGroup></t:Reminder>"
                    )
                })
                .collect();
            let list = self.reminders.raw.clone().unwrap_or_else(|| format!("<m:Reminders>{listed}</m:Reminders>"));
            ("Success", format!("<m:ResponseCode>NoError</m:ResponseCode>{list}"))
        } else {
            let mut answer = ("Success", "<m:ResponseCode>NoError</m:ResponseCode><m:UpdatedItemIds/>".to_string());
            for a in op.find("ReminderItemActions").map(|l| l.kids.clone()).unwrap_or_default() {
                let id = a.child("ItemId").and_then(|i| i.attr("Id")).unwrap_or_default().to_string();
                if !self.reminders.listed.iter().any(|(i, _, _)| *i == id) {
                    answer = ("Error", "<m:MessageText>The specified object was not found in the store.</m:MessageText><m:ResponseCode>ErrorItemNotFound</m:ResponseCode>".into());
                    break;
                }
                let action = a.text_of("ActionType").unwrap_or_default().to_string();
                let time = a.text_of("NewReminderTime").unwrap_or_default().to_string();
                self.reminders.done.push((action, id.clone(), time));
                self.reminders.listed.retain(|(i, _, _)| *i != id);
            }
            answer
        };
        let name = format!("{}Response", op.name);
        Some((
            200,
            format!(
                "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\">{header}<s:Body>\
                 <m:{name} ResponseClass=\"{class}\" xmlns:m=\"m\" xmlns:t=\"t\">{inner}</m:{name}></s:Body></s:Envelope>"
            ),
        ))
    }
}
