// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The archive policy on screen: the submenu of a message and of a folder, and the header line.

use quatrelle_engine::model::ArchiveTag;

use super::*;

const YEAR: &str = "22222222-aaaa-4bbb-8ccc-000000000002";
const NEVER: &str = "44444444-aaaa-4bbb-8ccc-000000000004";

fn offer(a: &mut MailApp) {
    a.session.apply(MailEvent::ArchiveTags {
        account: "a1".into(),
        tags: vec![
            ArchiveTag { id: YEAR.into(), name: "1 Year Move to Archive".into(), days: Some(365), never: false },
            ArchiveTag { id: NEVER.into(), name: "Never Move to Archive".into(), days: None, never: true },
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
            let near = lines.iter().filter(|(_, t)| ["1 Year Move to Archive", "Never Move to Archive", "As its folder"].contains(&t.as_str()));
            near.min_by(|a, b| (a.0 - y).abs().total_cmp(&(b.0 - y).abs())).filter(|(at, _)| (at - y).abs() < 5.0)
        })
        .map(|(_, t)| t.clone())
        .collect()
}

fn open_submenu(ctx: &egui::Context, a: &mut MailApp, row: &str) -> Vec<egui::Shape> {
    let at = settled_text_at(ctx, a, row, SIZE).expect("the row");
    right_click(ctx, a, at);
    let shapes = settle(ctx, a);
    let Some(sub) = text_at(&shapes, "Archive policy") else { return shapes };
    click_at(ctx, a, sub, 1, SIZE);
    settle(ctx, a)
}

/// A message's right-click menu has the submenu with the server's tags in the server's words,
/// the one it carries ticked, and « As its folder »; with none carried, that one is ticked.
#[test]
fn the_submenu_lists_the_servers_tags_with_the_one_in_place_ticked() {
    let ctx = egui::Context::default();
    for (current, want) in [(Some(NEVER), "Never Move to Archive"), (None, "As its folder")] {
        let (mut a, dir, _) = exchange_app("oa-policy-menu");
        offer(&mut a);
        let key = message("a1", "INBOX", 1, "").key;
        a.session.run("list.select", &json!({"key": key.to_id()})).unwrap();
        a.session.apply(MailEvent::ArchiveTagRead { key, tag: current.map(str::to_string), date: None });
        let shapes = open_submenu(&ctx, &mut a, "Plans");
        let all = texts(&shapes);
        for t in ["1 Year Move to Archive", "Never Move to Archive", "As its folder"] {
            assert!(all.iter().any(|x| x == t), "{t} in {all:?}");
        }
        assert_eq!(ticked(&shapes), [want], "{all:?}");
        let _ = std::fs::remove_dir_all(dir);
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
    let sent = handed
        .borrow()
        .iter()
        .any(|j| matches!(j, Job::Apply { op: quatrelle_engine::model::Op::SetArchiveTag { tag: Some(t), .. }, .. } if t == YEAR));
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
    assert!(texts(&shapes).iter().any(|t| t == "1 Year Move to Archive"), "{:?}", texts(&shapes));
    let _ = std::fs::remove_dir_all(dir);
    // No tags: no submenu, on a message and on a folder.
    for row in ["Plans", "Sent Items"] {
        let (mut a, dir, _) = exchange_app("oa-policy-none");
        let at = settled_text_at(&ctx, &mut a, row, SIZE).expect("the row");
        right_click(&ctx, &mut a, at);
        let all = texts(&settle(&ctx, &mut a));
        assert!(all.iter().any(|t| t == "Properties…" || t == "Reply"), "the menu is open: {all:?}");
        assert!(!all.iter().any(|t| t == "Archive policy"), "{row}: {all:?}");
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
    assert!(!all.iter().any(|t| t == "Archive policy"), "{all:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A message that carries a tag says so in its header: the tag's name and the day planned.
#[test]
fn the_header_names_the_tag_and_the_day() {
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
    assert!(!none.iter().any(|t| t.contains("Archive policy")), "{none:?}");
    a.session.apply(MailEvent::ArchiveTagRead { key, tag: Some(YEAR.into()), date: Some("2027-11-10".into()) });
    let line = texts(&settle(&ctx, &mut a)).into_iter().find(|t| t.contains("Archive policy:"));
    let line = line.expect("the header line");
    assert!(line.starts_with("Archive policy: 1 Year Move to Archive.") && line.contains("Planned for") && line.contains("2027"), "{line}");
    let _ = std::fs::remove_dir_all(dir);
}
