// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a data file holds beside plain mail, as the import brings it into the fork's own calendar,
//! address book, tasks and notes: files made by the crate's own writer, read back by `import`.

use outlook_pst::ltp::prop_context::GuidValue;
use outlook_pst::{UnicodePstAttachment, UnicodePstFile as Writer, UnicodePstMessage, UnicodePstName, UnicodePstProperty, UnicodePstValue};

use super::*;

mod kinds;
mod occurrences;

const OLE: [u8; 8] = [0xC0, 0, 0, 0, 0, 0, 0, 0x46];
pub(super) const APPOINTMENT: GuidValue = GuidValue::new(0x0006_2002, 0, 0, OLE);
pub(super) const ADDRESS: GuidValue = GuidValue::new(0x0006_2004, 0, 0, OLE);
pub(super) const COMMON: GuidValue = GuidValue::new(0x0006_2008, 0, 0, OLE);
pub(super) const LOG: GuidValue = GuidValue::new(0x0006_200A, 0, 0, OLE);
pub(super) const MEETING: GuidValue = GuidValue::new(0x6ED8_DA90, 0x450B, 0x101B, [0x98, 0xDA, 0x00, 0xAA, 0x00, 0x3F, 0x13, 0x05]);

/// A property named by its number in `set`.
pub(super) fn named(set: GuidValue, lid: u32, value: UnicodePstValue<'_>) -> UnicodePstProperty<'_> {
    UnicodePstProperty::named(set, UnicodePstName::Id(lid), value)
}

/// A FILETIME (100 ns since 1601) for Unix seconds.
pub(super) fn filetime(unix: i64) -> i64 {
    (unix + 11_644_473_600) * 10_000_000
}

/// An item of class `class` as the writer takes it, from Riley, sent 2023-11-14.
pub(super) fn item<'a>(class: &'a str, subject: &'a str, body: &'a str, properties: &'a [UnicodePstProperty<'a>]) -> UnicodePstMessage<'a> {
    UnicodePstMessage {
        subject,
        sender_name: "Riley",
        sender_email: "riley@studio.example",
        recipients: &[],
        body,
        html_body: None,
        message_id: "",
        delivery_time: filetime(1_700_000_000),
        message_class: class,
        properties,
    }
}

/// One item to write: the folder it goes in (from the top of the mailbox), that folder's class
/// (`IPF.Note` for mail), the item and its attachments.
pub(super) struct Put<'a> {
    pub folder: &'a [&'a str],
    pub class: &'a str,
    pub item: UnicodePstMessage<'a>,
    pub files: Vec<UnicodePstAttachment<'a>>,
}

impl<'a> Put<'a> {
    pub fn new(folder: &'a [&'a str], class: &'a str, item: UnicodePstMessage<'a>) -> Put<'a> {
        Put { folder, class, item, files: Vec::new() }
    }
}

/// A data file of these items, imported into a scratch cache (removed when dropped).
pub(super) struct Brought {
    pub dir: std::path::PathBuf,
    pub store: Store,
    pub done: super::super::Imported,
}

impl Brought {
    pub fn of(name: &str, items: &[Put<'_>]) -> Brought {
        let dir = std::env::temp_dir().join(format!("quatrelle-pst-pim-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("items.pst");
        drop(Writer::create_empty(&file).unwrap());
        for p in items {
            drop(Writer::create_folder(&file, p.folder, p.class).unwrap());
            drop(Writer::append_in_folder_with_attachments(&file, p.folder, &p.item, &p.files).unwrap());
        }
        let store = Store::open(dir.join("cache")).unwrap();
        let done = import(&file, &store, "d1", &mut |_| {}).unwrap();
        Brought { dir, store, done }
    }

    /// The contacts, calendar, tasks and notes that came in.
    pub fn pim(&self) -> Pim {
        self.store.load_account_doc("d1", PIM_DOC).unwrap_or_default()
    }

    /// The messages that came in to the folder at `path`, with their sources.
    pub fn messages(&self, path: &str) -> Vec<(quatrelle_model::Summary, Vec<u8>)> {
        let list = self.store.load_folder("d1", path).map(|s| s.messages).unwrap_or_default();
        list.into_iter().map(|s| (s.clone(), self.store.read_body(&s.key).unwrap())).collect()
    }
}

impl Drop for Brought {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
