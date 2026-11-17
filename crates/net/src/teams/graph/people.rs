// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! People through Graph: the organisation's directory searched for a new conversation, and the
//! conversation created.
//!
//! What is typed reaches the directory as one quoted term ([`search_term`]); what the directory
//! answers is a server's: its people are kept by [`person_of`] only (an object id, a name on one
//! line, an address without white space), [`PEOPLE_FOUND`] at most. A member is bound into a
//! creation's request only by an object id, which can't change the address it names.

use quatrelle_model::pim::teams::{
    ADDRESS_MAX, NEW_CHAT_PEOPLE, PEOPLE_FOUND, PEOPLE_QUERY_CHARS, Person, is_bidi, is_object_id, one_line, person_ok,
};

use super::*;

/// The most characters of what was typed looked at for a search.
const QUERY_READ: usize = PEOPLE_QUERY_CHARS * 16;

/// What a directory search sends of what was typed: its words on one line, without quotes,
/// backslashes, control or bidi characters, at most [`PEOPLE_QUERY_CHARS`] characters of them;
/// empty when nothing is left.
pub fn search_term(q: &str) -> String {
    let kept: String = q
        .chars()
        .take(QUERY_READ)
        .filter_map(|c| match c {
            c if c.is_whitespace() => Some(' '),
            '"' | '\\' => None,
            c if c.is_control() || is_bidi(c) => None,
            c => Some(c),
        })
        .collect();
    let words = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    words.chars().take(PEOPLE_QUERY_CHARS).collect::<String>().trim_end().to_string()
}

/// A person of the directory (Graph's `user`): an object id; a name on one line ([`one_line`] of
/// `displayName`, else of the address); an address (`mail`, else `userPrincipalName`) trimmed and
/// without control or bidi characters; a guest when `userType` says `Guest`. `None` for one that
/// isn't [`person_ok`].
pub fn person_of(v: &Value) -> Option<Person> {
    let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default();
    let id = text("id");
    if !is_object_id(id) {
        return None;
    }
    let address = [text("mail"), text("userPrincipalName")].into_iter().map(address_of).find(|a| !a.is_empty()).unwrap_or_default();
    let name = match one_line(text("displayName")) {
        n if n.is_empty() => one_line(&address),
        n => n,
    };
    let p = Person { id: id.to_string(), name, address, guest: text("userType").eq_ignore_ascii_case("guest") };
    person_ok(&p).then_some(p)
}

/// An address as the directory gives it: trimmed, without control or bidi characters (a
/// character past [`ADDRESS_MAX`] kept, for [`person_ok`] to refuse).
fn address_of(s: &str) -> String {
    s.trim().chars().filter(|c| !c.is_control() && !is_bidi(*c)).take(ADDRESS_MAX + 1).collect()
}

impl TeamsClient {
    /// The people of the directory whose name or address matches what was typed: Graph's
    /// `$search` of one quoted term ([`search_term`]) on `displayName` and `mail`, with
    /// `$count=true` and `ConsistencyLevel: eventual` as Graph wants them; each person read by
    /// [`person_of`] (one that isn't left out), each once, [`PEOPLE_FOUND`] at most. Nothing left
    /// to search asks nothing.
    pub fn users_search(&self, query: &str) -> Result<Vec<Person>> {
        let term = search_term(query);
        if term.is_empty() {
            return Ok(Vec::new());
        }
        let search = http::pct(&format!("\"displayName:{term}\" OR \"mail:{term}\""));
        let url = self.at(&format!("/users?$search={search}&$count=true&$top={PEOPLE_FOUND}&$select=id,displayName,mail,userPrincipalName,userType"));
        let req = self.prepared("GET", &url, None, false)?.header("ConsistencyLevel", "eventual");
        let (_, v) = self.answered(&req, true)?;
        let mut seen = HashSet::new();
        Ok(page(v)?.items.iter().filter_map(person_of).filter(|p| seen.insert(p.id.to_ascii_lowercase())).take(PEOPLE_FOUND).collect())
    }

    /// Create a conversation of the signed-in user (`me`) with these people, as Graph documents
    /// it: one person a one-to-one chat (Graph gives the one already there), more a group chat
    /// named `topic` when it says something; each member bound by their object id, the user and
    /// the organisation's people as owners, its guests as guests. Refused before any request
    /// unless `me` and every person's id are object ids, the people 1 to [`NEW_CHAT_PEOPLE`],
    /// each once and none the user. A creation isn't repeated on a 503: a group made twice is two.
    pub fn create_chat(&self, me: &str, people: &[Person], topic: Option<&str>) -> Result<Value> {
        let mut ids = HashSet::from([me.to_ascii_lowercase()]);
        let distinct = people.iter().all(|p| is_object_id(&p.id) && ids.insert(p.id.to_ascii_lowercase()));
        if !is_object_id(me) || people.is_empty() || people.len() > NEW_CHAT_PEOPLE || !distinct {
            return Err(NetError::Refused("not people a conversation can be created with".into()));
        }
        let member = |id: &str, role: &str| json!({"@odata.type": "#microsoft.graph.aadUserConversationMember", "roles": [role], "user@odata.bind": format!("{}/users('{id}')", self.base)});
        let mut members = vec![member(me, "owner")];
        members.extend(people.iter().map(|p| member(&p.id, if p.guest { "guest" } else { "owner" })));
        let mut body = json!({"chatType": if people.len() == 1 { "oneOnOne" } else { "group" }, "members": members});
        if people.len() > 1
            && let Some(topic) = topic.filter(|t| !t.trim().is_empty())
        {
            body["topic"] = json!(topic);
        }
        self.request("POST", &self.at("/chats"), Some(&body), false, false)
    }
}
