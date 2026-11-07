// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The account's contacts, contact groups, notes and tasks as Outlook items, in the data file's
//! own Contacts, Notes and Tasks folders (MS-OXOCNTC, MS-OXONOTE, MS-OXOTASK; the folders recorded
//! as the file's, MS-OXOSFLD 2.2, so Outlook shows them as its own): a card as an `IPM.Contact`
//! or, a group's, an `IPM.DistList` ([`contact`]), a note as an `IPM.StickyNote`, a task as an
//! `IPM.Task` ([`task`]). A file added to keeps its own items, and those it records as its
//! special folders take the account's. What Outlook has no place for stays in the item's text,
//! as the import reads it back (`pst_pim::rest`).

use outlook_pst::ltp::prop_context::GuidValue;
use outlook_pst::{UnicodePstSpecialFolder as Special, UnicodePstValue};
use quatrelle_model::pim::{NOTE_COLORS, NOTE_YELLOW, Note, Pim};

use super::item::{PSETID_COMMON, filetime};
use super::*;

mod batch;
use batch::write_batch;

/// PSETID_Address and PSETID_Note (MS-OXPROPS 1.3.2).
pub(super) const PSETID_ADDRESS: GuidValue = GuidValue::new(0x0006_2004, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const PSETID_NOTE: GuidValue = GuidValue::new(0x0006_200E, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
/// A note's size in Outlook's window, as Outlook makes one (PidLidNoteWidth, PidLidNoteHeight).
const NOTE_SIZE: (i32, i32) = (200, 166);

/// A property's value.
pub(super) enum Value {
    Int(i32),
    Bool(bool),
    /// 100 ns since 1601.
    Time(i64),
    Float(f64),
    Text(String),
    Bin(Vec<u8>),
    Texts(Vec<String>),
    Bins(Vec<Vec<u8>>),
}

/// Which property: tagged, named by a number in a property set, or the categories (`Keywords`).
pub(super) enum Key {
    Tag(u16),
    Lid(GuidValue, u32),
    Keywords,
}

pub(super) fn tag(id: u16, value: Value) -> (Key, Value) {
    (Key::Tag(id), value)
}

pub(super) fn lid(set: GuidValue, id: u32, value: Value) -> (Key, Value) {
    (Key::Lid(set, id), value)
}

/// A file of an item; `picture`: a contact's picture (PidTagAttachmentContactPhoto).
pub(super) struct File {
    pub(super) name: String,
    pub(super) mime: String,
    pub(super) data: Vec<u8>,
    pub(super) picture: bool,
}

/// An item, ready to write.
#[derive(Default)]
pub(super) struct Item {
    pub(super) class: &'static str,
    pub(super) subject: String,
    /// Its text, with Outlook's line ends.
    pub(super) body: String,
    /// When it last changed (100 ns since 1601): the writer's delivery time, which is its
    /// PidTagLastModificationTime too.
    pub(super) modified: i64,
    pub(super) props: Vec<(Key, Value)>,
    pub(super) files: Vec<File>,
}

impl Item {
    fn bytes(&self) -> usize {
        let props = self.props.iter().map(|(_, v)| match v {
            Value::Text(s) => s.len(),
            Value::Bin(b) => b.len(),
            Value::Texts(t) => t.iter().map(String::len).sum(),
            Value::Bins(b) => b.iter().map(Vec::len).sum(),
            _ => 8,
        });
        self.body.len().saturating_add(props.sum()).saturating_add(self.files.iter().map(|f| f.data.len()).sum())
    }
}

/// Text with Outlook's line ends (CR LF).
pub(super) fn crlf(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// What went into the data file.
#[derive(Default)]
pub(super) struct Written {
    pub(super) contacts: usize,
    pub(super) notes: usize,
    pub(super) tasks: usize,
}

/// The account's items for each of the file's folders, those it has; `now` is when the export
/// runs (Unix seconds), a card's time.
fn items(pim: &Pim, now: i64) -> Vec<(Special, Vec<Item>)> {
    let contacts: Vec<Item> = pim.cards.iter().map(|c| super::contact::item(&c.card, now)).collect();
    let notes = pim.notes.iter().map(|n| note(&n.note, now)).collect();
    let tasks = pim.tasks.iter().map(|t| super::task::item(&t.task, now)).collect();
    [(Special::Contacts, contacts), (Special::Notes, notes), (Special::Tasks, tasks)].into_iter().filter(|(_, items)| !items.is_empty()).collect()
}

/// A note (MS-OXONOTE 2.2.1): its text the body, its first line the subject, its colour.
fn note(n: &Note, now: i64) -> Item {
    let color = if n.color < NOTE_COLORS { n.color } else { NOTE_YELLOW };
    Item {
        class: "IPM.StickyNote",
        subject: n.title().to_string(),
        body: crlf(&n.text),
        modified: filetime(if n.modified > 0 { n.modified } else { now }),
        props: vec![
            lid(PSETID_NOTE, 0x8B00, Value::Int(i32::from(color))),
            lid(PSETID_NOTE, 0x8B02, Value::Int(NOTE_SIZE.0)),
            lid(PSETID_NOTE, 0x8B03, Value::Int(NOTE_SIZE.1)),
        ],
        ..Default::default()
    }
}

/// Write the account's contacts, notes and tasks (`pim`) into the data file at `path` (in
/// `format`; made when there is none yet: `created`); `existing`: a file added to, whose special
/// folders take them; `taken`: where the account's mail went, so a new folder isn't one of
/// those. The file has the mail by then: one that can't record a folder as its own keeps the
/// items, and the log says why.
pub(super) fn write(
    path: &Path,
    format: Format,
    pim: &Pim,
    taken: &[Vec<String>],
    existing: bool,
    created: &mut bool,
    now: i64,
) -> Result<Written, String> {
    let mut done = Written::default();
    let all = items(pim, now);
    if all.is_empty() {
        return Ok(done);
    }
    if !*created {
        UnicodePstFile::create_empty(path).map_err(|e| e.to_string())?;
        *created = true;
    }
    for (role, items) in all {
        let (names, recorded) = folder_for(path, format, role, existing, taken);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        // Added to again: what the folder has already isn't written twice.
        let had = if existing { already(path, format, &names) } else { HashSet::new() };
        let count = items.len();
        let items: Vec<Item> = items.into_iter().filter(|i| !had.contains(&same(i.class, &i.subject, &i.body))).collect();
        if items.len() < count {
            log::info!("{} of the account's {role:?} items are in the data file already", count - items.len());
        }
        format.create_folder(path, &names, role.container_class()).map_err(|e| e.to_string())?;
        let mut start = 0;
        while start < items.len() {
            let (mut end, mut bytes) = (start, 0usize);
            while end < items.len() && end - start < BATCH && (end == start || bytes < BATCH_BYTES) {
                bytes = bytes.saturating_add(items.get(end).map_or(0, Item::bytes));
                end += 1;
            }
            write_batch(path, format, &names, items.get(start..end).unwrap_or_default())?;
            start = end;
        }
        match role {
            Special::Contacts => done.contacts = count,
            Special::Notes => done.notes = count,
            _ => done.tasks = count,
        }
        if !recorded && let Err(e) = format.set_special_folder(path, role, &names) {
            log::info!("the data file's {role:?} can't be recorded: {e}");
        }
    }
    Ok(done)
}

/// The folder `role`'s items go in, and whether the file records it as such already: the one a
/// file added to records (a panic in the writer's reader costs only that), else the role's name,
/// or that name with a number when the account's mail took it.
fn folder_for(path: &Path, format: Format, role: Special, existing: bool, taken: &[Vec<String>]) -> (Vec<String>, bool) {
    if existing && let Ok(Ok(Some(names))) = std::panic::catch_unwind(|| format.special_folder(path, role)) {
        return (names, true);
    }
    let free = |name: &str| !taken.iter().any(|t| t.first().is_some_and(|f| f.eq_ignore_ascii_case(name)));
    let name = role.default_name();
    let name = std::iter::once(name.to_string())
        .chain((2..=place::MAX_SUFFIX).map(|n| format!("{name} ({n})")))
        .find(|n| free(n))
        .unwrap_or_else(|| name.to_string());
    (vec![name], false)
}

/// What tells an item from another in a folder: its class, subject and text (its time is when it
/// was exported, for a card).
fn same(class: &str, subject: &str, body: &str) -> (String, String, String) {
    (class.to_ascii_lowercase(), subject.trim().to_string(), body.replace("\r\n", "\n").trim().to_string())
}

/// What the items in the folder at `names` of the file at `path` are (by [`same`]); none when
/// there is no such folder, or it can't be read (a panic in the reader too).
fn already(path: &Path, format: Format, names: &[&str]) -> HashSet<(String, String, String)> {
    let read = std::panic::catch_unwind(|| -> io::Result<HashSet<(String, String, String)>> {
        let any = format.store(path)?;
        let open = |row: u32| any.properties().make_entry_id(NodeId::from(row)).and_then(|e| any.open_folder(&e));
        let mut folder = any.open_folder(&any.properties().ipm_sub_tree_entry_id()?)?;
        for name in names {
            let Some(table) = folder.hierarchy_table() else { return Ok(HashSet::new()) };
            let next = table
                .rows_matrix()
                .filter_map(|row| open(u32::from(row.id())).ok())
                .find(|f| f.properties().display_name().is_ok_and(|n| n == *name));
            let Some(next) = next else { return Ok(HashSet::new()) };
            folder = next;
        }
        let mut out = HashSet::new();
        for row in folder.contents_table().map(|t| t.rows_matrix().collect::<Vec<_>>()).unwrap_or_default() {
            let Ok(entry) = any.properties().make_entry_id(NodeId::from(u32::from(row.id()))) else { continue };
            let Ok(m) = any.open_message(&entry, Some(&[0x001A, 0x0037, 0x1000])) else { continue };
            let text = |id: u16| m.properties().get(id).and_then(crate::pst::text_of).unwrap_or_default();
            out.insert(same(&text(0x001A), &text(0x0037), &text(0x1000)));
        }
        Ok(out)
    });
    match read {
        Ok(Ok(found)) => found,
        _ => HashSet::new(),
    }
}

/// The categories, as an item's `Keywords`.
pub(super) fn keywords(categories: &[String]) -> Option<(Key, Value)> {
    let names: Vec<String> = categories.iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect();
    (!names.is_empty()).then_some((Key::Keywords, Value::Texts(names)))
}

/// A reminder at `at` (Unix seconds), as a task's (PidLidReminderSet, PidLidReminderDelta,
/// PidLidReminderTime and PidLidReminderSignalTime).
pub(super) fn reminder(at: i64) -> [(Key, Value); 4] {
    [
        lid(PSETID_COMMON, 0x8503, Value::Bool(true)),
        lid(PSETID_COMMON, 0x8501, Value::Int(0)),
        lid(PSETID_COMMON, 0x8502, Value::Time(filetime(at))),
        lid(PSETID_COMMON, 0x8560, Value::Time(filetime(at))),
    ]
}

/// A single value as the writer takes it (`None` for a list).
fn single(v: &Value) -> Option<UnicodePstValue<'_>> {
    Some(match v {
        Value::Int(n) => UnicodePstValue::Integer32(*n),
        Value::Bool(b) => UnicodePstValue::Boolean(*b),
        Value::Time(t) => UnicodePstValue::Time(*t),
        Value::Float(f) => UnicodePstValue::Floating64(*f),
        Value::Text(s) => UnicodePstValue::Unicode(s),
        Value::Bin(b) => UnicodePstValue::Binary(b),
        Value::Texts(_) | Value::Bins(_) => return None,
    })
}
