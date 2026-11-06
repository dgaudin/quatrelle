use super::*;
use crate::extra::tests::{cell, folder, message, messages, temp_pst, text};
use crate::messaging::attachment::{Attachment, AttachmentData, UnicodeAttachment};

/// A message's attachments as the reader reads them, in the attachment table's order.
fn attachments_of(message: &Rc<UnicodeMessage>) -> Vec<Rc<UnicodeAttachment>> {
    message
        .attachment_table()
        .map(|table| {
            table
                .rows_matrix()
                .map(|row| {
                    UnicodeAttachment::read(
                        message.clone(),
                        NodeId::from(u32::from(row.id())),
                        None,
                    )
                    .unwrap()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The file an attachment holds.
fn bytes(attachment: &UnicodeAttachment) -> Vec<u8> {
    match attachment.data() {
        Some(AttachmentData::Binary(value)) => value.buffer().to_vec(),
        other => panic!("not a file: {}", other.is_some()),
    }
}

/// The first message in the Inbox, as the file's own message type.
fn first_message(path: &std::path::Path) -> Rc<UnicodeMessage> {
    let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(path).unwrap())).unwrap();
    let (_, inbox) = folder(&store, &["Inbox"]);
    let (row, _) = messages(&store, &inbox)[0];
    UnicodeMessage::read(
        store.clone(),
        &store.properties().make_entry_id(NodeId::from(row)).unwrap(),
        None,
    )
    .unwrap()
}

/// A message with `attachments`, attached to another one.
fn attached<'a>(
    filename: &'a str,
    message: UnicodePstMessage<'a>,
    attachments: Vec<UnicodePstAttachment<'a>>,
) -> UnicodePstAttachment<'a> {
    UnicodePstAttachment {
        filename,
        content: UnicodePstAttachmentContent::Message(Box::new(UnicodePstEmbeddedMessage {
            message,
            attachments,
        })),
        ..Default::default()
    }
}

/// A message attached three deep, a file at the bottom, beside a file and with properties of
/// the attachment's own, reads back with the crate's reader as Outlook keeps it: method 5, its
/// PtypObject naming the attached message, which has its own recipients, named properties and
/// attachments.
#[test]
fn attached_messages_are_written_as_outlook_keeps_them() {
    let path = temp_pst("attached-messages");
    let recipients = [UnicodePstRecipient {
        name: "Ana",
        email: "ana@example.com",
        recipient_type: UnicodePstRecipientType::To,
        flags: Some(1),
        track_status: Some(4),
    }];
    let categories = ["Blue"];
    let keywords = [UnicodePstProperty::named(
        PS_PUBLIC_STRINGS,
        UnicodePstName::String("Keywords"),
        UnicodePstValue::MultipleUnicode(&categories),
    )];
    let hidden = [
        UnicodePstProperty::tagged(0x7FFE, UnicodePstValue::Boolean(true)),
        UnicodePstProperty::tagged(0x7FFD, UnicodePstValue::Integer32(4)),
    ];
    let bottom = UnicodePstAttachment {
        filename: "bottom.bin",
        mime_type: "application/octet-stream",
        data: b"at the bottom",
        ..Default::default()
    };
    let third = attached(
        "Third",
        message("Third", "<third@example.com>", &[]),
        vec![bottom],
    );
    let second = attached(
        "Second",
        UnicodePstMessage {
            message_class: "IPM.Note.SMIME.MultipartSigned",
            ..message("Second", "<second@example.com>", &[])
        },
        vec![third],
    );
    let first = attached(
        "First",
        UnicodePstMessage {
            recipients: &recipients,
            ..message("First", "<first@example.com>", &keywords)
        },
        vec![second],
    );
    let file = UnicodePstAttachment {
        filename: "beside.txt",
        mime_type: "text/plain",
        data: b"beside",
        properties: &hidden,
        ..Default::default()
    };
    drop(
        UnicodePstFile::create_with_attachments(
            &path,
            &message("Top", "<top@example.com>", &[]),
            &[file, first],
        )
        .unwrap(),
    );

    let top = first_message(&path);
    assert!(matches!(
        top.properties().get(0x0E1B),
        Some(PropertyValue::Boolean(true))
    ));
    let table = top.attachment_table().unwrap().clone();
    let rows: Vec<u32> = table.rows_matrix().map(|row| u32::from(row.id())).collect();
    assert!(matches!(
        cell(table.as_ref(), rows[0], 0x3705),
        Some(PropertyValue::Integer32(1))
    ));
    assert!(matches!(
        cell(table.as_ref(), rows[1], 0x3705),
        Some(PropertyValue::Integer32(5))
    ));
    let top_files = attachments_of(&top);
    assert_eq!(bytes(&top_files[0]), b"beside");
    let p = top_files[0].properties();
    assert!(matches!(p.get(0x7FFE), Some(PropertyValue::Boolean(true))));
    assert!(matches!(p.get(0x7FFD), Some(PropertyValue::Integer32(4))));

    let level = &top_files[1];
    assert_eq!(level.properties().attachment_method().unwrap(), 5);
    assert_eq!(text(level.properties().get(0x3001)), "First");
    let Some(PropertyValue::Object(object)) = level.properties().get(0x3701) else {
        panic!("no object");
    };
    // A message ID no other node has, as Outlook gives one: past the top message's.
    assert_eq!(object.node().id_type().unwrap(), NodeIdType::NormalMessage);
    assert!(
        object.node().index()
            > top.properties().get(0x67F2).map_or(0, |id| match id {
                PropertyValue::Integer32(id) => NodeId::from(*id as u32).index(),
                _ => u32::MAX,
            })
    );
    assert!(matches!(level.properties().get(0x0E20), Some(PropertyValue::Integer32(n)) if *n > 0));
    let first = level.embedded_message().unwrap();
    assert_eq!(text(first.properties().get(0x0037)), "First");
    assert_eq!(first.recipient_table().unwrap().rows_matrix().count(), 1);
    // MailCraft: an attached message's recipients keep their flags and track status too.
    let people = first.recipient_table().unwrap().clone();
    let row = people
        .rows_matrix()
        .map(|row| u32::from(row.id()))
        .next()
        .unwrap();
    assert!(matches!(
        cell(people.as_ref(), row, 0x5FFF),
        Some(PropertyValue::Integer32(4))
    ));
    // Its named property, given an ID from the file's map.
    let blue = first.properties().iter().any(|(id, value)| {
        *id >= 0x8000
            && matches!(value, PropertyValue::MultipleUnicode(v)
                if v.len() == 1 && v[0].to_string().starts_with("Blue"))
    });
    assert!(blue);

    let second = attachments_of(&first).remove(0).embedded_message().unwrap();
    assert_eq!(
        text(second.properties().get(0x001A)),
        "IPM.Note.SMIME.MultipartSigned"
    );
    let third = attachments_of(&second)
        .remove(0)
        .embedded_message()
        .unwrap();
    assert_eq!(text(third.properties().get(0x0037)), "Third");
    let files = attachments_of(&third);
    assert_eq!(bytes(&files[0]), b"at the bottom");
    assert_eq!(text(files[0].properties().get(0x3707)), "bottom.bin");

    // Appended alike, beside it.
    let again = attached(
        "Again",
        message("Again", "<again@example.com>", &[]),
        vec![],
    );
    drop(
        UnicodePstFile::append_with_attachments(
            &path,
            &message("Next", "<next@example.com>", &[]),
            &[again],
        )
        .unwrap(),
    );
    let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
    let (_, inbox) = folder(&store, &["Inbox"]);
    assert_eq!(messages(&store, &inbox).len(), 2);
    std::fs::remove_file(path).unwrap();
}

/// A message nested deeper than [`MAX_EMBEDDED_DEPTH`], bytes given twice, or an attachment's
/// own property that is named, the writer's own or given twice is an error before the file is
/// touched.
#[test]
fn attachments_that_cant_be_written_are_an_error() {
    let path = temp_pst("attached-errors");
    drop(UnicodePstFile::create(&path, &message("Kept", "<kept@example.com>", &[])).unwrap());
    let before = std::fs::read(&path).unwrap();
    let nested = |depth: usize| {
        let mut inner = Vec::new();
        for _ in 0..depth {
            inner = vec![attached(
                "Inner",
                message("Inner", "<inner@example.com>", &[]),
                inner,
            )];
        }
        inner
    };
    let named = [UnicodePstProperty::named(
        PS_PUBLIC_STRINGS,
        UnicodePstName::String("Keywords"),
        UnicodePstValue::Integer32(1),
    )];
    let owned = [UnicodePstProperty::tagged(
        0x3705,
        UnicodePstValue::Integer32(1),
    )];
    let twice = [
        UnicodePstProperty::tagged(0x7FFE, UnicodePstValue::Boolean(true)),
        UnicodePstProperty::tagged(0x7FFE, UnicodePstValue::Boolean(false)),
    ];
    let big = "x".repeat(4000);
    let too_big = [UnicodePstProperty::tagged(
        0x3716,
        UnicodePstValue::Unicode(&big),
    )];
    let mut cases = vec![nested(MAX_EMBEDDED_DEPTH + 1)];
    for properties in [&named[..], &owned, &twice, &too_big] {
        cases.push(vec![UnicodePstAttachment {
            filename: "a.txt",
            properties,
            ..Default::default()
        }]);
    }
    let mut beside = attached(
        "Inner",
        message("Inner", "<inner@example.com>", &[]),
        vec![],
    );
    beside.data = b"two";
    cases.push(vec![beside]);
    for attachments in cases {
        let err = UnicodePstFile::append_with_attachments(
            &path,
            &message("Bad", "<bad@example.com>", &[]),
            &attachments,
        )
        .err()
        .unwrap();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    // As deep as the limit goes is written.
    drop(
        UnicodePstFile::append_with_attachments(
            &path,
            &message("Deep", "<deep@example.com>", &[]),
            &nested(MAX_EMBEDDED_DEPTH),
        )
        .unwrap(),
    );
    let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
    let (_, inbox) = folder(&store, &["Inbox"]);
    let (row, _) = messages(&store, &inbox)[1];
    let mut top = UnicodeMessage::read(
        store.clone(),
        &store.properties().make_entry_id(NodeId::from(row)).unwrap(),
        None,
    )
    .unwrap();
    let mut depth = 0;
    while let Some(attachment) = attachments_of(&top).into_iter().next() {
        top = attachment.embedded_message().unwrap();
        depth += 1;
    }
    assert_eq!(depth, MAX_EMBEDDED_DEPTH);
    std::fs::remove_file(path).unwrap();
}

/// `size` bytes of a pattern no two blocks share, made as they are read.
struct Repeating(u64);

impl UnicodePstAttachmentSource for Repeating {
    fn size(&self) -> u64 {
        self.0
    }

    fn open(&self) -> io::Result<Box<dyn Read + '_>> {
        Ok(Box::new(Pattern { at: 0, end: self.0 }))
    }
}

struct Pattern {
    at: u64,
    end: u64,
}

fn pattern(at: u64) -> u8 {
    (at.wrapping_mul(2_654_435_761) >> 13) as u8
}

impl Read for Pattern {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf.len().min((self.end - self.at) as usize);
        for (i, b) in buf[..n].iter_mut().enumerate() {
            *b = pattern(self.at + i as u64);
        }
        self.at += n as u64;
        Ok(n)
    }
}

