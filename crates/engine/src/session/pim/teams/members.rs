// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The open chat's members, each found by their id in one step: the pane looks people up for
//! every message, mention and reaction it draws, every frame, in a chat of up to
//! [`MAX_MEMBERS`] members. An id is matched whatever its capitals (Graph writes a user's id in
//! either), the same way everywhere: a sender's name, a mention, "External", the user's own.

use quatrelle_model::pim::teams::{MAX_MEMBERS, Member, Sender, is_internal};

use super::*;

/// A chat's members in the order the worker gave them, and each one's place by their id in
/// small letters (the first of two with the same id).
#[derive(Clone, Debug, Default)]
pub struct ChatMembers {
    list: Vec<Member>,
    by_id: HashMap<String, usize>,
}

impl ChatMembers {
    /// The member with this id, whatever its capitals.
    pub fn get(&self, id: &str) -> Option<&Member> {
        self.by_id.get(&id.to_ascii_lowercase()).and_then(|&i| self.list.get(i))
    }

    /// The members, in the order the worker gave them.
    pub fn as_slice(&self) -> &[Member] {
        &self.list
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Member> {
        self.list.iter()
    }

    /// Whether a message's sender is of the organisation `tenant`: by their membership alone, as
    /// the model's `sender_internal` judges them. One the members don't have (a guest who left,
    /// anyone while the members aren't known) is outside, whatever the message says of them.
    pub fn sender_internal(&self, sender: &Sender, tenant: &str) -> bool {
        match sender {
            Sender::User { id, .. } => self.get(id).is_some_and(|m| is_internal(m, tenant)),
            _ => false,
        }
    }

    pub(super) fn clear(&mut self) {
        self.list.clear();
        self.by_id.clear();
    }

    /// Members added, at most [`MAX_MEMBERS`] in all.
    pub(super) fn extend(&mut self, members: impl IntoIterator<Item = Member>) {
        for m in members {
            if self.list.len() >= MAX_MEMBERS {
                break;
            }
            self.by_id.entry(m.user_id.to_ascii_lowercase()).or_insert(self.list.len());
            self.list.push(m);
        }
    }
}

impl FromIterator<Member> for ChatMembers {
    fn from_iter<I: IntoIterator<Item = Member>>(members: I) -> ChatMembers {
        let mut out = ChatMembers::default();
        out.extend(members);
        out
    }
}
