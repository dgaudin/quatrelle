//! The envelope in the notification area while new mail is unread (`Services::show_envelope`),
//! as Outlook shows one: Quatrelle's icon, an envelope already, with a red dot, its tip saying
//! how many messages; clicking it brings the window forward on the Inbox.
//!
//! - **Linux and the BSDs:** a StatusNotifierItem (KDE, Xfce, Cinnamon, and GNOME with the
//!   AppIndicator extension), served from a thread of its own; a desktop without a notification
//!   area shows nothing.
//! - **Windows:** an icon in the notification area.
//! - **macOS:** the count on the Dock's icon, where Mail and Outlook put it; the Dock's icon
//!   already brings the window forward.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The sizes the icon is offered in (the committed app icons): the desktop picks.
#[cfg(not(target_os = "macos"))]
const SIZES: [(u32, &[u8]); 5] = [
    (16, include_bytes!("../../../../assets/app-icon/hicolor/16x16/apps/ai.storyteller.quatrelle.png")),
    (24, include_bytes!("../../../../assets/app-icon/hicolor/24x24/apps/ai.storyteller.quatrelle.png")),
    (32, include_bytes!("../../../../assets/app-icon/hicolor/32x32/apps/ai.storyteller.quatrelle.png")),
    (48, include_bytes!("../../../../assets/app-icon/hicolor/48x48/apps/ai.storyteller.quatrelle.png")),
    (64, include_bytes!("../../../../assets/app-icon/hicolor/64x64/apps/ai.storyteller.quatrelle.png")),
];

/// The dot's colour: the light theme's danger red (`theme::Tokens::danger`).
#[cfg(not(target_os = "macos"))]
const DOT: [u8; 3] = [0xC4, 0x2B, 0x1C];

/// The envelope, shown and taken away as the window says.
#[derive(Default)]
pub(crate) struct Envelope {
    /// Set when the envelope is clicked, until the window takes it.
    clicked: Arc<AtomicBool>,
    /// Wakes the window, so a click is answered at once (the Dock's icon needs none).
    #[cfg(not(target_os = "macos"))]
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
    /// The thread serving the item: told the tip to show, or `None` to take it away (dropped,
    /// the thread takes it away and ends).
    #[cfg(all(unix, not(target_os = "macos")))]
    item: Option<std::sync::mpsc::Sender<Option<String>>>,
    #[cfg(windows)]
    icon: Option<tray_icon::TrayIcon>,
}

impl Envelope {
    pub(crate) fn new(waker: Arc<dyn Fn() + Send + Sync>) -> Envelope {
        #[cfg(target_os = "macos")]
        drop(waker);
        Envelope {
            #[cfg(not(target_os = "macos"))]
            waker: Some(waker),
            ..Default::default()
        }
    }

    /// Whether the envelope was clicked since the last call.
    pub(crate) fn take_click(&self) -> bool {
        self.clicked.swap(false, Ordering::Relaxed)
    }

    /// Shown while `new` messages are unread, `tip` on hovering it; taken away at 0.
    #[cfg(all(unix, not(target_os = "macos")))]
    pub(crate) fn show(&mut self, new: usize, tip: &str) {
        if self.item.is_none() {
            if new == 0 {
                return;
            }
            self.item = serve(self.clicked.clone(), self.waker.clone());
        }
        if let Some(item) = &self.item
            && item.send((new > 0).then(|| tip.to_string())).is_err()
        {
            self.item = None;
        }
    }

    #[cfg(windows)]
    pub(crate) fn show(&mut self, new: usize, tip: &str) {
        if new == 0 {
            // Dropped, it leaves the notification area.
            self.icon = None;
            return;
        }
        if let Some(icon) = &self.icon {
            if let Err(e) = icon.set_tooltip(Some(tip)) {
                log::warn!("envelope: {e}");
            }
            return;
        }
        let (clicked, waker) = (self.clicked.clone(), self.waker.clone());
        tray_icon::TrayIconEvent::set_event_handler(Some(move |e: tray_icon::TrayIconEvent| {
            use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
            if matches!(
                e,
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                    | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. }
            ) {
                clicked.store(true, Ordering::Relaxed);
                if let Some(wake) = &waker {
                    wake();
                }
            }
        }));
        let Some((width, height, rgba)) = pictures().into_iter().find(|(w, ..)| *w == 32) else { return };
        let built = tray_icon::Icon::from_rgba(rgba, width, height)
            .map_err(|e| e.to_string())
            .and_then(|icon| tray_icon::TrayIconBuilder::new().with_icon(icon).with_tooltip(tip).build().map_err(|e| e.to_string()));
        match built {
            Ok(icon) => self.icon = Some(icon),
            Err(e) => log::warn!("envelope: {e}"),
        }
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn show(&mut self, new: usize, _tip: &str) {
        let Some(mtm) = objc2::MainThreadMarker::new() else { return };
        let label = (new > 0).then(|| objc2_foundation::NSString::from_str(&new.to_string()));
        objc2_app_kit::NSApplication::sharedApplication(mtm).dockTile().setBadgeLabel(label.as_deref());
    }

