// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Commands: the View tab and the options.

use super::compose::insert_signature;
use super::*;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        // View
        CommandSpec::new("view.readingPane", "Reading pane", "View › Layout", reading_pane).params(r#"{"value": "right"|"bottom"|"off"}"#),
        CommandSpec::new("view.zoom", "Zoom", "View › Zoom", |s, v| {
            let percent = p::u64(v, "percent").ok_or_else(|| CmdError::Params("`percent` is required (50 to 400)".into()))?;
            let (low, high) = (u64::from(*crate::READING_ZOOM.start()), u64::from(*crate::READING_ZOOM.end()));
            s.prefs.reading_zoom = u16::try_from(percent.clamp(low, high)).unwrap_or(100);
            Ok(json!({"percent": s.prefs.reading_zoom}))
        })
        .params(r#"{"percent": number (the message read, 50 to 400; 100 as written)}"#),
        CommandSpec::new("view.conversations", "Show as conversations", "View › Messages", |s, v| {
            s.prefs.conversations = p::bool(v, "on").unwrap_or(!s.prefs.conversations);
            s.refresh_list();
            s.rethread_reading();
            Ok(json!({"on": s.prefs.conversations}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.preview", "Message preview", "View › Messages", |s, v| {
            s.prefs.show_preview = p::bool(v, "on").unwrap_or(!s.prefs.show_preview);
            Ok(json!({"on": s.prefs.show_preview}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.compact", "Compact list", "View › Layout", |s, v| {
            s.prefs.compact = p::bool(v, "on").unwrap_or(!s.prefs.compact);
            Ok(json!({"on": s.prefs.compact}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.todoBar", "To-Do bar", "View › Layout", |s, v| {
            s.prefs.todo_bar = p::bool(v, "on").unwrap_or(!s.prefs.todo_bar);
            Ok(json!({"on": s.prefs.todo_bar}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.listLayout", "Message list layout", "View › Layout", |s, v| {
            let value = p::req_str(v, "value")?;
            s.prefs.list_layout =
                crate::session::ListLayout::from_code(value).ok_or_else(|| CmdError::Params(format!("`{value}` isn't auto, cards or table")))?;
            Ok(json!({"value": s.prefs.list_layout.code(), "table": s.list_as_table()}))
        })
        .params(r#"{"value": "auto" (a table when the reading pane is at the bottom or off)|"cards"|"table"}"#),
        CommandSpec::new("view.arrange", "Arrange by", "View › Arrangement", |s, v| {
            let by = p::req_str(v, "by")?;
            s.prefs.arrange =
                crate::session::Arrange::from_code(by).ok_or_else(|| CmdError::Params(format!("`{by}` isn't date, from, to, subject, size, flag or attachments")))?;
            // A new arrangement starts in its natural order.
            s.prefs.reverse_sort = p::bool(v, "reverse").unwrap_or(false);
            s.refresh_list();
            Ok(json!({"by": s.prefs.arrange.code(), "reverse": s.prefs.reverse_sort}))
        })
        .params(r#"{"by": "date"|"from"|"to"|"subject"|"size"|"flag"|"attachments", "reverse"?: bool}"#),
        CommandSpec::new("view.reverseSort", "Reverse sort", "View › Arrangement", |s, v| {
            s.prefs.reverse_sort = p::bool(v, "on").unwrap_or(!s.prefs.reverse_sort);
            s.refresh_list();
            Ok(json!({"on": s.prefs.reverse_sort}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.folderPane", "Minimize the folder pane", "View › Layout", |s, v| {
            s.prefs.folder_pane_minimized = p::bool(v, "minimized").unwrap_or(!s.prefs.folder_pane_minimized);
            Ok(json!({"minimized": s.prefs.folder_pane_minimized}))
        })
        .params(r#"{"minimized"?: bool}"#),
        CommandSpec::new("view.collapse", "Collapse or expand in the folder pane", "Folder pane", |s, v| {
            let key = p::req_str(v, "key")?.trim();
            if key.is_empty() || key.len() > 600 {
                return Err(CmdError::Params("`key` names what to collapse: favorites, account:<id> or folder:<account>/<path>".into()));
            }
            let collapse = p::bool(v, "collapsed").unwrap_or(!s.prefs.collapsed.contains(key));
            if collapse {
                // A bounded list: the oldest go first when there are very many.
                while s.prefs.collapsed.len() >= 2_000 {
                    let Some(first) = s.prefs.collapsed.iter().next().cloned() else { break };
                    s.prefs.collapsed.remove(&first);
                }
                s.prefs.collapsed.insert(key.to_string());
            } else {
                s.prefs.collapsed.remove(key);
            }
            Ok(json!({"key": key, "collapsed": collapse}))
        })
        .params(r#"{"key": string (favorites, account:<id> or folder:<account>/<path>), "collapsed"?: bool (default: the other way)}"#),
        CommandSpec::new("view.focused", "Focused Inbox", "View › Messages", |s, v| {
            s.prefs.focused_inbox = p::bool(v, "on").unwrap_or(!s.prefs.focused_inbox);
            s.focus_other = false;
            s.refresh_list();
            Ok(json!({"on": s.prefs.focused_inbox}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("list.focusTab", "Focused / Other", "Message list", |s, v| {
            s.focus_other = match p::req_str(v, "value")? {
                x if x.eq_ignore_ascii_case("other") => true,
                x if x.eq_ignore_ascii_case("focused") => false,
                x => return Err(CmdError::Params(format!("unknown tab `{x}`"))),
            };
            s.selection.clear();
            s.reading = None;
            s.refresh_list();
            Ok(json!({"messages": s.list.len()}))
        })
        .params(r#"{"value": "focused"|"other"}"#),
        CommandSpec::new("view.confirmDelete", "Ask before deleting for good", "File › Options › Mail", |s, v| {
            s.prefs.confirm_permanent_delete = p::bool(v, "on").unwrap_or(!s.prefs.confirm_permanent_delete);
            Ok(json!({"on": s.prefs.confirm_permanent_delete}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.notifications", "Desktop notifications", "File › Options › Mail", |s, v| {
            s.prefs.notify = p::bool(v, "on").unwrap_or(!s.prefs.notify);
            Ok(json!({"on": s.prefs.notify}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.envelope", "Envelope in the notification area", "File › Options › Mail", |s, v| {
            s.prefs.envelope = p::bool(v, "on").unwrap_or(!s.prefs.envelope);
            Ok(json!({"on": s.prefs.envelope}))
        })
        .params(r#"{"on"?: bool} (an envelope in the notification area while new mail is unread)"#),
        CommandSpec::new("view.keepRunning", "Keep running in the notification area", "File › Options › Mail", |s, v| {
            s.prefs.keep_running = p::bool(v, "on").unwrap_or(!s.prefs.keep_running);
            Ok(json!({"on": s.prefs.keep_running}))
        })
        .params(r#"{"on"?: bool} (closing the window leaves Quatrelle running, its icon in the notification area bringing it back)"#),
        CommandSpec::new("view.remindersOnTop", "Show reminders on top of other windows", "File › Options › Mail", |s, v| {
            s.prefs.reminders_on_top = p::bool(v, "on").unwrap_or(!s.prefs.reminders_on_top);
            Ok(json!({"on": s.prefs.reminders_on_top}))
        })
        .params(r#"{"on"?: bool} (the Reminders window in a window of its own, over every other)"#),
        CommandSpec::new("calendar.webexRoom", "Webex rooms", "File › Options › Calendar", |s, v| {
            let url = p::req_str(v, "url")?;
            let text = |k: &str| match v.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(x)) => Ok(Some(x.as_str())),
                Some(_) => Err(CmdError::Params(format!("`{k}` is a string"))),
            };
            let remove = match v.get("remove") {
                None | Some(Value::Null) => false,
                Some(Value::Bool(b)) => *b,
                Some(_) => return Err(CmdError::Params("`remove` is true or false".into())),
            };
            s.prefs.set_webex_room(url, text("was")?, text("name")?, room_lines(v)?, remove)?;
            Ok(json!({"rooms": s.prefs.webex_rooms}))
        })
        .params(r#"{"url": "https://<site>.webex.com/meet/<name>", "was"?: string (the link it had: that room's link changes, in its place), "name"?: string (required for a new room; at most 80 characters), "details"?: string ("\n" between its lines) | [string] (at most 20 lines of 200 characters, copied into the notes after the link), "remove"?: bool (takes the room with that link out)} → {"rooms": [{"name", "url", "details"}]} (adds a room at the end, changes the one with that link, or removes it; the rooms the appointment window's Webex Meeting button offers)"#),
        CommandSpec::new("assistant.settings", "Assistant", "File › Options › Assistant", |s, v| {
            s.set_assistant(p::str(v, "url").unwrap_or_default(), p::str(v, "model").unwrap_or_default(), p::str(v, "key"))
        })
        .params(r#"{"url": string (an OpenAI-compatible server, …/v1; "" for none), "model"?: string, "key"?: string (kept in the keychain; "" forgets it)}"#),
        CommandSpec::new("assistant.models", "List the models", "File › Options › Assistant", |s, _| s.list_llm_models())
            .when(|s| if s.prefs.llm_url.trim().is_empty() { Some("set the language model's server first") } else { None }),
        CommandSpec::new("assistant.summarize", "Summarize", "Reading pane", |s, _| s.summarize())
            .when(|s| if s.prefs.llm_url.trim().is_empty() { Some("set a language model in File › Options › Assistant first") } else { None }),
        CommandSpec::new("assistant.suggestReply", "Suggest a reply", "Reading pane", |s, _| s.suggest_reply())
            .when(|s| if s.prefs.llm_url.trim().is_empty() { Some("set a language model in File › Options › Assistant first") } else { None }),
        CommandSpec::new("assistant.suggestFiling", "Suggest where it goes", "Reading pane", |s, _| s.suggest_filing())
            .when(|s| if s.prefs.llm_url.trim().is_empty() { Some("set a language model in File › Options › Assistant first") } else { None }),
        CommandSpec::new("assistant.replyWithSuggestion", "Reply with this text", "Reading pane", |s, _| s.reply_with_suggestion()),
        CommandSpec::new("assistant.close", "Close", "Reading pane", |s, _| s.close_assistant()),
        CommandSpec::new("assistant.rewrite", "Rewrite", "Message › Review", |s, v| s.rewrite(p::str(v, "how").unwrap_or("fix")))
            .params(r#"{"how": "fix"|"shorter"|"formal"|"friendly"|"english"} (the text written, up to the signature or the message quoted)"#),
        CommandSpec::new("assistant.endRewrite", "Use the rewritten text", "Message › Review", |s, v| s.end_rewrite(p::bool(v, "use").unwrap_or(false)))
            .params(r#"{"use": bool} (true: in place of the text written; false: let it go)"#),
        CommandSpec::new("assistant.scamCheck", "Spot scams in external mail", "File › Options › Assistant", |s, v| {
            let accounts = match v.get("accounts") {
                None | Some(Value::Null) => None,
                Some(Value::Array(list)) => Some(
                    list.iter().map(|a| a.as_str().map(str::to_string).ok_or_else(|| CmdError::Params("`accounts` is a list of account ids".into()))).collect::<Result<Vec<_>, _>>()?,
                ),
                Some(_) => return Err(CmdError::Params("`accounts` is a list of account ids".into())),
            };
            s.set_scam_check(p::bool(v, "enabled"), accounts, p::i64(v, "dailyCap"))
        })
        .params(r#"{"enabled"?: bool (needs a server and a model set), "accounts"?: [string] (the accounts it applies to, any with a server; turned on with none chosen: the Exchange ones, else all of them), "dailyCap"?: number (1 to 2000 messages a day, 200 by default)} → {"enabled", "accounts", "dailyCap"}"#),
        CommandSpec::new("assistant.testScam", "Try the scam check on examples", "File › Options › Assistant", |s, _| s.start_scam_test())
            .params(r#"{} → {"running", "done", "total", "scams", "spotted", "normals", "leftAlone", "unanswered", "misses": [string], "secondsPerMessage"} (twenty messages written for Quatrelle go to the model one at a time; nothing of yours is sent)"#),
        CommandSpec::new("scam.notScam", "Not a scam", "Reading pane", |s, v| {
            let k = parse_key(v).or_else(|| s.reading.as_ref().map(|r| r.key.clone())).ok_or_else(|| CmdError::Params("`key` is required".into()))?;
            s.not_a_scam(&k)
        })
        .params(r#"{"key"?: string (the message on screen when not given)} → {"trusted", "address"} (its verdict goes; the sender is never sent to the model again)"#),
        CommandSpec::new("scam.untrust", "Take back a trusted sender", "File › Options › Assistant", |s, v| s.scam_untrust(p::req_str(v, "address")?))
            .params(r#"{"address": string} → {"removed"}"#),
        CommandSpec::new("scam.verdict", "What the scam check found", "Reading pane", |s, v| {
            let k = parse_key(v).or_else(|| s.reading.as_ref().map(|r| r.key.clone())).ok_or_else(|| CmdError::Params("`key` is required".into()))?;
            Ok(match s.scam_verdict(&k) {
                Some((score, reasons)) => json!({"score": score, "reasons": reasons}),
                None => Value::Null,
            })
        })
        .params(r#"{"key"?: string (the message on screen when not given)} → {"score": 0-100, "reasons": [string]} or null"#),
        CommandSpec::new("view.newMailWindow", "New messages in their own window", "File › Options › Mail", |s, v| {
            s.prefs.new_mail_window = p::bool(v, "on").unwrap_or(!s.prefs.new_mail_window);
            Ok(json!({"on": s.prefs.new_mail_window}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.replyWindow", "Replies and forwards in their own window", "File › Options › Mail", |s, v| {
            s.prefs.reply_window = p::bool(v, "on").unwrap_or(!s.prefs.reply_window);
            Ok(json!({"on": s.prefs.reply_window}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.favorites", "Show Favorites", "View › Layout", |s, v| {
            s.prefs.show_favorites = p::bool(v, "on").unwrap_or(!s.prefs.show_favorites);
            Ok(json!({"on": s.prefs.show_favorites}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.markReadOnOpen", "Mark as read when opened", "File › Options › Mail", |s, v| {
            s.prefs.mark_read_on_open = p::bool(v, "on").unwrap_or(!s.prefs.mark_read_on_open);
            Ok(json!({"on": s.prefs.mark_read_on_open}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("calendar.show", "Show or hide a calendar", "Calendar › Calendars", |s, v| {
            let account = p::req_str(v, "account")?.to_string();
            let calendar = p::req_str(v, "calendar")?.to_string();
            if !s.pim.get(&account).is_some_and(|p| p.calendars.iter().any(|c| c.id == calendar)) {
                return Err(CmdError::Params(format!("no calendar `{calendar}` in `{account}`")));
            }
            let key = Session::calendar_key(&account, &calendar);
            let shown = p::bool(v, "shown").unwrap_or(s.prefs.hidden_calendars.contains(&key));
            if shown {
                s.prefs.hidden_calendars.remove(&key);
            } else {
                s.prefs.hidden_calendars.insert(key);
            }
            s.pim_revision = s.pim_revision.wrapping_add(1);
            Ok(json!({"account": account, "calendar": calendar, "shown": shown}))
        })
        .params(r#"{"account": string, "calendar": string, "shown"?: bool (default: the other way)}"#),
        CommandSpec::new("calendar.sideBySide", "Side by side", "Calendar › View", |s, v| {
            s.prefs.calendars_side_by_side = p::bool(v, "on").unwrap_or(!s.prefs.calendars_side_by_side);
            Ok(json!({"on": s.prefs.calendars_side_by_side}))
        })
        .params(r#"{"on"?: bool (each calendar in a column of its own in day and week)}"#),
        CommandSpec::new("calendar.months", "Months in the side pane", "Calendar › Side pane", |s, v| {
            let n = p::u64(v, "count").ok_or_else(|| CmdError::Params("`count` is required (1 to 6)".into()))?;
            s.prefs.calendar_months = u8::try_from(n.clamp(1, 6)).unwrap_or(1);
            Ok(json!({"count": s.prefs.calendar_months}))
        })
        .params(r#"{"count": number (1 to 6)}"#),
        CommandSpec::new("calendar.showOnly", "Show only this calendar", "Calendar › Calendars", |s, v| {
            s.show_only_calendar(p::req_str(v, "account")?, p::req_str(v, "calendar")?)
        })
        .params(r#"{"account": string, "calendar": string} (every other calendar, of every account, hidden)"#),
        CommandSpec::new("calendar.color", "Calendar color", "Calendar › Calendars", |s, v| {
            let color = match v.get("color") {
                None => return Err(CmdError::Params("`color` is required (0 to 15, 16, or null)".into())),
                Some(Value::Null) => None,
                Some(c) => Some(c.as_u64().and_then(|n| u8::try_from(n).ok()).ok_or_else(|| CmdError::Params("`color` is 0 to 15, 16, or null".into()))?),
            };
            s.set_calendar_color(p::req_str(v, "account")?, p::req_str(v, "calendar")?, color)
        })
        .params(r#"{"account": string, "calendar": string, "color": 0..15 (one of the palette's 16) | 16 (the account's) | null (automatic: the server's)} (kept on this computer)"#),
        CommandSpec::new("calendar.moveUp", "Move up", "Calendar › Calendars", |s, v| s.move_calendar(p::req_str(v, "account")?, p::req_str(v, "calendar")?, true))
            .params(r#"{"account": string, "calendar": string} (before the one above it in its group, kept on this computer)"#),
        CommandSpec::new("calendar.moveDown", "Move down", "Calendar › Calendars", |s, v| {
            s.move_calendar(p::req_str(v, "account")?, p::req_str(v, "calendar")?, false)
        })
        .params(r#"{"account": string, "calendar": string} (after the one below it in its group, kept on this computer)"#),
        CommandSpec::new("calendar.create", "New calendar", "Calendar › Calendars", |s, v| s.create_calendar(p::req_str(v, "account")?, p::req_str(v, "name")?))
            .params(r#"{"account": string, "name": string (up to 255 characters)} (made on the account's server, Microsoft 365's or Exchange's; it shows once the calendars are read again)"#)
            .when(calendars_made),
        CommandSpec::new("calendar.rename", "Rename calendar", "Calendar › Calendars", |s, v| {
            s.rename_calendar(p::req_str(v, "account")?, p::req_str(v, "calendar")?, p::req_str(v, "name")?)
        })
        .params(r#"{"account": string, "calendar": string, "name": string (up to 255 characters)} (one of the account's own, not its main one)"#)
        .when(calendars_made),
        CommandSpec::new("calendar.delete", "Delete calendar", "Calendar › Calendars", |s, v| s.delete_calendar(p::req_str(v, "account")?, p::req_str(v, "calendar")?))
            .params(r#"{"account": string, "calendar": string} (one of the account's own, not its main one; its events go with it, into Deleted Items)"#)
            .when(calendars_made),
        CommandSpec::new("calendar.remove", "Remove from the list", "Calendar › Calendars", |s, v| {
            s.remove_calendar(p::req_str(v, "account")?, p::req_str(v, "calendar")?)
        })
        .params(r#"{"account": string, "calendar": string} (on this computer only, never on the server: out of the side pane and the calendar, no longer read; not the account's main one, nor one opened by its owner's address or subscribed to by its link)"#),
        CommandSpec::new("calendar.restore", "Bring back to the list", "Calendar › Calendars", |s, v| {
            s.restore_calendar(p::req_str(v, "account")?, p::req_str(v, "calendar")?)
        })
        .params(r#"{"account": string, "calendar": string} → {"restored": bool} (a calendar removed from the list: listed, and read again)"#),
        CommandSpec::new("account.keepMail", "Mail kept on this computer", "File › Options › Mail", |s, v| {
            let days = p::u64(v, "days").ok_or_else(|| CmdError::Params("`days` is required (0: all)".into()))?;
            s.prefs.keep_days = u32::try_from(days.min(36_500)).unwrap_or(0);
            s.apply_sync_prefs();
            Ok(json!({"days": s.prefs.keep_days}))
        })
        .params(r#"{"days": number (mail received in the last days kept on this computer, whole; older mail is read on the server; 0: all)}"#),
        CommandSpec::new("account.onlineOnly", "Online only", "File › Options › Mail", |s, v| {
            let id = p::req_str(v, "account")?;
            let mut c = s.account(id).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
            c.online_only = p::bool(v, "on").unwrap_or(!c.online_only);
            s.update_account(c, None)?;
            let on = s.account(id).is_some_and(|a| a.config.keeps_online());
            Ok(json!({"account": id, "on": on}))
        })
        .params(r#"{"account": string, "on"?: bool (default: the other way; keep none of its mail on this computer, only in memory while Quatrelle runs)}"#),
        CommandSpec::new("note.arrange", "Arrange notes", "Notes › Arrange by", |s, v| {
            s.prefs.notes_order = match p::req_str(v, "by")? {
                "modified" => crate::session::NotesOrder::Modified,
                "title" => crate::session::NotesOrder::Title,
                "color" => crate::session::NotesOrder::Color,
                other => return Err(CmdError::Params(format!("`by` is modified, title or color, not `{other}`"))),
            };
            Ok(json!({"by": p::req_str(v, "by")?}))
        })
        .params(r#"{"by": "modified"|"title"|"color"}"#),
        CommandSpec::new("compose.insertSignature", "Insert signature", "Message › Signature", insert_signature)
            .params(r#"{"at"?: number (the character it goes at; default the end)}"#),
        CommandSpec::new("view.readingOptions", "Reading pane options", "File › Options › Mail", |s, v| {
            if let Some(on) = p::bool(v, "markRead") {
                s.prefs.mark_read_on_open = on;
            }
            match p::str(v, "when") {
                Some("shown") => s.prefs.read_in_pane = crate::session::ReadInPane::Shown,
                Some("left") => s.prefs.read_in_pane = crate::session::ReadInPane::Left,
                Some(other) => return Err(CmdError::Params(format!("`when` is shown or left, not `{other}`"))),
                None => {}
            }
            if let Some(secs) = p::u64(v, "after") {
                s.prefs.read_after_secs = u32::try_from(secs.min(999)).unwrap_or(999);
            }
            let when = match s.prefs.read_in_pane {
                crate::session::ReadInPane::Shown => "shown",
                crate::session::ReadInPane::Left => "left",
            };
            Ok(json!({"markRead": s.prefs.mark_read_on_open, "when": when, "after": s.prefs.read_after_secs}))
        })
        .params(r#"{"markRead"?: bool (in the reading pane at all; opening a message in its own window always does), "when"?: "shown"|"left" (when it's shown, or once another is chosen), "after"?: number (seconds shown first, with "shown")}"#),
        CommandSpec::new("view.animations", "Play animations", "File › Options › General", |s, v| {
            s.prefs.animations = p::bool(v, "on").unwrap_or(!s.prefs.animations);
            Ok(json!({ "on": s.prefs.animations }))
        })
        .params(r#"{"on"?: bool (none: switch)}"#),
        CommandSpec::new("view.photoLocation", "Send where photos were taken", "File › Options › Safety", |s, v| {
            s.prefs.keep_photo_location = p::bool(v, "on").unwrap_or(!s.prefs.keep_photo_location);
            Ok(json!({ "on": s.prefs.keep_photo_location }))
        })
        .params(r#"{"on"?: bool (none: switch; off, the place is taken out of attached photos as they're sent)}"#),
        CommandSpec::new("view.encryptWhenPossible", "Encrypt by itself when everyone can receive it", "File › Options › Safety", |s, v| {
            s.prefs.encrypt_when_possible = p::bool(v, "on").unwrap_or(!s.prefs.encrypt_when_possible);
            Ok(json!({ "on": s.prefs.encrypt_when_possible }))
        })
        .params(r#"{"on"?: bool (none: switch; on, a new message is encrypted while every recipient can receive it, until Encrypt is chosen)}"#),
        CommandSpec::new("view.cleanLinks", "Open links without their tracking", "File › Options › Safety", |s, v| {
            s.prefs.clean_links = p::bool(v, "on").unwrap_or(!s.prefs.clean_links);
            Ok(json!({ "on": s.prefs.clean_links }))
        })
        .params(r#"{"on"?: bool (none: switch; on, a link opens out of its redirect wrapper and without the tags naming the reader)}"#),
        CommandSpec::new("view.checkRevocation", "Check whether certificates were withdrawn", "File › Options › Safety", |s, v| {
            s.prefs.check_revocation = p::bool(v, "on").unwrap_or(!s.prefs.check_revocation);
            Ok(json!({ "on": s.prefs.check_revocation }))
        })
        .params(r#"{"on"?: bool (none: switch; on, a signed message's certificate is checked with its authority as it's shown, which tells the authority)}"#),
        CommandSpec::new("view.senderAddresses", "Show sender addresses in the message list", "File › Options › Safety", |s, v| {
            let show = p::req_str(v, "show")?;
            s.prefs.sender_addresses = crate::SenderAddresses::from_code(show).ok_or_else(|| {
                CmdError::Params(format!("`show` is never, strangers or always, not `{}`", show.chars().take(40).collect::<String>()))
            })?;
            Ok(json!({"show": s.prefs.sender_addresses.code()}))
        })
        .params(r#"{"show": "never"|"strangers"|"always" (strangers: for senders who aren't contacts, written to or in the directory's answers)}"#),
        CommandSpec::new("view.externalTag", "Tag mail from outside the organisation", "File › Options › Safety", external_tag).params(
            r#"{"on"?: bool (none: as it is), "domains"?: [string] | string (the organisation's domains, at most 100, separated by commas or spaces; empty: those of your accounts, mail services left out)}"#,
        ),
        CommandSpec::new("emoji.used", "Emoji", "Message › Insert › Emoji", |s, v| {
            let emoji = p::req_str(v, "emoji")?;
            if !s.prefs.emoji_used(emoji) {
                return Err(CmdError::Params(format!("`{emoji}` isn't an emoji")));
            }
            Ok(json!({ "recent": s.prefs.recent_emoji }))
        })
        .params(r#"{"emoji": string (put in a message: first among the recent ones Insert › Emoji shows)}"#),
        CommandSpec::new("view.pictures", "Load pictures from the web", "File › Options › Privacy", |s, v| {
            let value = p::req_str(v, "value")?;
            let to = crate::PictureLoading::from_code(value).ok_or_else(|| CmdError::Params(format!("`{value}` isn't ask, known or always")))?;
            s.set_picture_loading(to)
        })
        .params(r#"{"value": "ask" (only when asked, for a message or a sender)|"known" (also from the people saved in Contacts or written to, and the safe senders)|"always"; mail in Junk Email waits to be asked whatever it says}"#),
        CommandSpec::new("view.meetingBrowser", "Browser for Teams meetings", "File › Options › Applications", |s, v| {
            let path = match v.get("path") {
                None | Some(Value::Null) => None,
                Some(Value::String(path)) => Some(path.clone()),
                Some(_) => return Err(CmdError::Params("`path` must be a string or null".into())),
            };
            if let Some(path) = &path
                && (path.len() > 4096 || path.chars().any(char::is_control))
            {
                return Err(CmdError::Params("`path` is too long or has control characters".into()));
            }
            s.prefs.meeting_browser = path;
            Ok(json!({ "path": s.prefs.meeting_browser }))
        })
        .params(r#"{"path"?: string|null (none or null: the first Chrome, Chromium or Edge found; empty: the default browser; otherwise the program to run; Teams meetings open in its app window)}"#),
        CommandSpec::new("view.meetingProfile", "Browser profile for Teams meetings", "File › Options › Applications", |s, v| {
            let dir = match v.get("dir") {
                None | Some(Value::Null) => None,
                Some(Value::String(dir)) if crate::is_profile_dir(dir) => Some(dir.clone()),
                Some(_) => return Err(CmdError::Params("`dir` must be a profile folder's name or null".into())),
            };
            s.prefs.meeting_profile = dir;
            Ok(json!({ "dir": s.prefs.meeting_profile }))
        })
        .params(r#"{"dir"?: string|null (none or null: the profile the browser used last; otherwise that profile's folder, as "Default" or "Profile 1"; "Quatrelle": one the browser makes for meetings; Teams meetings open in it)}"#),
        CommandSpec::new("view.trustSender", "Show pictures from a sender", "File › Options › Privacy", |s, v| {
            let who = p::req_str(v, "address")?.trim().to_ascii_lowercase();
            if !quatrelle_model::addr::is_valid_email(&who) {
                return Err(CmdError::Params(format!("`{who}` is not an email address")));
            }
            if p::bool(v, "on").unwrap_or(true) {
                s.prefs.trusted_senders.insert(who.clone());
            } else {
                s.prefs.trusted_senders.remove(&who);
            }
            Ok(json!({"trusted": s.prefs.trusted_senders.iter().collect::<Vec<_>>()}))
        })
        .params(r#"{"address": string, "on"?: bool}"#),
    ]
}

/// Calendars are made, renamed and deleted where an account's server allows it (each calendar
/// says more: `Session::calendar_change`).
fn calendars_made(s: &Session) -> Option<&'static str> {
    if s.accounts.iter().any(|a| a.config.makes_calendars()) { None } else { Some("add a Microsoft 365 or Exchange account first") }
}

/// A Webex room's lines given (`details`): a string, `\n` between them, or a list of strings; the
/// blank ones left out and no more taken than one past the most (enough to refuse), whatever
/// their number.
fn room_lines(v: &Value) -> Result<Option<Vec<String>>, CmdError> {
    fn keep<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<String> {
        let blank = |l: &str| l.chars().all(|c| c.is_control() || c.is_whitespace());
        lines.filter(|l| !blank(l)).take(crate::WEBEX_ROOM_LINES_MAX + 1).map(str::to_string).collect()
    }
    match v.get("details") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(x)) => Ok(Some(keep(x.split('\n')))),
        Some(Value::Array(a)) if a.iter().all(Value::is_string) => Ok(Some(keep(a.iter().filter_map(Value::as_str)))),
        Some(_) => Err(CmdError::Params("`details` is the room's lines: a string (\"\\n\" between them) or a list of strings".into())),
    }
}

/// The External tag (issue #29): on or off, and the organisation's domains.
fn external_tag(s: &mut Session, v: &Value) -> CmdResult {
    const MOST: usize = 100;
    if let Some(given) = v.get("domains").filter(|d| !d.is_null()) {
        let words: Vec<&str> = match given {
            Value::String(text) => text.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).collect(),
            Value::Array(list) => {
                list.iter().map(|d| d.as_str().ok_or_else(|| CmdError::Params("`domains` lists domains as text".into()))).collect::<Result<_, _>>()?
            }
            _ => return Err(CmdError::Params("`domains` is a list of domains or a text".into())),
        };
        let mut domains: Vec<String> = Vec::new();
        for w in words.iter().map(|w| w.trim()).filter(|w| !w.is_empty()) {
            let d = quatrelle_model::lookalike::clean_domain(w)
                .ok_or_else(|| CmdError::Params(format!("`{}` isn't a domain", w.chars().take(80).collect::<String>())))?;
            if !domains.contains(&d) {
                if domains.len() >= MOST {
                    return Err(CmdError::Params(format!("at most {MOST} domains")));
                }
                domains.push(d);
            }
        }
        s.prefs.org_domains = domains;
    }
    if let Some(on) = p::bool(v, "on") {
        s.prefs.external_tag = on;
    }
    let organisation: Vec<String> = s.org_domains().into_iter().collect();
    Ok(json!({"on": s.prefs.external_tag, "domains": s.prefs.org_domains, "organisation": organisation}))
}

pub(super) fn reading_pane(s: &mut Session, v: &Value) -> CmdResult {
    s.prefs.reading_pane = match p::req_str(v, "value")?.to_ascii_lowercase().as_str() {
        "right" => ReadingPane::Right,
        "bottom" => ReadingPane::Bottom,
        "off" => ReadingPane::Off,
        other => return Err(CmdError::Params(format!("unknown reading pane position `{other}`"))),
    };
    Ok(json!({"value": p::req_str(v, "value")?}))
}
