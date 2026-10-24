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

    /// Issue #5: a Microsoft 365 sign-in (a refresh token of about 2,000 characters, and more
    /// with more claims) couldn't be kept in Windows' Credential Manager. Run here against the
    /// real keychain, under an entry of the test's own that it removes.
    #[test]
    fn a_long_sign_in_is_kept_in_the_keychain() {
        let token: String = (0..2000).map(|i| char::from(b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz-_."[i * 7 % 65])).collect();
        let mut a = AccountConfig::guess("keychain-test@contoso.example");
        a.id = format!("test-{}", std::process::id());
        let k = Keychain;
        let kept = k.set(&a, &token);
        let back = k.get(&a);
        k.delete(&a);
        assert_eq!(kept, Ok(()));
        assert_eq!(back.as_deref(), Some(token.as_str()));
        assert_eq!(k.get(&a), None, "removed with the account");
    }

    /// Trying to break it, on the real keychain: a password kept by an earlier version (UTF-16,
    /// one entry) still reads and is replaced; a secret growing and shrinking leaves no stale
    /// parts; a very long one and one in other scripts come back whole; removing leaves nothing.
    #[test]
    fn the_keychain_keeps_secrets_growing_shrinking_and_old() {
        let user = format!("test-{}|grow|example", std::process::id());
        let parts_left = || {
            (2..=secrets::MAX_PARTS)
                .filter(|n| keyring::Entry::new(SERVICE, &format!("{user}|part{n}")).is_ok_and(|e| e.get_secret().is_ok()))
                .count()
        };
        // As an earlier Quatrelle kept it.
        keyring::Entry::new(SERVICE, &user).unwrap().set_password("old pässword").unwrap();
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
        secrets::remove(SERVICE, &user);
    }
}
