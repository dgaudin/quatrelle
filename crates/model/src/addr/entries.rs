//! The typed entry of a contacts field: an address with its kind, what a field accepts, and the one
//! splitter that cuts typed text into entries.

use serde::{Deserialize, Serialize};

use super::{EmailAddress, is_valid_email};

/// What an entry is, as the directory says (`MailboxType` of Exchange's `ResolveNames`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AddresseeKind {
    Person,
    DistributionList,
    ContactGroup,
    Room,
    Equipment,
    SharedMailbox,
    PublicFolder,
    Microsoft365Group,
    #[default]
    Unknown,
}

impl AddresseeKind {
    const ALL: [AddresseeKind; 9] = [
        AddresseeKind::Person,
        AddresseeKind::DistributionList,
        AddresseeKind::ContactGroup,
        AddresseeKind::Room,
        AddresseeKind::Equipment,
        AddresseeKind::SharedMailbox,
        AddresseeKind::PublicFolder,
        AddresseeKind::Microsoft365Group,
        AddresseeKind::Unknown,
    ];

    /// The stable text of the kind (the serde name).
    pub fn code(self) -> &'static str {
        match self {
            AddresseeKind::Person => "person",
            AddresseeKind::DistributionList => "distributionList",
            AddresseeKind::ContactGroup => "contactGroup",
            AddresseeKind::Room => "room",
            AddresseeKind::Equipment => "equipment",
            AddresseeKind::SharedMailbox => "sharedMailbox",
            AddresseeKind::PublicFolder => "publicFolder",
            AddresseeKind::Microsoft365Group => "microsoft365Group",
            AddresseeKind::Unknown => "unknown",
        }
    }

    pub fn from_code(s: &str) -> Option<AddresseeKind> {
        Self::ALL.into_iter().find(|k| k.code() == s)
    }

    /// The kind of an Exchange `MailboxType`; one nobody documented reads as unknown.
    pub fn from_mailbox_type(s: &str) -> AddresseeKind {
        match s {
            "Mailbox" | "Contact" | "OneOff" => AddresseeKind::Person,
            "PublicDL" => AddresseeKind::DistributionList,
            "PrivateDL" => AddresseeKind::ContactGroup,
            "PublicFolder" => AddresseeKind::PublicFolder,
            "GroupMailbox" => AddresseeKind::Microsoft365Group,
            _ => AddresseeKind::Unknown,
        }
    }

    /// A list that expands into members.
    pub fn is_list(self) -> bool {
        matches!(self, AddresseeKind::DistributionList | AddresseeKind::ContactGroup)
    }
}

/// One entry of a contacts field: a name, an address, what it is, and (for a contact group, which
/// has no address) the item it stands for.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Addressee {
    pub name: String,
    pub address: String,
    #[serde(default)]
    pub kind: AddresseeKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub item_id: String,
}

impl Addressee {
    pub fn new(name: impl Into<String>, address: impl Into<String>, kind: AddresseeKind) -> Addressee {
        Addressee { name: name.into(), address: address.into(), kind, item_id: String::new() }
    }

    pub fn email(&self) -> EmailAddress {
        EmailAddress::new(self.name.clone(), self.address.clone())
    }

    /// The name if there is one, else the address.
    pub fn display(&self) -> &str {
        if self.name.trim().is_empty() { &self.address } else { self.name.trim() }
    }

    /// The entry as a field writes it. A contact group (no address) is written by its reference,
    /// `Team <group:ID>` or `<group:ID>` without a name; a group with no usable item id has no
    /// reference to write, so its name alone comes out (an empty string with no name either).
    pub fn text(&self) -> String {
        if self.kind == AddresseeKind::ContactGroup && self.address.is_empty() {
            let name = self.name.trim();
            if !valid_item_id(&self.item_id) {
                return name.to_string();
            }
            let r = group_ref(&self.item_id);
            return if name.is_empty() {
                format!("<{r}>")
            } else if name.contains([',', ';', '"', '<', '>', '@']) {
                format!("\"{}\" <{r}>", name.replace('"', "'"))
            } else {
                format!("{name} <{r}>")
            };
        }
        self.email().to_string()
    }
}

