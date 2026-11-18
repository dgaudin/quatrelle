// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: Teams, files: the files and pictures a message takes (checked, queued, shown at
//! once by name), a message refused with its files not sent again without them, an upload under
//! way; a file someone shared asked for once, handed to the window once saved (shown in its folder
//! when it could run), a saved path outside the folder refused, and the states kept bounded.

use std::path::PathBuf;

use quatrelle_model::pim::teams::{Attachment, FileOutcome, FileWhy, Op, OutKind, OutPart, OutState, Outgoing, Part, SEND_MAX, Sender, Unsent};

use super::*;
use crate::{
    CmdResult, TEAMS_ATTACH_AGAIN, TEAMS_FILES_KEPT, TEAMS_TOO_MANY_DOWNLOADS, TEAMS_TOO_MANY_FILES, TeamsFileState, TeamsSaved, teams_could_run,
};

const CHAT: &str = "19:chat1@thread.v2";
const OTHER: &str = "19:chat2@thread.v2";
const ME: &str = "u-me";
const TENANT_ID: &str = "tenant-1";
/// The id of m1's file.
const G: &str = "00000000-0000-0000-0000-0000000000f1";

fn from(id: &str) -> Sender {
    Sender::User { id: id.into(), name: String::new(), identity_type: Some("aadUser".into()), tenant_id: Some(TENANT_ID.into()) }
}

fn file(id: &str, name: &str, url: &str) -> Attachment {
    Attachment::File { id: id.into(), name: name.into(), url: url.into(), size: None }
}

/// Ana's message with her report, a picture and a card.
fn m1() -> ChatMessage {
    let attachments = vec![
        file(G, "report.pdf", "https://contoso-my.sharepoint.com/personal/ana/report.pdf"),
        Attachment::Hosted { id: "h1".into() },
        Attachment::Card,
    ];
    ChatMessage { sender: from("u-ana"), attachments, ..msg(CHAT, 1) }
}

/// The user's own "hello".
fn m2() -> ChatMessage {
    ChatMessage { sender: from(ME), body: crate::mime::teams_compose("hello", &[], &[]), ..msg(CHAT, 2) }
}

/// A message of Ana's numbered `i`, with the files `f{i}-0` … `f{i}-{n-1}`.
fn with_files(i: i64, n: usize) -> ChatMessage {
    let attachments =
        (0..n).map(|k| file(&format!("f{i}-{k}"), &format!("file {k}.pdf"), "https://contoso-my.sharepoint.com/personal/ana/x")).collect();
    ChatMessage { sender: from("u-ana"), attachments, ..msg(CHAT, i) }
}

fn window(s: &mut Session, chat: &str, messages: Vec<ChatMessage>) {
    s.apply(teams_event(teams::Event::Messages { chat: chat.into(), messages, fresh: false, older: false }));
}

/// Chat 1 open and on screen, the user `u-me`; its window m1 and m2; its jobs taken.
fn opened() -> Session {
    let mut s = signed_in(vec![row(1, 10), row(2, 20)]);
    s.prefs.teams.object_id = ME.into();
    s.prefs.teams.tenant_id = TENANT_ID.into();
    s.module = Module::Conversations;
    open(&mut s, CHAT);
    window(&mut s, CHAT, vec![m1(), m2()]);
    s.take_jobs();
    s
}

/// A path of the test's own, absolute wherever the tests run.
fn tmp(name: &str) -> String {
    std::env::temp_dir().join("q").join(name).to_string_lossy().into_owned()
}

/// The folder files opened are saved in.
fn scratch() -> PathBuf {
    crate::store::scratch_dir("open").expect("a folder of files opened")
}

fn in_scratch(name: &str) -> String {
    scratch().join(name).to_string_lossy().into_owned()
}

fn open_file(s: &mut Session, message: &str, attachment: &str) -> CmdResult {
    s.run("teams.openFile", &json!({"chat": CHAT, "message": message, "attachment": attachment}))
}

fn file_done(s: &mut Session, chat: &str, message: &str, attachment: &str, outcome: FileOutcome) {
    s.apply(teams_event(teams::Event::File { chat: chat.into(), message: message.into(), attachment: attachment.into(), outcome }));
}

