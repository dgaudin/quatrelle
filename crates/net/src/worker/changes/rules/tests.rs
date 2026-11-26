// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin

use super::*;

/// A server full of scripts, each full of rules, can't fill the dialog and the session without
/// bound: an account's rules made elsewhere are capped in all, the rest said in one entry.
#[test]
fn the_rules_made_elsewhere_are_capped_in_all() {
    let script = "if header :contains \"subject\" \"x\" { discard; }\n".repeat(1_000);
    let names: Vec<String> = (0..64).map(|i| format!("s{i}")).collect();
    let bodies: Vec<(String, String)> = names.iter().map(|n| (n.clone(), script.clone())).collect();
    let rules = foreign_rules("a1", &names, &bodies, |_| true, &[]);
    assert!(rules.len() <= MAX_FOREIGN_RULES + 1, "{} rules came through", rules.len());
    let last = rules.last().expect("some rules");
    assert!(!last.runnable && last.rule.is_none(), "the rest is one entry that can't be run: {last:?}");
    assert!(last.name.contains(&(64 * 1_000 - MAX_FOREIGN_RULES).to_string()), "it says how many: {}", last.name);
    // Under the cap nothing is said.
    let small = vec![("s0".to_string(), script.lines().take(10).collect::<Vec<_>>().join("\n"))];
    let few = foreign_rules("a1", &names[..1], &small, |_| true, &[]);
    assert_eq!(few.len(), 10, "{few:?}");
}
