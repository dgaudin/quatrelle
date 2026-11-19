// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the files kept with an appointment or a contact imported from a `.pst` (its
//! `ATTACH` and `X-QUATRELLE-ATTACH` lines, written as the import writes them) show as chips, as
//! a message's attachments do, in the appointment's window and on the contact's page, each with
//! Open and Save As; the calendar's card names them. With `QUATRELLE_FRAME_PNG` set to a folder,
//! the window and the page are painted there, light and dark.

use quatrelle_engine::model::ical::CalEvent;
use quatrelle_engine::model::vcard::{Card, Typed, base64_encode};

use super::super::list::follow_up::{Textures, frame_with_size};
use super::*;

const SIZE: (usize, usize) = (1280, 800);

/// The two files, as `pst_pim/files.rs` writes an event's.
fn event_lines() -> Vec<String> {
    vec![
        format!("ATTACH;FMTTYPE=application/pdf;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"Agenda draft.pdf\":{}", base64_encode(&[b'%'; 2400])),
        format!("ATTACH;FMTTYPE=text/plain;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"Room plan.txt\":{}", base64_encode(b"Room 4, second floor")),
    ]
}

/// The two files, as `pst_pim/files.rs` writes a card's.
fn card_lines() -> Vec<String> {
    let docx = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
    vec![
        format!("X-QUATRELLE-ATTACH;X-FILENAME=\"CV Hélène.docx\":data:{docx};base64,{}", base64_encode(&[7u8; 5000])),
        format!("X-QUATRELLE-ATTACH;X-FILENAME=\"Signed offer.pdf\":data:application/pdf;base64,{}", base64_encode(b"%PDF-1.4")),
    ]
}

/// The texts drawn once settled, light and dark, each painted as `name` when pictures are asked
/// for; the light ones.
fn drawn(ctx: &egui::Context, a: &mut MailApp, tex: &mut Textures, name: &str) -> Vec<String> {
    let pictures = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let mut light = Vec::new();
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        a.ui.dark = Some(dark);
        for _ in 0..12 {
            frame_with_size(ctx, a, tex, vec![], None, SIZE);
        }
        let png = pictures.as_ref().map(|d| d.join(format!("{name}-{theme}.png")));
        let (_, shapes) = frame_with_size(ctx, a, tex, vec![], png, SIZE);
        if !dark {
            light = shapes
                .iter()
                .filter_map(|s| match s {
                    egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                    _ => None,
                })
                .collect();
        }
    }
    a.ui.dark = Some(false);
    light
}

fn has(texts: &[String], wanted: &str) -> bool {
    texts.iter().any(|t| t == wanted)
}

