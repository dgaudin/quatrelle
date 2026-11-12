// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! "Work offline": nothing asked of Teams while it's on, the copy still answering; turned off,
//! the passes go on.

use std::sync::atomic::{AtomicU32, Ordering};

use quatrelle_model::pim::teams::{Picture, Status};

use super::*;

/// The outside pictures the test's stand-in for `get_public` was asked for.
static FETCHED: AtomicU32 = AtomicU32::new(0);

fn fetched(_url: &str, _max: usize) -> crate::Result<crate::http::Response> {
    FETCHED.fetch_add(1, Ordering::SeqCst);
    Ok(crate::http::Response { status: 200, body: b"GIF89a".to_vec(), ..crate::http::Response::default() })
}

/// Final review I5: "Work offline" stops Teams too.
#[test]
fn working_offline_asks_nothing_and_the_copy_still_answers() {
    let mut r = run_with(teams_with(vec![chat(CHAT, NOW - DAY, 2)]), |w| w.hooks.outside = Some(fetched));
    let from = r.events().len();
    r.w.work_offline(true);
    let asked = r.all_hits();
    r.post(CHAT, "u-ana", NOW + MINUTE, "Are you there?");
    r.w.job(Job::Open { chat: Some(CHAT.into()) });
    assert!(!r.messages(CHAT, from).is_empty(), "the chat opened is answered from the copy");
    r.w.job(Job::Picture { picture: Picture::Outside { url: "https://images.contoso.example/a.gif".into() } });
    r.turns(Duration::from_secs(70));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(r.all_hits(), asked, "nothing asked of Teams");
    assert_eq!(FETCHED.load(Ordering::SeqCst), 0, "nor of another site");
    assert!(r.events().into_iter().skip(from).any(|e| e == model::Event::Status { status: Status::Offline }), "{:?}", r.events());
    // Back online: the passes go on, the message missed meanwhile read.
    r.w.work_offline(false);
    r.turns(Duration::from_secs(11));
    assert!(ids(&r.messages(CHAT, from)).contains(&(NOW + MINUTE).to_string()));
    assert!(r.all_hits() > asked);
}
