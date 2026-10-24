//! Organising mail: categories (colour-coded keywords), inbox rules, snoozing, and the
//! Focused/Other split of the Inbox. Pure data and matching: the engine applies them.

use serde::{Deserialize, Serialize};

use crate::dates::{Clock, civil_from_days};
use crate::{FolderRole, Summary, keyword_for};

/// A category: a name and colour shown on messages, stored on the server as a keyword so other
/// clients (and other computers) see it too.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub name: String,
    pub keyword: String,
    /// An index into the UI's category palette.
    #[serde(default)]
    pub color: u8,
}

impl Category {
    pub fn new(name: &str, color: u8) -> Category {
        Category { name: name.trim().to_string(), keyword: keyword_for(name), color }
    }

    /// Whether a message carries it: by its keyword, or by its name as Exchange keeps it (read
    /// back as [`keyword_for`] the name: `Important` is `$label1` here, `important` there).
    pub fn marks(&self, f: &crate::Flags) -> bool {
        !f.keywords.is_empty() && (f.has(&self.keyword) || f.has(&keyword_for(&self.name)))
    }
}

/// The colours a category can have: Quatrelle's first eight, then the rest of Outlook's 25, so
/// a category made in Outlook keeps its colour here.
pub const CATEGORY_COLORS: u8 = 26;

/// Quatrelle's colour for an Outlook category colour, as Exchange's master category list numbers
/// them (MS-OXOCFG: 0 red, 1 orange, 2 peach, 3 yellow, 4 green, 5 teal, 6 olive, 7 blue,
/// 8 purple, 9 maroon, 10 steel, 11 dark steel, 12 gray, 13 dark gray, 14 black, then 15 to 24
/// the dark red to dark maroon); `None` for none (-1) or one unknown.
pub fn outlook_color(preset: i32) -> Option<u8> {
    const COLORS: [u8; 25] = [0, 1, 8, 5, 2, 6, 9, 3, 4, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25];
    usize::try_from(preset).ok().and_then(|i| COLORS.get(i)).copied()
}

/// The categories a new profile starts with. Their keywords are the ones Thunderbird uses for its
/// five default tags, so tags made there show here and back.
pub fn default_categories() -> Vec<Category> {
    [("Important", "$label1", 0), ("Work", "$label2", 1), ("Personal", "$label3", 2), ("To do", "$label4", 3), ("Later", "$label5", 4)]
        .into_iter()
        .map(|(n, k, c)| Category { name: n.into(), keyword: k.into(), color: c })
        .collect()
}

/// What a rule looks at. Text tests are case-insensitive "contains".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "if", content = "value", rename_all = "camelCase")]
pub enum Condition {
    /// The sender's name or address.
    From(String),
    /// Any To or Cc recipient.
    To(String),
    Subject(String),
    /// Subject, sender or recipients.
    AnyText(String),
    HasAttachment,
    /// Newsletters, lists, automatic mail.
    Bulk,
    /// The sender is one of these (Exchange keeps "from these people" as a list).
    FromAny(Vec<String>),
    /// Sent to one of these addresses (Exchange's "sent to people or groups").
    ToAny(Vec<String>),
    /// The subject contains one of these.
    SubjectAny(Vec<String>),
}

impl Condition {
    pub fn matches(&self, m: &Summary) -> bool {
        let has = |hay: &str, needle: &str| !needle.trim().is_empty() && hay.to_lowercase().contains(&needle.trim().to_lowercase());
        let addr = |a: &crate::EmailAddress| format!("{} {}", a.name, a.email);
        match self {
            Condition::From(t) => has(&addr(&m.from), t),
            Condition::To(t) => m.to.iter().chain(&m.cc).any(|a| has(&addr(a), t)),
            Condition::Subject(t) => has(&m.subject, t),
            Condition::AnyText(t) => has(&m.subject, t) || has(&addr(&m.from), t) || m.to.iter().chain(&m.cc).any(|a| has(&addr(a), t)),
            Condition::HasAttachment => m.has_attachments,
            Condition::Bulk => m.bulk,
            Condition::FromAny(any) => any.iter().any(|t| has(&addr(&m.from), t)),
            Condition::ToAny(any) => any.iter().any(|t| m.to.iter().chain(&m.cc).any(|a| has(&addr(a), t))),
            Condition::SubjectAny(any) => any.iter().any(|t| has(&m.subject, t)),
        }
    }
}