impl From<EmailAddress> for Addressee {
    fn from(a: EmailAddress) -> Addressee {
        Addressee { name: a.name, address: a.email, kind: AddresseeKind::Unknown, item_id: String::new() }
    }
}

const GROUP_PREFIX: &str = "group:";

/// The textual form of a contact group, inside `<…>` (`Team <group:AAMk…>`).
pub fn group_ref(item_id: &str) -> String {
    format!("{GROUP_PREFIX}{item_id}")
}

/// The item id of a group reference: 1 to 512 Exchange item id characters, nothing else.
pub fn parse_group_ref(s: &str) -> Option<&str> {
    s.trim().strip_prefix(GROUP_PREFIX).filter(|id| valid_item_id(id))
}

/// Whether an Exchange item id is usable: 1 to 512 of the characters item ids are made of.
pub fn valid_item_id(id: &str) -> bool {
    (1..=512).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'_' | b'-'))
}

/// Whether the text is a group reference, which is never an address.
pub(super) fn is_group_ref_text(s: &str) -> bool {
    s.trim().starts_with(GROUP_PREFIX)
}

/// Everything that has an address (all but a contact group).
const MATCHED_KINDS: [AddresseeKind; 7] = [
    AddresseeKind::Person,
    AddresseeKind::DistributionList,
    AddresseeKind::Room,
    AddresseeKind::Equipment,
    AddresseeKind::SharedMailbox,
    AddresseeKind::PublicFolder,
    AddresseeKind::Microsoft365Group,
];

/// What a field takes: the kinds it accepts and whether it holds one entry or several.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accepts {
    pub kinds: &'static [AddresseeKind],
    pub one: bool,
    /// The kinds the field takes only when they were typed as a full address, never when picked
    /// by name (a person's address typed in Rooms is accepted; the same person picked is not).
    pub typed: &'static [AddresseeKind],
    /// Text that nothing matches is valid as typed: no error on it, and nothing resolved unless
    /// asked (a rule's or a Search Folder's field, which looks for text as much as for people).
    pub as_typed: bool,
}

impl Accepts {
    /// To, Cc and Bcc: everyone, several.
    pub const RECIPIENTS: Accepts = Accepts { kinds: &AddresseeKind::ALL, one: false, typed: &[], as_typed: false };

    /// A meeting's required and optional attendees: people, lists, groups and shared mailboxes.
    pub const ATTENDEES: Accepts = Accepts {
        kinds: &[
            AddresseeKind::Person,
            AddresseeKind::DistributionList,
            AddresseeKind::ContactGroup,
            AddresseeKind::Microsoft365Group,
            AddresseeKind::SharedMailbox,
        ],
        one: false,
        typed: &[],
        as_typed: false,
    };

    /// A meeting's rooms: rooms and equipment, and a person or a shared mailbox only typed in full.
    pub const ROOMS: Accepts = Accepts {
        kinds: &[AddresseeKind::Room, AddresseeKind::Equipment],
        one: false,
        typed: &[AddresseeKind::Person, AddresseeKind::SharedMailbox],
        as_typed: false,
    };

    /// The From of a message: one person, shared mailbox or list.
    pub const ONE_SENDER: Accepts = Accepts {
        kinds: &[AddresseeKind::Person, AddresseeKind::SharedMailbox, AddresseeKind::DistributionList],
        one: true,
        typed: &[],
        as_typed: false,
    };

    /// The calendars to add (Add calendar > From Address Book): people, rooms, equipment and shared
    /// mailboxes. A list has no calendar.
    pub const CALENDARS: Accepts = Accepts {
        kinds: &[AddresseeKind::Person, AddresseeKind::Room, AddresseeKind::Equipment, AddresseeKind::SharedMailbox],
        one: false,
        typed: &[],
        as_typed: false,
    };

    /// The mailbox to open (Open another mailbox): a person's, a shared one or a room's.
    pub const MAILBOX: Accepts =
        Accepts { kinds: &[AddresseeKind::Person, AddresseeKind::SharedMailbox, AddresseeKind::Room], one: true, typed: &[], as_typed: false };

