//! Underlining misspelt words in the message being written.
//!
//! The dictionaries (French and English built in, and the system's: `/usr/share/hunspell`) are
//! read once, in the background, so the first message opens at once; until they're ready
//! nothing is underlined, and in the browser nothing is either (a page doesn't read two
//! megabytes of dictionary at start-up).
//!
//! A message is checked paragraph by paragraph, each remembered by its text: the editor lays
//! the text out on every frame, and only the paragraph being typed in is looked at again.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};

use quatrelle_spell::Spell;

static SPELL: OnceLock<Spell> = OnceLock::new();
#[cfg(not(target_arch = "wasm32"))]
static STARTED: std::sync::Once = std::sync::Once::new();

/// The words the writer added to the dictionary (preferences keep them between starts).
static PERSONAL: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// Whether words are underlined as they're typed (Options: Check the spelling as you type).
static ON: AtomicBool = AtomicBool::new(true);

/// Changes whenever what counts as right changes (a word added to the dictionary or taken
/// out): the paragraphs remembered are keyed by it, so they're looked at again.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The misspelt words of each paragraph looked at, by a hash of its text, its languages and
/// the generation: ranges within the paragraph.
static CACHE: Mutex<Option<HashMap<u64, Vec<(Range<usize>, String)>>>> = Mutex::new(None);
/// Most paragraphs remembered (then all forgotten at once).
const CACHE_MAX: usize = 4096;

/// The words let be in each message (Ignore all), by message id.
static IGNORED: Mutex<Vec<(u64, Vec<String>)>> = Mutex::new(Vec::new());
/// Most messages whose words are kept, and most words for each.
const IGNORED_MAX: usize = 64;

/// Where the system keeps Hunspell dictionaries, read when the app starts (never bundled).
#[cfg(not(target_arch = "wasm32"))]
const SYSTEM_DIRS: [&str; 4] = ["/usr/share/hunspell", "/usr/share/myspell", "/usr/share/myspell/dicts", "/usr/local/share/hunspell"];
/// A dictionary file larger than this isn't read.
#[cfg(not(target_arch = "wasm32"))]
const DICT_MAX: u64 = 64 << 20;

/// What a check goes by: the languages, the stretches of the text left alone, and the words
/// let be in the message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Check {
    /// The dictionaries used, by code (`fr`); empty: every dictionary.
    pub langs: Vec<String>,
    /// Byte ranges not checked (the signature, the original under a reply).
    pub skip: Vec<Range<usize>>,
    /// Words let be in this message (Ignore all), lower-case.
    pub ignored: Vec<String>,
}

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
            Ok(mut s) => {
                for (lang, aff, dic) in system_dictionaries() {
                    match s.add(&lang, &aff, &dic) {
                        Ok(true) => log::info!("spelling: {lang} dictionary read from the system"),
                        Ok(false) => {}
                        Err(e) => log::warn!("spelling: the system's {lang} dictionary wasn't read: {e}"),
                    }
                }
                let _ = SPELL.set(s);
                bump();
            }
            Err(e) => log::warn!("spelling is off: {e}"),
        });
    });
}

/// The system's Hunspell dictionaries, one per language (`de_DE` for German, when there are
/// several: the one named after its own country, else the first), as (code, aff, dic). Those
/// built in (French, English) aren't read again.
#[cfg(not(target_arch = "wasm32"))]
fn system_dictionaries() -> Vec<(String, String, String)> {
    use std::path::PathBuf;
    let mut dirs: Vec<PathBuf> = SYSTEM_DIRS.iter().map(PathBuf::from).collect();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/hunspell"));
    }
    let mut taken: Vec<String> = vec!["fr".into(), "en".into()];
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut stems: Vec<String> =
            entries.filter_map(|e| e.ok()).filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".dic")).map(str::to_string)).collect();
        stems.sort();
        let mut langs: Vec<String> = stems.iter().map(|s| language_of(s)).filter(|l| !l.is_empty() && !taken.contains(l)).collect();
        langs.dedup();
        for lang in langs {
            let own = format!("{lang}_{}", lang.to_ascii_uppercase());
            let Some(stem) = stems.iter().find(|s| **s == own).or_else(|| stems.iter().find(|s| language_of(s) == lang)) else { continue };
            let (Some(aff), Some(dic)) = (read_dictionary(&dir.join(format!("{stem}.aff"))), read_dictionary(&dir.join(format!("{stem}.dic"))))
            else {
                continue;
            };
            taken.push(lang.clone());
            out.push((lang, aff, dic));
        }
    }
    out
}