/// What a rule does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "do", content = "value", rename_all = "camelCase")]
pub enum RuleAction {
    /// Move to a folder (its path) of the same account.
    MoveTo(String),
    /// Move to the folder with this role (Archive, Junk…).
    MoveToRole(FolderRole),
    MarkRead,
    Flag,
    Pin,
    /// Add a category (its name).
    Categorize(String),
    /// Move to Deleted Items.
    Delete,
}

/// An inbox rule, run on mail as it arrives in an Inbox (the client applies it, like Outlook's
/// client-side rules).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Only this account's mail (`None`: every account).
    #[serde(default)]
    pub account: Option<String>,
    pub conditions: Vec<Condition>,
    /// All conditions must match (`false`: any one).
    #[serde(default = "yes")]
    pub match_all: bool,
    pub actions: Vec<RuleAction>,
    /// Don't run later rules on a message this one matched.
    #[serde(default)]
    pub stop: bool,
    /// Also run on the server (Exchange's inbox rules), so mail is filed while Quatrelle is
    /// closed. Only what the server can express goes there: see [`Rule::server_blockers`].
    #[serde(default)]
    pub server: bool,
    /// The id the server gave this rule, kept to change or remove it there (never set by hand).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_id: Option<String>,
    /// The place the server gives it among its rules (Exchange's `Priority`), kept when changing
    /// it so that the other rules keep theirs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<u32>,
    /// Why the server refused it the last time it was sent there (Exchange keeps Outlook's own
    /// copy of the rules and changes none while it's there, say). A refused rule isn't sent
    /// again by itself on every connection: it goes again once it's changed here (which clears
    /// this), or when the user sends the rules again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
}

fn yes() -> bool {
    true
}

/// Where a rule can run besides this computer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerKind {
    /// Exchange's inbox rules (EWS, Microsoft Graph): conditions combined with AND, no "any
    /// text", no newsletters, no flag or pin.
    Exchange,
    /// The IMAP account's Sieve script: everything but a look at the attachments and at
    /// newsletters, for which no extension a server is sure to have exists.
    Sieve,
}

