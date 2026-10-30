//! The session: a distribution list or a contact group expanded into its members, as Outlook's
//! "+" before a list does. A round trip like the rooms': the question goes out as
//! [`Job::ExpandList`] once, `Loading` while it's out, and [`Event::ListExpanded`] settles it.
//! At most [`MAX_SHOWN_MEMBERS`] members are kept and shown; the rest is counted ("and N more").

use quatrelle_model::addr::{group_ref, is_valid_email, valid_item_id};
use quatrelle_model::{Addressee, AddresseeKind};

use serde_json::json;

use super::*;
use crate::session::scheduling::Fetched;

/// Most members of a list kept and shown.
pub const MAX_SHOWN_MEMBERS: usize = 500;

/// Most members a list is said to have: what a server claims past it is not believed.
pub const MAX_TOTAL: usize = 100_000;

/// Most expansions kept (a Ready or Failed one past it is asked again when wanted).
const KEPT: usize = 200;

/// A question unanswered, or answered by a failure, is asked again after this long (seconds).
const RETRY_AFTER: i64 = 60;

/// Longest address asked about (bytes; an address is at most 254).
const MAX_ADDRESS: usize = 320;

/// Longest error kept (characters).
const MAX_ERROR: usize = 200;

/// A list's members.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expansion {
    /// The members shown, at most [`MAX_SHOWN_MEMBERS`], each with its kind (a list in it is a
    /// list, to expand in turn).
    pub members: Vec<Addressee>,
    /// There are more than `members`: the server gave only part of them, or says it has more
    /// than were kept. `total - members.len()` of them, when that is not zero.
    pub more: bool,
    /// How many members the list has, at least `members.len()` and at most [`MAX_TOTAL`].
    pub total: usize,
}

/// A list's place among the expansions: its address in lower case, or its group reference.
fn expansion_key(address: &str, item_id: Option<&str>) -> Option<String> {
    let address = address.trim().to_ascii_lowercase();
    match item_id {
        Some(id) if valid_item_id(id) => Some(group_ref(id)),
        _ if !address.is_empty() => Some(address),
        _ => None,
    }
}

impl Session {
    /// What's known of a list's members, for an account.
    pub fn expansion_now(&self, account: &str, who: &Addressee) -> Option<&Fetched<Expansion>> {
        let key = who_key(who)?.0;
        self.people.expansions.get(&(account.to_string(), key))
    }

    /// Ask for a list's members (once: an ask out is not repeated, an answer is kept) and say
    /// what's known. A list is a distribution list (by its address) or a contact group (by its
    /// item id); the directory is an Exchange account's.
    pub fn people_expand(&mut self, account: Option<&str>, who: &Addressee) -> CmdResult {
        let account = self.directory_account(account).ok_or_else(|| CmdError::Disabled("the directory is an Exchange account's".into()))?;
        if !who.kind.is_list() {
            return Err(CmdError::Params("only a list or a contact group has members".into()));
        }
        let (key, address, item_id) = who_key(who).ok_or_else(|| CmdError::Params("a list needs an address or an item id".into()))?;
        let k = (account.clone(), key);
        let now = self.clock.now;
        let p = &mut self.people;
        // An answer is kept; an ask out, or one that failed, is made again only after a minute.
        let ask = match p.expansions.get(&k) {
            Some(Fetched::Ready(_)) => false,
            Some(_) => p.expansions_out.get(&k).is_none_or(|&at| now.saturating_sub(at) >= RETRY_AFTER),
            None => true,
        };
        if ask {
            if !p.expansions.contains_key(&k) && p.expansions.len() >= KEPT {
                p.expansions.retain(|_, f| matches!(f, Fetched::Loading));
                p.expansions_out.retain(|k, _| p.expansions.contains_key(k));
            }
            p.expansions.insert(k.clone(), Fetched::Loading);
            p.expansions_out.insert(k.clone(), now);
            self.jobs.push(Job::ExpandList { account: account.clone(), address, item_id });
        }
        Ok(expansion_json(&account, self.people.expansions.get(&k)))
    }

    /// The server's answer about a list. An answer for a list nobody asked about (or an account
    /// gone) is ignored. Members are cut to [`MAX_SHOWN_MEMBERS`] and go in the directory index;
    /// the count says what was left out, whatever `more` says: members without an address are
    /// dropped by the net side, so fewer may come than the list has.
    pub(crate) fn list_expanded(
        &mut self,
        account: &str,
        address: String,
        item_id: Option<String>,
        mut members: Vec<Addressee>,
        more: bool,
        total: usize,
        error: Option<String>,
    ) {
        let Some(key) = expansion_key(&address, item_id.as_deref()) else { return };
        let k = (account.to_string(), key);
        if self.account(account).is_none() || !self.people.expansions.contains_key(&k) {
            return;
        }
        if let Some(e) = error {
            log::info!("{account}: a list's members couldn't be had");
            let e = e.chars().take(MAX_ERROR).collect();
            // When it failed, for the retry.
            self.people.expansions_out.insert(k.clone(), self.clock.now);
            self.people.expansions.insert(k, Fetched::Failed(e));
            return;
        }
        self.people.expansions_out.remove(&k);
        members.truncate(MAX_SHOWN_MEMBERS);
        self.index_directory(&members);
        let total = total.min(MAX_TOTAL).max(members.len());
        let more = more || total > members.len();
        self.people.expansions.insert(k, Fetched::Ready(Expansion { members, more, total }));
        // The members may be suggested now.
        self.contacts.changed();
    }

    /// An account is gone: so are its expansions.
    pub(crate) fn forget_expansions(&mut self, account: &str) {
        self.people.expansions.retain(|(a, _), _| a != account);
        self.people.expansions_out.retain(|(a, _), _| a != account);
    }
}

/// A list's key among the expansions and what the job asks: a contact group by its item id (it
/// has no address), anything else by its address, else by its item id.
fn who_key(who: &Addressee) -> Option<(String, String, Option<String>)> {
    let id = who.item_id.trim();
    let id = valid_item_id(id).then(|| id.to_string());
    let address = who.address.trim();
    let by_address = address.len() <= MAX_ADDRESS && is_valid_email(address);
    match (who.kind == AddresseeKind::ContactGroup || !by_address, id) {
        (true, Some(id)) => Some((group_ref(&id), String::new(), Some(id))),
        _ if by_address => Some((address.to_ascii_lowercase(), address.to_string(), None)),
        _ => None,
    }
}

/// An expansion as the command answers it.
fn expansion_json(account: &str, f: Option<&Fetched<Expansion>>) -> Value {
    let mut v = json!({"account": account, "state": f.map_or("none", Fetched::state), "members": [], "more": false, "total": 0});
    match f {
        Some(Fetched::Failed(e)) => v["error"] = json!(e),
        Some(Fetched::Ready(x)) => {
            v["members"] = Value::Array(x.members.iter().map(member_json).collect());
            v["more"] = json!(x.more);
            v["total"] = json!(x.total);
        }
        _ => {}
    }
    v
}

/// A member as the commands answer it.
fn member_json(m: &Addressee) -> Value {
    let mut o = json!({"name": m.name, "email": m.address, "kind": m.kind.code()});
    if !m.item_id.is_empty() {
        o["itemId"] = json!(m.item_id);
    }
    o
}
