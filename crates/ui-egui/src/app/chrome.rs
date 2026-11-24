// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What the window is dressed in: the top bar (the window's title bar too, when it draws its own),
//! the status bar, the ribbon and the search box.

mod bars;
pub(crate) mod caption;
mod ribbon;
mod search_box;

pub(crate) use bars::module_rail;
pub use caption::MIN_WINDOW_SIZE;
pub(crate) use ribbon::{RIBBON_H, junk_menu, quick_steps_menu, run_quick_step};
pub(crate) use search_box::{SearchDrop, scope_label};
