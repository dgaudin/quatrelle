//! Spelling for the composer: Hunspell dictionaries, read natively (no C library, no system
//! dictionary to find), French first and English beside it for technical terms.
//!
//! The dictionaries are the two `assets/dictionaries/` files, gunzipped and parsed once; a word
//! MailCraft doesn't know can be added by the writer (a personal list, kept by the caller).
//! Pure Rust, no I/O of its own: [`Dict::new`] takes the two files' text.
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

/// One Hunspell dictionary.
pub struct Dict {
    inner: spellbook::Dictionary,
}

impl Dict {
    /// A dictionary from its `.aff` and `.dic` text.
    pub fn new(aff: &str, dic: &str) -> Result<Dict, String> {
        spellbook::Dictionary::new(aff, dic).map(|inner| Dict { inner }).map_err(|e| e.to_string())
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
}

/// The dictionaries MailCraft carries: French, then English (technical terms a French text
/// borrows), and the words the writer added.
pub struct Spell {
    dicts: Vec<(&'static str, Dict)>,
    personal: HashSet<String>,
}

impl Spell {
    /// The built-in dictionaries. `Err` when one of them can't be read (never, in practice: they
    /// travel with the program), in which case spelling is simply not offered.
    pub fn built_in() -> Result<Spell, String> {
        let mut dicts = Vec::new();
        for (lang, aff, dic) in BUILT_IN {
            let (aff, dic) = (gunzip(aff)?, gunzip(dic)?);
            dicts.push((*lang, Dict::new(&aff, &dic)?));
        }
        Ok(Spell { dicts, personal: HashSet::new() })
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

    /// Whether a word is spelt right: known to one of the dictionaries, or added by the writer.
    pub fn check(&self, word: &str) -> bool {
        let w = word.trim_matches(|c: char| !c.is_alphanumeric());
        if w.is_empty() {
            return true;
        }
        if self.personal.contains(&w.to_lowercase()) {
            return true;
        }
        if self.dicts.iter().any(|(_, d)| d.check(w)) {
            return true;
        }
        // A capitalised or shouted word is right when its plain form is.
        let lower = w.to_lowercase();
        lower != w && self.dicts.iter().any(|(_, d)| d.check(&lower))
    }

    /// What could be meant instead, French suggestions first, then English ones.
    pub fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        let mut out = Vec::new();
        for (_, d) in &self.dicts {
            for s in d.suggest(word, max) {
                if !out.contains(&s) {
                    out.push(s);
                }
            }
            if out.len() >= max {
                break;
            }
        }
        out.truncate(max);
        out
    }

    /// The words of `text` that are spelt wrong, with their byte ranges: what the composer
    /// underlines. Addresses, links and anything with a digit are left alone (a part number
    /// isn't a misspelt word), and so are shouted acronyms (`PSTVL`, `ERP`).
    pub fn misspelt<'a>(&self, text: &'a str) -> Vec<(Range<usize>, &'a str)> {
        words(text).into_iter().filter(|(_, w)| !self.check(w)).collect()
    }
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
    }

    /// Suggestions come from the dictionaries, French first.
    #[test]
    fn suggestions_are_offered() {
        let s = spell();
        let v = s.suggest("bonjoure", 5);
        assert!(v.iter().any(|w| w == "bonjour"), "{v:?}");
        assert!(v.len() <= 5);
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
        let bad = s.misspelt("Le serveur est en panne, mailcraft aussi");
        assert!(bad.iter().any(|(_, w)| *w == "mailcraft"), "{bad:?}");
        assert!(!bad.iter().any(|(_, w)| *w == "serveur"), "{bad:?}");
    }
}
