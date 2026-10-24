//! Voting buttons: the choices a sender offers ("Yes", "No"…), each recipient answering with one.
//!
//! Exchange keeps them in a binary property of the message, the verb stream (`PidLidVerbStream`,
//! [MS-OXOMSG] 2.2.1.74), and a vote goes back as a reply naming the choice
//! (`PidLidVerbResponse`, 2.2.1.75) for the sender's client to count. The stream holds a
//! `VoteOption` per verb with an ANSI name, then a parallel `VoteOptionExtras` per verb with the
//! name in UTF-16; streams written by mail clients may list other verbs (reply, forward) too,
//! which aren't votes.

/// `PSETID_Common`, the property set both properties are in.
pub const PSETID_COMMON: &str = "00062008-0000-0000-C000-000000000046";
/// `PidLidVerbStream`'s id in it.
pub const VERB_STREAM: u16 = 0x8520;
/// `PidLidVerbResponse`'s id in it.
pub const VERB_RESPONSE: u16 = 0x8524;

/// Verbs read from one stream at most.
const MAX_VERBS: u32 = 64;
/// The verb type of a voting option.
const VOTE: u32 = 4;

/// The voting choices in a verb stream, in order: empty for anything that isn't one.
pub fn choices(stream: &[u8]) -> Vec<String> {
    let mut r = Reader { b: stream, at: 0 };
    let (Some(0x0102), Some(count)) = (r.u16(), r.u32()) else { return Vec::new() };
    if count > MAX_VERBS {
        return Vec::new();
    }
    let mut verbs: Vec<(u32, String)> = Vec::new();
    for _ in 0..count {
        let Some(v) = r.option() else { return Vec::new() };
        verbs.push(v);
    }
    // The names in UTF-16, when the stream has them (any language), else the ANSI ones.
    let wide: Vec<Option<String>> = if r.u16() == Some(0x0104) { (0..count).map(|_| r.extras()).collect() } else { Vec::new() };
    verbs
        .into_iter()
        .enumerate()
        .filter(|(_, (kind, _))| *kind == VOTE)
        .filter_map(|(i, (_, ansi))| {
            let name = wide.get(i).cloned().flatten().filter(|w| !w.trim().is_empty()).unwrap_or(ansi);
            let name = name.trim().to_string();
            (!name.is_empty()).then_some(name)
        })
        .take(32)
        .collect()
}

/// A verb stream offering `choices`, the response sent as soon as one is picked.
pub fn stream(choices: &[&str]) -> Vec<u8> {
    // Names longer than the 255 characters a count holds are cut.
    let names: Vec<Vec<char>> = choices.iter().map(|c| c.trim().chars().take(255).collect()).filter(|c: &Vec<char>| !c.is_empty()).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&0x0102u16.to_le_bytes());
    out.extend_from_slice(&u32::try_from(names.len()).unwrap_or(0).to_le_bytes());
    for (i, name) in names.iter().enumerate() {
        // ANSI: characters outside Latin-1 become `?` (the UTF-16 copy below is the real name).
        let ansi: Vec<u8> = name.iter().map(|c| u8::try_from(u32::from(*c)).unwrap_or(b'?')).collect();
        let n = u8::try_from(ansi.len()).unwrap_or(u8::MAX);
        out.extend_from_slice(&VOTE.to_le_bytes());
        out.push(n);
        out.extend_from_slice(&ansi);
        out.push(8);
        out.extend_from_slice(b"IPM.Note");
        out.push(0);
        out.push(n);
        out.extend_from_slice(&ansi);
        out.extend_from_slice(&0u32.to_le_bytes());
        out.push(0);
        // fUseUSHeaders, Internal4, SendBehavior (send at once), Internal5, ID, Internal6.
        for v in [0u32, 1, 1, 2, u32::try_from(i + 1).unwrap_or(u32::MAX), u32::MAX] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out.extend_from_slice(&0x0104u16.to_le_bytes());
    for name in &names {
        let wide: Vec<u16> = name.iter().collect::<String>().encode_utf16().take(255).collect();
        let n = u8::try_from(wide.len()).unwrap_or(u8::MAX);
        for _ in 0..2 {
            out.push(n);
            for w in &wide {
                out.extend_from_slice(&w.to_le_bytes());
            }
        }
    }
    out
}

