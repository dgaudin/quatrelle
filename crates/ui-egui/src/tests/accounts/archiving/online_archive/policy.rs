// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The archive and retention policies on screen: the submenu of a message and of a folder, in
//! two parts, and the header line.

use quatrelle_engine::model::{ArchiveTag, PolicyKind, dates};

use super::*;

const YEAR: &str = "22222222-aaaa-4bbb-8ccc-000000000002";
const NEVER: &str = "44444444-aaaa-4bbb-8ccc-000000000004";
const DELETE: &str = "55555555-aaaa-4bbb-8ccc-000000000005";
const WEEK: &str = "66666666-aaaa-4bbb-8ccc-000000000006";

/// Every tag's name, and the entry that takes a tag off.
const ENTRIES: [&str; 5] = ["1 Year Move to Archive", "Never Move to Archive", "3 Year Delete", "1 Week Delete", "As its folder"];

fn offer(a: &mut MailApp) {
    a.session.apply(MailEvent::ArchiveTags {
        account: "a1".into(),
        tags: vec![
            ArchiveTag { id: YEAR.into(), name: "1 Year Move to Archive".into(), days: Some(365), never: false, kind: PolicyKind::Archive },
            ArchiveTag { id: NEVER.into(), name: "Never Move to Archive".into(), days: None, never: true, kind: PolicyKind::Archive },
            ArchiveTag { id: DELETE.into(), name: "3 Year Delete".into(), days: Some(1095), never: false, kind: PolicyKind::Retention },
            ArchiveTag { id: WEEK.into(), name: "1 Week Delete".into(), days: Some(7), never: false, kind: PolicyKind::Retention },
        ],
    });
}

/// Where the checkmarks are drawn, as the text they sit beside: the line of text nearest to
/// each three-point stroke.
fn ticked(shapes: &[egui::Shape]) -> Vec<String> {
    let lines: Vec<(f32, String)> = shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some((t.pos.y + t.galley.size().y / 2.0, t.galley.text().to_string())),
            _ => None,
        })
        .collect();
    shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Path(p) if p.points.len() == 3 && p.points[0].x < p.points[1].x && p.points[1].x < p.points[2].x => {
                Some(p.points.iter().map(|q| q.y).sum::<f32>() / 3.0)
            }
            _ => None,
        })
        .filter_map(|y| {
            let near = lines.iter().filter(|(_, t)| ENTRIES.contains(&t.as_str()));
            near.min_by(|a, b| (a.0 - y).abs().total_cmp(&(b.0 - y).abs())).filter(|(at, _)| (at - y).abs() < 5.0)
        })
        .map(|(_, t)| t.clone())
        .collect()
}

fn open_submenu(ctx: &egui::Context, a: &mut MailApp, row: &str) -> Vec<egui::Shape> {
    let at = settled_text_at(ctx, a, row, SIZE).expect("the row");
    right_click(ctx, a, at);
    let shapes = settle(ctx, a);
    let Some(sub) = text_at(&shapes, "Policy") else { return shapes };
    click_at(ctx, a, sub, 1, SIZE);
    settle(ctx, a)
}

fn y_of(shapes: &[egui::Shape], text: &str) -> f32 {
    text_at(shapes, text).map_or(f32::NAN, |p| p.y)
}

