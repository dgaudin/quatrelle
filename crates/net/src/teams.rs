// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Microsoft Teams chats, read and written through Microsoft Graph: the first brick of the
//! Teams worker.
//!
//! * [`graph`]: the client for Graph's chat API — the signed-in user, chats, their members and
//!   messages, reactions, soft deletes, marking a chat read, files shared in a chat, hosted
//!   images, photos, presence — and its answers read into the model (`chat_of`, `message_of`…),
//!   each message behind a guard of its own.
//! * [`check`]: the step 0 check a user runs on their own account (`quatrelle-cli teams
//!   check`): counts and yes/no answers only, its test messages deleted on the way out.
//! * `store`: the local copy of the chats (`Teams.quatrelle`), bound to the identity it was made
//!   for, its messages in pages.
//! * `limits`: the limiter (Microsoft's per-second limits), the approved cadences and the
//!   slow-down after a 429.

pub mod check;
pub mod graph;
// The Teams worker (next) is what asks it; until then only its tests.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod limits;
// The Teams worker (next) is what reads and writes it; until then only its tests and fuzz target.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod store;

pub use check::{CheckOptions, run_check};
pub use graph::{Me, Page, Range, Stats, TeamsClient, chat_of, member_of, message_of, presence_of};
#[doc(hidden)]
pub use store::fuzz_teams_copy;

#[cfg(test)]
mod tests;
