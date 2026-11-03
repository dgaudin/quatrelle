// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: how an appointment looks in the calendar's grid (its Show As strip, the lock on a
//! private one, the title of one given without its subject).

use quatrelle_engine::AgendaItem;
use quatrelle_engine::model::ical::ShowAs;

use super::*;
use crate::calendar::{StripLook, chip_id, strip_look, title};

#[test]
fn each_show_as_has_its_strip() {
    assert_eq!(strip_look(ShowAs::Busy), StripLook::Solid);
    assert_eq!(strip_look(ShowAs::Tentative), StripLook::Hatched);
    assert_eq!(strip_look(ShowAs::Free), StripLook::Hollow);
    assert_eq!(strip_look(ShowAs::OutOfOffice), StripLook::Purple);
    assert_eq!(strip_look(ShowAs::WorkingElsewhere), StripLook::Dotted);
}

/// A colleague's private appointment comes without its subject: the grid says what it is rather
/// than drawing a blank box.
#[test]
fn a_private_appointment_without_a_subject_has_a_title() {
    let private = AgendaItem { private: true, ..Default::default() };
    assert_eq!(title(&private), "Private appointment");
    assert_eq!(title(&AgendaItem { summary: "  ".into(), ..private.clone() }), "Private appointment");
    assert_eq!(title(&AgendaItem::default()), "");
    assert_eq!(title(&AgendaItem { summary: "Lunch with Jun".into(), ..private }), "Lunch with Jun");
}

/// Every Show As, private or not, one without a subject: the Week and Month views draw each
/// appointment, nothing panics.
#[test]
fn the_week_and_month_draw_every_look() {
    let (mut a, dir) = app("calendar-look");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("view.module", json!({"name": "calendar"}));
    a.run("calendar.view", json!({"mode": "week"}));
    let shown = |a: &MailApp| {
        let (d0, d1) = a.session.cal_days();
        a.session.agenda(a.session.local_midnight(d0), a.session.local_midnight(d1))
    };
    let items = shown(&a);
    assert!(items.len() >= 5, "{items:?}");
    // Those in a calendar the user can write (the user's own, or an invitation to answer).
    let mut uids: Vec<String> = Vec::new();
    for it in items.iter().filter(|i| i.movable || i.answer.is_some()) {
        if !uids.contains(&it.uid) {
            uids.push(it.uid.clone());
        }
    }
    for (k, uid) in uids.iter().enumerate() {
        let look = ShowAs::ALL[k % ShowAs::ALL.len()];
        a.run("event.update", json!({"uid": uid, "send": false, "private": k % 2 == 0, "showAs": look.param()}));
    }
    // One given without its subject, as a colleague's private one comes from the server (no
    // command takes a blank title).
    let lunch = a.session.pim.get_mut("sample").and_then(|p| p.events.iter_mut().find(|e| e.event.summary == "Lunch with Jun")).unwrap();
    (lunch.event.summary, lunch.event.private) = (String::new(), true);
    a.session.pim_revision += 1;
    let items = shown(&a);
    for look in ShowAs::ALL {
        assert!(items.iter().any(|i| i.show_as == look), "no {look:?} in {items:?}");
    }
    assert!(items.iter().any(|i| i.private && i.summary.is_empty()), "{items:?}");
    assert!(items.iter().any(|i| !i.private));
    let drawn = |ctx: &egui::Context, items: &[AgendaItem], view: &str| {
        for it in items {
            assert!(ctx.read_response(chip_id(it)).is_some(), "{} at {} isn't drawn in the {view}", it.summary, it.start);
        }
    };
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    drawn(&ctx, &items, "week");
    // Tall enough that no day keeps some under "N more".
    a.run("calendar.view", json!({"mode": "month"}));
    for _ in 0..3 {
        frame_sized(&ctx, &mut a, vec![], egui::vec2(1280.0, 2400.0));
    }
    drawn(&ctx, &items, "month");
    let _ = std::fs::remove_dir_all(dir);
}
