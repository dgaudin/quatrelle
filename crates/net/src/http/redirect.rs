//! Redirects and credentials: a redirect is the server's to give, so a request followed to
//! another origin leaves its token, password and cookies behind (see [`Chain`]).

use super::{Request, Url};

/// The request along a chain of redirects: as given while the chain stays on the origin it was
/// given for, and without its credentials from the first hop that leaves it, for good (a
/// redirect back doesn't bring them back). A server a redirect names is not the one the token
/// was meant for, and a redirect is the server's to give.
pub(super) struct Chain<'a> {
    req: &'a Request,
    first: Url,
    bare: Option<Request>,
}

impl<'a> Chain<'a> {
    pub(super) fn new(req: &'a Request, first: &Url) -> Chain<'a> {
        Chain { req, first: first.clone(), bare: None }
    }

    /// The request to send next.
    pub(super) fn current(&self) -> &Request {
        self.bare.as_ref().unwrap_or(self.req)
    }

    /// A redirect is about to be followed to `next`.
    pub(super) fn follow(&mut self, next: &Url) {
        if self.bare.is_none() && !same_origin(&self.first, next) {
            self.bare = Some(self.req.without_credentials());
        }
    }
}

/// Whether two URLs are one origin: scheme, host and port (the host is already lower-cased).
pub(crate) fn same_origin(a: &Url, b: &Url) -> bool {
    a.https == b.https && a.host.eq_ignore_ascii_case(&b.host) && a.port == b.port
}

/// Headers that carry a credential.
pub(crate) fn is_credential(name: &str) -> bool {
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
    use super::same_origin;

    fn with_credentials(url: &str) -> Request {
        Request::get(url, 1000).header("Authorization", "Bearer x").header("cookie", "s=1")
    }

    /// A redirect within the origin keeps the token and the cookie; one to another site (here
    /// another port) sends neither, and a chain that comes back to the first site doesn't
    /// bring them back.
    #[test]
    fn a_redirect_to_another_origin_carries_no_credentials() {
        let ok = || b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec();
        // Same origin: both legs carry both.
        let (a, ha) = serve(vec![b"HTTP/1.1 302 Found\r\nLocation: /two\r\n\r\n".to_vec(), ok()]);
        assert_eq!(send(&with_credentials(&format!("{a}/one"))).unwrap().body, b"ok");
        let seen = ha.join().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen.iter().all(|r| credentials_seen(r) == (true, true)), "{seen:?}");
        // Another port: the first leg carries them, the second neither.
        let (b, hb) = serve(vec![ok()]);
        let (a, ha) = serve(vec![format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {b}/file\r\n\r\n").into_bytes()]);
        assert_eq!(send(&with_credentials(&format!("{a}/one"))).unwrap().body, b"ok");
        assert_eq!(credentials_seen(&ha.join().unwrap()[0]), (true, true));
        let seen = hb.join().unwrap();
        assert!(seen[0].starts_with("GET /file "), "{}", seen[0]);
        assert_eq!(credentials_seen(&seen[0]), (false, false), "{}", seen[0]);
        // A -> B -> A: only the first leg.
        let ((la, a), (lb, b)) = (bind(), bind());
        let ha = serve_on(la, vec![format!("HTTP/1.1 302 Found\r\nLocation: {b}/two\r\n\r\n").into_bytes(), ok()]);
        let hb = serve_on(lb, vec![format!("HTTP/1.1 302 Found\r\nLocation: {a}/three\r\n\r\n").into_bytes()]);
        assert_eq!(send(&with_credentials(&format!("{a}/one"))).unwrap().body, b"ok");
        let (seen_a, seen_b) = (ha.join().unwrap(), hb.join().unwrap());
        assert_eq!(credentials_seen(&seen_a[0]), (true, true));
        assert_eq!(credentials_seen(&seen_b[0]), (false, false));
        assert!(seen_a[1].starts_with("GET /three "), "{}", seen_a[1]);
        assert_eq!(credentials_seen(&seen_a[1]), (false, false), "back on the first site, still none: {}", seen_a[1]);
        // The headers go whatever their case, and the others stay.
        let r = Request::get("http://h/", 1)
            .header("AUTHORIZATION", "a")
            .header("proxy-authorization", "b")
            .header("Cookie", "c")
            .header("Accept", "text/plain")
            .without_credentials();
        assert_eq!(r.headers, vec![("Accept".to_string(), "text/plain".to_string())]);
    }

    #[test]
    fn origins() {
        let u = |s: &str| Url::parse(s).unwrap();
        assert!(same_origin(&u("https://Mail.example/a"), &u("https://mail.example:443/b")));
        assert!(!same_origin(&u("https://mail.example/"), &u("https://other.example/")));
        assert!(!same_origin(&u("https://mail.example/"), &u("https://mail.example:8443/")));
        assert!(!same_origin(&u("http://mail.example/"), &u("https://mail.example/")));
        assert!(!same_origin(&u("http://mail.example/"), &u("http://mail.example:81/")));
    }
}
