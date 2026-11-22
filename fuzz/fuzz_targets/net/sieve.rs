// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! ManageSieve's answers (RFC 5804): capabilities, literals, `OK`/`NO`/`BYE` with their codes,
//! the script read back (`GETSCRIPT`) with what Quatrelle keeps in its first lines, and the
//! session over any `Read + Write`. And the script itself: its settings and rules read from the
//! same bytes, written again, and read back the same. A line, a literal and one answer stay
//! within their caps; a refusal is an `Err`.
#![no_main]

use std::io::{self, Read, Write};

use libfuzzer_sys::fuzz_target;
use quatrelle_net::sieve::{self, Client, Settings};

/// The server: its bytes read in small pieces (as a socket hands them over), writes dropped.
struct Server<'a> {
    data: &'a [u8],
}

impl Read for Server<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf.len().min(self.data.len()).min(67);
        let (now, rest) = self.data.split_at(n);
        buf[..n].copy_from_slice(now);
        self.data = rest;
        Ok(n)
    }
}

impl Write for Server<'_> {
    fn write(&mut self, sent: &[u8]) -> io::Result<usize> {
        Ok(sent.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fuzz_target!(|data: &[u8]| {
    // The session: the greeting, sign-in, the script list, a script, the automatic replies, bye.
    if let Ok(mut c) = Client::from_stream(Server { data }) {
        let _ = c.login("me@studio.example", "pw");
        if let Ok((names, active)) = c.list() {
            assert!(names.len() <= 10_001, "{} scripts", names.len());
            if let Some(a) = active {
                assert!(names.contains(&a), "an active script not listed: {a:?}");
            }
        }
        let _ = c.get(sieve::SCRIPT);
        let _ = c.vacation();
        c.logout();
    }
    // The script's own text: its settings and rules, written again and read back.
    let text = String::from_utf8_lossy(data);
    let vacation = sieve::read_script(&text);
    let rules = sieve::read_rules(&text);
    assert!(rules.len() <= text.len() / 2 + 1, "{} rules", rules.len());
    if let Some(v) = vacation {
        let settings = Settings { vacation: v.clone(), rules: rules.clone(), categories: Vec::new() };
        let written = sieve::script(&settings, Some("theirs"));
        assert_eq!(sieve::read_script(&written), Some(v), "the replies written and read back");
        assert_eq!(sieve::read_rules(&written), rules, "the rules written and read back");
        // A script that does nothing still names what it carries; nothing there is a panic.
        let none = sieve::script(&Settings::default(), None);
        assert_eq!(sieve::read_script(&none), Some(Default::default()));
        assert!(sieve::read_rules(&none).is_empty());
    }
});
