// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
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
            | Event::DirectoryCertificates { .. }
            | Event::PgpKeys { .. }
            | Event::PgpDecrypted { .. }
            | Event::PgpBackedUp { .. }) => self.apply_smime(e),
            Event::Archived { account, moved, kept } => self.archived(&account, moved, kept),
            Event::PutBack { account, back, kept } => self.put_back_done(&account, back, kept),
            Event::DirectoryPeople { account, query, people, more, error } => self.apply_directory(account, query, people, more, error),
            Event::ListExpanded { account, address, item_id, members, more, total, error } => {
                self.list_expanded(&account, address, item_id, members, more, total, error)
            }
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
                    self.notice_with("{who}: {error}", &[("who", &who), ("error", error)], true);
                }
                if let SyncStatus::NeedsPassword { error } = &status {
                    let (who, browser) = self.account(&account).map(|a| (a.config.email.clone(), a.config.oauth.is_some())).unwrap_or_default();
                    // An account that signs in in the browser has no password to ask for.
                    let values = [("who", who.as_str()), ("error", error.as_str())];
                    match (browser, error.is_empty()) {
                        (true, true) => self.notice_with("{who} needs you to sign in again", &values, true),
                        (true, false) => self.notice_with("{who} needs you to sign in again: {error}", &values, true),
                        (false, true) => self.notice_with("{who} needs its password", &values, true),
                        (false, false) => self.notice_with("{who} needs its password: {error}", &values, true),
                    }
                    if !browser && self.asked_password.insert(account.clone()) {
                        self.ask_password.push(account.clone());
                    }
                }
                // Coming online, not every "Online" after it: the sync says it after each folder
                // and each check for new mail (Syncing, then Online, every half minute), and the
                // rules were read (and a refused one sent again) that often — over a hundred
                // times a pass on a big mailbox. Losing the server starts it over.
                if matches!(
                    status,
                    SyncStatus::Offline
                        | SyncStatus::Connecting
                        | SyncStatus::Error { .. }
                        | SyncStatus::NeedsPassword { .. }
                        | SyncStatus::NeedsTrust { .. }
                ) {
                    self.rules_asked.remove(&account);
                    self.rules_online.remove(&account);
                }
                // A server proposed another domain: the account waits for the user's answer, and the
                // question is asked once while Quatrelle runs (`ask_trust_once`).
                if let SyncStatus::NeedsTrust { site } = &status
                    && self.account(&account).is_some()
                {
                    self.ask_trust_once(&account, site);
                }
                let online = matches!(status, SyncStatus::Online { .. }) && self.rules_online.insert(account.clone());
                // A server that has started syncing is readable: its rules are asked for then,
                // not once the whole mailbox (public folders and all) has been walked — that
                // takes minutes, and the Rules dialog would sit empty until then.
                let connecting = matches!(status, SyncStatus::Syncing { .. }) && self.rules_asked.insert(account.clone());
                // Its mail synced in full: an Inbox empty until now (so never said to have
                // changed) is marked, so its first message counts as arriving, not as the backlog.
                if matches!(status, SyncStatus::Online { .. }) {
                    self.mark_inbox(&account);
                }
                let connected = matches!(status, SyncStatus::Online { .. });
                if let Some(a) = self.account_mut(&account) {
                    a.status = status;
                }
                // Once a day, mail older than the account's age is looked for (archiving by age).
                if connected {
                    self.consider_age(&account);
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
                // The pool counts a folder as it syncs it, and lists them all again now and then
                // (Drafts made, a draft's folder synced) with the others' counts from their last
                // sync, from before mail read here since reached the server: a count it reports
                // unchanged is no news, and the one counted here stands, until the folder is
                // synced (`FolderChanged`).
                let here: HashMap<String, (u32, u32)> =
                    self.account(&account).map(|a| a.folders.iter().map(|f| (f.path.clone(), (f.unread, f.total))).collect()).unwrap_or_default();
                for f in &mut folders {
                    let now = (f.unread, f.total);
                    let key = (account.clone(), f.path.clone());
                    let synced = self.counts_stale.remove(&key);
                    let before = self.reported_counts.insert(key, now);
                    if before == Some(now)
                        && !synced
                        && let Some(&(unread, total)) = here.get(&f.path)
                    {
                        (f.unread, f.total) = (unread, total);
                    }
                }
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
                // The folder list comes again every few seconds, mostly the same: the search
                // index follows what changed in it (the list shows the search again then), rather
                // than starting over.
                if self.rescope_index() && self.search.is_some() {
                    self.refresh_list();
                }
                self.follow_public_favorites();
            }
            Event::FolderChanged { account, folder } => match self.echo_of(&account, &folder) {
                // The echo of a flag or category changed here: the folder in memory has it already.
                mailboxes::Echo::Kept => {}
                // A move or delete made here, on disk now: read again, its count no news.
                mailboxes::Echo::Made => self.folder_changed(&account, &folder),
                // Synced, or changed on disk otherwise: the next count reported for it is news,
                // whatever it is (a mark undone on another device comes back this way). The count
                // last reported is kept, for a change the server refuses.
                mailboxes::Echo::None => {
                    self.counts_stale.insert((account.clone(), folder.clone()));
                    self.folder_changed(&account, &folder);
                }
            },
            Event::FolderSynced { account, folder, change } => {
                // A sync that found mail changed: the next count reported for the folder is news.
                self.counts_stale.insert((account.clone(), folder.clone()));
                self.folder_synced(&account, &folder, *change)
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
            // A discovery during a sync named another domain: the mail goes on, the user is asked.
            Event::TrustAsked { account, site } => {
                if self.account(&account).is_some() {
                    self.ask_trust_once(&account, &site);
                }
            }
            Event::CalendarFetched { account, calendar, ics, etag, modified, unchanged, error } => {
                let read = match (ics, error) {
                    (_, Some(e)) => pim::web::WebRead::Failed(e),
                    (Some(ics), None) => pim::web::WebRead::Read { ics, etag, modified },
                    (None, None) if unchanged => pim::web::WebRead::Unchanged,
                    (None, None) => pim::web::WebRead::Failed("nothing came back".into()),
                };
                self.apply_web_calendar(&account, &calendar, read);
            }
            Event::Rooms { account, lists, rooms, error } => self.rooms_answered(&account, lists, rooms, error),
            Event::Availability { account, from, to, people, error } => self.availability_answered(&account, from, to, people, error),
            Event::MessagesFiled { account, folder, added, failed } => {
                if self.filed.len() < 100 {
                    self.filed.push(Filed { account, folder, added, failed });
                }
            }
            Event::MessageFileRead { path, raw } => self.message_file_read(&path, raw),
            Event::Imported { account, messages, skipped, done, error, .. } => self.imported(&account, messages, skipped, done, error),
            Event::CompactProgress { account, in_place, done, total } => self.compact_progress(&account, in_place, done, total),
            Event::Compacted { account, before, after, recompressed, no_room, error } => {
                self.compacted(&account, Compacted { account: String::new(), before, after, recompressed }, no_room, error)
            }
            Event::FolderSizes { account, server, folders, quota, error } => self.sizes_answered(&account, server, folders, quota, error),
            Event::Recoverable { account, ask, offset, items, more, error } => self.recoverable_listed(&account, ask, offset, items, more, error),
            Event::RecoverableDone { account, ask, done, purge, error } => self.recoverable_done(&account, ask, done, purge, error),
            Event::LlmAnswered { id, text, error } => {
                if !self.scam_answered(id, &text, error.as_deref()) {
                    self.llm_answered(id, text, error);
                }
            }
            Event::LlmModels { url, models, error } => self.llm_models_answered(url, models, error),
            Event::WebexSignedIn { user, error } => self.webex_signed_in(user, error),
            Event::WebexCreated { ask, meeting, error } => self.webex_created(ask, meeting, error),
            Event::WebexUpdated { id, meeting, error } => self.webex_updated(&id, meeting, error),
            Event::WebexDeleted { error, .. } => self.webex_deleted(error),
            Event::Exported { account, messages, skipped, already, occurrences, replaced, done, error } => {
                self.exported(&account, messages, (skipped, already), (occurrences, replaced), done, error)
            }
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
                    self.notice_with("automatic replies weren't changed: {error}", &[("error", &e)], true);
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
            Event::EventFile { ask, data, error } => self.event_file_came(ask, data, error),
            Event::Found { account, query, items, error } => self.found_by_server(&account, &query, items, error),
            Event::Older { account, folder, items, more, error } => self.older_from_server(&account, &folder, items, more, error),
            Event::Voted { key, choice, error } => self.voted(&key, &choice, error),
            // Older mail read on the server is shown, not kept (nor indexed).
            Event::Body { key, raw } if self.is_older(&key) => {
                self.source_arrived(&key, &raw);
                // The open message's: parsed on another thread, where the window has one.
                if !self.defer_arrived(&key, &raw) {
                    self.show_body(&key, &raw);
                }
            }
            Event::Body { key, raw } => {
                self.source_arrived(&key, &raw);
                if !self.defer_arrived(&key, &raw) {
                    self.show_body(&key, &raw);
                }
                self.index_body(&key, &raw);
                self.downloaded(&key, true);
                self.scam_body(&key, Some(&raw));
            }
            Event::Filled { summary } => self.filled(*summary),
            Event::TextFirst { key, raw } => self.show_text_first(&key, &raw),
            Event::BodyFailed { key, error } => {
                self.source_failed(&key, &error);
                for w in self.windows.iter_mut().filter(|w| w.key == key && (w.parsed.is_none() || w.text_first)) {
                    w.error = Some(error.clone());
                }
                if let Some(r) = self.reading.as_mut()
                    && r.key == key
                {
                    r.error = Some(error);
                }
                self.downloaded(&key, false);
                self.scam_body(&key, None);
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
                    self.mark_original(key, flag);
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
                self.notice_with("Not sent: {error}", &[("error", &error)], true);
                self.after_outbox_change();
            }
            Event::DraftReturned { account, raw, error } => self.draft_returned(&account, &raw, &error),
            Event::ArchiveTags { account, tags } => self.tags_told(account, tags),
            Event::ArchiveTagRead { key, tag, date } => self.tag_read(key, tag, date),
            Event::OpFailed { account, op, error } => {
                self.policy_failed(&account, &op);
                // What it counted here (a message read) didn't happen: the folders it names count
                // as the server last said, which the sync that follows may not say again (its
                // count unchanged, it reports none).
                let folders: BTreeSet<String> = op.keys().iter().filter(|k| k.account == account).map(|k| k.folder.clone()).collect();
                for f in folders {
                    let reported = self.reported_counts.get(&(account.clone(), f.clone())).copied();
                    if let (Some((unread, total)), Some(a)) = (reported, self.account_mut(&account))
                        && let Some(folder) = a.folders.iter_mut().find(|x| x.path == f)
                    {
                        (folder.unread, folder.total) = (unread, total);
                    }
                }
                self.notice_with("A change wasn't kept: {error}", &[("error", &error)], true);
            }
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
                        Some(o) => self.notice_with("{who} is signed in (as {user})", &[("who", &who), ("user", &o)], false),
                        None => self.notice_with("{who} is signed in", &[("who", &who)], false),
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
                let signature = self.accounts.iter().any(|a| a.config.signature_raw_html.contains(url.split('?').next().unwrap_or(&url)));
                self.pictures.insert(url, p);
                if signature {
                    self.keep_signature_pictures();
                }
            }
            Event::Notice { account, message } => {
                // A change that failed among them: what the account says changed next is read.
                self.forget_echoes(&account);
                // Said whole when no account says it (an export, the keychain).
                match self.account(&account).map(|a| a.config.email.clone()).filter(|w| !w.is_empty()) {
                    Some(who) => self.notice_with("{who}: {error}", &[("who", &who), ("error", &message)], true),
                    None if account.is_empty() => self.notice(message, true),
                    None => self.notice_with("{who}: {error}", &[("who", &account), ("error", &message)], true),
                }
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