fn saved(s: &mut Session, message: &str, attachment: &str, path: &str) {
    file_done(s, CHAT, message, attachment, FileOutcome::Saved { path: path.into() });
}

fn sends(jobs: &[teams::Job]) -> Vec<&Op> {
    jobs.iter().filter_map(|j| if let teams::Job::Op { op } = j { Some(op) } else { None }).collect()
}

fn local_of(v: &Value) -> String {
    v["local"].as_str().unwrap_or_default().to_string()
}

fn outbox(chat: &str, items: Vec<Outgoing>, took: Option<&str>) -> Event {
    teams_event(teams::Event::Outbox { chat: chat.into(), items, took: took.map(str::to_string) })
}

#[test]
fn a_message_goes_with_its_files_and_pictures() {
    let mut s = opened();
    let (report, pasted) = (tmp("report.pdf"), tmp("pasted-1.png"));
    let local = local_of(&s.run("teams.send", &json!({"chat": CHAT, "text": "here", "files": [report], "pictures": [pasted]})).unwrap());
    let jobs = teams_jobs(&mut s);
    let [Op::Send { parts, local: sent, .. }] = &sends(&jobs)[..] else { panic!("{jobs:?}") };
    assert_eq!(sent, &local);
    assert_eq!(parts, &[Part::File { path: report }, Part::Picture { path: pasted }]);
    // Shown at once, each part by its name, nothing gone up yet.
    let [o] = &s.teams.outgoing[..] else { panic!("{:?}", s.teams.outgoing) };
    assert_eq!(
        o.parts,
        [
            OutPart { name: "report.pdf".into(), size: 0, picture: false, sent: 0 },
            OutPart { name: "pasted-1.png".into(), size: 0, picture: true, sent: 0 }
        ]
    );
}

#[test]
fn files_alone_are_a_message_and_nothing_at_all_isnt() {
    let mut s = opened();
    s.run("teams.send", &json!({"chat": CHAT, "text": "", "files": [tmp("report.pdf")]})).unwrap();
    s.run("teams.send", &json!({"chat": CHAT, "text": " \n ", "pictures": [tmp("pasted-1.png")]})).unwrap();
    let jobs = teams_jobs(&mut s);
    let sent = sends(&jobs);
    assert_eq!(sent.len(), 2, "{jobs:?}");
    assert!(sent.iter().all(|op| matches!(op, Op::Send { body, parts, .. } if body.blocks.is_empty() && parts.len() == 1)));
    // Nothing typed and nothing attached: nothing to send.
    for nothing in [json!({"chat": CHAT, "text": ""}), json!({"chat": CHAT, "text": "", "files": [], "pictures": null})] {
        assert!(matches!(s.run("teams.send", &nothing), Err(CmdError::Params(_))), "{nothing}");
    }
    assert!(teams_jobs(&mut s).is_empty());
    assert_eq!(s.teams.outgoing.len(), 2);
}

#[test]
fn eleven_parts_or_a_path_that_isnt_one_are_refused() {
    let mut s = opened();
    let paths = |n: usize| (0..n).map(|i| tmp(&format!("f{i}.pdf"))).collect::<Vec<_>>();
    let too_many = Err(CmdError::Params(TEAMS_TOO_MANY_FILES.into()));
    assert_eq!(s.run("teams.send", &json!({"chat": CHAT, "text": "x", "files": paths(11)})), too_many);
    assert_eq!(s.run("teams.send", &json!({"chat": CHAT, "text": "x", "files": paths(6), "pictures": paths(5)})), too_many);
    let long = tmp(&"a".repeat(5_000));
    // A path without a file's name (a root, a folder's `..`) would show as a chip without a name.
    let nameless = [json!(["/"]), json!([tmp("..")])];
    for bad in [json!(["report.pdf"]), json!([long]), json!([tmp("a\nb")]), json!([5]), json!(5), json!("x"), json!([""])].into_iter().chain(nameless)
    {
        for k in ["files", "pictures"] {
            let mut v = json!({"chat": CHAT, "text": "x"});
            v[k] = bad.clone();
            assert!(matches!(s.run("teams.send", &v), Err(CmdError::Params(_))), "{v}");
        }
    }
    assert!(teams_jobs(&mut s).is_empty());
    assert!(s.teams.outgoing.is_empty());
    // Ten together go.
    s.run("teams.send", &json!({"chat": CHAT, "text": "x", "files": paths(4), "pictures": paths(6)})).unwrap();
    assert_eq!(sends(&teams_jobs(&mut s)).len(), 1);
}

