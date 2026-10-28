//! Quatrelle's icon in the notification area (`Services::show_tray`): there while closing the
//! window leaves Quatrelle running, and the envelope Outlook shows while new mail is unread: the
//! icon, an envelope already, with a red dot, its tip saying how many messages. Clicked, it
//! brings the window back (on the Inbox for new mail); its menu opens Quatrelle or quits.
//!
//! - **Linux and the BSDs:** a StatusNotifierItem (KDE, Xfce, Cinnamon, and GNOME with the
//!   AppIndicator extension), served from a thread of its own; a desktop without a notification
//!   area shows nothing, and closing the window then quits.
//! - **Windows:** an icon in the notification area.
//! - **macOS:** the count on the Dock's icon, where Mail and Outlook put it; the Dock's icon
//!   already brings the window forward, and closing the window quits as it did.

use std::sync::Arc;
#[cfg(all(unix, not(target_os = "macos")))]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU8, Ordering};

use quatrelle_ui_egui::{TrayAction, TrayIcon};

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

/// What was done with the icon, as [`Tray::take_action`] hands it over.
const NOTHING: u8 = 0;
const OPEN: u8 = 1;
const QUIT: u8 = 2;

/// The icon, shown, changed and taken away as the window says.
#[derive(Default)]
pub(crate) struct Tray {
    /// What was done with the icon ([`NOTHING`], [`OPEN`], [`QUIT`]), until the window takes it.
    done: Arc<AtomicU8>,
    /// Wakes the window, so a click is answered at once (the Dock's icon needs none).
    #[cfg(not(target_os = "macos"))]
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Whether the window can be put away: not on Wayland itself, where a window can't hide.
    #[cfg(not(target_os = "macos"))]
    hides: bool,
    /// The thread serving the item: told what to show, or `None` to take it away (dropped, the
    /// thread takes it away and ends).
    #[cfg(all(unix, not(target_os = "macos")))]
    item: Option<std::sync::mpsc::Sender<Option<TrayIcon>>>,
    /// Whether a notification area has the item: GNOME takes its AppIndicator extension away
    /// while the screen is locked, and the item waits for it to come back.
    #[cfg(all(unix, not(target_os = "macos")))]
    hosted: Arc<AtomicBool>,
    #[cfg(windows)]
    icon: Option<WindowsIcon>,
}

/// The icon in Windows' notification area, and its menu's items (their text follows the language).
#[cfg(windows)]
struct WindowsIcon {
    icon: tray_icon::TrayIcon,
    open: tray_icon::menu::MenuItem,
    quit: tray_icon::menu::MenuItem,
    dotted: bool,
}

impl Tray {
    /// `hides`: whether the window can be put away (`Visible(false)`) for the icon to bring back.
    pub(crate) fn new(waker: Arc<dyn Fn() + Send + Sync>, hides: bool) -> Tray {
        #[cfg(target_os = "macos")]
        drop((waker, hides));
        Tray {
            #[cfg(not(target_os = "macos"))]
            waker: Some(waker),
            #[cfg(not(target_os = "macos"))]
            hides,
            ..Default::default()
        }
    }

    /// What was done with the icon since the last call.
    pub(crate) fn take_action(&self) -> Option<TrayAction> {
        match self.done.swap(NOTHING, Ordering::Relaxed) {
            OPEN => Some(TrayAction::Open),
            QUIT => Some(TrayAction::Quit),
            _ => None,
        }
    }

    /// Whether the icon is in a notification area, there to bring a window put away back.
    #[cfg(all(unix, not(target_os = "macos")))]
    pub(crate) fn shown(&self) -> bool {
        self.hides && self.item.is_some() && self.hosted.load(Ordering::Relaxed)
    }

    #[cfg(windows)]
    pub(crate) fn shown(&self) -> bool {
        self.hides && self.icon.is_some()
    }

    #[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
    pub(crate) fn shown(&self) -> bool {
        false
    }

    /// Shown as `shown` says: with the dot while new mail is unread, taken away when not shown.
    #[cfg(all(unix, not(target_os = "macos")))]
    pub(crate) fn show(&mut self, shown: &TrayIcon) {
        if self.item.is_none() {
            if !shown.shown {
                return;
            }
            self.item = serve(self.done.clone(), self.waker.clone(), self.hosted.clone());
        }
        if let Some(item) = &self.item
            && item.send(shown.shown.then(|| shown.clone())).is_err()
        {
            self.item = None;
        }
    }

