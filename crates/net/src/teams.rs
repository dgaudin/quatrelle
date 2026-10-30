//! Microsoft Teams chats, read and written through Microsoft Graph: the first brick of the
//! Teams worker.
//!
//! * [`graph`]: the client for Graph's chat API — the signed-in user, chats, their members and
//!   messages, reactions, soft deletes, marking a chat read, and files shared in a chat.
//! * [`check`]: the step 0 check the owner runs on his own account (`quatrelle-cli teams
//!   check`): counts and yes/no answers only, its test messages deleted on the way out.

pub mod check;
pub mod graph;

pub use check::{CheckOptions, run_check};
pub use graph::{Me, Page, Range, Stats, TeamsClient};

#[cfg(test)]
mod tests;
