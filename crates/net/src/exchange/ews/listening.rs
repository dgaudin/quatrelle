//! Exchange telling of changes as they happen (streaming notifications, Exchange 2010 SP1 and
//! later), as Outlook's Cached Exchange Mode is told: a subscription to a mailbox's folders, and
//! an answer the server keeps open, sending a notification whenever something in them changes.
//! The answer is read on a connection of its own, a slice at a time, so the account's connection
//! stays free for everything else.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use super::*;
use crate::stream::Stream;

/// What the server is asked to tell of.
const EVENT_TYPES: &str = "<t:EventTypes><t:EventType>NewMailEvent</t:EventType><t:EventType>CreatedEvent</t:EventType>\
<t:EventType>DeletedEvent</t:EventType><t:EventType>ModifiedEvent</t:EventType><t:EventType>MovedEvent</t:EventType>\
<t:EventType>CopiedEvent</t:EventType></t:EventTypes>";
/// How long the server keeps one answer open (minutes; Exchange takes 1 to 30), and how long one
/// may stay silent before it's taken for lost (a connection a router dropped without a word).
const STREAM_MINUTES: u64 = 10;
const SILENT_MAX: Duration = Duration::from_secs(STREAM_MINUTES * 60 + 60);
/// An answer the server ends sooner than this wasn't kept open: something in between (a proxy)
/// doesn't let it be, and asking again at once would only be refused again, and again.
#[cfg(not(test))]
const SHORTEST: Duration = Duration::from_secs(20);
#[cfg(test)]
const SHORTEST: Duration = Duration::from_millis(300);
/// Most folders of a shared mailbox one subscription is for, and most told of at once.
const MAX_LISTENED: usize = 300;
const MAX_HEARD: usize = 1000;

/// A subscription to a mailbox's changes, and the answer carrying them.
pub struct Listener {
    /// The shared mailbox told of (its address); `None` for the account's own.
    pub mailbox: Option<String>,
    subscription: String,
    /// The server the subscription lives on, as it named it (`X-BackEndOverrideCookie`): the
    /// answer is asked of the same one.
    cookie: Option<String>,
    body: Option<http::BodyStream<Stream>>,
    /// When the answer was opened, and when something last came on it.
    opened: Instant,
    last: Instant,
}

/// What the server told of: the folders whose content changed, and the folders a change of the
/// folder tree touched (one made, deleted or moved, and its parents) — by their ids; and when
/// the newest mail it told of came, by its clock (seconds since 1970).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Heard {
    pub folders: BTreeSet<String>,
    pub tree: BTreeSet<String>,
    pub new_mail: Option<i64>,
}

impl Heard {
    pub fn is_empty(&self) -> bool {
        self.folders.is_empty() && self.tree.is_empty()
    }
}

