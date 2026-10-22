//! The mail providers Quatrelle knows by their servers — who signs in through the browser
//! rather than with a password, Microsoft's own addresses and its rules — and what Exchange
//! Online needs: Graph's address, Autodiscover, and an account's own app registration.

use crate::AccountConfig;

/// The EWS address of an account whose Exchange server Autodiscover finds when it connects.
pub const EWS_AUTODISCOVER: &str = "autodiscover";

/// Microsoft Graph's address, for Exchange Online accounts.
pub const GRAPH_URL: &str = "https://graph.microsoft.com/v1.0";

/// The OAuth provider that runs an IMAP server, when it's one Quatrelle knows.
pub fn oauth_provider_for(imap_host: &str) -> Option<&'static str> {
    let h = imap_host.trim().to_ascii_lowercase();
    if h == "imap.gmail.com" || h == "imap.googlemail.com" {
        Some("google")
    } else if h == "outlook.office365.com" || h.ends_with(".outlook.com") || h == "imap-mail.outlook.com" {
        Some("microsoft")
    } else {
        None
    }
}

/// Microsoft's own addresses (Outlook.com, Hotmail, Live, MSN). Since September 2024 they
/// take no passwords from other mail apps over IMAP, POP or SMTP: only Microsoft sign-in.
pub fn microsoft_consumer(email: &str) -> bool {
    let domain = email.trim().rsplit_once('@').map(|(_, d)| d.to_ascii_lowercase()).unwrap_or_default();
    let name = domain.split('.').next().unwrap_or_default();
    matches!(name, "outlook" | "hotmail" | "live" | "msn" | "windowslive" | "passport") && domain.contains('.')
}

/// Whether Microsoft sign-in can be offered for this account: a registration is built into this
/// copy (`available` lists the providers with one), or the account brings its own.
pub fn microsoft_sign_in_possible(a: &AccountConfig, available: &[String]) -> bool {
    available.iter().any(|p| p == "microsoft") || a.own_oauth_app()
}

/// Whether this could be an Application (client) ID of a Microsoft app registration (a GUID; any
/// short token of letters, digits, dashes and dots is taken, for test registrations).
pub fn valid_oauth_client_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.'))
}

/// Whether this could be a Microsoft sign-in tenant: `common`, `organizations`, `consumers`, a
/// directory's id (a GUID) or its domain (`contoso.onmicrosoft.com`).
pub fn valid_oauth_tenant(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && !s.starts_with(['.', '-'])
        && !s.ends_with('.')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.'))
}

/// Why a Microsoft account signing in with a password can't connect (its status starts with this).
pub const MICROSOFT_NO_PASSWORDS: &str = "Microsoft doesn't accept passwords from mail apps for this account: it needs Sign in with Microsoft";

/// Why an account signed in with Microsoft can't open its mailbox over IMAP: Microsoft took the
/// sign-in, then its server said "User is authenticated but not connected". The status starts
/// with this, so the UI can offer what helps (signing in again, or Microsoft Graph).
pub const MICROSOFT_IMAP_CLOSED: &str = "Microsoft signed you in, but its mail server won't open this mailbox over IMAP";

/// An account that signs in to Microsoft's servers with a password rather than Microsoft
/// sign-in, which those servers refuse.
pub fn microsoft_password_account(a: &AccountConfig) -> bool {
    a.oauth.is_none()
        && a.graph.is_none()
        && a.ews.is_none()
        && (oauth_provider_for(&a.imap.host) == Some("microsoft") || microsoft_consumer(&a.email))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microsofts_own_addresses_need_its_sign_in() {
        for e in ["a@outlook.com", "a@hotmail.com", "a@hotmail.co.uk", "a@live.nl", "a@msn.com", "A@Outlook.FR"] {
            assert!(microsoft_consumer(e), "{e}");
        }
        for e in ["a@outlook-studio.example", "a@studio.example", "a@hotmail", "hotmail.com", ""] {
            assert!(!microsoft_consumer(e), "{e}");
        }
        let mut a = AccountConfig::guess("someone@hotmail.com");
        assert!(microsoft_password_account(&a));
        assert_eq!(AccountConfig::guess("a@live.nl").imap.host, "outlook.office365.com");
        a.oauth = Some("microsoft".into());
        assert!(!microsoft_password_account(&a));
        assert!(!microsoft_password_account(&AccountConfig::guess("zoe@studio.example")));
        // A work address on Microsoft 365's servers, signing in with a password.
        let mut work = AccountConfig::guess("zoe@contoso.example");
        work.imap.host = "outlook.office365.com".into();
        assert!(microsoft_password_account(&work));
    }
}