    #[cfg(windows)]
    pub(crate) fn show(&mut self, shown: &TrayIcon) {
        use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
        use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
        if !shown.shown {
            // Dropped, it leaves the notification area.
            self.icon = None;
            return;
        }
        let tip = if shown.tip.is_empty() { "Quatrelle".to_string() } else { format!("Quatrelle\n{}", shown.tip) };
        let dotted = shown.new > 0;
        if let Some(w) = &mut self.icon {
            if let Err(e) = w.icon.set_tooltip(Some(&tip)) {
                log::warn!("tray: {e}");
            }
            w.open.set_text(&shown.open);
            w.quit.set_text(&shown.quit);
            if w.dotted != dotted
                && let Some(picture) = picture(dotted)
            {
                match w.icon.set_icon(Some(picture)) {
                    Ok(()) => w.dotted = dotted,
                    Err(e) => log::warn!("tray: {e}"),
                }
            }
            return;
        }
        // Set once for the app's life (later calls are ignored), so they share `done`.
        let (done, waker) = (self.done.clone(), self.waker.clone());
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if matches!(
                e,
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                    | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. }
            ) {
                tell(&done, waker.as_ref(), OPEN);
            }
        }));
        let (done, waker) = (self.done.clone(), self.waker.clone());
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| match e.id.0.as_str() {
            "open" => tell(&done, waker.as_ref(), OPEN),
            "quit" => tell(&done, waker.as_ref(), QUIT),
            _ => {}
        }));
        let open = MenuItem::with_id("open", &shown.open, true, None);
        let quit = MenuItem::with_id("quit", &shown.quit, true, None);
        let built = Menu::with_items(&[&open, &PredefinedMenuItem::separator(), &quit]).map_err(|e| e.to_string()).and_then(|menu| {
            let icon = picture(dotted).ok_or_else(|| "no picture".to_string())?;
            tray_icon::TrayIconBuilder::new()
                .with_icon(icon)
                .with_tooltip(&tip)
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .build()
                .map_err(|e| e.to_string())
        });
        match built {
            Ok(icon) => self.icon = Some(WindowsIcon { icon, open, quit, dotted }),
            Err(e) => log::warn!("tray: {e}"),
        }
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn show(&mut self, shown: &TrayIcon) {
        let Some(mtm) = objc2::MainThreadMarker::new() else { return };
        let label = (shown.new > 0).then(|| objc2_foundation::NSString::from_str(&shown.new.to_string()));
        objc2_app_kit::NSApplication::sharedApplication(mtm).dockTile().setBadgeLabel(label.as_deref());
    }

    #[cfg(not(any(unix, windows)))]
    pub(crate) fn show(&mut self, _shown: &TrayIcon) {}
}

/// Something done with the icon: kept for the window, woken to take it.
#[cfg(not(target_os = "macos"))]
fn tell(done: &AtomicU8, waker: Option<&Arc<dyn Fn() + Send + Sync>>, what: u8) {
    done.store(what, Ordering::Relaxed);
    if let Some(wake) = waker {
        wake();
    }
}

/// The 32-pixel icon Windows shows, with the dot or without.
#[cfg(windows)]
fn picture(dotted: bool) -> Option<tray_icon::Icon> {
    let (width, height, rgba) = pictures(dotted).into_iter().find(|(w, ..)| *w == 32)?;
    tray_icon::Icon::from_rgba(rgba, width, height).map_err(|e| log::warn!("tray: {e}")).ok()
}

