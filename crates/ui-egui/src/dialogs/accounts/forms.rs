//! The account forms: adding an account (every kind of server), and an existing one's settings.

use super::*;

#[derive(Clone, Debug, Default)]
pub struct AccountForm {
    pub email: String,
    pub name: String,
    pub password: String,
    pub advanced: bool,
    /// Server fields follow the address until the user edits them.
    pub servers_edited: bool,
    pub username: String,
    pub imap_host: String,
    pub imap_port: String,
    pub imap_security: Security,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_security: Security,
    /// The outgoing server's own user name and password.
    pub smtp_sign_in: SmtpSignIn,
    /// Syncs over JMAP instead of IMAP and SMTP, through this session address.
    pub jmap: bool,
    pub jmap_url: String,
    /// Exchange Online through Microsoft Graph (signs in with Microsoft in the browser).
    pub graph: bool,
    /// An organisation's own Exchange server through EWS, at this address or server name.
    pub ews: bool,
    pub ews_url: String,
    /// The User-Agent EWS requests name; empty for Quatrelle's.
    pub user_agent: String,
    /// Downloads mail over POP3 into folders on this computer (sends over SMTP).
    pub pop: bool,
    pub pop_host: String,
    pub pop_port: String,
    pub pop_security: Security,
    /// POP: when the server's copy of downloaded mail goes.
    pub pop_keep: PopKeep,
    /// POP: an existing data file (Outlook's `.pst` or Quatrelle's own `.quatrelle`) whose mail comes in first.
    pub pst: String,
    /// Keep none of its mail on this computer (issue #4; not for POP). Until it's chosen, as
    /// Quatrelle runs: on when portable.
    pub online_only: Option<bool>,
    pub error: Option<String>,
    /// The address whose looked-up settings are filled in, and where they came from.
    pub found_for: Option<String>,
    pub found_source: Option<String>,
    /// Microsoft sign-in through the organisation's own app registration: its Application
    /// (client) ID and tenant (empty: the one built into this copy, at `common`).
    pub client_id: String,
    pub tenant: String,
}

/// The outgoing server's own sign-in, as Outlook's "Log on using" for SMTP: for servers whose
/// user name and password aren't the incoming server's (issue #3).
#[derive(Clone, Debug, Default)]
pub struct SmtpSignIn {
    pub own: bool,
    pub username: String,
    pub password: String,
    /// Its password is kept already (an account being changed): left empty, it stays.
    pub kept: bool,
}

impl SmtpSignIn {
    pub(crate) fn of(c: &AccountConfig) -> SmtpSignIn {
        SmtpSignIn {
            own: c.smtp_username.is_some(),
            username: c.smtp_username.clone().unwrap_or_default(),
            password: String::new(),
            kept: c.smtp_username.is_some(),
        }
    }

    /// `smtpUsername` (null: the incoming server's sign-in) and `smtpPassword`.
    pub(crate) fn params(&self, p: &mut serde_json::Value) {
        if self.own {
            p["smtpUsername"] = json!(self.username.trim());
            if !self.password.is_empty() {
                p["smtpPassword"] = json!(self.password);
            }
        } else {
            p["smtpUsername"] = serde_json::Value::Null;
        }
    }

    pub(crate) fn ui(&mut self, ui: &mut Ui, w: f32) {
        ui.checkbox(&mut self.own, tl!("The outgoing server has its own user name and password"));
        if self.own {
            ui.horizontal(|ui| {
                let half = ((w - 30.0) / 2.0).max(100.0);
                ui.add_space(22.0);
                ui.add_sized([half, 26.0], egui::TextEdit::singleline(&mut self.username).hint_text(tl!("Its user name")));
                let hint = if self.kept { tl!("Password (unchanged when empty)") } else { tl!("Its password") };
                ui.add_sized([half, 26.0], egui::TextEdit::singleline(&mut self.password).password(true).hint_text(hint));
            });
        }
    }
}

/// When a POP server's copy of downloaded mail goes: kept (for so many days, or until it's deleted
/// here for good), or removed at once.
#[derive(Clone, Debug)]
pub struct PopKeep {
    pub keep: bool,
    pub for_days: bool,
    pub days: String,
    pub until_deleted: bool,
}

impl Default for PopKeep {
    /// A copy stays on the server for 14 days.
    fn default() -> Self {
        PopKeep { keep: true, for_days: true, days: "14".into(), until_deleted: false }
    }
}

impl PopKeep {
    pub(crate) fn of(c: &AccountConfig) -> PopKeep {
        PopKeep {
            keep: !c.pop_delete,
            for_days: c.pop_keep_days.is_some(),
            days: c.pop_keep_days.unwrap_or(14).to_string(),
            until_deleted: c.pop_delete_with_trash,
        }
    }

