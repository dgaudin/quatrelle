// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The client's calls for files and people: the hosts a transfer may go to, OneDrive's upload
//! sessions and ranges (never with the token), a file's item, its share, a post with pictures and
//! files, a shared file's address, the directory searched and a conversation created; each
//! checked before any request, and the fake answering as Graph would.

use std::net::TcpStream;

use quatrelle_mime::sanitize_filename;
use quatrelle_model::pim::teams::{Attachment, PICTURE_INLINE_MAX, Person, is_bidi, is_object_id, one_line, person_ok};
use serde_json::{Value, json};

use super::*;
use crate::teams::graph::{
    CHUNK, FileRef, Hosted, RangeAnswer, Recipient, Shared, UploadSession, cancel_upload_on, drive_name, etag_guid, host_tail, next_expected_of,
    person_of, put_range_on, search_term, transfer_url, upload_status_on,
};

/// The signed-in user and the people of the directory, by object ids.
const MORGAN: &str = "00000000-0000-0000-0000-0000000000a0";
const ANA: &str = "00000000-0000-0000-0000-0000000000a1";
const BRUNO: &str = "00000000-0000-0000-0000-0000000000a2";
const CARLA: &str = "00000000-0000-0000-0000-0000000000a3";
const GRAPH: &str = "https://graph.microsoft.com/v1.0";

fn fake<T>(server: &Server, f: impl FnOnce(&mut FakeTeams) -> T) -> T {
    f(server.data.lock().unwrap().teams.as_mut().unwrap())
}

/// A connection to an upload session's server, as a lane opens one.
fn tcp(url: &crate::http::Url) -> TcpStream {
    TcpStream::connect((url.host.as_str(), url.port)).unwrap()
}

fn put(s: &UploadSession, bytes: &[u8], from: u64, total: u64) -> crate::Result<RangeAnswer> {
    put_range_on(&mut tcp(s.url()), s.url(), bytes, from, total)
}

/// A file sent up in one range: its item's id.
fn upload(c: &TeamsClient, name: &str, bytes: &[u8]) -> String {
    let s = c.upload_session(name).unwrap();
    match put(&s, bytes, 0, bytes.len() as u64).unwrap() {
        RangeAnswer::Done(id) => id,
        other => panic!("{other:?}"),
    }
}

/// The user (by an object id), a chat, and a directory of Ana, Bruno and Carla (a guest).
fn people_teams() -> FakeTeams {
    let mut t = FakeTeams::new(FakeUser::new(MORGAN, "Morgan Lee"), TENANT);
    let mut carla = FakeUser::new(CARLA, "Carla Dias");
    carla.mail = "carla@fabrikam.example".into();
    t.directory = vec![FakeUser::new(ANA, "Ana Lima"), FakeUser::new(BRUNO, "Bruno Reis"), carla];
    t.guests = vec![CARLA.into()];
    t
}

fn person(id: &str, name: &str, guest: bool) -> Person {
    let first = name.split_whitespace().next().unwrap().to_lowercase();
    Person { id: id.into(), name: name.into(), address: format!("{first}@contoso.example"), guest }
}

#[test]
fn transfer_hosts_are_microsofts_only() {
    let taken = transfer_url(GRAPH, "https://contoso-my.sharepoint.com/personal/x/_layouts/15/download.aspx?UniqueId=1").unwrap();
    assert_eq!((taken.host.as_str(), taken.port, taken.https), ("contoso-my.sharepoint.com", 443, true));
    let long = format!("https://contoso-my.sharepoint.com/{}", "a".repeat(9 << 10));
    for refused in [
        "https://evilsharepoint.com/x",
        "https://sharepoint.com.evil.example/x",
        "https://sharepoint.com/x",
        "https://.sharepoint.com/x",
        "https://a..sharepoint.com/x",
        "http://contoso-my.sharepoint.com/x",
        "https://contoso-my.sharepoint.com:8443/x",
        "https://user@contoso-my.sharepoint.com/x",
        "https://10.0.0.5/x",
        "https://[::1]/x",
        "http://127.0.0.1:8080/x",
        "https://contoso-my.sharepoint.com/x y",
        "https://contoso-my.sharepoint.com/x\u{7}",
        "https://contoso_my.sharepoint.com/x",
        "junk",
        "",
        &long,
    ] {
        assert!(matches!(transfer_url(GRAPH, refused), Err(NetError::Refused(m)) if m == "not one of Microsoft's file hosts"), "{refused:?}");
    }
    // The tests' servers, on this computer, may hand out their own plain address; still not elsewhere.
    let local = "http://127.0.0.1:9000/v1.0";
    assert!(transfer_url(local, "http://127.0.0.1:9000/upload/s").is_ok());
    assert!(matches!(transfer_url(local, "http://192.168.1.2/x"), Err(NetError::Refused(_))));
}

