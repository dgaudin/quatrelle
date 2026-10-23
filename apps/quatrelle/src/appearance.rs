//! The desktop's own style, for the window to follow: light or dark, the accent colour, and the
//! interface font.
//!
//! - **Linux and the BSDs:** the XDG settings portal (`org.freedesktop.appearance`), which GNOME
//!   and KDE both answer; winit doesn't tell egui whether the desktop is dark there.
//! - **Windows:** the accent colour in the user's registry (winit already follows dark mode).
//! - **macOS:** the accent colour from the global defaults.
//!
//! The style is read on a thread of its own every few seconds, so a change in the desktop's
//! settings shows without a restart.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use quatrelle_ui_egui::theme::SystemStyle;

/// How often the desktop's settings are read again.
const EVERY: Duration = Duration::from_secs(3);

/// The style as last read, shared with the reading thread.
#[derive(Clone, Default)]
pub struct Watch(Arc<Mutex<SystemStyle>>);

impl Watch {
    /// Start reading the desktop's style; `changed` is called when it changes (to repaint).
    pub fn start(changed: impl Fn() + Send + 'static) -> Watch {
        let w = Watch::default();
        let shared = w.0.clone();
        let first = read();
        if let Ok(mut s) = shared.lock() {
            *s = first;
        }
        let spawned = std::thread::Builder::new().name("appearance".into()).spawn(move || {
            let mut last = first;
            loop {
                std::thread::sleep(EVERY);
                let now = read();
                if now != last {
                    last = now;
                    if let Ok(mut s) = shared.lock() {
                        *s = now;
                    }
                    changed();
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("the desktop's style won't be followed: {e}");
        }
        w
    }

    pub fn get(&self) -> SystemStyle {
        self.0.lock().map(|s| *s).unwrap_or_default()
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn read() -> SystemStyle {
    portal::read()
}

#[cfg(target_os = "windows")]
fn read() -> SystemStyle {
    // DWM's accent colour: 0xAABBGGRR.
    let accent = windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\DWM")
        .and_then(|k| k.get_u32("AccentColor"))
        .ok()
        .map(|v| egui::Color32::from_rgb((v & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, ((v >> 16) & 0xFF) as u8));
    SystemStyle { dark: None, accent }
}

#[cfg(target_os = "macos")]
fn read() -> SystemStyle {
    // System Settings › Appearance › Accent colour: a number, or nothing for Multicolour
    // (which is blue in other apps' controls).
    let out = std::process::Command::new("defaults").args(["read", "-g", "AppleAccentColor"]).output().ok();
    let n = out.filter(|o| o.status.success()).and_then(|o| String::from_utf8(o.stdout).ok()).and_then(|s| s.trim().parse::<i32>().ok());
    let rgb = match n {
        Some(-1) => [0x8C, 0x8C, 0x8C],
        Some(0) => [0xFF, 0x52, 0x57],
        Some(1) => [0xF7, 0x82, 0x1B],
        Some(2) => [0xFF, 0xC6, 0x00],
        Some(3) => [0x62, 0xBA, 0x46],
        Some(5) => [0xA5, 0x50, 0xA7],
        Some(6) => [0xF7, 0x4F, 0x9E],
        _ => [0x00, 0x7A, 0xFF],
    };
    SystemStyle { dark: None, accent: Some(egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2])) }
}

#[cfg(not(any(unix, target_os = "windows")))]
fn read() -> SystemStyle {
    SystemStyle::default()
}

/// The XDG settings portal (`org.freedesktop.portal.Settings`).
#[cfg(all(unix, not(target_os = "macos")))]
mod portal {
    use quatrelle_ui_egui::theme::SystemStyle;
    use zbus::zvariant::{OwnedValue, Value};

    const NS: &str = "org.freedesktop.appearance";

    /// The session bus, opened once (`None` without one: no desktop settings to follow).
    static BUS: std::sync::OnceLock<Option<zbus::blocking::Connection>> = std::sync::OnceLock::new();

    pub fn read() -> SystemStyle {
        let Some(bus) = BUS.get_or_init(|| zbus::blocking::Connection::session().ok()) else { return SystemStyle::default() };
        SystemStyle { dark: scheme(&setting(bus, "color-scheme")), accent: accent(&setting(bus, "accent-color")) }
    }

    /// One setting: `ReadOne` (portal version 2), else the older `Read`, which wraps the value
    /// in one more variant.
    fn setting(bus: &zbus::blocking::Connection, key: &str) -> Option<OwnedValue> {
        let call = |method: &str| {
            bus.call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.Settings"),
                method,
                &(NS, key),
            )
            .ok()
            .and_then(|m| m.body().deserialize::<OwnedValue>().ok())
        };
        call("ReadOne").or_else(|| call("Read")).map(unwrap)
    }

    /// The value inside any variants around it (at most a few: the reply is the desktop's).
    fn unwrap(v: OwnedValue) -> OwnedValue {
        let mut v = v;
        for _ in 0..4 {
            let inner = match &*v {
                Value::Value(inner) => inner.try_to_owned().ok(),
                _ => None,
            };
            match inner {
                Some(i) => v = i,
                None => break,
            }
        }
        v
    }

    /// `color-scheme`: 1 prefers dark, 2 prefers light, 0 says nothing.
    pub(super) fn scheme(v: &Option<OwnedValue>) -> Option<bool> {
        match v.as_ref().and_then(|v| u32::try_from(v).ok()) {
            Some(1) => Some(true),
            Some(2) => Some(false),
            _ => None,
        }
    }

    /// `accent-color`: red, green and blue from 0 to 1; outside that range, none was chosen.
    pub(super) fn accent(v: &Option<OwnedValue>) -> Option<egui::Color32> {
        let (r, g, b) = v.as_ref().and_then(|v| <(f64, f64, f64)>::try_from(v.try_clone().ok()?).ok())?;
        let ch = |c: f64| (0.0..=1.0).contains(&c).then(|| (c * 255.0).round() as u8);
        Some(egui::Color32::from_rgb(ch(r)?, ch(g)?, ch(b)?))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn owned(v: Value<'_>) -> Option<OwnedValue> {
            v.try_to_owned().ok()
        }

        #[test]
        fn reads_the_portal_values() {
            assert_eq!(scheme(&owned(Value::U32(1))), Some(true));
            assert_eq!(scheme(&owned(Value::U32(2))), Some(false));
            assert_eq!(scheme(&owned(Value::U32(0))), None);
            assert_eq!(scheme(&owned(Value::from("dark"))), None);
            assert_eq!(scheme(&None), None);
            let blue = owned(Value::from((0.208_f64, 0.518_f64, 0.894_f64)));
            assert_eq!(accent(&blue), Some(egui::Color32::from_rgb(53, 132, 228)));
            // Out of range: no accent chosen.
            assert_eq!(accent(&owned(Value::from((-1.0_f64, 0.5_f64, 0.5_f64)))), None);
            assert_eq!(accent(&owned(Value::U32(3))), None);
            // `Read`'s extra variant comes off.
            let wrapped = owned(Value::Value(Box::new(Value::Value(Box::new(Value::U32(1))))));
            assert_eq!(scheme(&wrapped.map(unwrap)), Some(true));
        }
    }
}

/// The desktop's interface font on Linux and the BSDs: GNOME's (and its relatives') setting,
/// KDE's, else fontconfig's sans-serif; as a regular and a bold file. Only when both are their
/// own files (egui can't make a variable font bold), else `None` and the usual fonts are used.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn desktop_font_files() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let family = gnome_font().or_else(kde_font).unwrap_or_else(|| "sans-serif".to_string());
    let regular = fc_match(&format!("{family}:weight=regular"))?;
    let bold = fc_match(&format!("{family}:weight=bold"))?;
    (regular != bold).then_some((regular, bold))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn gnome_font() -> Option<String> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_ascii_lowercase();
    if !["gnome", "unity", "cinnamon", "mate", "budgie", "pantheon", "xfce"].iter().any(|d| desktop.contains(d)) {
        return None;
    }
    let out = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "font-name"]).output().ok()?;
    out.status.success().then_some(())?;
    font_family(String::from_utf8(out.stdout).ok()?.trim().trim_matches('\''))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn kde_font() -> Option<String> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_ascii_lowercase();
    if !desktop.contains("kde") {
        return None;
    }
    let home = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".config")))?;
    kde_general_font(&std::fs::read_to_string(home.join("kdeglobals")).ok()?)
}

