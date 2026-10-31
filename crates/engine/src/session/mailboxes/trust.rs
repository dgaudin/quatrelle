//! The servers an account trusts: its password or sign-in goes only to its own servers, the
//! sites of their domains, and the other domains the user allowed. A server that proposes
//! another one (Autodiscover, a calendar's discovery, JMAP's session) is asked about first; these
//! record the answer. The commands are `account.trustDomain`, `account.distrustDomain` and
//! `account.refuseDomain`.

use super::*;
use quatrelle_model::addr::trust::{site as site_of, url_host};

/// Other domains one account trusts at most.
const MAX_TRUSTED: usize = 20;

/// The site of a domain the user typed (or a URL they pasted: its host is taken), when it is a
/// host name: letters, digits, `-` and `.`, at most 253 characters, labels of 1 to 63 that don't
/// start or end with `-`, and a dot (or `localhost`).
fn domain_site(domain: &str) -> Result<String, CmdError> {
    let bad = || CmdError::Params(format!("not a domain: {}", domain.chars().take(80).collect::<String>()));
    let host = url_host(domain).ok_or_else(bad)?;
    let host = host.trim_end_matches('.');
    let label_ok = |l: &str| !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-');
    let ok = host.len() <= 253
        && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && host.split('.').all(label_ok)
        && (host.contains('.') || host.eq_ignore_ascii_case("localhost"));
    if ok { Ok(site_of(host)) } else { Err(bad()) }
}

impl Session {
    /// The account's id and the site of `domain`, or why they can't be.
    fn trust_target(&self, account: &str, domain: &str) -> Result<(String, String), CmdError> {
        let a = self.account(account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        Ok((a.config.id.clone(), domain_site(domain)?))
    }

    /// Start the account again, so what waited for the answer goes on.
    fn restart_account(&mut self, account: &str) {
        let Some(a) = self.account(account) else { return };
        if a.is_local() || self.prefs.work_offline {
            return;
        }
        let config = a.config.clone();
        self.jobs.push(Job::Start { account: config, password: None });
    }

    /// Allow another domain's servers to get the account's password or sign-in (the user said
    /// yes to a server's proposal, or added it in the account's settings). Kept as its site; the
    /// account starts again, and the question can be asked again after it is withdrawn.
    pub fn trust_domain(&mut self, account: &str, domain: &str) -> CmdResult {
        let (id, site) = self.trust_target(account, domain)?;
        let Some(a) = self.account_mut(&id) else { return Err(CmdError::Params(format!("no account `{id}`"))) };
        if !a.config.trusted_domains.contains(&site) {
            if a.config.trusted_domains.len() >= MAX_TRUSTED {
                return Err(CmdError::Params(format!("at most {MAX_TRUSTED} other servers")));
            }
            a.config.trusted_domains.push(site.clone());
        }
        let pair = (id.clone(), site.clone());
        self.asked_trust.remove(&pair);
        self.refused_trust.remove(&pair);
        self.ask_trust.retain(|p| *p != pair);
        self.save_accounts();
        self.restart_account(&id);
        Ok(serde_json::json!({ "account": id, "trusted": site }))
    }

    /// Withdraw a domain the account trusted; it starts again without it. If a server proposes
    /// the domain again in the same run, it is asked about again.
    pub fn distrust_domain(&mut self, account: &str, domain: &str) -> CmdResult {
        let (id, site) = self.trust_target(account, domain)?;
        let Some(a) = self.account_mut(&id) else { return Err(CmdError::Params(format!("no account `{id}`"))) };
        let before = a.config.trusted_domains.len();
        a.config.trusted_domains.retain(|d| url_host(d).map(|h| site_of(&h)).as_deref() != Some(site.as_str()));
        let removed = a.config.trusted_domains.len() != before;
        let pair = (id.clone(), site.clone());
        self.asked_trust.remove(&pair);
        self.ask_trust.retain(|p| *p != pair);
        if removed {
            self.save_accounts();
            self.restart_account(&id);
        }
        Ok(serde_json::json!({ "account": id, "removed": site, "wasTrusted": removed }))
    }

    /// The user didn't allow a domain: the account leaves it as it is, and it isn't asked about
    /// again until Quatrelle starts again.
    pub fn refuse_domain(&mut self, account: &str, domain: &str) -> CmdResult {
        let (id, site) = self.trust_target(account, domain)?;
        let email = self.account(&id).map(|a| a.config.email.clone()).unwrap_or_default();
        let pair = (id.clone(), site.clone());
        self.ask_trust.retain(|p| *p != pair);
        self.refused_trust.insert(pair);
        self.notice_with("{domain} wasn't allowed: {account} won't use it", &[("domain", &site), ("account", &email)], false);
        Ok(serde_json::json!({ "account": id, "refused": site }))
    }

    /// Ask whether the account may use another site, unless it was asked (and not withdrawn since)
    /// or refused in this run. The interface drains `ask_trust` as it does `ask_password`.
    // The network's events call it from the next tasks; the compiler is told until then (tests use it).
    #[cfg_attr(not(test), expect(dead_code, reason = "called by the NeedsTrust and TrustAsked events (credentials-trust tasks 5 and 6)"))]
    pub(crate) fn ask_trust_once(&mut self, account: &str, site: &str) {
        /// Questions asked in one run at most: a hostile server naming endless domains must not
        /// queue endless dialogs. The domains past it stay untrusted, so nothing goes to them.
        const MAX_ASKED: usize = 200;
        let pair = (account.to_string(), site.to_string());
        if site.is_empty()
            || site.chars().any(|c| c.is_control() || c.is_whitespace())
            || self.refused_trust.contains(&pair)
            || self.asked_trust.contains(&pair)
        {
            return;
        }
        if self.asked_trust.len() >= MAX_ASKED {
            log::info!("trust: too many questions this run, {site} stays untrusted");
            return;
        }
        self.asked_trust.insert(pair.clone());
        self.ask_trust.push(pair);
    }
}