/// The app icon in each of [`SIZES`], with the dot (`dotted`) or as it is: (width, height, RGBA).
#[cfg(not(target_os = "macos"))]
fn pictures(dotted: bool) -> Vec<(u32, u32, Vec<u8>)> {
    SIZES
        .iter()
        .filter_map(|(_, png)| image::load_from_memory_with_format(png, image::ImageFormat::Png).ok())
        .map(|img| {
            let img = img.into_rgba8();
            let (w, h) = img.dimensions();
            let mut rgba = img.into_raw();
            if dotted {
                dot(&mut rgba, w, h);
            }
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
fn serve(
    done: Arc<AtomicU8>,
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
    hosted: Arc<AtomicBool>,
) -> Option<std::sync::mpsc::Sender<Option<TrayIcon>>> {
    use ksni::blocking::TrayMethods;
    let (tx, rx) = std::sync::mpsc::channel::<Option<TrayIcon>>();
    let spawned = std::thread::Builder::new().name("tray".into()).spawn(move || {
        let icons = |dotted| pictures(dotted).into_iter().map(|(w, h, rgba)| argb(w, h, &rgba)).collect::<Vec<_>>();
        let (plain, dotted) = (icons(false), icons(true));
        let mut served: Option<ksni::blocking::Handle<Item>> = None;
        let mut told = false;
        while let Ok(shown) = rx.recv() {
            match (shown, &served) {
                (Some(shown), Some(handle)) => {
                    handle.update(|item| item.shown = shown);
                }
                (Some(shown), None) => {
                    // Until the desktop says otherwise (as the item registers, or later).
                    hosted.store(true, Ordering::Relaxed);
                    let item = Item {
                        shown,
                        plain: plain.clone(),
                        dotted: dotted.clone(),
                        done: done.clone(),
                        waker: waker.clone(),
                        hosted: hosted.clone(),
                    };
                    // A Flatpak can't own the item's bus name (the specification's way); hosts
                    // find it by its connection then. The notification area can be away (GNOME
                    // takes its AppIndicator extension away while the screen is locked, when mail
                    // often comes): the item waits for it rather than giving up.
                    match item.disable_dbus_name(std::env::var_os("FLATPAK_ID").is_some()).assume_sni_available(true).spawn() {
                        Ok(handle) => served = Some(handle),
                        Err(e) => {
                            hosted.store(false, Ordering::Relaxed);
                            if !told {
                                told = true;
                                log::info!("tray: not shown ({e})");
                            }
                        }
                    }
                }
                (None, _) => {
                    hosted.store(false, Ordering::Relaxed);
                    if let Some(handle) = served.take() {
                        handle.shutdown().wait();
                    }
                }
            }
        }
        hosted.store(false, Ordering::Relaxed);
        if let Some(handle) = served.take() {
            handle.shutdown().wait();
        }
    });
    match spawned {
        Ok(_) => Some(tx),
        Err(e) => {
            log::warn!("tray: {e}");
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

/// The StatusNotifierItem: Quatrelle's icon (with the dot while new mail is unread), how many
/// messages on hovering it, and its menu.
#[cfg(all(unix, not(target_os = "macos")))]
struct Item {
    shown: TrayIcon,
    plain: Vec<ksni::Icon>,
    dotted: Vec<ksni::Icon>,
    done: Arc<AtomicU8>,
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
    hosted: Arc<AtomicBool>,
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
        if self.shown.new > 0 { self.dotted.clone() } else { self.plain.clone() }
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: "Quatrelle".into(), description: self.shown.tip.clone(), ..Default::default() }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        tell(&self.done, self.waker.as_ref(), OPEN);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        // An underscore marks the access key there: one written stays one.
        let label = |s: &str| s.replace('_', "__");
        vec![
            StandardItem {
                label: label(&self.shown.open),
                activate: Box::new(|item: &mut Item| tell(&item.done, item.waker.as_ref(), OPEN)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: label(&self.shown.quit),
                activate: Box::new(|item: &mut Item| tell(&item.done, item.waker.as_ref(), QUIT)),
                ..Default::default()
            }
            .into(),
        ]
    }
    fn watcher_online(&self) {
        self.hosted.store(true, Ordering::Relaxed);
    }
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        self.hosted.store(false, Ordering::Relaxed);
        true
    }
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;

    /// The icon comes in every size, with the red dot in its top right corner and the envelope
    /// left as it was; without new mail, it's the app icon as it is.
    #[test]
    fn the_envelope_is_the_app_icon_with_a_red_dot() {
        for (w, h, rgba) in pictures(false) {
            let plain = image::load_from_memory(SIZES.iter().find(|(s, _)| *s == w).unwrap().1).unwrap().into_rgba8();
            assert_eq!((w, h), plain.dimensions());
            assert_eq!(rgba, plain.into_raw(), "{w} px: as it is");
        }
        let pictures = pictures(true);
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
    /// icon_registers -- --ignored`): the item is registered while shown, and gone after.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    #[ignore = "needs a desktop session with a StatusNotifierWatcher"]
    fn the_icon_registers_with_the_desktop() {
        let items = || -> Vec<String> {
            let bus = zbus::blocking::Connection::session().unwrap();
            let watcher =
                zbus::blocking::Proxy::new(&bus, "org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher", "org.kde.StatusNotifierWatcher").unwrap();
            watcher.get_property("RegisteredStatusNotifierItems").unwrap()
        };
        let mine = format!("StatusNotifierItem-{}-", std::process::id());
        let before = items().iter().filter(|i| i.contains(&mine)).count();
        let mut tray = Tray::new(Arc::new(|| {}), true);
        let shown = TrayIcon { shown: true, new: 0, tip: String::new(), open: "Open Quatrelle".into(), quit: "Exit".into() };
        tray.show(&shown);
        std::thread::sleep(std::time::Duration::from_secs(2));
        assert_eq!(items().iter().filter(|i| i.contains(&mine)).count(), before + 1, "{:?}", items());
        assert!(tray.shown(), "hosted: closing the window can put it away");
        // New mail: the same item, with the dot.
        tray.show(&TrayIcon { new: 2, tip: "2 new messages".into(), ..shown });
        std::thread::sleep(std::time::Duration::from_secs(1));
        assert_eq!(items().iter().filter(|i| i.contains(&mine)).count(), before + 1);
        tray.show(&TrayIcon::default());
        std::thread::sleep(std::time::Duration::from_secs(2));
        assert_eq!(items().iter().filter(|i| i.contains(&mine)).count(), before);
        assert!(!tray.shown());
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
