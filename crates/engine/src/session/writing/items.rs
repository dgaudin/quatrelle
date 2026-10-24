//! Items attached to a message being written, as Outlook's Attach Item: a message as an `.eml`,
//! a contact as a `.vcf`, an appointment as an `.ics`, chosen from what's on this computer.

use quatrelle_model::{ical, vcard};

use super::*;

/// What kind of item is attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Message,
    Contact,
    Event,
}

impl ItemKind {
    pub fn code(self) -> &'static str {
        match self {
            ItemKind::Message => "message",
            ItemKind::Contact => "contact",
            ItemKind::Event => "event",
        }
    }

    pub fn from_code(s: &str) -> Option<ItemKind> {
        [ItemKind::Message, ItemKind::Contact, ItemKind::Event].into_iter().find(|k| k.code() == s)
    }
}

/// An item to choose, as the chooser lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemChoice {
    /// The message's key ([`MessageKey::to_id`]), the contact's or the appointment's uid.
    pub id: String,
    pub title: String,
    /// Who sent the message, the contact's address, where the appointment is.
    pub detail: String,
    /// When the message came, when the appointment starts (0: no date).
    pub when: i64,
}

/// The items listed to choose from, at most (words typed find the others).
const CHOICES_MAX: usize = 300;
/// Appointments are listed from this long ago on (with words typed, all of them).
const EVENTS_BACK: i64 = 30 * 86_400;

impl Session {
    /// The items of a kind to choose from, those matching `query` in their title or detail: a
    /// folder's messages, newest first; every contact, by name; the appointments from a month
    /// ago on, soonest first (with words typed, all of them, the latest first).
    pub fn item_choices(&mut self, kind: ItemKind, account: &str, folder: &str, query: &str) -> Vec<ItemChoice> {
        let q = query.trim().to_lowercase();
        let hit = |c: &ItemChoice| q.is_empty() || c.title.to_lowercase().contains(&q) || c.detail.to_lowercase().contains(&q);
        match kind {
            ItemKind::Message => {
                let mut kept: Vec<&Summary> =
                    self.folder_state(account, folder).messages.iter().filter(|m| !m.flags.deleted).map(Arc::as_ref).collect();
                kept.sort_by_key(|m| std::cmp::Reverse(m.date));
                kept.into_iter()
                    .map(|m| ItemChoice { id: m.key.to_id(), title: m.subject.clone(), detail: m.from.display().to_string(), when: m.date })
                    .filter(hit)
                    .take(CHOICES_MAX)
                    .collect()
            }
            ItemKind::Contact => self
                .people("")
                .into_iter()
                .map(|p| ItemChoice {
                    id: p.card.uid.clone(),
                    title: p.card.display(),
                    detail: p.card.emails.first().map(|e| e.value.clone()).unwrap_or_default(),
                    when: 0,
                })
                .filter(hit)
                .take(CHOICES_MAX)
                .collect(),
            ItemKind::Event => {
                let from = if q.is_empty() { self.clock.now.saturating_sub(EVENTS_BACK) } else { i64::MIN };
                let mut found: Vec<ItemChoice> = self
                    .pim
                    .values()
                    .flat_map(|p| p.events.iter())
                    .filter(|e| e.event.recurrence_id.is_none() && (e.event.start >= from || e.event.rrule.is_some()))
                    .map(|e| ItemChoice {
                        id: e.event.uid.clone(),
                        title: e.event.summary.clone(),
                        detail: e.event.location.clone(),
                        when: e.event.start,
                    })
                    .filter(hit)
                    .collect();
                // One appointment in the user's calendar and in each colleague's that has it.
                found.sort_by(|a, b| a.id.cmp(&b.id));
                found.dedup_by(|a, b| a.id == b.id);
                if q.is_empty() {
                    found.sort_by_key(|c| c.when);
                } else {
                    found.sort_by_key(|c| std::cmp::Reverse(c.when));
                }
                found.truncate(CHOICES_MAX);
                found
            }
        }
    }

    /// An item as a file to attach: a message's source, as it came (`message/rfc822`); a
    /// contact as a vCard; an appointment, with its changed occurrences, as a calendar file.
    pub fn item_attachment(&self, kind: ItemKind, id: &str) -> Result<OutAttachment, CmdError> {
        let file = |title: &str, empty: &str, ext: &str| {
            let title = title.trim();
            mime::sanitize_filename(&format!("{}.{ext}", if title.is_empty() { empty } else { title }))
        };
        match kind {
            ItemKind::Message => {
                let key = MessageKey::from_id(id).ok_or_else(|| CmdError::Params(format!("no message `{id}`")))?;
                let raw = self
                    .store
                    .as_ref()
                    .and_then(|s| s.read_body(&key))
                    .ok_or_else(|| CmdError::Failed("this message isn't on this computer yet: open it, then attach it".into()))?;
                let subject = self.summary(&key).map(|m| m.subject.clone()).unwrap_or_default();
                Ok(OutAttachment { name: file(&subject, "message", "eml"), mime: "message/rfc822".into(), data: raw })
            }
            ItemKind::Contact => {
                let p = self.find_person(id).ok_or_else(|| CmdError::Params(format!("no contact `{id}`")))?;
                Ok(OutAttachment {
                    name: file(&p.card.display(), "contact", "vcf"),
                    mime: "text/vcard".into(),
                    data: vcard::write(&p.card).into_bytes(),
                })
            }
            ItemKind::Event => {
                let (account, master) = self.find_event(id).ok_or_else(|| CmdError::Params(format!("no appointment `{id}`")))?;
                let events: Vec<ical::CalEvent> = self
                    .pim
                    .get(account)
                    .map(|p| {
                        p.events
                            .iter()
                            .filter(|e| e.event.uid == master.event.uid && e.calendar == master.calendar)
                            .map(|e| e.event.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                let data = ical::write(&events, None, self.clock.now).into_bytes();
                Ok(OutAttachment { name: file(&master.event.summary, "appointment", "ics"), mime: "text/calendar".into(), data })
            }
        }
    }
}
