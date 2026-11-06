// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Commands: the files kept with an appointment or a contact (a `.pst` import's, a calendar's
//! inline `ATTACH`), saved where asked, or into the folder an app opens them from, as a
//! message's attachments are (`attachment.save`).

use super::*;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("event.saveFile", "Save a file of an appointment", "Calendar › Event", |s, v| {
            let uid = p::req_str(v, "uid")?;
            let (occurrence, index) = (p::i64(v, "occurrence"), index(v)?);
            let path = p::req_str(v, "path")?;
            let open = p::bool(v, "open").unwrap_or(false);
            // One left on the server (an Exchange appointment's, R25): asked of it, written when it
            // comes. Never a program to be opened, as for one here.
            if let Some(f) = s.event_files(uid, occurrence).get(index).filter(|f| f.on_server) {
                runs_nothing(&f.name, path, open)?;
            }
            if let Some(name) = s.fetch_event_file(uid, occurrence, index, path, open)? {
                return Ok(json!({"fetching": name, "path": path}));
            }
            let (name, data) = s.event_file(uid, occurrence, index)?;
            save(&name, &data, v)
        })
        .params(r#"{"uid": string, "occurrence"?: unix seconds (a series' occurrence), "index": number (its files from 0), "path": string, "open"?: bool (never a program); a file on the server is downloaded first: {"fetching": name}, written when it comes"#),
        CommandSpec::new("contact.saveFile", "Save a file of a contact", "People › Contact", |s, v| {
            let uid = p::req_str(v, "uid")?;
            let (name, data) = s.card_file(uid, index(v)?)?;
            save(&name, &data, v)
        })
        .params(r#"{"uid": string, "index": number (their files from 0), "path": string, "open"?: bool (never a program)}"#),
    ]
}

fn index(v: &Value) -> Result<usize, CmdError> {
    p::u64(v, "index").and_then(|i| usize::try_from(i).ok()).ok_or_else(|| CmdError::Params("`index` is required".into()))
}

/// Write the file `name` to `path`, marked as come from elsewhere. To be opened, a program,
/// script or shortcut never is (it would run), as with a message's attachment.
fn save(name: &str, data: &[u8], v: &Value) -> CmdResult {
    let path = p::req_str(v, "path")?;
    runs_nothing(name, path, p::bool(v, "open").unwrap_or(false))?;
    std::fs::write(path, data).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    compose::mark_from_internet(std::path::Path::new(path));
    Ok(json!({"saved": path, "bytes": data.len()}))
}

/// A file to be opened (`open`) that could run a program (by its name or the path's) is refused.
fn runs_nothing(name: &str, path: &str, open: bool) -> Result<(), CmdError> {
    if open {
        let leaf = std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy().into_owned();
        if quatrelle_mime::runs_code(&leaf) || quatrelle_mime::runs_code(name) {
            return Err(CmdError::Disabled(format!("{name} could run a program, so it isn't opened from here; save it and check it first")));
        }
    }
    Ok(())
}
