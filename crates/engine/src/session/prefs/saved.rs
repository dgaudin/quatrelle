// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Reading the saved preferences setting by setting.

use super::*;

/// The most unreadable items of a list kept as they were (and named); more are let go.
pub(crate) const MAX_UNREAD_ITEMS: usize = 100;

/// Whether an item of a saved list reads: the kind of error when it doesn't.
type ItemCheck = fn(&serde_json::Value) -> Result<(), serde_json::Error>;

fn reads_as<T: serde::de::DeserializeOwned>(item: &serde_json::Value) -> Result<(), serde_json::Error> {
    T::deserialize(item).map(drop)
}

/// The lists of the preferences, by saved name, with what an item of each is.
const LISTS: &[(&str, ItemCheck)] = &[
    ("categories", reads_as::<quatrelle_model::organize::Category>),
    ("quickSteps", reads_as::<quick_steps::QuickStep>),
    ("searchFolders", reads_as::<search_folders::SearchFolder>),
    ("favorites", reads_as::<FolderRef>),
    ("webexRooms", reads_as::<WebexRoom>),
    ("hiddenCalendars", reads_as::<String>),
    ("knownCalendars", reads_as::<String>),
    ("removedCalendars", reads_as::<String>),
    ("trustedSenders", reads_as::<String>),
    ("trustedShareServers", reads_as::<String>),
    ("blockedSenders", reads_as::<String>),
    ("safeSenders", reads_as::<String>),
    ("ignored", reads_as::<String>),
    ("collapsed", reads_as::<String>),
    ("paneOrder", reads_as::<String>),
    ("spelling", reads_as::<String>),
    ("recentSearches", reads_as::<String>),
    ("recentEmoji", reads_as::<String>),
    ("orgDomains", reads_as::<String>),
    ("scamAccounts", reads_as::<String>),
];

impl Prefs {
    /// The preferences to save: as they are, with the items of lists that this version couldn't
    /// read ([`Prefs::from_saved`]) put back where they were, as they were.
    pub fn to_saved(&self) -> serde_json::Value {
        let mut saved = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        let Some(object) = saved.as_object_mut() else { return saved };
        for (name, items) in &self.unread_items {
            // An empty list is left out of the saved settings, and `None` favorites are null.
            let slot = object.entry(name.clone()).or_insert(serde_json::Value::Null);
            if !slot.is_array() {
                *slot = serde_json::Value::Array(Vec::new());
            }
            if let Some(list) = slot.as_array_mut() {
                for (at, raw) in items {
                    list.insert((*at).min(list.len()), raw.clone());
                }
            }
        }
        saved
    }

    /// Takes out of the saved lists the items that don't read here: they wait, as they were, in
    /// `held` by list and place, and are named (place and kind of error, never the value).
    fn hold_unreadable_items(
        object: &mut serde_json::Map<String, serde_json::Value>,
        held: &mut BTreeMap<String, Vec<(usize, serde_json::Value)>>,
        named: &mut Vec<String>,
    ) {
        for (name, check) in LISTS {
            let Some(list) = object.get_mut(*name).and_then(serde_json::Value::as_array_mut) else { continue };
            let mut readable = Vec::with_capacity(list.len());
            let mut kept = Vec::new();
            let mut let_go = 0usize;
            for (at, item) in std::mem::take(list).into_iter().enumerate() {
                match check(&item) {
                    Ok(()) => readable.push(item),
                    Err(e) if kept.len() < MAX_UNREAD_ITEMS => {
                        named.push(format!("{name}[{at}] ({:?})", e.classify()));
                        kept.push((at, item));
                    }
                    Err(_) => let_go += 1,
                }
            }
            if let_go > 0 {
                named.push(format!("{name}: {let_go} more items not kept"));
            }
            *list = readable;
            if !kept.is_empty() {
                held.insert((*name).to_string(), kept);
            }
        }
    }

    /// The preferences as the app saved them, from this version or another that names or types
    /// a setting otherwise: a setting that can't be read starts as it does for everyone (its name
    /// is in the answer), the others are kept. In a list (the categories, the Quick Steps…), the
    /// items that read are kept and each that doesn't is named (`list[place] (kind of error)`) and
    /// held as it was, to be saved again in its place ([`Prefs::to_saved`]). `None` when `saved`
    /// isn't preferences at all.
    pub fn from_saved(saved: &serde_json::Value) -> Option<(Prefs, Vec<String>)> {
        if let Ok(p) = serde_json::from_value::<Prefs>(saved.clone()) {
            return Some((p, Vec::new()));
        }
        let mut object = saved.as_object()?.clone();
        let mut held = BTreeMap::new();
        let mut unread = Vec::new();
        Prefs::hold_unreadable_items(&mut object, &mut held, &mut unread);
        let (mut p, names) = Prefs::from_saved_settings(object);
        p.unread_items = held;
        unread.extend(names);
        Some((p, unread))
    }

