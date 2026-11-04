// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A Webex room in an appointment (File › Options › Calendar keeps them): its block at the end of
//! the notes, found again as Quatrelle wrote it, in whichever language the interface had then, to
//! give way to another room's or a scheduled meeting's; and the conference it's saved with.

use quatrelle_engine::WebexRoom;
use quatrelle_engine::model::ical::Conference;

use super::*;

/// A Webex room in the appointment, as Webex's own add-in puts one: at the end of the notes the
/// line before the link, the link and the room's lines, in place of any room's block there; its
/// link as the place when that's empty or another room's (one typed is kept); Join; its
/// conference. Nothing while the appointment has its scheduled Webex meeting.
#[cfg_attr(not(test), expect(dead_code, reason = "the Webex Meeting button's menu, next, adds its rooms with it"))]
pub(crate) fn add_room(f: &mut EventForm, room: &WebexRoom, rooms: &[WebexRoom]) {
    if f.webex_id.is_some() {
        return;
    }
    let url = room.url.trim();
    let mut block = vec![tl!("Join the Webex meeting:").to_string(), url.to_string()];
    block.extend(room.details.iter().cloned());
    put_block(f, &block, url, rooms);
}

/// `block` (a room's or a scheduled meeting's) at the end of the notes, any room's block there
/// taken out first; `link` as the place when that's empty or a room's link (one typed is kept);
/// Join; and the conference it's saved with, which the window now shows. A link that can't be a
/// conference leaves the one before: a room's, its link gone, is then taken out on save.
pub(super) fn put_block(f: &mut EventForm, block: &[String], link: &str, rooms: &[WebexRoom]) {
    let notes = without_room_block(&f.description, rooms);
    let notes = notes.trim_end();
    let gap = if notes.is_empty() { "" } else { "\n\n" };
    f.description = format!("{notes}{gap}{}\n", block.join("\n"));
    let place = f.location.trim();
    if place.is_empty() || room_of(rooms, place).is_some() {
        f.location = link.to_string();
    }
    f.join = Some(link.to_string());
    if let Some(c) = Conference::new(link, "Webex") {
        f.conference = Some(c);
        f.conference_shown = true;
    }
}

/// The notes without any room's block, as Quatrelle wrote it: the line before the link in any of
/// the catalogs' languages (the interface's may have changed since), a room's link on the next
/// line (ASCII case and spaces aside), then the lines after it that are still among that room's.
/// Every such block goes, with the blank lines left doubled where it was and those at the end of
/// the notes. What the room didn't write stays: a line typed, one of its lines changed in Options
/// since, a scheduled meeting's block. Notes with no block come back as they were.
pub(crate) fn without_room_block(notes: &str, rooms: &[WebexRoom]) -> String {
    if rooms.is_empty() {
        return notes.to_string();
    }
    let labels = crate::i18n::in_every_language("Join the Webex meeting:");
    let lines: Vec<&str> = notes.split('\n').collect();
    let mut kept: Vec<&str> = Vec::with_capacity(lines.len());
    let mut taken = false;
    let mut at = 0;
    while let Some(&line) = lines.get(at) {
        let Some(n) = block_at(lines.get(at..).unwrap_or_default(), &labels, rooms) else {
            kept.push(line);
            at += 1;
            continue;
        };
        taken = true;
        at += n;
        // At the start, or after a blank line: the blank lines after it would be doubled.
        if kept.last().is_none_or(|l| l.trim().is_empty()) {
            while lines.get(at).is_some_and(|l| l.trim().is_empty()) {
                at += 1;
            }
        }
    }
    if !taken {
        return notes.to_string();
    }
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept.pop();
    }
    kept.join("\n")
}

/// How many lines a room's block takes at the start of `lines`, when one starts there.
fn block_at(lines: &[&str], labels: &[String], rooms: &[WebexRoom]) -> Option<usize> {
    let first = lines.first()?.trim();
    if !labels.iter().any(|l| l.trim() == first) {
        return None;
    }
    let room = room_of(rooms, lines.get(1)?.trim())?;
    let its = |l: &str| {
        let l = l.trim();
        !l.is_empty() && room.details.iter().any(|d| d.trim() == l)
    };
    Some(2 + lines.iter().skip(2).take_while(|l| its(l)).count())
}

/// The room whose link `link` is (trimmed, ASCII case aside).
fn room_of<'a>(rooms: &'a [WebexRoom], link: &str) -> Option<&'a WebexRoom> {
    let link = link.trim();
    rooms.iter().find(|r| !link.is_empty() && r.url.trim().eq_ignore_ascii_case(link))
}

#[cfg(test)]
mod tests {
    use quatrelle_engine::model::WebexMeeting;
    use quatrelle_engine::model::job::Secret;

    use super::super::webex::take_meeting;
    use super::*;

