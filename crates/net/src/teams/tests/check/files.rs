// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The files check (`--files <chat>`) against the fake Graph server: one line for each step, no
//! name, address or id in them, and nothing of the check left in the chat or the drive.

use serde_json::json;

use super::*;
use crate::NetError;
use crate::teams::graph::{host_tail, transfer_public_only, without_host};

/// What the check sends up and what it says of it.
const CONTENT: &[u8] = b"Quatrelle check, please ignore";
const ANA_ID: &str = "00000000-0000-0000-0000-0000000000a1";
/// The start of each of the eight lines, in order.
const STEPS: [&str; 8] =
    ["upload session:", "upload:", "item:", "shared with the others:", "message with the file:", "listed as a file:", "download:", "cleaned up:"];

/// A group chat of the user, Ana (who has an object id) and Bruno (who has none).
fn files_server(change: impl FnOnce(&mut FakeTeams)) -> Server {
    let members = vec![me(), FakeUser::new(ANA_ID, "Ana Exemple"), FakeUser::new("u-bruno", "Bruno Reis")];
    let mut chat = FakeChat::new(CHAT, "group", None, members);
    chat.messages.push(FakeMessage::new("1", Some("u-bruno"), &rfc3339(T0), "<p>Hello</p>"));
    let mut t = teams_with(vec![chat]);
    t.drive = Some("https://contoso-my.sharepoint.com/personal/me_contoso_example/Documents".into());
    change(&mut t);
    serve_teams(t)
}

fn files_opts() -> CheckOptions {
    CheckOptions { files_chat: Some(CHAT.into()), ..opts() }
}

/// The eight lines the files check said, in the order said.
fn said(lines: &[String]) -> Vec<&str> {
    lines.iter().map(String::as_str).filter(|l| STEPS.iter().any(|s| l.starts_with(s))).collect()
}

#[test]
fn the_files_check_says_each_step_and_leaves_nothing() {
    let server = files_server(|_| {});
    let (r, lines) = check(&server, &files_opts());
    r.unwrap();
    assert_eq!(
        said(&lines),
        [
            "upload session: yes (host: …)",
            "upload: yes",
            "item: yes (attachment id: yes, on your OneDrive: yes)",
            "shared with the others: 2 of 2",
            "message with the file: yes",
            "listed as a file: yes",
            "download: yes (host: …)",
            "cleaned up: yes",
        ],
        "{lines:#?}"
    );
    // What went up: 30 bytes in one range, to a session of a name of its own, without the token.
    let t = teams_of(&server);
    assert_eq!(t.drive_files.sessions[0].name, "Quatrelle check.txt");
    assert_eq!(t.drive_files.ranges.len(), 1);
    assert_eq!(t.drive_files.ranges[0].1..=t.drive_files.ranges[0].2, 0..=29);
    assert_eq!(t.drive_files.ranges[0].3, 30);
    assert!(!headers(&server, "upload").iter().any(|(k, _)| k == "authorization"), "{:?}", headers(&server, "upload"));
    assert_eq!(hits(&server, "upload"), 1);
    assert_eq!(CONTENT.len(), 30);
    // The others only, ana by her object id and Bruno by his address; read only, silent.
    let invites = &t.drive_files.invites;
    assert_eq!(invites.len(), 1);
    assert_eq!(invites[0].1["recipients"], json!([{"objectId": ANA_ID}, {"email": "bruno@contoso.example"}]));
    assert_eq!(invites[0].1["roles"], json!(["read"]));
    assert_eq!(invites[0].1["sendInvitation"], json!(false));
    // The message carried the file and its text; the file was fetched without the token.
    let post = &t.posts[0].1;
    assert!(post["body"]["content"].as_str().unwrap().starts_with("<p>Quatrelle check, please ignore (file)</p><attachment id=\""), "{post}");
    assert_eq!(post["attachments"].as_array().map(Vec::len), Some(1));
    assert_eq!((hits(&server, "shares"), hits(&server, "download")), (1, 1));
    assert!(!headers(&server, "download").iter().any(|(k, _)| k == "authorization"), "{:?}", headers(&server, "download"));
    // Nothing is left: no item, the message deleted.
    assert!(t.drive_files.files.is_empty(), "{:?}", t.drive_files.files);
    let sent = mine(&server);
    assert!(sent.len() == 1 && sent[0].1, "{sent:?}");
}