impl Client {
    /// Ask to be told of every change in a mailbox: the account's own (`mailbox` `None`: all of
    /// its folders), or a shared one (its folders, by id). Refused by a server that can't tell
    /// (older than Exchange 2010 SP1, or with it turned off).
    pub fn listen(&self, mailbox: Option<&str>, folders: &[String]) -> Result<Listener> {
        let request = match mailbox {
            None => format!("<m:StreamingSubscriptionRequest SubscribeToAllFolders=\"true\">{EVENT_TYPES}</m:StreamingSubscriptionRequest>"),
            Some(_) => {
                let ids: String = folders.iter().take(MAX_LISTENED).map(|id| format!("<t:FolderId Id=\"{}\"/>", esc(id))).collect();
                format!("<m:StreamingSubscriptionRequest><t:FolderIds>{ids}</t:FolderIds>{EVENT_TYPES}</m:StreamingSubscriptionRequest>")
            }
        };
        // Where the subscription lives: the mailbox's own server, which says which it is.
        let anchor = mailbox.unwrap_or(&self.email).to_string();
        let resp = self.post_with(
            envelope(&format!("<m:Subscribe>{request}</m:Subscribe>")).into_bytes(),
            &[("X-AnchorMailbox", &anchor), ("X-PreferServerAffinity", "true")],
        )?;
        let cookie = resp
            .headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("Set-Cookie"))
            .filter_map(|(_, v)| v.split(';').next().map(str::trim))
            .find(|c| c.starts_with("X-BackEndOverrideCookie="))
            .map(str::to_string);
        let doc = Client::answer(&resp).map_err(|b| match b {
            Busy::Wait(_) => NetError::Later("Exchange is busy".into()),
            Busy::Failed(e) => e,
        })?;
        let m = operation(&doc, "SubscribeResponse")?;
        check_message(m)?;
        let subscription = m
            .text_of("SubscriptionId")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| NetError::Protocol("the server gave no subscription".into()))?
            .to_string();
        let now = Instant::now();
        Ok(Listener { mailbox: mailbox.map(str::to_string), subscription, cookie, body: None, opened: now, last: now })
    }

    /// The ids of the account's own calendar, contacts and tasks, each with its distinguished
    /// name: a change told in them is for their sync, not a mail folder's. Those the server won't
    /// name are left out (their changes wait for the next full sync).
    pub fn pim_folders(&self) -> Vec<(&'static str, String)> {
        // Tasks too: one made in Outlook showed here only at the next full sync, a quarter of an
        // hour on (27/10).
        const NAMES: [&str; 3] = ["calendar", "contacts", "tasks"];
        let ids: String = NAMES.iter().map(|d| format!("<t:DistinguishedFolderId Id=\"{d}\"/>")).collect();
        let doc = match self.call(&format!(
            "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds>{ids}</m:FolderIds></m:GetFolder>"
        )) {
            Ok(doc) => doc,
            Err(e) => {
                log::info!("the calendar's, contacts' and tasks' folders weren't found: {e}");
                return Vec::new();
            }
        };
        // Answered in the order asked.
        messages(&doc)
            .iter()
            .zip(NAMES)
            .filter(|(m, _)| m.attr("ResponseClass") != Some("Error"))
            .filter_map(|(m, name)| m.find("FolderId").and_then(|f| f.attr("Id")).map(|id| (name, id.to_string())))
            .collect()
    }

    /// The answer carrying a subscription's notifications, on a connection of its own, signed in
    /// for it.
    fn open_stream(&self, l: &Listener) -> Result<http::BodyStream<Stream>> {
        let body = envelope(&format!(
            "<m:GetStreamingEvents><m:SubscriptionIds><t:SubscriptionId>{}</t:SubscriptionId></m:SubscriptionIds>\
             <m:ConnectionTimeout>{STREAM_MINUTES}</m:ConnectionTimeout></m:GetStreamingEvents>",
            esc(&l.subscription)
        ));
        let anchor = l.mailbox.clone().unwrap_or_else(|| self.email.clone());
        let mut req = Request::post(&self.url, "text/xml; charset=utf-8", body.into_bytes(), MAX_ANSWER)
            .header("Accept", "text/xml")
            .header("X-AnchorMailbox", &anchor);
        if let Some(cookie) = &l.cookie {
            req = req.header("Cookie", cookie);
        }
        if let Some(agent) = &self.user_agent {
            req = req.header("User-Agent", agent);
        }
        let mut conn = Conn::open(&self.url)?;
        let scheme = *self.scheme.borrow();
        let req = match scheme {
            Scheme::Basic => req.header("Authorization", &self.basic()),
            Scheme::Ntlm(name) => {
                let negotiate =
                    Request { body: Vec::new(), ..req.clone() }.header("Authorization", &format!("{name} {}", b64(&crate::ntlm::negotiate())));
                let first = conn.send(&negotiate)?;
                if first.status == 401 {
                    let answer = self.ntlm_answer(&first, name)?;
                    req.header("Authorization", &answer)
                } else {
                    req
                }
            }
            Scheme::Unknown => req,
        };
        let answer = conn.stream(&req)?;
        match answer.status {
            200 => Ok(answer.body),
            401 | 403 => Err(NetError::Auth("the server refused to sign in its notifications".into())),
            s => Err(NetError::Protocol(super::requests::answered_http(s))),
        }
    }
}

