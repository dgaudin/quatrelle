//! The message editor's small buttons (formats, colours), the date and time choices, and the
//! word count; the ribbon itself is `ribbon.rs`.

use super::*;

/// A ribbon button: an icon or a letter, lit when its format is on.
pub(super) fn fmt_button(ui: &mut Ui, t: &Tokens, face: Face<'_>, on: bool, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    crate::widgets::named(ui, &resp, tl!(tip), true, Some(on));
    if on || resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, if on { t.accent_soft } else { t.hover });
    }
    match face {
        Face::Icon(ic) => icons::paint(ui.painter(), icons::centered(rect, 16.0), ic, t.text),
        Face::Letter(s, f) => {
            let galley = ui.painter().layout_job(LayoutJob::single_section(s.to_string(), f));
            ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.text);
        }
    }
    resp.on_hover_text(tl!(tip))
}

pub(super) enum Face<'a> {
    Icon(Icon),
    Letter(&'a str, TextFormat),
}

/// A colour button with its dropdown (font colour, highlight).
pub(super) fn color_button(
    ui: &mut Ui,
    t: &Tokens,
    letter: &str,
    current: Option<[u8; 3]>,
    swatches: &[[u8; 3]],
    none: &str,
    tip: &str,
) -> Option<Option<[u8; 3]>> {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 28.0), Sense::click());
    crate::widgets::named(ui, &resp, tl!(tip), true, None);
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    ui.painter().text(rect.center() - vec2(4.0, 3.0), egui::Align2::CENTER_CENTER, letter, theme::regular(14.0), t.text);
    let bar = Rect::from_min_size(egui::pos2(rect.min.x + 6.0, rect.max.y - 7.0), vec2(16.0, 3.5));
    let shown = current.map_or(if letter == "A" { t.text } else { Color32::TRANSPARENT }, |c| Color32::from_rgb(c[0], c[1], c[2]));
    ui.painter().rect_filled(bar, 1.0, shown);
    icons::paint(ui.painter(), Rect::from_min_size(egui::pos2(rect.max.x - 11.0, rect.center().y - 4.0), vec2(8.0, 8.0)), Icon::ChevronDown, t.dim);
    let resp = resp.on_hover_text(tl!(tip));
    let mut picked = None;
    egui::Popup::menu(&resp).show(|ui| {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(6.0 * 26.0);
            for c in swatches {
                let (r, s) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                // A stronger edge under the pointer.
                let edge = if s.hovered() { Stroke::new(2.0, t.text) } else { Stroke::new(1.0, t.border) };
                ui.painter().rect(r, 3.0, Color32::from_rgb(c[0], c[1], c[2]), edge, egui::StrokeKind::Inside);
                if s.clicked() {
                    picked = Some(Some(*c));
                }
            }
        });
        if ui.button(tl!(none)).clicked() {
            picked = Some(None);
        }
    });
    picked
}

/// The ways today's date and the time can go in: written out, as numbers, with the time, the
/// time alone.
pub(crate) fn date_choices(clock: &Clock) -> Vec<String> {
    let now = quatrelle_engine::model::dates::civil(clock.now, clock);
    let day = quatrelle_engine::model::dates::today(clock);
    let iso = format!("{:04}-{:02}-{:02}", now.year, now.month, now.day);
    let time = format!("{:02}:{:02}", now.hour, now.minute);
    vec![crate::calendar::long_date(day), iso.clone(), format!("{iso} {time}"), time]
}

/// What Word count shows: words, characters with and without spaces, paragraphs and lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub words: usize,
    pub chars: usize,
    pub chars_no_spaces: usize,
    pub paragraphs: usize,
    pub lines: usize,
}

/// Words in a run without spaces, as word processors count them: `2026-10-22` and `don't` are
/// one; in Chinese, Japanese and Korean each character is one (and the rest around them one each).
pub(super) fn token_words(token: &str) -> usize {
    let wide =
        |c: char| matches!(c as u32, 0x1100..=0x11FF | 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF66..=0xFF9F | 0x20000..=0x2FA1F);
    let mut n = 0;
    let mut in_run = false;
    for c in token.chars() {
        if wide(c) {
            n += 1;
            in_run = false;
        } else if c.is_alphanumeric() && !in_run {
            n += 1;
            in_run = true;
        }
    }
    // Punctuation alone (a dash between words) is no word.
    n
}

pub(crate) fn count_words(text: &str) -> Counts {
    Counts {
        words: text.split_whitespace().map(token_words).sum(),
        chars: text.chars().filter(|c| *c != '\n' && *c != '\r').count(),
        chars_no_spaces: text.chars().filter(|c| !c.is_whitespace()).count(),
        paragraphs: text.split('\n').filter(|l| !l.trim().is_empty()).count(),
        lines: if text.is_empty() { 0 } else { text.split('\n').count() },
    }
}
