// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What the network layer reports back to the engine.

use super::*;
use crate::Addressee;

/// What the network layer reports back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum Event {
    Status {
        account: String,
        status: SyncStatus,
    },
    /// The folder list (with counts) changed.
    Folders {
        account: String,
        folders: Vec<Folder>,
    },
    /// What [`Job::Archive`](crate::Job::Archive) did: `moved` pairs each message's key (now gone
    /// from the account, or on its way out through the journal) with its key in the Archives
    /// account; `kept` are those left where they were, with why (a message, in English).
    Archived {
        account: String,
        moved: Vec<(crate::MessageKey, crate::MessageKey)>,
        kept: Vec<(crate::MessageKey, String)>,
    },
    /// What [`Job::PutBack`](crate::Job::PutBack) did: `back` are the archived messages the server
    /// has again, with their UID there when it is known; `kept` stay in Archives, with why.
    PutBack {
        account: String,
        back: Vec<(crate::MessageKey, Option<u32>)>,
        kept: Vec<(crate::MessageKey, String)>,
    },
    /// A folder's cached contents changed on disk: reload it.
    FolderChanged {
        account: String,
        folder: String,
    },
    /// A sync changed a folder's list on disk, and what changed is told: the folder in memory
    /// and the list on screen take it as they are, rather than reading all of the folder again
    /// ([`Event::FolderChanged`] stays for what can't be told so: a first sync, a folder
    /// numbered anew, more changed than [`ListChange`] carries).
    FolderSynced {
        account: String,
        folder: String,
        change: Box<ListChange>,
    },
    /// A page of a folder's older mail read on the server ([`Job::Older`]), not kept; `more`
    /// when the server has more past it; `error` when it couldn't be asked.
    Older {
        account: String,
        folder: String,
        items: Vec<crate::Summary>,
        more: bool,
        error: Option<String>,
    },
    /// What the server found for a search (whole messages, ready to show), merged with what the
    /// local index found; `error` when it couldn't be asked (offline, a transport without one).
    Found {
        account: String,
        query: String,
        items: Vec<crate::Summary>,
        error: Option<String>,
    },
    /// A message's source arrived (it is cached on disk too).
    Body {
        key: MessageKey,
        raw: Vec<u8>,
    },
    /// A message downloaded: its summary as the cached folder has it now, filled in from its
    /// source (its preview, files, conversation). The folder in memory takes it, rather than
    /// reading all of the folder again.
    Filled {
        summary: Box<crate::Summary>,
    },
    /// A big message's text arrived ahead of the whole message: `raw` is a source holding the
    /// text and the list of files to come, not kept on disk. [`Event::Body`] follows.
    TextFirst {
        key: MessageKey,
        raw: Vec<u8>,
    },
    BodyFailed {
        key: MessageKey,
        error: String,
    },
    Sent {
        account: String,
        id: u64,
    },
    SendFailed {
        account: String,
        id: u64,
        error: String,
    },
    /// The message couldn't be sent yet (offline, or the server asked to try later): it waits in
    /// the Outbox and goes out when the account is back online.
    Queued {
        account: String,
        id: u64,
        error: String,
    },
    /// A mailbox change could not be applied on the server (it was rolled back locally).
    OpFailed {
        account: String,
        op: Op,
        error: String,
    },
    /// The archive policy tags the account's server offers (Exchange), read once a session; none
    /// when it has none or refuses.
    ArchiveTags {
        account: String,
        tags: Vec<crate::ArchiveTag>,
    },
    /// The archive policy tag a message carries (`None`: it follows its folder's) and the day the
    /// server plans to move it, as `YYYY-MM-DD`, read when it was opened.
    ArchiveTagRead {
        key: MessageKey,
        tag: Option<String>,
        date: Option<String>,
    },
    /// Quatrelle's recent releases (newest first), or why they couldn't be had.
    Releases {
        releases: Vec<Release>,
        error: Option<String>,
    },
    /// A downloaded picture (`data`), or why it couldn't be fetched.
    Fetched {
        url: String,
        data: Option<Vec<u8>>,
        error: Option<String>,
    },
    /// What [`Job::FetchCalendar`] read: the calendar (`ics`) with the version the server named,
    /// unchanged since the one asked about, or why it couldn't be read.
    CalendarFetched {
        account: String,
        calendar: String,
        ics: Option<String>,
        etag: String,
        modified: String,
        unchanged: bool,
        error: Option<String>,
    },
    /// Server settings found for an address (`None`: nothing published; guess instead).
    /// `unverified`: the servers were named through DNS records nothing vouches for, off the
    /// address's own domain; they are shown and confirmed before a password goes to them.
    Discovered {
        email: String,
        config: Option<Box<AccountConfig>>,
        source: String,
        #[serde(default)]
        unverified: bool,
    },
    /// Show this page in the browser (an OAuth sign-in page).
    OpenUrl {
        url: String,
    },
    /// An OAuth account is signed in (its token is in the keychain): start it. `user` is who
    /// the provider says signed in, when it said.
    SignedIn {
        account: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user: Option<String>,
    },
    /// How much of the account's space on the server is used and its limit, in bytes (`limit`
    /// 0: no limit published).
    Quota {
        account: String,
        used: u64,
        limit: u64,
    },
    /// The account's automatic replies as the server has them; `None` with an error when it
    /// offers none (or they couldn't be read or changed).
    Vacation {
        account: String,
        vacation: Option<Vacation>,
        error: Option<String>,
    },
    /// The server's rules were read, or brought up to date: the id each of Quatrelle's rules has
    /// there (by name), the rules it didn't make (Outlook's own, the server's web page, a Sieve
    /// script) as the server keeps them, the ids of the rules removed, or why it couldn't be
    /// done.
    ServerRules {
        account: String,
        ids: Vec<(String, String)>,
        #[serde(default)]
        others: Vec<crate::organize::ForeignRule>,
        cleared: Vec<String>,
        error: Option<String>,
    },
    /// A directory photo: its bytes and type (`image/jpeg`), or why there is none.
    Photo {
        email: String,
        media: String,
        data: Vec<u8>,
        error: Option<String>,
    },
    /// One of an appointment's files fetched from the server ([`crate::Job::FetchEventFile`]): its
    /// bytes, or why it couldn't be.
    EventFile {
        ask: u64,
        data: Vec<u8>,
        error: Option<String>,
    },
    /// A CalDAV / CardDAV sync's result, to merge.
    DavSynced {
        account: String,
        sync: Box<crate::pim::DavSync>,
    },
    /// The account's calendar or address book server leads to a site (a registrable domain) the
    /// account doesn't trust: nothing was sent there, and the account's calendars and contacts
    /// wait until the user answers (the account starts again if the site is allowed).
    TrustAsked {
        account: String,
        site: String,
    },
    /// What [`Job::FileMessages`] did: how many went into the folder, and the files that
    /// couldn't (their names, and why).
    MessagesFiled {
        account: String,
        folder: String,
        added: usize,
        failed: Vec<(String, String)>,
    },
    /// A message file read ([`Job::ReadMessageFile`]): the message, or why it can't be read.
    MessageFileRead {
        path: String,
        raw: Result<Vec<u8>, String>,
    },
    /// How a data file import is going (`done`: finished, stopped or failed with `error`). The
    /// folders come as [`Event::Folders`] first. `messages` of `total` (`None`: the file's counts
    /// couldn't be read) are in, the folder being read is `folder` (its path; empty before the
    /// first and at the end); `stopped`: it ended because it was asked to
    /// ([`Job::StopImport`](super::Job::StopImport)), and can be gone on with.
    Imported {
        account: String,
        messages: usize,
        skipped: usize,
        total: Option<usize>,
        folder: String,
        stopped: bool,
        done: bool,
        error: Option<String>,
    },
    /// How far Compact Now has come ([`Job::CompactDataFile`]): making smaller what an earlier
    /// version kept uncompressed (`done` of `total` files looked at; the window goes on as ever),
    /// then, `in_place`, compacting with the data file had to itself.
    CompactProgress {
        account: String,
        in_place: bool,
        done: u64,
        total: u64,
    },
    /// A data file compacted: its size before and after, what making attachments smaller saved
    /// first (`recompressed`, bytes as kept), whether the drive hadn't the room for that
    /// (`no_room`: only compacted), or why it couldn't be.
    Compacted {
        account: String,
        before: u64,
        after: u64,
        #[serde(default)]
        recompressed: u64,
        #[serde(default)]
        no_room: bool,
        error: Option<String>,
    },
    /// The size of each of an account's folders ([`Job::LocalFolderSizes`], or with `server`
    /// [`Job::ServerFolderSizes`]), from the top down, and the mailbox's limits where the
    /// server tells them; or why they couldn't be read.
    FolderSizes {
        account: String,
        server: bool,
        folders: Vec<FolderSize>,
        quota: Option<MailboxQuota>,
        error: Option<String>,
    },
    /// A page of Recoverable Items ([`Job::ListRecoverable`]): the items from `offset` on, whether
    /// the server has `more`, or why they couldn't be read.
    Recoverable {
        account: String,
        ask: u64,
        offset: u32,
        items: Vec<Recoverable>,
        more: bool,
        error: Option<String>,
    },
    /// What [`Job::RestoreRecoverable`] did: the ids it dealt with (`purge`: deleted for good,
    /// else back in Deleted Items), or why it stopped (some may be done all the same).
    RecoverableDone {
        account: String,
        ask: u64,
        done: Vec<String>,
        purge: bool,
        error: Option<String>,
    },
    /// How an export to a data file is going (`done`: finished, or failed with `error`).
    Exported {
        account: String,
        messages: usize,
        skipped: usize,
        /// Messages a data file already there had (by `Message-ID`), not written again.
        #[serde(default)]
        already: usize,
        /// Occurrences of series whose rule Outlook can't keep, each written as an appointment
        /// of its own.
        #[serde(default)]
        occurrences: usize,
        /// Calendar items, contacts, notes and tasks changed since a data file added to had
        /// them, written in place of their earlier copies.
        #[serde(default)]
        replaced: usize,
        done: bool,
        error: Option<String>,
    },
    /// A vote went (`error` `None`), or why it didn't.
    Voted {
        key: MessageKey,
        choice: String,
        error: Option<String>,
    },
    /// A draft nothing could keep: the server refused it (or the account is offline) and this
    /// computer's disk refused its copy. It comes back whole, to be opened in a window again,
    /// `error` saying why: a draft is never gone without being on the server, on the disk or on
    /// the screen.
    DraftReturned {
        account: String,
        raw: Vec<u8>,
        error: String,
    },
    /// Something the user should see in the status bar.
    Notice {
        account: String,
        message: String,
    },
    /// The categories the account's server keeps, as Outlook made them (Exchange's master
    /// category list): each name, and its colour in Quatrelle's palette when it has one.
    Categories {
        account: String,
        categories: Vec<(String, Option<u8>)>,
    },
    /// What the server found for a [`Job::SearchServer`]: the messages, now in their folders'
    /// lists, or why it couldn't search. Sent as they're found (more to come until `done`).
    ServerSearched {
        account: String,
        query: String,
        keys: Vec<MessageKey>,
        error: Option<String>,
        done: bool,
    },
    /// The rooms the server offers ([`Job::GetRooms`]): its room lists and the rooms in them
    /// (none when the organisation set no lists up, which many on-premises servers haven't),
    /// or why they couldn't be read.
    Rooms {
        account: String,
        lists: Vec<crate::scheduling::RoomList>,
        rooms: Vec<crate::scheduling::Room>,
        error: Option<String>,
    },
    /// When people and rooms are busy over a window ([`Job::GetAvailability`]): one entry per
    /// address asked about, in that order, each with its own refusal when the server gave
    /// one; `error` when the whole request failed.
    Availability {
        account: String,
        from: i64,
        to: i64,
        people: Vec<crate::scheduling::Availability>,
        error: Option<String>,
    },
    /// The language model's answer to [`Job::AskLlm`], or why there's none.
    LlmAnswered {
        id: u64,
        text: String,
        error: Option<String>,
    },
    /// The models the server at `url` offers ([`Job::ListLlmModels`]), or why they're not known.
    LlmModels {
        url: String,
        models: Vec<String>,
        error: Option<String>,
    },
    /// Webex's sign-in is done ([`Job::WebexSignIn`]): who signed in, when Webex said, or why
    /// it failed.
    WebexSignedIn {
        user: Option<String>,
        error: Option<WebexError>,
    },
    /// The meeting [`Job::WebexCreate`] made, or why there's none.
    WebexCreated {
        ask: u64,
        meeting: Option<WebexMeeting>,
        error: Option<WebexError>,
    },
    /// The meeting [`Job::WebexUpdate`] changed, as Webex now has it, or why it couldn't.
    WebexUpdated {
        id: String,
        meeting: Option<WebexMeeting>,
        error: Option<WebexError>,
    },
    /// Whether [`Job::WebexDelete`] deleted its meeting.
    WebexDeleted {
        id: String,
        error: Option<WebexError>,
    },
    /// What the address book keeps for each address asked about ([`Job::FindCertificates`]):
    /// its certificates (DER, none when it has none), or why it couldn't be asked.
    DirectoryCertificates {
        account: String,
        found: Vec<(String, Vec<Vec<u8>>)>,
        error: Option<String>,
    },
    /// Who the directory has for `query` ([`Job::SearchDirectory`]), `more` when it has others it
    /// didn't give; or why it couldn't be asked.
    DirectoryPeople {
        account: String,
        query: String,
        people: Vec<Addressee>,
        more: bool,
        error: Option<String>,
    },
    /// The members of the list [`Job::ExpandList`] asked about (its `address` or `item_id`, as
    /// asked), each with its kind (a list in it stays a list); `more` when the server gave only
    /// part of them, `total` how many it says the list has; or why it couldn't be asked.
    ListExpanded {
        account: String,
        address: String,
        item_id: Option<String>,
        members: Vec<Addressee>,
        more: bool,
        total: usize,
        error: Option<String>,
    },
    /// What [`Job::ImportIdentity`] kept: the certificate that signs and encrypts and its issuers
    /// (DER), and the older ones kept only to read mail encrypted to them (each one's certificate);
    /// or why not. A certificate older than the one in use is kept for reading only.
    Identity {
        account: String,
        chain: Vec<Vec<u8>>,
        older: Vec<Vec<u8>>,
        /// The certificate just kept went among the older ones (it's older than the one in use).
        for_reading: bool,
        error: Option<String>,
    },
    /// A message [`Job::Protect`] signed and/or encrypted, ready to send, or why it couldn't be.
    Protected {
        account: String,
        id: u64,
        raw: Option<Private>,
        error: Option<String>,
    },
    /// What [`Job::Decrypt`] found inside (a MIME entity); `authenticated`: it was encrypted with
    /// AES-GCM, so it can't have been changed on the way.
    Decrypted {
        key: MessageKey,
        content: Option<Private>,
        authenticated: bool,
        error: Option<String>,
    },
    /// What [`Job::FindHeldKeys`] found, or why it couldn't look.
    HeldKeys {
        keys: Vec<HeldKey>,
        error: Option<String>,
    },
    /// What [`Job::CheckRevocation`] heard about `cert` (DER, the chain's first).
    Revocation {
        cert: Vec<u8>,
        status: Revocation,
    },
    /// The account's own OpenPGP keys after [`Job::Pgp`] (imported, made, extended, used or
    /// forgotten): their public parts (binary), the one that signs and decrypts first; `changed`:
    /// what was just done; or why it couldn't be done (nothing was).
    PgpKeys {
        account: String,
        keys: Vec<Vec<u8>>,
        changed: Option<PgpChange>,
        error: Option<String>,
    },
    /// What [`PgpWork::Decrypt`] found inside (a MIME entity), what a signature in it says, and
    /// the public keys attached inside (binary, a few at most), to be learnt as those outside are.
    PgpDecrypted {
        key: MessageKey,
        content: Option<Private>,
        signature: Option<PgpSignature>,
        keys: Vec<Vec<u8>>,
        error: Option<String>,
    },
    /// [`PgpWork::Backup`] wrote key `fingerprint`'s backup, or why it couldn't.
    PgpBackedUp {
        account: String,
        fingerprint: String,
        error: Option<String>,
    },
}

