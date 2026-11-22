// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Exchange Web Services' answers, read as hostile XML: the folder list (`FindFolder`) and a
//! folder's changes (`SyncFolderItems`), the items a `FindItem` lists with their marks, response
//! codes and refusals, the directory (`ResolveNames`, `ExpandDL`, its certificates), the Inbox
//! rules (`GetInboxRules`, and the rule written back for it), reminders and archive tags. Never a
//! panic: counts, names and items stay within their caps; a refusal is an `Err`, not a value.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_net::ews;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let root = ews::parse(&text);
    // Every response message as a refusal, and the marks, reminders, tags and tips of the answer.
    for m in ews::messages(&root) {
        let _ = ews::check_message(m);
        let _ = ews::summary_listed(m);
    }
    let _ = ews::marks_listed(&root);
    let _ = ews::reminders_listed(&root);
    let _ = ews::server_major(&root);
    let _ = ews::tags_listed(&root);
    let _ = ews::mail_tips_read(&root);

    // The folder list: ids, shown names, parents and counts within their caps; Exchange's hidden
    // and system folders left out when the mailbox's own top folder is known.
    for top in [None, Some("root"), Some("AAMkFolder1")] {
        let (folders, _last) = ews::mailboxes_listed(&root, top);
        assert!(folders.len() <= 5000, "{} folders", folders.len());
        for f in &folders {
            assert!(f.name.chars().count() <= 255, "{:?}", f.name);
        }
    }

    // A folder's changes: a creation and an update carry their item, deletions and read marks an
    // id; no sync state is an error.
    if let Ok((changes, next, _last)) = ews::changes_listed(&root) {
        assert!(!next.is_empty());
        assert!(changes.len() <= 65_536, "{} changes", changes.len());
    }

    // The items a listing carries, with their marks, and each item's list entry.
    let _ = ews::items_listed(&root);
    let mut listings = Vec::new();
    root.all("Message", &mut listings);
    for it in listings.iter().take(256) {
        let summary = ews::summary_listed(it);
        assert!(summary.subject.chars().count() <= 64 * 1024);
    }

    // The directory: people and members each once, at most their caps, names and addresses cut.
    let (people, _more) = ews::people_listed(&root);
    assert!(people.len() <= 100, "{} people", people.len());
    for p in &people {
        assert!(p.name.chars().count() <= 200, "{:?}", p.name);
        assert!(p.address.len() <= 320, "{:?}", p.address);
        assert!(p.item_id.len() <= 512, "{:?}", p.item_id);
    }
    let (members, _more, total) = ews::members_listed(&root);
    assert!(members.len() <= 500 && total >= members.len(), "{} members of {total}", members.len());
    for c in ews::certificates_listed(&root, "zoe@studio.example") {
        assert!(!c.is_empty() && c.len() <= 64 << 10);
    }

    // The Inbox rules as the server keeps them, and the XML one goes back as.
    let rules = ews::rules_listed(&root);
    assert!(rules.len() <= 500, "{} rules", rules.len());
    for r in rules.iter().filter_map(|r| r.rule.as_ref()) {
        let xml = ews::rule_written(r, Some("inbox-id"), 1, r.remote_id.as_deref());
        let doc = ews::parse(&format!(
            "<soap:Envelope><soap:Body><m:ResponseMessages><m:GetInboxRulesResponseMessage>\
             <m:ResponseCode>NoError</m:ResponseCode><m:InboxRules>{xml}</m:InboxRules>\
             </m:GetInboxRulesResponseMessage></m:ResponseMessages></soap:Body></soap:Envelope>"
        ));
        let again = ews::rules_listed(&doc);
        assert_eq!(again.len(), 1, "a rule written and read back");
        if let Some(a) = again.first() {
            // XML 1.0 carries no control character: `esc` drops them, and a name is trimmed when
            // it is read, so compare what both keep.
            let clean = |s: &str| s.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string();
            assert_eq!(clean(&a.name), clean(&r.name));
            assert_eq!(a.enabled, r.enabled);
            assert!(a.rule.is_some());
        }
    }

    // Base64 in answers (a message source, a certificate, a picture) never makes more than its
    // text can.
    if let Some(bytes) = ews::unb64(&text) {
        assert!(bytes.len() <= text.len().saturating_mul(3) / 4 + 4);
    }
});
