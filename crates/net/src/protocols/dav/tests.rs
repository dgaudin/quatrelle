use super::*;

#[test]
fn hrefs_are_made_absolute() {
    assert_eq!(absolute("https://h.example/dav/cal/", "/dav/cal/1.ics").unwrap(), "https://h.example/dav/cal/1.ics");
}

/// Whether a request as the server saw it carried the password.
fn password_seen(request: &str) -> bool {
    request.to_ascii_lowercase().contains("authorization: basic dtpw")
}

/// A redirect within the server keeps the password; one to another site (`localhost` here,
/// the same server under another site's name) carries none, and a chain that comes back to the first site doesn't bring it back.
#[test]
fn a_redirect_to_another_site_never_carries_the_password() {
    use crate::http::tests::{bind, serve, serve_on};
    let dav = Dav::new("u", "p");
    let multi = || b"HTTP/1.1 207 Multi-Status\r\nContent-Length: 0\r\n\r\n".to_vec();
    for (method, depth) in [("PROPFIND", "0"), ("REPORT", "1")] {
        let go = |url: &str| dav.request(method, url, Some(depth), "<x/>", &[]).unwrap().0.status;
        // Same server: both legs carry it.
        let (a, ha) = serve(vec![b"HTTP/1.1 301 Moved\r\nLocation: /dav/\r\n\r\n".to_vec(), multi()]);
        assert_eq!(go(&format!("{a}/")), 207);
        let seen = ha.join().unwrap();
        assert!(seen.len() == 2 && seen.iter().all(|r| password_seen(r)), "{method}: {seen:?}");
        assert!(seen[1].starts_with(&format!("{method} /dav/ ")), "{}", seen[1]);
        // Another site: the first leg carries it, the second doesn't.
        let (b, hb) = serve(vec![multi()]);
        let b = b.replace("127.0.0.1", "localhost");
        let (a, ha) = serve(vec![format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {b}/dav/\r\n\r\n").into_bytes()]);
        assert_eq!(go(&format!("{a}/")), 207);
        assert!(password_seen(&ha.join().unwrap()[0]));
        let seen = hb.join().unwrap();
        assert!(seen[0].starts_with(&format!("{method} /dav/ ")) && !password_seen(&seen[0]), "{method}: {}", seen[0]);
        // A -> B -> A: the first leg only.
        let ((la, a), (lb, b)) = (bind(), bind());
        let b = b.replace("127.0.0.1", "localhost");
        let ha = serve_on(la, vec![format!("HTTP/1.1 302 Found\r\nLocation: {b}/two\r\n\r\n").into_bytes(), multi()]);
        let hb = serve_on(lb, vec![format!("HTTP/1.1 302 Found\r\nLocation: {a}/three\r\n\r\n").into_bytes()]);
        assert_eq!(go(&format!("{a}/one")), 207);
        let (seen_a, seen_b) = (ha.join().unwrap(), hb.join().unwrap());
        assert!(password_seen(&seen_a[0]) && !password_seen(&seen_b[0]), "{method}");
        assert!(seen_a[1].starts_with(&format!("{method} /three ")) && !password_seen(&seen_a[1]), "back on the first site: {}", seen_a[1]);
    }
}

/// A PUT is never redirected (the redirect is the answer, nothing is sent again): the
/// password stays with the server it was given for.
#[test]
fn a_put_is_not_sent_on_after_a_redirect() {
    use crate::http::tests::serve;
    let (b, hb) = serve(Vec::new());
    let (a, ha) = serve(vec![format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {b}/x.ics\r\nContent-Length: 0\r\n\r\n").into_bytes()]);
    let r = Dav::new("u", "p").put(&format!("{a}/x.ics"), "text/calendar", "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n", None);
    assert!(r.is_err(), "{r:?}");
    let seen = ha.join().unwrap();
    assert!(seen.len() == 1 && seen[0].starts_with("PUT /x.ics ") && password_seen(&seen[0]), "{seen:?}");
    assert!(hb.join().unwrap().is_empty(), "nothing reached the other server");
}
