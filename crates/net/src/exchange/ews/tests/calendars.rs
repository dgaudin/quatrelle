// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! EWS: a calendar made, renamed or deleted from here is refused, nothing sent, while Exchange's
//! other calendars of the account's own aren't read (one made would never show). The engine
//! doesn't ask; one kept in an older journal goes, said.

use super::*;
use crate::fakes::fake_ews::{self, Data};

#[test]
fn a_calendar_change_is_refused_and_nothing_sent() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = fake_ews::serve(d);
    let user = format!("{}\\{}", fake_ews::DOMAIN, fake_ews::USER);
    let c = Client::connect(&server.url, &user, fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    list_folders(&c, &mut st, "a1").unwrap();
    let before = server.data.lock().unwrap().calls.len();
    for op in [
        Op::CreateCalendar { name: "Garden club".into() },
        Op::RenameCalendar { calendar: "AAMkTrips=".into(), name: "Travel".into() },
        Op::DeleteCalendar { calendar: "AAMkTrips=".into() },
    ] {
        assert!(matches!(apply_remote(&c, &st, &op), Err(NetError::No(_))), "{op:?}");
    }
    assert_eq!(server.data.lock().unwrap().calls.len(), before, "nothing sent");
}