#[test]
fn a_refused_share_is_said_and_still_cleaned_up() {
    let server = files_server(|t| t.drive_files.forbid_share = true);
    let (r, lines) = check(&server, &files_opts());
    r.unwrap();
    let said = said(&lines);
    assert_eq!(said[3], "shared with the others: no (HTTP 403 Forbidden)", "{lines:#?}");
    assert_eq!(said[4], "message with the file: yes");
    assert_eq!(said[7], "cleaned up: yes");
    assert!(teams_of(&server).drive_files.files.is_empty());
    let sent = mine(&server);
    assert!(sent.len() == 1 && sent[0].1, "{sent:?}");

    // Some of them only: counted as granted of asked.
    let server = files_server(|t| t.drive_files.fail_share = vec!["bruno@contoso.example".into()]);
    let (r, lines) = check(&server, &files_opts());
    r.unwrap();
    assert_eq!(line(&lines, "shared with the others:"), "shared with the others: 1 of 2");
}

#[test]
fn a_failure_goes_no_further_and_the_clean_up_still_runs() {
    // A full OneDrive: no session, so nothing after it is tried, and nothing was made.
    let server = files_server(|t| t.drive_files.full = true);
    let (r, lines) = check(&server, &files_opts());
    r.unwrap();
    assert_eq!(
        said(&lines),
        [
            "upload session: no (HTTP 507 quotaLimitReached)",
            "upload: not tried",
            "item: not tried",
            "shared with the others: not tried",
            "message with the file: not tried",
            "listed as a file: not tried",
            "download: not tried",
            "cleaned up: yes",
        ],
        "{lines:#?}"
    );
    assert_eq!((hits(&server, "upload"), hits(&server, "post")), (0, 0));

    // An upload address off Microsoft's hosts is refused without a word of it.
    let server = files_server(|t| t.drive_files.upload_elsewhere = Some("https://evil.example/x?tempauth=SECRET".into()));
    let (r, lines) = check(&server, &files_opts());
    r.unwrap();
    assert_eq!(said(&lines)[0], "upload session: no (refused)");
    assert!(lines.iter().all(|l| !l.contains("evil") && !l.contains("SECRET")), "{lines:#?}");

    // The message refused: said; the file made is deleted all the same.
    let server = files_server(|t| t.forbid_post.push(CHAT.into()));
    let (r, lines) = check(&server, &files_opts());
    r.unwrap();
    let said = said(&lines);
    assert_eq!(said[3], "shared with the others: 2 of 2");
    assert_eq!(said[4], "message with the file: no (HTTP 403 Forbidden)");
    assert_eq!(&said[5..7], ["listed as a file: not tried", "download: not tried"]);
    assert_eq!(said[7], "cleaned up: yes");
    assert!(teams_of(&server).drive_files.files.is_empty());

    // A delete that fails is said, not hidden.
    let server = files_server(|t| _ = t.fail_next.insert("deleteItem".into(), 100));
    let (r, lines) = check(&server, &files_opts());
    r.unwrap();
    assert_eq!(line(&lines, "cleaned up:"), "cleaned up: no (the file is left)");
}

#[test]
fn a_lost_answer_to_the_last_range_leaves_a_file_that_is_said() {
    // The range was taken and the file made, but the answer never came: its item is unknown, so
    // it can't be deleted, and the check must not say it cleaned up.
    let server = files_server(|t| _ = t.answer_lost.insert("upload".into(), 1));
    let (r, lines) = check(&server, &files_opts());
    r.unwrap();
    let said = said(&lines);
    assert!(said[1].starts_with("upload: no"), "{lines:#?}");
    assert_eq!(said[7], "cleaned up: no (a file may be left)");
    assert_eq!(teams_of(&server).drive_files.files.len(), 1, "the fake did make it");
    // A refusal said before acting (the session gone) leaves nothing to look for.
    let server = files_server(|t| t.drive_files.expire_after = Some(0));
    let (r, lines) = check(&server, &files_opts());
    r.unwrap();
    assert_eq!(said_last(&lines), "cleaned up: yes");
}

fn said_last(lines: &[String]) -> &str {
    line(lines, "cleaned up:")
}