    /// `popDelete`, `popKeepDays` (0: for good) and `popDeleteWithTrash`.
    pub(crate) fn params(&self, p: &mut serde_json::Value) {
        let days = if self.keep && self.for_days { self.days.trim().parse::<u32>().ok().filter(|d| *d > 0).unwrap_or(14) } else { 0 };
        p["popDelete"] = json!(!self.keep);
        p["popKeepDays"] = json!(days);
        p["popDeleteWithTrash"] = json!(self.keep && self.until_deleted);
    }

    pub(crate) fn ui(&mut self, ui: &mut Ui) {
        ui.checkbox(&mut self.keep, tl!("Keep a copy of downloaded mail on the server"));
        ui.add_enabled_ui(self.keep, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                ui.checkbox(&mut self.for_days, tl!("Remove it from the server after"));
                ui.add_enabled(self.for_days, egui::TextEdit::singleline(&mut self.days).desired_width(36.0).align(egui::Align2::CENTER_TOP));
                ui.label(tl!("days"));
            });
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                ui.checkbox(&mut self.until_deleted, tl!("Remove it from the server once it's deleted from Deleted Items"));
            });
        });
    }
}

impl AccountForm {
    pub(crate) fn guess(&mut self) {
        let g = AccountConfig::guess(&self.email);
        self.imap_host = g.imap.host;
        self.imap_port = g.imap.port.to_string();
        self.imap_security = g.imap.security;
        self.smtp_host = g.smtp.host;
        self.smtp_port = g.smtp.port.to_string();
        self.smtp_security = g.smtp.security;
        self.guess_pop();
    }

    /// The POP server named after the IMAP one (pop.example.com for imap.example.com).
    pub(crate) fn guess_pop(&mut self) {
        self.pop_host = match self.imap_host.strip_prefix("imap.") {
            Some(rest) => format!("pop.{rest}"),
            None => self.imap_host.clone(),
        };
        self.pop_port = "995".into();
        self.pop_security = Security::Tls;
    }

    pub(crate) fn params(&self) -> serde_json::Value {
        let mut p = self.server_params();
        if self.online_only == Some(true) && !self.pop {
            p["onlineOnly"] = json!(true);
        }
        // The organisation's own Microsoft registration, when one was given.
        if !self.client_id.trim().is_empty() {
            p["oauthClientId"] = json!(self.client_id.trim());
        }
        if !self.tenant.trim().is_empty() {
            p["oauthTenant"] = json!(self.tenant.trim());
        }
        p
    }

    /// Whether the account would sign in with Microsoft: Exchange Online, or an IMAP server of
    /// Microsoft's (so its own registration can be given).
    pub(crate) fn signs_in_with_microsoft(&self) -> bool {
        self.graph
            || (!self.jmap
                && !self.ews
                && !self.pop
                && (quatrelle_engine::model::oauth_provider_for(&self.imap_host) == Some("microsoft")
                    || quatrelle_engine::model::microsoft_consumer(&self.email)))
    }

    pub(crate) fn server_params(&self) -> serde_json::Value {
        let port = |s: &str| s.trim().parse::<u64>().unwrap_or(0);
        if self.graph {
            return json!({"email": self.email.trim(), "name": self.name.trim(), "graph": true});
        }
        if self.ews {
            return json!({
                "email": self.email.trim(),
                "name": self.name.trim(),
                "password": self.password,
                "username": if self.username.trim().is_empty() { self.email.trim() } else { self.username.trim() },
                // No server named: Autodiscover finds it.
                "ews": if self.ews_url.trim().is_empty() { json!(true) } else { json!(self.ews_url.trim()) },
                "userAgent": self.user_agent.trim(),
            });
        }
        if self.jmap {
            return json!({
                "email": self.email.trim(),
                "name": self.name.trim(),
                "password": self.password,
                "username": if self.username.trim().is_empty() { self.email.trim() } else { self.username.trim() },
                "jmap": self.jmap_url.trim(),
            });
        }
        if self.pop {
            let mut p = json!({
                "email": self.email.trim(),
                "name": self.name.trim(),
                "password": self.password,
                "username": if self.username.trim().is_empty() { self.email.trim() } else { self.username.trim() },
                "pop": true,
                "popHost": self.pop_host.trim(),
                "popPort": port(&self.pop_port),
                "popSecurity": self.pop_security.code(),
                "smtpHost": self.smtp_host.trim(),
                "smtpPort": port(&self.smtp_port),
                "smtpSecurity": self.smtp_security.code(),
            });
            self.pop_keep.params(&mut p);
            self.smtp_sign_in.params(&mut p);
            if !self.pst.trim().is_empty() {
                p["pst"] = json!(self.pst.trim());
            }
            return p;
        }
        let mut p = json!({
            "email": self.email.trim(),
            "name": self.name.trim(),
            "password": self.password,
            "username": if self.username.trim().is_empty() { self.email.trim() } else { self.username.trim() },
            "imapHost": self.imap_host.trim(),
            "imapPort": port(&self.imap_port),
            "imapSecurity": self.imap_security.code(),
            "smtpHost": self.smtp_host.trim(),
            "smtpPort": port(&self.smtp_port),
            "smtpSecurity": self.smtp_security.code(),
        });
        self.smtp_sign_in.params(&mut p);
        p
    }
}

