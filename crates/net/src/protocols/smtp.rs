// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Message submission (RFC 6409) through `lettre`: implicit TLS on 465 (preferred, RFC 8314),
//! STARTTLS on 587, or plain only for local test servers. MAIL, RCPT, DATA and the message go a
//! step at a time ([`hand_over`]), so it's known how far a send got.

use std::cell::Cell;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::Duration;

use lettre::address::{Address, Envelope};
use lettre::transport::smtp::authentication::{Credentials, DEFAULT_MECHANISMS, Mechanism};
use lettre::transport::smtp::client::{Certificate, SmtpConnection, TlsParameters};
use lettre::transport::smtp::commands::{Data, Mail, Rcpt};
use lettre::transport::smtp::extension::{ClientId, Extension, MailBodyParameter, MailParameter};
use lettre::transport::smtp::response::Severity;
use quatrelle_model::{Security, ServerConfig};
use rustls::pki_types::CertificateDer;

use crate::{NetError, Result};

/// How long the server may take to answer.
#[cfg(not(test))]
const TIMEOUT: Duration = Duration::from_secs(60);

/// How long it may take over a message it's been sent: it may check the whole of it before it
/// answers (RFC 5321 §4.5.3.2.6: ten minutes, since giving up early is how mail arrives twice).
#[cfg(not(test))]
const SENT_TIMEOUT: Duration = Duration::from_secs(600);

/// The same in the tests, short enough to wait out, the second still the longer.
#[cfg(test)]
const TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(test)]
const SENT_TIMEOUT: Duration = Duration::from_secs(8);

/// The message goes to the server in pieces of this many bytes: once the last has gone, so has
/// the line that ends it, and only then may the server have taken it.
const PIECE: usize = 64 * 1024;

/// Submit `raw` (an RFC 5322 message) from `from` to `recipients`.
pub fn send(server: &ServerConfig, user: &str, password: &str, from: &str, recipients: &[String], raw: &[u8]) -> Result<()> {
    send_with(server, user, password, false, from, recipients, raw)
}

/// [`send`], signing in with an OAuth access token (XOAUTH2) when `oauth`.
pub fn send_with(server: &ServerConfig, user: &str, password: &str, oauth: bool, from: &str, recipients: &[String], raw: &[u8]) -> Result<()> {
    let from: Address = from.trim().parse().map_err(|e| NetError::Smtp(format!("sender {from}: {e}")))?;
    let mut to = Vec::with_capacity(recipients.len());
    for r in recipients {
        to.push(r.trim().parse::<Address>().map_err(|e| NetError::Smtp(format!("recipient {r}: {e}")))?);
    }
    if to.is_empty() {
        return Err(NetError::Smtp("no recipients".into()));
    }
    let envelope = Envelope::new(Some(from), to).map_err(|e| NetError::Smtp(e.to_string()))?;
    submit(server, user, password, oauth, &envelope, raw, recipients)
}

/// One message over one connection, as lettre's `SmtpTransport` sent it: connect and EHLO (each
/// of the server's addresses tried in turn), STARTTLS, AUTH, then [`hand_over`] and QUIT. Every
/// failure on the way is read by [`failure`], the sign-in's too: a refused one stays a refused
/// sign-in (renewed for OAuth, explained for Microsoft), never a refused message.
fn submit(server: &ServerConfig, user: &str, password: &str, oauth: bool, envelope: &Envelope, raw: &[u8], recipients: &[String]) -> Result<()> {
    let failed = |e: lettre::transport::smtp::Error| failure(e, recipients);
    let host = server.host.as_str();
    // Made once, for the handshake as the connection opens (465) or after STARTTLS (587).
    let tls = match server.security {
        Security::Tls | Security::StartTls => Some(tls_parameters(host, crate::stream::system_roots())?),
        Security::None => None,
    };
    let hello = hello_for(host, server.port);
    let wrapper = tls.as_ref().filter(|_| server.security == Security::Tls);
    let mut conn = SmtpConnection::connect((host, server.port), Some(TIMEOUT), &hello, wrapper, None).map_err(failed)?;
    if let Some(tls) = tls.as_ref().filter(|_| server.security == Security::StartTls) {
        conn.starttls(tls, &hello).map_err(failed)?;
    }
    if !user.is_empty() {
        let mechanisms: &[Mechanism] = if oauth { &[Mechanism::Xoauth2] } else { DEFAULT_MECHANISMS };
        conn.auth(mechanisms, &Credentials::new(user.to_string(), password.to_string())).map_err(failed)?;
    }
    let sent = hand_over(&mut conn, envelope, raw, recipients);
    if let Err(NetError::Protocol(_)) = sent {
        // Out of step with the server, which may be reading the message: let go of without a
        // QUIT, which it could take for a line of it.
        drop(conn);
    } else {
        conn.abort();
    }
    sent
}