    /// Each setting read alone (the rest as they start), those that can be kept.
    fn from_saved_settings(object: serde_json::Map<String, serde_json::Value>) -> (Prefs, Vec<String>) {
        if let Ok(p) = serde_json::from_value::<Prefs>(serde_json::Value::Object(object.clone())) {
            return (p, Vec::new());
        }
        let mut kept = serde_json::Map::new();
        let mut unread = Vec::new();
        for (name, value) in object {
            let alone = serde_json::Value::Object(std::iter::once((name.clone(), value.clone())).collect());
            if serde_json::from_value::<Prefs>(alone).is_ok() {
                kept.insert(name, value);
            } else {
                unread.push(name);
            }
        }
        // Only a setting of Prefs can fail to read alone, so the names are never more than they are.
        match serde_json::from_value(serde_json::Value::Object(kept)) {
            Ok(p) => (p, unread),
            // Each read alone, and not all together (nothing known does that): none kept.
            Err(_) => (Prefs::default(), unread),
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

    fn saved_lists() -> serde_json::Value {
        json!({
            "keepDays": 250,
            "categories": [
                {"name": "Plans", "keyword": "plans", "color": 3},
                {"name": "Later", "keyword": "later", "color": {"a": "newer"}, "icon": "star"},
                {"name": "Done", "keyword": "done", "color": 5}
            ],
            "quickSteps": [{"name": "To the team", "actions": [], "complete": false, "unset": false}, {"kind": "from a newer version", "zoe": "zoe@studio.example"}],
            "trustedSenders": ["zoe@studio.example", 42],
            "favorites": [{"account": "a", "folder": "INBOX"}, "newer"]
        })
    }

    /// A list keeps the items that read; the one that doesn't is named by place and kind of error
    /// (never its value) and nothing else starts over.
    #[test]
    fn a_list_keeps_its_readable_items() {
        let (p, unread) = Prefs::from_saved(&saved_lists()).unwrap();
        assert_eq!(p.keep_days, 250);
        assert_eq!(p.categories.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Plans", "Done"]);
        assert_eq!(p.quick_steps.len(), 1);
        assert_eq!(p.trusted_senders.len(), 1);
        assert_eq!(p.favorites.as_ref().map(Vec::len), Some(1));
        let mut unread = unread;
        unread.sort();
        assert_eq!(unread, ["categories[1] (Data)", "favorites[1] (Data)", "quickSteps[1] (Data)", "trustedSenders[1] (Data)"]);
        assert!(unread.iter().all(|n| !n.contains("zoe") && !n.contains("newer")));
    }

    /// Saved again, the items this version couldn't read are where they were: what a newer version
    /// saved isn't lost, with the list as it was, edited, or emptied.
    #[test]
    fn unread_items_are_saved_again_as_they_were() {
        let saved = saved_lists();
        let (mut p, _) = Prefs::from_saved(&saved).unwrap();
        let again = p.to_saved();
        for key in ["categories", "quickSteps", "trustedSenders", "favorites"] {
            assert_eq!(again[key], saved[key], "{key} as it was read");
        }
        // The user changes the list: the unread item stays at its place, the change is made.
        p.categories.push(quatrelle_model::organize::Category::new("Extra", 1));
        p.categories.remove(0);
        let again = p.to_saved();
        assert_eq!(again["categories"][0]["name"], "Done");
        assert_eq!(again["categories"][1]["icon"], "star", "the unread one kept as it was, at its place");
        assert_eq!(again["categories"][2]["name"], "Extra");
        // The user empties a list: the unread item is still there (and the key, which an empty list leaves out).
        p.quick_steps.clear();
        assert_eq!(p.to_saved()["quickSteps"], json!([{"kind": "from a newer version", "zoe": "zoe@studio.example"}]));
        // What the next read makes of it is the same again.
        let (q, unread) = Prefs::from_saved(&p.to_saved()).unwrap();
        assert_eq!(q.categories, p.categories);
        assert_eq!(unread.len(), 4);
        // Preferences without any unread item save as they always did.
        let plain = Prefs::default();
        assert_eq!(plain.to_saved(), serde_json::to_value(&plain).unwrap());
    }

    /// A list full of unreadable items keeps a few, names a few, and never grows the answer.
    #[test]
    fn unread_items_are_bounded() {
        let items: Vec<_> = (0..5000).map(|i| json!({"n": i})).collect();
        let (p, unread) = Prefs::from_saved(&json!({"categories": items})).unwrap();
        assert!(p.categories.is_empty());
        assert!(p.unread_items["categories"].len() <= MAX_UNREAD_ITEMS);
        assert!(unread.len() <= MAX_UNREAD_ITEMS + 1, "{} names", unread.len());
    }
}