/// What saves of a folder's list changed in it ([`Event::FolderSynced`]): the messages come or
/// changed, as the list on disk has them now, and the numbers gone; with what the list holds
/// after, to check the folder in memory against (it reads the folder again when they differ).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListChange {
    /// The numbering the list is under.
    pub validity: u32,
    /// Messages come or changed, whole.
    pub upserts: Vec<crate::Summary>,
    /// Numbers gone from the list.
    pub removed: Vec<u32>,
    /// How many messages the list holds after.
    pub count: usize,
    /// [`ListChange::print`] of the numbers the list holds after.
    pub print: u64,
}

impl ListChange {
    /// A print of a set of message numbers that doesn't depend on their order: two lists with
    /// the same numbers have the same print (a list that lost a message and gained another
    /// has another one, but for one chance in 2⁶⁴).
    pub fn print(uids: impl IntoIterator<Item = u32>) -> u64 {
        uids.into_iter().fold(0u64, |sum, uid| {
            // SplitMix64's finalizer: each number spread over all 64 bits.
            let mut z = u64::from(uid).wrapping_add(0x9e37_79b9_7f4a_7c15);
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            sum.wrapping_add(z ^ (z >> 31))
        })
    }
}

/// A certificate with its private key held where Quatrelle can't read it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldKey {
    /// What it's called there (the card's label, the keychain's name).
    pub label: String,
    /// The certificate (DER).
    pub cert: Vec<u8>,
    /// Where the key is, for the network layer to find it again (opaque here).
    pub at: String,
}

/// Whether a certificate's authority withdrew it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Revocation {
    /// Not withdrawn; the answer holds until `until` (Unix seconds), when it says.
    Good { until: Option<i64> },
    /// Withdrawn at `at` (Unix seconds).
    Revoked { at: i64 },
    /// No answer to go by (none given, or none to trust), and why.
    Unknown { why: String },
}
