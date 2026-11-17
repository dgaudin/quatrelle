// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What goes with a message: files attached with the paperclip (the system's file dialog, which
//! the window asks for: `app/actions/teams.rs`) and pictures pasted, as chips above the box
//! ([`chips`]: an icon, the name, the size, and a cross that takes it away). They go with the
//! text, by their paths (`teams.send`'s `files` and `pictures`, each in the order added:
//! [`paths`]). A file is taken when it's a regular file of 1 byte to [`FILE_MAX`], not there
//! already, while the message has fewer than [`PARTS_MAX`] parts ([`add_files`]); a picture
//! pasted is written to a new file of the user's own folder (`scratch_dir("teams")`, readable by
//! the user only), and goes inside the message when it's at most [`PICTURE_INLINE_MAX`], else as
//! a file (decision 4: [`add_picture`]). What isn't taken is said under the box until the text or
//! the parts change. An edit takes no file: nothing is added while one is under way. A send
//! refused comes back with its parts, looked at again ([`put_back`]).
//!
//! What is hostile here: the paths are the user's (the dialog's, or a refused command's params),
//! each looked at when added; the worker looks again when the message's turn comes (a file
//! changed since isn't sent). A path the engine wouldn't take (not absolute, past [`PATH_MAX`], a
//! control character, not UTF-8, no file name) isn't taken. Names are drawn on one line, without
//! bidi characters. The clipboard's bytes are written as they came (the worker sends a PNG or a
//! JPEG only). What it costs is bounded: [`PARTS_MAX`] parts, [`SAID_MAX`] sentences,
//! [`PASTED_MAX`] names tried for a picture's file.

use std::path::{Path, PathBuf};

use egui::{Rect, Sense, pos2, vec2};
use quatrelle_engine::model::pim::teams::{FILE_MAX, PARTS_MAX, PATH_MAX, PICTURE_INLINE_MAX, is_bidi, one_line};

use super::*;
use crate::icons;

/// The most sentences said of what wasn't taken.
const SAID_MAX: usize = 3;
/// The most names tried for a pasted picture's file (`pasted-1.png`, `pasted-2.png`…).
#[cfg(not(target_arch = "wasm32"))]
const PASTED_MAX: u32 = 10_000;
/// The most characters of a name in a sentence.
const NAME_SAID: usize = 60;
/// A chip's height, its narrowest and its widest (as the mail's attachments').
const CHIP_H: f32 = 28.0;
const CHIP_MIN: f32 = 220.0;
const CHIP_MAX: f32 = 320.0;

/// A file or a picture going with the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BoxPart {
    pub(crate) path: PathBuf,
    /// Its file's name, on one line.
    pub(crate) name: String,
    pub(crate) size: u64,
    /// A picture pasted, going inside the message (else a file, shared from OneDrive).
    pub(crate) picture: bool,
}

/// Why a file wasn't taken.
enum NotTaken {
    /// The message has [`PARTS_MAX`] parts already.
    Full,
    TooBig,
    Empty,
    /// Not there, not a regular file, or a path the engine wouldn't take.
    Unreadable,
}

/// `name` as a sentence says it: on one line, without bidi characters, cut at [`NAME_SAID`].
fn said_name(name: &str) -> String {
    let line: String = one_line(name).chars().filter(|&c| !is_bidi(c)).collect();
    if line.chars().count() > NAME_SAID { format!("{}…", line.chars().take(NAME_SAID - 1).collect::<String>()) } else { line }
}

/// What the parts' change says from now on: what was said of the last ones, and of the message,
/// goes.
fn changing(d: &mut ChatDraft) {
    d.not_taken.clear();
    d.said = None;
}

/// `sentence` said under the box (once, at most [`SAID_MAX`]).
fn say(d: &mut ChatDraft, sentence: String) {
    if d.not_taken.len() < SAID_MAX && !d.not_taken.contains(&sentence) {
        d.not_taken.push(sentence);
    }
}

