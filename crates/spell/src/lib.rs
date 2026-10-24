//! Spelling for the composer: Hunspell dictionaries, read natively (no C library), French and
//! English built in, and any the system has (German, Spanish…) added by the caller.
//!
//! The built-in dictionaries are the two `assets/dictionaries/` files, gunzipped and parsed
//! once; a word Quatrelle doesn't know can be added by the writer (a personal list, kept by the
//! caller). Pure Rust, no I/O of its own: [`Dict::new`] and [`Spell::add`] take the files' text.
//!
//! A check runs against the languages asked for (the message's), or all of them; a word in a
//! script no dictionary covers (Chinese in a French message) is left alone rather than marked.
//! Suggestions from several dictionaries are merged by how close they are to the word, the
//! first language asked for winning ties.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::HashSet;
use std::io::Read as _;
use std::ops::Range;

use flate2::read::GzDecoder;

/// A dictionary's two files, built in (gzipped), by language.
const BUILT_IN: &[(&str, &[u8], &[u8])] = &[
    ("fr", include_bytes!("../../../assets/dictionaries/fr_FR.aff.gz"), include_bytes!("../../../assets/dictionaries/fr_FR.dic.gz")),
    ("en", include_bytes!("../../../assets/dictionaries/en_US.aff.gz"), include_bytes!("../../../assets/dictionaries/en_US.dic.gz")),
];

/// Most dictionaries kept (the system's, past the built-in two).
const MAX_DICTS: usize = 24;

/// The writing system a word (or a dictionary) is in: a dictionary answers only for its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Script {
    Latin,
    Cyrillic,
    Greek,
    Arabic,
    Hebrew,
    /// Chinese, Japanese and Korean, which have no spelling to check this way.
    Cjk,
    Other,
}

/// The script of a character.
fn script_of_char(c: char) -> Option<Script> {
    if !c.is_alphabetic() {
        return None;
    }
    Some(match c as u32 {
        0x0041..=0x024F | 0x1E00..=0x1EFF | 0x2C60..=0x2C7F | 0xA720..=0xA7FF | 0xFF21..=0xFF5A => Script::Latin,
        0x0370..=0x03FF | 0x1F00..=0x1FFF => Script::Greek,
        0x0400..=0x052F | 0x2DE0..=0x2DFF | 0xA640..=0xA69F => Script::Cyrillic,
        0x0590..=0x05FF | 0xFB1D..=0xFB4F => Script::Hebrew,
        0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => Script::Arabic,
        0x1100..=0x11FF | 0x2E80..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xF900..=0xFAFF | 0xFF66..=0xFF9F | 0x20000..=0x2FA1F => Script::Cjk,
        _ => Script::Other,
    })
}

