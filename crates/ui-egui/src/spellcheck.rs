//! Underlining misspelt words in the message being written.
//!
//! The dictionaries (French, and English for technical terms) are read once, in the background,
//! so the first message opens at once; until they're ready nothing is underlined, and in the
//! browser nothing is either (a page doesn't read two megabytes of dictionary at start-up).

use mailcraft_spell::Spell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};

static SPELL: OnceLock<Spell> = OnceLock::new();
#[cfg(not(target_arch = "wasm32"))]
static STARTED: std::sync::Once = std::sync::Once::new();

/// The words the writer added to the dictionary (preferences keep them between starts).
static PERSONAL: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// Whether words are underlined as they're typed (Options: Check the spelling as you type).
static ON: AtomicBool = AtomicBool::new(true);

/// Underline words as they're typed, or not (the dialog still checks when asked).
pub fn set_on(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Read the dictionaries in the background. Called when the application starts (once).
pub fn start() {
    #[cfg(not(target_arch = "wasm32"))]
    STARTED.call_once(|| {
        std::thread::spawn(|| match Spell::built_in() {
            Ok(s) => {
                let _ = SPELL.set(s);
            }
            Err(e) => log::warn!("spelling is off: {e}"),
        });
    });
}

/// Read the dictionaries now: the tests, which can't wait for the background thread.
#[cfg(test)]
pub(crate) fn load_for_test() -> bool {
    if SPELL.get().is_none()
        && let Ok(s) = Spell::built_in()
    {
        let _ = SPELL.set(s);
    }
    SPELL.get().is_some()
}

/// The dictionaries, once read: `None` before that, and in the browser.
pub fn ready() -> Option<&'static Spell> {
    SPELL.get()
}

/// The words the writer added, from the preferences (lower-case).
pub fn set_personal(words: Vec<String>) {
    if let Ok(mut p) = PERSONAL.write() {
        *p = words.into_iter().map(|w| w.to_lowercase()).collect();
    }
}

/// Add a word to the dictionary for this run (the command puts it in the preferences).
pub fn add_word(word: &str) {
    let w = word.trim().to_lowercase();
    if w.is_empty() {
        return;
    }
    if let Ok(mut p) = PERSONAL.write()
        && !p.contains(&w)
    {
        p.push(w);
    }
}

fn is_personal(word: &str) -> bool {
    PERSONAL.read().is_ok_and(|p| p.iter().any(|x| x.eq_ignore_ascii_case(word)))
}

/// The words of `text` that aren't in the dictionaries nor in the writer's own list, with their
/// byte ranges.
pub fn misspelt(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let Some(spell) = ready() else { return Vec::new() };
    spell.misspelt(text).into_iter().filter(|(_, w)| !is_personal(w)).map(|(r, w)| (r, w.to_string())).collect()
}

/// What could be meant instead of a word (French suggestions first).
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    ready().map(|s| s.suggest(word, max)).unwrap_or_default()
}

/// Underline the words `job` lays out that aren't spelt right, in `text`'s own coordinates (the
/// job's text is that text, piece by piece): a red line under each, as Outlook marks them.
pub fn underline(job: &mut egui::text::LayoutJob, text: &str, colour: egui::Color32) {
    if !on() {
        return;
    }
    let Some(spell) = ready() else { return };
    if job.text != text {
        return;
    }
    let bad: Vec<(std::ops::Range<usize>, &str)> = spell.misspelt(text).into_iter().filter(|(_, w)| !is_personal(w)).collect();
    if bad.is_empty() {
        return;
    }
    let mut out = Vec::with_capacity(job.sections.len() + bad.len());
    for sec in std::mem::take(&mut job.sections) {
        let (from, to): (usize, usize) = (sec.byte_range.start.0, sec.byte_range.end.0);
        let mut at = from;
        for (r, _) in bad.iter().filter(|(r, _)| r.start < to && r.end > from) {
            let (s, e) = (r.start.max(from), r.end.min(to));
            if s > at {
                out.push(egui::text::LayoutSection { leading_space: sec.leading_space, byte_range: at.into()..s.into(), format: sec.format.clone() });
            }
            let mut f = sec.format.clone();
            f.underline = egui::Stroke::new(1.0, colour);
            out.push(egui::text::LayoutSection { leading_space: 0.0, byte_range: s.into()..e.into(), format: f });
            at = e;
        }
        if at < to {
            out.push(egui::text::LayoutSection { leading_space: 0.0, byte_range: at.into()..to.into(), format: sec.format.clone() });
        }
    }
    job.sections = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A section per stretch of text, the misspelt ones underlined, and nothing lost on the way.
    #[test]
    fn words_are_underlined_without_moving_the_text() {
        // The dictionaries are read here (the test thread), as the app does in the background.
        if ready().is_none()
            && let Ok(s) = Spell::built_in()
        {
            let _ = SPELL.set(s);
        }
        let text = "Le serveur mailcraft est en panne";
        let mut job = egui::text::LayoutJob::simple(text.to_string(), egui::FontId::default(), egui::Color32::WHITE, 400.0);
        underline(&mut job, text, egui::Color32::RED);
        let rebuilt: String =
            job.sections.iter().filter_map(|s| job.text.get(s.byte_range.start.0..s.byte_range.end.0).map(str::to_string)).collect();
        assert_eq!(rebuilt, text, "the text itself is unchanged");
        let marked: Vec<&str> = job
            .sections
            .iter()
            .filter(|s| s.format.underline.width > 0.0)
            .filter_map(|s| job.text.get(s.byte_range.start.0..s.byte_range.end.0))
            .collect();
        assert_eq!(marked, ["mailcraft"], "only the word that isn't in the dictionaries");
    }
}
