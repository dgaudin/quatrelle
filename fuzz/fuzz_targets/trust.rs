// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The URLs and hosts servers hand out (DAV hrefs, an Autodiscover's `EwsUrl`, a redirect's
//! `Location`): the host an account's credentials would go to, and its site.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_model::addr::trust::{site, url_host};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = site(&text);
    if let Some(host) = url_host(&text) {
        // What two parsers could read differently never gets through.
        assert!(!host.contains(['\\', '%', '@']), "{host:?}");
        let s = site(&host);
        assert_eq!(site(&s), s, "{host:?}");
        // A site reads back as itself: a question about it can be answered.
        if !s.is_empty() {
            assert_eq!(url_host(&s).map(|h| site(&h)).as_deref(), Some(s.as_str()), "{host:?}");
        }
    }
});
