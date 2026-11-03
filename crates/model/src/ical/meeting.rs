// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! How a meeting shows in a calendar and what it asks of its guests, as Outlook sets them: Show As
//! (`TRANSP`, and Microsoft's `X-MICROSOFT-CDO-BUSYSTATUS`, which says more), Private (`CLASS`),
//! whether answers are asked for (each guest's `RSVP`) and whether new times may be proposed
//! (`X-MICROSOFT-DISALLOW-COUNTER`); and which guests are optional (`ROLE`).

use super::*;

/// How its time shows to others (Outlook's Show As).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShowAs {
    Free,
    Tentative,
    #[default]
    Busy,
    OutOfOffice,
    WorkingElsewhere,
}

impl ShowAs {
    /// Every one, in Outlook's order.
    pub const ALL: [ShowAs; 5] = [ShowAs::Free, ShowAs::WorkingElsewhere, ShowAs::Tentative, ShowAs::Busy, ShowAs::OutOfOffice];

    pub fn is_busy(&self) -> bool {
        *self == ShowAs::Busy
    }

    /// Microsoft's name for it (`X-MICROSOFT-CDO-BUSYSTATUS`).
    pub fn cdo(&self) -> &'static str {
        match self {
            ShowAs::Free => "FREE",
            ShowAs::Tentative => "TENTATIVE",
            ShowAs::Busy => "BUSY",
            ShowAs::OutOfOffice => "OOF",
            ShowAs::WorkingElsewhere => "WORKINGELSEWHERE",
        }
    }

    /// From Microsoft's name (iCalendar's or EWS's: `OOF`, `WorkingElsewhere`…), any case.
    pub fn from_cdo(s: &str) -> Option<ShowAs> {
        let s = s.trim();
        ShowAs::ALL.into_iter().find(|x| x.cdo().eq_ignore_ascii_case(s))
    }

    /// Microsoft Graph's name for it (`showAs` on an event).
    pub fn graph(&self) -> &'static str {
        match self {
            ShowAs::Free => "free",
            ShowAs::WorkingElsewhere => "workingElsewhere",
            ShowAs::Tentative => "tentative",
            ShowAs::Busy => "busy",
            ShowAs::OutOfOffice => "oof",
        }
    }

    /// The command parameter's name for it (`free`, `outOfOffice`…).
    pub fn param(&self) -> &'static str {
        match self {
            ShowAs::Free => "free",
            ShowAs::Tentative => "tentative",
            ShowAs::Busy => "busy",
            ShowAs::OutOfOffice => "outOfOffice",
            ShowAs::WorkingElsewhere => "workingElsewhere",
        }
    }

    pub fn from_param(s: &str) -> Option<ShowAs> {
        ShowAs::ALL.into_iter().find(|x| x.param().eq_ignore_ascii_case(s.trim()))
    }
}

/// What the organizer meant the time to show as for a guest who accepts
/// (`X-MICROSOFT-CDO-INTENDEDSTATUS`): the first such line among the extra lines that reads.
pub fn intended_show_as(e: &CalEvent) -> Option<ShowAs> {
    e.extra.iter().take(400).find_map(|l| {
        let (head, value) = l.split_once(':')?;
        let name = head.split(';').next().unwrap_or("").trim();
        if !name.eq_ignore_ascii_case("X-MICROSOFT-CDO-INTENDEDSTATUS") {
            return None;
        }
        ShowAs::from_cdo(value)
    })
}

/// A guest answered (`partstat`, any case). Outlook sends the guest's copy tentative until it is
/// answered, so accepting turns it into what the organizer meant (else Busy); accepting
/// tentatively makes it Tentative unless it shows Free; anything else leaves it as it is.
pub fn show_as_answered(e: &mut CalEvent, partstat: &str) {
    let p = partstat.trim();
    if p.eq_ignore_ascii_case("ACCEPTED") {
        if let Some(intent) = intended_show_as(e) {
            e.show_as = intent;
        } else if e.show_as == ShowAs::Tentative {
            e.show_as = ShowAs::Busy;
        }
    } else if p.eq_ignore_ascii_case("TENTATIVE") && e.show_as != ShowAs::Free {
        e.show_as = ShowAs::Tentative;
    }
}

/// Which kind of private something is. The lock shows one way, but each has its own word, kept
/// through a change so that editing a title doesn't change what it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrivateKind {
    #[default]
    Private,
    Confidential,
    /// Exchange's Personal, which iCalendar has no word for: `CLASS:X-PERSONAL`.
    Personal,
}