#[test]
fn a_host_is_logged_by_its_last_two_labels() {
    assert_eq!(host_tail("contoso-my.sharepoint.com"), "…sharepoint.com");
    assert_eq!(host_tail("sharepoint.com"), "…sharepoint.com");
    for odd in ["x", "a b.example", "127.0.0.1", "::1", "", "a..b", "évil.example"] {
        assert_eq!(host_tail(odd), "…", "{odd:?}");
    }
}

#[test]
fn a_session_is_asked_under_teams_folder_with_a_safe_name() {
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 1)]));
    let c = client(&server);
    let s = c.upload_session("report 2026.pdf").unwrap();
    let (path, body) = fake(&server, |t| (t.drive_files.sessions[0].path.clone(), t.drive_files.sessions[0].body.clone()));
    assert_eq!(path, "/me/drive/root:/Microsoft%20Teams%20Chat%20Files/report%202026.pdf:/createUploadSession");
    assert_eq!(body, json!({"item": {"@microsoft.graph.conflictBehavior": "rename"}}));
    assert!(headers(&server, "uploadSession").iter().any(|(k, v)| k == "authorization" && v == &format!("Bearer {TOKEN}")));
    // Its address is never written out: its host's last two labels only (none for an address).
    let shown = format!("{s:?}");
    assert!(!shown.contains("127.0.0.1") && !shown.contains("upload"), "{shown}");
    // A name that isn't its own drive name is refused before any request: the caller names it.
    assert!(matches!(c.upload_session("../x"), Err(NetError::Refused(_))));
    assert!(matches!(c.upload_session("a#b.txt"), Err(NetError::Refused(_))));
    assert_eq!(hits(&server, "uploadSession"), 1);
    // A full OneDrive: its status and code.
    fake(&server, |t| t.drive_files.full = true);
    assert!(matches!(c.upload_session("more.pdf"), Err(NetError::Closed(m)) if m == "HTTP 507 quotaLimitReached"));

    assert_eq!(drive_name("..\\a#b%.txt"), "a_b_.txt");
    assert_eq!(drive_name("~$draft.docx"), "draft.docx");
    assert_eq!(drive_name("Desktop.ini"), "_Desktop.ini");
    assert_eq!(drive_name("a_vti_b.txt"), "a-vti_b.txt");
    assert_eq!(drive_name("CON.txt"), "_CON.txt");
    assert_eq!(drive_name("report 2026.pdf"), "report 2026.pdf");
    let long = drive_name(&format!("{}.pdf", "a".repeat(300)));
    assert_eq!(long.chars().count(), 180);
    assert!(long.ends_with(".pdf"), "{long}");
}

proptest::proptest! {
    /// A drive name is its own drive name and its own safe name, whatever it was made from.
    #[test]
    fn a_drive_name_is_its_own(name in "[a-zA-Z0-9 .~$#%_/\\\\:é\u{202E}\u{7}-]{0,260}") {
        let d = drive_name(&name);
        proptest::prop_assert_eq!(&sanitize_filename(&d), &d);
        proptest::prop_assert_eq!(&drive_name(&d), &d);
        proptest::prop_assert!(d.chars().count() <= 180 && !d.contains(['#', '%']) && !d.starts_with("~$") && !d.contains("_vti_"));
    }

    #[test]
    fn any_name_gives_a_drive_name_of_its_own(name in "\\PC{0,300}") {
        let d = drive_name(&name);
        proptest::prop_assert_eq!(&sanitize_filename(&d), &d);
        proptest::prop_assert_eq!(&drive_name(&d), &d);
    }
}

#[test]
fn an_upload_address_elsewhere_is_refused() {
    for elsewhere in ["https://evil.example/up", "https://contoso-my.sharepoint.com.evil.example/up", "http://192.168.1.2/up"] {
        let mut t = teams_with(vec![chat(CHAT, T0, 1)]);
        t.drive_files.upload_elsewhere = Some(elsewhere.into());
        let server = serve_teams(t);
        assert!(
            matches!(client(&server).upload_session("report.pdf"), Err(NetError::Refused(m)) if m == "not one of Microsoft's file hosts"),
            "{elsewhere}"
        );
    }
}

