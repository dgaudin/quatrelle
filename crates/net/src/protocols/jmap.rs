//! JMAP on the desktop: the portable transport in `quatrelle-jmap`, run over this crate's HTTP
//! client (each request waits for its answer, so the transport's futures are ready at once),
//! plus push over the session's EventSource (RFC 8620 §7.3).

use quatrelle_model::{Folder, MessageKey, Op};
use quatrelle_store::Store;

pub use quatrelle_jmap::{FOUND_MAX, FolderMark, Mailbox, State, VALIDITY, block_on, folders, name_special, session_url, utc_date};

use crate::sync::{SyncOptions, SyncReport};
use crate::{NetError, Result, http};

impl From<quatrelle_jmap::Error> for NetError {
    fn from(e: quatrelle_jmap::Error) -> NetError {
        use quatrelle_jmap::Error as E;
        match e {
            E::Io(m) => NetError::Io(std::io::Error::other(m)),
            E::Tls(m) => NetError::Tls(m),
            E::Protocol(m) => NetError::Protocol(m),
            E::No(m) => NetError::No(m),
            E::Bad(m) => NetError::Bad(m),
            E::Auth(m) => NetError::Auth(m),
            E::Closed(m) => NetError::Closed(m),
            E::Send(m) => NetError::Smtp(m),
            E::Store(e) => NetError::Store(e),
        }
    }
}

impl From<NetError> for quatrelle_jmap::Error {
    fn from(e: NetError) -> quatrelle_jmap::Error {
        use quatrelle_jmap::Error as E;
        match e {
            NetError::Io(e) => E::Io(e.to_string()),
            NetError::Tls(m) => E::Tls(m),
            NetError::Protocol(m) => E::Protocol(m),
            NetError::No(m) | NetError::Refused(m) => E::No(m),
            NetError::Bad(m) => E::Bad(m),
            NetError::Auth(m) => E::Auth(m),
            NetError::Closed(m) => E::Closed(m),
            NetError::Smtp(m) | NetError::Later(m) => E::Send(m),
            NetError::Store(e) => E::Store(e),
        }
    }
}

/// This crate's HTTP client, for the portable transport.
pub struct Blocking;

impl quatrelle_jmap::Http for Blocking {
    async fn send(&self, req: quatrelle_jmap::Request) -> quatrelle_jmap::Result<quatrelle_jmap::Response> {
        let r = http::Request { method: req.method, url: req.url, headers: req.headers, body: req.body, max: req.max };
        let resp = http::send(&r)?;
        Ok(quatrelle_jmap::Response { status: resp.status, body: resp.body })
    }
}

/// A signed-in JMAP session.
pub struct Client(quatrelle_jmap::Client<Blocking>);

impl Client {
    /// Fetch the session and check it offers mail. `bearer` signs in with an OAuth access token.
    pub fn connect(url: &str, user: &str, secret: &str, bearer: bool) -> Result<Client> {
        Ok(Client(block_on(quatrelle_jmap::Client::connect(Blocking, url, user, secret, bearer))?))
    }

    /// Carry a new OAuth access token from now on.
    pub fn set_bearer(&mut self, token: &str) {
        self.0.set_bearer(token);
    }

    /// Listen for changes (`state` events, with `ping`s to keep the line open); `None` when the
    /// server doesn't push.
    pub fn events(&self) -> Result<Option<http::EventStream<crate::stream::Stream>>> {
        let Some(url) = self.0.events_url() else { return Ok(None) };
        http::open_events(&url, &[("Authorization", self.0.authorization())]).map(Some)
    }

    /// Space used and the limit in bytes for the whole account (RFC 9425); `None` without quotas.
    pub fn quota(&self) -> Result<Option<(u64, u64)>> {
        Ok(block_on(self.0.quota())?)
    }

    /// The automatic replies (RFC 8621 §8); an error when the server offers none.
    pub fn vacation(&self) -> Result<quatrelle_model::job::Vacation> {
        Ok(block_on(self.0.vacation())?)
    }

    pub fn set_vacation(&self, v: &quatrelle_model::job::Vacation) -> Result<()> {
        Ok(block_on(self.0.set_vacation(v))?)
    }
}

/// Refresh the folder list (and the mailbox ids behind it).
pub fn list_folders(c: &Client, st: &mut State, account: &str) -> Result<Vec<Folder>> {
    Ok(block_on(quatrelle_jmap::list_folders(&c.0, st, account))?)
}

/// Bring one folder's cache up to date.
pub fn sync_folder(c: &Client, st: &mut State, store: &Store, account: &str, folder: &str, opts: &SyncOptions) -> Result<SyncReport> {
    sync_folder_at(c, st, store, account, folder, opts, crate::worker::now())
}

/// [`sync_folder`] as of `now` (Unix seconds): what came in before the time kept leaves the cache.
pub fn sync_folder_at(c: &Client, st: &mut State, store: &Store, account: &str, folder: &str, opts: &SyncOptions, now: i64) -> Result<SyncReport> {
    Ok(block_on(quatrelle_jmap::sync_folder(&c.0, st, store, account, folder, opts, now))?)
}

/// Download, newest first, up to `budget` sources of a folder's kept messages that have none
/// yet (whatever their size when `opts.full`); how many are still to download.
pub fn download_ahead(c: &Client, st: &State, store: &Store, account: &str, folder: &str, opts: &SyncOptions, budget: usize) -> Result<usize> {
    Ok(block_on(quatrelle_jmap::download_ahead(&c.0, st, store, account, folder, opts, budget))?)
}

/// What the server finds in a folder for a search (issue #4), brought into its list.
pub fn find_on_server(
    c: &Client,
    st: &mut State,
    store: &Store,
    account: &str,
    folder: &str,
    filter: &serde_json::Value,
    cap: usize,
) -> Result<(Vec<MessageKey>, usize)> {
    Ok(block_on(quatrelle_jmap::find_on_server(&c.0, st, store, account, folder, filter, cap))?)
}

/// Download one message's source (and cache it).
pub fn fetch_body(c: &Client, st: &State, store: &Store, key: &MessageKey) -> Result<Vec<u8>> {
    Ok(block_on(quatrelle_jmap::fetch_body(&c.0, st, store, key))?)
}

/// Apply one change on the server.
pub fn apply_remote(c: &Client, st: &State, op: &Op) -> Result<()> {
    Ok(block_on(quatrelle_jmap::apply_remote(&c.0, st, op))?)
}

/// Store a message in a folder (a draft, or the copy of a sent message).
pub fn append(c: &Client, st: &State, folder: &str, raw: &[u8], keywords: &[&str]) -> Result<String> {
    Ok(block_on(quatrelle_jmap::append(&c.0, st, folder, raw, keywords))?)
}

/// Send a message: stored in the Sent folder, then submitted.
pub fn send(c: &Client, st: &State, sent: Option<&str>, raw: &[u8], from: &str, recipients: &[String]) -> Result<()> {
    Ok(block_on(quatrelle_jmap::send(&c.0, st, sent, raw, from, recipients))?)
}
