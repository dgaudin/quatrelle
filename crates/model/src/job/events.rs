//! What the network layer reports back to the engine.

use super::*;
use crate::EmailAddress;

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
    /// A folder's cached contents changed on disk: reload it.
    FolderChanged {
        account: String,
        folder: String,
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
    Discovered {
        email: String,
        config: Option<Box<AccountConfig>>,
        source: String,
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
    /// A CalDAV / CardDAV sync's result, to merge.
    DavSynced {
        account: String,
        sync: Box<crate::pim::DavSync>,
    },
    /// How a data file import is going (`done`: finished, or failed with `error`). The folders
    /// come as [`Event::Folders`] first.
    Imported {
        account: String,
        messages: usize,
        skipped: usize,
        done: bool,
        error: Option<String>,
    },
    /// A data file compacted: its size before and after, or why it couldn't be.
    Compacted {
        account: String,
        before: u64,
        after: u64,
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
    /// How an export to a data file is going (`done`: finished, or failed with `error`).
    Exported {
        account: String,
        messages: usize,
        skipped: usize,
        done: bool,
        error: Option<String>,
    },
    /// A vote went (`error` `None`), or why it didn't.
    Voted {
        key: MessageKey,
        choice: String,
        error: Option<String>,
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
        people: Vec<EmailAddress>,
        more: bool,
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