#[test]
fn an_imported_appointments_files_show_in_its_window_and_card() {
    let (mut a, dir) = app("item-files-event");
    let ctx = egui::Context::default();
    let mut tex = Textures::default();
    frame(&ctx, &mut a, vec![]);
    let start = (NOW / 86_400 + 1) * 86_400 + 10 * 3600;
    let e =
        CalEvent { uid: "pst-files-1".into(), summary: "Budget review".into(), start, end: start + 3600, extra: event_lines(), ..Default::default() };
    a.session.create_event(None, None, e).unwrap();
    let item = a.session.agenda(NOW, NOW + 3 * 86_400).into_iter().find(|i| i.summary == "Budget review").unwrap();
    // The calendar's card names them.
    let rows = crate::calendar::card_rows(&a.session, &item);
    assert!(rows.iter().any(|(_, text)| text == "Agenda draft.pdf, Room plan.txt"), "{rows:?}");
    // Its window: a chip each, its name and size.
    a.ui.dialog = Dialog::open_event(&a.session, &item.uid, item.occurrence);
    let t = drawn(&ctx, &mut a, &mut tex, "event-files");
    assert_eq!(a.ui.event_windows.len(), 1);
    for want in ["Agenda draft.pdf", "2 KB", "Room plan.txt", "20 B"] {
        assert!(has(&t, want), "{want} in {t:?}");
    }
    // Its menu: Open and Save As, as a message's attachment has.
    let at = settled_text_at(&ctx, &mut a, "Agenda draft.pdf", egui::vec2(SIZE.0 as f32, SIZE.1 as f32)).unwrap();
    let size = egui::vec2(SIZE.0 as f32, SIZE.1 as f32);
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(at)], size);
    let right = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    frame_sized(&ctx, &mut a, vec![right(true)], size);
    frame_sized(&ctx, &mut a, vec![right(false)], size);
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(text_at(&shapes, "Open").is_some() && text_at(&shapes, "Save As…").is_some(), "the chip's menu");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_imported_contacts_files_show_on_their_page() {
    let (mut a, dir) = app("item-files-contact");
    let ctx = egui::Context::default();
    let mut tex = Textures::default();
    frame(&ctx, &mut a, vec![]);
    let c = Card {
        uid: "pst-card-1".into(),
        name: "Hélène Dupré".into(),
        emails: vec![Typed { value: "helene@studio.example".into(), kind: "work".into() }],
        extra: card_lines(),
        ..Default::default()
    };
    a.session.create_card(None, c).unwrap();
    a.run("view.module", json!({"name": "people"}));
    a.ui.person = Some("pst-card-1".into());
    let t = drawn(&ctx, &mut a, &mut tex, "contact-files");
    for want in ["CV Hélène.docx", "5 KB", "Signed offer.pdf", "8 B"] {
        assert!(has(&t, want), "{want} in {t:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// An Exchange appointment's files (R25): one kept with it, one left on the server until it's
/// opened, and the attached minutes; all three chips alike, each with its size.
#[test]
fn an_exchange_appointments_files_show_whether_here_or_on_the_server() {
    use quatrelle_engine::model::pim::StoredEvent;
    use quatrelle_engine::model::pim::files::server_file_line;
    let (mut a, dir) = app("item-files-exchange");
    let ctx = egui::Context::default();
    let mut tex = Textures::default();
    frame(&ctx, &mut a, vec![]);
    let start = (NOW / 86_400 + 1) * 86_400 + 13 * 3600;
    let account = a.session.accounts[0].config.id.clone();
    let deck = "application/vnd.openxmlformats-officedocument.presentationml.presentation";
    let extra = vec![
        format!("ATTACH;FMTTYPE=application/pdf;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"Agenda.pdf\":{}", base64_encode(&[b'%'; 3000])),
        server_file_line("Deck Q4.pptx", deck, 1_500_000, "AAMkAD-2=").unwrap(),
        format!(
            "ATTACH;FMTTYPE=message/rfc822;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"Minutes.eml\":{}",
            base64_encode(b"Subject: Minutes\r\n\r\nWe agreed.\r\n")
        ),
    ];
    let event = CalEvent { uid: "ews-files-1".into(), summary: "Budget review".into(), start, end: start + 3600, extra, ..Default::default() };
    a.session.pim_mut(&account).events.push(StoredEvent { calendar: "ews:calendar".into(), href: "R1".into(), event, ..Default::default() });
    a.ui.dialog = Dialog::open_event(&a.session, "ews-files-1", start);
    let t = drawn(&ctx, &mut a, &mut tex, "exchange-files");
    assert_eq!(a.ui.event_windows.len(), 1);
    for want in ["3 attachments", "Agenda.pdf", "3 KB", "Deck Q4.pptx", "Minutes.eml"] {
        assert!(has(&t, want), "{want} in {t:?}");
    }
    assert!(t.iter().any(|s| s.contains("MB")), "the deck's size: {t:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// An appointment's file a browser would run (an invitation's page, say) asks first, as a
/// message's attachment does: nothing is handed to another program until its Open. A PDF opens
/// at once.
#[test]
fn an_appointments_page_asks_before_it_opens() {
    let (mut a, dir) = app("item-files-risky");
    a.ui.has_files = true;
    let ctx = egui::Context::default();
    let mut tex = Textures::default();
    let size = egui::vec2(SIZE.0 as f32, SIZE.1 as f32);
    frame(&ctx, &mut a, vec![]);
    let start = (NOW / 86_400 + 1) * 86_400 + 15 * 3600;
    let extra = vec![
        format!("ATTACH;FMTTYPE=text/html;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"agenda.html\":{}", base64_encode(b"<p>Agenda</p>")),
        format!("ATTACH;FMTTYPE=application/pdf;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"Minutes.pdf\":{}", base64_encode(b"%PDF-1.4")),
    ];
    let e = CalEvent { uid: "risky-files-1".into(), summary: "Planning".into(), start, end: start + 3600, extra, ..Default::default() };
    a.session.create_event(None, None, e).unwrap();
    let item = a.session.agenda(NOW, NOW + 3 * 86_400).into_iter().find(|i| i.summary == "Planning").unwrap();
    a.ui.dialog = Dialog::open_event(&a.session, &item.uid, item.occurrence);
    drawn(&ctx, &mut a, &mut tex, "event-risky-files");
    // A double-click on the page: asked, nothing opened.
    let at = settled_text_at(&ctx, &mut a, "agenda.html", size).expect("the page's chip");
    click_at(&ctx, &mut a, at, 2, size);
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(text_at(&shapes, "Open this file?").is_some(), "asked first");
    assert!(a.session.notices.is_empty(), "nothing opened: {:?}", a.session.notices);
    // Its Open: handed to its program (the test's refuses, and that's said).
    let open = settled_text_at(&ctx, &mut a, "Open", size).expect("the question's Open");
    click_at(&ctx, &mut a, open, 1, size);
    assert!(a.session.notices.iter().any(|n| n.text.contains("agenda.html could not be opened")), "{:?}", a.session.notices);
    a.session.notices.clear();
    // The PDF: at once.
    let at = settled_text_at(&ctx, &mut a, "Minutes.pdf", size).expect("the PDF's chip");
    click_at(&ctx, &mut a, at, 2, size);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.notices.iter().any(|n| n.text.contains("Minutes.pdf could not be opened")), "{:?}", a.session.notices);
    let _ = std::fs::remove_dir_all(dir);
}
