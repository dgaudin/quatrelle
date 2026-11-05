// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The fuzz targets' seeds for data files (`fuzz/seeds/pst`, `pst_item`, `pst_props`,
//! `pst_recurrence`) read as
//! their targets read them, without cargo-fuzz: none may panic, and what `pst_item` checks of
//! the rebuilt header holds.

use std::borrow::Cow;
use std::path::PathBuf;

use outlook_pst::{UnicodePstFile as Writer, UnicodePstMessage};

use super::*;

/// The seeds of one target, by name.
fn seeds(target: &str) -> Vec<(String, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds").join(target);
    let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read(&p).unwrap()))
        .collect();
    out.sort();
    assert!(!out.is_empty(), "no seeds in {}", dir.display());
    out
}

/// `pst`: a whole data file, or five-byte patches (offset, byte) to a good one (here one the
/// writer makes, the target's being an export), imported into an empty cache.
#[test]
fn the_pst_seeds_import_without_a_panic() {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-seeds-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let good = dir.join("good.pst");
    let message = UnicodePstMessage {
        subject: "Plan — café",
        sender_name: "Riley",
        sender_email: "riley@example.com",
        recipients: &[],
        body: "Hello there.",
        html_body: Some("<p>Hello <b>there</b>.</p>"),
        message_id: "<m1@example.com>",
        delivery_time: 133_485_408_000_000_000,
        message_class: "IPM.Note",
        properties: &[],
    };
    drop(Writer::create(&good, &message).unwrap());
    let good = std::fs::read(&good).unwrap();
    for (name, data) in seeds("pst") {
        let file = if data.starts_with(b"!BDN") {
            data
        } else {
            let mut file = good.clone();
            for p in data.chunks_exact(5) {
                let at = u32::from_le_bytes([p[0], p[1], p[2], p[3]]) as usize % file.len();
                file[at] = p[4];
            }
            file
        };
        let path = dir.join(format!("{name}.pst"));
        std::fs::write(&path, &file).unwrap();
        let _ = import(&path, &Store::memory(), "d1", &mut |_| {});
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `pst_item`'s input: properties, each an id, a kind, a length and the value (5 starts the next
/// recipient's), as the target reads them.
fn props(mut data: &[u8]) -> (Vec<(u16, Val<'_>)>, Vec<Vec<(u16, Val<'_>)>>) {
    let mut item = Vec::new();
    let mut recipients: Vec<Vec<(u16, Val<'_>)>> = Vec::new();
    while let [a, b, kind, l1, l2, rest @ ..] = data {
        let len = usize::from(u16::from_le_bytes([*l1, *l2])).min(rest.len());
        let (value, after) = rest.split_at(len);
        data = after;
        let id = u16::from_le_bytes([*a, *b]);
        let number = |n: usize| value.iter().take(n).rev().fold(0i64, |acc, b| (acc << 8) | i64::from(*b));
        let v = match kind {
            0 => Val::Text(String::from_utf8_lossy(value)),
            1 => Val::Bin(value),
            2 => Val::Int(i64::from(number(4) as i32)),
            3 => Val::Bool(value.first().is_some_and(|b| *b != 0)),
            4 => Val::Time(number(8)),
            5 => {
                recipients.push(Vec::new());
                continue;
            }
            6 => {
                Val::Text(Cow::Owned(String::from_utf16_lossy(&value.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())))
            }
            7 => Val::Texts(value.split(|b| *b == 0).map(|t| String::from_utf8_lossy(t).into_owned()).collect()),
            _ => Val::Other,
        };
        match recipients.last_mut() {
            Some(r) => r.push((id, v)),
            None => item.push((id, v)),
        }
    }
    (item, recipients)
}

/// `pst_item`: each seed rebuilds, its added header lines clean, ASCII and short.
#[test]
fn the_pst_item_seeds_rebuild_with_clean_lines() {
    for (name, data) in seeds("pst_item") {
        let (item, recipients) = props(&data);
        let (raw, summary) = rebuilt(&item, &recipients).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!((-1..=1).contains(&summary.importance), "{name}");
        assert!(summary.flags.keywords.len() <= 65 && summary.flags.keywords.iter().all(|k| quatrelle_model::is_keyword(k)), "{name}");
        let end = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(raw.len());
        for line in String::from_utf8_lossy(&raw[..end]).split("\r\n") {
            assert!(line.len() <= 998 && line.chars().all(|c| c == '\t' || !c.is_control()), "{name}: {line:?}");
        }
        let _ = mime::parse(&raw);
    }
}

/// `pst_props`: a little-endian code page, a byte saying HTML, then the bytes.
#[test]
fn the_pst_props_seeds_read_in_every_page() {
    for (_, data) in seeds("pst_props") {
        let [a, b, c, d, html, bytes @ ..] = data.as_slice() else { continue };
        let cp = u32::from_le_bytes([*a, *b, *c, *d]);
        let _ = text_from_bytes(bytes, cp, html & 1 == 1);
        for page in [0, 932, 936, 949, 1200, 1201, 1251, 1252, 20127, 50220, 50225, 51932, 54936, 65000, 65001, 10007, 28606, 99_999] {
            let _ = text_from_bytes(bytes, page, html & 1 == 1);
        }
    }
}

/// `pst_recurrence`: the first occurrence's start (eight bytes), then an appointment's pattern;
/// the series and its changed occurrences, each replacing one occurrence, written and read again.
#[test]
fn the_pst_recurrence_seeds_read_without_a_panic() {
    for (name, data) in seeds("pst_recurrence") {
        let Some((start, pattern)) = data.split_first_chunk::<8>() else { continue };
        let events = super::super::repeating(pattern, i64::from_le_bytes(*start));
        let (series, changed) = events.split_first().unwrap();
        assert!(series.recurrence_id.is_none() && !changed.is_empty(), "{name}");
        for e in changed {
            assert!(e.recurrence_id.is_some() && e.rrule.is_none() && e.exdates.is_empty() && e.end >= e.start, "{name}");
            assert_eq!(e.uid, series.uid);
        }
        let _ = quatrelle_model::ical::parse(&quatrelle_model::ical::write(&events, None, 0), 0);
    }
}