/// `font=` in kdeglobals' `[General]` group: the family before the first comma.
#[cfg(all(unix, not(target_os = "macos")))]
fn kde_general_font(ini: &str) -> Option<String> {
    let mut general = false;
    for line in ini.lines().map(str::trim) {
        if line.starts_with('[') {
            general = line == "[General]";
        } else if general && let Some(v) = line.strip_prefix("font=") {
            let family = v.split(',').next().unwrap_or("").trim();
            return (!family.is_empty()).then(|| family.to_string());
        }
    }
    None
}

/// The family in a GTK font description (`Cantarell 11`, `Noto Sans Bold 10`): the words before
/// the size and any style words.
#[cfg(all(unix, not(target_os = "macos")))]
fn font_family(desc: &str) -> Option<String> {
    const STYLES: [&str; 8] = ["bold", "italic", "oblique", "light", "medium", "semibold", "regular", "condensed"];
    let mut words: Vec<&str> = desc.split_whitespace().collect();
    if words.last().is_some_and(|w| w.parse::<f32>().is_ok()) {
        words.pop();
    }
    while words.len() > 1 && words.last().is_some_and(|w| STYLES.contains(&w.to_ascii_lowercase().as_str())) {
        words.pop();
    }
    let family = words.join(" ");
    // A name fontconfig takes as a pattern: no characters that start its own syntax.
    (!family.is_empty() && !family.contains([':', '=', ','])).then_some(family)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn fc_match(pattern: &str) -> Option<std::path::PathBuf> {
    fc_match_named(pattern).map(|(_, path)| path)
}

/// fontconfig's match for a pattern: the family it found, and its file.
#[cfg(all(unix, not(target_os = "macos")))]
fn fc_match_named(pattern: &str) -> Option<(String, std::path::PathBuf)> {
    let out = std::process::Command::new("fc-match").args(["--format=%{family}|%{file}", pattern]).output().ok()?;
    out.status.success().then_some(())?;
    let text = String::from_utf8(out.stdout).ok()?;
    let (family, file) = text.trim().split_once('|')?;
    let path = std::path::PathBuf::from(file.trim());
    path.is_file().then(|| (family.split(',').next().unwrap_or("").trim().to_string(), path))
}

/// The files of a typeface by name (`Arial`, `Calibri`), for the message editor: its regular
/// face, and its bold when there's one of its own. On Linux fontconfig answers, with the font
/// standing in for one that isn't installed (Liberation Sans for Arial, Carlito for Calibri),
/// but not with its last resort for a name it doesn't know; on Windows and macOS the usual
/// files are looked for by name.
pub fn typeface_files(family: &str) -> Option<(std::path::PathBuf, Option<std::path::PathBuf>)> {
    let family = family.trim();
    if family.is_empty() || family.len() > 64 || family.contains([':', '=', ',', '/', '\\']) {
        return None;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let (found, regular) = fc_match_named(&format!("{family}:weight=regular"))?;
        let named = found.eq_ignore_ascii_case(family);
        // The last resort (the default sans, serif or monospace) isn't the typeface asked for.
        // The same three for every typeface: asked of fontconfig once, not three processes
        // more per name, run on the window's thread as a message opens.
        static LAST_RESORT: std::sync::OnceLock<Vec<std::path::PathBuf>> = std::sync::OnceLock::new();
        let last_resort =
            LAST_RESORT.get_or_init(|| ["sans-serif", "serif", "monospace"].iter().filter_map(|g| fc_match(g)).collect()).contains(&regular);
        if !named && last_resort {
            return None;
        }
        let bold = fc_match(&format!("{family}:weight=bold")).filter(|b| *b != regular);
        return Some((regular, bold));
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        let (dir, files): (std::path::PathBuf, &[(&str, &str, &str)]) = if cfg!(target_os = "windows") {
            (
                std::path::PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into())).join("Fonts"),
                &[
                    ("arial", "arial.ttf", "arialbd.ttf"),
                    ("calibri", "calibri.ttf", "calibrib.ttf"),
                    ("cambria", "cambria.ttc", "cambriab.ttf"),
                    ("courier new", "cour.ttf", "courbd.ttf"),
                    ("georgia", "georgia.ttf", "georgiab.ttf"),
                    ("segoe ui", "segoeui.ttf", "segoeuib.ttf"),
                    ("tahoma", "tahoma.ttf", "tahomabd.ttf"),
                    ("times new roman", "times.ttf", "timesbd.ttf"),
                    ("verdana", "verdana.ttf", "verdanab.ttf"),
                ],
            )
        } else {
            (
                std::path::PathBuf::from("/System/Library/Fonts/Supplemental"),
                &[
                    ("arial", "Arial.ttf", "Arial Bold.ttf"),
                    ("courier new", "Courier New.ttf", "Courier New Bold.ttf"),
                    ("georgia", "Georgia.ttf", "Georgia Bold.ttf"),
                    ("tahoma", "Tahoma.ttf", "Tahoma Bold.ttf"),
                    ("times new roman", "Times New Roman.ttf", "Times New Roman Bold.ttf"),
                    ("verdana", "Verdana.ttf", "Verdana Bold.ttf"),
                ],
            )
        };
        let lower = family.to_ascii_lowercase();
        let (_, regular, bold) = files.iter().find(|(name, _, _)| *name == lower)?;
        let regular = dir.join(regular);
        let bold = dir.join(bold);
        return regular.is_file().then(|| (regular, bold.is_file().then_some(bold)));
    }
    #[allow(unreachable_code)]
    None
}

