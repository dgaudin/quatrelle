// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! OpenPGP in the browser: not yet (adapted from upstream f32b54b). The page has no keychain for
//! an account's own keys, so every OpenPGP job is answered at once with why
//! ([`NOT_IN_BROWSER`]): nothing waits for an answer that won't come, and nothing to be signed or
//! encrypted goes as it is. A draft to seal is saved as it is, and the user told. Reading mail
//! signed with OpenPGP, and people's keys, are the engine's and work here as on the desktop.

use quatrelle_engine::model::job::{NOT_IN_BROWSER, PgpWork, Unavailable};

use super::{Rcs, Task, emit, enqueue};

pub(super) fn job(sh: &Rcs, id: &str, work: PgpWork) {
    match work.unavailable(id, NOT_IN_BROWSER) {
        Unavailable::Answer(e) => emit(sh, e),
        Unavailable::Saved(job, said) => {
            emit(sh, said);
            enqueue(sh, id, Task::Job(*job));
        }
    }
}
