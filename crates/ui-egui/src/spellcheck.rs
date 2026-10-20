//! Underlining misspelt words in the message being written.
//!
//! The dictionaries (French, and English for technical terms) are read once, in the background,
//! so the first message opens at once; until they're ready nothing is underlined, and in the
//! browser nothing is either (a page doesn't read two megabytes of dictionary at start-up).

use mailcraft_spell::Spell;
use std::sync::OnceLock;

static SPELL: OnceLock<Spell> = OnceLock::new();
#[cfg(not(target_arch = "wasm32"))]
static STARTED: std::sync::Once = std::sync::Once::new();

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

/// The dictionaries, once read: `None` before that, and in the browser.
pub fn ready() -> Option<&'static Spell> {
    SPELL.get()
}

/// Underline the words `job` lays out that aren't spelt right, in `text`'s own coordinates (the
/// job's text is that text, piece by piece): a red line under each, as Outlook marks them.
pub fn underline(job: &mut egui::text::LayoutJob, text: &str, colour: egui::Color32) {
    let Some(spell) = ready() else { return };
    if job.text != text {
        return;
    }
    let bad = spell.misspelt(text);
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