/// The script a word is written in: that of most of its letters.
pub fn script_of(word: &str) -> Script {
    let mut counts: Vec<(Script, usize)> = Vec::new();
    for s in word.chars().filter_map(script_of_char) {
        match counts.iter_mut().find(|(x, _)| *x == s) {
            Some((_, n)) => *n += 1,
            None => counts.push((s, 1)),
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map_or(Script::Other, |(s, _)| s)
}

/// The words' hashing, the same at every start: with one picked at random each time, words the
/// dictionary scores alike came in another order, and the last suggestions given changed from
/// one run to the next (the closest word sometimes left out).
type Fixed = std::hash::BuildHasherDefault<std::collections::hash_map::DefaultHasher>;

/// One Hunspell dictionary.
pub struct Dict {
    inner: spellbook::Dictionary<Fixed>,
    /// The script its words are in (from a sample of them).
    script: Script,
}

impl Dict {
    /// A dictionary from its `.aff` and `.dic` text.
    pub fn new(aff: &str, dic: &str) -> Result<Dict, String> {
        let script = dic_script(dic);
        spellbook::Dictionary::new_with_hasher(aff, dic, Fixed::default()).map(|inner| Dict { inner, script }).map_err(|e| e.to_string())
    }

    /// Whether the dictionary knows the word (its affixes included).
    pub fn check(&self, word: &str) -> bool {
        self.inner.check(word)
    }

    /// What could be meant instead, the likeliest first, at most `max` of them.
    pub fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        let mut out = Vec::new();
        self.inner.suggest(word, &mut out);
        out.truncate(max);
        out
    }

    pub fn script(&self) -> Script {
        self.script
    }
}

/// The script a `.dic` file's words are in: the commonest among its first few hundred.
fn dic_script(dic: &str) -> Script {
    let mut counts: Vec<(Script, usize)> = Vec::new();
    for line in dic.lines().skip(1).take(400) {
        let word = line.split(['/', '\t', ' ']).next().unwrap_or("");
        let s = script_of(word);
        if s == Script::Other {
            continue;
        }
        match counts.iter_mut().find(|(x, _)| *x == s) {
            Some((_, n)) => *n += 1,
            None => counts.push((s, 1)),
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map_or(Script::Latin, |(s, _)| s)
}

/// The dictionaries Quatrelle has: French and English built in, the system's added, and the
/// words the writer added.
pub struct Spell {
    dicts: Vec<(String, Dict)>,
    personal: HashSet<String>,
}

impl Spell {
    /// The built-in dictionaries. `Err` when one of them can't be read (never, in practice: they
    /// travel with the program), in which case spelling is simply not offered.
    pub fn built_in() -> Result<Spell, String> {
        let mut dicts = Vec::new();
        for (lang, aff, dic) in BUILT_IN {
            let (aff, dic) = (gunzip(aff)?, gunzip(dic)?);
            dicts.push((lang.to_string(), Dict::new(&aff, &dic)?));
        }
        Ok(Spell { dicts, personal: HashSet::new() })
    }

    /// Add a dictionary the caller read (one of the system's): `lang` is its code (`de`, from
    /// `de_DE`), `aff` and `dic` its files' text. One for a language already here is left out.
    pub fn add(&mut self, lang: &str, aff: &str, dic: &str) -> Result<bool, String> {
        let lang = lang.trim().to_ascii_lowercase();
        if lang.is_empty() || self.has(&lang) || self.dicts.len() >= MAX_DICTS {
            return Ok(false);
        }
        let d = Dict::new(aff, dic)?;
        self.dicts.push((lang, d));
        Ok(true)
    }

    /// The languages there are dictionaries for, in order (`fr`, `en`, then the system's).
    pub fn languages(&self) -> Vec<&str> {
        self.dicts.iter().map(|(l, _)| l.as_str()).collect()
    }

    pub fn has(&self, lang: &str) -> bool {
        self.dicts.iter().any(|(l, _)| l == lang)
    }

    /// The words this writer said are right (`a`, `b`), kept by the caller between messages.
    pub fn set_personal(&mut self, words: impl IntoIterator<Item = String>) {
        self.personal = words.into_iter().map(|w| w.to_lowercase()).collect();
    }

    /// Add a word to the personal list, and answer whether it was new.
    pub fn add_personal(&mut self, word: &str) -> bool {
        let w = word.trim().to_lowercase();
        !w.is_empty() && self.personal.insert(w)
    }

    pub fn personal(&self) -> Vec<String> {
        let mut v: Vec<String> = self.personal.iter().cloned().collect();
        v.sort();
        v
    }

    /// The dictionaries for `langs` (all of them when it's empty), in that order.
    fn dicts_for<'a>(&'a self, langs: &[&str]) -> Vec<&'a Dict> {
        if langs.is_empty() {
            return self.dicts.iter().map(|(_, d)| d).collect();
        }
        langs.iter().filter_map(|l| self.dicts.iter().find(|(x, _)| x == l).map(|(_, d)| d)).collect()
    }

    /// Whether a word is spelt right: known to one of the dictionaries, or added by the writer.
    pub fn check(&self, word: &str) -> bool {
        self.check_in(word, &[])
    }

    /// [`Spell::check`] against the dictionaries of `langs` (all when empty). A word in a
    /// script none of them covers is taken as right: there's nothing to check it with.
    pub fn check_in(&self, word: &str, langs: &[&str]) -> bool {
        let w = word.trim_matches(|c: char| !c.is_alphanumeric());
        if w.is_empty() {
            return true;
        }
        if self.personal.contains(&w.to_lowercase()) {
            return true;
        }
        let script = script_of(w);
        let dicts: Vec<&Dict> = self.dicts_for(langs).into_iter().filter(|d| d.script == script).collect();
        if dicts.is_empty() {
            return true;
        }
        if dicts.iter().any(|d| d.check(w)) {
            return true;
        }
        // A capitalised or shouted word is right when its plain form is.
        let lower = w.to_lowercase();
        lower != w && dicts.iter().any(|d| d.check(&lower))
    }

    /// What could be meant instead, the closest first, from every dictionary.
    pub fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        self.suggest_in(word, &[], max)
    }

    /// The suggestions taken from one dictionary, at most (it gives a dozen or two).
    const SUGGESTIONS_MAX: usize = 100;

    /// [`Spell::suggest`] from the dictionaries of `langs` (all when empty): the suggestions of
    /// each, merged and put in order of how close they are to the word (the first language
    /// first among those as close, then as its dictionary ranked them).
    pub fn suggest_in(&self, word: &str, langs: &[&str], max: usize) -> Vec<String> {
        let script = script_of(word);
        let lower = word.to_lowercase();
        // (distance, language rank, the dictionary's own rank, the word)
        let mut all: Vec<(usize, usize, usize, String)> = Vec::new();
        for (rank, d) in self.dicts_for(langs).into_iter().filter(|d| d.script == script).enumerate() {
            // Every suggestion the dictionary has, put in order here: those of the same score
            // come in no set order, so its first ten were sometimes without the closest word.
            for (i, s) in d.suggest(word, Self::SUGGESTIONS_MAX).into_iter().enumerate() {
                if all.iter().any(|(_, _, _, x)| *x == s) {
                    continue;
                }
                all.push((distance(&lower, &s.to_lowercase()), rank, i, s));
            }
        }
        all.sort_by_key(|a| (a.0, a.1, a.2));
        all.into_iter().map(|(_, _, _, s)| s).take(max).collect()
    }

    /// The words of `text` that are spelt wrong, with their byte ranges: what the composer
    /// underlines. Addresses, links and anything with a digit are left alone (a part number
    /// isn't a misspelt word), and so are shouted acronyms (`PSTVL`, `ERP`).
    pub fn misspelt<'a>(&self, text: &'a str) -> Vec<(Range<usize>, &'a str)> {
        self.misspelt_in(text, &[])
    }

    /// [`Spell::misspelt`] against the dictionaries of `langs` (all when empty).
    pub fn misspelt_in<'a>(&self, text: &'a str, langs: &[&str]) -> Vec<(Range<usize>, &'a str)> {
        words(text).into_iter().filter(|(_, w)| !self.check_in(w, langs)).collect()
    }
}

