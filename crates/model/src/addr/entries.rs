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

    /// The entry as a field writes it: a contact group (no address) by its reference.
    pub fn text(&self) -> String {
        if self.kind == AddresseeKind::ContactGroup && self.address.is_empty() {
            EmailAddress::new(self.name.clone(), group_ref(&self.item_id)).to_string()
        } else {
            self.email().to_string()
        }
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

pub fn parse_group_ref(s: &str) -> Option<&str> {
    s.trim().strip_prefix(GROUP_PREFIX)
}

/// What a field takes: the kinds it accepts and whether it holds one entry or several.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accepts {
    pub kinds: &'static [AddresseeKind],
    pub one: bool,
}

impl Accepts {
    /// To, Cc and Bcc: everyone, several.
    pub const RECIPIENTS: Accepts = Accepts { kinds: &AddresseeKind::ALL, one: false };

    /// Whether the field takes the kind (`Unknown`, an address alone, always).
    pub fn takes(&self, kind: AddresseeKind) -> bool {
        kind == AddresseeKind::Unknown || self.kinds.contains(&kind)
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
}
