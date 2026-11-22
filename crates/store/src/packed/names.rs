// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Which data file is each account's (`datafiles.json` in the root): read before anything is
//! made, written whole after each change. A list that can't be read stops the opening (taken for
//! none, every account's file was forgotten and a second one made beside it at its next write); a
//! damaged one is kept aside, and each account's file found again by its name where that can't be
//! mistaken ([`found_again`]).

use super::*;

/// The most data files looked at to find accounts' files again ([`found_again`]): past it, none
/// is guessed. The folder's other entries aren't counted.
pub(super) const FILES_MAX: usize = 4096;
/// How many damaged lists are kept aside (`.bad`, `.bad.2`… `.bad.9`).
const ASIDE_MAX: u32 = 9;

/// An account as the list of accounts has it.
struct Listed {
    /// Its folder's name ([`crate::safe_segment`]).
    account: String,
    email: String,
    /// Kept online only: it has no data file.
    online: bool,
}

/// The opening stopped on `path`: the list itself, or what making it again needs.
fn unreadable(path: &Path, source: io::Error) -> StoreError {
    StoreError::NamesUnreadable { path: path.display().to_string(), source }
}

/// The accounts listed: none when there's no list of them, an error when it can't be read or is
/// damaged. Read as it is: a damaged one stays where it is.
fn listed(root: &Path) -> Result<Vec<Listed>> {
    let path = root.join("accounts.json");
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(unreadable(&path, e)),
    };
    // Not the parser's words: they can quote what the list holds.
    let list: Vec<quatrelle_model::AccountConfig> =
        serde_json::from_slice(&bytes).map_err(|_| unreadable(&path, io::Error::new(io::ErrorKind::InvalidData, "damaged")))?;
    Ok(list.into_iter().map(|a| Listed { account: crate::safe_segment(&a.id), online: a.keeps_online(), email: a.email }).collect())
}

/// What an account's data file is called, before its extension and any number: its address, as
/// Outlook names a `.pst` (an opened data file's own name), else the account's folder name.
pub(super) fn stem(root: &Path, account: &str) -> String {
    let email = listed(root).unwrap_or_default().into_iter().find(|a| a.account == account).map(|a| a.email);
    stem_of(email.as_deref(), account)
}

fn stem_of(email: Option<&str>, account: &str) -> String {
    let stem = email.map(str::trim).filter(|e| !e.is_empty()).unwrap_or(account);
    let stem = stem
        .strip_suffix(".pst")
        .or_else(|| stem.strip_suffix(".ost"))
        .or_else(|| stem.strip_suffix(".quatrelle"))
        .or_else(|| stem.strip_suffix(&format!(".{}", crate::legacy::OLD_EXTENSION)))
        .unwrap_or(stem);
    let stem: String = stem
        .chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '_' } else { c })
        .take(80)
        .collect();
    let stem = stem.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if stem.is_empty() { account.to_string() } else { stem.to_string() }
}

/// Whether a name in the data files' folder is a data file's (its extension in any case).
fn is_data_file(name: &str) -> bool {
    Path::new(name).extension().is_some_and(|x| x.eq_ignore_ascii_case(EXTENSION))
}

/// Whether a data file's name is one [`Packs::name`] gives for `stem`, `<stem>.quatrelle` or
/// `<stem> (N).quatrelle`, in any case (a disk may not tell them apart).
fn of_stem(name: &str, stem: &str) -> bool {
    let Some(base) = Path::new(name).file_stem().and_then(|s| s.to_str()).filter(|_| is_data_file(name)) else { return false };
    let (base, stem) = (base.to_lowercase(), stem.to_lowercase());
    let numbered = base.strip_prefix(stem.as_str()).and_then(|r| r.strip_prefix(" (")).and_then(|r| r.strip_suffix(')'));
    base == stem || numbered.is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The list as kept: none there is an empty one, one that can't be read an error. A damaged one
/// is made again from the data files there ([`found_again`]) and kept aside ([`aside`]), once all
/// that takes has been read: when any of it can't be, nothing is changed and it's an error.
pub(super) fn read(root: &Path, dir: &Path) -> Result<BTreeMap<String, String>> {
    let path = root.join(NAMES);
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(unreadable(&path, e)),
    };
    if let Ok(names) = serde_json::from_slice(&bytes) {
        return Ok(names);
    }
    let accounts = listed(root)?;
    let names = found_again(&accounts, dir)?;
    // Kept aside as a copy written whole before the list is: one that can't be kept aside changes
    // nothing, and a list that can't be written after stays as it was, found again next time.
    let aside = aside(&path);
    write_atomic(&aside, &bytes).map_err(|e| unreadable(&path, source_of(e)))?;
    log::warn!("{}: damaged, kept as {}; {} of {} accounts' data files found again", path.display(), aside.display(), names.len(), accounts.len());
    save(root, &names);
    Ok(names)
}