/// The language of a dictionary file's stem: `de` from `de_DE`, `de-AT` or `de`; empty when
/// it isn't named that way.
#[cfg(not(target_arch = "wasm32"))]
fn language_of(stem: &str) -> String {
    let lang: String = stem.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let rest = stem.get(lang.len()..).unwrap_or("");
    if (2..=3).contains(&lang.len()) && (rest.is_empty() || rest.starts_with(['_', '-'])) { lang.to_ascii_lowercase() } else { String::new() }
}

/// A dictionary file's text: UTF-8, or Latin-1 (the encoding older dictionaries use).
#[cfg(not(target_arch = "wasm32"))]
fn read_dictionary(path: &std::path::Path) -> Option<String> {
    let size = std::fs::metadata(path).ok()?.len();
    if size > DICT_MAX {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(String::from_utf8(bytes).unwrap_or_else(|e| e.into_bytes().iter().map(|b| char::from(*b)).collect()))
}

/// Read the dictionaries now: the tests, which can't wait for the background thread.
#[cfg(test)]
pub(crate) fn load_for_test() -> bool {
    if SPELL.get().is_none()
        && let Ok(s) = Spell::built_in()
    {
        let _ = SPELL.set(s);
        bump();
    }
    SPELL.get().is_some()
}

/// The dictionaries, once read: `None` before that, and in the browser.
pub fn ready() -> Option<&'static Spell> {
    SPELL.get()
}

/// The languages there are dictionaries for (`fr`, `en`, then the system's), once read.
pub fn languages() -> Vec<String> {
    ready().map(|s| s.languages().into_iter().map(str::to_string).collect()).unwrap_or_default()
}

/// Whether there's a dictionary for `lang`.
pub fn has(lang: &str) -> bool {
    ready().is_some_and(|s| s.has(lang))
}

/// Everything remembered is stale: a word was added or taken out.
fn bump() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// The words the writer added, from the preferences (lower-case).
pub fn set_personal(words: Vec<String>) {
    if let Ok(mut p) = PERSONAL.write() {
        *p = words.into_iter().map(|w| w.to_lowercase()).collect();
    }
    bump();
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
    bump();
}

/// Stop accepting a word the writer added (the command takes it out of the preferences).
pub fn remove_word(word: &str) {
    let w = word.trim().to_lowercase();
    if let Ok(mut p) = PERSONAL.write() {
        p.retain(|x| *x != w);
    }
    bump();
}

/// Whether the writer added this word (it can then be taken back out).
pub fn added(word: &str) -> bool {
    is_personal(word)
}

/// The list is lower-case; the word is compared in lower case too, whatever its letters
/// (`Été` is `été`, which `eq_ignore_ascii_case` didn't know).
fn is_personal(word: &str) -> bool {
    let w = word.to_lowercase();
    PERSONAL.read().is_ok_and(|p| p.contains(&w))
}

/// Let `word` be in message `message` (Outlook's Ignore All): it isn't marked there again.
pub fn ignore_all(message: u64, word: &str) {
    let w = word.trim().to_lowercase();
    if w.is_empty() {
        return;
    }
    if let Ok(mut all) = IGNORED.lock() {
        if let Some((_, words)) = all.iter_mut().find(|(id, _)| *id == message) {
            if !words.contains(&w) && words.len() < 1000 {
                words.push(w);
            }
            return;
        }
        if all.len() >= IGNORED_MAX {
            all.remove(0);
        }
        all.push((message, vec![w]));
    }
}

/// The words let be in message `message`.
pub fn ignored(message: u64) -> Vec<String> {
    IGNORED.lock().ok().and_then(|all| all.iter().find(|(id, _)| *id == message).map(|(_, w)| w.clone())).unwrap_or_default()
}

/// The words of `text` that aren't in the dictionaries nor in the writer's own list, with their
/// byte ranges (the tests' shorthand for a check with nothing particular).
#[cfg(test)]
pub(crate) fn misspelt(text: &str) -> Vec<(Range<usize>, String)> {
    misspelt_with(text, &Check::default())
}

