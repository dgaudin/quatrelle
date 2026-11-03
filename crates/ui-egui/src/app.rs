// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The window (`MailApp`), in parts: each continues `impl MailApp` with what it draws or does.

mod actions;
mod alt_code;
mod bars;
mod costs;
mod dropped;
mod file_dialogs;
mod frame;
mod keys;
mod meetings;
mod ribbon;
mod search_box;
mod sending;
mod updates;
mod webex;
mod windows;

pub use alt_code::{alt_code_pages, alt_codes};
pub(crate) use costs::Merging;
pub(crate) use file_dialogs::FileDialogs;
pub(crate) use meetings::{meeting_browser, meeting_profile};
pub(crate) use ribbon::junk_menu;
pub(crate) use search_box::{SearchDrop, scope_label};
pub use windows::{WindowPlace, after_open};
