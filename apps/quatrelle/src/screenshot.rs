//! `QUATRELLE_SCREENSHOT=window.png`: the window saved as a picture once it has settled, then
//! closed — to look at a change in the window itself (and only it), where the desktop offers no
//! way to capture one window (GNOME on Wayland).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// Frames drawn before the picture is asked for: the panes laid out, the icons rendered.
const SETTLE_FRAMES: u32 = 90;

/// Frames the window is given to close itself once the picture is saved, before the process
/// ends (a window that stayed open with the picture long saved was the whole of a run).
const CLOSE_FRAMES: u32 = 30;

const WAITING: u8 = 0;
const ASKED: u8 = 1;
const SAVED: u8 = 2;

/// The picture asked for, and how far it has got: asked for once the window has settled, saved
/// (or not) by the callback that brings it, then the window closed — from the UI's own frame,
/// not from that callback: a `Close` sent from there went unheard, and the window stayed.
pub struct Shot {
    path: PathBuf,
    state: Arc<AtomicU8>,
    /// The frame the window was first asked to close in.
    closing_since: Option<u32>,
}

impl Shot {
    /// Where the picture goes, when asked for.
    pub fn wanted() -> Option<Shot> {
        let path = std::env::var_os("QUATRELLE_SCREENSHOT").filter(|p| !p.is_empty()).map(PathBuf::from)?;
        Some(Shot { path, state: Arc::new(AtomicU8::new(WAITING)), closing_since: None })
    }

    /// Each frame. Returns true when the window wouldn't close by itself: the process should
    /// end, its state kept.
    pub fn step(&mut self, ctx: &egui::Context, frames: u32) -> bool {
        match self.state.load(Ordering::SeqCst) {
            WAITING => {
                // Frames keep coming until the picture is asked for.
                ctx.request_repaint();
                if frames >= SETTLE_FRAMES {
                    self.state.store(ASKED, Ordering::SeqCst);
                    let (state, path) = (self.state.clone(), self.path.clone());
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |image| {
                        save(&image, &path);
                        state.store(SAVED, Ordering::SeqCst);
                    })));
                }
                false
            }
            // The picture comes with a later frame: keep drawing them, so it does, and so the
            // frame after sees it saved.
            ASKED => {
                ctx.request_repaint();
                false
            }
            _ => {
                let since = *self.closing_since.get_or_insert(frames);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                ctx.request_repaint();
                frames.saturating_sub(since) > CLOSE_FRAMES
            }
        }
    }
}

fn save(image: &egui::ColorImage, path: &std::path::Path) {
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    let (w, h) = (u32::try_from(image.size[0]).unwrap_or(0), u32::try_from(image.size[1]).unwrap_or(0));
    match image::RgbaImage::from_raw(w, h, bytes).map(|img| img.save(path)) {
        Some(Ok(())) => log::info!("window saved as {}", path.display()),
        Some(Err(e)) => log::warn!("the window couldn't be saved as {}: {e}", path.display()),
        None => log::warn!("the window's picture came with the wrong size"),
    }
}
