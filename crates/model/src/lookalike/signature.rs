// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The signature read against the sender. CEO fraud copies a real signature (the real address,
//! the real website) under a mail from somewhere else: the name and the signature look right to
//! anyone who doesn't read the address the mail came from.
//!
//! [`own_text`] is the author's own part of a message (up to the first quotation mark: an
//! "Original Message" line, Outlook's header block, a "wrote:" line, a quoted line…);
//! [`signature_shows`] reads the addresses and websites of its signature;
//! [`SenderCheck::check_signature`] warns when one of those belongs to the organisation, to
//! someone known, or carries the sender's name, while the mail comes from elsewhere. The text is
//! the sender's: every step is bounded.

use std::collections::{BTreeSet, VecDeque};

use crate::EmailAddress;
use crate::addr::trust::url_host;

use super::{SenderCheck, SenderWarning, clean_domain, domain_of, is_external, is_free_mail, name_key, related};

/// The most of a message's text read for its author's part, in bytes.
const READ_MOST: usize = 64 * 1024;
/// Without a `-- ` line, the signature is the author's last lines that aren't empty, this many.
const SIGNATURE_LINES: usize = 15;
/// The most addresses and sites read in a signature.
const MOST_SHOWN: usize = 10;
/// A signature's line has at most this many words besides its addresses and sites.
const MOST_OTHER_WORDS: usize = 4;
/// A word longer than this (in bytes) is no address or website anyone would read.
const LONGEST_WORD: usize = 2048;
/// A header block's second line comes within this many lines of its first.
const HEADER_REACH: usize = 4;

/// What a signature gives: an address, or a website by its domain (lower case, `www.` left out).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    Address(String),
    Site(String),
}

/// The start of `text`, at most [`READ_MOST`] bytes, cut between two characters.
fn head(text: &str) -> &str {
    let mut end = READ_MOST.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default()
}

/// Whether `line` (lower case, trimmed) starts with one of `labels` and then a colon, with a
/// space, a no-break space or nothing before it (`de :`, `de\u{a0}:`, `de:`).
fn labelled(line: &str, labels: &[&str]) -> bool {
    labels.iter().any(|l| line.strip_prefix(l).is_some_and(|rest| rest.trim_start_matches([' ', '\u{a0}', '\u{202f}', '\t']).starts_with(':')))
}

/// The first line of a header block.
const FROM: &[&str] = &["from", "de", "von", "da", "van"];
/// The most words of a header block's other labels (`Enviado el`, `Enviada em`, `Sent`).
const LABEL_WORDS: usize = 3;
/// The end of a line that introduces what someone else wrote.
const WROTE: &[&str] = &["wrote", "a écrit", "schrieb", "escribió", "ha scritto", "schreef"];
/// How such a line starts when a mail program cut it in two.
const ON: &[&str] = &["on ", "le ", "am ", "el ", "il ", "op "];

/// A line (lower case, trimmed) that starts with a short label: one to [`LABEL_WORDS`] words of
/// letters, then a colon after a space, a no-break space or nothing (`sent:`, `enviado el:`,
/// `à :`, `para\u{a0}:`); not a link's `https://`. What a header block's lines after its first
/// start with, in every language Outlook writes.
fn short_label(line: &str) -> bool {
    let Some((label, after)) = line.split_once(':') else { return false };
    let label = label.trim_end_matches([' ', '\u{a0}', '\u{202f}', '\t']);
    let mut words = 0usize;
    for word in label.split([' ', '\u{a0}', '\u{202f}', '\t']) {
        if word.is_empty() || !word.chars().all(char::is_alphabetic) {
            return false;
        }
        words += 1;
        if words > LABEL_WORDS {
            return false;
        }
    }
    words > 0 && !after.starts_with("//")
}

/// Gmail's German or Dutch attribution line (`Am … schrieb Ana <`, `Op … schreef`), cut in two
/// before the end that closes it with a colon.
fn wrapped_attribution(line: &str, next: Option<&str>) -> bool {
    ["am ", "op "].iter().any(|o| line.starts_with(o))
        && [(" schrieb", " schrieb "), (" schreef", " schreef ")].iter().any(|(end, inside)| line.ends_with(end) || line.contains(inside))
        && next.is_some_and(|n| n.ends_with(':'))
}

/// A line (lower case, trimmed) that by itself starts what isn't the author's: a row of dashes
/// around words, Outlook's row of underscores, a "wrote:" line, Apple Mail's forwarding line, a
/// quoted line.
fn mark(line: &str) -> bool {
    let dashes = line.len() >= 6 && line.starts_with("---") && line.ends_with("---") && line.trim_matches('-').chars().any(char::is_alphanumeric);
    let underscores = line.len() >= 10 && line.chars().all(|c| c == '_');
    dashes || underscores || wrote(line) || labelled(line, &["begin forwarded message", "début du message réexpédié"]) || line.starts_with('>')
}

