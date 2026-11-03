// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The fake EWS server: a meeting's invitations or update go to someone. Asked to send them
//! (anything but `SendToNone`), Exchange refuses the change of an appointment with no guest to
//! send to, or with a guest it won't take, and leaves the item as it was.

use super::*;

impl Data {
    /// The refusal of an appointment (as it would be once created or changed) sent so, if any.
    pub(super) fn refuses_recipients(&self, item: &El, how: Option<&str>) -> Option<String> {
        if item.name != "CalendarItem" || how.is_none_or(|h| h == "SendToNone") {
            return None;
        }
        let mut found = Vec::new();
        for list in ["RequiredAttendees", "OptionalAttendees", "Resources"] {
            if let Some(l) = item.child(list) {
                l.all("EmailAddress", &mut found);
            }
        }
        // The mailbox itself isn't someone to send to.
        let me = format!("{USER}@");
        let guests: Vec<String> = found.iter().map(|e| e.text.trim().to_ascii_lowercase()).filter(|g| !g.starts_with(&me)).collect();
        let text = if guests.is_empty() {
            "At least one recipient isn't valid., A message can't be sent because it contains no recipients."
        } else if guests.iter().any(|g| self.refused_guests.contains(g)) {
            "At least one recipient isn't valid."
        } else {
            return None;
        };
        // A refused guest whose address starts `odd-` is answered under another code, the words
        // the same: the on-premises server's real code was never seen, only its message.
        let code = if guests.iter().any(|g| g.starts_with("odd-") && self.refused_guests.contains(g)) {
            "ErrorRecipientNotFound"
        } else {
            "ErrorInvalidRecipients"
        };
        Some(format!("<m:R ResponseClass=\"Error\"><m:MessageText>{text}</m:MessageText><m:ResponseCode>{code}</m:ResponseCode></m:R>"))
    }
}