#[test]
fn ranges_go_in_order_without_the_token() {
    assert_eq!(CHUNK, 16 * 327_680);
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 1)]));
    let c = client(&server);
    let s = c.upload_session("report.pdf").unwrap();
    let body: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
    assert_eq!(put(&s, &body[..655_360], 0, 700_000).unwrap(), RangeAnswer::Next(655_360));
    assert_eq!(upload_status_on(&mut tcp(s.url()), s.url(), 700_000).unwrap(), 655_360);
    // A range already there, or one that isn't a multiple of 320 KiB, is refused by OneDrive.
    assert!(matches!(put(&s, &body[..327_680], 0, 700_000), Err(NetError::Protocol(m)) if m == "HTTP 416"));
    assert!(matches!(put(&s, &body[655_360..655_370], 655_360, 700_000), Err(NetError::Protocol(m)) if m.starts_with("HTTP 400")));
    let RangeAnswer::Done(id) = put(&s, &body[655_360..], 655_360, 700_000).unwrap() else { panic!("not done") };
    let (sid, ranges, files) = fake(&server, |t| (t.drive_files.sessions[0].id.clone(), t.drive_files.ranges.clone(), t.drive_files.files.clone()));
    assert_eq!(ranges, [(sid.clone(), 0, 655_359, 700_000), (sid, 655_360, 699_999, 700_000)]);
    assert_eq!((files[0].id.as_str(), files[0].name.as_str(), files[0].bytes == body), (id.as_str(), "report.pdf", true));
    assert!(!headers(&server, "upload").iter().any(|(k, _)| k == "authorization"), "{:?}", headers(&server, "upload"));
    // Finished, the session is gone.
    assert!(matches!(upload_status_on(&mut tcp(s.url()), s.url(), 700_000), Err(NetError::Refused(m)) if m == "the upload session is gone"));
    // A name taken gets OneDrive's number.
    let again = upload(&c, "report.pdf", b"another");
    assert_eq!(fake(&server, |t| t.drive_files.files.iter().find(|f| f.id == again).unwrap().name.clone()), "report 1.pdf");
    // A session cancelled is gone too.
    let s = c.upload_session("gone.pdf").unwrap();
    cancel_upload_on(&mut tcp(s.url()), s.url()).unwrap();
    assert!(matches!(put(&s, b"x", 0, 1), Err(NetError::Refused(_))));
    // The token is refused there, as OneDrive says it might be.
    let mut req = crate::http::Request::get(&format!("{}{}", s.url().origin(), s.url().path), 1 << 10).header("Authorization", "Bearer t");
    req.method = "DELETE".into();
    assert_eq!(crate::http::send(&req).unwrap().status, 401);
    // What isn't a range is refused before anything is written.
    for (bytes, from, total) in [(&b""[..], 0, 1), (&b"ab"[..], 0, 1), (&b"ab"[..], 5, 6)] {
        assert!(matches!(put(&s, bytes, from, total), Err(NetError::Refused(_))), "{from} {total}");
    }

    // An answer must fit the range it answers: the next byte past it and before the end, or the
    // file made once the last range is in.
    let accepted = |next: &str| answer(202, &format!(r#"{{"nextExpectedRanges": ["{next}"]}}"#));
    let made = |id: &str| answer(201, &format!(r#"{{"id": "{id}"}}"#));
    let (base, h) = crate::http::tests::serve(vec![
        accepted("0-"),
        accepted("15-"),
        accepted("20-"),
        accepted("10-"),
        made("item-1"),
        made("a/b"),
        made("item-1"),
    ]);
    let url = crate::http::Url::parse(&format!("{base}/upload/s")).unwrap();
    let put = |bytes: &[u8], total: u64| put_range_on(&mut tcp(&url), &url, bytes, 0, total);
    assert!(put(&[1; 10], 20).is_err(), "backwards");
    assert_eq!(put(&[1; 10], 20).unwrap(), RangeAnswer::Next(15), "later is taken");
    assert!(put(&[1; 10], 20).is_err(), "nothing left to send");
    assert!(matches!(put(&[1; 10], 10), Err(NetError::Protocol(m)) if m == "Graph: an upload not finished after its last range"));
    assert!(matches!(put(&[1; 10], 20), Err(NetError::Protocol(m)) if m == "Graph: an upload finished early"));
    assert!(matches!(put(&[1; 10], 10), Err(NetError::Protocol(_))), "an id that isn't one");
    assert_eq!(put(&[1; 10], 10).unwrap(), RangeAnswer::Done("item-1".into()));
    let seen = h.join().unwrap();
    assert!(seen[0].starts_with("PUT /upload/s HTTP/1.1\r\n") && seen[0].contains("\r\nContent-Range: bytes 0-9/20\r\n"), "{}", seen[0]);
    assert!(seen.iter().all(|r| !r.to_ascii_lowercase().contains("authorization")));
    assert_eq!(next_expected_of(&json!({"nextExpectedRanges": ["655360-699999", "700000-"]}), 700_000), Some(655_360));
    for odd in [json!({"nextExpectedRanges": ["700001-"]}), json!({"nextExpectedRanges": []}), json!({"nextExpectedRanges": ["x-"]}), json!({})] {
        assert_eq!(next_expected_of(&odd, 700_000), None, "{odd}");
    }
}

#[test]
fn an_etag_gives_its_guid_or_nothing() {
    assert_eq!(etag_guid("\"{5B8C7D0E-0000-0000-0000-00000000000A},2\"").as_deref(), Some("5b8c7d0e-0000-0000-0000-00000000000a"));
    assert_eq!(etag_guid("5b8c7d0e-0000-0000-0000-00000000000a").as_deref(), Some("5b8c7d0e-0000-0000-0000-00000000000a"));
    for odd in [
        "",
        "{x},1",
        "\"{5B8C7D0E-0000-0000-0000-00000000000},2\"",
        "\"{5B8C7D0E1-0000-0000-0000-00000000000A},2\"",
        "{5B8C7D0E-0000-0000-0000-00000000000AB}",
    ] {
        assert_eq!(etag_guid(odd), None, "{odd:?}");
    }
}

#[test]
fn an_item_gives_what_a_message_needs() {
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 1)]));
    let c = client(&server);
    let id = upload(&c, "report.pdf", &[7; 1000]);
    let item = c.drive_item(&id).unwrap();
    let etag = fake(&server, |t| t.drive_files.files[0].etag.clone());
    assert_eq!((item.id.as_str(), item.name.as_str(), item.size), (id.as_str(), "report.pdf", 1000));
    assert_eq!(Some(item.guid.clone()), etag_guid(&etag));
    assert!(is_object_id(&item.guid));
    assert!(
        item.web_dav_url.starts_with("https://") && item.web_dav_url.ends_with("/Microsoft%20Teams%20Chat%20Files/report.pdf"),
        "{}",
        item.web_dav_url
    );
    fake(&server, |t| t.drive_files.no_guid = true);
    assert!(matches!(c.drive_item(&id), Err(NetError::Protocol(m)) if m == "Graph: a file's item without what a message needs"));
    let before = hits(&server, "item");
    for odd in ["a/b", "", "..", "a?b"] {
        assert!(matches!(c.drive_item(odd), Err(NetError::Refused(_))), "{odd:?}");
    }
    assert_eq!(hits(&server, "item"), before);
    // The clean-up of a check: the item goes.
    c.delete_item(&id).unwrap();
    assert!(fake(&server, |t| t.drive_files.files.is_empty()));
    assert!(matches!(c.delete_item("a/b"), Err(NetError::Refused(_))));
}