/// Why `path` wasn't taken, said.
fn say_why(d: &mut ChatDraft, why: NotTaken, path: &Path) {
    let name = said_name(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let sentence = match why {
        NotTaken::Full => tl!("At most 10 files and pictures go with a message").to_string(),
        NotTaken::TooBig => tf!("{name} is bigger than Teams takes here (100 MB)", name = name),
        NotTaken::Empty => tf!("{name} is empty", name = name),
        NotTaken::Unreadable => tf!("{name} can't be read", name = name),
    };
    say(d, sentence);
}

/// Whether the engine takes `path` for a part: absolute, UTF-8, at most [`PATH_MAX`] bytes,
/// without a control character, ending in a file's name.
fn path_ok(path: &Path) -> bool {
    path.to_str().is_some_and(|p| p.len() <= PATH_MAX && !p.chars().any(char::is_control)) && path.is_absolute() && path.file_name().is_some()
}

/// The size of the regular file at `path` (a link followed, as the worker follows it); none for
/// anything else. The browser has no files.
#[cfg(not(target_arch = "wasm32"))]
fn regular_size(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().filter(std::fs::Metadata::is_file).map(|m| m.len())
}

#[cfg(target_arch = "wasm32")]
fn regular_size(_: &Path) -> Option<u64> {
    None
}

/// `path` taken as a part (a file, or a pasted `picture`) when it can be one; there already, it
/// isn't taken twice (and nothing is said).
fn take(d: &mut ChatDraft, path: PathBuf, picture: bool) -> Result<(), NotTaken> {
    if d.parts.iter().any(|p| p.path == path) {
        return Ok(());
    }
    if d.parts.len() >= PARTS_MAX {
        return Err(NotTaken::Full);
    }
    if !path_ok(&path) {
        return Err(NotTaken::Unreadable);
    }
    let size = match regular_size(&path).ok_or(NotTaken::Unreadable)? {
        0 => return Err(NotTaken::Empty),
        n if n > FILE_MAX => return Err(NotTaken::TooBig),
        n => n,
    };
    let name = one_line(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    d.parts.push(BoxPart { path, name, size, picture: picture && size <= PICTURE_INLINE_MAX });
    Ok(())
}

/// Files picked in the dialog, added to the message in their order: each a regular file of 1 byte
/// to [`FILE_MAX`], not there already, while there are fewer than [`PARTS_MAX`]; why one isn't
/// taken is said. Nothing while a message is edited.
pub(crate) fn add_files(d: &mut ChatDraft, paths: Vec<PathBuf>) {
    if d.editing.is_some() {
        return;
    }
    changing(d);
    for path in paths {
        if let Err(why) = take(d, path.clone(), false) {
            say_why(d, why, &path);
        }
    }
}

/// A picture pasted (`png`, the clipboard's), added to the message: written to a new file of the
/// user's own folder, a picture inside the message when it's at most [`PICTURE_INLINE_MAX`] bytes,
/// else a file. Nothing while a message is edited.
pub(crate) fn add_picture(d: &mut ChatDraft, png: &[u8]) {
    if d.editing.is_some() || png.is_empty() {
        return;
    }
    changing(d);
    let size = u64::try_from(png.len()).unwrap_or(u64::MAX);
    let pasted = Path::new("pasted.png");
    if d.parts.len() >= PARTS_MAX {
        say_why(d, NotTaken::Full, pasted);
    } else if size > FILE_MAX {
        say_why(d, NotTaken::TooBig, pasted);
    } else if let Some(path) = pasted_file(png) {
        if let Err(why) = take(d, path.clone(), true) {
            say_why(d, why, &path);
            let _ = std::fs::remove_file(&path);
        }
    } else {
        say(d, tl!("There's no folder to put the picture in").to_string());
    }
}

/// `png` written to a new file of the user's own folder for pictures pasted (`pasted-1.png`, the
/// first name free), readable by the user only; none when there's no folder or it can't be
/// written. The browser has none.
#[cfg(not(target_arch = "wasm32"))]
fn pasted_file(png: &[u8]) -> Option<PathBuf> {
    use std::io::Write;
    let dir = quatrelle_engine::store::scratch_dir("teams")?;
    for n in 1..=PASTED_MAX {
        let path = dir.join(format!("pasted-{n}.png"));
        let mut open = std::fs::OpenOptions::new();
        open.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open.mode(0o600);
        }
        match open.open(&path) {
            Ok(mut f) => {
                if f.write_all(png).is_ok() {
                    return Some(path);
                }
                drop(f);
                let _ = std::fs::remove_file(&path);
                return None;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
}

#[cfg(target_arch = "wasm32")]
fn pasted_file(_: &[u8]) -> Option<PathBuf> {
    None
}

/// The files and pictures of a `teams.send` refused (`params`), back in the box before those added
/// since: each looked at again (one no longer there, or past its cap, left out and said), at most
/// [`PARTS_MAX`] in all.
pub(super) fn put_back(d: &mut ChatDraft, params: &Value) {
    let since = std::mem::take(&mut d.parts);
    changing(d);
    let list = |k: &str| params.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let files = list("files").iter().map(|p| (p, false));
    let pictures = list("pictures").iter().map(|p| (p, true));
    // One past the cap is enough to say so.
    for (p, picture) in files.chain(pictures).take(PARTS_MAX.saturating_add(1)) {
        let Some(path) = p.as_str().filter(|p| !p.is_empty() && p.len() <= PATH_MAX).map(PathBuf::from) else { continue };
        if let Err(why) = take(d, path.clone(), picture) {
            say_why(d, why, &path);
        }
    }
    for p in since {
        if d.parts.iter().any(|q| q.path == p.path) {
            continue;
        }
        if d.parts.len() >= PARTS_MAX {
            say_why(d, NotTaken::Full, &p.path);
            break;
        }
        d.parts.push(p);
    }
}

/// The parts' paths as `teams.send` takes them: the files', then the pictures', each in the order
/// added.
pub(super) fn paths(parts: &[BoxPart]) -> (Vec<String>, Vec<String>) {
    let of = |picture: bool| parts.iter().filter(|p| p.picture == picture).map(|p| p.path.to_string_lossy().into_owned()).collect();
    (of(false), of(true))
}

/// Whether the frame's events paste a picture: one came (`PasteImage`: egui read it off the
/// clipboard, which held no text), or Ctrl+V (⌘V) was pressed and no text came with it.
pub(super) fn pasting(events: &[egui::Event]) -> bool {
    let text = events.iter().any(|e| matches!(e, egui::Event::Paste(_)));
    let shortcut = |e: &egui::Event| matches!(e, egui::Event::Key { key: Key::V, pressed: true, modifiers, .. } if modifiers.matches_logically(Modifiers::COMMAND));
    !text && events.iter().any(|e| matches!(e, egui::Event::PasteImage(_)) || shortcut(e))
}

/// The parts as chips over the box, wrapped into rows (scrolling past two): each its icon (a
/// picture's, or a file's), its name on one line, its size, and a cross that takes it away.
pub(super) fn chips(ui: &mut Ui, t: &Tokens, d: &mut ChatDraft) {
    if d.parts.is_empty() {
        return;
    }
    let mut remove = None;
    let id = ui.make_persistent_id("teams-box-parts");
    widgets::chip_grid(ui, id, d.parts.len(), CHIP_MIN, CHIP_MAX, CHIP_H, 2, |ui, i, w| {
        let Some(p) = d.parts.get(i) else { return };
        let name: String = p.name.chars().filter(|&c| !is_bidi(c)).collect();
        let (rect, resp) = ui.allocate_exact_size(vec2(w, CHIP_H), Sense::hover());
        let x_rect = Rect::from_center_size(pos2(rect.right() - 15.0, rect.center().y), vec2(22.0, 22.0));
        let x = ui.interact(x_rect, resp.id.with("remove"), Sense::click()).on_hover_text(tl!("Remove"));
        widgets::named(ui, &x, &format!("{}: {name}", tl!("Remove")), true, None);
        let painter = ui.painter();
        let fill = if resp.hovered() { t.hover_row } else { t.surface };
        painter.rect(rect, 4.0, fill, egui::Stroke::new(1.0, t.faint.gamma_multiply(0.55)), egui::StrokeKind::Inside);
        if x.hovered() {
            painter.rect_filled(x_rect, 3.0, t.hover);
        }
        icons::paint(painter, icons::centered(x_rect, 11.0), Icon::Close, if x.hovered() { t.text } else { t.dim });
        let icon = Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), vec2(14.0, 14.0));
        icons::paint(painter, icon, if p.picture { Icon::Image } else { Icon::File }, t.dim);
        let size = painter.layout_no_wrap(widgets::size_text(p.size), theme::regular(11.5), t.dim);
        let size_at = x_rect.left() - 4.0 - size.size().x;
        let g = widgets::one_line(ui, &name, theme::regular(12.0), t.text, size_at - 8.0 - (icon.right() + 6.0));
        painter.galley(pos2(icon.right() + 6.0, rect.center().y - g.size().y / 2.0), g, t.text);
        painter.galley(pos2(size_at, rect.center().y - size.size().y / 2.0), size, t.dim);
        resp.on_hover_text(name);
        if x.clicked() {
            remove = Some(i);
        }
    });
    if let Some(i) = remove.filter(|&i| i < d.parts.len()) {
        d.parts.remove(i);
        changing(d);
    }
    ui.add_space(4.0);
}
