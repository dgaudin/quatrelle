// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Rule lines (`<hr>`): a tag ends where a browser ends it, a `>` in a quoted attribute and
//! all.

use super::*;

/// A `>` inside a quoted attribute doesn't end the tag: the rest of an `<hr title="x>y">` isn't
/// left in the text after its rule line.
#[test]
fn a_gt_inside_a_quoted_attribute_doesnt_end_a_rule_tag() {
    assert_eq!(mark_rules("<p>a</p><hr title=\"x>y\"><p>b</p>"), format!("<p>a</p><p>{RULE_MARK}</p><p>b</p>"));
}