impl Listener {
    /// What the server told of, waiting at most `wait` for something to come. The answer is
    /// asked for again when the server ended it (it does every few minutes) or it went silent;
    /// an error when the subscription is gone (it's made again then) or the server won't.
    pub fn heard(&mut self, c: &Client, wait: Duration) -> Result<Heard> {
        if self.body.is_some() && self.last.elapsed() > SILENT_MAX {
            self.body = None;
        }
        if self.body.is_none() {
            self.body = Some(c.open_stream(self)?);
            self.opened = Instant::now();
            self.last = self.opened;
        }
        let mut heard = Heard::default();
        let mut wait = wait;
        loop {
            let Some(body) = self.body.as_mut() else { return Ok(heard) };
            while let Some(doc) = take_envelope(body.text()) {
                match told(&doc, &mut heard) {
                    Ok(false) => {}
                    Ok(true) => return self.ended(heard),
                    Err(e) => {
                        self.body = None;
                        return Err(e);
                    }
                }
            }
            match body.fill(wait) {
                Ok(true) => {
                    self.last = Instant::now();
                    wait = Duration::ZERO;
                }
                Ok(false) => return Ok(heard),
                Err(NetError::Closed(_)) => return self.ended(heard),
                Err(e) => {
                    self.body = None;
                    return Err(e);
                }
            }
        }
    }

    /// The server ended the answer: asked for again at the next look, unless it ended it at once.
    fn ended(&mut self, heard: Heard) -> Result<Heard> {
        self.body = None;
        if self.opened.elapsed() < SHORTEST && heard.is_empty() {
            return Err(NetError::Later("the server ended its notifications as soon as they began".into()));
        }
        Ok(heard)
    }
}

/// The first whole envelope that came, taken from `text`: the answer is envelopes one after the
/// other, cut wherever the connection cut them.
fn take_envelope(text: &mut Vec<u8>) -> Option<El> {
    let mut from = 0;
    loop {
        let at = from + text.get(from..)?.windows(9).position(|w| w == b"Envelope>")?;
        let end = at + 9;
        // Its closing tag: `</Envelope>` or `</prefix:Envelope>`.
        let open = text.get(..at)?.iter().rposition(|b| *b == b'<')?;
        let between = text.get(open + 1..at)?;
        let closing = between.first() == Some(&b'/')
            && (between.len() == 1 || between.last() == Some(&b':'))
            && between.iter().skip(1).all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-' | b'.'));
        if closing {
            let doc = parse(&String::from_utf8_lossy(text.get(..end)?));
            text.drain(..end);
            return Some(doc);
        }
        from = end;
    }
}