#[test]
fn the_places_of_the_files_count_in_the_length() {
    let mut s = opened();
    // Written `<p>…</p>`, 40 bytes short of the cap: a file's place takes 67.
    let text = "a".repeat(SEND_MAX - 7 - 40);
    s.run("teams.send", &json!({"chat": CHAT, "text": text})).unwrap();
    let with_file = s.run("teams.send", &json!({"chat": CHAT, "text": text, "files": [tmp("report.pdf")]}));
    assert_eq!(with_file, Err(CmdError::Params(crate::TEAMS_TOO_LONG.into())));
    assert_eq!(sends(&teams_jobs(&mut s)).len(), 1);
}

#[test]
fn an_edit_takes_no_file() {
    let mut s = opened();
    let v = json!({"chat": CHAT, "message": "m2", "text": "hello again", "files": [tmp("report.pdf")], "pictures": [tmp("pasted-1.png")]});
    s.run("teams.edit", &v).unwrap();
    let jobs = teams_jobs(&mut s);
    assert!(matches!(&sends(&jobs)[..], [Op::Edit { message, .. }] if message == "m2"), "{jobs:?}");
    assert!(s.teams.outgoing.iter().all(|o| o.parts.is_empty()), "{:?}", s.teams.outgoing);
}

#[test]
fn a_message_refused_with_its_files_is_not_sent_again_without_them() {
    let mut s = opened();
    let local = local_of(&s.run("teams.send", &json!({"chat": CHAT, "text": "here", "files": [tmp("report.pdf")]})).unwrap());
    teams_jobs(&mut s);
    // The outbox refused it at once (the file was gone): it keeps nothing of it, its parts neither.
    let refused = |chat: &str, local: &str| Outgoing {
        seq: 0,
        local: local.into(),
        chat: chat.into(),
        body: crate::mime::teams_compose("here", &[], &[]),
        mentions: Vec::new(),
        what: OutKind::Send,
        parts: Vec::new(),
        state: OutState::NotSent { reason: Unsent::FileGone, code: None },
    };
    s.apply(outbox(CHAT, vec![refused(CHAT, &local)], Some(&local)));
    let [o] = &s.teams.outgoing[..] else { panic!("{:?}", s.teams.outgoing) };
    assert_eq!(o.parts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["report.pdf"], "shown with the files it had");
    // Sent again, it would go without its files: the files are attached again instead.
    let retry = s.run("teams.retry", &json!({"seq": 0, "local": local}));
    assert_eq!(retry, Err(CmdError::Disabled(TEAMS_ATTACH_AGAIN.into())));
    assert!(teams_jobs(&mut s).is_empty());
    assert_eq!(s.teams.outgoing.len(), 1, "left where it was");
    // The same for a chat not open, once it's opened.
    let other = local_of(&s.run("teams.send", &json!({"chat": OTHER, "text": "here", "pictures": [tmp("pasted-1.png")]})).unwrap());
    s.apply(outbox(OTHER, vec![refused(OTHER, &other)], Some(&other)));
    open(&mut s, OTHER);
    teams_jobs(&mut s);
    assert_eq!(s.teams.outgoing.iter().map(|o| (o.local.as_str(), o.parts.len())).collect::<Vec<_>>(), [(other.as_str(), 1)]);
    let retry = s.run("teams.retry", &json!({"seq": 0, "local": other}));
    assert_eq!(retry, Err(CmdError::Disabled(TEAMS_ATTACH_AGAIN.into())));
    assert!(teams_jobs(&mut s).is_empty());
    // Deleted, one with files said without its number goes from the worker's outbox too, by its
    // local id: it may be one the worker keeps (read back with its files not shared).
    s.run("teams.discard", &json!({"seq": 0, "local": other})).unwrap();
    assert_eq!(teams_jobs(&mut s), [teams::Job::DiscardUnnumbered { local: other.clone() }]);
    assert!(s.teams.outgoing.is_empty());
    // One without files is sent again, as before.
    let plain = local_of(&s.run("teams.send", &json!({"chat": OTHER, "text": "plain"})).unwrap());
    teams_jobs(&mut s);
    let mut alone = refused(OTHER, &plain);
    alone.body = crate::mime::teams_compose("plain", &[], &[]);
    s.apply(outbox(OTHER, vec![alone], Some(&plain)));
    s.run("teams.retry", &json!({"seq": 0, "local": plain})).unwrap();
    assert_eq!(sends(&teams_jobs(&mut s)).len(), 1);
}

