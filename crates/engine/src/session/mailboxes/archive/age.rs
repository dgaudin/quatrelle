// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The session: archiving by age. An IMAP account that asks for it is looked through once a day,
//! once it is connected, a few folders a frame (the window never waits); mail older than the
//! account's number of months is offered in a bar, and the user's *Archive* sends it through the
//! same archiving as the button, in batches. *Not now* is kept in the account's `archive.json`, so
//! it holds until the next day even after a restart.

use super::*;

/// The most one account archives by age in a day.
pub(crate) const PER_DAY: usize = 2000;
/// A month for the cutoff: a twelfth of the average year.
const MONTH_SECONDS: i64 = 2_629_800;
const DOC: &str = "archive.json";

/// What is kept with the account (`archive.json`).
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Doc {
    /// The day *Not now* was said (days since 1970, local).
    #[serde(default)]
    not_now: i64,
    /// The day `used` counts.
    #[serde(default)]
    day: i64,
    /// The messages sent off by age on that day.
    #[serde(default)]
    used: u32,
}

/// Looking through one account's folders for mail to offer.
struct Scan {
    cutoff: i64,
    months: u32,
    /// Folders still to look at.
    todo: Vec<String>,
    found: Vec<Arc<Summary>>,
    /// How many more may be offered today.
    room: usize,
}

/// Mail waiting for the user's answer in the bar.
struct Offer {
    months: u32,
    found: Vec<Arc<Summary>>,
}

/// An archiving by age under way: how many of its messages the network layer has answered for.
struct Run {
    done: usize,
    total: usize,
}

/// The session's memory of archiving by age (nothing here outlives the program but `archive.json`).
#[derive(Default)]
pub(crate) struct AgeState {
    scans: BTreeMap<String, Scan>,
    offers: BTreeMap<String, Offer>,
    /// The day each account was last looked through.
    checked: BTreeMap<String, i64>,
    runs: BTreeMap<String, Run>,
}

/// Folders looked at in one go (one frame): each is read from the disk.
const FOLDERS_PER_STEP: usize = 2;

/// Where mail is never archived by age: the folders the user keeps work or rubbish in.
fn left_alone(f: &Folder) -> bool {
    let outbox = f.path.rsplit(f.delimiter.unwrap_or('/')).next().is_some_and(|n| n.eq_ignore_ascii_case("outbox"));
    !f.selectable || f.mailbox.is_some() || outbox || matches!(f.role, FolderRole::Drafts | FolderRole::Trash | FolderRole::Junk)
}

impl Session {
    /// The local day (days since 1970).
    fn age_today(&self) -> i64 {
        (self.clock.now + i64::from(self.clock.offset)).div_euclid(86_400)
    }

    fn age_doc(&self, account: &str) -> Doc {
        self.store.as_ref().and_then(|st| st.load_account_doc::<Doc>(account, DOC).ok()).unwrap_or_default()
    }

    fn save_age_doc(&self, account: &str, doc: &Doc) {
        if let Some(st) = &self.store
            && let Err(e) = st.save_account_doc(account, DOC, doc)
        {
            log::warn!("archive.json couldn't be written: {e}");
        }
    }

    /// How many more messages may go by age today.
    fn age_room(&self, doc: &Doc) -> usize {
        let used = if doc.day == self.age_today() { usize::try_from(doc.used).unwrap_or(PER_DAY) } else { 0 };
        PER_DAY.saturating_sub(used)
    }

    fn age_connected(&self, account: &str) -> bool {
        self.account(account).is_some_and(|a| matches!(a.status, SyncStatus::Online { .. } | SyncStatus::Syncing { .. }))
    }

    /// An account is connected (or has asked again): look through it if it is time, that is once
    /// a day, when the account asks for it, nothing is waiting in the bar, and *Not now* wasn't
    /// said today.
    pub(crate) fn consider_age(&mut self, account: &str) {
        let Some(months) = self.account(account).and_then(|a| a.config.archive_after_months) else { return };
        let today = self.age_today();
        if !self.is_imap(account)
            || !self.age_connected(account)
            || self.age.checked.get(account) == Some(&today)
            || self.age.scans.contains_key(account)
            || self.age.offers.contains_key(account)
        {
            return;
        }
        let doc = self.age_doc(account);
        let room = self.age_room(&doc);
        if doc.not_now == today || room == 0 {
            return;
        }
        self.age.checked.insert(account.to_string(), today);
        // A run the network layer never finished answering (a day old) is forgotten.
        self.age.runs.remove(account);
        let todo: Vec<String> =
            self.account(account).map(|a| a.folders.iter().filter(|f| !left_alone(f)).map(|f| f.path.clone()).collect()).unwrap_or_default();
        let cutoff = self.clock.now.saturating_sub(i64::from(months).saturating_mul(MONTH_SECONDS));
        self.age.scans.insert(account.to_string(), Scan { cutoff, months, todo, found: Vec::new(), room });
    }

