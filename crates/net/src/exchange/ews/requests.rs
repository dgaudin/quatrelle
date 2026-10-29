//! EWS: SOAP requests on a kept connection, signed in with NTLM or Basic, and their answers.

use super::*;

impl Client {
    /// Connect and check the password (asks for the Inbox).
    pub fn connect(url: &str, user: &str, password: &str, email: &str) -> Result<Client> {
        Client::connect_as(url, user, password, email, None)
    }

    /// [`Client::connect`], naming itself `agent` to the server.
    pub fn connect_as(url: &str, user: &str, password: &str, email: &str, agent: Option<&str>) -> Result<Client> {
        let c = Client::new(url, user, password, email)?.with_agent(agent);
        c.call("<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds><t:DistinguishedFolderId Id=\"inbox\"/></m:FolderIds></m:GetFolder>")?;
        Ok(c)
    }

    /// Send on the kept connection, a new one opened when there's none (or it can't be used).
    /// `fresh` false: only on the connection there is (an NTLM answer must go on the connection
    /// that was challenged). `Err((e, again))`: `again` when the request failed on a connection
    /// the server had already closed, before reading it — it can be sent again.
    pub(super) fn send_on_conn(&self, req: &Request, fresh: bool) -> std::result::Result<Response, (NetError, bool)> {
        let mut slot = self.conn.borrow_mut();
        if slot.as_ref().is_none_or(|k| !k.conn.usable()) {
            *slot = None;
            if !fresh {
                return Err((NetError::Closed("the server dropped the connection while signing in".into()), false));
            }
            *slot = Some(Kept { conn: Conn::open(&self.url).map_err(|e| (e, false))?, signed_in: false });
        }
        let Some(kept) = slot.as_mut() else { return Err((NetError::Closed("no connection".into()), false)) };
        match kept.conn.send(req) {
            Ok(r) => Ok(r),
            Err(e) => {
                let again = kept.conn.failed_unanswered();
                *slot = None;
                Err((e, again))
            }
        }
    }

    /// Whether the kept connection is signed in (NTLM) and still usable.
    pub(super) fn signed_in(&self) -> bool {
        self.conn.borrow().as_ref().is_some_and(|k| k.signed_in && k.conn.usable())
    }

    pub(super) fn set_signed_in(&self) {
        if let Some(k) = self.conn.borrow_mut().as_mut() {
            k.signed_in = true;
        }
    }