#[test]
fn an_upload_under_way_is_shown_as_the_worker_says() {
    let mut s = opened();
    let uploading = Outgoing {
        seq: 4,
        local: "100-1".into(),
        chat: CHAT.into(),
        body: crate::mime::teams_compose("here", &[], &[]),
        mentions: Vec::new(),
        what: OutKind::Send,
        parts: vec![OutPart { name: "report.pdf".into(), size: 200, picture: false, sent: 84 }],
        state: OutState::Uploading,
    };
    s.apply(outbox(CHAT, vec![uploading.clone()], None));
    assert_eq!(s.teams.outgoing, [uploading]);
}

#[test]
fn an_upload_under_way_can_be_cancelled() {
    // Its files going up, the message isn't in Teams yet: the worker lets its upload go.
    let mut s = opened();
    let mut uploading = Outgoing {
        seq: 4,
        local: "100-1".into(),
        chat: CHAT.into(),
        body: crate::mime::teams_compose("here", &[], &[]),
        mentions: Vec::new(),
        what: OutKind::Send,
        parts: vec![OutPart { name: "report.pdf".into(), size: 200, picture: false, sent: 84 }],
        state: OutState::Uploading,
    };
    s.apply(outbox(CHAT, vec![uploading.clone()], None));
    s.run("teams.discard", &json!({"seq": 4})).unwrap();
    assert!(s.teams.outgoing.is_empty());
    assert_eq!(teams_jobs(&mut s), [teams::Job::Discard { seq: 4 }]);
    // Being posted, it may be in Teams already: kept.
    uploading.state = OutState::Sending;
    s.apply(outbox(CHAT, vec![uploading], None));
    assert_eq!(s.run("teams.discard", &json!({"seq": 4})), Err(CmdError::Disabled(crate::TEAMS_MAY_BE_IN_TEAMS.into())));
}

#[test]
fn a_file_is_asked_once() {
    let mut s = opened();
    assert_eq!(open_file(&mut s, "m1", G), Ok(json!({"asked": true})));
    assert_eq!(open_file(&mut s, "m1", G), Ok(json!({"asked": false})), "a second click waits for the first");
    assert_eq!(teams_jobs(&mut s), [teams::Job::File { chat: CHAT.into(), message: "m1".into(), created: m1().created, attachment: G.into() }]);
    assert_eq!(s.teams.file_state("m1", G), Some(&TeamsFileState::Asked));
    assert_eq!(s.take_teams_saved(), None);
}

#[test]
fn only_a_file_of_a_message_in_the_window_is_asked() {
    let mut s = opened();
    let mut deleted = with_files(3, 1);
    deleted.deleted = Some(2_000_000);
    let mut no_address = with_files(4, 1);
    no_address.attachments = vec![file("f4-0", "report.pdf", "")];
    window(&mut s, CHAT, vec![deleted, no_address]);
    let disabled = |r: CmdResult| matches!(r, Err(CmdError::Disabled(_)));
    assert!(disabled(s.run("teams.openFile", &json!({"chat": OTHER, "message": "m1", "attachment": G}))), "another chat");
    assert!(disabled(open_file(&mut s, "m9", G)), "a message not there");
    assert!(disabled(open_file(&mut s, "m3", "f3-0")), "a deleted message");
    assert!(disabled(open_file(&mut s, "m1", "h1")), "a picture");
    assert!(disabled(open_file(&mut s, "m1", "card")), "the card");
    assert!(disabled(open_file(&mut s, "m2", G)), "another message's file");
    assert!(disabled(open_file(&mut s, "m4", "f4-0")), "a file without an address");
    for junk in
        [json!({"chat": CHAT, "message": "m1"}), json!({"chat": CHAT, "message": 1, "attachment": G}), json!({"message": "m1", "attachment": G})]
    {
        assert!(matches!(s.run("teams.openFile", &junk), Err(CmdError::Params(_))), "{junk}");
    }
    // A chat whose messages Teams refuses: nothing asked of it.
    let mut unreadable = row(1, 10);
    unreadable.unreadable = true;
    s.apply(teams_event(teams::Event::Chats { rows: vec![unreadable] }));
    assert!(disabled(open_file(&mut s, "m1", G)), "unreadable");
    assert!(teams_jobs(&mut s).is_empty());
    assert_eq!(s.teams.file_state("m1", G), None);
    // A chat left is read only: its files still open.
    let mut left = row(1, 10);
    left.left = true;
    s.apply(teams_event(teams::Event::Chats { rows: vec![left] }));
    assert_eq!(open_file(&mut s, "m1", G), Ok(json!({"asked": true})));
    assert_eq!(teams_jobs(&mut s).len(), 1);
}