/// A source that gives `given` bytes and says it has `said`.
struct Lying {
    said: u64,
    given: u64,
}

impl UnicodePstAttachmentSource for Lying {
    fn size(&self) -> u64 {
        self.said
    }

    fn open(&self) -> io::Result<Box<dyn Read + '_>> {
        Ok(Box::new(Pattern {
            at: 0,
            end: self.given,
        }))
    }
}

/// A file read from a source past an XBLOCK's reach (1,021 blocks, 8.3 MB) is written as it is
/// read, an XXBLOCK over its XBLOCKs: no more than a few MB of blocks wait to be written at once,
/// where its bytes given whole wait all together. It reads back byte for byte. A source that
/// gives less or more than it said is an error, and the file keeps what it had.
#[test]
fn a_file_from_a_source_is_written_as_it_is_read() {
    const SIZE: u64 = 20 << 20;
    let path = temp_pst("attached-source");
    let source = Repeating(SIZE);
    let file = UnicodePstAttachment {
        filename: "big.bin",
        mime_type: "application/octet-stream",
        content: UnicodePstAttachmentContent::Source(&source),
        ..Default::default()
    };
    MOST_PENDING.with(|most| most.set(0));
    drop(
        UnicodePstFile::create_with_attachments(
            &path,
            &message("Big", "<big@example.com>", &[]),
            &[file],
        )
        .unwrap(),
    );
    let most = MOST_PENDING.with(|most| most.get());
    assert!(most < 4 << 20, "{most} bytes waited");

    let top = first_message(&path);
    let files = attachments_of(&top);
    assert!(
        matches!(files[0].properties().get(0x0E20), Some(PropertyValue::Integer32(n)) if *n as u64 > SIZE)
    );
    let read = bytes(&files[0]);
    assert_eq!(read.len() as u64, SIZE);
    assert!(read
        .iter()
        .enumerate()
        .all(|(i, b)| *b == pattern(i as u64)));
    drop((files, top));

    // The same bytes given whole wait to be written together.
    let whole: Vec<u8> = (0..SIZE).map(pattern).collect();
    MOST_PENDING.with(|most| most.set(0));
    drop(
        UnicodePstFile::append_with_attachments(
            &path,
            &message("Whole", "<whole@example.com>", &[]),
            &[UnicodePstAttachment {
                filename: "whole.bin",
                data: &whole,
                ..Default::default()
            }],
        )
        .unwrap(),
    );
    assert!(MOST_PENDING.with(|most| most.get()) as u64 >= SIZE);

    for (said, given) in [(100_000, 99_999), (100_000, 100_001)] {
        let lying = Lying { said, given };
        let before = std::fs::read(&path).unwrap();
        let err = UnicodePstFile::append_with_attachments(
            &path,
            &message("Lying", "<lying@example.com>", &[]),
            &[UnicodePstAttachment {
                filename: "lying.bin",
                content: UnicodePstAttachmentContent::Source(&lying),
                ..Default::default()
            }],
        )
        .err()
        .unwrap();
        assert!(err.to_string().contains("source"), "{err}");
        // The blocks written went to free space: what the file had still reads.
        assert_eq!(std::fs::read(&path).unwrap()[..0x200], before[..0x200]);
        let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
        let (_, inbox) = folder(&store, &["Inbox"]);
        assert_eq!(messages(&store, &inbox).len(), 2);
    }
    std::fs::remove_file(path).unwrap();
}