impl Rule {
    /// What keeps this rule from running on `kind`, in words (`[]` when it can go there).
    pub fn server_blockers_for(&self, kind: ServerKind) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = Vec::new();
        if kind == ServerKind::Exchange && !self.match_all {
            out.push("mail matching any one of its conditions");
        }
        for c in &self.conditions {
            match c {
                Condition::AnyText(_) if kind == ServerKind::Exchange => out.push("looking in the subject, sender and recipients at once"),
                Condition::Bulk => out.push("newsletters and automatic mail"),
                Condition::HasAttachment if kind == ServerKind::Sieve => out.push("checking for attachments"),
                Condition::From(_)
                | Condition::To(_)
                | Condition::Subject(_)
                | Condition::AnyText(_)
                | Condition::HasAttachment
                | Condition::FromAny(_)
                | Condition::ToAny(_)
                | Condition::SubjectAny(_) => {}
            }
        }
        if kind == ServerKind::Exchange {
            for a in &self.actions {
                match a {
                    RuleAction::Flag => out.push("flagging"),
                    RuleAction::Pin => out.push("pinning"),
                    RuleAction::MoveTo(_) | RuleAction::MoveToRole(_) | RuleAction::MarkRead | RuleAction::Categorize(_) | RuleAction::Delete => {}
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// What keeps this rule from running on Exchange (see [`Rule::server_blockers_for`]).
    pub fn server_blockers(&self) -> Vec<&'static str> {
        self.server_blockers_for(ServerKind::Exchange)
    }

    /// Whether `kind` can run this rule as it is.
    pub fn server_ready_for(&self, kind: ServerKind) -> bool {
        self.server_blockers_for(kind).is_empty()
    }

    /// Whether Exchange can run this rule as it is.
    pub fn server_ready(&self) -> bool {
        self.server_blockers_for(ServerKind::Exchange).is_empty()
    }

    /// Whether an IMAP account's Sieve can run this rule as it is.
    pub fn sieve_ready(&self) -> bool {
        self.server_ready_for(ServerKind::Sieve)
    }

    pub fn matches(&self, m: &Summary) -> bool {
        if !self.enabled || self.conditions.is_empty() || self.actions.is_empty() {
            return false;
        }
        if self.account.as_ref().is_some_and(|a| *a != m.key.account) {
            return false;
        }
        if self.match_all { self.conditions.iter().all(|c| c.matches(m)) } else { self.conditions.iter().any(|c| c.matches(m)) }
    }
}

/// The actions of every enabled rule that matches `m`, in rule order.
pub fn actions_for(rules: &[Rule], m: &Summary) -> Vec<RuleAction> {
    let mut out = Vec::new();
    for r in rules.iter().filter(|r| r.matches(m)) {
        out.extend(r.actions.iter().cloned());
        if r.stop {
            break;
        }
    }
    out
}

/// A rule Quatrelle didn't make but the account's server runs (Outlook's own, the server's web
/// page, a Sieve script the user wrote): listed as it is, never changed from here — whoever made
/// it owns it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignRule {
    pub account: String,
    /// The id the server knows it by: what turns it off or removes it there.
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// Quatrelle knows every condition and action of it, so it can run it here.
    pub runnable: bool,
    /// The rule in Quatrelle's terms, when it can be run here.
    pub rule: Option<Rule>,
    /// What it does, in a line (the server's own words where Quatrelle has none).
    pub summary: String,
}

/// A snoozed message: it waits in the Snoozed folder and comes back to `back_to` at `until`.
/// Found again by Message-ID, since moving gives it a new UID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snooze {
    pub account: String,
    pub message_id: String,
    /// The Snoozed folder's path.
    pub folder: String,
    pub back_to: String,
    pub until: i64,
    #[serde(default)]
    pub subject: String,
}

/// Snooze choices, as the menu offers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnoozeUntil {
    /// In three hours.
    LaterToday,
    /// 8:00 tomorrow.
    Tomorrow,
    /// 8:00 next Saturday.
    Weekend,
    /// 8:00 next Monday.
    NextWeek,
}

impl SnoozeUntil {
    pub const ALL: [SnoozeUntil; 4] = [SnoozeUntil::LaterToday, SnoozeUntil::Tomorrow, SnoozeUntil::Weekend, SnoozeUntil::NextWeek];

    pub fn code(self) -> &'static str {
        match self {
            SnoozeUntil::LaterToday => "later",
            SnoozeUntil::Tomorrow => "tomorrow",
            SnoozeUntil::Weekend => "weekend",
            SnoozeUntil::NextWeek => "nextWeek",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SnoozeUntil::LaterToday => "Later today",
            SnoozeUntil::Tomorrow => "Tomorrow",
            SnoozeUntil::Weekend => "This weekend",
            SnoozeUntil::NextWeek => "Next week",
        }
    }

    pub fn from_code(s: &str) -> Option<SnoozeUntil> {
        SnoozeUntil::ALL.into_iter().find(|p| p.code().eq_ignore_ascii_case(s))
    }

    /// When, in Unix seconds, for a user at `clock` (local time).
    pub fn at(self, clock: &Clock) -> i64 {
        let day = clock.local(clock.now).div_euclid(86_400);
        let morning = |d: i64| clock.utc(d * 86_400 + 8 * 3600);
        // 1970-01-01 was a Thursday: 0 = Monday … 6 = Sunday.
        let weekday = (day + 3).rem_euclid(7);
        match self {
            SnoozeUntil::LaterToday => {
                let t = clock.now + 3 * 3600;
                t - t.rem_euclid(3600) + 3600 * i64::from(t.rem_euclid(3600) > 0)
            }
            SnoozeUntil::Tomorrow => morning(day + 1),
            SnoozeUntil::Weekend => morning(day + if weekday < 5 { 5 - weekday } else { 7 - weekday + 5 }),
            SnoozeUntil::NextWeek => morning(day + 7 - weekday),
        }
    }
}

