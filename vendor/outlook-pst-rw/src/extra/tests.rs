use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn temp_pst(label: &str) -> std::path::PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "outlook-pst-{label}-{}-{stamp}.pst",
        std::process::id()
    ))
}

pub(crate) fn message<'a>(
    subject: &'a str,
    message_id: &'a str,
    properties: &'a [UnicodePstProperty<'a>],
) -> UnicodePstMessage<'a> {
    UnicodePstMessage {
        subject,
        sender_name: "Sender",
        sender_email: "sender@example.com",
        recipients: &[],
        body: "Body",
        html_body: None,
        message_id,
        delivery_time: 133_750_080_000_000_000,
        message_class: "IPM.Note",
        properties,
    }
}

pub(crate) fn folder(store: &Rc<UnicodeStore>, path: &[&str]) -> (NodeId, Rc<dyn Folder>) {
    let mut node = store
        .properties()
        .ipm_sub_tree_entry_id()
        .unwrap()
        .node_id();
    for name in path {
        node = find_child_folder(store.as_ref(), node, name)
            .unwrap()
            .unwrap();
    }
    let folder = store
        .open_folder(&store.properties().make_entry_id(node).unwrap())
        .unwrap();
    (node, folder)
}

/// A table's cell, by row and property.
pub(crate) fn cell(table: &dyn TableContext, row: u32, id: u16) -> Option<PropertyValue> {
    let row = table
        .rows_matrix()
        .find(|candidate| u32::from(candidate.id()) == row)?;
    let values = row.columns(table.context()).unwrap();
    let (column, value) = table
        .context()
        .columns()
        .iter()
        .zip(values)
        .find(|(column, _)| column.prop_id() == id)?;
    Some(table.read_column(&value?, column.prop_type()).unwrap())
}

/// A folder's messages, with their rows' IDs.
pub(crate) fn messages(
    store: &Rc<UnicodeStore>,
    folder: &Rc<dyn Folder>,
) -> Vec<(u32, Rc<dyn Message>)> {
    folder
        .contents_table()
        .unwrap()
        .rows_matrix()
        .map(|row| {
            let id = u32::from(row.id());
            let entry = store.properties().make_entry_id(NodeId::from(id)).unwrap();
            (id, store.open_message(&entry, None).unwrap())
        })
        .collect()
}

pub(crate) fn text(value: Option<&PropertyValue>) -> String {
    match value {
        Some(PropertyValue::Unicode(value)) => value.to_string(),
        value => panic!("not text: {value:?}"),
    }
}