/// A message's right-click menu has the « Policy » submenu in two parts, light and dark: the
/// archive tags under their heading, then the retention tags under theirs, each in the server's
/// words with « As its folder », the tag carried ticked in each part (« As its folder » when
/// none is).
#[test]
fn the_submenu_lists_the_servers_tags_in_two_parts_with_the_ones_in_place_ticked() {
    let ctx = egui::Context::default();
    let cases = [
        (Some(NEVER), Some(WEEK), ["Never Move to Archive", "1 Week Delete"]),
        (None, None, ["As its folder", "As its folder"]),
        (Some(YEAR), None, ["1 Year Move to Archive", "As its folder"]),
    ];
    for dark in [Some(false), Some(true)] {
        for (archive, retention, want) in cases {
            let (mut a, dir, _) = exchange_app("oa-policy-menu");
            a.ui.dark = dark;
            offer(&mut a);
            let key = message("a1", "INBOX", 1, "").key;
            a.session.run("list.select", &json!({"key": key.to_id()})).unwrap();
            a.session.apply(MailEvent::ArchiveTagRead {
                key,
                tag: archive.map(str::to_string),
                date: None,
                retention_tag: retention.map(str::to_string),
                retention_date: None,
            });
            let shapes = open_submenu(&ctx, &mut a, "Plans");
            let all = texts(&shapes);
            for t in ENTRIES.iter().chain(&["Archive policy", "Retention policy"]) {
                assert!(all.iter().any(|x| x == t), "{t} in {all:?}");
            }
            assert_eq!(all.iter().filter(|t| *t == "As its folder").count(), 2, "one in each part: {all:?}");
            let order = ["Archive policy", "1 Year Move to Archive", "Never Move to Archive", "Retention policy", "3 Year Delete", "1 Week Delete"];
            let ys: Vec<f32> = order.iter().map(|t| y_of(&shapes, t)).collect();
            assert!(ys.windows(2).all(|w| w[0] < w[1]), "{order:?} drawn in this order: {ys:?}");
            assert_eq!(ticked(&shapes), want, "{dark:?} {all:?}");
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// Choosing a tag runs the command with its id.
#[test]
fn choosing_a_tag_sets_it() {
    let ctx = egui::Context::default();
    let (mut a, dir, handed) = exchange_app("oa-policy-choose");
    offer(&mut a);
    let key = message("a1", "INBOX", 1, "").key;
    a.session.run("list.select", &json!({"key": key.to_id()})).unwrap();
    let _ = open_submenu(&ctx, &mut a, "Plans");
    let at = settled_text_at(&ctx, &mut a, "1 Year Move to Archive", SIZE).expect("the tag");
    click_at(&ctx, &mut a, at, 1, SIZE);
    settle(&ctx, &mut a);
    let sent = handed.borrow().iter().any(|j| {
        matches!(j, Job::Apply { op: quatrelle_engine::model::Op::SetArchiveTag { tag: Some(t), kind: PolicyKind::Archive, .. }, .. } if t == YEAR)
    });
    assert!(sent, "{:?}", handed.borrow());
    // A retention tag: the retention command, with its id.
    let _ = open_submenu(&ctx, &mut a, "Plans");
    let at = settled_text_at(&ctx, &mut a, "1 Week Delete", SIZE).expect("the retention tag");
    click_at(&ctx, &mut a, at, 1, SIZE);
    settle(&ctx, &mut a);
    let sent = handed.borrow().iter().any(|j| {
        matches!(j, Job::Apply { op: quatrelle_engine::model::Op::SetArchiveTag { tag: Some(t), kind: PolicyKind::Retention, .. }, .. } if t == WEEK)
    });
    assert!(sent, "{:?}", handed.borrow());
    let _ = std::fs::remove_dir_all(dir);
}

/// A folder of the mailbox has it too; a server that offers no tag, or the archive's own
/// mail, none.
#[test]
fn the_submenu_is_absent_without_tags_and_in_the_archive() {
    let ctx = egui::Context::default();
    // A folder of the mailbox.
    let (mut a, dir, _) = exchange_app("oa-policy-folder");
    offer(&mut a);
    let shapes = open_submenu(&ctx, &mut a, "Sent Items");
    assert!(
        texts(&shapes).iter().any(|t| t == "1 Year Move to Archive") && texts(&shapes).iter().any(|t| t == "1 Week Delete"),
        "{:?}",
        texts(&shapes)
    );
    let _ = std::fs::remove_dir_all(dir);
    // No tags: no submenu, on a message and on a folder.
    for row in ["Plans", "Sent Items"] {
        let (mut a, dir, _) = exchange_app("oa-policy-none");
        let at = settled_text_at(&ctx, &mut a, row, SIZE).expect("the row");
        right_click(&ctx, &mut a, at);
        let all = texts(&settle(&ctx, &mut a));
        assert!(all.iter().any(|t| t == "Properties…" || t == "Reply"), "the menu is open: {all:?}");
        assert!(!all.iter().any(|t| t == "Policy"), "{row}: {all:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
    // The archive's mail and folders.
    let (mut a, dir, _) = exchange_app("oa-policy-archive");
    offer(&mut a);
    open_projects(&mut a);
    settle(&ctx, &mut a);
    let at = settled_text_at(&ctx, &mut a, "Old plans", SIZE).expect("the message");
    right_click(&ctx, &mut a, at);
    let all = texts(&settle(&ctx, &mut a));
    assert!(all.iter().any(|t| t == "Reply"), "the menu is open: {all:?}");
    assert!(!all.iter().any(|t| t == "Policy"), "{all:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A day as the interface writes it.
fn shown(y: i64, m: u32, d: u32) -> String {
    crate::i18n::date(&dates::day_label(dates::days_from_civil(y, m, d)))
}

/// A message that carries tags says so in its header: each tag's name and the day the server
/// plans, the archive one then the retention one; only the one it carries when it carries one.
#[test]
fn the_header_names_the_tags_and_the_days() {
    let ctx = egui::Context::default();
    let (mut a, dir, _) = exchange_app("oa-policy-header");
    offer(&mut a);
    let key = message("a1", "INBOX", 1, "").key;
    a.session
        .store
        .as_ref()
        .unwrap()
        .write_body(&key, b"From: Zoe Park <zoe@contoso.example>\r\nTo: riley@contoso.example\r\nSubject: Plans\r\n\r\nHello.\r\n")
        .unwrap();
    a.session.open_message(&key);
    let none = texts(&settle(&ctx, &mut a));
    assert!(none.iter().any(|t| t == "Hello."), "the message is open: {none:?}");
    assert!(!none.iter().any(|t| t.contains("Archive:") || t.contains("Deletion:")), "{none:?}");
    let read = |a: &mut MailApp, tag: Option<&str>, date: Option<&str>, retention: Option<&str>, retention_date: Option<&str>| {
        a.session.apply(MailEvent::ArchiveTagRead {
            key: key.clone(),
            tag: tag.map(str::to_string),
            date: date.map(str::to_string),
            retention_tag: retention.map(str::to_string),
            retention_date: retention_date.map(str::to_string),
        });
        texts(&settle(&ctx, a)).into_iter().find(|t| t.contains("Archive:") || t.contains("Deletion:"))
    };
    let both = read(&mut a, Some(YEAR), Some("2027-11-10"), Some(WEEK), Some("2026-11-19")).expect("the header line");
    let want = format!("Archive: 1 Year Move to Archive ({}) · Deletion: 1 Week Delete ({})", shown(2027, 11, 10), shown(2026, 11, 19));
    assert_eq!(both, want);
    let deletion = read(&mut a, None, None, Some(DELETE), None).expect("the header line");
    assert_eq!(deletion, "Deletion: 3 Year Delete");
    let archive = read(&mut a, Some(NEVER), None, None, None).expect("the header line");
    assert_eq!(archive, "Archive: Never Move to Archive");
    let _ = std::fs::remove_dir_all(dir);
}
