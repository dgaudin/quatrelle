// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: Message-IDs of runs started in the same second (adapted from 77d1aac3).

use super::*;

/// Two messages sent by runs started in the same second had the same Message-ID (made from the
/// clock in seconds alone), and a mail service that has seen one drops the other as a copy.
/// Sessions started together make different ones. Each opens on a thread of its own: a new
/// thread's hash keys are drawn from the system, as a new run's are (the same thread's later
/// ones only count up, which two runs in the same second wouldn't).
#[test]
fn sessions_started_together_make_different_message_ids() {
    let open = || std::thread::spawn(|| Session::open(Some(Store::memory()), quatrelle_model::dates::Clock::new(NOW, 0)).random()).join().unwrap();
    assert_ne!(open(), open());
}

/// A front end whose platform gives Rust no randomness adds its own.
#[test]
fn mixed_in_randomness_changes_the_ids() {
    let mut a = Session::open(None, quatrelle_model::dates::Clock::new(NOW, 0));
    let mut b = Session::open(None, quatrelle_model::dates::Clock::new(NOW, 0));
    a.mix_random(1);
    b.mix_random(2);
    assert_ne!(a.random(), b.random());
}