#[test]
fn a_share_is_read_only_signed_in_and_silent() {
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 1)]));
    let c = client(&server);
    let id = upload(&c, "report.pdf", b"a report");
    let to = [Recipient::Id(ANA), Recipient::Address("guest@fabrikam.example")];
    assert_eq!(c.invite(&id, &to).unwrap(), Shared { granted: 2, failed: 0 });
    let invites = fake(&server, |t| t.drive_files.invites.clone());
    assert_eq!(invites[0].0, id);
    assert_eq!(
        invites[0].1,
        json!({"recipients": [{"objectId": ANA}, {"email": "guest@fabrikam.example"}], "roles": ["read"], "requireSignIn": true, "sendInvitation": false})
    );
    fake(&server, |t| t.drive_files.fail_share = vec!["guest@fabrikam.example".into()]);
    assert_eq!(c.invite(&id, &to).unwrap(), Shared { granted: 1, failed: 1 });
    fake(&server, |t| t.drive_files.forbid_share = true);
    assert!(matches!(c.invite(&id, &to), Err(NetError::Protocol(m)) if m == "HTTP 403 Forbidden"));
    // Who isn't one is refused before any request.
    let many: Vec<Recipient<'_>> = (0..101).map(|_| Recipient::Id(ANA)).collect();
    for bad in [&[Recipient::Id("u/x")][..], &[Recipient::Address("a b@x.example")], &[], &many[..]] {
        assert!(matches!(c.invite(&id, bad), Err(NetError::Refused(_))));
    }
    assert!(matches!(c.invite("a/b", &to), Err(NetError::Refused(_))));
    assert_eq!(hits(&server, "invite"), 3);
}

