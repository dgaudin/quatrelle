// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The saved settings: one that can't be read leaves the others as they were saved, and is named
//! in the log without its value.

use std::sync::{Mutex, Once, PoisonError};

use super::*;

/// What the log said about the saved settings, whichever test said it.
static SAID: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Lines;

impl log::Log for Lines {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, r: &log::Record) {
        let line = format!("{}", r.args());
        if line.contains("saved setting") {
            SAID.lock().unwrap_or_else(PoisonError::into_inner).push(line);
        }
    }

    fn flush(&self) {}
}

fn listen() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = log::set_logger(&Lines);
        log::set_max_level(log::LevelFilter::Trace);
    });
}

/// A saved state with settings the user made, one of them typed otherwise by another version.
fn saved_by_another_version() -> serde_json::Value {
    json!({
        "prefs": {
            "trustedSenders": ["zoe@studio.example"],
            "categories": [{"name": "Plans", "keyword": "plans", "color": 3}],
            "quickSteps": [{"name": "To the team", "actions": []}],
            "ctrlEnter": "send",
            "readingPane": {"somewhere": "new"},
        },
        "windows": "zoe@studio.example",
        "dark": true,
    })
}

#[test]
fn saved_settings_keep_what_can_be_read() {
    let (mut b, dir) = app("saved-settings-kept");
    let mut state = saved_by_another_version();
    state["prefs"]["quickSteps"] = json!([]);
    b.restore_state(&state);
    assert!(b.session.prefs.trusted_senders.contains("zoe@studio.example"), "approved senders kept");
    assert_eq!(b.session.prefs.categories.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Plans"]);
    assert_eq!(b.session.prefs.reading_pane, quatrelle_engine::Prefs::default().reading_pane, "the unread one starts over");
    // And what is saved next keeps them: nothing is rewritten to its defaults.
    let again = b.saved_state();
    assert_eq!(again["prefs"]["trustedSenders"], json!(["zoe@studio.example"]));
    assert_eq!(again["prefs"]["categories"][0]["name"], "Plans");
    assert_eq!(again["dark"], true);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ctrl_enter_survives_a_setting_it_doesnt_know() {
    let (mut a, dir) = app("saved-settings-ctrl-enter");
    a.restore_state(&json!({"prefs": {"ctrlEnter": "send", "readingPane": 42}}));
    assert_eq!(a.session.prefs.ctrl_enter, quatrelle_engine::CtrlEnter::Send);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_log_names_a_setting_and_never_its_value() {
    listen();
    let (mut a, dir) = app("saved-settings-log");
    a.restore_state(&json!({"prefs": {"readingPane": "zoe@studio.example"}, "windows": "zoe@studio.example", "listColumns": "zoe@studio.example"}));
    let said = SAID.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert!(said.iter().any(|l| l.contains("readingPane")), "the setting is named: {said:?}");
    assert!(said.iter().any(|l| l.contains("windows")), "the key is named: {said:?}");
    assert!(said.iter().all(|l| !l.contains("zoe@studio.example") && !l.contains("studio")), "no value in the log: {said:?}");
    let _ = std::fs::remove_dir_all(dir);
}
