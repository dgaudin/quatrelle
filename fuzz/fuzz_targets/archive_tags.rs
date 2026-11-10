// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Exchange's list of retention policy tags (`GetUserRetentionPolicyTags`), read as hostile XML:
//! what comes out is only personal archive tags, with a GUID for an id, a name that can be
//! shown, a bounded number of days, once each, and no more than the cap.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let root = quatrelle_net::ews::parse(&String::from_utf8_lossy(data));
    let tags = quatrelle_net::ews::tags_listed(&root);
    assert!(tags.len() <= quatrelle_model::MAX_ARCHIVE_TAGS);
    for (i, t) in tags.iter().enumerate() {
        assert!(quatrelle_model::is_tag_id(&t.id), "{:?}", t.id);
        assert!(!t.name.is_empty() && t.name.chars().count() <= quatrelle_model::MAX_TAG_NAME && !t.name.chars().any(char::is_control), "{:?}", t.name);
        assert!(t.days.is_none_or(|d| d > 0 && d <= quatrelle_model::MAX_TAG_DAYS), "{:?}", t.days);
        assert!(t.never == t.days.is_none(), "{t:?}");
        assert!(!tags[..i].iter().any(|o| o.id == t.id), "{:?} twice", t.id);
    }
});