#[test]
fn a_saved_file_is_handed_to_the_window_once() {
    let mut s = opened();
    open_file(&mut s, "m1", G).unwrap();
    let path = in_scratch("report.pdf");
    saved(&mut s, "m1", G, &path);
    assert_eq!(s.take_teams_saved(), Some(TeamsSaved { path: path.clone(), name: "report.pdf".into(), open: true }));
    assert_eq!(s.take_teams_saved(), None);
    assert_eq!(s.teams.file_state("m1", G), Some(&TeamsFileState::Saved { path: path.clone() }));
    // Said again: it isn't asked any more, nothing handed.
    saved(&mut s, "m1", G, &path);
    assert_eq!(s.take_teams_saved(), None);
}

#[test]
fn a_saved_file_that_isnt_there_is_never_made() {
    // Marked as come from elsewhere only when it's there: on Windows, the mark (a stream of the
    // file) would make an empty file of a name that isn't.
    let mut s = opened();
    open_file(&mut s, "m1", G).unwrap();
    let path = in_scratch(&format!("never made {}.pdf", std::process::id()));
    assert!(!std::path::Path::new(&path).exists());
    saved(&mut s, "m1", G, &path);
    assert!(!std::path::Path::new(&path).exists(), "nothing written");
    assert_eq!(s.take_teams_saved().map(|h| h.path), Some(path));
}

#[test]
fn a_saved_file_is_handed_to_the_window_again_without_a_download() {
    let mut s = opened();
    open_file(&mut s, "m1", G).unwrap();
    let path = in_scratch("report.pdf");
    saved(&mut s, "m1", G, &path);
    s.take_teams_saved();
    teams_jobs(&mut s);
    assert_eq!(open_file(&mut s, "m1", G), Ok(json!({"asked": false})));
    assert!(teams_jobs(&mut s).is_empty(), "never downloaded twice");
    assert_eq!(s.take_teams_saved(), Some(TeamsSaved { path: path.clone(), name: "report.pdf".into(), open: true }));
    // Clicked twice before the window took it: handed once.
    open_file(&mut s, "m1", G).unwrap();
    open_file(&mut s, "m1", G).unwrap();
    assert!(s.take_teams_saved().is_some());
    assert_eq!(s.take_teams_saved(), None);
}

#[test]
fn a_file_that_could_run_is_shown_in_its_folder_not_opened() {
    let names = [
        ("setup.exe", false),
        ("run.desktop", false),
        ("page.html", false),
        ("drawing.svg", false),
        ("macros.docm", false),
        ("link.lnk", false),
        ("PAGE.HTM", false),
        // No extension, or one of a type Quatrelle doesn't know: the system's opener would go by
        // what the file holds.
        ("README", false),
        ("report.pdfx", false),
        ("page.foo", false),
        ("notes.txt", true),
        ("report.pdf", true),
    ];
    let mut s = opened();
    window(&mut s, CHAT, vec![with_files(5, names.len())]);
    for (k, (name, open)) in names.into_iter().enumerate() {
        let attachment = format!("f5-{k}");
        open_file(&mut s, "m5", &attachment).unwrap();
        let path = in_scratch(name);
        saved(&mut s, "m5", &attachment, &path);
        assert_eq!(s.take_teams_saved(), Some(TeamsSaved { path, name: name.into(), open }), "{name}");
    }
}

