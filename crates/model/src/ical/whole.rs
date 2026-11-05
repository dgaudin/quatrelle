// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Lines kept as written (an event's unknown ones, a zone's definition), made whole before they go
//! back out: a stray `END` in them must not end what's written around them on a server.

use super::*;

/// Lines kept as written (an event's, a zone's definition), made whole to write back: line breaks
/// taken out, every component they begin ended (by its own name), an `END` that ends nothing left
/// out, and a component named in `left_out` left out with all it holds (a `VEVENT` in a zone's
/// definition would be an event of its own when read back). Read back, they begin and end what
/// they did.
pub(super) fn whole<'a>(raw: impl IntoIterator<Item = &'a String>, left_out: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    // The components open around the line: each one's name, and whether it's written.
    let mut open: Vec<(String, bool)> = Vec::new();
    for r in raw {
        let s = r.replace(['\r', '\n'], "");
        // Not a folded line's next part, nor nothing.
        let s = s.trim_start_matches([' ', '\t']);
        let Some(l) = parse_line(s) else { continue };
        let writing = open.iter().all(|(_, w)| *w);
        match l.name.as_str() {
            "BEGIN" => {
                let name = l.value.trim().to_string();
                let w = writing && !left_out.iter().any(|x| name.eq_ignore_ascii_case(x));
                if w {
                    out.push(s.to_string());
                }
                // Bounded by the lines kept (200 an event, a zone's own cap).
                open.push((name, w));
            }
            "END" => {
                if let Some((name, true)) = open.pop() {
                    out.push(format!("END:{name}"));
                }
            }
            _ if writing => out.push(s.to_string()),
            _ => {}
        }
    }
    while let Some((name, w)) = open.pop() {
        if w {
            out.push(format!("END:{name}"));
        }
    }
    out
}
