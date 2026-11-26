// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: the To-Do bar's parts share its height as dragged, kept with the preferences.

use super::*;

/// `view.todoBarShares` keeps the calendar's and People's shares of the bar's height (the tasks
/// take the rest), in percent; a share out of bounds, or two leaving the last part no room, is
/// refused and nothing changes.
#[test]
fn the_todo_bars_parts_keep_their_shares() {
    let mut s = sample_session("todo-bar-shares");
    assert_eq!(s.prefs.todo_bar_shares, None, "the parts as tall as they come, until dragged");
    let r = s.run("view.todoBarShares", &json!({"calendar": 30, "people": 15})).unwrap();
    assert_eq!((r["calendar"].as_u64(), r["people"].as_u64()), (Some(30), Some(15)));
    assert_eq!(s.prefs.todo_bar_shares, Some([30, 15]));
    for bad in [json!({"calendar": 2, "people": 15}), json!({"calendar": 60, "people": 40}), json!({"calendar": "a"})] {
        assert!(s.run("view.todoBarShares", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.prefs.todo_bar_shares, Some([30, 15]), "unchanged by a refusal");
    done(s);
}
