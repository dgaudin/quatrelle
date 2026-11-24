// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Teams' teams and channels through Graph (v1.0), read only: the teams the user joined, a
//! team's channels, a channel's root messages (with their replies or without), a thread's
//! replies, a channel's members, and whether the old channel delta answers.
//!
//! Every id goes into the address as one segment of it ([`own_segment`]), else it is refused
//! before any request; a next page is followed only on the client's own address
//! ([`TeamsClient::own`]). The kinds Graph added after v1.0 was published (a `shared` channel)
//! are asked for as they are (`Prefer: include-unknown-enum-members`).

use super::*;

/// The most root messages, or replies, Graph gives a page.
const MAX_TOP: u32 = 50;

/// Refused before any request unless every id can be one segment of the address.
fn ids_ok(ids: &[&str]) -> Result<()> {
    if ids.iter().all(|id| own_segment(id)) { Ok(()) } else { Err(NetError::Refused("not a team's, a channel's or a message's id".into())) }
}

impl TeamsClient {
    /// A page of the teams the signed-in user is a member of; `next` is the previous page's
    /// [`Page::next`].
    pub fn joined_teams(&self, next: Option<&str>) -> Result<Page> {
        let url = match next {
            Some(n) => self.own(n)?,
            None => self.at("/me/joinedTeams"),
        };
        page(self.json("GET", &url, None, false)?)
    }

    /// A page of a team's channels (a private or a shared one only when the user is in it).
    pub fn channels(&self, team: &str, next: Option<&str>) -> Result<Page> {
        ids_ok(&[team])?;
        let url = match next {
            Some(n) => self.own(n)?,
            None => self.at(&format!("/teams/{}/channels", seg(team))),
        };
        page(self.json("GET", &url, None, true)?)
    }

    /// A page of a channel's root messages, `top` of them (1 to 50), as Graph lists them (by
    /// the last change to the thread, the newest first: no range can be asked); with `expand`,
    /// each with its replies (`$expand=replies`).
    pub fn channel_messages(&self, team: &str, channel: &str, top: u32, expand: bool, next: Option<&str>) -> Result<Page> {
        ids_ok(&[team, channel])?;
        let url = match next {
            Some(n) => self.own(n)?,
            None => {
                let top = top.clamp(1, MAX_TOP);
                let expand = if expand { "&$expand=replies" } else { "" };
                self.at(&format!("/teams/{}/channels/{}/messages?$top={top}{expand}", seg(team), seg(channel)))
            }
        };
        page(self.json("GET", &url, None, true)?)
    }

    /// A page of a thread's replies (the root message `root` of a channel's), 50 at most.
    pub fn replies(&self, team: &str, channel: &str, root: &str, next: Option<&str>) -> Result<Page> {
        ids_ok(&[team, channel, root])?;
        let url = match next {
            Some(n) => self.own(n)?,
            None => self.at(&format!("/teams/{}/channels/{}/messages/{}/replies?$top={MAX_TOP}", seg(team), seg(channel), seg(root))),
        };
        page(self.json("GET", &url, None, true)?)
    }

    /// A page of a channel's members.
    pub fn channel_members(&self, team: &str, channel: &str, next: Option<&str>) -> Result<Page> {
        ids_ok(&[team, channel])?;
        let url = match next {
            Some(n) => self.own(n)?,
            None => self.at(&format!("/teams/{}/channels/{}/members", seg(team), seg(channel))),
        };
        page(self.json("GET", &url, None, false)?)
    }

    /// Whether the old delta of a channel's messages (`…/messages/delta`, no longer documented
    /// for a user's sign-in) answers: its status, and whether the answer carries a link to go on
    /// with (none is followed). An error status is an error, said as [`status_error`] says it.
    pub fn channel_delta_status(&self, team: &str, channel: &str) -> Result<(u16, bool)> {
        ids_ok(&[team, channel])?;
        let url = self.at(&format!("/teams/{}/channels/{}/messages/delta", seg(team), seg(channel)));
        let (status, v) = self.answered(&self.prepared("GET", &url, None, true)?, true)?;
        let linked = ["@odata.nextLink", "@odata.deltaLink"].iter().any(|k| v.get(*k).is_some_and(Value::is_string));
        Ok((status, linked))
    }
}
