// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An answered invitation shows as its organizer meant; Graph's names for Show As.

use super::*;

fn event(show_as: ShowAs, extra: &[&str]) -> CalEvent {
    CalEvent { show_as, extra: extra.iter().map(|s| s.to_string()).collect(), ..CalEvent::default() }
}

const BUSY: &str = "X-MICROSOFT-CDO-INTENDEDSTATUS:BUSY";

#[test]
fn accepted_shows_as_the_organizer_meant() {
    for (line, want) in
        [(BUSY, ShowAs::Busy), ("X-MICROSOFT-CDO-INTENDEDSTATUS:FREE", ShowAs::Free), ("X-MICROSOFT-CDO-INTENDEDSTATUS:OOF", ShowAs::OutOfOffice)]
    {
        let mut e = event(ShowAs::Tentative, &[line]);
        show_as_answered(&mut e, "ACCEPTED");
        assert_eq!(e.show_as, want, "{line}");
        assert_eq!(e.extra, vec![line.to_string()], "the line stays, once");
    }
}

#[test]
fn accepted_without_intent_is_busy_when_it_was_tentative() {
    for (from, want) in [(ShowAs::Tentative, ShowAs::Busy), (ShowAs::Free, ShowAs::Free), (ShowAs::WorkingElsewhere, ShowAs::WorkingElsewhere)] {
        let mut e = event(from, &[]);
        show_as_answered(&mut e, "ACCEPTED");
        assert_eq!(e.show_as, want, "{from:?}");
    }
}

#[test]
fn tentatively_accepted_is_tentative_unless_free() {
    let mut e = event(ShowAs::Busy, &[]);
    show_as_answered(&mut e, "TENTATIVE");
    assert_eq!(e.show_as, ShowAs::Tentative);
    let mut e = event(ShowAs::Free, &[]);
    show_as_answered(&mut e, "TENTATIVE");
    assert_eq!(e.show_as, ShowAs::Free);
}

#[test]
fn declined_or_junk_leaves_it() {
    for p in ["DECLINED", "", "NEEDS-ACTION", "x\u{0}"] {
        let mut e = event(ShowAs::Busy, &[]);
        show_as_answered(&mut e, p);
        assert_eq!(e.show_as, ShowAs::Busy, "{p:?}");
    }
    let mut e = event(ShowAs::Tentative, &[]);
    show_as_answered(&mut e, "accepted");
    assert_eq!(e.show_as, ShowAs::Busy, "any case");
    let mut e = event(ShowAs::Busy, &[]);
    show_as_answered(&mut e, "Tentative");
    assert_eq!(e.show_as, ShowAs::Tentative, "any case");
}

#[test]
fn intended_status_with_junk() {
    assert_eq!(intended_show_as(&event(ShowAs::Busy, &["X-MICROSOFT-CDO-INTENDEDSTATUS:NONSENSE"])), None);
    assert_eq!(intended_show_as(&event(ShowAs::Busy, &[])), None);
    let e = event(ShowAs::Busy, &["X-MICROSOFT-CDO-INTENDEDSTATUS:NONSENSE", "x-microsoft-cdo-intendedstatus;X=1:free", BUSY]);
    assert_eq!(intended_show_as(&e), Some(ShowAs::Free), "the first readable line counts");
}

#[test]
fn graph_names_both_ways() {
    for x in ShowAs::ALL {
        assert_eq!(ShowAs::from_cdo(x.graph()), Some(x));
    }
    let names: Vec<_> = ShowAs::ALL.iter().map(|x| x.graph()).collect();
    assert_eq!(names, ["free", "workingElsewhere", "tentative", "busy", "oof"]);
    assert_eq!(ShowAs::from_cdo("unknown"), None);
}
