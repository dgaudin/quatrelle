// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Exchange's, Graph's and IMAP's answers, read as a message's text: the first byte picks the
//! server, the rest is a `GetItem` answer (EWS), a message's JSON (Graph) or what an IMAP server
//! says after its greeting to `UID FETCH 1 (UID BODYSTRUCTURE)` and to the header and text
//! parts it names. Whatever message is built is read the way the engine reads it, with
//! `parse_text_first`. Each of an EWS answer's response messages is read as a refusal too
//! (`check_message`, with the fields its `MessageXml` names), as a `GetReminders` answer
//! (`reminders_listed`), for the server's version (`server_major`) and for each message's marks
//! (`marks_listed`).
#![no_main]

use std::io::{self, Read, Write};

use libfuzzer_sys::fuzz_target;

/// An IMAP server: its bytes, after a greeting, read in small pieces; writes dropped.
struct Imap<'a> {
    data: &'a [u8],
}

impl Read for Imap<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf.len().min(self.data.len()).min(61);
        let (now, rest) = self.data.split_at(n);
        buf[..n].copy_from_slice(now);
        self.data = rest;
        Ok(n)
    }
}

impl Write for Imap<'_> {
    fn write(&mut self, sent: &[u8]) -> io::Result<usize> {
        Ok(sent.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&kind, rest)) = data.split_first() else {
        return;
    };
    let built = if kind % 3 == 2 {
        let script = [b"* OK ready\r\n".as_slice(), rest].concat();
        match quatrelle_net::imap::Client::from_stream(Imap { data: &script }) {
            Ok(mut c) => c.text_first(1).ok().flatten(),
            Err(_) => None,
        }
    } else if kind & 1 == 0 {
        let root = quatrelle_net::ews::parse(&String::from_utf8_lossy(rest));
        for m in quatrelle_net::ews::messages(&root) {
            let _ = quatrelle_net::ews::check_message(m);
        }
        // Read as a `GetReminders` answer too, for the server's version, and for each
        // message's marks (its follow-up status and the time it was done among them).
        let _ = quatrelle_net::ews::reminders_listed(&root);
        let _ = quatrelle_net::ews::marks_listed(&root);
        let _ = quatrelle_net::ews::server_major(&root);
        quatrelle_net::ews::text_first(&root)
    } else {
        match serde_json::from_slice(rest) {
            Ok(v) => quatrelle_net::graph::text_first(&v),
            Err(_) => None,
        }
    };
    if let Some(raw) = built {
        let _ = quatrelle_mime::parse_text_first(&raw);
        let _ = quatrelle_mime::parse(&raw);
    }
});
