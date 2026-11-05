// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What an Outlook data file keeps of a message comes through the import: files made by the
//! crate's own writer, read back by `import`.

use outlook_pst::messaging::named_prop::PS_PUBLIC_STRINGS;
use outlook_pst::{UnicodePstFile as Writer, UnicodePstMessage, UnicodePstName, UnicodePstProperty, UnicodePstValue};

use super::*;

/// A data file holding `message` alone, imported: its summary and its source.
fn import_one(name: &str, properties: &[UnicodePstProperty<'_>]) -> (quatrelle_model::Summary, Vec<u8>) {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-fidelity-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("mail.pst");
    let message = UnicodePstMessage {
        subject: "Plan",
        sender_name: "Ana",
        sender_email: "ana@example.com",
        recipients: &[],
        body: "Hello.",
        html_body: None,
        message_id: "<plan@example.com>",
        // 2024-01-01T00:00:00Z
        delivery_time: 133_485_408_000_000_000,
        message_class: "IPM.Note",
        properties,
    };
    drop(Writer::create(&file, &message).unwrap());
    let store = Store::open(dir.join("cache")).unwrap();
    let done = import(&file, &store, "a1", &mut |_| {}).unwrap();
    assert_eq!(done.messages, 1, "{done:?}");
    let s = done.folders.iter().find_map(|f| store.load_folder("a1", &f.path).ok().and_then(|s| s.messages.into_iter().next())).unwrap();
    let raw = store.read_body(&s.key).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    (s, raw)
}

/// Outlook's categories (the string-named `Keywords`) come in as the message's keywords.
#[test]
fn categories_come_in_as_keywords() {
    let names = ["Work", "To do"];
    let categories = UnicodePstProperty::named(PS_PUBLIC_STRINGS, UnicodePstName::String("Keywords"), UnicodePstValue::MultipleUnicode(&names));
    let (s, _) = import_one("categories", &[categories]);
    assert_eq!(s.flags.keywords, ["work", "to_do"]);
}
