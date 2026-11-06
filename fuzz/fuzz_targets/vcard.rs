// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A contact card (an attachment, a CardDAV contact) read, written back and read again.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_model::vcard;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    for c in vcard::parse(&text).iter().take(16) {
        let _ = c.display();
        let _ = c.matches("a");
        // Their files (R23), as the contact's page lists and opens them.
        for i in 0..quatrelle_model::pim::files::card_files(c).len().min(4) {
            let _ = quatrelle_model::pim::files::card_file_data(c, i);
        }
        let _ = vcard::parse(&vcard::write(c));
    }
    let _ = vcard::birthday_of(&text);
    let _ = vcard::birthday_for_exchange(&text);
});
