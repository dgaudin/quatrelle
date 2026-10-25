//! The session: what the network side reports, applied.

use super::*;

impl Session {
    /// Apply an event from the network layer.
    pub fn apply(&mut self, e: Event) {
        match e {
            e @ (Event::Identity { .. }
            | Event::Protected { .. }
            | Event::Decrypted { .. }
            | Event::Revocation { .. }
            | Event::HeldKeys { .. }
            | Event::DirectoryCertificates { .. }) => self.apply_smime(e),
            Event::Status { account, status } => {
                // Show more ended (with nothing new, or failing): its footer can be asked again.
                if !matches!(status, SyncStatus::Syncing { .. } | SyncStatus::Connecting)
                    && self.showing_more.as_ref().is_some_and(|(a, _)| *a == account)
                {
                    self.showing_more = None;
                }
                // Refusals that need something done say so once, as they come.
                if let SyncStatus::Error { error } = &status
                    && (error.starts_with(quatrelle_model::MICROSOFT_IMAP_CLOSED) || error.starts_with(quatrelle_model::MICROSOFT_NO_PASSWORDS))
                    && self.account(&account).is_some_and(|a| a.status != status)
                {
                    let who = self.account(&account).map(|a| a.config.email.clone()).unwrap_or_default();
                    self.notice(format!("{who}: {error}"), true);
                }
                if let SyncStatus::NeedsPassword { error } = &status {
                    let (who, browser) = self.account(&account).map(|a| (a.config.email.clone(), a.config.oauth.is_some())).unwrap_or_default();
                    let why = if error.is_empty() { String::new() } else { format!(": {error}") };
                    // An account that signs in in the browser has no password to ask for.
                    let what = if browser { "needs you to sign in again" } else { "needs its password" };
                    self.notice(format!("{who} {what}{why}"), true);
                    if !browser && self.asked_password.insert(account.clone()) {
                        self.ask_password.push(account.clone());
                    }
                }
                // Coming online, not every "Online" after it: the sync says it after each folder
                // and each check for new mail (Syncing, then Online, every half minute), and the
                // rules were read (and a refused one sent again) that often — over a hundred
                // times a pass on a big mailbox. Losing the server starts it over.
                if matches!(status, SyncStatus::Offline | SyncStatus::Connecting | SyncStatus::Error { .. } | SyncStatus::NeedsPassword { .. }) {
                    self.rules_asked.remove(&account);
                    self.rules_online.remove(&account);
                }
                let online = matches!(status, SyncStatus::Online { .. }) && self.rules_online.insert(account.clone());
                // A server that has started syncing is readable: its rules are asked for then,
                // not once the whole mailbox (public folders and all) has been walked — that
                // takes minutes, and the Rules dialog would sit empty until then.
                let connecting = matches!(status, SyncStatus::Syncing { .. }) && self.rules_asked.insert(account.clone());
                if let Some(a) = self.account_mut(&account) {
                    a.status = status;
                }
                // Rules made while the account was offline reach its server now it's connected,
                // and the ones it keeps that Quatrelle didn't make are listed.
                if online || connecting {
                    self.resend_server_rules(&account);
                    self.refresh_server_rules(&account);
                }
            }
            Event::Folders { account, mut folders } => {
                // They're this account's, whatever name they were kept under.
                for f in &mut folders {
                    if f.account != account {
                        f.account.clone_from(&account);
                    }
                }
                sort_folders(&mut folders);
                let first = self.account(&account).is_some_and(|a| a.folders.is_empty());
                if let Some(a) = self.account_mut(&account) {
                    a.folders = folders;
                }

                // A view on a folder that no longer exists falls back to the Inbox.
                let missing = matches!(&self.view, View::Folder { account: va, folder } if *va == account && self.folder(va, folder).is_none());
                if missing {
                    let inbox =
                        self.account(&account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone()).unwrap_or_else(|| "INBOX".into());
                    self.open_view(View::Folder { account: account.clone(), folder: inbox });
                } else if first {
                    self.refresh_list();
                }
                self.mark_junk(&account);
                self.index = None;
                self.follow_public_favorites();
            }
            Event::FolderChanged { account, folder } => {
                if self.showing_more.as_ref().is_some_and(|(a, f)| *a == account && *f == folder) {
                    self.showing_more = None;
                }
                self.reload_folder(&account, &folder);
                self.note_new_mail(&account, &folder);
                let visible = self.search.is_some() || self.view_folders().iter().any(|(a, f)| *a == account && *f == folder);
                if visible {
                    self.refresh_list();
                }
                if let Some(r) = &self.reading
                    && r.key.account == account
                    && r.key.folder == folder
                {
                    let key = r.key.clone();
                    let thread = self.thread_of(&key);
                    if let Some(r) = self.reading.as_mut() {
                        r.thread = thread;
                    }
                }
            }
            Event::ServerSearched { account, query, keys, error, done } => {
                if let Some(ss) = self.server_search.as_mut().filter(|ss| ss.query == query) {
                    // What's found so far shows at once; the account is done at its last answer.
                    if done {
                        ss.pending = ss.pending.saturating_sub(1);
                    }
                    ss.found.extend(keys);
                    if let Some(e) = error {
                        let who = self.accounts.iter().find(|a| a.config.id == account).map(|a| a.config.email.clone()).unwrap_or(account);
                        if let Some(ss) = self.server_search.as_mut() {
                            ss.errors.push(format!("{who}: {e}"));
                        }
                    }
                    self.refresh_list();
                }
            }
            Event::DavSynced { account, sync } => self.apply_dav(&account, &sync),
            Event::Rooms { account, lists, rooms, error } => self.rooms_answered(&account, lists, rooms, error),
            Event::Availability { account, from, to, people, error } => self.availability_answered(&account, from, to, people, error),
            Event::Imported { account, messages, skipped, done, error } => self.imported(&account, messages, skipped, done, error),
            Event::Compacted { account, before, after, error } => self.compacted(&account, before, after, error),
            Event::FolderSizes { account, server, folders, quota, error } => self.sizes_answered(&account, server, folders, quota, error),
            Event::LlmAnswered { id, text, error } => self.llm_answered(id, text, error),
            Event::Exported { account, messages, skipped, done, error } => self.exported(&account, messages, skipped, done, error),
            Event::Categories { account, categories } => self.take_master_categories(&account, categories),
            Event::Quota { account, used, limit } => {
                if let Some(a) = self.account_mut(&account) {
                    a.quota = Some((used, limit));
                }
            }
            Event::Vacation { account, vacation, error } => {
                let mut failed = None;
                if let Some(a) = self.account_mut(&account) {
                    a.vacation = match (vacation, error) {
                        (Some(v), None) => VacationState::Loaded(v),
                        (Some(v), Some(e)) => {
                            // Read back after a change that failed: show what the server has.
                            failed = Some(e);
                            VacationState::Loaded(v)
                        }
                        (None, e) => VacationState::Failed(e.unwrap_or_else(|| "this server doesn't offer automatic replies".into())),
                    };
                }
                if let Some(e) = failed {
                    self.notice(format!("automatic replies weren't changed: {e}"), true);
                }
            }
            Event::ServerRules { account, ids, others, cleared, error } => {
                // The rules the server keeps that Quatrelle didn't make (or no longer does):
                // listed as they are, whoever made them owns them.
                // A refusal says nothing of the rules the server keeps: the list stays as it was
                // (emptied, the rules made in Outlook vanished from the dialog each time).
                if error.is_none() {
                    log::info!("rules: {account} keeps {} rules made elsewhere", others.len());
                    self.foreign_rules.retain(|r| r.account != account);
                    self.foreign_rules.extend(others.into_iter().map(|mut r| {
                        r.account.clone_from(&account);
                        r
                    }));
                    self.foreign_rules.sort_by(|a, b| {
                        (a.account.as_str(), a.name.to_lowercase().as_str()).cmp(&(b.account.as_str(), b.name.to_lowercase().as_str()))
                    });
                }
                if let Some(e) = error {
                    self.server_rules_refused(&account, e);
                } else {
                    // Keep the id the server gave each rule: it's what changes or removes it
                    // there next time (nothing else may touch it).
                    let mut changed = false;
                    for r in self.rules.iter_mut().filter(|r| r.account.as_deref() == Some(&account)) {
                        let Some((_, id)) = ids.iter().find(|(name, _)| name == &r.name) else { continue };
                        if r.remote_id.as_deref() != Some(id.as_str()) {
                            r.remote_id = Some(id.clone());
                            changed = true;
                        }
                    }
                    for r in &mut self.rules {
                        let gone = r.remote_id.as_deref().is_some_and(|id| cleared.iter().any(|c| c == id));
                        if gone {
                            r.remote_id = None;
                            changed = true;
                        }
                    }
                    if changed {
                        self.save_rules();
                    }
                }
            }
            Event::Photo { email, media, data, error } => {
                // A picture (or the answer that there is none): kept by address, so the People
                // card shows it without asking again.
                let photo = if error.is_none() && !data.is_empty() {
                    Some(Photo { media, base64: quatrelle_model::vcard::base64_encode(&data) })
                } else {
                    None
                };
                self.person_photos.insert(email.to_ascii_lowercase(), photo);
            }
            Event::Found { account, query, items, error } => self.found_by_server(&account, &query, items, error),
            Event::Older { account, folder, items, more, error } => self.older_from_server(&account, &folder, items, more, error),
            Event::Voted { key, choice, error } => self.voted(&key, &choice, error),
            // Older mail read on the server is shown, not kept (nor indexed).
            Event::Body { key, raw } if self.is_older(&key) => self.show_body(&key, &raw),
            Event::Body { key, raw } => {
                self.show_body(&key, &raw);
                self.index_body(&key, &raw);
                self.downloaded(&key, true);
            }
            Event::BodyFailed { key, error } => {
                for w in self.windows.iter_mut().filter(|w| w.key == key && w.parsed.is_none()) {
                    w.error = Some(error.clone());
                }
                if let Some(r) = self.reading.as_mut()
                    && r.key == key
                {
                    r.error = Some(error);
                }
                self.downloaded(&key, false);
            }
            Event::Sent { id, .. } => {
                if let Some(c) = self.compose.clone().filter(|c| c.id == id) {
                    self.finish_sent(&c, true);
                } else {
                    self.notice("Message sent", false);
                }
                // Delivered from the Outbox: set the flag it owed.
                if let Some((key, flag)) = self.pending_marks.remove(&id) {
                    self.save_pending_marks();
                    self.apply_op(&key.account.clone(), Op::SetFlag { keys: vec![key], flag, on: true });
                }
                self.after_outbox_change();
            }
            Event::Queued { id, error, .. } => {
                if let Some(c) = self.compose.clone().filter(|c| c.id == id) {
                    // It's safe in the Outbox: as far as the user is concerned, the message is done
                    // (a Replied/Forwarded flag stays owed until delivery).
                    self.finish_sent(&c, false);
                    self.notices.pop();
                    self.notice("Saved in the Outbox: it will be sent when you're back online", false);
                } else {
                    log::info!("outbox {id}: still waiting ({error})");
                }
                self.after_outbox_change();
            }
            Event::SendFailed { id, error, .. } => {
                if let Some(c) = self.compose.as_mut().filter(|c| c.id == id) {
                    c.sending = false;
                    c.error = Some(error.clone());
                }
                self.notice(format!("Not sent: {error}"), true);
                self.after_outbox_change();
            }
            Event::OpFailed { error, .. } => self.notice(format!("The server refused a change: {error}"), true),
            Event::OpenUrl { url } => {
                if self.open_urls.len() < 8 {
                    self.open_urls.push(url);
                }
            }
            Event::SignedIn { account, user } => {
                // Who the browser signed in as, kept when it isn't the account's address: tried
                // when the mailbox won't open under that, and named when it still won't.
                let user = user.map(|u| u.trim().to_string()).filter(|u| !u.is_empty() && u.len() < 320);
                if let Some(a) = self.account_mut(&account) {
                    let other = user.filter(|u| !u.eq_ignore_ascii_case(&a.config.email));
                    let changed = a.config.oauth_user != other;
                    a.config.oauth_user = other.clone();
                    let config = a.config.clone();
                    let who = config.email.clone();
                    if changed {
                        self.save_accounts();
                    }
                    self.jobs.push(Job::Start { account: config, password: None });
                    match other {
                        Some(o) => self.notice(format!("{who} is signed in (as {o})"), false),
                        None => self.notice(format!("{who} is signed in"), false),
                    }
                }
            }
            Event::Discovered { email, config, source } => {
                self.discovered.insert(email.trim().to_ascii_lowercase(), Some((config.map(|c| *c), source)));
            }
            Event::Releases { releases, error } => {
                self.update = match error {
                    Some(e) => UpdateCheck::Failed(e),
                    None => match newer_release(env!("CARGO_PKG_VERSION"), self.build_sha.as_deref(), &releases) {
                        Some(r) => UpdateCheck::Available(r.clone()),
                        None => UpdateCheck::UpToDate(releases.first().map(|r| r.tag.clone()).unwrap_or_default()),
                    },
                };
            }
            Event::Fetched { url, data, error } => {
                let p = match (data, error) {
                    (Some(d), _) => Picture::Ready(Arc::new(d)),
                    (None, e) => Picture::Failed(e.unwrap_or_default()),
                };
                self.pictures.insert(url, p);
            }
            Event::Notice { account, message } => {
                let who = self.account(&account).map(|a| a.config.email.clone()).unwrap_or(account);
                self.notice(format!("{who}: {message}"), true);
            }
        }
    }
}

impl Session {
    /// The categories an account's server keeps (Exchange's master list, as Outlook made them):
    /// each takes the colour Outlook gives it, and one not made here joins the list (as
    /// Outlook's Categorize lists the master list). Those made here only stay.
    fn take_master_categories(&mut self, account: &str, categories: Vec<(String, Option<u8>)>) {
        let mut changed = 0usize;
        for (name, color) in categories.into_iter().take(500) {
            let name = name.trim().to_string();
            if name.is_empty() || name.chars().count() > 255 {
                continue;
            }
            let picked = self.category_color_of(&name);
            match self.prefs.categories.iter_mut().find(|c| c.name.eq_ignore_ascii_case(&name)) {
                Some(c) => {
                    if let Some(color) = color.filter(|col| *col != c.color) {
                        c.color = color;
                        changed += 1;
                    }
                }
                None => {
                    self.prefs.categories.push(quatrelle_model::organize::Category::new(&name, color.unwrap_or(picked)));
                    changed += 1;
                }
            }
        }
        if changed > 0 {
            log::info!("{account}: {changed} categories taken from the server's list");
            self.revision = self.revision.wrapping_add(1);
        }
    }
}
