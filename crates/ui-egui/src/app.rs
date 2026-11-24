// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The window (`MailApp`), in parts: each continues `impl MailApp` with what it draws or does.

mod actions;
mod alt_code;
mod chrome;
mod costs;
mod dropped;
mod file_dialogs;
mod frame;
mod keys;
mod meetings;
mod sending;
pub(crate) mod source;
mod updates;
mod webex;
mod windows;

pub(crate) use actions::cleaned_on_written_host;
pub use alt_code::{alt_code_pages, alt_codes};
pub(crate) use chrome::{SearchDrop, scope_label};
pub(crate) use chrome::{junk_menu, quick_steps_menu, run_quick_step};
pub(crate) use costs::Merging;
pub(crate) use file_dialogs::FileDialogs;
pub(crate) use frame::FrameGuard;
pub use frame::FramePart;
pub(crate) use frame::{Caught, guarded_window};
#[cfg(test)]
pub(crate) use frame::{PanicAt, panic_if};
pub(crate) use keys::delete_from_window;
#[cfg(test)]
pub(crate) use meetings::{PageOutcome, ProfileKind, page_line};
pub(crate) use meetings::{meeting_browser, meeting_profile};
pub(crate) use windows::forget_open;
pub use windows::{WindowPlace, after_open};
