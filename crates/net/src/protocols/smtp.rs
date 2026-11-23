// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
//! Message submission (RFC 6409) through `lettre`: implicit TLS on 465 (preferred, RFC 8314),
//! STARTTLS on 587, or plain only for local test servers.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::Duration;

use lettre::address::{Address, Envelope};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Certificate, Tls, TlsParameters};
use lettre::transport::smtp::extension::ClientId;
use lettre::{SmtpTransport, Transport};
use quatrelle_model::{Security, ServerConfig};
use rustls::pki_types::CertificateDer;

use crate::{NetError, Result};

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
    let tls = |host: &str| tls_parameters(host, crate::stream::system_roots());
    let mut b = SmtpTransport::builder_dangerous(server.host.as_str())
        .port(server.port)
        .timeout(Some(Duration::from_secs(60)))
        .hello_name(hello_for(server.host.as_str(), server.port));
    b = match server.security {
        Security::Tls => b.tls(Tls::Wrapper(tls(&server.host)?)),
        Security::StartTls => b.tls(Tls::Required(tls(&server.host)?)),
        Security::None => b.tls(Tls::None),
    };
    if !user.is_empty() {
        b = b.credentials(Credentials::new(user.to_string(), password.to_string()));
        if oauth {
            b = b.authentication(vec![lettre::transport::smtp::authentication::Mechanism::Xoauth2]);
        }
    }
    let transport = b.build();
    transport.send_raw(&envelope, raw).map_err(|e| {
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
    })?;
    Ok(())
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
