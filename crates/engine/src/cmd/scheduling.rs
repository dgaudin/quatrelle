//! Commands: the rooms to book and when people and rooms are free, for a meeting being written
//! (Outlook's Room Finder and Scheduling Assistant).

use super::*;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("calendar.rooms", "Room Finder", "Calendar › Event › Rooms", |s, v| {
            let account = scheduling_account(s, v)?;
            s.request_rooms(&account, p::bool(v, "refresh").unwrap_or(false))
        })
        .params(
            r#"{"account"?: string (an Exchange or Microsoft 365 account asks its server; any other offers the rooms its calendars know), "refresh"?: bool} → {"state": "loading"|"ready"|"failed", "lists": [{"name", "email"}], "rooms": [{"name", "email", "list"}], "fromCalendar": bool}; asked once, the answer kept"#,
        ),
        CommandSpec::new("calendar.availability", "Scheduling Assistant", "Calendar › Event › Scheduling", |s, v| {
            let account = scheduling_account(s, v)?;
            let emails: Vec<String> = match v.get("emails") {
                Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
                Some(Value::String(x)) => quatrelle_model::addr::parse_list(x)
                    .map_err(|bad| CmdError::Params(format!("not an email address: {bad}")))?
                    .into_iter()
                    .map(|a| a.email)
                    .collect(),
                _ => return Err(CmdError::Params("`emails` is needed: whose availability".into())),
            };
            // A date alone is that whole local day.
            let when = |k: &str, end: bool| -> Option<i64> {
                match p::str(v, k) {
                    Some(x) => parse_local(x, &s.clock).map(|(t, all_day)| if all_day { s.clock.utc(t + if end { 86_400 } else { 0 }) } else { t }),
                    None => p::i64(v, k),
                }
            };
            let from = when("from", false).ok_or_else(|| CmdError::Params("`from` is needed (YYYY-MM-DD[THH:MM] or Unix seconds)".into()))?;
            let to = when("to", true).unwrap_or_else(|| from.saturating_add(86_400));
            s.request_availability(&account, &emails, from, to, p::bool(v, "refresh").unwrap_or(false))
        })
        .params(
            r#"{"emails": [string] | "a@x, Name <b@y>" (people and rooms), "from": "YYYY-MM-DD[THH:MM]" | Unix seconds, "to"?: … (default: a day after from; two months at most), "account"?: string, "refresh"?: bool} → each one's state (loading|ready|failed) and busy stretches [{"start", "end", "kind": "busy"|"tentative"|"outOfOffice"|"workingElsewhere"|"noData", "subject"}]; asked once per window, the answer kept"#,
        ),
    ]
}

/// The account whose server (or calendars) answer: the one named, else the one on screen when
/// it has an Exchange server, else the first that has one, else the one on screen.
fn scheduling_account(s: &Session, v: &Value) -> Result<String, CmdError> {
    if let Some(a) = p::str(v, "account") {
        return if s.account(a).is_some() { Ok(a.to_string()) } else { Err(CmdError::Params(format!("no account `{a}`"))) };
    }
    let exchange = |a: &&crate::AccountState| a.config.ews.is_some() || a.config.graph.is_some();
    s.current_account()
        .filter(exchange)
        .or_else(|| s.accounts.iter().find(exchange))
        .or_else(|| s.current_account())
        .map(|a| a.config.id.clone())
        .ok_or_else(|| CmdError::Disabled("add an account first".into()))
}
