//! The contacts field's commands: suggestions as a name is typed.

use quatrelle_model::AddresseeKind;
use quatrelle_model::addr::Accepts;

use super::*;
use crate::{FieldId, Suggestion};

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("people.suggest", "Suggest people", "Message › To", suggest).params(
            r#"{"window": number, "field": string (to, cc, bcc, replyTo, book-to, book-cc, book-bcc, cli), "text": string, "account"?: string, "accepts"?: [kind codes] (none: every kind)} → [{"name", "email", "kind", "itemId"?, "source"}]"#,
        ),
    ]
}

/// Suggestions for a field, from what is on this computer (the server is never asked).
fn suggest(s: &mut Session, v: &Value) -> CmdResult {
    let window = p::u64(v, "window").ok_or_else(|| CmdError::Params("`window` (number) is required".into()))?;
    let name = p::req_str(v, "field")?;
    let field = FieldId::parse(window, name).ok_or_else(|| CmdError::Params(format!("there's no field `{name}`")))?;
    let text = p::req_str(v, "text")?;
    let account = match p::str(v, "account") {
        Some(a) if s.account(a).is_some() => Some(a.to_string()),
        Some(a) => return Err(CmdError::Params(format!("no account `{a}`"))),
        None => None,
    };
    let kinds = accepted_kinds(v)?;
    let list = s.people_suggest_kinds(field, account.as_deref(), &kinds, text);
    Ok(Value::Array(list.iter().map(suggestion_json).collect()))
}

/// The kinds a field takes, from their codes (none given: every kind), each once.
fn accepted_kinds(v: &Value) -> Result<Vec<AddresseeKind>, CmdError> {
    let codes = match v.get("accepts") {
        None | Some(Value::Null) => return Ok(Accepts::RECIPIENTS.kinds.to_vec()),
        Some(Value::Array(codes)) => codes,
        Some(_) => return Err(CmdError::Params("`accepts` is a list of kinds".into())),
    };
    let mut kinds = Vec::new();
    for c in codes {
        let k = c.as_str().and_then(AddresseeKind::from_code).ok_or_else(|| CmdError::Params(format!("there's no kind {c}")))?;
        if !kinds.contains(&k) {
            kinds.push(k);
        }
    }
    Ok(kinds)
}

/// A suggestion as the commands answer it.
pub(super) fn suggestion_json(x: &Suggestion) -> Value {
    let mut o = json!({"name": x.who.name, "email": x.who.address, "kind": x.who.kind.code(), "source": x.source.code()});
    if !x.who.item_id.is_empty() {
        o["itemId"] = json!(x.who.item_id);
    }
    o
}