    const ANA: &str = "https://acme.webex.com/meet/ana";
    const TEAM: &str = "https://acme.webex.com/meet/team";
    const LINK: &str = "https://acme.webex.com/acme/j.php?MTID=m1";

    fn ana() -> WebexRoom {
        WebexRoom { name: "Ana".into(), url: ANA.into(), details: vec!["N° 123".into(), "Vidéo : ana@acme.webex.com".into()] }
    }

    fn team() -> WebexRoom {
        WebexRoom { name: "Équipe".into(), url: TEAM.into(), details: Vec::new() }
    }

    /// Ana's block as the room writes it, its first line `label`.
    fn ana_block(label: &str) -> String {
        format!("{label}\n{ANA}\nN° 123\nVidéo : ana@acme.webex.com\n")
    }

    fn strings(features: &[&str]) -> Vec<String> {
        features.iter().map(|f| f.to_string()).collect()
    }

    /// The block goes whichever catalog's language its label was written in, with the blank line
    /// before it; its link is the room's whatever its case, the lines' spaces aside.
    #[test]
    fn a_rooms_block_is_found_as_it_was_written() {
        let rooms = [ana(), team()];
        for label in ["Join the Webex meeting:", "Rejoindre la réunion Webex :", "Webex 会議に参加:"] {
            let notes = format!("Ordre du jour\n\n{}", ana_block(label));
            assert_eq!(without_room_block(&notes, &rooms), "Ordre du jour", "{label}");
        }
        assert_eq!(without_room_block(&ana_block("Join the Webex meeting:"), &rooms), "");
        let shouted = "Ordre du jour\n\n Join the Webex meeting: \r\nHTTPS://Acme.Webex.com/meet/ANA \r\n  N° 123\r\n";
        assert_eq!(without_room_block(shouted, &rooms), "Ordre du jour");
        // At the start, or between two paragraphs: no blank line left doubled.
        let notes = format!("{}\nMerci\n", ana_block("Join the Webex meeting:"));
        assert_eq!(without_room_block(&notes, &rooms), "Merci");
        let notes = format!("Ordre du jour\n\n{}\nMerci", ana_block("Join the Webex meeting:"));
        assert_eq!(without_room_block(&notes, &rooms), "Ordre du jour\n\nMerci");
        let notes = format!("Ordre du jour\n\n{}\nJoin the Webex meeting:\n{TEAM}\n", ana_block("Join the Webex meeting:"));
        assert_eq!(without_room_block(&notes, &rooms), "Ordre du jour", "every block goes");
    }

    /// A line the room didn't write stays: one typed after the block, one of its lines changed in
    /// Options since, a scheduled meeting's block, the label alone or before a link no room has.
    #[test]
    fn what_the_room_did_not_write_stays() {
        let rooms = [ana(), team()];
        let notes = format!("Ordre du jour\n\n{}Merci d'arriver à l'heure\n", ana_block("Join the Webex meeting:"));
        assert_eq!(without_room_block(&notes, &rooms), "Ordre du jour\n\nMerci d'arriver à l'heure");
        let edited = [WebexRoom { details: vec!["N° 456".into()], ..ana() }];
        let notes = format!("Ordre du jour\n\n{}", ana_block("Join the Webex meeting:"));
        assert_eq!(without_room_block(&notes, &edited), "Ordre du jour\n\nN° 123\nVidéo : ana@acme.webex.com", "the label and the link go");
        for notes in [
            format!("Ordre du jour\n\nJoin the Webex meeting:\n{LINK}\nMeeting number: 2623456701\n"),
            "Ordre du jour\n\nJoin the Webex meeting:\n".to_string(),
            "Ordre du jour\n\nJoin the Webex meeting:".to_string(),
            format!("Join the Webex meeting:\n\n{ANA}\n"),
            format!("Rejoindre :\n{ANA}\nN° 123\n"),
            "Join the Webex meeting:\nhttps://acme.webex.com/meet/zoe\n".to_string(),
        ] {
            assert_eq!(without_room_block(&notes, &rooms), notes);
        }
        let notes = ana_block("Join the Webex meeting:");
        assert_eq!(without_room_block(&notes, &[]), notes, "no room, no block");
    }