    /// The account's number of months, or the account itself, changed: look again.
    pub(crate) fn age_settings_changed(&mut self, account: &str) {
        self.age.scans.remove(account);
        self.age.offers.remove(account);
        self.age.checked.remove(account);
        self.consider_age(account);
    }

    /// Look at a few more folders of each account being looked through; the one that is done
    /// puts its bar up when it found something.
    pub(crate) fn step_age_scans(&mut self) {
        let accounts: Vec<String> = self.age.scans.keys().cloned().collect();
        for account in accounts {
            let Some(mut scan) = self.age.scans.remove(&account) else { continue };
            if self.account(&account).is_none() {
                continue;
            }
            let leaving: HashSet<MessageKey> = self.leaving.keys().cloned().collect();
            for _ in 0..FOLDERS_PER_STEP {
                let Some(path) = scan.todo.pop() else { break };
                let room = scan.room.saturating_sub(scan.found.len());
                let here = self.scan_state(&account, &path).messages.iter().filter(|m| {
                    let received = m.received_at();
                    received > 0
                        && received < scan.cutoff
                        && !m.flags.pinned()
                        && !m.flags.flagged
                        && !m.flags.draft
                        && !m.flags.deleted
                        && !leaving.contains(&m.key)
                });
                let found: Vec<Arc<Summary>> = here.take(room).cloned().collect();
                scan.found.extend(found);
                if scan.found.len() >= scan.room {
                    scan.todo.clear();
                }
            }
            if !scan.todo.is_empty() {
                self.age.scans.insert(account, scan);
            } else if !scan.found.is_empty() {
                self.age.offers.insert(account, Offer { months: scan.months, found: scan.found });
            }
        }
    }

    /// The bar's offer for `account`: how many messages and how many months old. Only while the
    /// account is connected.
    pub fn archive_offer(&self, account: &str) -> Option<(usize, u32)> {
        if !self.age_connected(account) {
            return None;
        }
        self.age.offers.get(account).map(|o| (o.found.len(), o.months))
    }

    /// An archiving by age under way: messages answered for, and in all.
    pub fn archive_progress(&self) -> Option<(usize, usize)> {
        self.age.runs.values().next().map(|r| (r.done, r.total))
    }

    /// Forget the days the accounts were looked through, as a restart does not (tests: the cap).
    #[cfg(test)]
    pub(crate) fn age_forget_for_tests(&mut self) {
        self.age.checked.clear();
    }

    /// Whether an account is being looked through (the window draws again to carry on).
    pub fn archive_scanning(&self) -> bool {
        !self.age.scans.is_empty()
    }

    /// `archive.byAge`: archive what the bar offered, in batches, through the same archiving as
    /// the button. The day's room is spent now.
    pub fn archive_by_age(&mut self, account: &str) -> CmdResult {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        if !self.age_connected(account) {
            return Err(CmdError::Disabled("not connected".into()));
        }
        let Some(offer) = self.age.offers.remove(account) else {
            return Err(CmdError::Disabled("nothing to archive by age".into()));
        };
        let total = offer.found.len();
        let mut doc = self.age_doc(account);
        let today = self.age_today();
        let used = if doc.day == today { doc.used } else { 0 };
        doc.day = today;
        doc.used = used.saturating_add(u32::try_from(total).unwrap_or(u32::MAX));
        let selected = offer.found.iter().any(|m| self.selection.contains(&m.key));
        let next = self.successor(&self.selection.clone());
        if let Err(e) = self.queue_archive(&offer.found) {
            self.age.offers.insert(account.to_string(), offer);
            return Err(e);
        }
        self.save_age_doc(account, &doc);
        self.age.runs.insert(account.to_string(), Run { done: 0, total });
        self.refresh_list();
        if selected {
            self.after_removal(next);
        }
        Ok(serde_json::json!({ "archiving": total }))
    }

    /// `archive.notNow`: the bar is put away until tomorrow (kept, so a restart doesn't ask).
    pub fn archive_not_now(&mut self, account: &str) -> CmdResult {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        self.age.offers.remove(account);
        let mut doc = self.age_doc(account);
        doc.not_now = self.age_today();
        self.save_age_doc(account, &doc);
        Ok(serde_json::json!({ "account": account }))
    }

    /// The network layer answered for some of an archiving by age.
    pub(crate) fn age_progress(&mut self, account: &str, answered: usize) {
        let finished = self.age.runs.get_mut(account).is_some_and(|r| {
            r.done = r.done.saturating_add(answered).min(r.total);
            r.done >= r.total
        });
        if finished {
            self.age.runs.remove(account);
        }
    }
}
