//! Message ids and their numbers, each id kept small. Exchange's ids are some 150 characters of
//! base64, 195,000 of them in the owner's mailbox, and the two tables that looked them up held
//! 50 MB; they differ from one another in a few bytes only (where the message and its folder
//! are), so each is kept as its differences from an earlier one of the same length, a template:
//! 16 bytes, the tables some 11 MB. Written to disk, they're the ids again.
//!
//! An id must always come out the same, or it would find no number and be given another (its
//! message downloaded again, a mark made on the old number lost): it's told against the first
//! template that fits, and templates are only ever added to, so a later one never comes first.
//! A code is read back whenever it's made, and one that doesn't give the id exactly isn't used,
//! so no two ids ever share one: such an id, like one that isn't base64, is kept whole.

use std::collections::HashMap;
use std::sync::Arc;

use quatrelle_model::vcard::base64_decode;
use serde::{Deserialize, Serialize};

use crate::url::base64;

/// The bytes of a code after its first (what it is and how long): a template's number, then
/// the runs of bytes that differ from it, each as its place, its length and the bytes.
const ROOM: usize = 15;

/// Templates at most (a template's number is a byte).
const TEMPLATES: usize = 255;

/// What a code is (its first byte's high half; the low half is how many of the others it uses):
/// a short id's own bytes…
const RAW: u8 = 0x00;
/// …standard base64 told against a template…
const STANDARD: u8 = 0x10;
/// …or base64 in the URL's alphabet (`-` and `_`).
const URL_SAFE: u8 = 0x20;

/// An id as kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Code([u8; ROOM + 1]);

impl Code {
    fn new(kind: u8, bytes: &[u8]) -> Option<Code> {
        let n = u8::try_from(bytes.len()).ok().filter(|n| usize::from(*n) <= ROOM)?;
        let mut c = [0u8; ROOM + 1];
        let (head, rest) = c.split_first_mut()?;
        *head = kind | n;
        rest.get_mut(..bytes.len())?.copy_from_slice(bytes);
        Some(Code(c))
    }

    fn kind(&self) -> u8 {
        self.0.first().map_or(0, |b| b & 0xF0)
    }

    fn bytes(&self) -> &[u8] {
        let n = self.0.first().map_or(0, |b| usize::from(b & 0x0F));
        self.0.get(1..=n).unwrap_or(&[])
    }
}

/// Message ids and their numbers, both ways.
#[derive(Clone, Default)]
pub struct Ids {
    /// Ids met before (decoded), what the others are told against.
    templates: Vec<Box<[u8]>>,
    numbers: HashMap<Code, u32>,
    codes: HashMap<u32, Code>,
    /// Ids kept whole, shared by both tables.
    whole: HashMap<Arc<str>, u32>,
    whole_ids: HashMap<u32, Arc<str>>,
}

impl std::fmt::Debug for Ids {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ids({} told against {} templates, {} whole)", self.numbers.len(), self.templates.len(), self.whole.len())
    }
}

/// An id's bytes, if it's base64, and in which alphabet it would be.
fn decoded(id: &str) -> Option<(u8, Vec<u8>)> {
    let kind = if id.contains(['-', '_']) { URL_SAFE } else { STANDARD };
    let b = base64_decode(id)?;
    // A run's place is a byte.
    (b.len() <= 255).then_some((kind, b))
}

/// `b` as its differences from `t` (the template numbered `n`, as long), if they fit.
fn against(kind: u8, n: usize, b: &[u8], t: &[u8]) -> Option<Code> {
    let mut out = [0u8; ROOM];
    let (head, _) = out.split_first_mut()?;
    *head = u8::try_from(n).ok()?;
    let mut at = 1;
    let mut i = 0;
    while i < b.len() {
        if b.get(i) == t.get(i) {
            i += 1;
            continue;
        }
        // One run, over gaps of up to two equal bytes (cheaper than another run's place and length).
        let start = i;
        let mut end = i + 1;
        let mut j = end;
        while j < b.len() && j <= end + 2 {
            if b.get(j) != t.get(j) {
                end = j + 1;
            }
            j += 1;
        }
        let run = b.get(start..end)?;
        let seg = out.get_mut(at..at + 2 + run.len())?;
        let (place, rest) = seg.split_first_mut()?;
        let (len, bytes) = rest.split_first_mut()?;
        *place = u8::try_from(start).ok()?;
        *len = u8::try_from(run.len()).ok()?;
        bytes.copy_from_slice(run);
        at += 2 + run.len();
        i = end;
    }
    Code::new(kind, out.get(..at)?)
}

impl Ids {
    /// The id a code stands for.
    fn id_of(&self, c: &Code) -> Option<String> {
        let bytes = c.bytes();
        let kind = c.kind();
        if kind == RAW {
            return String::from_utf8(bytes.to_vec()).ok();
        }
        let (n, mut rest) = bytes.split_first()?;
        let mut b = self.templates.get(usize::from(*n))?.to_vec();
        while let [place, len, tail @ ..] = rest {
            let (run, more) = tail.split_at_checked(usize::from(*len))?;
            let start = usize::from(*place);
            b.get_mut(start..start + run.len())?.copy_from_slice(run);
            rest = more;
        }
        let s = base64(&b);
        match kind {
            STANDARD => Some(s),
            URL_SAFE => Some(
                s.chars()
                    .map(|ch| match ch {
                        '+' => '-',
                        '/' => '_',
                        ch => ch,
                    })
                    .collect(),
            ),
            _ => None,
        }
    }

