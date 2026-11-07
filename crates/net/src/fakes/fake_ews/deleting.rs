// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What the fake EWS server keeps of the mail deleted (`DeleteItem`, `EmptyFolder`), as
//! Exchange does by the request's `DeleteType`: `SoftDelete` puts it in Recoverable Items
//! (Deletions), where Recover Deleted Items in Outlook and Outlook on the web brings it back
//! while the retention lasts; `HardDelete` keeps nothing the user can reach (Purges).

use super::*;

impl Data {
    /// Take the items `gone` picks out of their folders, as `op` asks.
    pub(super) fn take_items(&mut self, op: &El, gone: impl Fn(&It) -> bool) {
        let (taken, kept): (Vec<It>, Vec<It>) = std::mem::take(&mut self.items).into_iter().partition(|i| gone(i));
        self.items = kept;
        match op.attr("DeleteType") {
            Some("SoftDelete") => self.recoverable.extend(taken),
            Some("HardDelete") => self.purged.extend(taken),
            _ => {}
        }
    }
}

impl Data {
    /// What `FindItem` answers for the Recoverable Items folder: its items a page at a time, newest
    /// first, with the fields a list of them shows (or the server's refusal, `refuse_recoverable`).
    pub(super) fn recoverable_page(&self, op: &El) -> String {
        if self.refuse_recoverable {
            return "<m:R ResponseClass=\"Error\"><m:MessageText>not supported</m:MessageText><m:ResponseCode>ErrorInvalidServerVersion</m:ResponseCode></m:R>".into();
        }
        let view = op.child("IndexedPageItemView");
        let max: usize = view.and_then(|v| v.attr("MaxEntriesReturned")).and_then(|o| o.parse().ok()).unwrap_or(1000).min(1000);
        let offset: usize = view.and_then(|v| v.attr("Offset")).and_then(|o| o.parse().ok()).unwrap_or(0);
        let mut list: Vec<&It> = self.recoverable.iter().collect();
        list.sort_by(|a, b| b.received.cmp(&a.received));
        let last = offset + max >= list.len();
        let items: String = list
            .iter()
            .skip(offset)
            .take(max)
            .map(|i| {
                let x = self.item_xml(i, false, false);
                let class = if i.class.is_empty() { "IPM.Note" } else { i.class.as_str() };
                x.replace(
                    "</t:Message>",
                    &format!("<t:ItemClass>{class}</t:ItemClass><t:LastModifiedTime>{}</t:LastModifiedTime></t:Message>", i.received),
                )
            })
            .collect();
        format!(
            "<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode><m:RootFolder TotalItemsInView=\"{}\" IncludesLastItemInRange=\"{last}\"><t:Items>{items}</t:Items></m:RootFolder></m:R>",
            list.len()
        )
    }
}