    #[cfg(not(any(unix, windows)))]
    pub(crate) fn show(&mut self, _new: usize, _tip: &str) {}
}

/// The app icon in each of [`SIZES`] with the dot: (width, height, RGBA).
#[cfg(not(target_os = "macos"))]
fn pictures() -> Vec<(u32, u32, Vec<u8>)> {
    SIZES
        .iter()
        .filter_map(|(_, png)| image::load_from_memory_with_format(png, image::ImageFormat::Png).ok())
        .map(|img| {
            let img = img.into_rgba8();
            let (w, h) = img.dimensions();
            let mut rgba = img.into_raw();
            dot(&mut rgba, w, h);
            (w, h, rgba)
        })
        .collect()
}

/// Draws the red dot, ringed in white, in the top right corner of an RGBA picture.
#[cfg(not(target_os = "macos"))]
fn dot(rgba: &mut [u8], w: u32, h: u32) {
    let side = w.min(h) as f32;
    let (cx, cy) = (w as f32 - side * 0.21, side * 0.21);
    let (ring, fill) = (side * 0.21, side * 0.21 - (side * 0.07).max(1.0));
    for y in 0..h {
        for x in 0..w {
            let d = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy);
            let i = ((y * w + x) * 4) as usize;
            let Some(px) = rgba.get_mut(i..i + 4) else { continue };
            over(px, [255, 255, 255], (ring - d + 0.5).clamp(0.0, 1.0));
            over(px, DOT, (fill - d + 0.5).clamp(0.0, 1.0));
        }
    }
}

/// Paints `color` at `cover` over the (straight alpha) pixel `px`.
#[cfg(not(target_os = "macos"))]
fn over(px: &mut [u8], color: [u8; 3], cover: f32) {
    if cover <= 0.0 {
        return;
    }
    let below = f32::from(px.get(3).copied().unwrap_or(0)) / 255.0;
    let alpha = cover + below * (1.0 - cover);
    for (c, top) in px.iter_mut().take(3).zip(color) {
        let mixed = (f32::from(top) * cover + f32::from(*c) * below * (1.0 - cover)) / alpha;
        *c = mixed.round().clamp(0.0, 255.0) as u8;
    }
    if let Some(a) = px.get_mut(3) {
        *a = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
    }
}

/// Serves the StatusNotifierItem from a thread of its own, told by the window what to show.
#[cfg(all(unix, not(target_os = "macos")))]
fn serve(clicked: Arc<AtomicBool>, waker: Option<Arc<dyn Fn() + Send + Sync>>) -> Option<std::sync::mpsc::Sender<Option<String>>> {
    use ksni::blocking::TrayMethods;
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    let spawned = std::thread::Builder::new().name("envelope".into()).spawn(move || {
        let icons: Vec<ksni::Icon> = pictures().into_iter().map(|(w, h, rgba)| argb(w, h, &rgba)).collect();
        let mut served: Option<ksni::blocking::Handle<Item>> = None;
        let mut told = false;
        while let Ok(shown) = rx.recv() {
            match (shown, &served) {
                (Some(tip), Some(handle)) => {
                    handle.update(|item| item.tip = tip);
                }
                (Some(tip), None) => {
                    let item = Item { tip, icons: icons.clone(), clicked: clicked.clone(), waker: waker.clone() };
                    // A Flatpak can't own the item's bus name (the specification's way); hosts
                    // find it by its connection then. The notification area can be away (GNOME
                    // takes its AppIndicator extension away while the screen is locked, when mail
                    // often comes): the item waits for it rather than giving up.
                    match item.disable_dbus_name(std::env::var_os("FLATPAK_ID").is_some()).assume_sni_available(true).spawn() {
                        Ok(handle) => served = Some(handle),
                        Err(e) if !told => {
                            told = true;
                            log::info!("envelope: not shown ({e})");
                        }
                        Err(_) => {}
                    }
                }
                (None, _) => {
                    if let Some(handle) = served.take() {
                        handle.shutdown().wait();
                    }
                }
            }
        }
        if let Some(handle) = served.take() {
            handle.shutdown().wait();
        }
    });
    match spawned {
        Ok(_) => Some(tx),
        Err(e) => {
            log::warn!("envelope: {e}");
            None
        }
    }
}