/// MAIL, RCPT, DATA and the message, as lettre's `SmtpConnection::send` goes through them, one
/// at a time so it's known how far it got:
/// - till the line ending the message has gone, the server can't have taken it: a failure is
///   its refusal ([`failure`]) or no answer (`Io`), and the message can go again;
/// - DATA answered "go on" (3xx) is the only way on: any other positive answer is one to
///   another command, the connection out of step, and nothing of the message goes (`Protocol`:
///   [`submit`] lets the connection go without a QUIT);
/// - once the whole message has gone, only the server's answer says what became of it: taken
///   (2xx) is sent, a refusal is said, and anything else (no answer, the connection dropped, a
///   positive answer out of turn) is `Unconfirmed`: it may be on its way, and sending it again
///   unasked is how it arrives twice.
fn hand_over(conn: &mut SmtpConnection, envelope: &Envelope, raw: &[u8], recipients: &[String]) -> Result<()> {
    let failed = |e: lettre::transport::smtp::Error| failure(e, recipients);
    let lacks = |what: &str| NetError::Io(std::io::Error::other(format!("the server does not support {what}")));
    let mut options = Vec::new();
    let ascii = |a: &Address| AsRef::<str>::as_ref(a).is_ascii();
    if !envelope.from().is_none_or(ascii) || !envelope.to().iter().all(ascii) {
        if !conn.server_info().supports_feature(Extension::SmtpUtfEight) {
            return Err(lacks("addresses outside ASCII (SMTPUTF8)"));
        }
        options.push(MailParameter::SmtpUtfEight);
    }
    if !raw.is_ascii() {
        if !conn.server_info().supports_feature(Extension::EightBitMime) {
            return Err(lacks("text outside ASCII (8BITMIME)"));
        }
        options.push(MailParameter::Body(MailBodyParameter::EightBitMime));
    }
    conn.command(Mail::new(envelope.from().cloned(), options)).map_err(failed)?;
    for to in envelope.to() {
        conn.command(Rcpt::new(to.clone(), Vec::new())).map_err(failed)?;
    }
    let go_on = conn.command(Data).map_err(failed)?;
    if go_on.code().severity != Severity::PositiveIntermediate {
        return Err(NetError::Protocol(format!("the server answered {} to DATA", go_on.code())));
    }
    conn.set_timeout(Some(SENT_TIMEOUT)).map_err(NetError::Io)?;
    // Set once every piece has gone to the connection: the line ending the message goes next.
    let whole = Cell::new(false);
    let pieces = raw.chunks(PIECE).chain(std::iter::from_fn(|| {
        whole.set(true);
        None
    }));
    let answer = conn.message_iter(pieces);
    // The QUIT that follows waits a minute at most, whatever became of the message.
    if let Err(e) = conn.set_timeout(Some(TIMEOUT)) {
        log::info!("SMTP: the wait for the goodbye couldn't be shortened: {e}");
    }
    match answer {
        Ok(taken) if taken.code().severity == Severity::PositiveCompletion => Ok(()),
        // A positive answer of another class is one to another command: never "sent".
        Ok(other) => Err(NetError::Unconfirmed(format!("an answer out of turn ({})", other.code()))),
        // An answer, whatever it says, is the server's word on the message; cut before its end,
        // the server can't have it (it takes a message at its end), and it can go again (`Io`).
        Err(e) if e.status().is_some() || !whole.get() => Err(failed(e)),
        Err(e) => Err(NetError::Unconfirmed(e.to_string())),
    }
}