#[test]
fn a_partial_share_counts_what_it_doesnt_say_was_granted_as_failed() {
    // A 207 says some recipients failed: those its answer doesn't list as granted are counted so.
    let partial = |value: &str| answer(207, &format!(r#"{{"value": {value}}}"#));
    let (base, h) = crate::http::tests::serve(vec![
        answer(200, ME_JSON),
        partial("[]"),
        partial(r#"[{"id": "p1"}]"#),
        partial(r#"[{"id": "p1"}, {"error": {"code": "notAllowed"}}]"#),
        partial(r#"[{"id": "p1"}, {"id": "p2"}, {"id": "p3"}]"#),
        partial(r#"[1, "x", {"error": null}]"#),
    ]);
    let (c, _) = TeamsClient::connect(&base, "t").unwrap();
    let to = [Recipient::Id(ANA), Recipient::Address("guest@fabrikam.example")];
    let shared: Vec<Shared> = (0..5).map(|_| c.invite("item-1", &to).unwrap()).collect();
    let said = |granted, failed| Shared { granted, failed };
    assert_eq!(shared, [said(0, 2), said(1, 1), said(1, 1), said(2, 0), said(1, 1)]);
    assert_eq!(h.join().unwrap().len(), 6);
}

#[test]
fn a_post_carries_its_pictures_and_files() {
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 1)]));
    let c = client(&server);
    let item = c.drive_item(&upload(&c, "report.pdf", b"%PDF a report")).unwrap();
    let png: &[u8] = b"\x89PNG\r\n\x1a\n a picture";
    let picture = Hosted { bytes: png, mime: "image/png" };
    let file = FileRef { guid: &item.guid, url: &item.web_dav_url, name: &item.name };
    let html = format!("<p>Here</p><p><img src=\"../hostedContents/1/$value\"></p><attachment id=\"{}\"></attachment>", item.guid);
    let made = c.send_parts(CHAT, &html, &[], &[picture], &[file]).unwrap();
    let posts = fake(&server, |t| t.posts.clone());
    assert_eq!(
        posts[0].1["hostedContents"],
        json!([{"@microsoft.graph.temporaryId": "1", "contentBytes": crate::imap::base64(png), "contentType": "image/png"}])
    );
    assert_eq!(
        posts[0].1["attachments"],
        json!([{"id": item.guid, "contentType": "reference", "contentUrl": item.web_dav_url, "name": "report.pdf"}])
    );
    let m = message_of(CHAT, &made);
    assert!(m.attachments.iter().any(|a| matches!(a, Attachment::File { name, .. } if name == "report.pdf")), "{:?}", m.attachments);
    // Without parts the post is as before.
    c.send_html(CHAT, "<p>hi</p>", &[]).unwrap();
    let plain = fake(&server, |t| t.posts[1].1.clone());
    assert!(plain.get("hostedContents").is_none() && plain.get("attachments").is_none(), "{plain}");

    // What Graph would refuse, or Teams couldn't show, is refused before any request.
    let before = hits(&server, "post");
    let big = vec![0u8; usize::try_from(PICTURE_INLINE_MAX + 1).unwrap()];
    let svg = Hosted { bytes: b"<svg/>", mime: "image/svg+xml" };
    let pictures: Vec<Hosted<'_>> = (0..6).map(|_| Hosted { bytes: png, mime: "image/png" }).collect();
    let files: Vec<FileRef<'_>> = (0..5).map(|_| FileRef { guid: &item.guid, url: &item.web_dav_url, name: &item.name }).collect();
    let refused = [
        c.send_parts(CHAT, &html, &[], &[Hosted { bytes: &big, mime: "image/png" }], &[]),
        c.send_parts(CHAT, &html, &[], &[Hosted { bytes: b"", mime: "image/png" }], &[]),
        c.send_parts(CHAT, &html, &[], &[svg], &[]),
        c.send_parts(CHAT, &html, &[], &pictures, &files),
        c.send_parts(CHAT, &html, &[], &[], &[FileRef { guid: "x", url: &item.web_dav_url, name: "report.pdf" }]),
        c.send_parts(CHAT, &html, &[], &[], &[FileRef { guid: &item.guid, url: "http://contoso-my.sharepoint.com/x", name: "report.pdf" }]),
        c.send_parts(CHAT, &html, &[], &[], &[FileRef { guid: &item.guid, url: &item.web_dav_url, name: "" }]),
        c.send_parts("19:a/b", &html, &[], &[], &[]),
    ];
    for (i, r) in refused.into_iter().enumerate() {
        assert!(matches!(r, Err(NetError::Refused(_))), "{i}: {r:?}");
    }
    assert_eq!(hits(&server, "post"), before);
    // A body without the file's place, or a picture's: Graph refuses it.
    let no_file = "<p>Here</p><p><img src=\"../hostedContents/1/$value\"></p>";
    assert!(matches!(c.send_parts(CHAT, no_file, &[], &[picture], &[file]), Err(NetError::Protocol(m)) if m.starts_with("HTTP 400")));
    assert!(matches!(c.send_parts(CHAT, &html, &[], &[], &[file]), Err(NetError::Protocol(m)) if m.starts_with("HTTP 400")));
    // A file that isn't in the user's OneDrive is refused as well.
    let foreign = FileRef { guid: &item.guid, url: "https://contoso-my.sharepoint.com/other.pdf", name: "report.pdf" };
    assert!(matches!(c.send_parts(CHAT, &html, &[], &[picture], &[foreign]), Err(NetError::Protocol(m)) if m.starts_with("HTTP 400")));
}

/// A file shared in a chat, by the URL its attachment carries.
const SHARED: &str = "https://contoso.sharepoint.com/sites/team/Shared%20Documents/plan.pdf";

#[test]
fn a_shared_files_address_comes_without_its_bytes() {
    let mut t = teams_with(vec![chat(CHAT, T0, 1)]);
    t.shares.insert(SHARED.into(), vec![7; 100_000]);
    let server = serve_teams(t);
    let c = client(&server);
    let to = c.shared_location(SHARED).unwrap();
    assert!(to.path.starts_with("/v1.0/download/u!") && to.host == "127.0.0.1", "{to:?}");
    assert_eq!((hits(&server, "shares"), hits(&server, "download")), (1, 0));
    assert!(headers(&server, "shares").iter().any(|(k, v)| k == "authorization" && v == &format!("Bearer {TOKEN}")));
    assert!(matches!(c.shared_location("https://contoso.sharepoint.com/nothing"), Err(NetError::Protocol(m)) if m.starts_with("HTTP 404")));
    assert!(matches!(c.shared_location(""), Err(NetError::Refused(_))));

    // Served by Graph itself, it has no address; a redirect elsewhere than Microsoft's hosts is refused.
    let to = |loc: &str| format!("HTTP/1.1 302 Found\r\nLocation: {loc}\r\nContent-Length: 0\r\n\r\n").into_bytes();
    let (base, h) = crate::http::tests::serve(vec![
        answer(200, ME_JSON),
        answer(200, "{}"),
        to("https://evil.example/x"),
        to("https://contoso-my.sharepoint.com.evil.example/x"),
        to("https://evil.example/x"),
    ]);
    let (c, _) = TeamsClient::connect(&base, "t").unwrap();
    assert!(matches!(c.shared_location(SHARED), Err(NetError::Protocol(m)) if m == "Graph: a shared file without its address"));
    for _ in 0..2 {
        assert!(matches!(c.shared_location(SHARED), Err(NetError::Refused(m)) if m == "not one of Microsoft's file hosts"));
    }
    // A download follows the same rule, before any connection to the other host.
    assert!(matches!(c.download_shared(SHARED, 1 << 20), Err(NetError::Refused(m)) if m == "not one of Microsoft's file hosts"));
    assert_eq!(h.join().unwrap().len(), 5);
}

#[test]
fn a_redirect_that_isnt_an_address_is_refused_without_saying_it() {
    // A download's address carries its own authorisation: an error never quotes it.
    let to = |loc: &str| format!("HTTP/1.1 302 Found\r\nLocation: {loc}\r\nContent-Length: 0\r\n\r\n").into_bytes();
    let odd = [
        "https://user@contoso-my.sharepoint.com/x?tempauth=SECRET",
        "https://contoso-my .sharepoint.com/x?tempauth=SECRET",
        "https://contoso-my.sharepoint.com:99999/x?tempauth=SECRET",
        "https://contoso-my.sharepoint.com\t/x?tempauth=SECRET",
    ];
    let mut answers = vec![answer(200, ME_JSON)];
    answers.extend(odd.iter().map(|l| to(l)));
    answers.push(to(odd[0]));
    let (base, h) = crate::http::tests::serve(answers);
    let (c, _) = TeamsClient::connect(&base, "t").unwrap();
    let mut errors: Vec<NetError> = odd.iter().map(|_| c.shared_location(SHARED).unwrap_err()).collect();
    errors.push(c.download_shared(SHARED, 1 << 20).unwrap_err());
    for e in errors {
        let said = e.to_string();
        assert!(matches!(&e, NetError::Refused(m) if m == "not one of Microsoft's file hosts"), "{said}");
        assert!(!said.contains("contoso") && !said.contains("SECRET"), "{said}");
    }
    assert_eq!(h.join().unwrap().len(), 6);
}

#[test]
fn a_search_is_one_quoted_term() {
    let server = serve_teams(people_teams());
    let c = client(&server);
    c.users_search("ana\" OR \"mail:eve\\").unwrap();
    assert_eq!(fake(&server, |t| t.last_search.clone()), "\"displayName:ana OR mail:eve\" OR \"mail:ana OR mail:eve\"");
    assert!(headers(&server, "users").contains(&("consistencylevel".into(), "eventual".into())), "{:?}", headers(&server, "users"));
    let found = c.users_search("ana").unwrap();
    assert_eq!(found, [person(ANA, "Ana Lima", false)]);
    assert_eq!(search_term("  a\u{7}b\u{202E}  c "), "ab c");
    assert_eq!(search_term("a\tb\\c\"d"), "a bcd");
    assert_eq!(search_term(&"a".repeat(80)).chars().count(), 64);
    assert_eq!(search_term(&format!("{} b", "a".repeat(63))), "a".repeat(63));
    let before = hits(&server, "users");
    assert!(c.users_search("  \"\\ ").unwrap().is_empty());
    assert_eq!(hits(&server, "users"), before);
}

#[test]
fn people_come_clean_and_few() {
    let mut t = people_teams();
    let mut bidi = FakeUser::new("00000000-0000-0000-0000-000000000b01", "x");
    bidi.display_name = format!("\u{202E}Contoso {}", "x".repeat(250));
    let mut no_mail = FakeUser::new("00000000-0000-0000-0000-000000000b02", "Contoso Nomail");
    no_mail.mail = String::new();
    no_mail.upn = "nomail@contoso.example".into();
    let guest = FakeUser::new("00000000-0000-0000-0000-000000000b03", "Contoso Guest");
    t.guests.push(guest.id.clone());
    let mut list = vec![FakeUser::new("a/b", "Contoso Slash"), bidi, no_mail, guest];
    list.extend((0..36).map(|i| FakeUser::new(&format!("00000000-0000-0000-0000-000000000c{i:02}"), &format!("Contoso Person {i}"))));
    t.directory = list;
    let server = serve_teams(t);
    let found = client(&server).users_search("contoso").unwrap();
    assert!(found.len() <= 25 && found.len() >= 20, "{}", found.len());
    assert!(found.iter().all(person_ok), "{found:?}");
    assert!(!found.iter().any(|p| p.id == "a/b"));
    let bidi = found.iter().find(|p| p.id.ends_with("b01")).unwrap();
    assert!(bidi.name.chars().count() <= 80 && !bidi.name.chars().any(is_bidi) && bidi.name == one_line(&bidi.name), "{:?}", bidi.name);
    assert_eq!(found.iter().find(|p| p.id.ends_with("b02")).unwrap().address, "nomail@contoso.example");
    assert!(found.iter().find(|p| p.id.ends_with("b03")).unwrap().guest);
    assert_eq!(found.iter().filter(|p| p.guest).count(), 1);

    // A directory that gives more than asked is cut at 25; the request asks as Graph wants.
    let people: Vec<Value> = (0..40)
        .map(|i| json!({"id": format!("00000000-0000-0000-0000-0000000001{i:02}"), "displayName": format!("P {i}"), "mail": null, "userPrincipalName": format!("p{i}@contoso.example")}))
        .collect();
    let (base, h) = crate::http::tests::serve(vec![answer(200, ME_JSON), answer(200, &json!({ "value": people }).to_string())]);
    let (c, _) = TeamsClient::connect(&base, "t").unwrap();
    let found = c.users_search("p").unwrap();
    assert_eq!(found.len(), 25);
    assert_eq!(found[0].address, "p0@contoso.example");
    let seen = h.join().unwrap();
    assert!(
        seen[1].starts_with(
            "GET /users?$search=%22displayName%3Ap%22%20OR%20%22mail%3Ap%22&$count=true&$top=25&$select=id,displayName,mail,userPrincipalName,userType HTTP/1.1\r\n"
        ),
        "{}",
        seen[1]
    );
    assert!(seen[1].contains("\r\nConsistencyLevel: eventual\r\n"), "{}", seen[1]);

    // One read alone: an id that is a GUID, a name on one line, a guest by its type.
    let p = person_of(&json!({"id": ANA, "displayName": " Ana\n Lima ", "mail": " ana@contoso.example ", "userType": "guest"})).unwrap();
    assert_eq!(p, Person { id: ANA.into(), name: "Ana Lima".into(), address: "ana@contoso.example".into(), guest: true });
    assert_eq!(person_of(&json!({"id": ANA, "mail": "ana@contoso.example"})).unwrap().name, "ana@contoso.example");
    for odd in
        [json!({"id": "x')/../me", "displayName": "Eve"}), json!({"id": ANA}), json!({"id": ANA, "displayName": "Eve", "mail": "e v@x"}), json!("x")]
    {
        assert_eq!(person_of(&odd), None, "{odd}");
    }
}

#[test]
fn a_chat_is_created_as_graph_documents_it() {
    let server = serve_teams(people_teams());
    let c = client(&server);
    let base = server.base.clone();
    let member = |id: &str, role: &str| json!({"@odata.type": "#microsoft.graph.aadUserConversationMember", "roles": [role], "user@odata.bind": format!("{base}/users('{id}')")});
    let ana = person(ANA, "Ana Lima", false);
    let made = c.create_chat(MORGAN, std::slice::from_ref(&ana), None).unwrap();
    assert_eq!(made["chatType"], "oneOnOne");
    let creations = fake(&server, |t| t.creations.clone());
    assert_eq!(creations[0], json!({"chatType": "oneOnOne", "members": [member(MORGAN, "owner"), member(ANA, "owner")]}));
    // The same one-to-one again: Graph gives the chat already there.
    assert_eq!(c.create_chat(MORGAN, std::slice::from_ref(&ana), Some("ignored")).unwrap()["id"], made["id"]);
    assert!(fake(&server, |t| t.creations[1].get("topic").is_none()), "a one-to-one has no topic");

    let group = c.create_chat(MORGAN, &[ana.clone(), person(BRUNO, "Bruno Reis", false)], Some("Launch")).unwrap();
    assert_eq!((group["chatType"].as_str(), group["topic"].as_str()), (Some("group"), Some("Launch")));
    let creations = fake(&server, |t| t.creations.clone());
    assert_eq!(
        creations[2],
        json!({"chatType": "group", "topic": "Launch", "members": [member(MORGAN, "owner"), member(ANA, "owner"), member(BRUNO, "owner")]})
    );
    c.create_chat(MORGAN, &[ana.clone(), person(BRUNO, "Bruno Reis", false)], Some(" ")).unwrap();
    assert!(fake(&server, |t| t.creations[3].get("topic").is_none()));
    let carla = Person { address: "carla@fabrikam.example".into(), ..person(CARLA, "Carla Dias", true) };
    c.create_chat(MORGAN, &[carla], None).unwrap();
    assert_eq!(fake(&server, |t| t.creations[4]["members"][1].clone()), member(CARLA, "guest"));
    assert_eq!(fake(&server, |t| t.chats.len()), 4, "four made, one found");
    fake(&server, |t| t.forbid_create = true);
    assert!(matches!(c.create_chat(MORGAN, &[person(BRUNO, "Bruno Reis", false)], None), Err(NetError::Protocol(m)) if m == "HTTP 403 Forbidden"));
}

#[test]
fn a_member_id_cant_break_the_bind() {
    let server = serve_teams(people_teams());
    let c = client(&server);
    let ana = person(ANA, "Ana Lima", false);
    let eve = Person { id: "x')/../me".into(), ..person(ANA, "Eve Moss", false) };
    let many: Vec<Person> = (0..21).map(|i| person(&format!("00000000-0000-0000-0000-0000000002{i:02}"), "Someone Else", false)).collect();
    let me = person(MORGAN, "Morgan Lee", false);
    let upper = Person { id: ANA.to_uppercase(), ..ana.clone() };
    let cases: [(&str, &[Person]); 7] = [
        (MORGAN, &[eve]),
        ("u-me", std::slice::from_ref(&ana)),
        (MORGAN, &many),
        (MORGAN, &[me]),
        (MORGAN, &[ana.clone(), ana.clone()]),
        (MORGAN, &[ana.clone(), upper]),
        (MORGAN, &[]),
    ];
    for (i, (me, people)) in cases.iter().enumerate() {
        assert!(matches!(c.create_chat(me, people, None), Err(NetError::Refused(_))), "{i}");
    }
    assert_eq!(hits(&server, "createChat"), 0);
}