#[test]
fn a_message_keeps_its_class_and_added_properties() {
    let path = temp_pst("added-properties");
    let headers = format!(
        "Received: by example.com\r\n{}",
        "X-Long: 1\r\n".repeat(600)
    );
    let categories = ["Red", "Läuft"];
    let blobs: [&[u8]; 2] = [b"one", b""];
    let unread = [
        UnicodePstProperty::tagged(0x0E07, UnicodePstValue::Integer32(0)),
        UnicodePstProperty::tagged(0x0017, UnicodePstValue::Integer32(2)),
        UnicodePstProperty::tagged(0x1090, UnicodePstValue::Integer32(2)),
        UnicodePstProperty::tagged(0x0039, UnicodePstValue::Time(133_000_000_000_000_000)),
        UnicodePstProperty::tagged(0x007D, UnicodePstValue::Unicode(&headers)),
        UnicodePstProperty::tagged(0x0071, UnicodePstValue::Binary(&[1, 2, 3])),
        UnicodePstProperty::tagged(0x0E1F, UnicodePstValue::Boolean(true)),
        UnicodePstProperty::tagged(0x6601, UnicodePstValue::Integer16(-7)),
        UnicodePstProperty::tagged(0x6602, UnicodePstValue::MultipleUnicode(&categories)),
        UnicodePstProperty::tagged(0x6603, UnicodePstValue::MultipleBinary(&blobs)),
        UnicodePstProperty::tagged(0x6604, UnicodePstValue::Floating64(0.4)),
    ];
    let first = UnicodePstMessage {
        message_class: "IPM.Note.Custom",
        ..message("Unread", "<unread@example.com>", &unread)
    };
    drop(UnicodePstFile::create(&path, &first).unwrap());
    // A message given as read, with an attachment: mfHasAttach stays the writer's.
    let read = [UnicodePstProperty::tagged(
        0x0E07,
        UnicodePstValue::Integer32(MESSAGE_FLAG_READ | 0x08),
    )];
    let attachment = UnicodePstAttachment {
        filename: "a.txt",
        mime_type: "text/plain",
        content_id: None,
        data: b"data",
        ..Default::default()
    };
    drop(
        UnicodePstFile::append_with_attachments(
            &path,
            &message("Read", "<read@example.com>", &read),
            &[attachment],
        )
        .unwrap(),
    );
    // And one that says nothing: read, as before.
    drop(UnicodePstFile::append(&path, &message("Plain", "<plain@example.com>", &[])).unwrap());

    let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
    let (inbox_node, inbox) = folder(&store, &["Inbox"]);
    let contents = inbox.contents_table().unwrap().clone();
    let columns: Vec<u16> = contents
        .context()
        .columns()
        .iter()
        .map(|column| column.prop_id())
        .collect();
    assert!(columns.contains(&0x0E07) && columns.contains(&0x001A));
    let all = messages(&store, &inbox);
    assert_eq!(all.len(), 3);
    let by_subject = |subject: &str| {
        all.iter()
            .find(|(_, m)| text(m.properties().get(0x0037)) == subject)
            .unwrap()
            .clone()
    };

    let (row, m) = by_subject("Unread");
    let p = m.properties();
    assert_eq!(text(p.get(0x001A)), "IPM.Note.Custom");
    assert!(matches!(p.get(0x0E07), Some(PropertyValue::Integer32(0))));
    assert!(matches!(p.get(0x0017), Some(PropertyValue::Integer32(2))));
    assert!(matches!(p.get(0x1090), Some(PropertyValue::Integer32(2))));
    assert!(matches!(
        p.get(0x0039),
        Some(PropertyValue::Time(133_000_000_000_000_000))
    ));
    assert!(matches!(
        p.get(0x0E06),
        Some(PropertyValue::Time(133_750_080_000_000_000))
    ));
    assert_eq!(text(p.get(0x007D)), headers);
    assert!(matches!(p.get(0x0071), Some(PropertyValue::Binary(b)) if b.buffer() == [1, 2, 3]));
    assert!(matches!(p.get(0x0E1F), Some(PropertyValue::Boolean(true))));
    assert!(matches!(p.get(0x6601), Some(PropertyValue::Integer16(-7))));
    match p.get(0x6602) {
        Some(PropertyValue::MultipleUnicode(values)) => {
            let values: Vec<String> = values
                .iter()
                .map(|value| value.to_string().trim_end_matches('\0').to_string())
                .collect();
            assert_eq!(values, categories);
        }
        value => panic!("categories: {value:?}"),
    }
    match p.get(0x6603) {
        Some(PropertyValue::MultipleBinary(values)) => {
            let values: Vec<&[u8]> = values.iter().map(|value| value.buffer()).collect();
            assert_eq!(values, blobs);
        }
        value => panic!("binaries: {value:?}"),
    }
    // MailCraft: a 64-bit float, as a task's percent complete is kept.
    assert!(matches!(p.get(0x6604), Some(PropertyValue::Floating64(f)) if *f == 0.4));
    // The contents table's row says the same, for the columns it has.
    let table = contents.as_ref();
    assert!(matches!(
        cell(table, row, 0x0E07),
        Some(PropertyValue::Integer32(0))
    ));
    assert_eq!(text(cell(table, row, 0x001A).as_ref()), "IPM.Note.Custom");
    for (id, value) in [(0x0017, 2), (0x1090, 2)] {
        if columns.contains(&id) {
            assert!(
                matches!(cell(table, row, id), Some(PropertyValue::Integer32(v)) if v == value)
            );
        }
    }
    if columns.contains(&0x0039) {
        assert!(matches!(
            cell(table, row, 0x0039),
            Some(PropertyValue::Time(133_000_000_000_000_000))
        ));
    }

    let (row, m) = by_subject("Read");
    assert!(matches!(
        m.properties().get(0x0E07),
        Some(PropertyValue::Integer32(0x19))
    ));
    assert!(matches!(
        cell(table, row, 0x0E07),
        Some(PropertyValue::Integer32(0x19))
    ));
    assert_eq!(text(m.properties().get(0x001A)), "IPM.Note");
    let (_, m) = by_subject("Plain");
    assert!(matches!(
        m.properties().get(0x0E07),
        Some(PropertyValue::Integer32(1))
    ));

    // Inbox counts its unread message, in its own properties and its parent's table.
    assert!(matches!(
        inbox.properties().get(0x3603),
        Some(PropertyValue::Integer32(1))
    ));
    assert!(matches!(
        inbox.properties().get(0x3602),
        Some(PropertyValue::Integer32(3))
    ));
    let (_, top) = folder(&store, &[]);
    let hierarchy = top.hierarchy_table().unwrap();
    let unread_cell = cell(hierarchy.as_ref(), u32::from(inbox_node), 0x3603);
    assert!(matches!(unread_cell, Some(PropertyValue::Integer32(1))));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn a_class_or_property_that_cant_be_written_is_an_error() {
    let path = temp_pst("added-properties-refused");
    drop(UnicodePstFile::create(&path, &message("First", "<first@example.com>", &[])).unwrap());
    let before = std::fs::read(&path).unwrap();
    let int = UnicodePstValue::Integer32(1);
    let bad: [&[UnicodePstProperty<'_>]; 5] = [
        &[UnicodePstProperty::tagged(0, int)],
        &[UnicodePstProperty::tagged(0x8001, int)],
        &[UnicodePstProperty::tagged(0x0E08, int)],
        &[UnicodePstProperty::tagged(
            0x001A,
            UnicodePstValue::Unicode("IPM"),
        )],
        &[
            UnicodePstProperty::tagged(0x0017, int),
            UnicodePstProperty::tagged(0x0017, int),
        ],
    ];
    for properties in bad {
        let error = UnicodePstFile::append(&path, &message("Bad", "<bad@example.com>", properties))
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
    let no_class = UnicodePstMessage {
        message_class: "",
        ..message("Bad", "<bad@example.com>", &[])
    };
    let error = UnicodePstFile::append(&path, &no_class).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_file(path).unwrap();
}

/// A meeting's organizer and attendees keep their flags and how each answered, in their rows of
/// the recipient table; a recipient given neither has no such cells. Written in a new file and
/// added to one, as the rows are made alike.
#[test]
fn recipients_keep_their_flags_and_track_status() {
    let path = temp_pst("recipient-flags");
    let recipients = [
        UnicodePstRecipient {
            name: "Organizer",
            email: "organizer@example.com",
            flags: Some(3),
            track_status: Some(0),
            ..Default::default()
        },
        UnicodePstRecipient {
            name: "Accepted",
            email: "accepted@example.com",
            flags: Some(1),
            track_status: Some(3),
            ..Default::default()
        },
        UnicodePstRecipient {
            name: "Plain",
            email: "plain@example.com",
            recipient_type: UnicodePstRecipientType::Cc,
            ..Default::default()
        },
    ];
    let meeting = UnicodePstMessage {
        recipients: &recipients,
        message_class: "IPM.Appointment",
        ..message("Meeting", "", &[])
    };
    drop(UnicodePstFile::create(&path, &meeting).unwrap());
    drop(UnicodePstFile::append(&path, &meeting).unwrap());
    let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
    let (_, inbox) = folder(&store, &["Inbox"]);
    let all = messages(&store, &inbox);
    assert_eq!(all.len(), 2);
    for (_, m) in all {
        let table = m.recipient_table().unwrap().clone();
        let rows: Vec<u32> = table.rows_matrix().map(|row| u32::from(row.id())).collect();
        let int = |row: u32, id: u16| match cell(table.as_ref(), row, id) {
            Some(PropertyValue::Integer32(n)) => Some(n),
            None => None,
            value => panic!("not an integer: {value:?}"),
        };
        let got: Vec<_> = rows
            .iter()
            .map(|row| (int(*row, 0x0C15), int(*row, 0x5FFD), int(*row, 0x5FFF)))
            .collect();
        assert_eq!(
            got,
            [
                (Some(1), Some(3), Some(0)),
                (Some(1), Some(1), Some(3)),
                (Some(2), None, None)
            ]
        );
        // The columns after them still read.
        assert_eq!(
            text(cell(table.as_ref(), rows[2], 0x39FE).as_ref()),
            "plain@example.com"
        );
        assert!(matches!(
            cell(table.as_ref(), rows[2], 0x0E0F),
            Some(PropertyValue::Boolean(true))
        ));
    }
    std::fs::remove_file(path).unwrap();
}
