//! What a folder keeps as Outlook's Cached Exchange Mode does: the mail the server received in
//! the last [`SyncOptions::since_days`], and what a search of the server brought in.

use std::collections::HashSet;

use quatrelle_jmap::FOUND_MAX;
use serde::{Deserialize, Serialize};

use super::*;

/// Kept with a folder's cache, beside its list (and gone with it): what the list alone can't
/// tell.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Kept {
    /// The age the folder was listed with ([`SyncOptions::since_days`], 0: all of it): a longer
    /// one lists it again, once.
    pub days: u32,
    /// Messages a search of the server brought in (issue #4), newest finds first, [`FOUND_MAX`]
    /// at most: kept however old, while the server has them there.
    pub found: Vec<u32>,
}

const KEPT_DOC: &str = "kept.json";

impl Kept {
    /// Empty when there's none, or it's damaged; a disk that can't be read is an error (the
    /// finds aren't let go for it).
    pub fn load(store: &Store, account: &str, folder: &str) -> Result<Kept> {
        Ok(store.load_folder_doc(account, folder, KEPT_DOC)?)
    }

    pub fn save(&self, store: &Store, account: &str, folder: &str) -> Result<()> {
        Ok(store.save_folder_doc(account, folder, KEPT_DOC, self)?)
    }

    /// What a search of the server just brought in, added to what earlier ones did.
    pub fn add_found(&mut self, found: &[u32]) {
        let mut next: Vec<u32> = found.to_vec();
        next.extend(self.found.iter().filter(|n| !found.contains(n)));
        next.truncate(FOUND_MAX);
        self.found = next;
    }

    /// Whether `days` keeps more than the age the folder was listed with: the older mail it now
    /// takes in is listed.
    pub fn widened(&self, days: u32) -> bool {
        self.days != 0 && (days == 0 || days > self.days)
    }
}

/// What a search of the server fetched into a folder's list (those it found that weren't
/// there, in rising order): marked, so the age kept doesn't let them go. (One the server no
/// longer had is forgotten at the next sync.)
pub(super) fn mark_found(store: &Store, account: &str, folder: &str, came: &[u32]) -> Result<()> {
    if came.is_empty() {
        return Ok(());
    }
    let mut kept = Kept::load(store, account, folder)?;
    kept.add_found(&came.iter().rev().copied().collect::<Vec<u32>>());
    kept.save(store, account, folder)
}

/// Now, in Unix seconds.
pub(super) fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// The day of a time (UTC) as IMAP writes dates (`23-Oct-2024`, RFC 3501 §9 `date-text`).
pub(crate) fn imap_date(t: i64) -> String {
    let (y, m, d) = quatrelle_model::dates::civil_from_days(t.div_euclid(86_400));
    let month = MONTHS.get(m.saturating_sub(1) as usize).copied().unwrap_or("Jan");
    format!("{d}-{month}-{y}")
}

/// The `SEARCH` keys listing what a folder keeps: what the server received since the cutoff
/// (`SINCE`, by its INTERNALDATE, the day of the cutoff and after), or all of it.
pub(super) fn listing(cutoff: Option<i64>) -> String {
    cutoff.map_or_else(|| "ALL".to_string(), |t| format!("SINCE {}", imap_date(t)))
}

/// Let go of what the server received before the cutoff, with its source: older mail is read on
/// the server. What a search of the server found stays. How many went.
pub(super) fn age_out(store: &Store, st: &mut FolderState, kept: &Kept, cutoff: Option<i64>) -> usize {
    let Some(cutoff) = cutoff else { return 0 };
    if st.messages.iter().all(|m| m.received_at() >= cutoff) {
        return 0;
    }
    let found: HashSet<u32> = kept.found.iter().copied().collect();
    let (keep, gone): (Vec<Summary>, Vec<Summary>) =
        std::mem::take(&mut st.messages).into_iter().partition(|m| m.received_at() >= cutoff || found.contains(&m.key.uid));
    for m in &gone {
        store.remove_body(&m.key);
    }
    st.messages = keep;
    gone.len()
}

/// Of a folder's finds, those still in its list (the rest went from the server).
pub(super) fn keep_found(kept: &mut Kept, st: &FolderState) {
    if kept.found.is_empty() {
        return;
    }
    let cached: HashSet<u32> = st.messages.iter().map(|m| m.key.uid).collect();
    kept.found.retain(|u| cached.contains(u));
}