/// [`misspelt`] as `check` says: in its languages, outside the stretches it skips, and not the
/// words it lets be. Paragraphs already looked at (same text, languages and dictionary) are
/// answered from memory.
pub fn misspelt_with(text: &str, check: &Check) -> Vec<(Range<usize>, String)> {
    let Some(spell) = ready() else { return Vec::new() };
    let generation = GENERATION.load(Ordering::Relaxed);
    let langs: Vec<&str> = check.langs.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    let mut at = 0usize;
    for para in text.split('\n') {
        if !para.trim().is_empty() {
            let mut h = std::hash::DefaultHasher::new();
            (generation, &check.langs, para).hash(&mut h);
            let key = h.finish();
            let cached = CACHE.lock().ok().and_then(|c| c.as_ref().and_then(|c| c.get(&key).cloned()));
            let found = match cached {
                Some(v) => v,
                None => {
                    let v: Vec<(Range<usize>, String)> =
                        spell.misspelt_in(para, &langs).into_iter().filter(|(_, w)| !is_personal(w)).map(|(r, w)| (r, w.to_string())).collect();
                    if let Ok(mut c) = CACHE.lock() {
                        let map = c.get_or_insert_with(HashMap::new);
                        if map.len() >= CACHE_MAX {
                            map.clear();
                        }
                        map.insert(key, v.clone());
                    }
                    v
                }
            };
            out.extend(found.into_iter().map(|(r, w)| (r.start + at..r.end + at, w)));
        }
        at += para.len() + 1;
    }
    out.retain(|(r, w)| {
        !check.skip.iter().any(|s| r.start < s.end && r.end > s.start)
            && !check.ignored.iter().any(|i| i.eq_ignore_ascii_case(w) || *i == w.to_lowercase())
    });
    out
}

/// What could be meant instead of a word, the closest first (French suggestions first among
/// those as close), from every dictionary.
#[cfg(test)]
pub(crate) fn suggest(word: &str, max: usize) -> Vec<String> {
    suggest_in(word, &[], max)
}

/// [`suggest`] from the dictionaries of `langs` (every one when empty), the first one's words
/// first among those as close.
pub fn suggest_in(word: &str, langs: &[String], max: usize) -> Vec<String> {
    let langs: Vec<&str> = langs.iter().map(String::as_str).collect();
    ready().map(|s| s.suggest_in(word, &langs, max)).unwrap_or_default()
}

/// Underline the words `job` lays out that aren't spelt right, in `text`'s own coordinates (the
/// job's text is that text, piece by piece): a red line under each, as Outlook marks them
/// (the tests' shorthand for a check with nothing particular).
#[cfg(test)]
pub(crate) fn underline(job: &mut egui::text::LayoutJob, text: &str, colour: egui::Color32) {
    underline_with(job, text, colour, &Check::default());
}

/// Underline the words `job` lays out that aren't spelt right as `check` says, in `text`'s own
/// coordinates (the job's text is that text, piece by piece): a red line under each, as
/// Outlook marks them.
pub fn underline_with(job: &mut egui::text::LayoutJob, text: &str, colour: egui::Color32, check: &Check) {
    if !on() || ready().is_none() || job.text != text {
        return;
    }
    let bad = misspelt_with(text, check);
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
        assert!(load_for_test());
        let text = "Le serveur quatrelle est en panne";
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
        assert_eq!(marked, ["quatrelle"], "only the word that isn't in the dictionaries");
    }

    /// What a check goes by: the message's language, the stretches left alone (the signature,
    /// the original under a reply) and the words let be; and a word the writer added counts
    /// whatever its letters (`Été`, which an ASCII comparison missed).
    #[test]
    fn a_check_skips_what_it_is_told_to() {
        assert!(load_for_test());
        let text = "Le servver est là\nquatrelle aussi\n-- \nDidier quatrelle";
        let all = misspelt(text);
        assert_eq!(all.iter().map(|(_, w)| w.as_str()).collect::<Vec<_>>(), ["servver", "quatrelle", "quatrelle"]);
        let sig = text.find("-- ").unwrap_or(0);
        let check = Check { langs: vec!["fr".into()], skip: std::iter::once(sig..text.len()).collect(), ignored: vec!["quatrelle".into()] };
        assert_eq!(misspelt_with(text, &check).iter().map(|(_, w)| w.as_str()).collect::<Vec<_>>(), ["servver"]);
        // Remembered: the same text, in the same languages, answers the same (and faster).
        assert_eq!(misspelt_with(text, &check), misspelt_with(text, &check));
        // In English alone, French words are wrong; in both, nothing of either is.
        let en = Check { langs: vec!["en".into()], ..Default::default() };
        assert!(misspelt_with("Le serveur", &en).iter().any(|(_, w)| w == "serveur"));
        assert!(misspelt_with("Le serveur backup", &Check::default()).is_empty());
        // Ignore all is kept for the message, and nothing else.
        ignore_all(7, "Quatrelle");
        assert_eq!(ignored(7), ["quatrelle"]);
        assert!(ignored(8).is_empty());
        // A word added with its accent, whatever its case afterwards.
        add_word("Été");
        assert!(added("ÉTÉ") && added("été"));
        assert!(misspelt("ÉTÉ").is_empty());
        remove_word("été");
        assert!(!added("Été"));
        // Suggestions: the closest word first, in the language asked for.
        assert_eq!(suggest_in("teh", &["en".into()], 3).first().map(String::as_str), Some("the"));
    }
}