/// What a failure from lettre means for the message, wherever it came on the way (connecting,
/// STARTTLS, AUTH, MAIL, RCPT, DATA, the message's answer): refused (5xx, [`refusal`]: the
/// sign-in or the message), to try again later (4xx), or no answer at all.
fn failure(e: lettre::transport::smtp::Error, recipients: &[String]) -> NetError {
    let msg = e.to_string();
    if e.is_permanent() {
        let code = e.status().and_then(|c| c.to_string().parse::<u16>().ok()).unwrap_or(550);
        refusal(code, &msg, recipients)
    } else if e.is_transient() {
        // 4xx: the server asks to try again later.
        NetError::Later(msg)
    } else {
        // No answer at all: no network, refused connection, TLS failure or a timeout.
        NetError::Io(std::io::Error::other(msg))
    }
}

/// What Quatrelle calls itself in its EHLO to `host` (RFC 5321 section 4.1.4): the address literal
/// of this computer's address on the way there, since a desktop has no name the world knows
/// (never the machine's name, which may say more than the server needs). lettre's own is
/// `[127.0.0.1]`, and some providers' spam filters drop mail greeted so. The address is the one
/// the system would send from: a socket "connected" without sending anything says it. No way to
/// the server: lettre's default (the connection fails anyway). Adapted from a6f00b41.
fn hello_for(host: &str, port: u16) -> ClientId {
    let Some(server) = (host, port).to_socket_addrs().ok().and_then(|mut a| a.next()) else {
        return ClientId::default();
    };
    let any: SocketAddr = if server.is_ipv4() { (Ipv4Addr::UNSPECIFIED, 0).into() } else { (Ipv6Addr::UNSPECIFIED, 0).into() };
    let local = UdpSocket::bind(any).and_then(|s| {
        s.connect(server)?;
        s.local_addr()
    });
    match local.map(|a| a.ip()) {
        Ok(IpAddr::V4(ip)) if !ip.is_unspecified() => ClientId::Ipv4(ip),
        Ok(IpAddr::V6(ip)) if !ip.is_unspecified() => ip.to_ipv4_mapped().map_or(ClientId::Ipv6(ip), ClientId::Ipv4),
        _ => ClientId::default(),
    }
}

/// TLS for `host`: lettre's bundled Mozilla roots and `roots` (the system's, which IMAP and
/// HTTPS trust too). lettre fails the whole build on a root it can't read, so `roots` holds
/// only usable ones, the system's distrusted ones already left out
/// ([`crate::stream::system_roots`]). lettre takes no verifier of ours, so unlike IMAP it can't
/// also refuse a server or intermediate certificate the system distrusts (`docs/gaps.md`).
fn tls_parameters(host: &str, roots: &[CertificateDer<'static>]) -> Result<TlsParameters> {
    let mut p = TlsParameters::builder(host.to_string());
    for der in roots {
        if let Ok(c) = Certificate::from_der(der.to_vec()) {
            p = p.add_root_certificate(c);
        }
    }
    p.build_rustls().map_err(|e| NetError::Tls(e.to_string()))
}

/// What a 5xx reply means, by its code (RFC 4954 §6, RFC 3463), never by words in its text
/// ("553 5.7.1 sender not owned by authenticated user" is a refusal of the message):
/// - 530 / 534 / 535 / 538, or the enhanced codes 5.7.8 (bad credentials) and 5.7.9 (mechanism
///   too weak): the sign-in was refused. Not the message's fault: it waits for a new sign-in.
/// - anything else (550/551/552/553/554 on MAIL, RCPT or DATA): this message is refused for good,
///   naming the recipients the server's text names.
fn refusal(code: u16, msg: &str, recipients: &[String]) -> NetError {
    // Enhanced status codes stand alone in the text, maybe wrapped in `( )` or followed by `:`.
    let says = |status: &str| msg.split(|c: char| c.is_whitespace() || ":()".contains(c)).any(|token| token == status);
    let sign_in_code = matches!(code, 530 | 534 | 535 | 538);
    if sign_in_code || says("5.7.8") || says("5.7.9") {
        return NetError::Auth(msg.to_string());
    }
    let lower = msg.to_ascii_lowercase();
    let named: Vec<String> =
        recipients.iter().map(|r| r.trim().to_ascii_lowercase()).filter(|r| !r.is_empty() && lower.contains(r.as_str())).collect();
    if named.is_empty() { NetError::Smtp(msg.to_string()) } else { NetError::Smtp(format!("{} refused: {msg}", named.join(", "))) }
}

#[cfg(test)]
pub(crate) mod tests;
