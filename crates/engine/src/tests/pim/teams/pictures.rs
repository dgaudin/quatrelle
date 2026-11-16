// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a picture the worker says can't be had (R39): kept as asked (never asked again
//! this session), handed to the window once, at most [`PICTURES_MAX`] waiting.

use super::*;
use crate::PICTURES_MAX;

fn hosted(i: usize) -> Picture {
    Picture::Hosted { chat: "19:chat1@thread.v2".into(), message: format!("m{i}"), hosted: "h1".into() }
}

fn ask(s: &mut Session, p: &Picture) {
    let params = match p {
        Picture::Hosted { chat, message, hosted } => json!({"kind": "hosted", "chat": chat, "message": message, "hosted": hosted}),
        Picture::Photo { user } => json!({"kind": "photo", "user": user}),
        Picture::Outside { url } => json!({"kind": "outside", "url": url}),
    };
    s.run("teams.picture", &params).unwrap();
}

fn no_picture(s: &mut Session, p: &Picture) {
    s.apply(teams_event(teams::Event::NoPicture { picture: p.clone() }));
}

#[test]
fn a_failed_picture_is_kept_asked_and_handed_to_the_window() {
    let mut s = signed_in(vec![row(1, 10)]);
    let p = hosted(1);
    ask(&mut s, &p);
    assert_eq!(teams_jobs(&mut s).len(), 1);
    no_picture(&mut s, &p);
    no_picture(&mut s, &p);
    assert_eq!(s.teams.take_no_pictures(), vec![p.clone()], "once");
    assert!(s.teams.take_no_pictures().is_empty());
    // Never asked again this session, the window having taken it or not.
    ask(&mut s, &p);
    assert!(teams_jobs(&mut s).is_empty());
    assert!(s.teams.is_asked(&p));
    // One never asked for isn't taken.
    no_picture(&mut s, &hosted(2));
    assert!(s.teams.take_no_pictures().is_empty());
    // An outside one: asked once while waiting, then said, and never asked again.
    let url = Picture::Outside { url: "https://pictures.example/a.png".into() };
    ask(&mut s, &url);
    ask(&mut s, &url);
    assert_eq!(teams_jobs(&mut s).len(), 1, "a double click asks once");
    assert!(s.teams.is_asked(&url));
    no_picture(&mut s, &url);
    assert_eq!(s.teams.take_no_pictures(), vec![url.clone()]);
    ask(&mut s, &url);
    assert!(teams_jobs(&mut s).is_empty());
}

#[test]
fn the_failed_pictures_waiting_for_the_window_are_bounded() {
    let mut s = signed_in(vec![row(1, 10)]);
    teams_jobs(&mut s);
    for i in 0..PICTURES_MAX + 6 {
        let p = hosted(i);
        ask(&mut s, &p);
        no_picture(&mut s, &p);
    }
    let taken = s.teams.take_no_pictures();
    assert_eq!(taken.len(), PICTURES_MAX);
    assert_eq!(taken.first(), Some(&hosted(6)), "the oldest dropped");
    // One dropped unseen may be asked again (the worker answers at once).
    assert!(!s.teams.is_asked(&hosted(0)));
    ask(&mut s, &hosted(0));
    assert_eq!(teams_jobs(&mut s).len(), PICTURES_MAX + 6 + 1);
}