/// An RGBA picture as a StatusNotifierItem wants it: ARGB, each pixel big-endian.
#[cfg(all(unix, not(target_os = "macos")))]
fn argb(width: u32, height: u32, rgba: &[u8]) -> ksni::Icon {
    let data = rgba.as_chunks::<4>().0.iter().flat_map(|&[r, g, b, a]| [a, r, g, b]).collect();
    ksni::Icon { width: i32::try_from(width).unwrap_or(0), height: i32::try_from(height).unwrap_or(0), data }
}

/// The StatusNotifierItem: Quatrelle's icon with the dot, and how many messages on hovering it.
#[cfg(all(unix, not(target_os = "macos")))]
struct Item {
    tip: String,
    icons: Vec<ksni::Icon>,
    clicked: Arc<AtomicBool>,
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
}

#[cfg(all(unix, not(target_os = "macos")))]
impl ksni::Tray for Item {
    fn id(&self) -> String {
        "quatrelle".into()
    }
    fn title(&self) -> String {
        "Quatrelle".into()
    }
    fn category(&self) -> ksni::Category {
        ksni::Category::Communications
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icons.clone()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: "Quatrelle".into(), description: self.tip.clone(), ..Default::default() }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        self.clicked.store(true, Ordering::Relaxed);
        if let Some(wake) = &self.waker {
            wake();
        }
    }
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;

    /// The icon comes in every size, with the red dot in its top right corner and the envelope
    /// left as it was.
    #[test]
    fn the_envelope_is_the_app_icon_with_a_red_dot() {
        let pictures = pictures();
        assert_eq!(pictures.iter().map(|(w, ..)| *w).collect::<Vec<_>>(), vec![16, 24, 32, 48, 64]);
        for (w, h, rgba) in &pictures {
            let at = |x: u32, y: u32| rgba[((y * w + x) * 4) as usize..][..4].to_vec();
            let side = *w as f32;
            let centre = at(*w - (side * 0.21) as u32 - 1, (side * 0.21) as u32);
            assert_eq!(centre[..3], DOT, "{w} px: the dot");
            assert_eq!(centre[3], 255);
            let plain = image::load_from_memory(SIZES.iter().find(|(s, _)| s == w).unwrap().1).unwrap().into_rgba8();
            let (x, y) = (w / 3, h * 2 / 3);
            assert_eq!(at(x, y), plain.get_pixel(x, y).0.to_vec(), "{w} px: the envelope untouched");
        }
    }

    /// On a desktop with a notification area (run by hand: `cargo test -p quatrelle
    /// envelope_registers -- --ignored`): the item is registered while shown, and gone after.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    #[ignore = "needs a desktop session with a StatusNotifierWatcher"]
    fn the_envelope_registers_with_the_desktop() {
        let items = || -> Vec<String> {
            let bus = zbus::blocking::Connection::session().unwrap();
            let watcher =
                zbus::blocking::Proxy::new(&bus, "org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher", "org.kde.StatusNotifierWatcher").unwrap();
            watcher.get_property("RegisteredStatusNotifierItems").unwrap()
        };
        let mine = format!("StatusNotifierItem-{}-", std::process::id());
        let before = items().iter().filter(|i| i.contains(&mine)).count();
        let mut envelope = Envelope::new(Arc::new(|| {}));
        envelope.show(2, "2 new messages");
        std::thread::sleep(std::time::Duration::from_secs(2));
        assert_eq!(items().iter().filter(|i| i.contains(&mine)).count(), before + 1, "{:?}", items());
        envelope.show(0, "");
        std::thread::sleep(std::time::Duration::from_secs(2));
        assert_eq!(items().iter().filter(|i| i.contains(&mine)).count(), before);
    }

    /// Painted over a transparent pixel, a colour keeps its own value; half covered, it's half
    /// see-through.
    #[test]
    fn a_colour_painted_over_nothing_keeps_its_value() {
        let mut px = [0, 0, 0, 0];
        over(&mut px, DOT, 0.5);
        assert_eq!(px, [DOT[0], DOT[1], DOT[2], 128]);
        let mut px = [255, 255, 255, 255];
        over(&mut px, [0, 0, 0], 1.0);
        assert_eq!(px, [0, 0, 0, 255]);
    }
}
