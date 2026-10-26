//! The fake EWS server on localhost: connections kept open, NTLM or Basic sign-in, and each request routed to the data.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

use super::*;

pub struct Server {
    pub url: String,
    pub data: Arc<Mutex<Data>>,
}

pub(super) fn respond(s: &mut impl Write, status: u16, headers: &[(&str, String)], body: &[u8]) {
    let mut head = format!("HTTP/1.1 {status} X\r\nContent-Type: text/xml; charset=utf-8\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

/// Count one down; whether there was one.
fn take_one(n: &mut usize) -> bool {
    if *n == 0 {
        return false;
    }
    *n -= 1;
    true
}

/// Start the server; it runs until the test process ends.
pub fn serve(data: Data) -> Server {
    serve_at(data, "127.0.0.1:0")
}

/// Start the server on an address of its own.
pub(super) fn serve_at(data: Data, at: &str) -> Server {
    let l = TcpListener::bind(at).unwrap();
    let url = format!("http://127.0.0.1:{}/EWS/Exchange.asmx", l.local_addr().unwrap().port());
    let mut data = data;
    data.url = url.clone();
    let data = Arc::new(Mutex::new(data));
    let shared = data.clone();
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(s) = conn else { continue };
            let shared = shared.clone();
            std::thread::spawn(move || serve_connection(s, &shared));
        }
    });
    Server { url, data }
}

