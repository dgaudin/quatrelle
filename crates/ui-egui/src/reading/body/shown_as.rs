// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! One message as plain text, or back as sent, on its own (adapted from upstream ee0b46ad and
//! 01208ca2): the setting for all mail stays as it is. Or as Markdown: its formatting written
//! out as text ([`mime::rich_markdown`]), for this message too.

use std::sync::Arc;

use super::*;

/// The messages shown otherwise than the setting for all mail (File › Options, View › Plain
/// text) says: one message formatted though all mail is read as plain text, or the other way
/// round, as Outlook shows one message as HTML from above it and leaves the setting as it was.
/// And those shown as Markdown, whatever the setting.
#[derive(Default)]
pub struct ShownAs {
    /// The setting these were chosen against: changed, they're forgotten.
    against: bool,
    /// Each message's key, and whether it's shown formatted.
    each: std::collections::HashMap<MessageKey, bool>,
    /// The messages shown as Markdown.
    markdown: std::collections::HashSet<MessageKey>,
    /// The Markdown last made of the messages shown so (made of which text, links cleaned or
    /// not), so it isn't made again each frame.
    made: Vec<(MessageKey, Arc<mime::Parsed>, bool, Arc<str>)>,
}

/// More than this many messages switched, the oldest choices aren't worth keeping.
const SHOWN_AS_MAX: usize = 256;
/// The Markdown of this many messages is kept (the reading pane's, and a few windows').
const MADE_MAX: usize = 4;

impl ShownAs {
    /// Whether message `key` is shown formatted, `all` being the setting for all mail.
    pub fn formatted(&self, key: &MessageKey, all: bool) -> bool {
        if self.against != all {
            return all;
        }
        self.each.get(key).copied().unwrap_or(all)
    }

    /// Show message `key` formatted (`on`) or as plain text, whatever the setting `all` says
    /// (and no longer as Markdown).
    pub fn set(&mut self, key: &MessageKey, on: bool, all: bool) {
        self.markdown.remove(key);
        if self.against != all || self.each.len() >= SHOWN_AS_MAX {
            self.each.clear();
            self.against = all;
        }
        if on == all {
            self.each.remove(key);
        } else {
            self.each.insert(key.clone(), on);
        }
    }

    /// Whether message `key` is shown as Markdown.
    pub fn markdown(&self, key: &MessageKey) -> bool {
        self.markdown.contains(key)
    }

    /// Show message `key` as Markdown (`on`), or as before.
    pub fn set_markdown(&mut self, key: &MessageKey, on: bool) {
        if !on {
            self.markdown.remove(key);
            return;
        }
        if self.markdown.len() >= SHOWN_AS_MAX {
            self.markdown.clear();
        }
        self.markdown.insert(key.clone());
    }
}

/// Whether message `key` (parsed as `p`) is shown formatted: it has formatting, and it's shown
/// that way, by the setting or for itself (not as Markdown).
pub(crate) fn shown_formatted(s: &Session, st: &UiState, key: &MessageKey, p: &mime::Parsed) -> bool {
    !p.rich.is_empty() && !st.shown_as.markdown(key) && st.shown_as.formatted(key, s.prefs.show_html)
}

/// The Markdown message `key` (parsed as `p`) is shown as, when it is: its formatted lines
/// written out, its links cleaned when links are (as the formatted view shows them). Made once
/// for each text.
pub(crate) fn shown_markdown(s: &Session, st: &mut UiState, key: &MessageKey, p: &Arc<mime::Parsed>) -> Option<Arc<str>> {
    if p.rich.is_empty() || !st.shown_as.markdown(key) {
        return None;
    }
    let clean = s.prefs.clean_links;
    let made = &mut st.shown_as.made;
    if let Some((_, _, _, md)) = made.iter().find(|(k, q, c, _)| k == key && Arc::ptr_eq(q, p) && *c == clean) {
        return Some(md.clone());
    }
    let md: Arc<str> = Arc::from(mime::rich_markdown(&p.rich, clean));
    made.retain(|(k, _, _, _)| k != key);
    if made.len() >= MADE_MAX {
        made.remove(0);
    }
    made.push((key.clone(), p.clone(), clean, md.clone()));
    Some(md)
}

/// One message's "Show as plain text" (or, shown so, "Show formatted"; `formatted` is how it is
/// shown now) in its More actions (⋯), and, formatted, its "Show as Markdown":
/// here rather than over the text, where it was clicked by mistake, and for this message only
/// (File › Options and View › Plain text switch all mail).
pub(crate) fn plain_text_item(ui: &mut Ui, key: &MessageKey, p: &mime::Parsed, formatted: bool, acts: &mut Vec<Action>) {
    if !(p.had_html && !p.rich.is_empty()) {
        return;
    }
    let label = if formatted { tl!("Show as plain text") } else { tl!("Show formatted") };
    if ui.button((widgets::menu_icon(Some(Icon::ClearFormat)), label)).clicked() {
        acts.push(Action::ShowFormatted(key.clone(), !formatted));
        ui.close();
    }
    // Shown as text or as Markdown, the line above the text has the way to Markdown, or back.
    if formatted && ui.button((widgets::menu_icon(None), tl!("Show as Markdown"))).clicked() {
        acts.push(Action::ShowMarkdown(key.clone(), true));
        ui.close();
    }
}