/// A line (lower case, trimmed) ending `wrote:` or the like, or one ending `:` with `schrieb` or
/// `schreef` inside.
fn wrote(line: &str) -> bool {
    let Some(rest) = line.strip_suffix(':') else { return false };
    // Gmail in German and Dutch: "Am …, um … Uhr schrieb Ana <ana@…>:", "Op … schreef Ana <…>:".
    if [" schrieb ", " schreef "].iter().any(|w| rest.contains(w)) {
        return true;
    }
    let rest = rest.trim_end_matches([' ', '\u{a0}', '\u{202f}', '\t']);
    WROTE.iter().any(|w| rest.strip_suffix(w).is_some_and(|before| before.is_empty() || before.ends_with(char::is_whitespace)))
}

/// A line (trimmed) without the stars an HTML message's converted text writes around bold and
/// italics (`**De :**`, `*… wrote:*`). A list's bullet (`* item`, a star then a space) stays: an
/// item isn't emphasis.
fn unemphasised(line: &str) -> &str {
    let start = if line.starts_with("**") {
        line.get(2..)
    } else if line.starts_with('*') && !line.get(1..).is_some_and(|r| r.starts_with(char::is_whitespace)) {
        line.get(1..)
    } else {
        Some(line)
    };
    start.unwrap_or(line).trim_end_matches('*').trim()
}

/// The author's own part of a message's text: up to the first line that starts a quoted or
/// forwarded message, at most [`READ_MOST`] bytes.
pub fn own_text(text: &str) -> &str {
    let text = head(text);
    // Each line's start and its lower-case, trimmed form, for the header block's look ahead,
    // bold and italics' stars left aside (Outlook's `**De :**`).
    let mut lines: Vec<(usize, String)> = Vec::new();
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        lines.push((at, unemphasised(line.trim()).to_lowercase()));
        at += line.len();
    }
    for (i, (start, line)) in lines.iter().enumerate() {
        if mark(line) {
            // "On Mon, … Ana <ana@…>" / "wrote:" cut in two: the quote starts on the line before.
            let cut_before = wrote(line) && !ON.iter().any(|o| line.starts_with(o));
            let before = i.checked_sub(1).and_then(|p| lines.get(p)).filter(|(_, l)| cut_before && ON.iter().any(|o| l.starts_with(o)));
            return text.get(..before.map_or(*start, |(s, _)| *s)).unwrap_or_default();
        }
        // Gmail's German or Dutch line, cut in two before its address's end.
        if wrapped_attribution(line, lines.get(i + 1).map(|(_, l)| l.as_str())) {
            return text.get(..*start).unwrap_or_default();
        }
        // A header block: a From line, then within a few lines one with a short label (`Sent:`,
        // `Enviado el:`, `Para:`), whatever the language.
        if labelled(line, FROM) {
            let block = lines.iter().skip(i + 1).take(HEADER_REACH).take_while(|(_, l)| !mark(l)).any(|(_, l)| short_label(l));
            if block {
                return text.get(..*start).unwrap_or_default();
            }
        }
    }
    text
}

/// The number of a link's note (`[1]: https://…`), the line of an HTML message's converted text
/// that says where a link goes: the notes are put at the end of the text, whichever part the
/// links are in.
fn link_note(line: &str) -> Option<&str> {
    let (n, after) = line.trim().strip_prefix('[')?.split_once(']')?;
    (!n.is_empty() && n.len() <= 6 && n.bytes().all(|b| b.is_ascii_digit()) && after.starts_with(':')).then_some(n)
}

/// The numbers of the links' notes a line refers to (`[Dana][1]`).
fn link_refs(line: &str) -> impl Iterator<Item = &str> {
    line.match_indices("][").filter_map(move |(at, _)| {
        let (n, _) = line.get(at + 2..)?.split_once(']')?;
        (!n.is_empty() && n.len() <= 6 && n.bytes().all(|b| b.is_ascii_digit())).then_some(n)
    })
}

/// The signature of the author's own text: what follows its last `-- ` line, or else its last
/// [`SIGNATURE_LINES`] lines that aren't empty (links' notes left aside).
fn signature(own: &str) -> &str {
    let mut after_dashes = None;
    let mut last: VecDeque<usize> = VecDeque::with_capacity(SIGNATURE_LINES + 1);
    let mut at = 0;
    for line in own.split_inclusive('\n') {
        let next = at + line.len();
        let bare = line.trim_end_matches(['\n', '\r']);
        if bare.trim_end() == "--" {
            after_dashes = Some(next);
        } else if !bare.trim().is_empty() && link_note(bare).is_none() {
            last.push_back(at);
            if last.len() > SIGNATURE_LINES {
                last.pop_front();
            }
        }
        at = next;
    }
    let start = after_dashes.or_else(|| last.front().copied()).unwrap_or(own.len());
    own.get(start..).unwrap_or_default()
}

