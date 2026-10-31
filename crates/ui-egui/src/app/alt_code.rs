//! Alt+numpad character codes on Windows (issue #23): hold Alt, type a code on the numeric
//! keypad, let go of Alt, and its character is typed (`0201` → É), in any text. Windows makes
//! the character itself, but the window library drops it (it comes after the keys, with none of
//! its own), so the digits are collected here and the character typed on Alt's release, in the
//! system's code pages ([`quatrelle_engine::mime::alt_code_char`]). After WordCraft's.

/// A code another key ended, while Alt is still held.
const INTERRUPTED: &str = "-";

/// The digits typed so far with Alt held, in a window.
fn typed(viewport: egui::ViewportId) -> egui::Id {
    egui::Id::unique("alt-code").with(viewport)
}

/// The Windows code pages for the user's language (ANSI, OEM), which Alt codes are read in.
pub fn alt_code_pages() -> (u16, u16) {
    quatrelle_engine::mime::alt_code_pages(sys_locale::get_locale().as_deref())
}

/// Run on a window's input before egui reads it: digits typed with Alt alone held are taken out
/// and collected; when Alt is let go, the character their code names is typed as text. egui doesn't
/// tell the keypad's digits from the top row's, so those compose a code too (with Alt alone they
/// do nothing else). Any other key, or the window losing focus, drops a half-typed code.
pub fn alt_codes(ctx: &egui::Context, raw: &mut egui::RawInput, pages: (u16, u16)) {
    use egui::Key;
    let id = typed(raw.viewport_id);
    let mut code: String = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    let alt_only = |m: &egui::Modifiers| m.alt && !m.ctrl && !m.shift && !m.command && !m.mac_cmd;
    let digit = |k: Key| match k {
        Key::Num0 => Some('0'),
        Key::Num1 => Some('1'),
        Key::Num2 => Some('2'),
        Key::Num3 => Some('3'),
        Key::Num4 => Some('4'),
        Key::Num5 => Some('5'),
        Key::Num6 => Some('6'),
        Key::Num7 => Some('7'),
        Key::Num8 => Some('8'),
        Key::Num9 => Some('9'),
        _ => None,
    };
    let mut out = Vec::with_capacity(raw.events.len());
    // A digit taken out just before: the text egui-winit sends right after its key is its own.
    let mut took_digit = false;
    for e in raw.events.drain(..) {
        let after_digit = std::mem::take(&mut took_digit);
        match &e {
            egui::Event::Key { key, pressed, repeat, modifiers, .. } if alt_only(modifiers) && digit(*key).is_some() => {
                took_digit = *pressed;
                // A held digit's repeat adds nothing (as on Windows); codes past seven digits
                // name nothing, so collecting stops.
                if let Some(d) = digit(*key).filter(|_| *pressed && !*repeat && code.len() < 8 && code != INTERRUPTED) {
                    code.push(d);
                }
            }
            // Its digits' text, where a platform sends it with Alt held: never typed as digits,
            // also after another key ended the code.
            egui::Event::Text(t)
                if (after_digit || (!code.is_empty() && code != INTERRUPTED)) && !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()) => {}
            // Alt let go: the code's character, where it was typed.
            egui::Event::ModifiersChanged(m) if !m.alt && !code.is_empty() => {
                if let Some(c) = quatrelle_engine::mime::alt_code_char(&code, pages.0, pages.1) {
                    out.push(egui::Event::Text(c.to_string()));
                }
                code.clear();
                out.push(e);
            }
            // Another key with Alt held ends the code unfinished, and nothing more is typed
            // until Alt is let go (as on Windows); without Alt, there's nothing to end.
            egui::Event::Key { pressed: true, modifiers, .. } => {
                code = if modifiers.alt { INTERRUPTED.to_string() } else { String::new() };
                out.push(e);
            }
            egui::Event::WindowFocused(false) => {
                code.clear();
                out.push(e);
            }
            _ => out.push(e),
        }
    }
    raw.events = out;
    ctx.data_mut(|d| d.insert_temp(id, code));
}

#[cfg(test)]
mod tests {
    use egui::{Event, Key, Modifiers};

    /// One window's input through the Alt codes, French Windows' code pages (1252, 850).
    fn through(ctx: &egui::Context, events: Vec<Event>) -> Vec<Event> {
        let mut raw = egui::RawInput { events, ..Default::default() };
        super::alt_codes(ctx, &mut raw, (1252, 850));
        raw.events
    }

    fn key(k: Key, pressed: bool, modifiers: Modifiers) -> Event {
        Event::Key { key: k, physical_key: Some(k), pressed, repeat: false, modifiers }
    }

    /// Alt held, each digit pressed (with the text a platform may send with it) and let go, Alt
    /// let go.
    fn code(digits: &str, with_text: bool) -> Vec<Event> {
        let mut v = vec![Event::ModifiersChanged(Modifiers::ALT)];
        for d in digits.chars() {
            let Some(k) = Key::from_name(&d.to_string()) else { continue };
            v.push(key(k, true, Modifiers::ALT));
            if with_text {
                v.push(Event::Text(d.to_string()));
            }
            v.push(key(k, false, Modifiers::ALT));
        }
        v.push(Event::ModifiersChanged(Modifiers::NONE));
        v
    }