/// The system's colour emoji font, for emoji drawn as pictures: `QUATRELLE_EMOJI_FONT` (a font
/// file, to try another), else Segoe UI Emoji on Windows, Apple Color Emoji on macOS, and on
/// Linux and the BSDs fontconfig's answer for `emoji` and the places Noto Color Emoji is
/// packaged in; the first that has colour glyphs. Its file and the face index in a collection.
pub fn emoji_font_file() -> Option<(std::path::PathBuf, u32)> {
    let mut candidates: Vec<std::path::PathBuf> = std::env::var_os("QUATRELLE_EMOJI_FONT").map(std::path::PathBuf::from).into_iter().collect();
    if cfg!(target_os = "windows") {
        let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        candidates.push(std::path::PathBuf::from(dir).join("Fonts").join("seguiemj.ttf"));
    } else if cfg!(target_os = "macos") {
        candidates.push("/System/Library/Fonts/Apple Color Emoji.ttc".into());
    } else {
        #[cfg(all(unix, not(target_os = "macos")))]
        candidates.extend(fc_match("emoji"));
        candidates.extend(
            [
                "/usr/share/fonts/google-noto-color-emoji-fonts/NotoColorEmoji.ttf",
                "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
                "/usr/share/fonts/noto-emoji/NotoColorEmoji.ttf",
                "/usr/share/fonts/noto/NotoColorEmoji.ttf",
                "/usr/share/fonts/TTF/NotoColorEmoji.ttf",
            ]
            .map(std::path::PathBuf::from),
        );
    }
    candidates.into_iter().find(|p| p.is_file() && std::fs::File::open(p).ok().is_some_and(|f| colour_font(f, 0))).map(|p| (p, 0))
}

