//! An account's worker: how much space each folder takes on the server and the mailbox's limits
//! (Outlook's Folder Size, Server Data), on the second connection so it never waits behind a sync.

use super::*;

impl Worker {
    /// Each folder's size on the server, answered with [`Event::FolderSizes`]; the space used
    /// and the limit go to the File page's bar too ([`Event::Quota`]).
    pub(super) fn folder_sizes(&mut self) {
        let r = (|| -> Result<(Vec<quatrelle_model::FolderSize>, Option<quatrelle_model::MailboxQuota>)> {
            if self.conn.is_none() {
                self.connect()?;
            }
            match self.conn.as_ref() {
                Some(Conn::Ews(c)) => c.folder_sizes(),
                _ => Err(NetError::No("this server doesn't tell each folder's size".into())),
            }
        })();
        let account = self.id();
        match r {
            Ok((folders, quota)) => {
                let used = folders.iter().map(|f| f.size).fold(0u64, u64::saturating_add);
                log::info!("{account}: {} folders, {used} bytes on the server, limits {quota:?}", folders.len());
                self.sink.emit(Event::Quota { account: account.clone(), used, limit: quota.map_or(0, |q| q.limit()) });
                self.sink.emit(Event::FolderSizes { account, server: true, folders, quota, error: None });
            }
            Err(e) => {
                let said = e.to_string();
                if e.is_fatal() || matches!(e, NetError::Auth(_)) {
                    self.conn = None;
                }
                self.sink.emit(Event::FolderSizes { account, server: true, folders: Vec::new(), quota: None, error: Some(said) });
            }
        }
    }
}

/// The space an Exchange mailbox uses (its folders' sizes added up) and its limit (0: none
/// told), as Outlook's File › Info shows them.
pub(super) fn ews_quota(c: &crate::ews::Client) -> Result<Option<(u64, u64)>> {
    let (folders, quota) = c.folder_sizes()?;
    let used = folders.iter().map(|f| f.size).fold(0u64, u64::saturating_add);
    Ok(Some((used, quota.map_or(0, |q| q.limit()))))
}
