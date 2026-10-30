//! How Quatrelle looks and moves: colours and type, icons drawn in code, widgets, motion, fonts,
//! thumbnails, and right-to-left text.

pub mod bidi;
pub mod craft_fonts;
pub(crate) mod emoji;
pub mod icons;
pub mod motion;
// Drawn by its own tests until the message window's fields use it (the contacts field, delivery
// 1, its next step): parts of it are unused until then.
#[allow(dead_code)]
pub(crate) mod people_field;
pub mod theme;
pub(crate) mod thumbs;
pub(crate) mod typefaces;
pub(crate) mod widgets;