    /// The code an id has, from the templates there are (none: it needs a template of its own,
    /// or is kept whole).
    fn told(&self, id: &str) -> Option<Code> {
        let gives = |c: &Code| self.id_of(c).as_deref() == Some(id);
        if id.len() <= ROOM {
            return Code::new(RAW, id.as_bytes()).filter(gives);
        }
        let (kind, b) = decoded(id)?;
        self.templates.iter().enumerate().filter(|(_, t)| t.len() == b.len()).find_map(|(n, t)| against(kind, n, &b, t).filter(gives))
    }

    /// The code an id has, a template made of it if it fits none (none: it's kept whole).
    fn code(&mut self, id: &str) -> Option<Code> {
        if let Some(c) = self.told(id) {
            return Some(c);
        }
        if id.len() <= ROOM || self.templates.len() >= TEMPLATES {
            return None;
        }
        let (kind, b) = decoded(id)?;
        let c = Code::new(kind, &[u8::try_from(self.templates.len()).ok()?])?;
        self.templates.push(b.into());
        if self.id_of(&c).as_deref() == Some(id) {
            Some(c)
        } else {
            // Not base64 as written (no code has it yet).
            self.templates.pop();
            None
        }
    }

    /// The number an id has.
    pub fn get(&self, id: &str) -> Option<u32> {
        self.told(id).and_then(|c| self.numbers.get(&c).copied()).or_else(|| self.whole.get(id).copied())
    }

    /// The id a number stands for.
    pub fn id(&self, n: u32) -> Option<String> {
        match self.codes.get(&n) {
            Some(c) => self.id_of(c),
            None => self.whole_ids.get(&n).map(|id| id.to_string()),
        }
    }

    /// Give an id a number.
    pub fn insert(&mut self, id: &str, n: u32) {
        match self.code(id) {
            Some(c) => {
                self.numbers.insert(c, n);
                self.codes.insert(n, c);
                self.whole_ids.remove(&n);
            }
            None => {
                let shared: Arc<str> = Arc::from(id);
                self.whole.insert(shared.clone(), n);
                self.whole_ids.insert(n, shared);
                self.codes.remove(&n);
            }
        }
    }

    /// Only the id's way: the id has the number.
    fn give_number(&mut self, id: &str, n: u32) {
        match self.code(id) {
            Some(c) => {
                self.numbers.insert(c, n);
            }
            None => {
                self.whole.insert(Arc::from(id), n);
            }
        }
    }

    /// Only the number's way: the number stands for the id.
    fn give_id(&mut self, n: u32, id: &str) {
        match self.code(id) {
            Some(c) => {
                self.codes.insert(n, c);
                self.whole_ids.remove(&n);
            }
            None => {
                let shared = self.whole.get_key_value(id).map_or_else(|| Arc::from(id), |(k, _)| k.clone());
                self.whole_ids.insert(n, shared);
                self.codes.remove(&n);
            }
        }
    }

    /// Take another's, each table as it had it (a number given again keeps its latest id).
    pub fn absorb(&mut self, other: &Ids) {
        for (id, n) in other.iter() {
            self.give_number(&id, n);
        }
        let back: Vec<(u32, String)> = other
            .codes
            .iter()
            .filter_map(|(n, c)| Some((*n, other.id_of(c)?)))
            .chain(other.whole_ids.iter().map(|(n, id)| (*n, id.to_string())))
            .collect();
        for (n, id) in back {
            self.give_id(n, &id);
        }
    }

    /// Every id with its number.
    pub fn iter(&self) -> impl Iterator<Item = (String, u32)> + '_ {
        self.numbers.iter().filter_map(|(c, n)| Some((self.id_of(c)?, *n))).chain(self.whole.iter().map(|(id, n)| (id.to_string(), *n)))
    }

    /// The numbers the ids have.
    pub fn numbers(&self) -> impl Iterator<Item = u32> + '_ {
        self.numbers.values().chain(self.whole.values()).copied()
    }

    /// Keep the ids whose number `keep` keeps.
    pub fn retain(&mut self, keep: impl Fn(u32) -> bool) {
        self.numbers.retain(|_, n| keep(*n));
        self.codes.retain(|n, _| keep(*n));
        self.whole.retain(|_, n| keep(*n));
        self.whole_ids.retain(|n, _| keep(*n));
    }

    pub fn len(&self) -> usize {
        self.numbers.len() + self.whole.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// On disk, the ids themselves and their numbers (what Quatrelle always wrote).
impl Serialize for Ids {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_map(self.iter())
    }
}

impl<'de> Deserialize<'de> for Ids {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Read;
        impl<'de> serde::de::Visitor<'de> for Read {
            type Value = Ids;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("message ids and their numbers")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Ids, A::Error> {
                let mut ids = Ids::default();
                while let Some((id, n)) = map.next_entry::<String, u32>()? {
                    ids.insert(&id, n);
                }
                Ok(ids)
            }
        }
        d.deserialize_map(Read)
    }
}

#[cfg(test)]
mod tests;