impl PrivateKind {
    /// Plain Private, the default.
    pub fn is_plain(&self) -> bool {
        *self == PrivateKind::Private
    }

    /// The kind a `CLASS` names; `None` for one that isn't private.
    pub fn from_class(value: &str) -> Option<PrivateKind> {
        match value.trim().to_ascii_uppercase().as_str() {
            "PRIVATE" => Some(PrivateKind::Private),
            "CONFIDENTIAL" => Some(PrivateKind::Confidential),
            "X-PERSONAL" => Some(PrivateKind::Personal),
            _ => None,
        }
    }

    /// Its `CLASS`.
    pub fn class(self) -> &'static str {
        match self {
            PrivateKind::Private => "PRIVATE",
            PrivateKind::Confidential => "CONFIDENTIAL",
            PrivateKind::Personal => "X-PERSONAL",
        }
    }

    /// The kind an Exchange `Sensitivity` names; `None` for `Normal`.
    pub fn from_sensitivity(value: &str) -> Option<PrivateKind> {
        [PrivateKind::Private, PrivateKind::Confidential, PrivateKind::Personal]
            .into_iter()
            .find(|k| k.sensitivity().eq_ignore_ascii_case(value.trim()))
    }

    /// Its Exchange `Sensitivity`.
    pub fn sensitivity(self) -> &'static str {
        match self {
            PrivateKind::Private => "Private",
            PrivateKind::Confidential => "Confidential",
            PrivateKind::Personal => "Personal",
        }
    }
}

/// An optional guest (`ROLE=OPT-PARTICIPANT`), Outlook's Optional line.
pub const OPTIONAL: &str = "OPT-PARTICIPANT";

impl Attendee {
    /// Invited as optional rather than required.
    pub fn is_optional(&self) -> bool {
        self.role == OPTIONAL
    }
}

/// The role a guest's `ROLE` gives, as kept (a required one, the default, is kept empty).
pub(super) fn role_of(param: Option<&str>) -> String {
    match param.map(|r| r.trim().to_ascii_uppercase()) {
        Some(r) if matches!(r.as_str(), "OPT-PARTICIPANT" | "NON-PARTICIPANT" | "CHAIR") => r,
        _ => String::new(),
    }
}

/// Read one of the properties this module knows into `e`; `false` for any other. `said`: the
/// event's Microsoft status was read already (it says more than `TRANSP`, whichever comes first).
pub(super) fn read(e: &mut CalEvent, name: &str, value: &str, said: &mut bool) -> bool {
    match name {
        "X-MICROSOFT-CDO-BUSYSTATUS" => {
            if let Some(s) = ShowAs::from_cdo(value) {
                e.show_as = s;
                *said = true;
            }
        }
        "TRANSP" => {
            if !*said && value.trim().eq_ignore_ascii_case("TRANSPARENT") {
                e.show_as = ShowAs::Free;
            }
        }
        "CLASS" => {
            let kind = PrivateKind::from_class(value);
            e.private = kind.is_some();
            e.private_kind = kind.unwrap_or_default();
        }
        "X-MICROSOFT-DISALLOW-COUNTER" => e.no_new_times = value.trim().eq_ignore_ascii_case("TRUE"),
        "X-QUATRELLE-WEBEX-ID" => e.webex_id = Some(value.trim()).filter(|v| valid_webex_id(v)).map(str::to_string),
        _ => return false,
    }
    true
}

/// A Webex meeting id that is safe to keep and to put in an address: 1 to 200 ASCII letters,
/// digits, `_` and `-`.
pub fn valid_webex_id(id: &str) -> bool {
    (1..=200).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Its lines for what this module knows (none for a busy, public event that takes answers and
/// new times, the defaults).
pub(super) fn lines(e: &CalEvent) -> Vec<String> {
    let mut out = Vec::new();
    if e.show_as == ShowAs::Free {
        out.push("TRANSP:TRANSPARENT".to_string());
    }
    if !e.show_as.is_busy() {
        out.push(format!("X-MICROSOFT-CDO-BUSYSTATUS:{}", e.show_as.cdo()));
    }
    if e.private {
        out.push(format!("CLASS:{}", e.private_kind.class()));
    }
    if e.no_new_times {
        out.push("X-MICROSOFT-DISALLOW-COUNTER:TRUE".to_string());
    }
    if let Some(id) = e.webex_id.as_deref().filter(|id| valid_webex_id(id)) {
        out.push(format!("X-QUATRELLE-WEBEX-ID:{id}"));
    }
    out
}
