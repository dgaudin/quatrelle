// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Redirects and credentials: a redirect is the server's to give, so a request followed to a
//! site it wasn't meant for leaves its token, password and cookies behind (see [`Chain`]).

use quatrelle_model::addr::trust::site;

use super::{Request, Url};

/// The request along a chain of redirects: as given while the chain stays within the site of
/// the URL it was given for (its registrable domain, whatever the host, port or scheme) or the
/// sites the request trusts, and without its credentials from the first hop that leaves them,
/// for good (a redirect back doesn't bring them back). A server a redirect names is not the one
/// the token was meant for, and a redirect is the server's to give. What is compared is the host
/// of the URL the client will connect to, not the text of the `Location` header.
pub(super) struct Chain<'a> {
    req: &'a Request,
    site: String,
    bare: Option<Request>,
}

impl<'a> Chain<'a> {
    pub(super) fn new(req: &'a Request, first: &Url) -> Chain<'a> {
        Chain { req, site: site_of(first), bare: None }
    }

    /// The request to send next.
    pub(super) fn current(&self) -> &Request {
        self.bare.as_ref().unwrap_or(self.req)
    }

    /// A redirect is about to be followed to `next`.
    pub(super) fn follow(&mut self, next: &Url) {
        if self.bare.is_none() && !self.keeps(next) {
            self.bare = Some(self.req.without_credentials());
        }
    }

    /// Whether the credentials may go to `next`: its site is the first one's or a trusted one.
    fn keeps(&self, next: &Url) -> bool {
        let to = site_of(next);
        to == self.site || self.req.trusted.iter().any(|t| t.eq_ignore_ascii_case(&to))
    }
}

/// A URL's site, an IPv6 address in brackets as the account's trusted sites spell it (`Url`
/// keeps its host without them): unbracketed, a trusted `[::1]` would never match.
fn site_of(u: &Url) -> String {
    if u.host.contains(':') { site(&format!("[{}]", u.host)) } else { site(&u.host) }
}

/// Headers that carry a credential.
fn is_credential(name: &str) -> bool {
    ["Authorization", "Proxy-Authorization", "Cookie"].iter().any(|c| name.trim().eq_ignore_ascii_case(c))
}

impl Request {
    /// This request without the headers that carry a credential. (A credential lives only in a
    /// header: `Request` has no field of its own for one.)
    fn without_credentials(&self) -> Request {
        Request { headers: self.headers.iter().filter(|(k, _)| !is_credential(k)).cloned().collect(), ..self.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{bind, credentials_seen, serve, serve_on};
    use super::super::*;
    use super::{is_credential, site};

    fn with_credentials(url: &str) -> Request {
        Request::get(url, 1000).header("Authorization", "Bearer x").header("cookie", "s=1")
    }

    /// The same server under its other name: `127.0.0.1` and `localhost` are two sites.
    fn by_name(url: &str) -> String {
        url.replace("127.0.0.1", "localhost")
    }

    fn ok() -> Vec<u8> {
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec()
    }

    /// A redirect within the site keeps the token and the cookie.
    #[test]
    fn a_redirect_within_the_site_carries_the_credentials() {
        let (a, ha) = serve(vec![b"HTTP/1.1 302 Found\r\nLocation: /two\r\n\r\n".to_vec(), ok()]);
        assert_eq!(send(&with_credentials(&format!("{a}/one"))).unwrap().body, b"ok");
        let seen = ha.join().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen.iter().all(|r| credentials_seen(r) == (true, true)), "{seen:?}");
    }

    /// Another port of the same host is the same site: the credentials go along.
    #[test]
    fn another_port_of_the_same_host_keeps_the_credentials() {
        let (b, hb) = serve(vec![ok()]);
        let (a, ha) = serve(vec![format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {b}/file\r\n\r\n").into_bytes()]);
        assert_eq!(send(&with_credentials(&format!("{a}/one"))).unwrap().body, b"ok");
        assert_eq!(credentials_seen(&ha.join().unwrap()[0]), (true, true));
        let seen = hb.join().unwrap();
        assert!(seen[0].starts_with("GET /file "), "{}", seen[0]);
        assert_eq!(credentials_seen(&seen[0]), (true, true), "{}", seen[0]);
    }

    /// One leg on another site sends neither, and a chain that comes back to the first site
    /// doesn't bring them back.
    #[test]
    fn another_site_drops_them_for_good() {
        let ((la, a), (lb, b)) = (bind(), bind());
        let b = by_name(&b);
        let ha = serve_on(la, vec![format!("HTTP/1.1 302 Found\r\nLocation: {b}/two\r\n\r\n").into_bytes(), ok()]);
        let hb = serve_on(lb, vec![format!("HTTP/1.1 302 Found\r\nLocation: {a}/three\r\n\r\n").into_bytes()]);
        assert_eq!(send(&with_credentials(&format!("{a}/one"))).unwrap().body, b"ok");
        let (seen_a, seen_b) = (ha.join().unwrap(), hb.join().unwrap());
        assert_eq!(credentials_seen(&seen_a[0]), (true, true));
        assert!(seen_b[0].starts_with("GET /two "), "{}", seen_b[0]);
        assert_eq!(credentials_seen(&seen_b[0]), (false, false));
        assert!(seen_a[1].starts_with("GET /three "), "{}", seen_a[1]);
        assert_eq!(credentials_seen(&seen_a[1]), (false, false), "back on the first site, still none: {}", seen_a[1]);
    }

    /// A site the request trusts keeps them.
    #[test]
    fn a_trusted_site_keeps_them() {
        let (b, hb) = serve(vec![ok()]);
        let b = by_name(&b);
        let (a, ha) = serve(vec![format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {b}/file\r\n\r\n").into_bytes()]);
        let req = with_credentials(&format!("{a}/one")).trusting(vec!["localhost".into()]);
        assert_eq!(send(&req).unwrap().body, b"ok");
        assert_eq!(credentials_seen(&ha.join().unwrap()[0]), (true, true));
        let seen = hb.join().unwrap();
        assert_eq!(credentials_seen(&seen[0]), (true, true), "{}", seen[0]);
    }

    /// Not trusting it, the same hop sends nothing: the trusted list is what made the difference.
    #[test]
    fn a_site_not_trusted_gets_nothing() {
        let (b, hb) = serve(vec![ok()]);
        let b = by_name(&b);
        let (a, ha) = serve(vec![format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {b}/file\r\n\r\n").into_bytes()]);
        let req = with_credentials(&format!("{a}/one")).trusting(vec!["example.org".into()]);
        assert_eq!(send(&req).unwrap().body, b"ok");
        ha.join().unwrap();
        assert_eq!(credentials_seen(&hb.join().unwrap()[0]), (false, false));
    }

    #[test]
    fn the_answer_says_where_it_ended() {
        let (a, ha) = serve(vec![b"HTTP/1.1 302 Found\r\nLocation: /two?x=1\r\n\r\n".to_vec(), ok()]);
        let resp = send(&Request::get(&format!("{a}/one"), 1000)).unwrap();
        ha.join().unwrap();
        assert_eq!(resp.url.as_deref(), Some(format!("{a}/two?x=1").as_str()));
        // No redirect: where it was asked.
        let (a, ha) = serve(vec![ok()]);
        let resp = send(&Request::get(&format!("{a}/here"), 1000)).unwrap();
        ha.join().unwrap();
        assert_eq!(resp.url.as_deref(), Some(format!("{a}/here").as_str()));
        // A streamed download says it too.
        let (b, hb) = serve(vec![ok()]);
        let (a, ha) = serve(vec![format!("HTTP/1.1 302 Found\r\nLocation: {b}/file\r\n\r\n").into_bytes()]);
        let head = get_into(&Request::get(&format!("{a}/one"), 1000), |_| Ok(Some(Vec::new()))).unwrap();
        ha.join().unwrap();
        hb.join().unwrap();
        assert_eq!(head.url.as_deref(), Some(format!("{b}/file").as_str()));
    }

    /// The rule on names: the first URL's registrable domain, or a trusted site; trusting one
    /// site doesn't open a third; and once dropped, dropped.
    #[test]
    fn the_chain_keeps_the_credentials_by_site() {
        let u = |s: &str| Url::parse(s).unwrap();
        let first = u("https://mail.example.com/a");
        let credentials = |c: &Chain| c.current().headers.iter().any(|(k, _)| is_credential(k));
        let req = with_credentials("https://mail.example.com/a").trusting(vec!["partner.test".into(), "Other.Test".into()]);
        // Another host of the site, another port, another path.
        let mut c = Chain::new(&req, &first);
        c.follow(&u("https://imap.example.com:8443/b"));
        assert!(credentials(&c));
        // A trusted site (however the list spells it, and with any host in it).
        c.follow(&u("https://sso.partner.test/c"));
        assert!(credentials(&c));
        c.follow(&u("https://x.other.test/c"));
        assert!(credentials(&c));
        // A third site: gone, and not back with the first site, nor with a trusted one.
        c.follow(&u("https://evil.test/d"));
        assert!(!credentials(&c));
        c.follow(&u("https://mail.example.com/e"));
        assert!(!credentials(&c));
        c.follow(&u("https://sso.partner.test/f"));
        assert!(!credentials(&c));
        // Without the list, the trusted site is a third one.
        let plain = with_credentials("https://mail.example.com/a");
        let mut c = Chain::new(&plain, &first);
        c.follow(&u("https://sso.partner.test/c"));
        assert!(!credentials(&c));
        // The headers go whatever their case, and the others stay.
        let r = Request::get("http://h/", 1)
            .header("AUTHORIZATION", "a")
            .header("proxy-authorization", "b")
            .header("Cookie", "c")
            .header("Accept", "text/plain")
            .without_credentials();
        assert_eq!(r.headers, vec![("Accept".to_string(), "text/plain".to_string())]);
    }

    /// The error for a site the account doesn't allow: said with the site, retryable (an answer
    /// from the user can lift it), not a broken connection.
    #[test]
    fn untrusted_says_the_site() {
        let e = crate::NetError::Untrusted { site: "evil.test".into() };
        assert_eq!(e.to_string(), "evil.test isn't allowed for this account");
        assert!(e.is_retryable() && !e.is_fatal());
    }

    /// An IPv6 host is compared as the trusted list spells it, in brackets (as DAV does).
    #[test]
    fn an_ipv6_site_is_trusted_as_written() {
        let u = |s: &str| Url::parse(s).unwrap();
        let credentials = |c: &Chain| c.current().headers.iter().any(|(k, _)| is_credential(k));
        let req = with_credentials("https://mail.example.com/a").trusting(vec!["[2001:db8::1]".into()]);
        let mut c = Chain::new(&req, &u("https://mail.example.com/a"));
        c.follow(&u("https://[2001:db8::1]:8443/b"));
        assert!(credentials(&c));
        c.follow(&u("https://[2001:db8::2]/c"));
        assert!(!credentials(&c));
        // From an IPv6 address to the same one on another port: its own site.
        let req = with_credentials("https://[::1]/a");
        let mut c = Chain::new(&req, &u("https://[::1]/a"));
        c.follow(&u("https://[::1]:8443/b"));
        assert!(credentials(&c));
    }

    /// Two URLs are one site whatever the scheme and port: the registrable domain counts.
    #[test]
    fn sites() {
        let u = |s: &str| Url::parse(s).unwrap();
        let same_site = |a: &Url, b: &Url| site(&a.host) == site(&b.host);
        assert!(same_site(&u("https://Mail.example.com/a"), &u("https://imap.example.com:8443/b")));
        assert!(same_site(&u("http://mail.example.com/"), &u("https://mail.example.com/")));
        assert!(!same_site(&u("https://mail.example.com/"), &u("https://mail.example.org/")));
        assert!(!same_site(&u("http://127.0.0.1/"), &u("http://localhost/")));
    }
}