/// Where a damaged list is kept: the first of `.bad`, `.bad.2`… `.bad.9` not there; all of them
/// there, the one kept longest.
fn aside(path: &Path) -> PathBuf {
    let slots: Vec<PathBuf> =
        (1..=ASIDE_MAX).map(|i| path.with_extension(if i == 1 { "json.bad".to_string() } else { format!("json.bad.{i}") })).collect();
    let free = slots.iter().find(|p| fs::symlink_metadata(p).is_err_and(|e| e.kind() == io::ErrorKind::NotFound));
    let oldest = || slots.iter().min_by_key(|p| fs::metadata(p).and_then(|m| m.modified()).ok());
    free.or_else(oldest).cloned().unwrap_or_else(|| path.with_extension("json.bad"))
}

/// Each account's data file in `dir`, found again without the list, by its name alone (a data
/// file doesn't say whose it is): an account gets the one data file there of its name
/// ([`of_stem`]), under the name the disk has, when that name is no other account's. Two files of
/// its name (an account removed, its file kept, and the account added again beside it), or two
/// accounts of one address: only the list said which was which, so none is guessed (an account
/// read another's mail). One left without starts a new file; the old ones stay where they are.
/// One kept online only has none. A folder that can't be listed is an error.
fn found_again(accounts: &[Listed], dir: &Path) -> Result<BTreeMap<String, String>> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(unreadable(dir, e)),
    };
    // The data files there, by their names as the disk has them, and whether each is a file.
    let mut files: Vec<(String, bool)> = Vec::new();
    for e in rd {
        let e = e.map_err(|e| unreadable(dir, e))?;
        let Some(name) = e.file_name().to_str().map(str::to_string).filter(|n| is_data_file(n)) else { continue };
        if files.len() >= FILES_MAX {
            // One of an account's name could be among those not looked at.
            log::warn!("{}: more than {FILES_MAX} data files; none found again by its name", dir.display());
            return Ok(BTreeMap::new());
        }
        files.push((name, e.path().is_file()));
    }
    let stems: Vec<(&str, String)> = accounts.iter().map(|a| (a.account.as_str(), stem_of(Some(&a.email), &a.account))).collect();
    let mut names = BTreeMap::new();
    let mut unsure = 0usize;
    for (a, (_, stem)) in accounts.iter().zip(&stems) {
        if a.online {
            continue;
        }
        let theirs: Vec<&(String, bool)> = files.iter().filter(|(n, _)| of_stem(n, stem)).collect();
        match theirs.as_slice() {
            [] => {}
            [(file, true)] if !stems.iter().any(|(b, s)| *b != a.account && of_stem(file, s)) => {
                names.insert(a.account.clone(), file.clone());
            }
            _ => unsure = unsure.saturating_add(1),
        }
    }
    if unsure > 0 {
        log::warn!("{unsure} accounts' data files not told apart by their names: not guessed");
    }
    Ok(names)
}

/// Write the list whole; a failure is said in the log (the next change writes it whole again).
pub(super) fn save(root: &Path, names: &BTreeMap<String, String>) {
    let path = root.join(NAMES);
    let written = serde_json::to_vec_pretty(names)
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
        .and_then(|json| write_atomic(&path, &json).map_err(source_of));
    if let Err(e) = written {
        log::warn!("{}: not saved: {e}", path.display());
    }
}

/// What failed, without the path a store error names.
fn source_of(e: StoreError) -> io::Error {
    match e {
        StoreError::Io { source, .. } | StoreError::NamesUnreadable { source, .. } => source,
        StoreError::Json { .. } => io::Error::from(io::ErrorKind::InvalidData),
    }
}

impl Packs {
    /// An account listed with no data file yet gets one now when a data file of its name is
    /// there already (an account removed, its file kept, and the account added again): numbered
    /// beside it from the start. Made at its first write, until then the old file was the only
    /// one of that name, and a list damaged meanwhile gave it to the account ([`found_again`]).
    pub(crate) fn number_beside_its_name(&self, account: &str, email: &str) -> io::Result<()> {
        if Packs::lock(&self.names).contains_key(account) {
            return Ok(());
        }
        let stem = stem_of(Some(email), account);
        let Ok(rd) = fs::read_dir(&self.dir) else { return Ok(()) };
        if rd.flatten().any(|e| e.file_name().to_str().is_some_and(|n| of_stem(n, &stem))) {
            self.open_db(account, true, true)?;
        }
        Ok(())
    }
}