/// The parts of a signature's line between spaced separators (`Dana – CEO – www.…`, `a | b`):
/// a signature written on one line, as phones send it.
fn parts(line: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(line);
    std::iter::from_fn(move || {
        let r = rest?;
        let mut spaced = false;
        for (at, c) in r.char_indices() {
            if spaced && matches!(c, '|' | '–' | '—' | '•' | '·') {
                let after = r.get(at + c.len_utf8()..);
                if after.is_some_and(|a| a.starts_with(char::is_whitespace)) {
                    rest = after;
                    return r.get(..at);
                }
            }
            spaced = c.is_whitespace();
        }
        rest = None;
        Some(r)
    })
}

/// The addresses and websites the signature of `text` gives, in order, once each, at most
/// [`MOST_SHOWN`]. Only a signature's kind of line counts: one where they come with at most
/// [`MOST_OTHER_WORDS`] other words (a name, a title, a label such as `Tél.` or `Web`), so a
/// sentence naming a website or an address ("I can't log in to www.…") isn't read; a line in
/// parts between spaced separators (`Dana Whitfield – CEO – www.…`) counts the words of each part
/// apart. A link's note (where an HTML link goes, at the end of the text) is read only for a link
/// on such a line: not for one in a sentence of the message.
pub fn signature_shows(text: &str) -> Vec<Shown> {
    let mut out: Vec<Shown> = Vec::new();
    let separator = |c: char| c.is_whitespace() || matches!(c, '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | '"' | ',' | ';' | '|' | '«' | '»');
    // The notes of the links on the signature's lines (they come after those lines).
    let mut linked: BTreeSet<&str> = BTreeSet::new();
    for line in signature(own_text(text)).split('\n') {
        if link_note(line).is_some_and(|n| !linked.contains(n)) {
            continue;
        }
        for part in parts(line) {
            if out.len() >= MOST_SHOWN {
                return out;
            }
            let mut found: Vec<Shown> = Vec::new();
            let mut others = 0usize;
            for word in part.split(separator) {
                let word = word.trim_matches(|c: char| matches!(c, '.' | ':' | '!' | '?' | '\'' | '\u{2019}' | '*' | '`'));
                if word.is_empty() || word.len() > LONGEST_WORD {
                    continue;
                }
                match read_word(word) {
                    Some(shown) if found.len() < MOST_SHOWN => found.push(shown),
                    Some(_) => {}
                    // Separators (`–`, `•`, `/`) and a phone number's digits aren't words.
                    None if word.chars().any(char::is_alphabetic) => {
                        others += 1;
                        if others > MOST_OTHER_WORDS {
                            break;
                        }
                    }
                    None => {}
                }
            }
            if others > MOST_OTHER_WORDS {
                continue;
            }
            if linked.len() < MOST_SHOWN {
                linked.extend(link_refs(part).take(MOST_SHOWN));
            }
            for shown in found {
                if out.len() < MOST_SHOWN && !out.contains(&shown) {
                    out.push(shown);
                }
            }
        }
    }
    out
}

/// One word of a signature as an address (`dana@corp.example`, `mailto:…`, `E-mail:…`) or a
/// website (`www.…`, `http(s)://…`).
fn read_word(word: &str) -> Option<Shown> {
    let lower = word.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.") {
        let host = url_host(&lower)?;
        let host = host.strip_prefix("www.").unwrap_or(&host);
        return clean_domain(host).map(Shown::Site);
    }
    if !lower.contains('@') {
        return None;
    }
    // `mailto:` or a label before the address; a question after it (`?subject=…`).
    let address = lower.rsplit(':').next().unwrap_or_default();
    let address = address.split('?').next().unwrap_or_default();
    let (user, domain) = address.split_once('@')?;
    let ok = !user.is_empty()
        && user.chars().all(|c| c.is_alphanumeric() || "._%+-'!#$&*=^`{|}~/".contains(c))
        && crate::addr::is_valid_email(address)
        && clean_domain(domain).as_deref() == Some(domain);
    ok.then(|| Shown::Address(address.to_string()))
}