/// The server settings of an existing account.
#[derive(Clone, Debug, Default)]
pub struct EditForm {
    pub account: String,
    pub email: String,
    pub name: String,
    pub username: String,
    pub jmap: bool,
    pub jmap_url: String,
    /// An Exchange server's EWS address (EWS accounts only).
    pub ews: Option<String>,
    /// The User-Agent its EWS requests name; empty for Quatrelle's.
    pub user_agent: String,
    /// A POP account's POP3 server (host, port, security).
    pub pop: Option<(String, String, Security)>,
    /// POP: when the server's copy of downloaded mail goes.
    pub pop_keep: PopKeep,
    pub imap_host: String,
    pub imap_port: String,
    pub imap_security: Security,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_security: Security,
    /// The outgoing server's own user name and password.
    pub smtp_sign_in: SmtpSignIn,
    /// Keep none of its mail on this computer (issue #4; not for POP).
    pub online_only: bool,
    /// Exchange, as Outlook's Cached Exchange Mode: shared mailboxes kept on this computer, and
    /// the public folders among the Favorites.
    pub shared_kept: bool,
    pub public_favorites_kept: bool,
    /// The provider it signs in with in the browser (`microsoft`: its own registration can be
    /// given below), when it does.
    pub oauth: Option<String>,
    /// Microsoft sign-in through the organisation's own app registration: its Application
    /// (client) ID and tenant (empty: the one built into this copy, at `common`).
    pub client_id: String,
    pub tenant: String,
    pub error: Option<String>,
}

impl EditForm {
    pub(crate) fn params(&self) -> serde_json::Value {
        let mut p = self.server_params();
        if self.pop.is_none() {
            p["onlineOnly"] = json!(self.online_only);
        }
        if self.ews.is_some() {
            p["sharedOnline"] = json!(!self.shared_kept);
            p["publicFavoritesKept"] = json!(self.public_favorites_kept);
        }
        // Its own Microsoft registration, as given (emptied: the built-in one again).
        if self.oauth.as_deref() == Some("microsoft") {
            p["oauthClientId"] = json!(self.client_id.trim());
            p["oauthTenant"] = json!(self.tenant.trim());
        }
        p
    }

    pub(crate) fn server_params(&self) -> serde_json::Value {
        let port = |s: &str| s.trim().parse::<u64>().unwrap_or(0);
        if let Some(u) = &self.ews {
            return json!({
                "account": self.account,
                "name": self.name.trim(),
                "username": self.username.trim(),
                "ews": u.trim(),
                "userAgent": self.user_agent.trim(),
            });
        }
        if self.jmap {
            return json!({"account": self.account, "name": self.name.trim(), "username": self.username.trim(), "jmap": self.jmap_url.trim()});
        }
        if let Some((host, p, security)) = &self.pop {
            let mut v = json!({
                "account": self.account,
                "name": self.name.trim(),
                "username": self.username.trim(),
                "popHost": host.trim(),
                "popPort": port(p),
                "popSecurity": security.code(),
                "smtpHost": self.smtp_host.trim(),
                "smtpPort": port(&self.smtp_port),
                "smtpSecurity": self.smtp_security.code(),
            });
            self.pop_keep.params(&mut v);
            self.smtp_sign_in.params(&mut v);
            return v;
        }
        let mut v = json!({
            "account": self.account,
            "name": self.name.trim(),
            "username": self.username.trim(),
            "jmap": null,
            "imapHost": self.imap_host.trim(),
            "imapPort": port(&self.imap_port),
            "imapSecurity": self.imap_security.code(),
            "smtpHost": self.smtp_host.trim(),
            "smtpPort": port(&self.smtp_port),
            "smtpSecurity": self.smtp_security.code(),
        });
        if !self.jmap {
            self.smtp_sign_in.params(&mut v);
        }
        v
    }
}
