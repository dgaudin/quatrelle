// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A folder listing over the fake Graph server says which of the other mailboxes opened in the
//! account it couldn't list, so that what such a mailbox held isn't taken for gone.

use super::*;

/// A mailbox this account may not open is said (by its address), and only it: the one it may
/// open lists.
#[test]
fn a_graph_listing_says_which_mailbox_it_couldnt_open() {
    let support = "support@contoso.example";
    let boss = "boss@contoso.example";
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.shared = vec![support.into()];
    d.shared_folder(support, "Inbox", None, Some("inbox"));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let listed = crate::graph::list_folders(&c, &mut crate::jmap::State::default(), "a1", &[support.into(), boss.into()]).unwrap();
    assert_eq!(listed.missed, crate::graph::Missed { shared: vec![boss.into()], ..Default::default() });
    assert!(listed.folders.iter().any(|f| f.path == "support@contoso.example/Inbox"), "the others still list");
}