    /// A room's block takes the place of the one before, its link the place of the other room's;
    /// a place typed stays.
    #[test]
    fn another_room_replaces_the_first() {
        let rooms = [ana(), team()];
        let mut f = EventForm { description: "Ordre du jour\n".into(), ..Default::default() };
        add_room(&mut f, &rooms[0], &rooms);
        assert_eq!(f.description, format!("Ordre du jour\n\n{}", ana_block("Join the Webex meeting:")));
        assert_eq!((f.location.as_str(), f.join.as_deref()), (ANA, Some(ANA)));
        add_room(&mut f, &rooms[1], &rooms);
        assert_eq!(f.description, format!("Ordre du jour\n\nJoin the Webex meeting:\n{TEAM}\n"));
        assert_eq!(f.location, TEAM, "it was ana's link");
        assert_eq!(f.join.as_deref(), Some(TEAM));
        assert_eq!(f.conference, Some(Conference { url: TEAM.into(), label: "Webex".into(), features: strings(&["AUDIO", "VIDEO"]) }));
        assert!(f.conference_shown);
        add_room(&mut f, &rooms[1], &rooms);
        assert_eq!(f.description.matches(TEAM).count(), 1, "the same room once: {:?}", f.description);
        let mut f = EventForm { location: "Salle 3".into(), ..Default::default() };
        add_room(&mut f, &rooms[0], &rooms);
        add_room(&mut f, &rooms[1], &rooms);
        assert_eq!((f.location.as_str(), f.join.as_deref()), ("Salle 3", Some(TEAM)));
        assert!(!f.description.contains("ana"), "{:?}", f.description);
    }

    /// Webex's scheduled meeting takes the room's place in the notes, the place and the conference;
    /// once it's there, a room adds nothing.
    #[test]
    fn a_scheduled_meeting_replaces_a_room() {
        let rooms = [ana()];
        let m = WebexMeeting {
            id: "m1".into(),
            web_link: LINK.into(),
            meeting_number: "2623456701".into(),
            password: Secret(String::new()),
            sip_address: String::new(),
        };
        let mut f = EventForm { description: "Ordre du jour".into(), ..Default::default() };
        add_room(&mut f, &rooms[0], &rooms);
        take_meeting(&mut f, &m, &rooms);
        assert!(!f.description.contains("ana") && !f.description.contains("N° 123"), "{:?}", f.description);
        assert_eq!(f.description, format!("Ordre du jour\n\nJoin the Webex meeting:\n{LINK}\nMeeting number: 2623456701\n"));
        assert_eq!((f.location.as_str(), f.join.as_deref(), f.webex_id.as_deref()), (LINK, Some(LINK), Some("m1")));
        assert_eq!(f.conference, Some(Conference { url: LINK.into(), label: "Webex".into(), features: strings(&["AUDIO", "VIDEO"]) }));
        assert!(f.conference_shown);
        let before = (f.description.clone(), f.location.clone(), f.join.clone(), f.conference.clone());
        add_room(&mut f, &team(), &[ana(), team()]);
        assert_eq!((f.description, f.location, f.join, f.conference), before);
    }

    /// A meeting whose link can't be a conference (Webex never gives one with a space, but who
    /// knows) leaves the room's conference in the form: its link gone, it's taken out on save
    /// rather than kept for good by the engine.
    #[test]
    fn a_meeting_with_a_link_refused_takes_the_rooms_conference_out() {
        let rooms = [ana()];
        let m = WebexMeeting {
            id: "m1".into(),
            web_link: "https://acme.webex.com/acme/j.php?MTID=m 1".into(),
            meeting_number: String::new(),
            password: Secret(String::new()),
            sip_address: String::new(),
        };
        let mut f = EventForm::default();
        add_room(&mut f, &rooms[0], &rooms);
        take_meeting(&mut f, &m, &rooms);
        assert_eq!(f.conference.as_ref().map(|c| c.url.as_str()), Some(ANA), "the room's, kept");
        assert_eq!(f.params().get("conference"), Some(&Value::Null), "and taken out");
    }

    /// Saved, the conference the window opened with is left to the engine while it's unchanged and
    /// its link is in the place or the notes; once it was there and is in neither, it's taken out.
    /// One never shown there (another app's) stays. A room's goes along.
    #[test]
    fn the_conference_goes_when_its_link_is_erased() {
        const JITSI: &str = "https://jitsi.example.org/team";
        let jitsi = Conference { url: JITSI.into(), label: "Jitsi".into(), features: strings(&["VIDEO"]) };
        let mut f = EventForm {
            conference: Some(jitsi.clone()),
            conference_opened: Some(jitsi),
            conference_shown: true,
            description: format!("Rejoindre : {JITSI}\n"),
            ..Default::default()
        };
        assert!(f.params().get("conference").is_none(), "unchanged: the engine keeps it");
        f.description = "Rejoindre : plus tard".into();
        assert_eq!(f.params().get("conference"), Some(&Value::Null), "taken out");
        f.location = JITSI.into();
        assert!(f.params().get("conference").is_none(), "still in the place");
        f.location.clear();
        f.conference_shown = false;
        assert!(f.params().get("conference").is_none(), "never shown here");
        add_room(&mut f, &ana(), &[ana()]);
        assert_eq!(f.params()["conference"], json!({"url": ANA, "label": "Webex"}), "a room's");
        assert!(EventForm::default().params().get("conference").is_none());
    }
}
