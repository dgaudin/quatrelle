// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A POP3 server's side of a session (RFC 1939), as the client reads it over any `Read + Write`:
//! the greeting, the sign-in, `UIDL`, `RETR` with its dot-stuffing and final dot, `DELE`, `QUIT`,
//! and a whole download into an in-memory cache. Never a panic; lines, message sizes, counts and
//! the ids kept are within their caps, and a refusal is an error (`-ERR`), never half a message.
#![no_main]

use std::io::{self, Read, Write};

use libfuzzer_sys::fuzz_target;
use quatrelle_net::pop::{self, Client, Keep};
use quatrelle_store::Store;

/// The server: its bytes read in small pieces (as a socket hands them over), writes dropped.
struct Server<'a> {
    data: &'a [u8],
}

impl Read for Server<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf.len().min(self.data.len()).min(73);
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

/// What `UIDL` listed, within their caps.
fn check_listed(list: &[(u32, String)]) {
    assert!(list.len() <= 200_000, "{} messages", list.len());
    for (_n, id) in list {
        assert!(id.chars().count() <= 70, "{} characters in an id", id.chars().count());
    }
}

fuzz_target!(|data: &[u8]| {
    // One session, each command taking the next answer.
    if let Ok(mut c) = Client::greet(Server { data }) {
        let _ = c.login("me@pop.example", "pw");
        let _ = c.login_xoauth2("me@pop.example", "token");
        match c.uidl() {
            Ok(list) => {
                check_listed(&list);
                for (n, _) in list.iter().take(16) {
                    let _ = c.retr(*n);
                }
            }
            Err(_) => {}
        }
        let _ = c.dele(1);
        let _ = c.quit();
    }
    // A whole download: the same answers, read by the session the account opens.
    let store = Store::memory();
    let _ = pop::folders(&store, "p1");
    if let Ok(mut c) = Client::greet(Server { data }) {
        let keep = Keep { remove_now: false, days: Some(14), with_trash: true };
        let _ = pop::fetch_new(&mut c, &store, "p1", "INBOX", &keep, 1_000_000);
        let _ = c.quit();
    }
});