/// How far apart two words are: the edits (a letter put in, taken out, changed, or two swapped)
/// that turn one into the other. Capped for long words.
pub fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().take(64).collect();
    let b: Vec<char> = b.chars().take(64).collect();
    let (n, m) = (a.len(), b.len());
    // d[i][j]: the distance between a[..i] and b[..j].
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    if let Some(first) = d.first_mut() {
        for (j, cell) in first.iter_mut().enumerate() {
            *cell = j;
        }
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = best;
        }
    }
    d[n][m]
}

/// The words of a text, with their byte ranges: letters (any script), with the apostrophes and
/// hyphens that are inside a word (`aujourd'hui`, `rendez-vous`), skipping what isn't prose.
pub fn words(text: &str) -> Vec<(Range<usize>, &str)> {
    let mut out: Vec<(Range<usize>, &str)> = Vec::new();
    let mut start: Option<usize> = None;
    let mut end = 0usize;
    let bytes = text.as_bytes();
    for (i, c) in text.char_indices() {
        let word_char = c.is_alphabetic();
        // An apostrophe or a hyphen only belongs to a word between two letters.
        let joiner =
            matches!(c, '\'' | '\u{2019}' | '-') && start.is_some() && text[i + c.len_utf8()..].chars().next().is_some_and(char::is_alphabetic);
        if word_char || joiner {
            if start.is_none() {
                start = Some(i);
            }
            end = i + c.len_utf8();
        } else if let Some(s) = start.take() {
            if let Some(w) = text.get(s..end) {
                out.push((s..end, w));
            }
            end = s;
        }
    }
    if let Some(s) = start
        && let Some(w) = text.get(s..end)
    {
        out.push((s..end, w));
    }
    // What prose skips: anything with a digit or an address in it, and acronyms.
    out.retain(|(r, w)| {
        let around = |from: usize, to: usize| text.get(from..to).unwrap_or("");
        let before = around(r.start.saturating_sub(1), r.start);
        let after = around(r.end, (r.end + 1).min(bytes.len()));
        let glued = before.contains('@') || after.contains('@') || text.get(r.start..r.end + 1).is_some_and(|s| s.contains('/') || s.contains('@'));
        let digits = text.get(r.start.saturating_sub(1)..(r.end + 1).min(text.len())).is_some_and(|s| s.chars().any(|c| c.is_ascii_digit()));
        let shouted = w.chars().count() <= 6 && w.chars().filter(|c| c.is_alphabetic()).all(|c| c.is_uppercase());
        !(glued || digits || shouted)
    });
    out
}

