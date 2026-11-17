// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The folders of files handed to other apps (and of pictures pasted in a Teams conversation's
//! box): the user's own, cleared as Quatrelle goes, and their old files let go at its start.

use std::time::{Duration, SystemTime};

use super::*;

/// Files handed to other apps go in a folder of the user's own, made when missing; on Linux
/// and the BSDs not in the shared /tmp, and owner-only.
#[test]
fn scratch_folders_are_the_users_own() {
    let a = scratch_dir("test-scratch").unwrap();
    assert!(a.is_dir());
    assert_eq!(scratch_dir("test-scratch"), Some(a.clone()), "the same folder each time");
    if cfg!(all(unix, not(target_os = "macos"))) {
        assert!(!a.starts_with("/tmp"), "{}", a.display());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let parent = a.parent().unwrap();
            assert_eq!(fs::metadata(parent).unwrap().permissions().mode() & 0o777, 0o700);
        }
    }
    let _ = fs::remove_dir(&a);
}

/// What is cleared of those folders when Quatrelle goes takes in the pictures pasted in a Teams
/// conversation's box.
#[test]
fn the_scratch_folders_cleared_take_in_teams() {
    assert!(SCRATCH.contains(&"teams"), "{SCRATCH:?}");
}

/// Quatrelle starting lets go of the pictures pasted more than a week ago, and only of them: a
/// message still waiting may name a younger one (one too big to go inside its message goes up as a
/// file, read when its turn comes); the folder, and what's in it that isn't a file, stay.
#[test]
fn only_pasted_pictures_older_than_a_week_go_at_the_start() {
    let dir = std::env::temp_dir().join(format!("quatrelle-remove-older-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("folder")).unwrap();
    let now = SystemTime::now();
    let aged = |name: &str, days: u64| {
        let path = dir.join(name);
        fs::write(&path, b"png").unwrap();
        fs::File::options().write(true).open(&path).unwrap().set_modified(now - Duration::from_secs(days * 86_400)).unwrap();
        path
    };
    let (new, six, eight) = (aged("pasted-1.png", 0), aged("pasted-2.png", 6), aged("pasted-3.png", 8));
    assert_eq!(PASTED_KEPT, Duration::from_secs(7 * 86_400));
    assert_eq!(remove_older(&dir, PASTED_KEPT, now), 1);
    assert!(new.exists() && six.exists(), "younger than a week: kept");
    assert!(!eight.exists(), "older: gone");
    assert!(dir.join("folder").is_dir(), "a folder in it stays");
    let _ = fs::remove_dir_all(&dir);
}
