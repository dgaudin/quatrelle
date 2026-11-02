//! Exchange's and Graph's answers, read as a message's text: the first byte picks the server, the
//! rest is a `GetItem` answer (EWS) or a message's JSON (Graph). Whatever message is built is read
//! the way the engine reads it, with `parse_text_first`.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&kind, rest)) = data.split_first() else {
        return;
    };
    let built = if kind & 1 == 0 {
        let root = quatrelle_net::ews::parse(&String::from_utf8_lossy(rest));
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