#[test]
fn teams_could_run_more_than_the_mails_attachments() {
    // What mime's runs_code says, and web pages, SVGs and documents with macros, whatever their
    // capitals; a name without an extension too.
    for name in ["setup.exe", "a.desktop", "x.lnk", "report.pdf.exe", "README", "archive.", "notes v1.2 final", ".profile"] {
        assert!(teams_could_run(name), "{name}");
    }
    for ext in
        ["html", "htm", "xhtml", "shtml", "mht", "mhtml", "svg", "svgz", "docm", "dotm", "xlsm", "xltm", "xlsb", "pptm", "potm", "ppsm", "sldm"]
    {
        assert!(teams_could_run(&format!("file.{ext}")), "{ext}");
        assert!(teams_could_run(&format!("file.{}", ext.to_ascii_uppercase())), "{ext}");
    }
    for name in ["report.pdf", "notes.txt", "photo.JPG", "sheet.xlsx", "deck.pptx", "letter.docx", "archive.zip", "report (2).pdf"] {
        assert!(!teams_could_run(name), "{name}");
    }
    // The mail's own rule stays as it is.
    assert!(!crate::mime::runs_code("page.html") && !crate::mime::runs_code("macros.docm"));
}

/// Review 1.5, files 7: the Office formats before 2007 carry macros as the `m` ones do: a document,
/// a template, a sheet, a deck or a show of them, or a rich text, is shown in its folder, never
/// opened, whatever its capitals.
#[test]
fn a_legacy_office_file_is_shown_in_its_folder() {
    for name in ["plan.doc", "plan.dot", "sheet.xls", "sheet.xlt", "deck.ppt", "deck.pot", "deck.pps", "letter.rtf", "old.doc"] {
        assert!(teams_could_run(name), "{name}");
        assert!(teams_could_run(&name.to_ascii_uppercase()), "{name}");
    }
}

/// Final review, core I-1: the system's opener types a file whose extension it doesn't know by
/// what it holds (on Linux a web page saved as `report.pdfx` opens in the browser): only a type
/// Quatrelle knows, and that can't run, is opened; anything else is shown in its folder.
#[test]
fn only_a_type_quatrelle_knows_is_opened() {
    for name in ["report.pdfx", "page.foo", "draw.qqq", "notes.zzz", "README", "archive.tar.gz", "plan.dwg", "a.b.c"] {
        assert!(teams_could_run(name), "shown in its folder: {name}");
    }
    let known = ["report.pdf", "letter.docx", "photo.png", "notes.txt", "PHOTO.JPEG", "sheet.xlsx", "deck.pptx", "data.csv"];
    for name in known.into_iter().chain(["song.mp3", "clip.mp4", "archive.zip", "invite.ics", "card.vcf", "text.odt", "scan.tiff"]) {
        assert!(!teams_could_run(name), "opened: {name}");
    }
}

/// Final review, core M-1: a saved file removed since, opened again, is handed again; the window,
/// finding it gone, says so (`teams.fileGone`) and it is asked again at once. Gone again after
/// that download, it is only forgotten: the next Open asks it, nothing goes by itself.
#[test]
fn a_saved_file_removed_since_is_asked_again() {
    let mut s = opened();
    let path = in_scratch("report.pdf");
    let job = || teams::Job::File { chat: CHAT.into(), message: "m1".into(), created: m1().created, attachment: G.into() };
    open_file(&mut s, "m1", G).unwrap();
    saved(&mut s, "m1", G, &path);
    s.take_teams_saved();
    teams_jobs(&mut s);
    let gone = |s: &mut Session| s.run("teams.fileGone", &json!({"path": in_scratch("report.pdf")}));
    assert_eq!(open_file(&mut s, "m1", G), Ok(json!({"asked": false})));
    assert_eq!(s.take_teams_saved().map(|h| h.path), Some(path.clone()), "handed again");
    assert_eq!(gone(&mut s), Ok(json!({"asked": true})));
    assert_eq!(s.teams.file_state("m1", G), Some(&TeamsFileState::Asked));
    assert_eq!(teams_jobs(&mut s), [job()], "downloaded again");
    saved(&mut s, "m1", G, &path);
    assert!(s.take_teams_saved().is_some());
    assert_eq!(gone(&mut s), Ok(json!({"asked": false})), "gone again at once: not asked by itself");
    assert_eq!(s.teams.file_state("m1", G), None);
    assert!(teams_jobs(&mut s).is_empty());
    assert_eq!(open_file(&mut s, "m1", G), Ok(json!({"asked": true})), "the next Open asks it");
    assert_eq!(teams_jobs(&mut s), [job()]);
    // A path never handed, and junk: nothing.
    assert_eq!(s.run("teams.fileGone", &json!({"path": in_scratch("other.pdf")})), Ok(json!({"asked": false})));
    for junk in [json!({}), json!({"path": 5}), json!({"path": ""}), json!({"path": "/a\u{1}b"}), json!({"path": "x".repeat(5_000)})] {
        assert!(matches!(s.run("teams.fileGone", &junk), Err(CmdError::Params(_))), "{junk}");
    }
    assert_eq!(s.teams.file_state("m1", G), Some(&TeamsFileState::Asked));
}

