// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Pictures put into the message being written: where they go, and the cursor after them.

use super::*;

impl MailApp {
    /// Put a picture's mark at the body's cursor.
    /// Where a picture goes: at the cursor in the message.
    pub(crate) fn picture_at(&self) -> usize {
        let n = self.session.compose.as_ref().map_or(0, |c| c.body.chars().count());
        self.ui.editor.as_ref().filter(|e| self.session.compose.as_ref().is_some_and(|c| c.id == e.id)).map_or(n, |e| e.sel.start.min(n))
    }

    /// A picture into the message being written, at the cursor (`params`: its `path`, or its
    /// `name` and `data`; `size`: its own, from its header): one wider than a page at a page's
    /// width, and a wide one on a line of its own, as Outlook puts a screenshot in (from
    /// upstream 865a712); the cursor then goes after it.
    pub(crate) fn insert_picture(&mut self, mut params: serde_json::Value, size: Option<(u32, u32)>) {
        let (width, line) = rich_edit::placement(size);
        params["at"] = json!(self.picture_at());
        if let Some(w) = width {
            params["width"] = json!(w);
        }
        if line {
            params["line"] = json!(true);
        }
        let r = self.session.run("compose.insertPicture", &params);
        self.place_picture(r);
    }

    pub(crate) fn place_picture(&mut self, r: quatrelle_engine::CmdResult) {
        match r {
            // In place: the cursor goes after it (after the line break that follows a picture
            // on a line of its own, where the command says it ends).
            Ok(v) => {
                if let Some(e) = self.ui.editor.as_mut() {
                    let end = v["end"].as_u64().and_then(|n| usize::try_from(n).ok()).unwrap_or(e.sel.start + 1);
                    e.sel = end..end;
                }
            }
            Err(e) => self.session.notice(e.to_string(), true),
        }
    }
}
