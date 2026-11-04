// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The Webex rooms the Webex Meeting button's menu offers (File › Options › Calendar): each a
//! name, a personal room's link, and free lines (meeting number, video address, dial-in
//! numbers…) copied into the notes after the link. Two rooms never share a link (ASCII case
//! aside); the single room of 1.0 becomes the first, unnamed.

use quatrelle_model::ical::is_webex_link;

use super::*;

/// The most rooms kept.
pub const WEBEX_ROOMS_MAX: usize = 20;
/// The longest room name, in characters.
pub const WEBEX_ROOM_NAME_MAX: usize = 80;
/// The most free lines a room keeps.
pub const WEBEX_ROOM_LINES_MAX: usize = 20;
/// The longest free line, in characters.
pub const WEBEX_ROOM_LINE_MAX: usize = 200;

/// A Webex room the Webex Meeting button puts in an appointment.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebexRoom {
    /// Its name in the menu; empty for the room carried over from 1.0's single one (shown « Ma
    /// salle personnelle » until it's named).
    pub name: String,
    /// A Webex personal room's link (`https://<site>.webex.com/meet/<name>`).
    pub url: String,
    /// Its free lines, copied into the notes after the link.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<String>,
}

/// A name or a line as kept: no control character, no space at either end.
fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string()
}

/// `text` cleaned and cut to `max` characters (then trimmed again: a cut leaves no space at the
/// end).
fn cut(text: &str, max: usize) -> String {
    clean(text).chars().take(max).collect::<String>().trim_end().to_string()
}

/// The place of the room with exactly this link.
fn position(rooms: &[WebexRoom], link: &str) -> Option<usize> {
    rooms.iter().position(|r| r.url == link)
}

fn refused(why: impl Into<String>) -> CmdError {
    CmdError::Params(why.into())
}

impl Prefs {
    /// The single room of 1.0 (`webex_room`), when it's a Webex link no room has, becomes the
    /// first room, unnamed; then the rooms are cut to what a room keeps, never refused (a
    /// settings file edited by hand): a link that isn't a Webex room's, or one already kept (ASCII
    /// case aside), left out; names and lines cut; at most [`WEBEX_ROOMS_MAX`]. Called on the
    /// settings read.
    pub fn carry_over_webex_rooms(&mut self) {
        let old = std::mem::take(&mut self.webex_room);
        let old = old.trim();
        if is_webex_link(old) && !self.webex_rooms.iter().any(|r| r.url.trim().eq_ignore_ascii_case(old)) {
            self.webex_rooms.insert(0, WebexRoom { name: String::new(), url: old.to_string(), details: Vec::new() });
        }
        let mut kept: Vec<WebexRoom> = Vec::new();
        for r in std::mem::take(&mut self.webex_rooms) {
            if kept.len() >= WEBEX_ROOMS_MAX {
                break;
            }
            let url = r.url.trim();
            if !is_webex_link(url) || kept.iter().any(|k| k.url.eq_ignore_ascii_case(url)) {
                continue;
            }
            let details: Vec<String> =
                r.details.iter().map(|l| cut(l, WEBEX_ROOM_LINE_MAX)).filter(|l| !l.is_empty()).take(WEBEX_ROOM_LINES_MAX).collect();
            kept.push(WebexRoom { name: cut(&r.name, WEBEX_ROOM_NAME_MAX), url: url.to_string(), details });
        }
        self.webex_rooms = kept;
    }

    /// Add a room, change one, or take one out (`remove`), by its link as kept (`url`, trimmed):
    /// the room whose link was `was` gets `url` as its link, in its place; else the room with the
    /// link `url` changes; with neither, a new room goes at the end, its name required. A `name` or
    /// `details` not given stays as it was. Lines lose their control characters and spaces at
    /// either end, and the blank ones go. Whatever can't be kept (not a Webex personal room's
    /// link, one another room has whatever its case, a name or a line too long, too many lines or
    /// rooms, a named room's name taken off) is refused, and nothing changes.
    pub fn set_webex_room(
        &mut self,
        url: &str,
        was: Option<&str>,
        name: Option<&str>,
        details: Option<Vec<String>>,
        remove: bool,
    ) -> Result<(), CmdError> {
        let url = url.trim();
        if !is_webex_link(url) {
            let shown: String = url.chars().take(80).collect();
            return Err(refused(format!("`{shown}` is not a Webex personal room (https://<site>.webex.com/meet/<name>)")));
        }
        if remove {
            let at = position(&self.webex_rooms, url).ok_or_else(|| refused(format!("no Webex room has the link `{url}`")))?;
            self.webex_rooms.remove(at);
            return Ok(());
        }
        let name = name.map(clean);
        if name.as_ref().is_some_and(|n| n.chars().count() > WEBEX_ROOM_NAME_MAX) {
            return Err(refused(format!("a Webex room's name is at most {WEBEX_ROOM_NAME_MAX} characters")));
        }
        // One line past the most is enough to refuse.
        let details: Option<Vec<String>> =
            details.map(|lines| lines.iter().map(|l| clean(l)).filter(|l| !l.is_empty()).take(WEBEX_ROOM_LINES_MAX + 1).collect());
        if let Some(lines) = &details {
            if lines.len() > WEBEX_ROOM_LINES_MAX {
                return Err(refused(format!("a Webex room keeps at most {WEBEX_ROOM_LINES_MAX} lines")));
            }
            if lines.iter().any(|l| l.chars().count() > WEBEX_ROOM_LINE_MAX) {
                return Err(refused(format!("a Webex room's line is at most {WEBEX_ROOM_LINE_MAX} characters")));
            }
        }
        let found = was.map(str::trim).and_then(|w| position(&self.webex_rooms, w)).or_else(|| position(&self.webex_rooms, url));
        if self.webex_rooms.iter().enumerate().any(|(i, r)| Some(i) != found && r.url.eq_ignore_ascii_case(url)) {
            return Err(refused(format!("another Webex room has the link `{url}`")));
        }
        match found.and_then(|at| self.webex_rooms.get_mut(at)) {
            Some(room) => {
                if let Some(n) = name {
                    if n.is_empty() && !room.name.is_empty() {
                        return Err(refused("a Webex room keeps its name"));
                    }
                    room.name = n;
                }
                room.url = url.to_string();
                if let Some(lines) = details {
                    room.details = lines;
                }
            }
            None => {
                let name = name.filter(|n| !n.is_empty()).ok_or_else(|| refused("a new Webex room needs a name"))?;
                if self.webex_rooms.len() >= WEBEX_ROOMS_MAX {
                    return Err(refused(format!("at most {WEBEX_ROOMS_MAX} Webex rooms")));
                }
                self.webex_rooms.push(WebexRoom { name, url: url.to_string(), details: details.unwrap_or_default() });
            }
        }
        Ok(())
    }
}
