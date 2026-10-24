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
        "CLASS" => e.private = matches!(value.trim().to_ascii_uppercase().as_str(), "PRIVATE" | "CONFIDENTIAL"),
        "X-MICROSOFT-DISALLOW-COUNTER" => e.no_new_times = value.trim().eq_ignore_ascii_case("TRUE"),
        _ => return false,
    }
    true
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
        out.push("CLASS:PRIVATE".to_string());
    }
    if e.no_new_times {
        out.push("X-MICROSOFT-DISALLOW-COUNTER:TRUE".to_string());
    }
    out
}
