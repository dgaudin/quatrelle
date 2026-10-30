//! Microsoft Teams chats, read and written through Microsoft Graph: the first brick of the
//! Teams worker.
//!
//! * [`graph`]: the client for Graph's chat API — the signed-in user, chats, their members and
//!   messages, reactions, soft deletes, marking a chat read, and files shared in a chat.

pub mod graph;

pub use graph::{Me, Page, Range, Stats, TeamsClient};

#[cfg(test)]
mod tests;