    fn texts(events: &[Event]) -> String {
        events.iter().filter_map(|e| if let Event::Text(t) = e { Some(t.as_str()) } else { None }).collect()
    }

    /// French Windows: a leading 0 reads the ANSI page (1252), none the OEM page (850), above
    /// 255 Unicode; the digits never reach the text as digits, even when sent as text.
    #[test]
    fn a_code_types_its_character_and_not_its_digits() {
        let ctx = egui::Context::default();
        for (digits, want) in [("0201", "É"), ("0200", "È"), ("0199", "Ç"), ("0128", "€"), ("130", "é"), ("144", "É"), ("8364", "€")] {
            for with_text in [false, true] {
                let out = through(&ctx, code(digits, with_text));
                assert_eq!(texts(&out), want, "Alt+{digits}");
                assert!(!out.iter().any(|e| matches!(e, Event::Key { key: Key::Num0 | Key::Num1 | Key::Num2, .. })), "Alt+{digits}: {out:?}");
            }
        }
        // A held digit's repeat adds nothing: Alt+0201 with the 2 held is still É.
        let mut held = code("0201", false);
        held.insert(4, Event::Key { key: Key::Num2, physical_key: Some(Key::Num2), pressed: true, repeat: true, modifiers: Modifiers::ALT });
        assert_eq!(texts(&through(&ctx, held)), "É");
    }

    /// A code that names nothing types nothing: 0, a control character, a code too long, past
    /// Unicode.
    #[test]
    fn a_code_naming_nothing_types_nothing() {
        let ctx = egui::Context::default();
        for digits in ["0", "0000", "0009", "0127", "99999999", "1114112", "123456789012"] {
            let out = through(&ctx, code(digits, true));
            assert_eq!(texts(&out), "", "Alt+{digits}");
        }
    }

    /// Alt alone, and Alt with a letter (a menu's key), go on as they came; AltGr (Ctrl+Alt on
    /// Windows) is never a code, its digits and their text go through.
    #[test]
    fn alt_with_letters_and_altgr_are_left_alone() {
        let ctx = egui::Context::default();
        let alone = vec![Event::ModifiersChanged(Modifiers::ALT), Event::ModifiersChanged(Modifiers::NONE)];
        assert_eq!(through(&ctx, alone.clone()), alone);
        let alt_f = vec![
            Event::ModifiersChanged(Modifiers::ALT),
            key(Key::F, true, Modifiers::ALT),
            key(Key::F, false, Modifiers::ALT),
            Event::ModifiersChanged(Modifiers::NONE),
        ];
        assert_eq!(through(&ctx, alt_f.clone()), alt_f);
        let altgr = Modifiers::ALT | Modifiers::CTRL;
        let at = vec![
            Event::ModifiersChanged(altgr),
            key(Key::Num0, true, altgr),
            Event::Text("@".into()),
            key(Key::Num0, false, altgr),
            key(Key::Num2, true, altgr),
            Event::Text("~".into()),
            key(Key::Num2, false, altgr),
            Event::ModifiersChanged(Modifiers::NONE),
        ];
        assert_eq!(through(&ctx, at.clone()), at);
    }

    /// Another key ends the code: nothing is typed until Alt is let go, not the code's
    /// character, nor the digits that come after, even when sent as text.
    #[test]
    fn an_interrupted_code_types_nothing() {
        let ctx = egui::Context::default();
        let mut cut = code("0201", true);
        // After Alt+0, Alt+Tab.
        cut.insert(4, key(Key::Tab, true, Modifiers::ALT));
        let out = through(&ctx, cut);
        assert_eq!(texts(&out), "", "{out:?}");
        assert!(out.iter().any(|e| matches!(e, Event::Key { key: Key::Tab, .. })));
        // The next code, Alt held again, is read afresh.
        assert_eq!(texts(&through(&ctx, code("0199", true))), "Ç");
        // A window losing focus drops a half-typed code.
        let mut first = code("02", false);
        first.pop();
        through(&ctx, first);
        through(&ctx, vec![Event::WindowFocused(false)]);
        assert_eq!(texts(&through(&ctx, vec![Event::ModifiersChanged(Modifiers::NONE)])), "");
    }

    /// Each window has its own code: one typed half in the main window and one whole in a
    /// message's window don't mix.
    #[test]
    fn each_window_has_its_own_code() {
        let ctx = egui::Context::default();
        let message = egui::ViewportId::from_hash_of("message-window");
        let in_window = |viewport, events| {
            let mut raw = egui::RawInput { viewport_id: viewport, events, ..Default::default() };
            super::alt_codes(&ctx, &mut raw, (1252, 850));
            texts(&raw.events)
        };
        let mut whole = code("0201", false);
        let rest = whole.split_off(7);
        assert_eq!(in_window(egui::ViewportId::ROOT, whole), "");
        assert_eq!(in_window(message, code("0199", false)), "Ç");
        assert_eq!(in_window(egui::ViewportId::ROOT, rest), "É");
    }
}