/// Requests on one kept-open connection: NTLM signs it in once.
fn serve_connection(mut s: std::net::TcpStream, shared: &Arc<Mutex<Data>>) {
    let mut reader = BufReader::new(s.try_clone().unwrap());
    let mut signed_in = false;
    let mut challenge: Option<crate::ntlm::Challenge> = None;
    let mut answered = 0usize;
    loop {
        let mut first = String::new();
        if reader.read_line(&mut first).unwrap_or(0) == 0 {
            return;
        }
        if answered > 0 && take_one(&mut shared.lock().unwrap().drop_kept) {
            let _ = s.shutdown(std::net::Shutdown::Both);
            return;
        }
        answered += 1;
        let (mut len, mut auth, mut routed, mut anchor, mut cookie) = (0usize, String::new(), String::new(), String::new(), String::new());
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                if k.eq_ignore_ascii_case("content-length") {
                    len = v.trim().parse().unwrap_or(0);
                }
                if k.eq_ignore_ascii_case("authorization") {
                    auth = v.trim().to_string();
                }
                if k.eq_ignore_ascii_case("x-publicfoldermailbox") {
                    routed = v.trim().to_string();
                }
                if k.eq_ignore_ascii_case("x-anchormailbox") {
                    anchor = v.trim().to_string();
                }
                if k.eq_ignore_ascii_case("cookie") {
                    cookie = v.trim().to_string();
                }
            }
        }
        let mut body = vec![0u8; len];
        let _ = reader.read_exact(&mut body);
        let basic = shared.lock().unwrap().basic;
        if basic {
            let expected = format!("Basic {}", crate::imap::base64(format!("{DOMAIN}\\{USER}:{PASSWORD}").as_bytes()));
            if auth != expected {
                respond(&mut s, 401, &[("WWW-Authenticate", "Basic realm=\"x\"".into())], b"");
                continue;
            }
        } else {
            // A connection the server no longer has signed in (it was recycled, or the session
            // expired): the request is refused once, as Exchange does.
            if signed_in && std::mem::take(&mut shared.lock().unwrap().forget_sign_in) {
                signed_in = false;
            }
            if !signed_in {
                match auth.strip_prefix("NTLM ").and_then(unb64) {
                    Some(m) if m.get(8) == Some(&1) => {
                        let c = crate::ntlm::make_challenge([9; 8], &[]);
                        challenge = crate::ntlm::challenge(&c).ok();
                        respond(&mut s, 401, &[("WWW-Authenticate", format!("NTLM {}", crate::imap::base64(&c)))], b"");
                        continue;
                    }
                    Some(m) if m.get(8) == Some(&3) => {
                        let ok = challenge.as_ref().is_some_and(|c| crate::ntlm::verify(c, &m, USER, DOMAIN, PASSWORD));
                        if !ok {
                            respond(&mut s, 401, &[("WWW-Authenticate", "NTLM".into()), ("WWW-Authenticate", "Negotiate".into())], b"");
                            continue;
                        }
                        signed_in = true;
                        shared.lock().unwrap().handshakes += 1;
                    }
                    _ => {
                        respond(&mut s, 401, &[("WWW-Authenticate", "Negotiate".into()), ("WWW-Authenticate", "NTLM".into())], b"");
                        continue;
                    }
                }
            }
        }
        if take_one(&mut shared.lock().unwrap().busy) {
            let fault = "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><s:Fault>\
                <faultcode>a:ErrorServerBusy</faultcode><faultstring>The server cannot service this request right now. Try again later.</faultstring>\
                <detail><e:ResponseCode xmlns:e=\"http://schemas.microsoft.com/exchange/services/2006/errors\">ErrorServerBusy</e:ResponseCode>\
                <e:Message xmlns:e=\"http://schemas.microsoft.com/exchange/services/2006/errors\">The server cannot service this request right now. Try again later.</e:Message>\
                <t:MessageXml xmlns:t=\"http://schemas.microsoft.com/exchange/services/2006/types\"><t:Value Name=\"BackOffMilliseconds\">10</t:Value></t:MessageXml>\
                </detail></s:Fault></s:Body></s:Envelope>";
            respond(&mut s, 500, &[], fault.as_bytes());
            continue;
        }
        let doc = parse(&String::from_utf8_lossy(&body));
        // Autodiscover: this server's own address.
        if doc.find("Autodiscover").is_some_and(|a| a.find("EMailAddress").is_some()) {
            let (url, pf, hierarchy) = {
                let d = shared.lock().unwrap();
                (d.url.clone(), d.pf_mailbox.clone(), d.pf_hierarchy.clone())
            };
            // The public folder mailbox, asked for by its GUID: its address.
            let asked = doc.find("EMailAddress").map(|e| e.text.clone()).unwrap_or_default();
            if let Some(pf) = pf.filter(|_| asked.starts_with(PF_GUID)) {
                let answer = format!(
                    "<?xml version=\"1.0\"?><Autodiscover><Response><User><DisplayName>Public folders</DisplayName>\
                     <AutoDiscoverSMTPAddress>{pf}</AutoDiscoverSMTPAddress></User><Account><AccountType>email</AccountType><Action>settings</Action>\
                     <Protocol><Type>EXCH</Type><EwsUrl>{url}</EwsUrl></Protocol></Account></Response></Autodiscover>"
                );
                respond(&mut s, 200, &[], answer.as_bytes());
                continue;
            }
            // The account's: its server, and its public folder hierarchy mailbox.
            let public =
                hierarchy.map(|h| format!("<PublicFolderInformation><SmtpAddress>{h}</SmtpAddress></PublicFolderInformation>")).unwrap_or_default();
            // Mailboxes mapped to the account (full access, automapped), as Outlook is told.
            let mapped: String = shared
                .lock()
                .unwrap()
                .shared
                .iter()
                .filter(|b| b.automap)
                .map(|b| {
                    let id = if b.legacy {
                        format!("<LegacyDN>{}</LegacyDN><Server>mbx.contoso.example</Server>", legacy_dn(&b.address))
                    } else {
                        format!("<SmtpAddress>{}</SmtpAddress><OwnerSmtpAddress>{}</OwnerSmtpAddress>", b.address, b.address)
                    };
                    format!("<AlternativeMailbox><Type>Delegate</Type><DisplayName>{}</DisplayName>{id}</AlternativeMailbox>", esc(&b.name))
                })
                .collect();
            let public = format!(
                "{public}{mapped}<AlternativeMailbox><Type>Archive</Type><DisplayName>Online Archive</DisplayName><SmtpAddress>archive@contoso.example</SmtpAddress></AlternativeMailbox>"
            );
            let answer = format!(
                "<?xml version=\"1.0\"?><Autodiscover><Response><User><DisplayName>Riley</DisplayName></User><Account><AccountType>email</AccountType><Action>settings</Action>\
                 <Protocol><Type>EXCH</Type><EwsUrl>{url}</EwsUrl></Protocol>{public}</Account></Response></Autodiscover>"
            );
            respond(&mut s, 200, &[], answer.as_bytes());
            continue;
        }
        let Some(op) = doc.find("Body").and_then(|b| b.kids.first()).cloned() else {
            respond(
                &mut s,
                500,
                &[],
                b"<soap:Envelope><soap:Body><soap:Fault><faultstring>no operation</faultstring></soap:Fault></soap:Body></soap:Envelope>",
            );
            continue;
        };
        // Changes told as they happen: the subscription (the server it lives on said in a
        // cookie), and the answer kept open that carries them, the connection's last.
        if op.name == "Subscribe" {
            let message = {
                let mut d = shared.lock().unwrap();
                d.calls.push(op.name.clone());
                d.subscribe(&op)
            };
            let xml = format!(
                "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><m:SubscribeResponse xmlns:m=\"m\" xmlns:t=\"t\">\
                 <m:ResponseMessages>{}</m:ResponseMessages></m:SubscribeResponse></s:Body></s:Envelope>",
                message.replace("<m:R ", "<m:SubscribeResponseMessage ").replace("</m:R>", "</m:SubscribeResponseMessage>")
            );
            respond(&mut s, 200, &[("Set-Cookie", format!("{}; path=/", super::streaming::BACKEND))], xml.as_bytes());
            continue;
        }
        if op.name == "GetStreamingEvents" {
            super::streaming::stream_events(&mut s, shared, &op, &cookie);
            return;
        }
        // Slow answers: the fork holds every mail search (`slow_sync`), the shared version only
        // a mail folder's listing and not a PIM folder's (`hold`); whole messages and `GetFolder` too.
        let hold = {
            let d = shared.lock().unwrap();
            d.hold
                .filter(|_| op.name == "FindItem" && !d.pim_folder(op.child("ParentFolderIds").and_then(|p| p.kids.first())))
                .or_else(|| d.slow_sync.filter(|_| op.name == "FindItem"))
                .or_else(|| d.slow_sources.filter(|_| op.name == "GetItem" && op.find("IncludeMimeContent").is_some_and(|m| m.text == "true")))
                .or_else(|| d.slow_counts.filter(|_| op.name == "GetFolder"))
        };
        if let Some(h) = hold {
            std::thread::sleep(h);
        }
        // A mailbox without a picture, as Exchange 2016 tells of it: HTTP 404, nothing else.
        if op.name == "GetUserPhoto" && op.text_of("Email").is_some_and(|e| e.ends_with("@http404.example")) {
            respond(&mut s, 404, &[], b"");
            continue;
        }
        let mut d = shared.lock().unwrap();
        d.calls.push(op.name.clone());
        d.routed = routed;
        if !anchor.is_empty() && d.anchors.len() < 1000 {
            d.anchors.push(anchor);
        }
        let (name, parts) = d.route(&op);
        drop(d);
        // Operations Exchange answers in the response element itself, not in a list of
        // response messages: automatic replies, room lists and rooms, free/busy.
        let plain = name.starts_with("GetUserOof")
            || name.starts_with("SetUserOof")
            || matches!(name.as_str(), "GetRoomListsResponse" | "GetRoomsResponse" | "GetUserAvailabilityResponse");
        let inner = if plain {
            parts.concat()
        } else {
            let tag = name.trim_end_matches("Response").to_string() + "ResponseMessage";
            format!(
                "<m:ResponseMessages>{}</m:ResponseMessages>",
                parts.concat().replace("<m:R ", &format!("<m:{tag} ")).replace("</m:R>", &format!("</m:{tag}>"))
            )
        };
        let xml = format!(
            "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><m:{name} xmlns:m=\"m\" xmlns:t=\"t\">{inner}</m:{name}></s:Body></s:Envelope>"
        );
        respond(&mut s, 200, &[], xml.as_bytes());
    }
}
