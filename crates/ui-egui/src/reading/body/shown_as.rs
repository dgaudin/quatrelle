// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! One message as plain text, or back as sent, on its own (adapted from upstream ee0b46ad and
//! 01208ca2): the setting for all mail stays as it is.

use super::*;

/// The messages shown otherwise than the setting for all mail (File › Options, View › Plain
/// text) says: one message formatted though all mail is read as plain text, or the other way
/// round, as Outlook shows one message as HTML from above it and leaves the setting as it was.
#[derive(Default)]
pub struct ShownAs {
    /// The setting these were chosen against: changed, they're forgotten.
    against: bool,
    /// Each message's key, and whether it's shown formatted.
    each: std::collections::HashMap<MessageKey, bool>,
}

/// More than this many messages switched, the oldest choices aren't worth keeping.
const SHOWN_AS_MAX: usize = 256;

impl ShownAs {
    /// Whether message `key` is shown formatted, `all` being the setting for all mail.
    pub fn formatted(&self, key: &MessageKey, all: bool) -> bool {
        if self.against != all {
            return all;
        }
        self.each.get(key).copied().unwrap_or(all)
    }

    /// Show message `key` formatted (`on`) or as plain text, whatever the setting `all` says.
    pub fn set(&mut self, key: &MessageKey, on: bool, all: bool) {
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
}

/// Whether message `key` (parsed as `p`) is shown formatted: it has formatting, and it's shown
/// that way, by the setting or for itself.
pub(crate) fn shown_formatted(s: &Session, st: &UiState, key: &MessageKey, p: &mime::Parsed) -> bool {
    !p.rich.is_empty() && st.shown_as.formatted(key, s.prefs.show_html)
}

/// One message's "Show as plain text" (or, shown so, "Show formatted"; `formatted` is how it is
/// shown now) in its More actions (⋯):
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
}
