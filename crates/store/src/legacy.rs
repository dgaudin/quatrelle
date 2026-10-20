//! The names Quatrelle had when it was forked from MailCraft, read so a mailbox, its settings
//! and its passwords stay where they were. Everything else uses the new names: this is the only
//! place the old ones are written.
//!
//! Each is taken over once, at a start or when the thing it names is first needed: an old app
//! folder is renamed, an old data files folder too, an account's old data file is renamed when
//! its store opens, and a password is read from the old keychain entry and written back under
//! the new one. Nothing is deleted before its replacement is in place.

use std::path::{Path, PathBuf};

/// The per-user app folder (`%APPDATA%`, `~/Library/Application Support`, `$XDG_DATA_HOME`).
pub const OLD_APP: &str = "mailcraft";
/// The data files' folder in Documents.
pub const OLD_FILES: &str = "MailCraft Files";
/// An account's data file's extension.
pub const OLD_EXTENSION: &str = "mailcraft";
/// The marker file that asked for portable mode beside the executable.
pub const OLD_PORTABLE_MARKER: &str = "MailCraft.portable";
/// The folder portable mode kept everything in.
pub const OLD_PORTABLE_DIR: &str = "MailCraftData";
/// The keychain service passwords and sign-ins were kept under.
pub const OLD_KEYCHAIN: &str = "MailCraft";
/// What `QUATRELLE_DATA` was called.
pub const OLD_DATA_VAR: &str = "MAILCRAFT_DATA";
/// What `QUATRELLE_LANGUAGE` was called.
pub const OLD_LANGUAGE_VAR: &str = "MAILCRAFT_LANGUAGE";

/// `parent/new_name`, taking over `parent/old_name` when that's the only one there: renamed, so
/// a start finds everything where it was (settings, accounts, data files). A filesystem that
/// won't rename keeps the old folder in use rather than losing it.
pub fn take_over(parent: &Path, new_name: &str, old_name: &str) -> PathBuf {
    let target = parent.join(new_name);
    if target.exists() {
        return target;
    }
    let old = parent.join(old_name);
    if old.is_dir() {
        if std::fs::rename(&old, &target).is_ok() {
            return target;
        }
        log::info!("{} stays where it is: it couldn't be renamed to {}", old.display(), target.display());
        return old;
    }
    target
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("quatrelle-legacy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// An old folder is taken over, its files with it; a new one that's there is left alone.
    #[test]
    fn an_old_folder_is_taken_over_once() {
        let d = temp("folder");
        std::fs::create_dir_all(d.join(OLD_APP).join("mails")).unwrap();
        std::fs::write(d.join(OLD_APP).join("accounts.json"), b"[]").unwrap();
        let taken = take_over(&d, "Quatrelle", OLD_APP);
        assert_eq!(taken, d.join("Quatrelle"));
        assert!(taken.join("accounts.json").exists() && taken.join("mails").is_dir());
        assert!(!d.join(OLD_APP).exists(), "the old one is gone, renamed");
        // Again: nothing to do, the new one is used as it is.
        assert_eq!(take_over(&d, "Quatrelle", OLD_APP), d.join("Quatrelle"));
        // Neither there: the new name, to be made.
        let e = temp("empty");
        assert_eq!(take_over(&e, "Quatrelle", OLD_APP), e.join("Quatrelle"));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&e);
    }
}
