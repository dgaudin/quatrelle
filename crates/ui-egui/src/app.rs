//! The window (`MailApp`), in parts: each continues `impl MailApp` with what it draws or does.

mod actions;
mod bars;
mod costs;
mod dropped;
mod frame;
mod keys;
mod meetings;
mod ribbon;
mod search_box;
mod updates;
mod windows;

pub(crate) use costs::Merging;
pub(crate) use meetings::{meeting_browser, meeting_profile};
pub(crate) use ribbon::junk_menu;
pub(crate) use search_box::{SearchDrop, scope_label};
pub use windows::{WindowPlace, after_open};
