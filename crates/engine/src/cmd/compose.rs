//! Commands: writing messages, and attachments.

use quatrelle_model::MessageKey;

use super::*;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        // Compose
        CommandSpec::new("compose.list", "Messages being written", "Message", |s, _| {
            let active = s.compose.as_ref().map(|c| c.id);
            Ok(json!(s.all_composes().map(|c| json!({"id": c.id, "subject": c.subject, "to": c.to, "active": Some(c.id) == active})).collect::<Vec<_>>()))
        }),
        CommandSpec::new("compose.activate", "Switch to a message being written", "Message", |s, v| {
            let id = p::u64(v, "id").ok_or_else(|| CmdError::Params("`id` is required (from compose.list)".into()))?;
            if s.activate_compose(id) { Ok(json!({"active": id})) } else { Err(CmdError::Params(format!("no message {id} is being written"))) }
        })
        .params(r#"{"id": number (from compose.list)}"#),
        CommandSpec::new("compose.keepAll", "Keep the messages being written", "Closing Quatrelle", |s, _| {
            let kept = s.keep_drafts();
            Ok(json!({ "kept": kept }))
        }),
        CommandSpec::new("compose.update", "Edit message", "Message", compose_update)
            .params(r#"{"to"?: string, "cc"?: string, "bcc"?: string, "subject"?: string, "body"?: string, "marks"?: bool (the body uses **bold**, *italic*, - lists…), "account"?: string, "from"?: string (who it's from: a shared mailbox the account opens, or another address it may send from; empty: the account's own), "importance"?: "high"|"normal"|"low", "readReceipt"?: bool, "replyTo"?: string (where replies go), "sendAfter"?: number (Unix seconds: held in the Outbox until then; 0 sends at once), "voting"?: string (voting buttons, `;`-separated: "Approve;Reject"; Exchange accounts)}"#)
            .when(has_compose),
        CommandSpec::new("compose.language", "Language", "Message › Review", |s, v| {
            let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
            let lang = p::str(v, "language").unwrap_or("").trim().to_ascii_lowercase();
            if lang.len() > 16 || !lang.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_')) {
                return Err(CmdError::Params("`language` is a code such as `fr`, `en` or `all`".into()));
            }
            c.language = lang;
            Ok(json!({"language": c.language}))
        })
        .params(r#"{"language": string (`fr`, `en`… as the dictionaries are named; `all`: every dictionary; empty: the interface's language)}"#)
        .when(has_compose),
        CommandSpec::new("compose.checkNames", "Check Names", "Message › Names", check_names).when(has_compose),
        CommandSpec::new("compose.chooseName", "Choose who a name means", "Message › Names", choose_name)
            .params(r#"{"token": string (the name as typed), "email"?: string (the person meant; none: leave the name as typed), "name"?: string}"#)
            .when(has_compose),
        CommandSpec::new("contacts.directory", "Search the directory", "Message › Names", search_directory).params(
            r#"{"query": string (the start of a name, a first or last name, an alias or an address), "account"?: string (an Exchange account)}"#,
        ),
        CommandSpec::new("compose.replaceText", "Replace a stretch of the message", "Message", compose_replace_text)
            .params(
                r#"{"from": number (byte offset), "to": number, "text": string, "field"?: "body" (default) | "subject" (the subject: byte offsets into it)}"#,
            )
            .when(has_compose),
        CommandSpec::new("spelling.add", "Add a word to the dictionary", "Message", |s, v| {
            let word = p::req_str(v, "word")?.trim().to_lowercase();
            if word.is_empty() || word.chars().count() > 64 {
                return Err(CmdError::Params("a word is needed".into()));
            }
            if !s.prefs.spelling.iter().any(|w| w.eq_ignore_ascii_case(&word)) {
                s.prefs.spelling.push(word.clone());
                s.prefs.spelling.sort();
            }
            Ok(json!({"word": word, "words": s.prefs.spelling}))
        })
        .params(r#"{"word": string}"#),
        CommandSpec::new("spelling.remove", "Remove a word from the dictionary", "Message", |s, v| {
            let word = p::req_str(v, "word")?.trim().to_lowercase();
            s.prefs.spelling.retain(|w| !w.eq_ignore_ascii_case(&word));
            Ok(json!({"word": word, "words": s.prefs.spelling}))
        })
        .params(r#"{"word": string}"#),
        CommandSpec::new("compose.attach", "Attach file", "Message › Insert", attach).params(r#"{"path": string} | {"name": string, "data": [bytes]}"#).when(has_compose),
        CommandSpec::new("compose.attachItem", "Attach item", "Message › Insert", attach_item)
            .params(r#"{"kind": "message" | "contact" | "event", "id": string}"#)
            .when(has_compose),
        CommandSpec::new("compose.insertPicture", "Insert picture", "Message › Insert", insert_picture)
            .params(r#"{"path": string} | {"name": string, "data": [bytes]}, "insert"?: bool (default true), "at"?: number (character; default the end), "width"?: number (CSS pixels it's shown and sent at; default its own width), "line"?: bool (on a line of its own)"#)
            .when(has_compose),
        CommandSpec::new("compose.resizePicture", "Picture size", "Message › Picture", resize_picture)
            .params(r#"{"cid": string, "width"?: number (CSS pixels; 0 or none: its own width)}"#)
            .when(has_compose),
        CommandSpec::new("compose.format", "Plain text / Formatted", "Message › Format", |s, v| {
            let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
            c.plain = p::bool(v, "plain").unwrap_or(!c.plain);
            Ok(json!({"plain": c.plain}))
        })
        .params(r#"{"plain"?: bool}"#)
        .when(has_compose),
        CommandSpec::new("compose.removeAttachment", "Remove attachment", "Message › Insert", remove_attachment).params(r#"{"index": number}"#).when(has_compose),
        CommandSpec::new("compose.send", "Send", "Message", |s, v| {
            s.send_compose(p::bool(v, "allowEmptySubject").unwrap_or(false), p::bool(v, "checked").unwrap_or(false))
        })
        .key("Mod+Enter / Alt+S")
        .params(r#"{"allowEmptySubject"?: bool, "checked"?: bool (without it, a message to a lookalike address or showing many people each other's addresses is held, and the answer is {"check": [...]})}"#)
        .when(has_compose),
        CommandSpec::new("compose.hideRecipients", "Put everyone in Bcc", "Message", |s, _| s.hide_recipients()).when(has_compose),
        CommandSpec::new("compose.saveDraft", "Save draft", "Message", |s, _| s.save_draft()).key("Mod+S").when(has_compose),
        CommandSpec::new("compose.discard", "Discard", "Message", discard).when(has_compose),
        CommandSpec::new("compose.popOut", "Pop out", "Message", |s, _| {
            let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
            c.window = true;
            Ok(json!({"window": true}))
        })
        .when(has_compose),
        CommandSpec::new("compose.popIn", "Pop in", "Message", |s, _| {
            let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
            c.window = false;
            Ok(json!({"window": false}))
        })
        .when(has_compose),
        CommandSpec::new("compose.close", "Close", "Message", close_compose).params(r#"{"save"?: bool (keep it in Drafts; default true)}"#).when(has_compose),
        CommandSpec::new("draft.edit", "Edit draft", "Message list › Drafts", |s, v| {
            let k = key_param(s, v)?;
            s.edit_draft(&k)
        })
        .params(r#"{"key": "account:validity:uid:folder"}"#),
        CommandSpec::new("contacts.suggest", "Suggest recipients", "Message › To", |s, v| {
            let q = p::req_str(v, "query")?.to_string();
            let limit = p::u64(v, "limit").map_or(8, |n| usize::try_from(n).unwrap_or(8).clamp(1, 100));
            let list: Vec<Value> = s
                .suggest_contacts(&q, limit)
                .into_iter()
                .map(|c| json!({"name": c.address.name, "email": c.address.email, "address": c.address.to_string(), "sent": c.sent, "received": c.received}))
                .collect();
            Ok(json!(list))
        })
        .params(r#"{"query": string, "limit"?: number}"#),
        // Outlook's × beside a suggestion.
        CommandSpec::new("contacts.forget", "Remove from suggestions", "Message › To", |s, v| {
            let email = p::req_str(v, "email")?.trim().to_string();
            if !email.contains('@') {
                return Err(CmdError::Params(format!("`{email}` isn't an email address")));
            }
            Ok(json!({ "email": email, "removed": s.forget_contact(&email) }))
        })
        .params(r#"{"email": string} (stays out of the suggestions until written to)"#),
        // Attachments
        CommandSpec::new("attachment.save", "Save attachment", "Reading pane", save_attachment)
            .params(r#"{"index": number, "path": string, "open"?: bool (saved to be opened by another app: never a file that runs code)}"#)
            .when(has_reading),
        CommandSpec::new("attachment.saveAll", "Save all attachments", "Reading pane", save_all_attachments).params(r#"{"dir": string, "indices"?: [number] (only these attachments)}"#).when(has_reading),
        CommandSpec::new("attachment.preview", "Preview attachment", "Reading pane", preview_attachment).params(r#"{"index": number}"#).when(has_reading),
    ]
}

pub(super) fn compose_update(s: &mut Session, v: &Value) -> CmdResult {
    let account = p::str(v, "account").map(str::to_string);
    if let Some(a) = &account
        && s.account(a).is_none()
    {
        return Err(CmdError::Params(format!("no account `{a}`")));
    }
    let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    if c.sending {
        return Err(CmdError::Disabled("the message is being sent".into()));
    }
    let cap = |x: &str| x.chars().take(1 << 20).collect::<String>();
    for (k, field) in [("to", &mut c.to), ("cc", &mut c.cc), ("bcc", &mut c.bcc), ("subject", &mut c.subject)] {
        if let Some(x) = p::str(v, k) {
            *field = cap(x);
        }
    }
    if let Some(x) = p::str(v, "body") {
        if p::bool(v, "marks").unwrap_or(false) {
            let (text, styles) = quatrelle_mime::rich::from_markup(&cap(x));
            c.body = text;
            c.styles = styles;
        } else {
            c.set_text(cap(x));
        }
    }
    if !c.bcc.is_empty() {
        c.show_cc = true;
    }
    if let Some(i) = p::str(v, "importance") {
        c.importance = match i {
            "high" => 1,
            "low" => -1,
            "normal" => 0,
            other => return Err(CmdError::Params(format!("importance `{other}` isn't high, normal or low"))),
        };
    }
    if let Some(r) = p::bool(v, "readReceipt") {
        c.read_receipt = r;
    }
    if let Some(x) = p::str(v, "voting") {
        c.voting = x
            .split(';')
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .take(32)
            .map(|c| c.chars().take(255).collect::<String>())
            .collect::<Vec<_>>()
            .join(";");
    }
    if let Some(x) = p::str(v, "replyTo") {
        c.reply_to = cap(x);
    }
    if let Some(at) = p::i64(v, "sendAfter") {
        c.send_after = (at > 0).then_some(at);
    }
    // Another account: from its own address, unless `from` says otherwise.
    if let Some(a) = &account {
        c.account = a.clone();
        c.from.clear();
    }
    let id = c.id;
    if let Some(from) = p::str(v, "from") {
        s.set_compose_from(None, from)?;
    }
    Ok(json!({"compose": id}))
}

/// Largest attachment accepted (most servers refuse messages over 25–35 MB).
pub(super) const ATTACH_MAX: usize = 25 << 20;

/// A file from `path`, or `name` and `data` (bytes), at most 25 MB.
pub(super) fn file_param(v: &Value) -> Result<(String, Vec<u8>), CmdError> {
    let (name, data) = if let Some(path) = p::str(v, "path") {
        let pb = std::path::PathBuf::from(path);
        let data = read_capped(path, ATTACH_MAX as u64).map_err(|e| match e {
            CmdError::Params(m) if m.ends_with("too big") => CmdError::Failed(format!("{path} is larger than 25 MB")),
            other => other,
        })?;
        (pb.file_name().and_then(|n| n.to_str()).unwrap_or("attachment").to_string(), data)
    } else {
        let name = p::req_str(v, "name")?.to_string();
        let data: Vec<u8> = v
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| CmdError::Params("`path`, or `name` and `data`, are required".into()))?
            .iter()
            .take(ATTACH_MAX + 1)
            .map(|b| b.as_u64().and_then(|x| u8::try_from(x).ok()).ok_or_else(|| CmdError::Params("`data` must be bytes".into())))
            .collect::<Result<_, _>>()?;
        (name, data)
    };
    if data.len() > ATTACH_MAX {
        return Err(CmdError::Failed("attachments are limited to 25 MB".into()));
    }
    Ok((name, data))
}

/// What a message being written carries, in bytes.
pub(super) fn compose_size(c: &crate::Compose) -> usize {
    c.attachments.iter().map(|a| a.data.len()).sum::<usize>() + c.inline.iter().map(|p| p.data.len()).sum::<usize>()
}

pub(super) fn attach(s: &mut Session, v: &Value) -> CmdResult {
    let (name, data) = file_param(v)?;
    let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    let total: usize = compose_size(c) + data.len();
    if total > ATTACH_MAX {
        return Err(CmdError::Failed("attachments are limited to 25 MB in total".into()));
    }
    let name = sanitize_filename(&name);
    let mime = mime_from_name(&name).to_string();
    c.attachments.push(OutAttachment { name: name.clone(), mime, data });
    Ok(json!({"attached": name, "count": c.attachments.len()}))
}

/// Attach an item, as Outlook's Attach Item: a message (its key) as an `.eml`, a contact (its
/// uid) as a `.vcf`, an appointment (its uid) as an `.ics`.
pub(super) fn attach_item(s: &mut Session, v: &Value) -> CmdResult {
    let kind =
        p::str(v, "kind").and_then(crate::ItemKind::from_code).ok_or_else(|| CmdError::Params("`kind` is message, contact or event".into()))?;
    let id = p::req_str(v, "id")?;
    let a = s.item_attachment(kind, id)?;
    let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    if compose_size(c) + a.data.len() > ATTACH_MAX {
        return Err(CmdError::Failed("attachments are limited to 25 MB in total".into()));
    }
    let name = a.name.clone();
    c.attachments.push(a);
    Ok(json!({"attached": name, "count": c.attachments.len()}))
}

/// Place a picture in the text. With `"insert": false` the mark isn't added to the text (the
/// editor puts it at the cursor itself).
pub(super) fn insert_picture(s: &mut Session, v: &Value) -> CmdResult {
    let (name, data) = file_param(v)?;
    let name = sanitize_filename(&name);
    let mime = mime_from_name(&name).to_string();
    if !mime.starts_with("image/") {
        return Err(CmdError::Params(format!("{name} isn't a picture")));
    }
    let cid = s.new_cid();
    let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    if compose_size(c) + data.len() > ATTACH_MAX {
        return Err(CmdError::Failed("pictures and attachments are limited to 25 MB in total".into()));
    }
    c.inline.push(quatrelle_mime::InlinePicture { cid: cid.clone(), name, mime, data });
    c.plain = false;
    if !p::bool(v, "insert").unwrap_or(true) {
        return Ok(json!({"cid": cid}));
    }
    let n = c.body.chars().count();
    let mut at = p::u64(v, "at").and_then(|a| usize::try_from(a).ok()).unwrap_or(n).min(n);
    // On a line of its own (a screenshot, from upstream 865a712): a line break before it and
    // after it, where there's text there.
    if p::bool(v, "line").unwrap_or(false) {
        if at > 0 && c.body.chars().nth(at - 1) != Some('\n') {
            insert_text_at(c, at, "\n");
            at += 1;
        }
        if c.body.chars().nth(at).is_some_and(|ch| ch != '\n') {
            insert_text_at(c, at, "\n");
        }
    }
    insert_picture_at(c, at, &cid, picture_width(v));
    Ok(json!({"cid": cid, "end": at + 1}))
}

/// Text into the message at character `at`, in the look of what's before it.
fn insert_text_at(c: &mut crate::session::Compose, at: usize, text: &str) {
    let old = c.body.clone();
    let byte = old.char_indices().nth(at).map_or(old.len(), |(b, _)| b);
    c.body.insert_str(byte, text);
    let mut styles = std::mem::take(&mut c.styles);
    let around = styles.style(at.saturating_sub(1)).for_typing();
    styles.edit_at(&old, &c.body, &around, Some(at + text.chars().count()));
    c.styles = styles;
}

/// Narrowest and widest a picture is set to, in CSS pixels.
pub const PICTURE_WIDTHS: std::ops::RangeInclusive<u16> = 16..=4000;

/// The `width` asked for a picture, within [`PICTURE_WIDTHS`]; `None` (its own width) when
/// there's none, or 0.
fn picture_width(v: &Value) -> Option<u16> {
    let w = p::u64(v, "width").filter(|w| *w > 0)?;
    Some(u16::try_from(w).unwrap_or(u16::MAX).clamp(*PICTURE_WIDTHS.start(), *PICTURE_WIDTHS.end()))
}

/// Show and send picture `cid` at another width (Outlook's picture handles and its Size), or
/// at its own again (from upstream 865a712).
pub(super) fn resize_picture(s: &mut Session, v: &Value) -> CmdResult {
    use quatrelle_mime::rich::{PICTURE, Style};
    let cid = p::req_str(v, "cid")?;
    let width = picture_width(v);
    let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    c.styles.fit(&c.body);
    let at: Vec<usize> =
        c.body.chars().enumerate().filter(|(i, ch)| *ch == PICTURE && c.styles.style(*i).picture.as_deref() == Some(cid)).map(|(i, _)| i).collect();
    if at.is_empty() {
        return Err(CmdError::Params(format!("no picture `{cid}` in the message")));
    }
    let mut styles = std::mem::take(&mut c.styles);
    for i in at {
        styles.restyle(&c.body, i..i + 1, |s| Style { picture_width: width, ..s.clone() });
    }
    c.styles = styles;
    Ok(json!({"cid": cid, "width": width}))
}

/// The account's signature into the message being written, at character `at` (else the end),
/// on a line of its own (after a `-- ` line when the preferences ask for one): Outlook's
/// Insert › Signature.
pub(super) fn insert_signature(s: &mut Session, v: &Value) -> CmdResult {
    use quatrelle_mime::rich::Style;
    let account = s.compose.as_ref().map(|c| c.account.clone()).ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    let rich = s.signature_rich(&account);
    let pictures = s.signature_pictures(&account);
    if rich.0.trim().is_empty() {
        return Err(CmdError::Params("this account has no signature yet: make one with Signatures…".into()));
    }
    // One written in HTML stands as itself, drawn and sent as written.
    let html = s.html_signature(&account);
    let sig = if html.is_some() { quatrelle_mime::rich::SIGNATURE.to_string() } else { rich.0.trim_end().to_string() };
    let block = s.signature_in_message(&sig);
    let dashes = s.prefs.signature_dashes;
    let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    let old = c.body.clone();
    // The signature put in before, still there as it was, is replaced (as choosing a signature
    // in Outlook replaces the one there); otherwise it goes in at the cursor.
    let there = if c.signature.is_empty() { None } else { old.rfind(&c.signature).map(|b| (b, b + c.signature.len())) };
    let (start, stop, text) = match there {
        Some((b, e)) => (b, e, block.clone()),
        None => {
            let n = old.chars().count();
            let at = p::u64(v, "at").and_then(|a| usize::try_from(a).ok()).unwrap_or(n).min(n);
            let b = old.char_indices().nth(at).map_or(old.len(), |(b, _)| b);
            let before = if at > 0 && old.chars().nth(at - 1) != Some('\n') { "\n" } else { "" };
            (b, b, format!("{before}{block}\n"))
        }
    };
    c.signature = block;
    c.body.replace_range(start..stop, &text);
    let at = old.get(..start).unwrap_or("").chars().count();
    let end = at + text.chars().count();
    let mut styles = std::mem::take(&mut c.styles);
    styles.edit_at(&old, &c.body, &Style::default(), Some(end));
    c.styles = styles;
    match html {
        Some(html) => c.signature_html = html,
        None => crate::session::style_signature(c, &rich, dashes),
    }
    crate::session::add_pictures(c, pictures);
    Ok(json!({"inserted": at, "end": end}))
}

/// Look a name up in the organisation's directory (Exchange's Global Address List): who it has,
/// once it has answered (`searching` until then).
fn search_directory(s: &mut Session, v: &Value) -> CmdResult {
    let query = p::req_str(v, "query")?.trim().to_string();
    if query.is_empty() {
        return Err(CmdError::Params("`query` is empty".into()));
    }
    let account = s.directory_account(p::str(v, "account")).ok_or_else(|| CmdError::Disabled("the directory is an Exchange account's".into()))?;
    s.search_directory(&account, &query, false);
    Ok(match s.directory_found(&account, &query) {
        Some(f) if !s.directory_searching(&account, &query) => json!({"account": account, "people": f.people, "more": f.more, "error": f.error}),
        _ => json!({"account": account, "searching": true}),
    })
}

/// Outlook's Check Names ([`Session::check_names`]).
pub(super) fn check_names(s: &mut Session, _: &Value) -> CmdResult {
    s.check_names()
}

/// The person meant by a name several people fit ([`Session::choose_name`]).
fn choose_name(s: &mut Session, v: &Value) -> CmdResult {
    let token = p::req_str(v, "token")?.to_string();
    let person = match p::str(v, "email").map(str::trim).filter(|e| !e.is_empty()) {
        Some(email) if email.contains('@') => Some(quatrelle_model::EmailAddress::new(p::str(v, "name").unwrap_or("").trim(), email)),
        Some(email) => return Err(CmdError::Params(format!("not an email address: {email}"))),
        None => None,
    };
    s.choose_name(&token, person)
}

/// Put picture `cid` into the text at character `at`, `width` CSS pixels wide (`None`: its own).
pub fn insert_picture_at(c: &mut crate::session::Compose, at: usize, cid: &str, width: Option<u16>) {
    use quatrelle_mime::rich::{PICTURE, Style};
    let old = c.body.clone();
    let byte = old.char_indices().nth(at).map_or(old.len(), |(b, _)| b);
    c.body.insert(byte, PICTURE);
    let mut styles = std::mem::take(&mut c.styles);
    styles.edit_at(&old, &c.body, &Style::default(), Some(at + 1));
    styles.restyle(&c.body, at..at + 1, |s| Style { picture: Some(cid.to_string()), picture_width: width, ..s.clone() });
    c.styles = styles;
}

pub(super) fn remove_attachment(s: &mut Session, v: &Value) -> CmdResult {
    let i = p::u64(v, "index").ok_or_else(|| CmdError::Params("`index` is required".into()))?;
    let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    let i = usize::try_from(i).map_err(|_| CmdError::Params("bad index".into()))?;
    if i >= c.attachments.len() {
        return Err(CmdError::Params(format!("no attachment {i}")));
    }
    let a = c.attachments.remove(i);
    Ok(json!({"removed": a.name}))
}

/// Close the message being written: kept in Drafts (unless `save` is false or nothing was
/// written), as closing a message window in Outlook offers.
pub(super) fn close_compose(s: &mut Session, v: &Value) -> CmdResult {
    let c = s.compose.clone().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    let written = s.compose_written();
    let saved = if p::bool(v, "save").unwrap_or(true) && written && !c.sending {
        s.save_draft()?;
        true
    } else {
        false
    };
    s.compose = None;
    if saved {
        s.notice("Saved in Drafts", false);
    }
    Ok(json!({"closed": c.id, "saved": saved}))
}

/// Discard the message being written, as Outlook's Discard: the copies kept in Drafts go too.
pub(super) fn discard(s: &mut Session, _: &Value) -> CmdResult {
    let c = s.compose.take();
    if let Some(c) = &c {
        s.remove_saved_copies(c);
    }
    Ok(json!({"discarded": c.map(|c| c.id)}))
}

impl Session {
    /// The open message's attachment `index`, as bytes.
    pub fn attachment_bytes(&self, index: usize) -> Result<Vec<u8>, CmdError> {
        let r = self.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        // A signed or encrypted message's attachments are the ones inside it.
        let raw = self.readable_raw(&r.key).ok_or_else(|| CmdError::Failed("the message isn't downloaded".into()))?;
        quatrelle_mime::attachment_data(&raw, index).ok_or_else(|| CmdError::Params(format!("no attachment {index}")))
    }

    /// The open message's source (`.eml`).
    pub fn message_source(&self) -> Result<Vec<u8>, CmdError> {
        let r = self.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        self.source_of(&r.key).ok_or_else(|| CmdError::Failed("the message isn't downloaded".into()))
    }

    /// A message's source, when it's on this computer (an Outbox one's as it will go).
    pub fn source_of(&self, key: &MessageKey) -> Option<Vec<u8>> {
        if key.folder == crate::session::OUTBOX {
            self.outbox_entry(key).and_then(|(a, i)| self.store.as_ref().and_then(|st| st.outbox_raw(&a, i.id)))
        } else {
            self.store.as_ref().and_then(|st| st.read_body(key))
        }
    }

    /// The file a message is saved as: its subject (`message` without one), safe to write.
    pub fn message_file(&self, key: &MessageKey, ext: &str) -> String {
        let subject = self.summary(key).map(|m| m.subject.trim().to_string()).unwrap_or_default();
        let stem: String = sanitize_filename(if subject.is_empty() { "message" } else { &subject }).chars().take(80).collect();
        format!("{stem}.{ext}")
    }

    /// The open message as a PDF: subject, header, text and the names of its attachments.
    pub fn message_pdf(&self) -> Result<Vec<u8>, CmdError> {
        let r = self.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        let m = r.parsed.as_ref().ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?;
        let mut header = vec![("From".to_string(), m.from.to_string())];
        if !m.to.is_empty() {
            header.push(("To".into(), quatrelle_model::addr::format_list(&m.to)));
        }
        if !m.cc.is_empty() {
            header.push(("Cc".into(), quatrelle_model::addr::format_list(&m.cc)));
        }
        if m.date > 0 {
            header.push(("Date".into(), quatrelle_model::dates::full_time(m.date, &self.clock)));
        }
        let files: Vec<String> = m.attachments.iter().filter(|a| !a.inline).map(|a| a.name.clone()).collect();
        if !files.is_empty() {
            header.push(("Attachments".into(), files.join(", ")));
        }
        let doc = quatrelle_print::Doc { title: m.subject.clone(), header, body: m.text.clone() };
        let fonts = self.print_fonts.as_ref().map(|(r, b)| quatrelle_print::Fonts { regular: r, bold: b.as_deref() });
        quatrelle_print::pdf(&doc, fonts).map_err(CmdError::Failed)
    }
}

pub(super) fn save_attachment(s: &mut Session, v: &Value) -> CmdResult {
    let index = p::u64(v, "index").and_then(|i| usize::try_from(i).ok()).ok_or_else(|| CmdError::Params("`index` is required".into()))?;
    let path = p::req_str(v, "path")?;
    // Opened from a message, a program, script or shortcut would run: never handed on.
    if p::bool(v, "open").unwrap_or(false) {
        let leaf = std::path::Path::new(path).file_name().unwrap_or_default();
        let name = leaf.to_string_lossy().into_owned();
        if quatrelle_mime::runs_code(&name) {
            return Err(CmdError::Disabled(format!("{name} could run a program, so it isn't opened from here; save it and check it first")));
        }
    }
    let data = s.attachment_bytes(index)?;
    std::fs::write(path, &data).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    mark_from_internet(std::path::Path::new(path));
    Ok(json!({"saved": path, "bytes": data.len()}))
}

/// Windows' mark that a file came from the internet (its `Zone.Identifier` stream, zone 3), as
/// browsers and mail programs add: SmartScreen checks a program before it runs, and Office opens
/// the file in Protected View. Best effort; nothing elsewhere.
fn mark_from_internet(path: &std::path::Path) {
    if cfg!(windows) {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        let _ = std::fs::write(
            stream,
            "[ZoneTransfer]
ZoneId=3
",
        );
    }
}

/// Save every attachment (not the pictures inside the text) into a folder; names that are
/// taken get a number.
pub(super) fn save_all_attachments(s: &mut Session, v: &Value) -> CmdResult {
    let dir = std::path::PathBuf::from(p::req_str(v, "dir")?);
    if !dir.is_dir() {
        return Err(CmdError::Params(format!("{} is not a folder", dir.display())));
    }
    let r = s.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
    let parsed = r.parsed.as_ref().ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?;
    let raw = s.readable_raw(&r.key).ok_or_else(|| CmdError::Failed("the message isn't downloaded".into()))?;
    // Only some of them, when asked (the ones selected).
    let only: Option<Vec<u64>> = v.get("indices").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).take(10_000).collect());
    let mut saved = Vec::new();
    for a in parsed.attachments.iter().filter(|a| !a.inline && only.as_ref().is_none_or(|o| o.contains(&(a.index as u64)))) {
        let Some(data) = quatrelle_mime::attachment_data(&raw, a.index) else { continue };
        let name = sanitize_filename(&a.name);
        let (stem, ext) = name.rsplit_once('.').map_or((name.clone(), String::new()), |(s, e)| (s.to_string(), format!(".{e}")));
        let mut path = dir.join(&name);
        let mut n = 2;
        while path.exists() && n < 1000 {
            path = dir.join(format!("{stem} ({n}){ext}"));
            n += 1;
        }
        std::fs::write(&path, &data).map_err(|e| CmdError::Failed(format!("{}: {e}", path.display())))?;
        mark_from_internet(&path);
        saved.push(path.to_string_lossy().to_string());
    }
    Ok(json!({"saved": saved}))
}

/// An attachment as text, for previewing (`message` kind: an attached email, parsed).
pub(super) fn preview_attachment(s: &mut Session, v: &Value) -> CmdResult {
    let index = p::u64(v, "index").and_then(|i| usize::try_from(i).ok()).ok_or_else(|| CmdError::Params("`index` is required".into()))?;
    let r = s.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
    let parsed = r.parsed.as_ref().ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?;
    let a = parsed.attachments.iter().find(|a| a.index == index).ok_or_else(|| CmdError::Params(format!("no attachment {index}")))?;
    let kind = quatrelle_mime::text_preview_kind(&a.mime, &a.name).ok_or_else(|| CmdError::Failed(format!("{} can't be shown as text", a.name)))?;
    let raw = s.readable_raw(&r.key).ok_or_else(|| CmdError::Failed("the message isn't downloaded".into()))?;
    let data = quatrelle_mime::attachment_data(&raw, index).ok_or_else(|| CmdError::Failed("the attachment can't be read".into()))?;
    if kind == "message" {
        let m = quatrelle_mime::parse(&data).ok_or_else(|| CmdError::Failed("the attached message can't be read".into()))?;
        let name = a.name.clone();
        let warning = s.sender_warning(&m.from);
        return Ok(json!({
            "kind": "message", "name": name, "subject": m.subject, "from": m.from.to_string(), "to": quatrelle_model::addr::format_list(&m.to),
            "date": m.date, "text": m.text, "warning": warning,
        }));
    }
    // At most 512 KB, cut on a character boundary.
    const MAX: usize = 512 << 10;
    let text = String::from_utf8_lossy(data.get(..MAX).unwrap_or(&data)).to_string();
    Ok(json!({"kind": "text", "name": a.name, "text": text, "truncated": data.len() > MAX}))
}

/// The open message's source, as an `.eml` file.
pub(super) fn save_message(s: &mut Session, v: &Value) -> CmdResult {
    let path = p::req_str(v, "path")?;
    let raw = s.message_source()?;
    std::fs::write(path, &raw).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    Ok(json!({"saved": path, "bytes": raw.len()}))
}

/// The open message's Internet headers, as its source carries them: who really sent it, the
/// servers it passed through, what they checked (Outlook shows them in a message's Properties).
pub(super) fn message_headers(s: &mut Session, _: &Value) -> CmdResult {
    let raw = s.message_source()?;
    // Up to the blank line that ends them (a source without one is all header).
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").or_else(|| raw.windows(2).position(|w| w == b"\n\n")).unwrap_or(raw.len());
    // At most 256 KB (a header that long is a broken message, not one to read).
    const MAX: usize = 256 << 10;
    let head = raw.get(..end.min(MAX)).unwrap_or(&raw);
    let text = String::from_utf8_lossy(head).replace("\r\n", "\n");
    Ok(json!({"kind": "headers", "text": text, "truncated": end > MAX}))
}

/// The messages selected, as `.eml` files, as Outlook's Save As from the list: one to `path`, or
/// each in `dir` under its subject (a name taken already gets a number). Those not on this
/// computer are counted, not saved.
pub(super) fn save_messages(s: &mut Session, v: &Value) -> CmdResult {
    let keys: Vec<MessageKey> = match v.get("keys").and_then(Value::as_array) {
        Some(list) => list.iter().filter_map(|k| k.as_str().and_then(MessageKey::from_id)).collect(),
        None => s.selection.clone(),
    };
    if keys.is_empty() {
        return Err(CmdError::Disabled("select messages first".into()));
    }
    let write = |path: &std::path::Path, raw: &[u8]| std::fs::write(path, raw).map_err(|e| CmdError::Failed(format!("{}: {e}", path.display())));
    if let Some(path) = p::str(v, "path") {
        let [key] = keys.as_slice() else { return Err(CmdError::Params("`path` is for one message; `dir` for several".into())) };
        let raw = s.source_of(key).ok_or_else(|| CmdError::Failed("the message isn't downloaded".into()))?;
        write(std::path::Path::new(path), &raw)?;
        return Ok(json!({"saved": [path], "missing": 0}));
    }
    let dir = std::path::PathBuf::from(p::req_str(v, "dir")?);
    let (mut saved, mut missing) = (Vec::new(), 0usize);
    for key in &keys {
        let Some(raw) = s.source_of(key) else {
            missing += 1;
            continue;
        };
        let name = s.message_file(key, "eml");
        let stem = name.trim_end_matches(".eml").to_string();
        let path = (1..1000)
            .map(|n| dir.join(if n == 1 { name.clone() } else { format!("{stem} ({n}).eml") }))
            .find(|p| !p.exists())
            .unwrap_or_else(|| dir.join(&name));
        write(&path, &raw)?;
        saved.push(path.to_string_lossy().to_string());
    }
    Ok(json!({"saved": saved, "missing": missing}))
}

/// The open message as a PDF.
pub(super) fn export_pdf(s: &mut Session, v: &Value) -> CmdResult {
    let path = p::req_str(v, "path")?;
    let bytes = s.message_pdf()?;
    std::fs::write(path, &bytes).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    Ok(json!({"saved": path, "bytes": bytes.len()}))
}