    /// A rule's or a Search Folder's people, several: whatever has an address, and text as typed.
    pub const MATCHED: Accepts = Accepts { kinds: &MATCHED_KINDS, one: false, typed: &[], as_typed: true };

    /// A rule's single person (Sent only to…): the kinds of [`Accepts::MATCHED`], one entry.
    pub const MATCHED_ONE: Accepts = Accepts { kinds: &MATCHED_KINDS, one: true, typed: &[], as_typed: true };

    /// Whether the field takes the kind (`Unknown`, an address alone, always).
    pub fn takes(&self, kind: AddresseeKind) -> bool {
        Accepts::taken_by(self.kinds, kind)
    }

    /// Whether the field takes the kind picked by name or typed in full: [`Accepts::takes`] or one
    /// of the kinds it takes only typed.
    pub fn takes_typed(&self, kind: AddresseeKind) -> bool {
        self.takes(kind) || self.typed.contains(&kind)
    }

    /// Whether a field taking `kinds` (given at run time, as a command does) takes the kind: the
    /// rule of [`Accepts::takes`].
    pub fn taken_by(kinds: &[AddresseeKind], kind: AddresseeKind) -> bool {
        kind == AddresseeKind::Unknown || kinds.contains(&kind)
    }
}

/// Most entries one field holds.
pub const MAX_ENTRIES: usize = 500;

/// Cut typed text into entries: on `,` and `;` outside quotes and `<…>`, trimmed, empties dropped.
/// A comma-separated part that is no address and has no `<…>` is joined to the part after it when
/// that one has `<…>` (`Doe, John <j@x>`); a `;` never joins.
pub fn split_entries(input: &str) -> Vec<&str> {
    // (start, end, a comma follows)
    let mut raw: Vec<(usize, usize, bool)> = Vec::new();
    let (mut quoted, mut angle, mut start) = (false, false, 0);
    for (i, c) in input.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '<' if !quoted => angle = true,
            '>' if !quoted => angle = false,
            ',' | ';' if !quoted && !angle => {
                raw.push((start, i, c == ','));
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    raw.push((start, input.len(), false));

    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&(s, e, comma)) = raw.get(i) {
        i += 1;
        let part = input.get(s..e).unwrap_or_default().trim();
        if part.is_empty() {
            continue;
        }
        if comma
            && !part.contains('<')
            && !is_valid_email(part)
            && let Some(&(s2, e2, _)) = raw.get(i)
            && input.get(s2..e2).unwrap_or_default().contains('<')
        {
            i += 1;
            out.push(input.get(s..e2).unwrap_or_default().trim());
            continue;
        }
        out.push(part);
    }
    out
}

/// A field's text as a rule or a Search Folder takes it: the entries of [`split_entries`]; one that
/// reads as an address gives that address alone (`Doe, John <j@x.fr>` is `j@x.fr`), any other gives
/// itself, with one pair of surrounding `"` removed (`@proginov.com`, `"Doe, John"`).
pub fn matched(text: &str) -> Vec<String> {
    split_entries(text)
        .into_iter()
        .map(|e| match super::parse_one(e) {
            Some(a) => a.email,
            None => e.strip_prefix('"').and_then(|r| r.strip_suffix('"')).unwrap_or(e).to_string(),
        })
        .collect()
}

