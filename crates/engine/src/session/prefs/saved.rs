// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Reading the saved preferences setting by setting.

use super::*;

impl Prefs {
    /// The preferences as the app saved them, from this version or another that names or types
    /// a setting otherwise: a setting that can't be read starts as it does for everyone (its name
    /// is in the answer), the others are kept. `None` when `saved` isn't preferences at all.
    pub fn from_saved(saved: &serde_json::Value) -> Option<(Prefs, Vec<String>)> {
        if let Ok(p) = serde_json::from_value::<Prefs>(saved.clone()) {
            return Some((p, Vec::new()));
        }
        // One at a time: each read alone (the rest as they start), those that can be kept.
        let mut kept = serde_json::Map::new();
        let mut unread = Vec::new();
        for (name, value) in saved.as_object()? {
            let alone = serde_json::Value::Object(std::iter::once((name.clone(), value.clone())).collect());
            if serde_json::from_value::<Prefs>(alone).is_ok() {
                kept.insert(name.clone(), value.clone());
            } else {
                unread.push(name.clone());
            }
        }
        // Only a setting of Prefs can fail to read alone, so the names are never more than they are.
        match serde_json::from_value(serde_json::Value::Object(kept)) {
            Ok(p) => Some((p, unread)),
            // Each read alone, and not all together (nothing known does that): none kept.
            Err(_) => Some((Prefs::default(), unread)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Preferences saved by a version that typed one setting otherwise: that one starts as it
    /// does for everyone and is named, the others are kept; what isn't preferences at all is none.
    #[test]
    fn saved_preferences_keep_what_can_be_read() {
        let mut saved = serde_json::to_value(Prefs { conversations: true, keep_days: 250, ..Default::default() }).unwrap();
        assert_eq!(Prefs::from_saved(&saved).map(|(p, unread)| (p.keep_days, unread)), Some((250, Vec::new())));
        saved["readingPane"] = json!("somewhereNew");
        saved["compact"] = json!("very");
        saved["fromALaterVersion"] = json!({"any": "thing"});
        let (p, mut unread) = Prefs::from_saved(&saved).unwrap();
        unread.sort();
        assert_eq!(unread, ["compact", "readingPane"]);
        assert!(p.conversations && p.keep_days == 250);
        let d = Prefs::default();
        assert_eq!((p.reading_pane, p.compact), (d.reading_pane, d.compact));
        assert!(Prefs::from_saved(&json!("not preferences")).is_none());
        assert!(Prefs::from_saved(&json!([1, 2])).is_none());
    }

    /// The names that start over are never more than the settings there are.
    #[test]
    fn the_names_of_unread_settings_are_bounded() {
        let fields = serde_json::to_value(Prefs::default()).unwrap().as_object().map_or(0, |o| o.len());
        let mut saved = serde_json::Map::new();
        for (name, _) in serde_json::to_value(Prefs::default()).unwrap().as_object().unwrap() {
            saved.insert(name.clone(), json!({"neither": ["a", "setting"]}));
        }
        for i in 0..5000 {
            saved.insert(format!("unknown{i}"), json!(i));
        }
        let (_, unread) = Prefs::from_saved(&serde_json::Value::Object(saved)).unwrap();
        assert!(unread.len() <= fields, "{} names for {fields} settings", unread.len());
        assert!(unread.iter().all(|n| !n.starts_with("unknown")));
    }
}