impl SenderCheck {
    /// What the message's own signature gives against where it came from (`from`): the first
    /// address or website of the signature of `text` that is
    ///
    /// - **a.** at the organisation `org` (its domains and those under them), the mail coming
    ///   from outside it;
    /// - **b.** an address known exactly (a contact, someone written to, the directory), the
    ///   mail coming from another;
    /// - **c.** an address carrying the sender's name, the mail coming from a mail service.
    ///
    /// Mail services' addresses and sites, those related to the sender's domain and the user's
    /// own addresses never count. Nothing for the user's own mail, a known sender, or `listed`
    /// mail (a list, bulk) unless it comes from a mail service.
    pub fn check_signature(&self, from: &EmailAddress, text: &str, listed: bool, org: &BTreeSet<String>) -> Option<SenderWarning> {
        let real = from.email.trim().to_lowercase();
        let domain = domain_of(&real);
        // The user's own mail, or a known sender's (`knows` counts the user's own addresses).
        if domain.is_empty() || !crate::addr::is_valid_email(&real) || self.knows(&real) {
            return None;
        }
        let free = is_free_mail(&domain);
        if listed && !free {
            return None;
        }
        let outside = is_external(&real, org);
        let senders_name = if free { name_key(&from.name) } else { None };
        signature_shows(text).into_iter().find_map(|shown| {
            let (what, d) = match &shown {
                Shown::Address(a) => (a, domain_of(a)),
                Shown::Site(s) => (s, s.clone()),
            };
            if d.is_empty() || is_free_mail(&d) || related(&d, &domain) || self.is_own(what) {
                return None;
            }
            let at_org = outside && org.iter().any(|o| d == *o || d.strip_suffix(o.as_str()).is_some_and(|r| r.ends_with('.')));
            let known = matches!(shown, Shown::Address(_)) && self.people().knows_address(what);
            let named = matches!(shown, Shown::Address(_))
                && senders_name.is_some()
                && what.split_once('@').is_some_and(|(user, _)| name_key(&user.replace(['.', '_', '-'], " ")) == senders_name);
            (at_org || known || named).then(|| SenderWarning::SignatureShowsOther { shown: what.clone(), real: real.clone() })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::time::{Duration, Instant};

    use crate::EmailAddress;
    use crate::lookalike::{KnownPeople, SenderCheck, SenderWarning, lookup_name};

    fn org() -> BTreeSet<String> {
        BTreeSet::from(["studio.example".to_string()])
    }

    /// The user is Ana, at studio.example; she knows `people` too.
    fn knowing(people: &[(&str, &str)]) -> SenderCheck {
        let mut k = KnownPeople::new();
        k.add(&EmailAddress::new("Ana Lopez", "ana@studio.example"));
        for (name, email) in people {
            k.add(&EmailAddress::new(*name, *email));
        }
        SenderCheck::new(org(), k, ["ana@studio.example".to_string()])
    }

    fn warns(c: &SenderCheck, name: &str, email: &str, text: &str, listed: bool) -> Option<SenderWarning> {
        c.check_signature(&EmailAddress::new(name, email), text, listed, &org())
    }

    fn shown(shown: &str, real: &str) -> Option<SenderWarning> {
        Some(SenderWarning::SignatureShowsOther { shown: shown.into(), real: real.into() })
    }

    #[test]
    fn lookup_name_as_a_person_types_it() {
        for name in [
            "Dr. Dana Whitfield, CEO",
            "\u{FF24}\u{FF41}\u{FF4E}\u{FF41} Whitfield",
            "\u{1D403}\u{1D41A}\u{1D427}\u{1D41A} Whitfield",
            "D\u{0430}na Whitfield",
            "Dana\u{200B} Whitfield",
            "Dana J. Whitfield",
            "\u{202E}Dana Whitfield\u{202C}",
        ] {
            assert_eq!(lookup_name(name).as_deref(), Some("Dana Whitfield"), "{name:?}");
        }
        assert_eq!(lookup_name("Z0e Park").as_deref(), Some("Zoe Park"));
        assert_eq!(lookup_name("Wh1tfield Dana").as_deref(), Some("Whitfield Dana"));
        assert_eq!(lookup_name("Jérôme Dupré").as_deref(), Some("Jérôme Dupré"));
        assert_eq!(lookup_name("Jose\u{301} Garci\u{301}a").as_deref(), Some("Jose\u{301} Garci\u{301}a"));
        assert_eq!(lookup_name("Whitfield, Dana").as_deref(), Some("Whitfield Dana"));
        assert_eq!(lookup_name("Anne-Marie O'Brien").as_deref(), Some("Anne-Marie O'Brien"));
        // A name written in another script is that script's name, not a disguise.
        assert_eq!(lookup_name("Иван Петров").as_deref(), Some("Иван Петров"));
        for name in ["IT Support", "Dana", "", "Dana Whitfield 2", "dana@corp.example"] {
            assert_eq!(lookup_name(name), None, "{name:?}");
        }
        let huge = format!("{} {}", "a".repeat(150), "b".repeat(150));
        assert!(lookup_name(&huge).map_or(0, |n| n.chars().count()) <= 100);
        let long = format!("{} {}", "a".repeat(55), "b".repeat(55));
        assert!(lookup_name(&long).is_some_and(|n| n.chars().count() <= 100));
    }

    #[test]
    fn own_text_stops_at_each_quotation_mark() {
        let underscores = "_".repeat(32);
        for mark in [
            "-----Original Message-----",
            "-----Message d'origine-----",
            "---------- Forwarded message ---------",
            underscores.as_str(),
            "From: Ana <a@b.example>\nSent: Monday",
            "De : Ana\nEnvoyé : lundi",
            "De\u{a0}: Ana\nEnvoyé\u{a0}: lundi",
            "Von: Ana\nGesendet: Montag",
            "From: Ana\n\nTo: Bo\nDate: Monday",
            "On Mon, 3 Nov 2026, Ana <a@b.example> wrote:",
            "Le lun. 3 nov. 2026 à 10:00, Ana <a@b.example> a écrit :",
            "Ana <a@b.example> schrieb:",
            "Am Mo., 3. Nov. 2026 um 10:00 Uhr schrieb Ana <a@b.example>:",
            "Op ma 3 nov. 2026 om 10:00 schreef Ana <a@b.example>:",
            "> quoted",
            "Begin forwarded message:",
            "Début du message réexpédié :",
        ] {
            let text = format!("Hello\n\nDana\n{mark}\nquoted dana@x.example");
            let own = own_text(&text);
            assert!(!own.contains("quoted"), "{mark:?}: {own:?}");
            assert!(own.contains("Dana"), "{mark:?}: {own:?}");
        }
        // Gmail's line cut in two: the address before `wrote:` is the quoted one's.
        let text = "Thanks\n\nDana\nOn Mon, 3 Nov 2026 at 10:00, Ana <a@b.example>\nwrote:\nquoted";
        assert!(!own_text(text).contains("a@b.example"));
        assert!(own_text(text).contains("Dana"));
    }

    /// An HTML message's converted text writes Outlook's bold labels with stars (`**De :**`), and
    /// a "wrote:" line in italics with one.
    #[test]
    fn own_text_reads_marks_through_bold_and_italics() {
        for mark in [
            "**De :** Zoe Park <zoe@studio.example>\n**Envoyé :** lundi 3 novembre 2026 10:00",
            "**From:** Zoe Park\n**Sent:** Monday",
            "*On Mon, 3 Nov 2026, Zoe Park <zoe@studio.example> wrote:*",
        ] {
            let text = format!("Hello\n\nDana\n{mark}\nquoted dana@x.example");
            let own = own_text(&text);
            assert!(!own.contains("quoted"), "{mark:?}: {own:?}");
            assert!(own.contains("Dana"), "{mark:?}: {own:?}");
        }
        // A list's bullet (`* ` before each `<li>`, or typed) isn't emphasis: its items start no
        // header block.
        for text in ["Hi\n\n* From: Paris\n* To: Lyon\n\nDana\nwww.x.example", "Bonjour\n\n* De : 9 h\n* À : 17 h\n\nDana\nwww.x.example"] {
            assert_eq!(own_text(text), text);
        }
    }

    /// An HTML message's links are noted at the end of its converted text (`[1]: https://…`):
    /// a link in a sentence of the message isn't the signature's, a link on its lines is.
    #[test]
    fn links_notes_count_for_the_signatures_links_only() {
        let text = "Hello Zoe, the new prices are on [your portal][1].\n\nBo Chen\nSupplier Ltd\n\n[1]: https://portal.studio.example/prices";
        assert_eq!(signature_shows(text), Vec::<Shown>::new());
        let text =
            "Please wire it today, see [the details][1].\n\n[Dana Whitfield][2]\nCEO\n\n[1]: https://pay.example/x\n[2]: mailto:dana@studio.example";
        assert_eq!(signature_shows(text), vec![Shown::Address("dana@studio.example".into())]);
        // Many links in the message don't push the signature out of its last lines.
        let notes: String = (1..=40).map(|i| format!("\n[{i}]: https://n{i}.example")).collect();
        let text = format!("Hi\n\nDana\nwww.studio.example\n{notes}");
        assert_eq!(signature_shows(&text), vec![Shown::Site("studio.example".into())]);
    }

    /// Outlook's header block in Spanish, Portuguese, Italian, Dutch and German: whatever its
    /// labels (`Enviado el:`, `Para:`, `Assunto:`…), a short label after the From line makes it a
    /// block (Ruling 18). An outsider's reply quoting the user's own signature says nothing.
    #[test]
    fn header_blocks_in_other_languages() {
        let mut k = KnownPeople::new();
        k.add(&EmailAddress::new("Zoe Park", "zoe@studio.example"));
        let zoe = SenderCheck::new(org(), k, ["zoe@studio.example".to_string()]);
        let mine = "Hola Bo\n\nZoe Park\nwww.studio.example\nzoe@studio.example";
        for header in [
            "De: Zoe Park <zoe@studio.example>\nEnviado el: lunes, 3 de noviembre de 2026 10:00\nPara: Bo Chen\nAsunto: Precios",
            "De: Zoe Park <zoe@studio.example>\nEnviada em: segunda-feira, 3 de novembro de 2026 10:00\nPara: Bo Chen\nAssunto: Preços",
            "Da: Zoe Park <zoe@studio.example>\nInviato: lunedì 3 novembre 2026 10:00\nA: Bo Chen\nOggetto: Prezzi",
            "Van: Zoe Park <zoe@studio.example>\nVerzonden: maandag 3 november 2026 10:00\nAan: Bo Chen\nOnderwerp: Prijzen",
            "Von: Zoe Park <zoe@studio.example>\nGesendet: Montag, 3. November 2026 10:00\nAn: Bo Chen\nBetreff: Preise",
            "**De:** Zoe Park <zoe@studio.example>\n**Enviado el:** lunes\n**Para:** Bo Chen",
            "De: Zoe Park\n\u{a0}\nPara\u{a0}: Bo Chen",
        ] {
            let text = format!("Gracias.\n\nBo\n{header}\n\n{mine}");
            assert!(!own_text(&text).contains("Hola"), "{header:?}");
            assert_eq!(zoe.check_signature(&EmailAddress::new("Bo Chen", "bo@supplier.example"), &text, false, &org()), None, "{header:?}");
        }
        // Gmail's German and Dutch lines, wrapped over two.
        for attribution in [
            "Am Mo., 3. Nov. 2026 um 10:00 Uhr schrieb Zoe Park <\nzoe@studio.example>:",
            "Am Mo., 3. Nov. 2026 um 10:00 Uhr schrieb\nZoe Park <zoe@studio.example>:",
            "Op ma 3 nov. 2026 om 10:00 schreef Zoe Park <\nzoe@studio.example>:",
        ] {
            let text = format!("Danke.\n\nBo\n{attribution}\n\n{mine}");
            assert_eq!(own_text(&text), "Danke.\n\nBo\n", "{attribution:?}");
            assert_eq!(zoe.check_signature(&EmailAddress::new("Bo Chen", "bo@supplier.example"), &text, false, &org()), None, "{attribution:?}");
        }
        // A link on the line after a From line isn't a label.
        let text = "From: our team\nhttps://www.studio.example\n\nDana";
        assert_eq!(own_text(text), text);
        // A label too long, or not of letters, isn't one either.
        for text in ["From: Paris\nthe four of us: fine", "From: Paris\n10h30: train", "From: Paris\n* To: Lyon"] {
            assert_eq!(own_text(text), text);
        }
    }

    #[test]
    fn a_from_line_alone_is_no_quote() {
        let text = "From: our team, with love\n\nDana\nwww.studio.example";
        assert_eq!(own_text(text), text);
        // A second header line too far below isn't a header block either.
        let text = "From: our team\n\n\n\n\nSent: by hand";
        assert_eq!(own_text(text), text);
        // Nor a signature delimiter, a lone row of dashes, or a short row of underscores.
        let text = "Hi\n-- \nDana\n------\n_____";
        assert_eq!(own_text(text), text);
    }

    #[test]
    fn the_signature_is_after_dashes_or_the_last_lines() {
        assert_eq!(signature_shows("Hi\n-- \nDana\ndana@corp.example"), vec![Shown::Address("dana@corp.example".into())]);
        assert_eq!(signature_shows("Write to x@one.example\n-- \nDana\ndana@corp.example"), vec![Shown::Address("dana@corp.example".into())]);
        let mut lines: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
        lines[1] = "x@one.example".into();
        lines[28] = "y@two.example".into();
        assert_eq!(signature_shows(&lines.join("\n")), vec![Shown::Address("y@two.example".into())]);
        // Empty lines don't count among the fifteen.
        let text = format!("x@one.example\n{}", "\n\nline".repeat(14));
        assert_eq!(signature_shows(&text), vec![Shown::Address("x@one.example".into())]);
    }

    #[test]
    fn addresses_and_sites_are_read() {
        let text = "Dana\nwww.Studio.example | https://studio.example/about\n[Mail me][1]\n\n[1]: mailto:Dana@Corp.example?subject=Hi";
        assert_eq!(signature_shows(text), vec![Shown::Site("studio.example".into()), Shown::Address("dana@corp.example".into())]);
        assert_eq!(signature_shows("Dana Whitfield <Dana@Corp.example>, (dana@corp.example)."), vec![Shown::Address("dana@corp.example".into())]);
        assert_eq!(signature_shows("E-mail:dana@corp.example"), vec![Shown::Address("dana@corp.example".into())]);
        assert_eq!(signature_shows("http://user@www.corp.example:8080/x"), vec![Shown::Site("corp.example".into())]);
        // Not addresses or sites: no domain, junk around it, a bare word.
        assert_eq!(signature_shows("dana@localhost\nwww.\nhttps://\nstudio.example\n@\nx@y@z.example\nwww.a_b!.example"), vec![]);
        let many: Vec<String> = (0..25).map(|i| format!("p{i}@corp{i}.example")).collect();
        assert_eq!(signature_shows(&many.join(" ")).len(), 10);
    }

    #[test]
    fn a_sentence_naming_the_organisation_is_no_signature() {
        let c = knowing(&[("Dana Whitfield", "dana@corp.example")]);
        for text in [
            "Bonjour,\n\nJe n'arrive pas à me connecter sur www.studio.example depuis hier.",
            "Bonjour,\n\nj'ai écrit à support@studio.example sans réponse",
            "I can't log in to www.studio.example",
        ] {
            assert_eq!(warns(&c, "Marie Client", "marie.client@gmail.com", text, false), None, "{text:?}");
        }
        // A signature's line: a name, a title, separators, the site.
        let text = "Cordialement,\n\nDana Whitfield | CEO | Studio | www.studio.example";
        assert_eq!(warns(&c, "Marie Client", "marie.client@gmail.com", text, false), shown("studio.example", "marie.client@gmail.com"));
        // A label is a word, separators and a phone number's digits aren't.
        let text = "Merci.\n\nEmail : dana@corp.example";
        assert_eq!(warns(&c, "Marie Client", "marie.client@gmail.com", text, false), shown("dana@corp.example", "marie.client@gmail.com"));
        let text = "Merci.\n\nTél. : +33 1 23 45 67 89 – Web : www.studio.example";
        assert_eq!(signature_shows(text), vec![Shown::Site("studio.example".into())]);
        // A sentence of more words, even with a separator, isn't (Ruling 17: only the part
        // holding the site counts its words).
        assert_eq!(signature_shows("I can't log in to www.studio.example | since Monday"), vec![]);
    }

    /// A signature on one line, its parts between spaced separators (Ruling 17): only the part
    /// holding the address or site counts its words.
    #[test]
    fn a_one_line_signature_is_read_by_its_parts() {
        let c = knowing(&[]);
        let text = "Merci.\n\nDana Whitfield – Directrice Générale – Studio SAS – www.studio.example";
        assert_eq!(warns(&c, "Dana Whitfield", "dg.studio@gmail.com", text, false), shown("studio.example", "dg.studio@gmail.com"));
        assert_eq!(signature_shows("Dana Whitfield, CEO, Studio, Paris, France • www.studio.example"), vec![Shown::Site("studio.example".into())]);
        assert_eq!(signature_shows("Dana Whitfield — Head of Finance · dana@corp.example"), vec![Shown::Address("dana@corp.example".into())]);
        // A separator stuck to a word doesn't part the line.
        assert_eq!(signature_shows("Dana Whitfield, CEO, Studio, Paris, France •www.studio.example"), vec![]);
        assert_eq!(warns(&c, "Marie Client", "marie.client@gmail.com", "I can't log in to www.studio.example", false), None);
    }

    #[test]
    fn case_a_the_organisation_from_outside() {
        let c = knowing(&[]);
        let text = "Please wire it today.\n\nDana Whitfield\nCEO\nwww.studio.example";
        assert_eq!(warns(&c, "Direction", "dg.studio@gmail.com", text, false), shown("studio.example", "dg.studio@gmail.com"));
        // A mailing list header on a mail service excuses nothing.
        assert_eq!(warns(&c, "Direction", "dg.studio@gmail.com", text, true), shown("studio.example", "dg.studio@gmail.com"));
        // A site under the organisation's domain, the sender's address in capitals.
        let text = "Pay now.\n-- \nhttps://intranet.studio.example/finance";
        assert_eq!(warns(&c, "Direction", "DG.Studio@Gmail.com", text, false), shown("intranet.studio.example", "dg.studio@gmail.com"));
        // From inside the organisation, a colleague's page is no warning; with no organisation,
        // nothing is.
        assert_eq!(warns(&c, "Bo Chen", "bo@eu.studio.example", text, false), None);
        let none = BTreeSet::new();
        assert_eq!(c.check_signature(&EmailAddress::new("Direction", "dg.studio@gmail.com"), text, false, &none), None);
    }

    #[test]
    fn case_b_an_address_known_exactly() {
        let c = knowing(&[("Dana Whitfield", "dana@corp.example")]);
        let text = "Merci de traiter ce virement.\n\nDana Whitfield\nDirectrice générale\ndana@corp.example";
        assert_eq!(
            warns(&c, "Direction Générale", "dg@corp-direction.example", text, false),
            shown("dana@corp.example", "dg@corp-direction.example")
        );
    }

    #[test]
    fn case_c_the_senders_name_on_free_mail() {
        let c = knowing(&[]);
        let text = "Call me.\n\nDana Whitfield\ndana.whitfield@unknown.example";
        assert_eq!(warns(&c, "Dana Whitfield", "dw.ceo@gmail.com", text, false), shown("dana.whitfield@unknown.example", "dw.ceo@gmail.com"));
        assert!(warns(&c, "Whitfield, Dana", "dw.ceo@yahoo.co.uk", "Dana\nwhitfield_dana@unknown.example", false).is_some());
        // From an organisation's own domain the name is its business.
        assert_eq!(warns(&c, "Dana Whitfield", "dw@other-corp.example", text, false), None);
        // Another name in the address.
        assert_eq!(warns(&c, "Dana Whitfield", "dw.ceo@gmail.com", "Call me.\n\nbo.chen@unknown.example", false), None);
    }

    #[test]
    fn what_says_nothing() {
        let case_a = "Please wire it today.\n\nDana Whitfield\nCEO\nwww.studio.example";
        // The sender is known.
        let c = knowing(&[("Direction", "dg.studio@gmail.com")]);
        assert!(c.knows("dg.studio@gmail.com"));
        assert_eq!(warns(&c, "Direction", "dg.studio@gmail.com", case_a, false), None);
        let c = knowing(&[("Dana Whitfield", "dana@corp.example")]);
        // A list from its own domain.
        assert_eq!(warns(&c, "Corp News", "news@corp-news.example", case_a, true), None);
        assert_eq!(warns(&c, "Corp News", "news@corp-news.example", case_a, false), shown("studio.example", "news@corp-news.example"));
        // A mail service's address in the signature.
        assert_eq!(warns(&c, "Dana Whitfield", "dw.ceo@gmail.com", "Hi\n-- \nDana Whitfield\nsomeone@gmail.com", false), None);
        // A third party's signature, in what the sender forwards or answers.
        let third = "Voir ci-dessous.\n\nDe : Dana Whitfield <dana@corp.example>\nEnvoyé : lundi 3 novembre 2026\nÀ : Bob\n\nDana Whitfield\nwww.studio.example\ndana@corp.example";
        assert_eq!(warns(&c, "Bob Martin", "bob@partner.example", third, false), None);
        let unquoted = third.replace("De :", "Re :");
        assert!(warns(&c, "Bob Martin", "bob@partner.example", &unquoted, false).is_some());
        // A domain related to the sender's.
        assert_eq!(warns(&c, "Ana Lopez", "ana@studio.example", "Hi\n\nwww.mail.studio.example", false), None);
        assert_eq!(warns(&c, "Dana Whitfield", "dana@mail.corp.example", "Hi\n\nDana\ndana@corp.example", false), None);
        // The user's own address, in their message quoted without a mark (Review Focus 1).
        assert_eq!(warns(&c, "Bob Martin", "bob@partner.example", "Agreed.\n\nAna Lopez\nana@studio.example", false), None);
        // The user's own mail; no address at all.
        assert_eq!(warns(&c, "Ana Lopez", "ana@studio.example", "Hi\n\nDana\ndana@corp.example\nwww.studio.example", false), None);
        assert_eq!(warns(&c, "Dana Whitfield", "", case_a, false), None);
        assert_eq!(warns(&c, "Dana Whitfield", "not an address", case_a, false), None);
    }

    #[test]
    fn hostile_text_in_bounded_time() {
        let c = knowing(&[("Dana Whitfield", "dana@corp.example")]);
        let from = EmailAddress::new("Dana Whitfield", "dw.ceo@gmail.com");
        let texts = [
            "x".repeat(1_000_000),
            "x\n".repeat(100_000),
            "\u{200B}".repeat(100_000),
            "é".repeat(40_000),
            format!("a{}", "é".repeat(40_000)),
            format!("ab{}", "€".repeat(40_000)),
            "@".repeat(200_000),
            "www.".repeat(100_000),
            "a@b.example ".repeat(50_000),
            "From: a\n".repeat(50_000),
            "-- \n".repeat(50_000),
            format!("x@{}example", "a.".repeat(100_000)),
            "\r\n".repeat(100_000),
        ];
        let started = Instant::now();
        for text in &texts {
            let _ = c.check_signature(&from, text, false, &org());
            let own = own_text(text);
            assert!(own.len() <= READ_MOST);
            assert!(signature_shows(text).len() <= MOST_SHOWN);
            let _ = lookup_name(text);
        }
        assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
    }
}