/// The reverse of [`matched`]: the pieces joined by `"; "`, one holding `,` or `;` written `"…"`
/// (its own `"` removed), so [`matched`] reads the same pieces back.
pub fn matched_text(pieces: &[String]) -> String {
    pieces.iter().map(|p| if p.contains([',', ';']) { format!("\"{}\"", p.replace('"', "")) } else { p.clone() }).collect::<Vec<_>>().join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addr::parse_list;

    #[test]
    fn a_display_name_with_a_comma_stays_one_entry() {
        assert_eq!(split_entries("Doe, John <j@x>"), ["Doe, John <j@x>"]);
        assert_eq!(split_entries("doe@x, John <j@x>"), ["doe@x", "John <j@x>"]);
        assert_eq!(split_entries("a@x,b@y"), ["a@x", "b@y"]);
        assert_eq!(split_entries("\"Doe, John\" <j@x>; Zoe"), ["\"Doe, John\" <j@x>", "Zoe"]);
        assert_eq!(split_entries("Zoe; nobody here, dupont"), ["Zoe", "nobody here", "dupont"]);
        assert_eq!(split_entries(" ; , "), Vec::<&str>::new());
        assert_eq!(parse_list("Doe, John <j@x>").unwrap(), [EmailAddress::new("Doe, John", "j@x")]);
        assert_eq!(parse_list("Doe, John <j@x>").unwrap()[0].to_string(), "\"Doe, John\" <j@x>");
    }

    #[test]
    fn kinds_come_from_mailbox_types_and_groups_have_a_text_form() {
        assert_eq!(AddresseeKind::from_mailbox_type("PrivateDL"), AddresseeKind::ContactGroup);
        assert_eq!(AddresseeKind::from_mailbox_type("GroupMailbox"), AddresseeKind::Microsoft365Group);
        assert_eq!(AddresseeKind::from_mailbox_type("Something2029"), AddresseeKind::Unknown);
        for k in [AddresseeKind::Person, AddresseeKind::Room, AddresseeKind::Unknown] {
            assert_eq!(AddresseeKind::from_code(k.code()), Some(k));
        }
        let g = Addressee { name: "Team".into(), address: String::new(), kind: AddresseeKind::ContactGroup, item_id: "AAMk=".into() };
        assert_eq!(g.text(), "Team <group:AAMk=>");
        assert_eq!(parse_group_ref("group:AAMk="), Some("AAMk="));
        assert!(parse_list(&g.text()).is_err(), "a group isn't an address to send to");
        assert!(Accepts::RECIPIENTS.takes(AddresseeKind::Unknown) && Accepts::RECIPIENTS.takes(AddresseeKind::ContactGroup));
        assert_eq!(Addressee::from(EmailAddress::new("Zoe", "z@x")).kind, AddresseeKind::Unknown);
    }

    #[test]
    fn places_take_what_the_spec_says() {
        use AddresseeKind::*;
        assert!(Accepts::ROOMS.takes(Room) && Accepts::ROOMS.takes(Equipment));
        assert!(!Accepts::ROOMS.takes(Person));
        assert!(Accepts::ROOMS.takes_typed(Person) && Accepts::ROOMS.takes_typed(SharedMailbox));
        assert!(!Accepts::ROOMS.takes_typed(DistributionList));
        assert!(Accepts::ATTENDEES.takes(SharedMailbox) && Accepts::ATTENDEES.takes(Microsoft365Group));
        assert!(!Accepts::ATTENDEES.takes(PublicFolder) && !Accepts::ATTENDEES.takes(Room));
        assert!(!Accepts::ATTENDEES.takes_typed(Room), "attendees type nothing more");
        assert_eq!([Accepts::ONE_SENDER.one, Accepts::ATTENDEES.one, Accepts::ROOMS.one], [true, false, false]);
        assert!(!Accepts::ONE_SENDER.takes(Room) && Accepts::ONE_SENDER.takes(DistributionList));
        assert!(!Accepts::ONE_SENDER.takes(ContactGroup));
        for a in [Accepts::RECIPIENTS, Accepts::ATTENDEES, Accepts::ROOMS, Accepts::ONE_SENDER] {
            assert!(a.takes(Unknown) && a.takes_typed(Unknown));
        }
    }

    #[test]
    fn a_group_reference_takes_only_item_id_characters() {
        assert_eq!(parse_group_ref("group:"), None);
        assert_eq!(parse_group_ref("group:x@y.z"), None);
        assert_eq!(parse_group_ref("group:AA Mk"), None);
        assert_eq!(parse_group_ref("group:a,b"), None);
        assert_eq!(parse_group_ref("group:a<b"), None);
        assert_eq!(parse_group_ref(&format!("group:{}", "A".repeat(512))).map(str::len), Some(512));
        assert_eq!(parse_group_ref(&format!("group:{}", "A".repeat(513))), None);
        assert!(parse_list("Team <group:x@y.z>").is_err());
        assert!(!is_valid_email("group:x@y.z"));
        for id in ["AAMk=", "a+b/c_d-e", "x"] {
            let g = Addressee { name: "T".into(), address: String::new(), kind: AddresseeKind::ContactGroup, item_id: id.into() };
            let text = g.text();
            assert!(parse_list(&text).is_err() && !is_valid_email(&text));
            assert_eq!(parse_group_ref(text.trim_start_matches("T <").trim_end_matches('>')), Some(id));
        }
    }

    #[test]
    fn a_group_text_reads_back_as_a_group() {
        let mut g = Addressee { name: String::new(), address: String::new(), kind: AddresseeKind::ContactGroup, item_id: "AAMk=".into() };
        assert_eq!(g.text(), "<group:AAMk=>");
        g.item_id.clear();
        assert_eq!(g.text(), "");
        g.name = "Team".into();
        assert_eq!(g.text(), "Team");
    }

    #[test]
    fn the_join_rule_is_pinned() {
        assert_eq!(split_entries("a@x, Doe, John <j@x>"), ["a@x", "Doe, John <j@x>"]);
        assert_eq!(split_entries("Doe, John <j@x>, Zoe <z@x>"), ["Doe, John <j@x>", "Zoe <z@x>"]);
        assert_eq!(split_entries("Doe; John <j@x>"), ["Doe", "John <j@x>"]);
        assert_eq!(split_entries("Zoe <z@x"), ["Zoe <z@x"]);
        assert!(parse_list("Zoe <z@x").is_err());
    }

    #[test]
    fn kind_codes_match_serde_names() {
        for k in AddresseeKind::ALL {
            assert_eq!(serde_json::to_string(&k).unwrap(), format!("\"{}\"", k.code()));
            assert_eq!(serde_json::from_str::<AddresseeKind>(&format!("\"{}\"", k.code())).unwrap(), k);
            assert_eq!(AddresseeKind::from_code(k.code()), Some(k));
        }
    }

    #[test]
    fn the_new_places_take_what_the_spec_says() {
        use AddresseeKind::*;
        assert!(Accepts::CALENDARS.takes(Room) && Accepts::CALENDARS.takes(SharedMailbox) && Accepts::CALENDARS.takes(Equipment));
        assert!(!Accepts::CALENDARS.takes(DistributionList) && !Accepts::CALENDARS.takes(ContactGroup) && !Accepts::CALENDARS.takes(PublicFolder));
        assert!(
            Accepts::MAILBOX.one
                && Accepts::MAILBOX.takes(Room)
                && !Accepts::MAILBOX.takes(DistributionList)
                && !Accepts::MAILBOX.takes(Microsoft365Group)
        );
        const { assert!(Accepts::MATCHED.as_typed && Accepts::MATCHED_ONE.as_typed && Accepts::MATCHED_ONE.one && !Accepts::MATCHED.one) };
        assert!(Accepts::MATCHED.takes(PublicFolder) && Accepts::MATCHED.takes(DistributionList) && !Accepts::MATCHED.takes(ContactGroup));
        const { assert!(!Accepts::RECIPIENTS.as_typed && !Accepts::ROOMS.as_typed) };
    }

    #[test]
    fn a_rule_takes_addresses_alone_and_text_as_typed() {
        assert_eq!(matched("Doe, John <j@x.fr>; @proginov.com"), vec!["j@x.fr", "@proginov.com"]);
        assert_eq!(matched("\"Doe, John\"; Dupont"), vec!["Doe, John", "Dupont"]);
        assert_eq!(matched(" ; "), Vec::<String>::new());
        let v: Vec<String> = vec!["Doe, John".into(), "@proginov.com".into(), "j@x.fr".into(), "a;b".into()];
        assert_eq!(matched_text(&v), "\"Doe, John\"; @proginov.com; j@x.fr; \"a;b\"");
        assert_eq!(matched(&matched_text(&v)), v);
    }
}
