// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Passwords and sign-ins in the system keychain, under the app's own name.

use quatrelle_engine::model::AccountConfig;
use quatrelle_net::worker::Credentials;

use crate::secrets;

/// Passwords in the system keychain (Windows Credential Manager, macOS Keychain, Secret Service).
pub(crate) struct Keychain;

pub(crate) const SERVICE: &str = "Quatrelle";

/// The keychain entry an account's password or sign-in is kept under.
fn entry(a: &AccountConfig) -> String {
    format!("{}|{}|{}", a.id, a.username, a.imap.host)
}

/// The outgoing server's own password, for an account whose SMTP sign-in is its own.
fn smtp_entry(a: &AccountConfig) -> String {
    format!("{}|smtp", a.id)
}

// Kept whatever their length (`secrets.rs`): a Microsoft 365 sign-in is longer than one Windows
// Credential Manager entry holds (issue #5).
impl Credentials for Keychain {
    fn get(&self, account: &AccountConfig) -> Option<String> {
        // A password kept before the rename is taken over: read from the fork's entry, put under
        // this app's name, and the old one removed only once the new one is there.
        secrets::load(SERVICE, &entry(account)).or_else(|| {
            let password = secrets::load(quatrelle_engine::store::legacy::OLD_KEYCHAIN, &entry(account))?;
            if secrets::store(SERVICE, &entry(account), &password).is_ok() {
                secrets::remove(quatrelle_engine::store::legacy::OLD_KEYCHAIN, &entry(account));
            }
            Some(password)
        })
    }
    fn set(&self, account: &AccountConfig, password: &str) -> Result<(), String> {
        secrets::store(SERVICE, &entry(account), password)
    }
    fn delete(&self, account: &AccountConfig) {
        secrets::remove(SERVICE, &entry(account));
    }
    fn get_smtp(&self, account: &AccountConfig) -> Option<String> {
        secrets::load(SERVICE, &smtp_entry(account)).or_else(|| {
            let password = secrets::load(quatrelle_engine::store::legacy::OLD_KEYCHAIN, &smtp_entry(account))?;
            if secrets::store(SERVICE, &smtp_entry(account), &password).is_ok() {
                secrets::remove(quatrelle_engine::store::legacy::OLD_KEYCHAIN, &smtp_entry(account));
            }
            Some(password)
        })
    }
    fn set_smtp(&self, account: &AccountConfig, password: &str) -> Result<(), String> {
        secrets::store(SERVICE, &smtp_entry(account), password)
    }
    fn delete_smtp(&self, account: &AccountConfig) {
        secrets::remove(SERVICE, &smtp_entry(account));
    }
    // Another secret of the account's (`smime`: its S/MIME certificate's private key).
    fn get_secret(&self, account: &AccountConfig, what: &str) -> Option<String> {
        secrets::load(SERVICE, &format!("{}|{what}", account.id))
    }
    fn set_secret(&self, account: &AccountConfig, what: &str, secret: &str) -> Result<(), String> {
        secrets::store(SERVICE, &format!("{}|{what}", account.id), secret)
    }
    fn delete_secret(&self, account: &AccountConfig, what: &str) {
        secrets::remove(SERVICE, &format!("{}|{what}", account.id));
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::secrets::one_at_a_time;

    /// Removes a keychain test's entries when it ends, failed or not.
    struct Cleanup<F: FnMut()>(F);

    impl<F: FnMut()> Drop for Cleanup<F> {
        fn drop(&mut self) {
            (self.0)()
        }
    }

    /// The entries of keychain test runs that were stopped before they could remove them (`test-`
    /// and a process id no longer running). 226 of them once filled Credential Manager until
    /// Windows refused any more (error 8), and every run after failed.
    fn remove_stopped_runs_entries() {
        use std::process::Command;
        let Ok(list) = Command::new("cmdkey").arg("/list").output() else { return };
        let suffix = format!(".{SERVICE}");
        for line in String::from_utf8_lossy(&list.stdout).lines() {
            let Some((_, target)) = line.split_once("target=") else { continue };
            let target = target.trim();
            let pid = target.strip_prefix("test-").and_then(|r| r.split('|').next()).and_then(|p| p.parse::<u32>().ok());
            let Some(pid) = pid.filter(|&p| p != std::process::id()) else { continue };
            // When tasklist can't say, the run is taken as still going: never another's entries.
            let stopped = Command::new("tasklist")
                .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
                .output()
                .is_ok_and(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")));
            if target.ends_with(&suffix) && stopped {
                let _ = Command::new("cmdkey").arg(format!("/delete:{target}")).output();
            }
        }
    }

    /// Issue #5: a Microsoft 365 sign-in (a refresh token of about 2,000 characters, and more
    /// with more claims) couldn't be kept in Windows' Credential Manager. Run here against the
    /// real keychain, under an entry of the test's own that it removes.
    #[test]
    fn a_long_sign_in_is_kept_in_the_keychain() {
        remove_stopped_runs_entries();
        let token: String = (0..2000).map(|i| char::from(b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz-_."[i * 7 % 65])).collect();
        let mut a = AccountConfig::guess("keychain-test@contoso.example");
        a.id = format!("test-{}", std::process::id());
        let k = Keychain;
        let _cleanup = Cleanup(|| k.delete(&a));
        let kept = k.set(&a, &token);
        let back = k.get(&a);
        k.delete(&a);
        assert_eq!(kept, Ok(()));
        assert_eq!(back.as_deref(), Some(token.as_str()));
        assert_eq!(k.get(&a), None, "removed with the account");
    }

    /// Trying to break it, on the real keychain: a password kept by an earlier version (UTF-16,
    /// one entry) still reads and is replaced; a secret growing and shrinking leaves no stale
    /// parts; a very long one and one in other scripts come back whole; removing leaves nothing,
    /// not even the parts a write that failed in an earlier version left.
    #[test]
    fn the_keychain_keeps_secrets_growing_shrinking_and_old() {
        remove_stopped_runs_entries();
        let user = format!("test-{}|grow|example", std::process::id());
        let _cleanup = Cleanup(|| secrets::remove(SERVICE, &user));
        let parts_left = || {
            let _turn = one_at_a_time();
            (2..=secrets::MAX_PARTS)
                .filter(|n| keyring::Entry::new(SERVICE, &format!("{user}|part{n}")).is_ok_and(|e| e.get_secret().is_ok()))
                .count()
        };
        // As an earlier Quatrelle kept it.
        {
            let _turn = one_at_a_time();
            keyring::Entry::new(SERVICE, &user).unwrap().set_password("old pässword").unwrap()
        };
        assert_eq!(secrets::load(SERVICE, &user).as_deref(), Some("old pässword"));
        let long = "t".repeat(7000);
        secrets::store(SERVICE, &user, &long).unwrap();
        assert_eq!(secrets::load(SERVICE, &user).as_deref(), Some(long.as_str()));
        assert_eq!(parts_left(), 2, "7000 bytes in three entries");
        secrets::store(SERVICE, &user, "short").unwrap();
        assert_eq!(secrets::load(SERVICE, &user).as_deref(), Some("short"));
        assert_eq!(parts_left(), 0, "the longer one's parts went");
        let huge: String = "密码🔑-".repeat(3000);
        secrets::store(SERVICE, &user, &huge).unwrap();
        assert_eq!(secrets::load(SERVICE, &user).as_deref(), Some(huge.as_str()));
        secrets::remove(SERVICE, &user);
        assert_eq!(secrets::load(SERVICE, &user), None);
        assert_eq!(parts_left(), 0, "every part removed");
        assert!(secrets::store(SERVICE, &user, &"x".repeat(200_000)).is_err(), "too long is refused, not cut");
        assert_eq!(secrets::load(SERVICE, &user), None, "and nothing half-kept");
        assert_eq!(parts_left(), 0, "no parts of the refused one either");
        // Parts with no first part to name them, as a write that failed partway left them in
        // earlier versions (127 of them filled Credential Manager): removing takes them too.
        for n in [2, 9, secrets::MAX_PARTS] {
            let _turn = one_at_a_time();
            keyring::Entry::new(SERVICE, &format!("{user}|part{n}")).unwrap().set_secret(b"orphan").unwrap();
        }
        assert_eq!(parts_left(), 3);
        secrets::remove(SERVICE, &user);
        assert_eq!(parts_left(), 0, "orphaned parts removed");
    }
}