/// The subject of a vote on a message: the choice, then the message's subject.
pub fn response_subject(choice: &str, subject: &str) -> String {
    let subject = subject.trim();
    if subject.is_empty() { choice.trim().to_string() } else { format!("{}: {subject}", choice.trim()) }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let end = self.at.checked_add(n)?;
        let s = self.b.get(self.at..end)?;
        self.at = end;
        Some(s)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1)?.first().copied()
    }

    fn u16(&mut self) -> Option<u16> {
        let s = self.take(2)?;
        Some(u16::from_le_bytes([*s.first()?, *s.get(1)?]))
    }

    fn u32(&mut self) -> Option<u32> {
        let s = self.take(4)?;
        Some(u32::from_le_bytes([*s.first()?, *s.get(1)?, *s.get(2)?, *s.get(3)?]))
    }

    /// A counted ANSI string (Latin-1, as Windows' Western code page mostly is).
    fn ansi(&mut self) -> Option<String> {
        let n = usize::from(self.u8()?);
        Some(self.take(n)?.iter().map(|b| char::from(*b)).collect())
    }

    /// One `VoteOption`: its verb type and name.
    fn option(&mut self) -> Option<(u32, String)> {
        let kind = self.u32()?;
        let name = self.ansi()?;
        self.ansi()?; // the message class
        self.ansi()?; // Internal1String
        self.ansi()?; // the name again
        self.take(4 + 1)?; // Internal2, Internal3
        self.take(4 * 6)?; // fUseUSHeaders, Internal4, SendBehavior, Internal5, ID, Internal6
        Some((kind, name))
    }

    /// One `VoteOptionExtras`: the name in UTF-16 (given twice).
    fn extras(&mut self) -> Option<String> {
        let mut wide = || -> Option<String> {
            let n = usize::from(self.u8()?);
            let bytes = self.take(n.checked_mul(2)?)?;
            // Two bytes a UTF-16 unit, little-endian (an odd tail, if any, is left out).
            let (units, _) = bytes.as_chunks::<2>();
            let units: Vec<u16> = units.iter().map(|c| u16::from_le_bytes(*c)).collect();
            Some(String::from_utf16_lossy(&units))
        };
        let name = wide()?;
        wide()?;
        Some(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [MS-OXOMSG] 4.5: a message with Yes, No and Maybe.
    const EXAMPLE: &str = "02 01 03 00 00 00 04 00 00 00 03 59 65 73 08 49 50 4D 2E 4E 6F 74 65 00 03 59 65 73 00 00 00 00 \
        00 00 00 00 00 01 00 00 00 02 00 00 00 02 00 00 00 01 00 00 00 FF FF FF FF 04 00 00 00 02 4E 6F \
        08 49 50 4D 2E 4E 6F 74 65 00 02 4E 6F 00 00 00 00 00 00 00 00 00 01 00 00 00 02 00 00 00 02 00 \
        00 00 02 00 00 00 FF FF FF FF 04 00 00 00 05 4D 61 79 62 65 08 49 50 4D 2E 4E 6F 74 65 00 05 4D \
        61 79 62 65 00 00 00 00 00 00 00 00 00 01 00 00 00 02 00 00 00 02 00 00 00 03 00 00 00 FF FF FF \
        FF 04 01 03 59 00 65 00 73 00 03 59 00 65 00 73 00 02 4E 00 6F 00 02 4E 00 6F 00 05 4D 00 61 00 \
        79 00 62 00 65 00 05 4D 00 61 00 79 00 62 00 65 00";

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|h| u8::from_str_radix(h, 16).unwrap()).collect()
    }

    #[test]
    fn reads_the_specifications_example() {
        let b = hex(EXAMPLE);
        assert_eq!(b.len(), 0xD1);
        assert_eq!(choices(&b), ["Yes", "No", "Maybe"]);
    }

    #[test]
    fn writes_what_it_reads() {
        let b = stream(&["Approve", "Reject", "Peut-être", "はい"]);
        assert_eq!(choices(&b), ["Approve", "Reject", "Peut-être", "はい"]);
        // Byte for byte the specification's, but for sending at once rather than asking first.
        let mut example = hex(EXAMPLE);
        for at in [0x29, 0x5A, 0x91] {
            example[at] = 1;
        }
        assert_eq!(stream(&["Yes", "No", "Maybe"]), example);
        assert!(choices(&stream(&[" ", ""])).is_empty());
    }

    #[test]
    fn other_verbs_are_left_out() {
        // A reply verb (type 1) before the vote.
        let mut b = stream(&["Reply", "Yes"]);
        b[6] = 1;
        assert_eq!(choices(&b), ["Yes"]);
    }

    #[test]
    fn damaged_streams_give_nothing() {
        let b = hex(EXAMPLE);
        for n in 0..b.len() {
            // Cut short anywhere: no panic, and only whole options.
            let got = choices(&b[..n]);
            assert!(got.is_empty() || got == ["Yes", "No", "Maybe"], "{n}: {got:?}");
        }
        assert!(choices(&[]).is_empty());
        assert!(choices(&hex("02 01 FF FF FF FF")).is_empty());
        assert!(choices(&hex("03 01 01 00 00 00")).is_empty());
        // No UTF-16 copy: the ANSI names.
        let mut ansi_only = b.clone();
        ansi_only.truncate(0xA1);
        assert_eq!(choices(&ansi_only), ["Yes", "No", "Maybe"]);
    }

    #[test]
    fn response_subjects() {
        assert_eq!(response_subject("Yes", "Lunch on Friday?"), "Yes: Lunch on Friday?");
        assert_eq!(response_subject(" No ", ""), "No");
    }
}
