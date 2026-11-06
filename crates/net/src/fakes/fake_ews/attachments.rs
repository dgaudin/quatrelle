// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The fake EWS server's attachments of calendar and contact items: an item lists them
//! (`Attachments`, each its id, name, type and size, never its bytes), and `GetAttachment` gives
//! them, a file's bytes (`Content`) and an attached item with its source when asked for it
//! (`IncludeMimeContent`), as Exchange does. An item keeps them as they were put in its element,
//! bytes included; the attachments asked for are counted.

use super::*;

/// What an item's list says of each attachment: the rest (its bytes, the attached item) only
/// `GetAttachment` gives.
const LISTED: [&str; 8] = ["AttachmentId", "Name", "ContentType", "ContentId", "Size", "LastModifiedTime", "IsInline", "IsContactPhoto"];

/// An item's element as `GetItem` gives it: its attachments listed without their bytes.
pub(super) fn listed(el: &mut El) {
    for list in el.kids.iter_mut().filter(|k| k.name == "Attachments") {
        for a in &mut list.kids {
            a.kids.retain(|k| LISTED.contains(&k.name.as_str()));
        }
    }
}

impl Data {
    /// `GetAttachment`'s answers, one for each id asked, in order: the attachment with its bytes
    /// (a contact's picture made without them gives three bytes of a JPEG), or not found.
    pub(super) fn get_attachment(&mut self, op: &El) -> Vec<String> {
        let mime = op.child("AttachmentShape").and_then(|s| s.text_of("IncludeMimeContent")).is_some_and(|v| v.trim() == "true");
        let asked: Vec<String> =
            op.child("AttachmentIds").map(|l| l.kids.iter().filter_map(|k| k.attr("Id").map(str::to_string)).collect()).unwrap_or_default();
        let mut out = Vec::new();
        for id in asked {
            self.attachments_given.push(id.clone());
            let found = self.pim.iter().find_map(|(_, _, el)| {
                el.children("Attachments")
                    .flat_map(|l| l.kids.iter())
                    .find(|a| a.child("AttachmentId").and_then(|i| i.attr("Id")) == Some(id.as_str()))
                    .cloned()
            });
            let Some(mut a) = found else {
                out.push(
                    "<m:R ResponseClass=\"Error\"><m:MessageText>The attachment could not be found.</m:MessageText>\
                     <m:ResponseCode>ErrorItemNotFound</m:ResponseCode></m:R>"
                        .to_string(),
                );
                continue;
            };
            if a.name == "FileAttachment" && a.child("Content").is_none() {
                a.kids.push(El { name: "Content".into(), text: "/9j/".into(), ..Default::default() });
            }
            // An attached item's source only when asked for.
            if !mime {
                for k in &mut a.kids {
                    k.kids.retain(|x| x.name != "MimeContent");
                }
            }
            out.push(format!(
                "<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode><m:Attachments>{}</m:Attachments></m:R>",
                xml(&a)
            ));
        }
        out
    }
}