/// A saved file handed to the window carries Windows' mark of the internet (zone 3), however it
/// was handed: saved, handed again by an Open, or downloaded again once removed.
#[test]
fn a_saved_file_is_marked_as_from_the_internet() {
    let mut s = opened();
    let path = in_scratch(&format!("marked {}.pdf", std::process::id()));
    let marked = |path: &str| !cfg!(windows) || std::fs::read_to_string(format!("{path}:Zone.Identifier")).is_ok_and(|z| z.contains("ZoneId=3"));
    open_file(&mut s, "m1", G).unwrap();
    std::fs::write(&path, b"%PDF-1.7").unwrap();
    saved(&mut s, "m1", G, &path);
    assert!(s.take_teams_saved().is_some_and(|h| marked(&h.path)), "saved");
    open_file(&mut s, "m1", G).unwrap();
    assert!(s.take_teams_saved().is_some_and(|h| marked(&h.path)), "handed again");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(s.run("teams.fileGone", &json!({"path": path})), Ok(json!({"asked": true})));
    std::fs::write(&path, b"%PDF-1.7").unwrap();
    saved(&mut s, "m1", G, &path);
    assert!(s.take_teams_saved().is_some_and(|h| marked(&h.path)), "downloaded again");
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn a_saved_path_outside_the_folder_is_refused() {
    let dir = scratch();
    let elsewhere = [
        "/etc/passwd".to_string(),
        dir.join("..").join("x.pdf").to_string_lossy().into_owned(),
        dir.join("sub").join("x.pdf").to_string_lossy().into_owned(),
        in_scratch("re\u{1}port.pdf"),
        in_scratch("CON.txt"),
        format!("{}-evil{}x.pdf", dir.to_string_lossy(), std::path::MAIN_SEPARATOR),
        "x.pdf".to_string(),
        String::new(),
    ];
    let mut s = opened();
    window(&mut s, CHAT, vec![with_files(6, elsewhere.len() + 1)]);
    for (k, path) in elsewhere.iter().enumerate() {
        let attachment = format!("f6-{k}");
        open_file(&mut s, "m6", &attachment).unwrap();
        saved(&mut s, "m6", &attachment, path);
        assert_eq!(s.take_teams_saved(), None, "{path:?}");
        assert_eq!(s.teams.file_state("m6", &attachment), Some(&TeamsFileState::Failed { why: FileWhy::Refused, code: None }), "{path:?}");
    }
    // Never asked, or another chat's: nothing handed, nothing kept.
    let never = format!("f6-{}", elsewhere.len());
    saved(&mut s, "m6", &never, &in_scratch("x.pdf"));
    assert_eq!(s.take_teams_saved(), None);
    assert_eq!(s.teams.file_state("m6", &never), None);
    open_file(&mut s, "m1", G).unwrap();
    file_done(&mut s, OTHER, "m1", G, FileOutcome::Saved { path: in_scratch("report.pdf") });
    assert_eq!(s.take_teams_saved(), None);
    assert_eq!(s.teams.file_state("m1", G), Some(&TeamsFileState::Asked));
}

#[test]
fn a_failed_download_is_kept_to_say_and_can_be_asked_again() {
    let mut s = opened();
    open_file(&mut s, "m1", G).unwrap();
    teams_jobs(&mut s);
    file_done(&mut s, CHAT, "m1", G, FileOutcome::Failed { why: FileWhy::TooBig, code: None });
    assert_eq!(s.teams.file_state("m1", G), Some(&TeamsFileState::Failed { why: FileWhy::TooBig, code: None }));
    assert_eq!(s.take_teams_saved(), None);
    assert_eq!(open_file(&mut s, "m1", G), Ok(json!({"asked": true})));
    assert_eq!(teams_jobs(&mut s).len(), 1, "asked again");
    assert_eq!(s.teams.file_state("m1", G), Some(&TeamsFileState::Asked));
    // Teams' answer is kept only in the client's shape.
    let codes = [
        (Some("HTTP 403 accessDenied"), Some("HTTP 403 accessDenied")),
        (Some("HTTP 404"), Some("HTTP 404")),
        (Some("HTTP 403 <b>denied</b>"), None),
        (Some("403 Forbidden"), None),
        (Some("HTTP 4033 x"), None),
        (Some("HTTP 403 \u{202E}x"), None),
        (None, None),
    ];
    let long = format!("HTTP 403 {}", "a".repeat(60));
    for (given, kept) in codes.iter().map(|(g, k)| (g.map(str::to_string), k.map(str::to_string))).chain([(Some(long), None)]) {
        open_file(&mut s, "m1", G).unwrap();
        file_done(&mut s, CHAT, "m1", G, FileOutcome::Failed { why: FileWhy::Refused, code: given.clone() });
        assert_eq!(s.teams.file_state("m1", G), Some(&TeamsFileState::Failed { why: FileWhy::Refused, code: kept }), "{given:?}");
    }
}

#[test]
fn the_files_kept_are_bounded() {
    let mut s = opened();
    let n = TEAMS_FILES_KEPT + 1;
    window(&mut s, CHAT, vec![with_files(7, n)]);
    let key = |k: usize| format!("f7-{k}");
    for k in 0..n {
        open_file(&mut s, "m7", &key(k)).unwrap();
        saved(&mut s, "m7", &key(k), &in_scratch(&format!("file {k}.pdf")));
    }
    let kept = (0..n).filter(|k| s.teams.file_state("m7", &key(*k)).is_some()).count();
    assert_eq!(kept, TEAMS_FILES_KEPT);
    assert_eq!(s.teams.file_state("m7", &key(0)), None, "the oldest went");
    // The files waiting for the window are bounded too, the oldest let go.
    let handed: Vec<TeamsSaved> = std::iter::from_fn(|| s.take_teams_saved()).collect();
    assert_eq!(handed.len(), TEAMS_FILES_KEPT);
    assert_eq!(handed.first().map(|h| h.name.as_str()), Some("file 1.pdf"));
    // Every one being downloaded: none is let go, the next refused.
    let mut s = opened();
    window(&mut s, CHAT, vec![with_files(7, n)]);
    for k in 0..TEAMS_FILES_KEPT {
        open_file(&mut s, "m7", &key(k)).unwrap();
    }
    teams_jobs(&mut s);
    assert_eq!(open_file(&mut s, "m7", &key(TEAMS_FILES_KEPT)), Err(CmdError::Disabled(TEAMS_TOO_MANY_DOWNLOADS.into())));
    assert!(teams_jobs(&mut s).is_empty());
    assert_eq!(s.teams.file_state("m7", &key(0)), Some(&TeamsFileState::Asked));
}

#[test]
fn the_files_go_with_the_chat() {
    let mut s = opened();
    open_file(&mut s, "m1", G).unwrap();
    // The same chat opened again keeps them.
    open(&mut s, CHAT);
    assert_eq!(s.teams.file_state("m1", G), Some(&TeamsFileState::Asked));
    // Another: they go, and the first chat's answer isn't taken.
    open(&mut s, OTHER);
    assert_eq!(s.teams.file_state("m1", G), None);
    open(&mut s, CHAT);
    window(&mut s, CHAT, vec![m1()]);
    saved(&mut s, "m1", G, &in_scratch("report.pdf"));
    assert_eq!(s.take_teams_saved(), None);
    assert_eq!(s.teams.file_state("m1", G), None);
}
