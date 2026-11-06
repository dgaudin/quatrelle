use super::*;
use crate::extra::tests::{folder, message, messages, temp_pst, text};

const PSETID_COMMON: GuidValue = GuidValue::new(0x0006_2008, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const PSETID_TASK: GuidValue = GuidValue::new(0x0006_2003, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const CUSTOM: GuidValue = GuidValue::new(0x1234_5678, 0x9ABC, 0xDEF0, [1, 2, 3, 4, 5, 6, 7, 8]);

/// The map's names and their property IDs, as the crate's reader reads the map.
fn map_ids(store: &Rc<UnicodeStore>) -> HashMap<NameKey, u16> {
    let map = store.named_property_map().unwrap();
    let properties = map.properties();
    let guids = properties.stream_guid().unwrap();
    let mut ids = HashMap::new();
    for entry in properties.stream_entry().unwrap() {
        let set = match entry.guid() {
            NamedPropertyGuid::None => continue,
            NamedPropertyGuid::Mapi => guid_bytes(&PS_MAPI),
            NamedPropertyGuid::PublicStrings => guid_bytes(&PS_PUBLIC_STRINGS),
            NamedPropertyGuid::GuidIndex(index) => guid_bytes(&guids[usize::from(index)]),
        };
        let key = match entry.id() {
            NamedPropertyId::Number(id) => NameKey::Id(set, id),
            NamedPropertyId::StringOffset(offset) => {
                let name = properties.lookup_string(offset).unwrap().to_string();
                NameKey::String(set, name.encode_utf16().collect())
            }
        };
        assert!(ids.insert(key, entry.prop_id()).is_none(), "a name twice");
    }
    ids
}

/// Every name is in the hash bucket MS-PST 2.4.7.5 puts it in: (its LID, or its string's CRC)
/// XOR its `(wGuid << 1) | N`, modulo the bucket count; the record has that hash, not the
/// string's offset.
fn assert_hashed(store: &Rc<UnicodeStore>) {
    let map = store.named_property_map().unwrap();
    let properties = map.properties();
    let buckets = u32::from(properties.bucket_count().unwrap());
    let strings = match properties.get(STREAM_STRING) {
        Some(PropertyValue::Binary(value)) => value.buffer().to_vec(),
        _ => Vec::new(),
    };
    let entries = match properties.get(STREAM_ENTRY) {
        Some(PropertyValue::Binary(value)) => value.buffer().to_vec(),
        _ => Vec::new(),
    };
    assert!(!entries.is_empty());
    for entry in entries.chunks_exact(8) {
        let id = u32::from_le_bytes(entry[0..4].try_into().unwrap());
        let guid = u16::from_le_bytes(entry[4..6].try_into().unwrap());
        let hash = if guid & 1 == 0 {
            id
        } else {
            let units = string_at(&strings, id).unwrap();
            let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
            crate::crc::compute_crc(0, &bytes)
        };
        let bucket = FIRST_BUCKET + ((hash ^ u32::from(guid)) % buckets) as u16;
        let mut record = hash.to_le_bytes().to_vec();
        record.extend_from_slice(&entry[4..8]);
        match properties.get(bucket) {
            Some(PropertyValue::Binary(value)) => assert!(
                value.buffer().chunks_exact(8).any(|r| r == record),
                "entry {entry:02X?} not in bucket {bucket:04X}"
            ),
            value => panic!("bucket {bucket:04X}: {value:?}"),
        }
    }
}

#[test]
fn outlooks_own_map_is_hashed_as_the_writer_hashes() {
    let path = temp_pst("named-outlook-map");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/Empty.pst"),
        &path,
    )
    .unwrap();
    let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
    assert_hashed(&store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn named_properties_are_looked_up_or_added_to_the_map() {
    // Outlook's own file, permute-encoded as Outlook made it.
    let path = temp_pst("named-properties");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/Empty.pst"),
        &path,
    )
    .unwrap();
    let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
    let before = map_ids(&store);
    drop(store);

    let keywords = ["Red category", "Été"];
    let custom_name = "x-mailcraft-test";
    let properties = [
        UnicodePstProperty::named(
            PS_PUBLIC_STRINGS,
            UnicodePstName::String("Keywords"),
            UnicodePstValue::MultipleUnicode(&keywords),
        ),
        UnicodePstProperty::named(
            PSETID_COMMON,
            UnicodePstName::Id(0x8530),
            UnicodePstValue::Unicode("Follow up"),
        ),
        UnicodePstProperty::named(
            PSETID_TASK,
            UnicodePstName::Id(0x8105),
            UnicodePstValue::Time(133_760_000_000_000_000),
        ),
        UnicodePstProperty::named(
            CUSTOM,
            UnicodePstName::String(custom_name),
            UnicodePstValue::Integer32(42),
        ),
        UnicodePstProperty::named(
            CUSTOM,
            UnicodePstName::Id(7),
            UnicodePstValue::Boolean(true),
        ),
        UnicodePstProperty::tagged(0x0E07, UnicodePstValue::Integer32(0)),
    ];
    drop(
        UnicodePstFile::append_in_folder(
            &path,
            &["Inbox"],
            &message("Named", "<named@example.com>", &properties),
        )
        .unwrap(),
    );

    let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
    let after = map_ids(&store);
    // What the map had keeps its ID.
    for (key, id) in &before {
        assert_eq!(after.get(key), Some(id), "{key:?}");
    }
    let id_of = |set: &GuidValue, name| *after.get(&NameKey::of(set, name)).unwrap();
    let keywords_id = id_of(&PS_PUBLIC_STRINGS, UnicodePstName::String("Keywords"));
    let flag_id = id_of(&PSETID_COMMON, UnicodePstName::Id(0x8530));
    let due_id = id_of(&PSETID_TASK, UnicodePstName::Id(0x8105));
    let custom_id = id_of(&CUSTOM, UnicodePstName::String(custom_name));
    let custom_lid = id_of(&CUSTOM, UnicodePstName::Id(7));
    let added = after.len() - before.len();
    assert!(added >= 2, "the custom set's names are new");
    for id in [keywords_id, flag_id, due_id, custom_id, custom_lid] {
        assert!(id >= 0x8000);
    }
    assert_hashed(&store);

    let (_, inbox) = folder(&store, &["Inbox"]);
    let all = messages(&store, &inbox);
    let (_, m) = all
        .iter()
        .find(|(_, m)| text(m.properties().get(0x0037)) == "Named")
        .unwrap();
    let p = m.properties();
    match p.get(keywords_id) {
        Some(PropertyValue::MultipleUnicode(values)) => {
            let values: Vec<String> = values
                .iter()
                .map(|value| value.to_string().trim_end_matches('\0').to_string())
                .collect();
            assert_eq!(values, keywords);
        }
        value => panic!("keywords: {value:?}"),
    }
    assert_eq!(text(p.get(flag_id)), "Follow up");
    assert!(matches!(
        p.get(due_id),
        Some(PropertyValue::Time(133_760_000_000_000_000))
    ));
    assert!(matches!(
        p.get(custom_id),
        Some(PropertyValue::Integer32(42))
    ));
    assert!(matches!(
        p.get(custom_lid),
        Some(PropertyValue::Boolean(true))
    ));
    drop(store);

    // The same names again, and one more: only that one is added.
    let more = [
        properties[0],
        properties[3],
        UnicodePstProperty::named(
            CUSTOM,
            UnicodePstName::String("x-another"),
            UnicodePstValue::Unicode("v"),
        ),
    ];
    let batch = [
        UnicodePstBatchMessage {
            message: message("Again", "<again@example.com>", &more),
            attachments: &[],
        },
        UnicodePstBatchMessage {
            message: message("Twice", "<twice@example.com>", &more),
            attachments: &[],
        },
    ];
    drop(
        UnicodePstFile::append_many_in_folder_with_attachments(&path, &["Inbox"], &batch).unwrap(),
    );
    let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
    let last = map_ids(&store);
    assert_eq!(last.len(), after.len() + 1);
    for (key, id) in &after {
        assert_eq!(last.get(key), Some(id), "{key:?}");
    }
    assert_hashed(&store);
    let another = *last
        .get(&NameKey::of(&CUSTOM, UnicodePstName::String("x-another")))
        .unwrap();
    let (_, inbox) = folder(&store, &["Inbox"]);
    let all = messages(&store, &inbox);
    for subject in ["Again", "Twice"] {
        let (_, m) = all
            .iter()
            .find(|(_, m)| text(m.properties().get(0x0037)) == subject)
            .unwrap();
        assert!(matches!(
            m.properties().get(custom_id),
            Some(PropertyValue::Integer32(42))
        ));
        assert_eq!(text(m.properties().get(another)), "v");
    }
    // The first message reads as it did.
    let (_, m) = all
        .iter()
        .find(|(_, m)| text(m.properties().get(0x0037)) == "Named")
        .unwrap();
    assert_eq!(text(m.properties().get(flag_id)), "Follow up");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn a_name_that_cant_be_written_is_an_error() {
    let path = temp_pst("named-refused");
    drop(UnicodePstFile::create(&path, &message("First", "<first@example.com>", &[])).unwrap());
    let before = std::fs::read(&path).unwrap();
    let long = "n".repeat(MAX_NAME + 1);
    let value = UnicodePstValue::Integer32(1);
    let bad: [&[UnicodePstProperty<'_>]; 3] = [
        &[UnicodePstProperty::named(
            CUSTOM,
            UnicodePstName::String(""),
            value,
        )],
        &[UnicodePstProperty::named(
            CUSTOM,
            UnicodePstName::String(&long),
            value,
        )],
        &[
            UnicodePstProperty::named(CUSTOM, UnicodePstName::Id(1), value),
            UnicodePstProperty::named(CUSTOM, UnicodePstName::Id(1), value),
        ],
    ];
    for properties in bad {
        let error = UnicodePstFile::append(&path, &message("Bad", "<bad@example.com>", properties))
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_file(path).unwrap();
}
