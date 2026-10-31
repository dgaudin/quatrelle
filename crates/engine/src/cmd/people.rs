//! The contacts field's commands: suggestions as a name is typed, and Check Names for any field.

use quatrelle_model::addr::{Accepts, valid_item_id};
use quatrelle_model::{Addressee, AddresseeKind};

use super::*;
use crate::{FieldId, Suggestion};

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("people.suggest", "Suggest people", "Message › To", suggest).params(
            r#"{"window": number, "field": string (to, cc, bcc, replyTo, book-to, book-cc, book-bcc, cli, from, required, optional, rooms, att-required, att-optional, att-rooms, calendar, mailbox, rule-one, rule-any, search-people), "text": string, "account"?: string, "accepts"?: [kind codes] (none: every kind)} → [{"name", "email", "kind", "itemId"?, "source"}]"#,
        ),
        CommandSpec::new("people.resolve", "Check Names", "Message › Names", resolve).params(
            r#"{"window": number, "field": string (as people.suggest), "text"?: string (a field that isn't a message's: its text; a message's To, Cc, Bcc or Reply-To is checked in the message), "account"?: string (whose directory), "accepts"?: [kind codes] (of several that fit, these first)} → {"text", "resolved", "unresolved", "searching"}"#,
        ),
        CommandSpec::new("people.expand", "Expand a list", "Message › Names", expand).params(
            r#"{"account"?: string (whose directory), "email"?: string (a distribution list's address), "itemId"?: string (a contact group's), "name"?: string, "kind"?: kind code (default distributionList)} → {"account", "state": "loading"|"ready"|"failed", "members": [{"name", "email", "kind", "itemId"?}], "more", "total", "error"?}"#,
        ),
        CommandSpec::new("people.choose", "Choose who a name means", "Message › Names", choose).params(
            r#"{"window": number, "field": string, "token": string (the name as typed), "email"?: string (the person meant; neither it nor itemId: leave the name as typed), "name"?: string, "kind"?: kind code, "itemId"?: string (a contact group's)} → {"token", "written"}"#,
        ),
    ]
}

/// The field a command is about: its window and its name.
fn field_of(v: &Value) -> Result<FieldId, CmdError> {
    let window = p::u64(v, "window").ok_or_else(|| CmdError::Params("`window` (number) is required".into()))?;
    let name = p::req_str(v, "field")?;
    FieldId::parse(window, name).ok_or_else(|| CmdError::Params(format!("there's no field `{name}`")))
}

/// The account a command names, if it names one that exists.
fn account_of(s: &Session, v: &Value) -> Result<Option<String>, CmdError> {
    match p::str(v, "account") {
        Some(a) if s.account(a).is_some() => Ok(Some(a.to_string())),
        Some(a) => Err(CmdError::Params(format!("no account `{a}`"))),
        None => Ok(None),
    }
}

/// Suggestions for a field, from what is on this computer (the server is never asked).
fn suggest(s: &mut Session, v: &Value) -> CmdResult {
    let field = field_of(v)?;
    let text = p::req_str(v, "text")?;
    let account = account_of(s, v)?;
    let kinds = accepted_kinds(v)?;
    let list = s.people_suggest_kinds(field, account.as_deref(), &kinds, text);
    Ok(Value::Array(list.iter().map(suggestion_json).collect()))
}

/// Check Names on a field ([`Session::people_resolve`]).
fn resolve(s: &mut Session, v: &Value) -> CmdResult {
    let field = field_of(v)?;
    let text = match v.get("text") {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) => Some(t.as_str()),
        Some(_) => return Err(CmdError::Params("`text` is a string".into())),
    };
    let account = account_of(s, v)?;
    let kinds = accepted_kinds(v)?;
    let r = s.people_resolve_kinds(field, account.as_deref(), &kinds, text)?;
    Ok(json!({"text": r.text, "resolved": r.resolved, "unresolved": r.unresolved, "searching": r.searching}))
}

/// The person meant by a name several fit, in a field ([`Session::people_choose`]).
fn choose(s: &mut Session, v: &Value) -> CmdResult {
    let field = field_of(v)?;
    let token = p::req_str(v, "token")?.to_string();
    let person = chosen(v)?;
    s.people_choose(field, &token, person)
}

/// The members of a distribution list or a contact group ([`Session::people_expand`]).
fn expand(s: &mut Session, v: &Value) -> CmdResult {
    let account = account_of(s, v)?;
    let kind = match p::str(v, "kind") {
        Some(k) => AddresseeKind::from_code(k).ok_or_else(|| CmdError::Params(format!("there's no kind `{k}`")))?,
        None => AddresseeKind::DistributionList,
    };
    let item_id = match p::str(v, "itemId").map(str::trim).filter(|i| !i.is_empty()) {
        Some(id) if valid_item_id(id) => id.to_string(),
        Some(_) => return Err(CmdError::Params("`itemId` isn't an item id".into())),
        None => String::new(),
    };
    let who = Addressee {
        name: p::str(v, "name").unwrap_or("").trim().to_string(),
        address: p::str(v, "email").unwrap_or("").trim().to_string(),
        kind,
        item_id,
    };
    s.people_expand(account.as_deref(), &who)
}

/// The person a choice names: an address (`email`, with its `name`), or a contact group by its
/// `itemId` (it has none); its `kind` when given. `None`: no one, the name left as typed.
pub(super) fn chosen(v: &Value) -> Result<Option<Addressee>, CmdError> {
    let name = p::str(v, "name").unwrap_or("").trim();
    let kind = match p::str(v, "kind") {
        Some(k) => AddresseeKind::from_code(k).ok_or_else(|| CmdError::Params(format!("there's no kind `{k}`")))?,
        None => AddresseeKind::Unknown,
    };
    match p::str(v, "email").map(str::trim).filter(|e| !e.is_empty()) {
        Some(email) if email.contains('@') => Ok(Some(Addressee::new(name, email, kind))),
        Some(email) => Err(CmdError::Params(format!("not an email address: {email}"))),
        None => match p::str(v, "itemId").map(str::trim).filter(|i| !i.is_empty()) {
            Some(id) if valid_item_id(id) => Ok(Some(Addressee { item_id: id.to_string(), ..Addressee::new(name, "", AddresseeKind::ContactGroup) })),
            Some(_) => Err(CmdError::Params("`itemId` isn't an item id".into())),
            None => Ok(None),
        },
    }
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
