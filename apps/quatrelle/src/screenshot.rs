//! `QUATRELLE_SCREENSHOT=window.png`: the window saved as a picture once it has settled, then
//! closed — to look at a change in the window itself (and only it), where the desktop offers no
//! way to capture one window (GNOME on Wayland).

use std::path::PathBuf;

/// Frames drawn before the picture is asked for: the panes laid out, the icons rendered.
const SETTLE_FRAMES: u32 = 90;

/// Where the picture goes, when asked for.
pub fn wanted() -> Option<PathBuf> {
    std::env::var_os("QUATRELLE_SCREENSHOT").filter(|p| !p.is_empty()).map(PathBuf::from)
}

/// Ask for the picture once the window has settled; it's saved, and the window closed, when it
/// comes.
pub fn step(ctx: &egui::Context, frames: u32, path: &std::path::Path) {
    if frames < SETTLE_FRAMES {
        ctx.request_repaint();
        return;
    }
    if frames != SETTLE_FRAMES {
        return;
    }
    let (ctx2, path) = (ctx.clone(), path.to_path_buf());
    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |image| {
        let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
        let (w, h) = (u32::try_from(image.size[0]).unwrap_or(0), u32::try_from(image.size[1]).unwrap_or(0));
        match image::RgbaImage::from_raw(w, h, bytes).map(|img| img.save(&path)) {
            Some(Ok(())) => log::info!("window saved as {}", path.display()),
            Some(Err(e)) => log::warn!("the window couldn't be saved as {}: {e}", path.display()),
            None => log::warn!("the window's picture came with the wrong size"),
        }
        ctx2.send_viewport_cmd(egui::ViewportCommand::Close);
        ctx2.request_repaint();
    })));
}