#[test]
fn a_file_that_is_not_on_their_drive_is_said() {
    let server = files_server(|t| t.drive_files.item_elsewhere = Some("other-my.sharepoint.com".into()));
    let (r, lines) = check(&server, &files_opts());
    r.unwrap();
    assert_eq!(said(&lines)[2], "item: yes (attachment id: yes, on your OneDrive: no)", "{lines:#?}");
    assert_eq!(said(&lines)[7], "cleaned up: yes");
}

#[test]
fn the_lines_name_no_one_and_no_file() {
    let server = files_server(|_| {});
    let (r, lines) = check(&server, &CheckOptions { write_chat: None, ..files_opts() });
    r.unwrap();
    assert!(!said(&lines).is_empty());
    for l in &lines {
        for private in [
            CHAT,
            "19:a1b2",
            ANA_ID,
            "Ana",
            "Bruno",
            "Morgan",
            "Lima",
            "Reis",
            "contoso",
            "@",
            "check.txt",
            "Quatrelle check",
            "http",
            "127.0.0.1",
            "upload/",
            "tempauth",
            "ITEM",
        ] {
            assert!(!l.contains(private), "{private:?} in {l:?}");
        }
    }
    // The file's own words and the upload's are not in the log either way: a failure, too.
    let server = files_server(|t| t.drive_files.full = true);
    let (_, lines) = check(&server, &files_opts());
    assert!(lines.iter().all(|l| !l.contains("check.txt") && !l.contains("127.0.0.1")), "{lines:#?}");
}

#[test]
fn a_chat_alone_or_one_that_isnt_there_is_said() {
    // Nobody but the user: nothing to share with, the rest as usual.
    let mut t = teams_with(vec![FakeChat::new(CHAT, "group", None, vec![me()])]);
    t.chats[0].messages.push(FakeMessage::new("1", Some(ME), &rfc3339(T0), "<p>Note</p>"));
    let server = serve_teams(t);
    let (r, lines) = check(&server, &files_opts());
    r.unwrap();
    assert_eq!(said(&lines)[3], "shared with the others: 0 of 0", "{lines:#?}");
    assert_eq!(said(&lines)[7], "cleaned up: yes");
    // No OneDrive address to compare with (Graph says none): not known, not no.
    assert_eq!(said(&lines)[2], "item: yes (attachment id: yes, on your OneDrive: not known)");
    // A chat that doesn't exist: the members can't be read, the message isn't made.
    let server = files_server(|_| {});
    let (r, lines) = check(&server, &CheckOptions { files_chat: Some("19:no-such-quixote@thread.v2".into()), ..opts() });
    r.unwrap();
    let said = said(&lines);
    assert!(said[3].starts_with("shared with the others: no (HTTP 404"), "{lines:#?}");
    assert!(said[4].starts_with("message with the file: no (HTTP 404"), "{lines:#?}");
    assert_eq!(said[7], "cleaned up: yes");
    assert!(teams_of(&server).drive_files.files.is_empty());
}

#[test]
fn an_upload_connection_needs_a_public_address_but_the_tests_loopback() {
    // Graph's own address: the internet only. The tests' server on this computer: this computer.
    assert!(transfer_public_only("https://graph.microsoft.com/v1.0"));
    assert!(transfer_public_only("https://127.0.0.1:8443/v1.0"));
    assert!(transfer_public_only("http://192.168.1.5/v1.0"));
    assert!(!transfer_public_only("http://127.0.0.1:8080"));
    assert!(!transfer_public_only("http://localhost:8080"));
    // The log says a host's last two labels, or nothing for an address.
    assert_eq!(host_tail("contoso-my.sharepoint.com"), "…sharepoint.com");
    assert_eq!(host_tail("127.0.0.1"), "…");
    // A connection's error names the host by that alone, whatever the system said.
    let gone = std::io::Error::new(std::io::ErrorKind::TimedOut, "contoso-my.sharepoint.com:443: timed out");
    for e in [
        NetError::Io(gone),
        NetError::Tls("contoso-my.sharepoint.com: invalid certificate".into()),
        NetError::Protocol("contoso-my.sharepoint.com is on this computer or its network".into()),
    ] {
        let said = without_host(e, &host_tail("contoso-my.sharepoint.com")).to_string();
        assert!(said.contains("…sharepoint.com") && !said.contains("contoso"), "{said}");
    }
}