fn gunzip(bytes: &[u8]) -> Result<String, String> {
    let mut out = String::new();
    GzDecoder::new(bytes).read_to_string(&mut out).map_err(|e| format!("a dictionary couldn't be read: {e}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spell() -> Spell {
        match Spell::built_in() {
            Ok(s) => s,
            Err(e) => panic!("{e}"),
        }
    }

    /// The dictionaries travel with the program and answer for both languages: a French text
    /// borrowing English technical words is right, and a made-up word isn't.
    #[test]
    fn french_and_english_both_count() {
        let s = spell();
        for w in ["bonjour", "courriel", "aujourd'hui", "rendez-vous", "logiciel"] {
            assert!(s.check(w), "{w} is French");
        }
        for w in ["email", "server", "firewall", "backup"] {
            assert!(s.check(w), "{w} is English, kept for technical words");
        }
        assert!(!s.check("bonjoure"));
        assert!(!s.check("zzzzzzzz"));
        // A word the writer added, whatever its language.
        let mut s = s;
        assert!(s.add_personal("Contoso"));
        assert!(s.check("contoso"), "added words are case-insensitive");
        assert!(!s.add_personal("contoso"), "added once");
        assert_eq!(s.languages(), ["fr", "en"]);
    }

    /// Suggestions come from the dictionaries, the closest word first: a typo's own word before
    /// others as likely, and the language asked for first among those as close.
    #[test]
    fn suggestions_are_offered_closest_first() {
        let s = spell();
        let v = s.suggest("bonjoure", 5);
        assert!(v.iter().any(|w| w == "bonjour"), "{v:?}");
        assert!(v.len() <= 5);
        assert_eq!(s.suggest_in("teh", &["en"], 5).first().map(String::as_str), Some("the"), "{:?}", s.suggest_in("teh", &["en"], 5));
        assert_eq!(s.suggest_in("recieve", &["en", "fr"], 5).first().map(String::as_str), Some("receive"));
        assert_eq!(s.suggest_in("interressant", &["fr", "en"], 5).first().map(String::as_str), Some("intéressant"));
        assert_eq!(s.suggest("interressant", 5).first().map(String::as_str), Some("intéressant"), "both languages, French first");
        assert_eq!((distance("teh", "the"), distance("recieve", "receive"), distance("abc", "abc"), distance("", "ab")), (1, 1, 0, 2));
    }

    /// What gets underlined: words, not addresses, part numbers or acronyms.
    #[test]
    fn words_skip_what_is_not_prose() {
        let w: Vec<&str> = words("Bonjour, écrivez à zoe@studio.example").into_iter().map(|(_, w)| w).collect();
        assert!(w.contains(&"Bonjour") && w.contains(&"écrivez"), "{w:?}");
        assert!(!w.contains(&"zoe") && !w.contains(&"studio"), "an address isn't prose: {w:?}");
    }

    #[test]
    fn text_is_skipped_where_it_is_not_prose() {
        let w: Vec<&str> = words("RTX4090 et PSTVL, mais bonjour").into_iter().map(|(_, w)| w).collect();
        assert_eq!(w, ["et", "mais", "bonjour"], "a part number and an acronym are left alone");
        let s = spell();
        let bad = s.misspelt("Le serveur est en panne, quatrelle aussi");
        assert!(bad.iter().any(|(_, w)| *w == "quatrelle"), "{bad:?}");
        assert!(!bad.iter().any(|(_, w)| *w == "serveur"), "{bad:?}");
    }

    /// A likely correction isn't always the first one: the menu lists ten (as Outlook does), so a
    /// "vai" typed for "vais" is among them.
    #[test]
    fn a_likely_word_is_among_the_first_ten() {
        let s = spell();
        let first: Vec<String> = s.suggest("vai", 10);
        assert!(first.iter().any(|w| w == "vais"), "{first:?}");
    }

    /// A check against one language alone: an English word is wrong in French, and a word in
    /// a script no dictionary covers (Chinese, Arabic) is left alone rather than marked.
    #[test]
    fn one_language_at_a_time_and_scripts_without_a_dictionary() {
        let s = spell();
        assert!(s.check_in("server", &["en"]) && !s.check_in("server", &["fr"]) && s.check_in("server", &[]));
        assert!(s.check_in("serveur", &["fr"]) && !s.check_in("serveur", &["en"]));
        let bad: Vec<&str> = s.misspelt_in("Le serveur 服务器 والخادم сервер est là", &["fr"]).into_iter().map(|(_, w)| w).collect();
        assert!(bad.is_empty(), "nothing to check them with: {bad:?}");
        assert_eq!(
            (script_of("服务器"), script_of("сервер"), script_of("Été"), script_of("والخادم")),
            (Script::Cjk, Script::Cyrillic, Script::Latin, Script::Arabic)
        );
        assert!(s.suggest("服务器", 5).is_empty());
        // A dictionary added from the system: its language is offered, and used.
        let mut s = s;
        let aff = "SET UTF-8\nTRY esianrtolcdugmphbyfvkwzESIANRTOLCDUGMPHBYFVKWZ\n";
        let dic = "3\nHaus\nBaum\nStraße\n";
        assert_eq!(s.add("de", aff, dic), Ok(true));
        assert_eq!(s.add("de", aff, dic), Ok(false), "once");
        assert!(s.check_in("Straße", &["de"]) && !s.check_in("Strasse", &["de"]));
        assert_eq!(s.languages(), ["fr", "en", "de"]);
        assert_eq!(dic_script("1\nсервер\n"), Script::Cyrillic);
    }
}