/// Read one envelope into `heard`; `true` when it ends the answer (the server closes it).
fn told(doc: &El, heard: &mut Heard) -> Result<bool> {
    if let Some(fault) = doc.find("Fault") {
        return Err(NetError::No(fault.text_of("faultstring").unwrap_or("the server refused its notifications").to_string()));
    }
    let mut closed = false;
    for m in messages(doc) {
        check_message(m)?;
        closed |= m.text_of("ConnectionStatus").is_some_and(|s| s.trim() == "Closed");
        let mut notes = Vec::new();
        m.all("Notification", &mut notes);
        for e in notes.iter().flat_map(|n| n.kids.iter()) {
            if !e.name.ends_with("Event") || e.name == "StatusEvent" {
                continue;
            }
            if e.name == "NewMailEvent"
                && let Some(t) = e.text_of("TimeStamp").and_then(quatrelle_jmap::utc_date)
            {
                heard.new_mail = heard.new_mail.max(Some(t));
            }
            let id = |name: &str| e.child(name).and_then(|f| f.attr("Id")).map(str::to_string);
            match id("FolderId") {
                // A folder's own counts changed.
                Some(folder) if e.name == "ModifiedEvent" => {
                    heard.folders.insert(folder);
                }
                // The tree did: a folder made, deleted, moved or copied.
                Some(folder) => heard.tree.extend([Some(folder), id("ParentFolderId"), id("OldParentFolderId")].into_iter().flatten()),
                // A message: the folder it's in, and the one it left.
                None => heard.folders.extend([id("ParentFolderId"), id("OldParentFolderId")].into_iter().flatten()),
            }
        }
    }
    if heard.folders.len() + heard.tree.len() > MAX_HEARD {
        return Err(NetError::Protocol("told of too many folders at once".into()));
    }
    Ok(closed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope_of(inner: &str) -> String {
        format!(
            "<Envelope xmlns=\"http://schemas.xmlsoap.org/soap/envelope/\"><soap11:Header xmlns:soap11=\"s\"/><soap11:Body xmlns:soap11=\"s\">\
             <m:GetStreamingEventsResponse xmlns:m=\"m\" xmlns:t=\"t\"><m:ResponseMessages><m:GetStreamingEventsResponseMessage ResponseClass=\"Success\">\
             <m:ResponseCode>NoError</m:ResponseCode>{inner}</m:GetStreamingEventsResponseMessage></m:ResponseMessages>\
             </m:GetStreamingEventsResponse></soap11:Body></Envelope>"
        )
    }

    /// The envelopes of an answer are read one by one however the connection cut them, and what
    /// each tells of is the folders that changed — a message's, the one it left, a folder's own
    /// counts — or the tree, and when a new mail came; keep-alives and the server's goodbye tell
    /// of nothing.
    #[test]
    fn notifications_are_read_as_they_come() {
        let new_mail = envelope_of(
            "<m:ConnectionStatus>OK</m:ConnectionStatus><m:Notifications><m:Notification><t:SubscriptionId>S1</t:SubscriptionId>\
             <t:NewMailEvent><t:TimeStamp>2026-10-24T08:00:00Z</t:TimeStamp><t:ItemId Id=\"I1\" ChangeKey=\"k\"/><t:ParentFolderId Id=\"INBOX\" ChangeKey=\"k\"/></t:NewMailEvent>\
             <t:MovedEvent><t:ItemId Id=\"I2\"/><t:ParentFolderId Id=\"FILED\"/><t:OldItemId Id=\"I1\"/><t:OldParentFolderId Id=\"INBOX\"/></t:MovedEvent>\
             <t:ModifiedEvent><t:FolderId Id=\"FILED\"/><t:ParentFolderId Id=\"ROOT\"/><t:UnreadCount>3</t:UnreadCount></t:ModifiedEvent>\
             <t:StatusEvent><t:Watermark>w</t:Watermark></t:StatusEvent></m:Notification></m:Notifications>",
        );
        let made = envelope_of(
            "<m:Notifications><m:Notification><t:CreatedEvent><t:FolderId Id=\"NEW\"/><t:ParentFolderId Id=\"ROOT\"/></t:CreatedEvent></m:Notification></m:Notifications>",
        );
        let alive = envelope_of("<m:ConnectionStatus>OK</m:ConnectionStatus>");
        let goodbye = envelope_of("<m:ConnectionStatus>Closed</m:ConnectionStatus>");
        let all = format!("{alive}\n{new_mail}{made}{goodbye}");
        for cut in [1, 7, 100, all.len()] {
            let mut text = Vec::new();
            let mut heard = Heard::default();
            let mut ends = Vec::new();
            for piece in all.as_bytes().chunks(cut) {
                text.extend_from_slice(piece);
                while let Some(doc) = take_envelope(&mut text) {
                    ends.push(told(&doc, &mut heard).unwrap());
                }
            }
            assert_eq!(ends, [false, false, false, true], "cut every {cut}");
            assert_eq!(heard.folders.iter().map(String::as_str).collect::<Vec<_>>(), ["FILED", "INBOX"]);
            assert_eq!(heard.tree.iter().map(String::as_str).collect::<Vec<_>>(), ["NEW", "ROOT"]);
            assert_eq!(heard.new_mail, quatrelle_jmap::utc_date("2026-10-24T08:00:00Z"), "when the new mail came, by the server's clock");
        }
        // A subscription the server no longer has is an error (it's made again).
        let gone = "<Envelope><Body><m:GetStreamingEventsResponse><m:ResponseMessages><m:GetStreamingEventsResponseMessage ResponseClass=\"Error\">\
                    <m:MessageText>The subscription was not found.</m:MessageText><m:ResponseCode>ErrorSubscriptionNotFound</m:ResponseCode>\
                    </m:GetStreamingEventsResponseMessage></m:ResponseMessages></m:GetStreamingEventsResponse></Body></Envelope>";
        let mut text = gone.as_bytes().to_vec();
        let doc = take_envelope(&mut text).unwrap();
        assert!(told(&doc, &mut Heard::default()).is_err());
        // Half an envelope waits for the rest; an opening tag isn't an end.
        let mut text = b"<s:Envelope><s:Body>".to_vec();
        assert!(take_envelope(&mut text).is_none());
        assert_eq!(text.len(), 20, "nothing taken");
    }
}