/// Whether a font file's face `index` has colour glyphs (a `CBDT`, `sbix` or `COLR` table):
/// read from its table directory, the first few kilobytes, so a monochrome font (fontconfig
/// may answer `emoji` with one) costs no more than that.
fn colour_font(mut file: impl std::io::Read, index: u32) -> bool {
    let mut head = vec![0_u8; 16 << 10];
    let mut read = 0;
    while read < head.len() {
        match file.read(&mut head[read..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => read += n,
        }
    }
    head.truncate(read);
    has_colour_tables(&head, index)
}

/// Whether the font (or face `index` of a collection) in `bytes` lists a colour table.
fn has_colour_tables(bytes: &[u8], index: u32) -> bool {
    let u32_at = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let u16_at = |at: usize| bytes.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    // A collection: its faces' offset tables are listed after the header.
    let start = if bytes.starts_with(b"ttcf") { u32_at(12 + 4 * index as usize).map(|o| o as usize) } else { Some(0) };
    let Some(start) = start else { return false };
    let Some(count) = u16_at(start + 4) else { return false };
    (0..usize::from(count).min(512))
        .filter_map(|i| bytes.get(start + 12 + 16 * i..start + 16 + 16 * i))
        .any(|tag| matches!(tag, b"CBDT" | b"sbix" | b"COLR"))
}

#[cfg(test)]
mod colour_tests {
    use super::*;

    /// A font with a `COLR`, `sbix` or `CBDT` table in its directory is a colour font, in a
    /// collection too; a text font, garbage or a truncated file is not.
    #[test]
    fn reads_colour_tables_from_the_directory() {
        let sfnt = |tags: &[&[u8; 4]]| -> Vec<u8> {
            let mut v = vec![0, 1, 0, 0];
            v.extend_from_slice(&(tags.len() as u16).to_be_bytes());
            v.extend_from_slice(&[0; 6]);
            for t in tags {
                v.extend_from_slice(*t);
                v.extend_from_slice(&[0; 12]);
            }
            v
        };
        assert!(has_colour_tables(&sfnt(&[b"cmap", b"CBDT", b"glyf"]), 0));
        assert!(has_colour_tables(&sfnt(&[b"COLR", b"CPAL"]), 0));
        assert!(has_colour_tables(&sfnt(&[b"sbix"]), 0));
        assert!(!has_colour_tables(&sfnt(&[b"cmap", b"glyf", b"hmtx"]), 0));
        assert!(!has_colour_tables(b"not a font", 0));
        assert!(!has_colour_tables(&sfnt(&[b"CBDT"])[..10], 0), "truncated");
        // A collection of two: text, then colour.
        let (text, colour) = (sfnt(&[b"glyf"]), sfnt(&[b"sbix"]));
        let mut ttc = b"ttcf\0\x01\0\0\0\0\0\x02".to_vec();
        let first = 12 + 8;
        ttc.extend_from_slice(&(first as u32).to_be_bytes());
        ttc.extend_from_slice(&((first + text.len()) as u32).to_be_bytes());
        ttc.extend_from_slice(&text);
        ttc.extend_from_slice(&colour);
        assert!(!has_colour_tables(&ttc, 0));
        assert!(has_colour_tables(&ttc, 1));
        assert!(!has_colour_tables(&ttc, 2), "past the end");
        assert!(colour_font(std::io::Cursor::new(&ttc), 1));
    }
}

#[cfg(all(test, unix, not(target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn reads_the_desktop_font_names() {
        assert_eq!(font_family("Cantarell 11").as_deref(), Some("Cantarell"));
        assert_eq!(font_family("Adwaita Sans 11").as_deref(), Some("Adwaita Sans"));
        assert_eq!(font_family("Noto Sans Bold 10").as_deref(), Some("Noto Sans"));
        assert_eq!(font_family("Ubuntu").as_deref(), Some("Ubuntu"));
        assert_eq!(font_family("").as_deref(), None);
        assert_eq!(font_family("Evil:weight=0 11").as_deref(), None);
        let ini = "[Colors:View]\nfont=Wrong,1\n[General]\nColorScheme=BreezeDark\nfont=Noto Sans,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n";
        assert_eq!(kde_general_font(ini).as_deref(), Some("Noto Sans"));
        assert_eq!(kde_general_font("[General]\nfont=\n").as_deref(), None);
        assert_eq!(kde_general_font("font=Noto Sans,10\n").as_deref(), None);
    }
}