    /// The `WWW-Authenticate` schemes of a 401.
    pub(super) fn offered(resp: &Response) -> Vec<String> {
        resp.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case("WWW-Authenticate")).map(|(_, v)| v.trim().to_string()).collect()
    }

    /// Sign the connection in with NTLM, sending `req` as its last step: answers the server's
    /// answer to `req`. The first step goes without the body (the server only answers it with
    /// its challenge): a message being sent went up three times.
    pub(super) fn ntlm(&self, req: &Request, name: &'static str) -> std::result::Result<Response, (NetError, bool)> {
        let negotiate = Request { body: Vec::new(), ..req.clone() }.header("Authorization", &format!("{name} {}", b64(&crate::ntlm::negotiate())));
        let first = self.send_on_conn(&negotiate, true)?;
        if first.status != 401 {
            // Taken without a challenge (the server has the connection signed in already): the
            // request itself, on the same connection.
            self.set_signed_in();
            return self.send_on_conn(req, false);
        }
        let answer = self.ntlm_answer(&first, name).map_err(|e| (e, false))?;
        let resp = self.send_on_conn(&req.clone().header("Authorization", &answer), false)?;
        if resp.status != 401 {
            self.set_signed_in();
        }
        Ok(resp)
    }

    /// The `Authorization` answering the NTLM challenge in `first` (the server's 401).
    pub(super) fn ntlm_answer(&self, first: &Response, name: &str) -> Result<String> {
        let token = Client::offered(first)
            .iter()
            .find_map(|v| v.strip_prefix(name).map(|t| t.trim().to_string()))
            .filter(|t| !t.is_empty())
            .ok_or_else(|| NetError::Auth("the server didn't answer the sign-in".into()))?;
        let challenge = crate::ntlm::challenge(&unb64(&token).ok_or_else(|| NetError::Protocol("bad NTLM challenge".into()))?)?;
        let mut client_challenge = [0u8; 8];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut client_challenge)
            .map_err(|_| NetError::Protocol("no randomness".into()))?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let filetime = (now + 11_644_473_600) * 10_000_000;
        let answer = crate::ntlm::authenticate(&challenge, &self.user, &self.password, filetime, client_challenge);
        Ok(format!("{name} {}", b64(&answer)))
    }

    /// The `Authorization` of Basic sign-in.
    pub(super) fn basic(&self) -> String {
        format!("Basic {}", b64(format!("{}:{}", self.user, self.password).as_bytes()))
    }

    /// Send a request signed in the way the server wants.
    pub(crate) fn post(&self, body: Vec<u8>) -> Result<Response> {
        self.post_with(body, &[])
    }

    /// [`Client::post`] with headers of its own (where a subscription is to live).
    pub(super) fn post_with(&self, body: Vec<u8>, extra: &[(&str, &str)]) -> Result<Response> {
        let mut base = Request::post(&self.url, "text/xml; charset=utf-8", body, MAX_ANSWER).header("Accept", "text/xml");
        if let Some(agent) = &self.user_agent {
            base = base.header("User-Agent", agent);
        }
        for (k, v) in extra {
            base = base.header(k, v);
        }
        if let Some(mailbox) = self.route.borrow().as_ref() {
            base = base.header("X-AnchorMailbox", mailbox).header("X-PublicFolderMailbox", mailbox);
        } else if let Some(mailbox) = self.anchor.borrow().as_ref() {
            base = base.header("X-AnchorMailbox", mailbox);
        }
        // A connection kept open that the server had closed (idle too long) fails the request
        // before it's read: it goes again on a new connection, once.
        let mut resent = false;
        // A signed-in connection told 401 is signed in again, once, before it's a refusal.
        let mut signed_again = false;
        let mut attempt = 0;
        while attempt < 3 {
            let scheme = *self.scheme.borrow();
            let was_signed_in = matches!(scheme, Scheme::Ntlm(_)) && self.signed_in();
            let sent = match scheme {
                Scheme::Basic => self.send_on_conn(&base.clone().header("Authorization", &self.basic()), true),
                Scheme::Ntlm(name) if !was_signed_in => self.ntlm(&base, name),
                Scheme::Ntlm(_) | Scheme::Unknown => self.send_on_conn(&base, true),
            };
            let resp = match sent {
                Ok(resp) => resp,
                Err((e, true)) if !resent => {
                    log::info!("ews: a kept connection was closed by the server ({e}); sent again on a new one");
                    resent = true;
                    continue;
                }
                Err((e, _)) => return Err(e),
            };
            attempt += 1;
            if resp.status != 401 {
                return Ok(resp);
            }
            // Not signed in: learn how, once.
            let offered = Client::offered(&resp);
            let starts = |p: &str| offered.iter().any(|v| v.len() >= p.len() && v.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p)));
            let next = if starts("NTLM") {
                Scheme::Ntlm("NTLM")
            } else if starts("Negotiate") {
                Scheme::Ntlm("Negotiate")
            } else if starts("Basic") {
                Scheme::Basic
            } else {
                return Err(NetError::Auth("the server offers no sign-in Quatrelle knows".into()));
            };
            // A connection that was signed in and is told 401 (the server recycled it, or the
            // session expired) isn't a wrong password: sign in once more, on a new connection,
            // before telling the account to ask for it.
            if was_signed_in && next == scheme && !signed_again {
                signed_again = true;
                *self.conn.borrow_mut() = None;
                continue;
            }
            if scheme == next || attempt >= 3 {
                return Err(NetError::Auth("the server refused the user name or password".into()));
            }
            *self.scheme.borrow_mut() = next;
        }
        Err(NetError::Auth("the server refused the user name or password".into()))
    }

    /// Run one operation; the answer's elements (a SOAP fault is an error). Something not found
    /// while the account has public folders is asked for again in the public folder mailboxes
    /// holding their content (Exchange 2013 and later answer only there).
    pub fn call(&self, body: &str) -> Result<El> {
        let doc = self.call_here(body)?;
        let missing = |doc: &El| messages(doc).iter().any(|m| matches!(m.text_of("ResponseCode"), Some("ErrorFolderNotFound" | "ErrorItemNotFound")));
        if self.route.borrow().is_some() || self.anchor.borrow().is_some() || !missing(&doc) {
            return Ok(doc);
        }
        for mailbox in self.shared_open.borrow().clone() {
            if let Ok(there) = self.anchored_to(&mailbox, || self.call_here(body))
                && !missing(&there)
            {
                return Ok(there);
            }
        }
        let guids = self.pf_replicas.borrow().clone();
        for guid in guids {
            let Ok(mailbox) = self.public_folder_mailbox(&guid) else { continue };
            if let Ok(there) = self.routed_to(&mailbox, || self.call_here(body))
                && !missing(&there)
            {
                return Ok(there);
            }
        }
        Ok(doc)
    }

    /// [`Client::call`] where the request is sent, without looking elsewhere. A server that's
    /// busy (`ErrorServerBusy`, HTTP 503) is left alone for as long as it asks, then asked again;
    /// still busy, it's for later.
    pub(super) fn call_here(&self, body: &str) -> Result<El> {
        let mut waited = 0;
        loop {
            match self.call_once(body) {
                Err(Busy::Wait(wait)) if waited < 2 && wait <= MAX_BACK_OFF => {
                    waited += 1;
                    log::info!("ews: the server is busy; asked again in {} s", wait.as_secs());
                    std::thread::sleep(wait);
                }
                Err(Busy::Wait(_)) => return Err(NetError::Later("Exchange is busy and asked to be left alone for a while".into())),
                Err(Busy::Failed(e)) => return Err(e),
                Ok(doc) => return Ok(doc),
            }
        }
    }

    pub(super) fn call_once(&self, body: &str) -> std::result::Result<El, Busy> {
        let asked = std::time::Instant::now();
        let resp = self.post(envelope(body).into_bytes()).map_err(Busy::Failed)?;
        let received = std::time::Instant::now();
        let answer = Client::answer(&resp);
        let head = resp.head_at.unwrap_or(received);
        self.count_spent(Timing {
            requests: 1,
            server: head.saturating_duration_since(asked),
            receiving: received.saturating_duration_since(head),
            reading: received.elapsed(),
            bytes: resp.body.len(),
        });
        answer
    }

    /// What an answer says: its elements, or why not (a SOAP fault, a busy server, HTTP).
    pub(super) fn answer(resp: &Response) -> std::result::Result<El, Busy> {
        let text = String::from_utf8_lossy(&resp.body);
        let doc = parse(&text);
        if let Some(wait) = busy(&doc) {
            return Err(Busy::Wait(wait));
        }
        if let Some(fault) = doc.find("Fault") {
            let why = fault.find("faultstring").map(|f| f.text.clone()).unwrap_or_else(|| "the server refused the request".into());
            // The fault's detail says which refusal it is (`ErrorInvalidRequest`) and, when the
            // server gives one, a message naming the part it didn't like: "The request is invalid."
            // alone left a refused calendar change unexplained.
            let detail = fault.find("detail");
            let code = detail.and_then(|d| d.text_of("ResponseCode")).unwrap_or("").trim().to_string();
            let more = detail.and_then(|d| d.text_of("Message")).unwrap_or("").trim().to_string();
            let transient = TRANSIENT.contains(&code.as_str());
            let said = [code, more].into_iter().filter(|s| !s.is_empty() && *s != why).collect::<Vec<_>>().join(": ");
            let message = if said.is_empty() { why } else { format!("{why} ({said})") };
            // The answer came whole: the connection is fine, and a refusal of one request (a
            // folder it won't list, an item it won't take) isn't a reason to drop it and start
            // the whole sync again — that kept a big mailbox's sync from ever finishing.
            return Err(Busy::Failed(if transient { NetError::Later(message) } else { NetError::No(message) }));
        }
        if resp.status == 503 {
            let after = resp.header("Retry-After").and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(5);
            return Err(Busy::Wait(std::time::Duration::from_secs(after)));
        }
        if !resp.ok() {
            return Err(Busy::Failed(NetError::Protocol(answered_http(resp.status))));
        }
        Ok(doc)
    }

    /// Run an operation on one thing: its single response message, checked.
    /// The account's own address.
    pub(crate) fn email(&self) -> &str {
        &self.email
    }

    pub(crate) fn call_one(&self, body: &str) -> Result<El> {
        let doc = self.call(body)?;
        let m = messages(&doc).first().map(|m| (*m).clone()).ok_or_else(|| NetError::Protocol("an empty answer".into()))?;
        check_message(&m)?;
        Ok(m)
    }

    /// Run an operation on many things: every response message checked.
    pub(crate) fn call_all(&self, body: &str) -> Result<()> {
        let doc = self.call(body)?;
        for m in messages(&doc) {
            check_message(m)?;
        }
        Ok(())
    }
}

/// What an HTTP refusal without a SOAP answer says.
pub(super) fn answered_http(status: u16) -> String {
    format!("Exchange answered HTTP {status}")
}