/// The calendar date of a day number (for tests and labels).
pub fn date_of(day: i64) -> (i64, u32, u32) {
    civil_from_days(day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EmailAddress, MessageKey};

    fn m(from: &str, subject: &str, bulk: bool) -> Summary {
        Summary {
            key: MessageKey { account: "a1".into(), ..Default::default() },
            from: EmailAddress::new("", from),
            to: vec![EmailAddress::new("Me", "me@x.example")],
            subject: subject.into(),
            bulk,
            ..Default::default()
        }
    }

    #[test]
    fn a_rule_can_look_at_a_list_of_people_or_subjects() {
        let r = Rule {
            name: "Alerts".into(),
            enabled: true,
            account: None,
            conditions: vec![Condition::FromAny(vec!["nagios@".into(), "zabbix@".into()])],
            match_all: true,
            actions: vec![RuleAction::MoveTo("Alerts".into())],
            stop: false,
            server: false,
            remote_id: None,
            priority: None,
            refused: None,
        };
        assert!(r.matches(&m("nagios@asp.example", "disk", false)), "the first of them");
        assert!(r.matches(&m("ZABBIX@asp.example", "disk", false)), "the other, case aside");
        assert!(!r.matches(&m("someone@asp.example", "disk", false)));
        let subjects = Condition::SubjectAny(vec!["disk full".into(), "raid".into()]);
        assert!(subjects.matches(&m("x@y", "RAID degraded", false)));
        assert!(!subjects.matches(&m("x@y", "hello", false)));
        let to = Condition::ToAny(vec!["team@x.example".into()]);
        assert!(!to.matches(&m("x@y", "hello", false)), "the message goes to me@x.example, not to that list");
    }

    #[test]
    fn rules_match_and_stop() {
        let news = Rule {
            name: "News".into(),
            enabled: true,
            account: None,
            conditions: vec![Condition::Bulk, Condition::From("letters".into())],
            match_all: true,
            actions: vec![RuleAction::MoveTo("News".into()), RuleAction::MarkRead],
            stop: true,
            server: false,
            remote_id: None,
            priority: None,
            refused: None,
        };
        let flag = Rule {
            name: "Boss".into(),
            conditions: vec![Condition::From("boss@".into())],
            actions: vec![RuleAction::Flag],
            stop: false,
            ..news.clone()
        };
        let rules = vec![news.clone(), flag.clone()];
        assert_eq!(actions_for(&rules, &m("letters@shop.example", "Sale", true)), vec![RuleAction::MoveTo("News".into()), RuleAction::MarkRead]);
        assert!(actions_for(&rules, &m("letters@shop.example", "Hi", false)).is_empty(), "all conditions must hold");
        assert_eq!(actions_for(&rules, &m("BOSS@x.example", "Now", false)), vec![RuleAction::Flag]);
        let any = Rule { match_all: false, ..news.clone() };
        assert!(any.matches(&m("x@y.example", "Hi", true)));
        assert!(!Rule { enabled: false, ..news.clone() }.matches(&m("letters@x", "", true)));
        assert!(!Rule { account: Some("a2".into()), ..news }.matches(&m("letters@x", "", true)));
        assert!(Condition::To("me@".into()).matches(&m("a@b", "", false)));
        assert!(!Condition::Subject("  ".into()).matches(&m("a@b", "anything", false)), "an empty test matches nothing");
        // Rules round-trip through JSON (rules.json, the CLI).
        let json = serde_json::to_string(&rules).unwrap();
        assert!(json.contains("\"if\":\"bulk\""), "{json}");
        assert_eq!(serde_json::from_str::<Vec<Rule>>(&json).unwrap(), rules);
    }

    /// Exchange's inbox rules combine their conditions with AND and know no "any text", no
    /// newsletters, no flag and no pin: a rule says what keeps it from running there.
    #[test]
    fn a_rule_knows_what_exchange_can_run() {
        let base = Rule {
            name: "R".into(),
            enabled: true,
            account: Some("a1".into()),
            conditions: vec![Condition::Subject("invoice".into())],
            match_all: true,
            actions: vec![RuleAction::MoveTo("Invoices".into())],
            stop: false,
            server: false,
            remote_id: None,
            priority: None,
            refused: None,
        };
        assert!(base.server_ready(), "a subject test and a move: Exchange does that");
        for (change, why) in [
            (Rule { conditions: vec![Condition::AnyText("x".into())], ..base.clone() }, "looking in the subject"),
            (Rule { conditions: vec![Condition::Bulk], ..base.clone() }, "newsletters"),
            (Rule { match_all: false, ..base.clone() }, "any one of its conditions"),
            (Rule { actions: vec![RuleAction::Flag], ..base.clone() }, "flagging"),
            (Rule { actions: vec![RuleAction::Pin], ..base.clone() }, "pinning"),
        ] {
            let blockers = change.server_blockers();
            assert!(blockers.iter().any(|b| b.contains(why)), "{why}: {blockers:?}");
            assert!(!change.server_ready());
        }
        assert!(Rule { actions: vec![RuleAction::MoveToRole(FolderRole::Archive)], ..base.clone() }.server_ready());
        assert!(Rule { actions: vec![RuleAction::Categorize("Blue".into())], ..base }.server_ready());
    }

    /// Rules kept before the marker came in read as they were; one marked keeps its reason.
    #[test]
    fn a_rule_remembers_why_the_server_refused_it() {
        let mut r: Rule = serde_json::from_str(r#"{"name":"x","conditions":[],"actions":[]}"#).unwrap();
        assert_eq!((r.refused.as_deref(), r.enabled, r.match_all), (None, true, true));
        r.refused = Some("the server refused: no".into());
        let back: Rule = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back.refused.as_deref(), Some("the server refused: no"));
        r.refused = None;
        assert!(!serde_json::to_string(&r).unwrap().contains("refused"), "left out when there's none");
    }

    #[test]
    fn snooze_times_are_local() {
        // Wednesday 2026-10-14 15:20 UTC, in UTC+2 (17:20 local).
        let clock = Clock::new(1_791_991_200 + 20 * 60, 7200);
        let local_day = |t: i64| date_of((t + 7200).div_euclid(86_400));
        let local_hour = |t: i64| (t + 7200).rem_euclid(86_400) / 3600;
        let later = SnoozeUntil::LaterToday.at(&clock);
        assert!(later > clock.now + 3 * 3600 - 1 && later % 3600 == 0);
        let t = SnoozeUntil::Tomorrow.at(&clock);
        assert_eq!((local_day(t), local_hour(t)), ((2026, 10, 15), 8));
        let w = SnoozeUntil::Weekend.at(&clock);
        assert_eq!((local_day(w), local_hour(w)), ((2026, 10, 17), 8), "Saturday");
        let n = SnoozeUntil::NextWeek.at(&clock);
        assert_eq!((local_day(n), local_hour(n)), ((2026, 10, 19), 8), "Monday");
        // On a Sunday, "this weekend" is next Saturday and "next week" is tomorrow.
        let sunday = Clock::new(1_792_324_800, 0); // 2026-10-18 12:00 UTC
        assert_eq!(date_of(sunday.now.div_euclid(86_400)), (2026, 10, 18));
        assert_eq!(date_of(SnoozeUntil::NextWeek.at(&sunday).div_euclid(86_400)), (2026, 10, 19));
        assert_eq!(date_of(SnoozeUntil::Weekend.at(&sunday).div_euclid(86_400)), (2026, 10, 24));
        assert_eq!(SnoozeUntil::from_code("NEXTWEEK"), Some(SnoozeUntil::NextWeek));
    }

    #[test]
    fn default_categories_use_shared_keywords() {
        let c = default_categories();
        assert_eq!(c[0].keyword, "$label1");
        assert_eq!(Category::new(" Clients ", 6), Category { name: "Clients".into(), keyword: "clients".into(), color: 6 });
    }
}
