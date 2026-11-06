use super::*;
use crate::extra::tests::{cell, message, temp_pst};
use crate::messaging::attachment::{AnsiAttachment, Attachment, AttachmentData};

/// An ANSI file's store, as the reader reads one.
fn ansi_store(path: &Path) -> Rc<AnsiStore> {
    AnsiStore::read(Rc::new(AnsiPstFile::open(path).unwrap())).unwrap()
}

/// The folder at `path` under the IPM subtree.
fn ansi_folder(store: &Rc<AnsiStore>, path: &[&str]) -> (NodeId, Rc<dyn Folder>) {
    let mut node = store
        .properties()
        .ipm_sub_tree_entry_id()
        .unwrap()
        .node_id();
    let any: &dyn Store = store.as_ref();
    for name in path {
        node = find_child_folder(any, node, name).unwrap().unwrap();
    }
    let folder = store
        .open_folder(&store.properties().make_entry_id(node).unwrap())
        .unwrap();
    (node, folder)
}

/// A folder's messages, as the file's own message type, in its contents table's order.
fn ansi_messages(store: &Rc<AnsiStore>, folder: &Rc<dyn Folder>) -> Vec<Rc<AnsiMessage>> {
    folder
        .contents_table()
        .unwrap()
        .rows_matrix()
        .map(|row| {
            let entry = store
                .properties()
                .make_entry_id(NodeId::from(u32::from(row.id())))
                .unwrap();
            AnsiMessage::read(store.clone(), &entry, None).unwrap()
        })
        .collect()
}

/// An 8-bit string's bytes.
fn bytes8(value: Option<&PropertyValue>) -> Vec<u8> {
    match value {
        Some(PropertyValue::String8(value)) => value.buffer().to_vec(),
        value => panic!("not an 8-bit string: {value:?}"),
    }
}

fn windows_1252(value: &str) -> Vec<u8> {
    CodePage::Windows1252.encode(value)
}

/// Every block of the ANSI file at `path` as MS-PST 2.2.2.8 keeps one: its 12-byte trailer
/// naming its size and BID, its CRC and signature right; and no two blocks or B-tree pages
/// share space, all of them before the end of the file the header gives.
pub(crate) fn check_ansi_file(path: &Path) {
    let raw = std::fs::read(path).unwrap();
    let mut file = File::open(path).unwrap();
    let header = FileHeader::read(&mut file).unwrap();
    assert_eq!(header.format(), Format::Ansi);
    let eof = header.file_eof();
    assert_eq!(eof, raw.len() as u64);
    let bbt = read_block_btree(&mut file, Format::Ansi, header.block_btree()).unwrap();
    let nbt = read_node_btree(&mut file, Format::Ansi, header.node_btree()).unwrap();
    let mut taken = Vec::new();
    for entry in bbt.entries() {
        let entry = ansi_bbt_entry(&entry).unwrap();
        let offset = entry.block().index().index() as usize;
        let size = usize::from(entry.size());
        let disk = usize::from(ndb::block::block_size(entry.size() + 12));
        let block = &raw[offset..offset + disk];
        let trailer = &block[disk - 12..];
        assert_eq!(u16::from_le_bytes([trailer[0], trailer[1]]), entry.size());
        let id = u32::from(entry.block().block());
        assert_eq!(
            u16::from_le_bytes([trailer[2], trailer[3]]),
            block_sig::compute_sig(offset as u32, id)
        );
        assert_eq!(u32::from_le_bytes(trailer[4..8].try_into().unwrap()), id);
        assert_eq!(
            u32::from_le_bytes(trailer[8..12].try_into().unwrap()),
            crate::crc::compute_crc(0, &block[..size])
        );
        taken.push((offset, offset + disk));
    }
    let pages = bbt
        .leaves
        .iter()
        .map(|(page, _)| *page)
        .chain(nbt.leaves.iter().map(|(page, _)| *page))
        .chain(bbt.inner.iter().flatten().map(|(page, _)| *page))
        .chain(nbt.inner.iter().flatten().map(|(page, _)| *page));
    for page in pages {
        let start = page.index().index() as usize;
        taken.push((start, start + PAGE_SIZE));
    }
    taken.sort();
    for pair in taken.windows(2) {
        assert!(
            pair[0].1 <= pair[1].0,
            "{:X?} overlaps {:X?}",
            pair[0],
            pair[1]
        );
    }
    assert!(taken.last().is_some_and(|last| last.1 as u64 <= eof));
}

/// The files a message holds.
fn files(message: &Rc<AnsiMessage>) -> Vec<Vec<u8>> {
    message
        .attachment_table()
        .map(|table| {
            table
                .rows_matrix()
                .map(|row| {
                    let attachment = AnsiAttachment::read(
                        message.clone(),
                        NodeId::from(u32::from(row.id())),
                        None,
                    )
                    .unwrap();
                    match attachment.data() {
                        Some(AttachmentData::Binary(value)) => value.buffer().to_vec(),
                        other => panic!("not a file: {}", other.is_some()),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

const RECIPIENTS: &[UnicodePstRecipient<'static>] = &[UnicodePstRecipient {
    name: "Zoë Recipient",
    email: "zoe@example.com",
    recipient_type: UnicodePstRecipientType::To,
    flags: None,
    track_status: None,
}];

/// A new ANSI file is in the ANSI format, its strings 8-bit, with Outlook's folders and an
/// Inbox, and the store doesn't say it takes Unicode strings.
#[test]
fn a_new_ansi_file_reads_as_one() {
    let path = temp_pst("ansi-new");
    drop(AnsiPstFile::create_empty(&path).unwrap());
    assert_eq!(Format::of_file(&path).unwrap(), Format::Ansi);
    assert!(UnicodePstFile::open(&path).is_err());
    let store = ansi_store(&path);
    if let Some(PropertyValue::Integer32(mask)) = store.properties().get(0x340D) {
        assert_eq!(mask & STORE_UNICODE_OK, 0);
    }
    assert_eq!(bytes8(store.properties().get(0x3001)), b"Empty");
    let (_, inbox) = ansi_folder(&store, &["Inbox"]);
    assert_eq!(bytes8(inbox.properties().get(0x3001)), b"Inbox");
    assert_eq!(bytes8(inbox.properties().get(0x3613)), b"IPF.Note");
    assert!(ansi_messages(&store, &inbox).is_empty());
    check_ansi_file(&path);
    // Outlook's own folders came across, their names 8-bit.
    let ipm = store
        .open_folder(&store.properties().ipm_sub_tree_entry_id().unwrap())
        .unwrap();
    let names: Vec<Vec<u8>> = ipm
        .hierarchy_table()
        .unwrap()
        .rows_matrix()
        .map(|row| {
            let folder = store
                .open_folder(
                    &store
                        .properties()
                        .make_entry_id(NodeId::from(u32::from(row.id())))
                        .unwrap(),
                )
                .unwrap();
            bytes8(folder.properties().get(0x3001))
        })
        .collect();
    assert!(names.contains(&b"Deleted Items".to_vec()));
    assert!(names.contains(&b"Inbox".to_vec()));
    std::fs::remove_file(path).unwrap();
}

/// Messages go into an ANSI file with their recipients, a body over a block, an HTML body and
/// a file of several blocks, their strings in Windows-1252 when they are all in it and in UTF-8
/// (`PidTagMessageCodepage` 65001) when not; the store and new folders are named in
/// Windows-1252. They read back with the reader, and a second batch adds to the first.
#[test]
fn messages_go_into_an_ansi_file_and_read_back() {
    let path = temp_pst("ansi-append");
    drop(AnsiPstFile::create_empty(&path).unwrap());
    drop(AnsiPstFile::set_store_display_name(&path, "Archivé").unwrap());
    let body = "Grüße aus dem Büro. ".repeat(600);
    let file = (0..20_000_u32).map(|n| (n % 251) as u8).collect::<Vec<_>>();
    let attachments = [UnicodePstAttachment {
        filename: "Résumé.bin",
        mime_type: "application/octet-stream",
        data: &file,
        ..Default::default()
    }];
    let western = UnicodePstMessage {
        recipients: RECIPIENTS,
        body: &body,
        html_body: Some("<p>Grüße</p>"),
        ..message("Café", "<cafe@example.com>", &[])
    };
    let eastern = message("日本語の件名", "<nihongo@example.com>", &[]);
    drop(
        AnsiPstFile::append_many_in_folder_with_attachments(
            &path,
            &["Projekte", "Ablage"],
            &[
                UnicodePstBatchMessage {
                    message: western,
                    attachments: &attachments,
                },
                UnicodePstBatchMessage {
                    message: eastern,
                    attachments: &[],
                },
            ],
        )
        .unwrap(),
    );
    let more = (0..40)
        .map(|_| UnicodePstBatchMessage {
            message: western,
            attachments: &attachments,
        })
        .collect::<Vec<_>>();
    drop(
        AnsiPstFile::append_many_in_folder_with_attachments(&path, &["Projekte", "Ablage"], &more)
            .unwrap(),
    );
    drop(AnsiPstFile::append_many_with_attachments(&path, &more[..1]).unwrap());

    let store = ansi_store(&path);
    assert_eq!(
        bytes8(store.properties().get(0x3001)),
        windows_1252("Archivé")
    );
    let (node, folder) = ansi_folder(&store, &["Projekte", "Ablage"]);
    assert_eq!(bytes8(folder.properties().get(0x3001)), b"Ablage");
    assert!(matches!(
        folder.properties().get(0x3602),
        Some(PropertyValue::Integer32(42))
    ));
    let messages = ansi_messages(&store, &folder);
    assert_eq!(messages.len(), 42);
    let first = &messages[0];
    let props = first.properties();
    assert_eq!(bytes8(props.get(0x0037)), windows_1252("Café"));
    assert_eq!(bytes8(props.get(0x1000)), windows_1252(&body));
    assert!(matches!(
        props.get(0x3FFD),
        Some(PropertyValue::Integer32(1252))
    ));
    match props.get(0x1013) {
        Some(PropertyValue::Binary(html)) => assert_eq!(html.buffer(), "<p>Grüße</p>".as_bytes()),
        other => panic!("no HTML: {other:?}"),
    }
    assert_eq!(files(first), std::slice::from_ref(&file));
    let recipients = first.recipient_table().unwrap();
    let row = recipients.rows_matrix().next().unwrap();
    let recipient = u32::from(row.id());
    assert!(matches!(
        cell(recipients.as_ref(), recipient, 0x3001),
        Some(PropertyValue::String8(name)) if name.buffer() == windows_1252("Zoë Recipient")
    ));
    let props = messages[1].properties();
    assert_eq!(bytes8(props.get(0x0037)), "日本語の件名".as_bytes());
    assert!(matches!(
        props.get(0x3FFD),
        Some(PropertyValue::Integer32(65001))
    ));
    for message in &messages[2..] {
        assert_eq!(
            bytes8(message.properties().get(0x0037)),
            windows_1252("Café")
        );
        assert_eq!(files(message), std::slice::from_ref(&file));
    }
    // The contents table's cells are 8-bit, in each message's code page.
    let contents = folder.contents_table().unwrap();
    let rows: Vec<u32> = contents
        .rows_matrix()
        .map(|row| u32::from(row.id()))
        .collect();
    assert!(matches!(
        cell(contents.as_ref(), rows[1], 0x0037),
        Some(PropertyValue::String8(subject)) if subject.buffer() == "日本語の件名".as_bytes()
    ));
    // The parent's hierarchy row has the new count.
    let (_, parent) = ansi_folder(&store, &["Projekte"]);
    assert!(matches!(
        cell(
            parent.hierarchy_table().unwrap().as_ref(),
            u32::from(node),
            0x3602
        ),
        Some(PropertyValue::Integer32(42))
    ));
    let (_, inbox) = ansi_folder(&store, &["Inbox"]);
    assert_eq!(ansi_messages(&store, &inbox).len(), 1);
    drop(store);
    check_ansi_file(&path);
    std::fs::remove_file(path).unwrap();
}

/// Folders are made in an ANSI file with their classes, recorded as special folders, and found
/// again.
#[test]
fn an_ansi_file_takes_folders_and_special_folders() {
    let path = temp_pst("ansi-folders");
    drop(AnsiPstFile::create_empty(&path).unwrap());
    drop(AnsiPstFile::create_folder(&path, &["Kalender"], "IPF.Appointment").unwrap());
    drop(
        AnsiPstFile::set_special_folder(&path, UnicodePstSpecialFolder::Calendar, &["Kalender"])
            .unwrap(),
    );
    drop(
        AnsiPstFile::set_special_folder(&path, UnicodePstSpecialFolder::SentItems, &["Gesendet"])
            .unwrap(),
    );
    assert_eq!(
        AnsiPstFile::special_folder(&path, UnicodePstSpecialFolder::Calendar).unwrap(),
        Some(vec!["Kalender".to_string()])
    );
    assert_eq!(
        AnsiPstFile::special_folder(&path, UnicodePstSpecialFolder::SentItems).unwrap(),
        Some(vec!["Gesendet".to_string()])
    );
    let store = ansi_store(&path);
    let (_, calendar) = ansi_folder(&store, &["Kalender"]);
    assert_eq!(
        bytes8(calendar.properties().get(0x3613)),
        b"IPF.Appointment"
    );
    std::fs::remove_file(path).unwrap();
}

/// The top folders of an ANSI file: each one's 8-bit name and how many messages it holds.
fn top_folders(path: &Path) -> Vec<(Vec<u8>, usize)> {
    let store = ansi_store(path);
    let top = store
        .open_folder(&store.properties().ipm_sub_tree_entry_id().unwrap())
        .unwrap();
    let mut out = top
        .hierarchy_table()
        .unwrap()
        .rows_matrix()
        .map(|row| {
            let entry = store
                .properties()
                .make_entry_id(NodeId::from(u32::from(row.id())))
                .unwrap();
            let folder = store.open_folder(&entry).unwrap();
            let count = folder
                .contents_table()
                .map_or(0, |table| table.rows_matrix().count());
            (bytes8(folder.properties().get(0x3001)), count)
        })
        .collect::<Vec<_>>();
    out.sort();
    out
}

/// A folder whose name Windows-1252 can't hold whole, or holds above 0x7F, is found again by
/// that name (and by the name the reader reads back), so each append goes into the one folder
/// rather than a new folder of the same written name; names written alike are one folder.
#[test]
fn an_ansi_folder_is_found_again_by_its_name() {
    let path = temp_pst("ansi-folder-names");
    drop(AnsiPstFile::create_empty(&path).unwrap());
    let one = |subject| {
        [UnicodePstBatchMessage {
            message: message(subject, "<one@example.com>", &[]),
            attachments: &[],
        }]
    };
    for name in ["Café", "Проекты", "Receipts – 2024", "Wysłane"] {
        for subject in ["First", "Second"] {
            drop(
                AnsiPstFile::append_many_in_folder_with_attachments(&path, &[name], &one(subject))
                    .unwrap(),
            );
        }
        // And by the name read back (`?` where Windows-1252 has no place, a byte to a character).
        let back = top_folders(&path)
            .into_iter()
            .map(|(name, _)| name)
            .find(|stored| *stored == windows_1252(name))
            .unwrap();
        let back: String = back.iter().map(|&byte| char::from(byte)).collect();
        drop(
            AnsiPstFile::append_many_in_folder_with_attachments(&path, &[&back], &one("Third"))
                .unwrap(),
        );
    }
    // Another name written as `Проекты` is (`???????`): that folder.
    drop(
        AnsiPstFile::append_many_in_folder_with_attachments(&path, &["Справка"], &one("Fourth"))
            .unwrap(),
    );
    // A subfolder of one, appended to twice.
    for subject in ["Fifth", "Sixth"] {
        drop(
            AnsiPstFile::append_many_in_folder_with_attachments(
                &path,
                &["Проекты", "Отчёты"],
                &one(subject),
            )
            .unwrap(),
        );
    }
    let mut want = vec![
        (b"Deleted Items".to_vec(), 0),
        (b"Inbox".to_vec(), 0),
        (windows_1252("Café"), 3),
        (b"???????".to_vec(), 4),
        (b"Receipts \x96 2024".to_vec(), 3),
        (b"Wys?ane".to_vec(), 3),
    ];
    want.sort();
    assert_eq!(top_folders(&path), want);
    let store = ansi_store(&path);
    let (_, reports) = ansi_folder(&store, &["Проекты", "Отчёты"]);
    assert_eq!(ansi_messages(&store, &reports).len(), 2);
    check_ansi_file(&path);
    std::fs::remove_file(path).unwrap();
}

/// Making an ANSI file leaves the files beside it as they were: Outlook's template it starts
/// from is written in the temporary folder, under a name no file has.
#[test]
fn a_new_ansi_file_leaves_its_neighbours_alone() {
    let path = temp_pst("ansi-neighbours");
    let mut beside = path.as_os_str().to_owned();
    beside.push(".template");
    let beside = std::path::PathBuf::from(beside);
    std::fs::write(&beside, b"the user's own").unwrap();
    drop(AnsiPstFile::create_empty(&path).unwrap());
    assert_eq!(std::fs::read(&beside).unwrap(), b"the user's own");
    check_ansi_file(&path);
    std::fs::remove_file(beside).unwrap();
    std::fs::remove_file(path).unwrap();
}

/// An ANSI file encoded as Outlook encodes its files (permute, or cyclic) is added to: its
/// blocks read decoded, its new data blocks written encoded alike.
#[test]
fn encoded_ansi_files_are_added_to() {
    for (label, method) in [
        ("permute", NdbCryptMethod::Permute),
        ("cyclic", NdbCryptMethod::Cyclic),
    ] {
        let path = temp_pst(&format!("ansi-{label}"));
        drop(AnsiPstFile::create_empty(&path).unwrap());
        encode(&path, method);
        let body = "an encoded body ".repeat(1_000);
        let input = UnicodePstMessage {
            body: &body,
            ..message("Encoded", "<encoded@example.com>", &[])
        };
        let batch = [UnicodePstBatchMessage {
            message: input,
            attachments: &[],
        }];
        drop(AnsiPstFile::append_many_with_attachments(&path, &batch).unwrap());
        drop(AnsiPstFile::append_many_in_folder_with_attachments(&path, &["Neu"], &batch).unwrap());
        let pst = AnsiPstFile::open(&path).unwrap();
        assert_eq!(pst.header().crypt_method(), method);
        let store = AnsiStore::read(Rc::new(pst)).unwrap();
        for path in [&["Inbox"][..], &["Neu"][..]] {
            let (_, folder) = ansi_folder(&store, path);
            let messages = ansi_messages(&store, &folder);
            assert_eq!(messages.len(), 1);
            assert_eq!(
                bytes8(messages[0].properties().get(0x1000)),
                body.as_bytes()
            );
        }
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.windows(16).any(|window| window == b"an encoded body "));
        std::fs::remove_file(path).unwrap();
    }
}

/// Encode every data block of the ANSI file at `path` with `method`, as Outlook would have.
fn encode(path: &Path, method: NdbCryptMethod) {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    let mut header = AnsiHeader::read(&mut file).unwrap();
    let root = FileHeader::Ansi(header.clone()).block_btree();
    let entries = collect_block_btree_entries(&mut file, Format::Ansi, root).unwrap();
    for entry in entries
        .iter()
        .filter(|entry| !entry.block().block().is_internal())
    {
        let entry = ansi_bbt_entry(entry).unwrap();
        let offset = u64::from(entry.block().index().index());
        file.seek(SeekFrom::Start(offset)).unwrap();
        let block = AnsiDataBlock::read(&mut file, entry.size(), NdbCryptMethod::None).unwrap();
        file.seek(SeekFrom::Start(offset)).unwrap();
        AnsiDataBlock::new(method, block.data().to_vec(), *block.trailer())
            .unwrap()
            .write(&mut file)
            .unwrap();
    }
    header.set_crypt_method(method);
    file.seek(SeekFrom::Start(0)).unwrap();
    header.write(&mut file).unwrap();
}

/// Each API takes its own format: a Unicode file isn't added to as an ANSI one, nor the other
/// way round, and neither is touched.
#[test]
fn each_format_is_written_through_its_own_type() {
    let ansi = temp_pst("ansi-own");
    let unicode = temp_pst("unicode-own");
    drop(AnsiPstFile::create_empty(&ansi).unwrap());
    drop(UnicodePstFile::create_empty(&unicode).unwrap());
    let (ansi_before, unicode_before) = (
        std::fs::read(&ansi).unwrap(),
        std::fs::read(&unicode).unwrap(),
    );
    let batch = [UnicodePstBatchMessage {
        message: message("Wrong", "<wrong@example.com>", &[]),
        attachments: &[],
    }];
    let wrong = |error: io::Error| error.kind() == io::ErrorKind::InvalidInput;
    assert!(UnicodePstFile::append_many_with_attachments(&ansi, &batch).is_err_and(wrong));
    assert!(AnsiPstFile::append_many_with_attachments(&unicode, &batch).is_err_and(wrong));
    assert!(UnicodePstFile::create_folder(&ansi, &["X"], "IPF.Note").is_err_and(wrong));
    assert!(AnsiPstFile::create_folder(&unicode, &["X"], "IPF.Note").is_err_and(wrong));
    assert_eq!(std::fs::read(&ansi).unwrap(), ansi_before);
    assert_eq!(std::fs::read(&unicode).unwrap(), unicode_before);
    std::fs::remove_file(ansi).unwrap();
    std::fs::remove_file(unicode).unwrap();
}

/// An ANSI file doesn't grow past its 2 GB, and a folder of one holds 65,536 items at most:
/// both are an error before anything is written past them.
#[test]
fn an_ansi_file_stops_at_its_limits() {
    let mut space = FileSpace::new(ANSI_MAX_FILE_SIZE - 4_096, Format::Ansi);
    assert!(reserve_file_range(&mut space, 512, 64).is_ok());
    let error = reserve_file_range(&mut space, 8_192, 64).unwrap_err();
    assert!(error.to_string().contains("can't grow past 2 GB"));
    let mut unicode = FileSpace::new(ANSI_MAX_FILE_SIZE - 4_096, Format::Unicode);
    assert!(reserve_file_range(&mut unicode, 8_192, 64).is_ok());

    let mut data = Vec::new();
    write_row_number(&mut data, 65_535, Format::Ansi).unwrap();
    assert_eq!(data, [0xFF, 0xFF]);
    let error = write_row_number(&mut data, 65_536, Format::Ansi).unwrap_err();
    assert!(error.to_string().contains("65,536 items at most"));
    write_row_number(&mut data, 65_536, Format::Unicode).unwrap();
}

/// Windows-1252 has the characters it has, the C1 controls its undefined bytes stand for among
/// them, and a `?` for the others; a message whose text it can't hold is written in UTF-8.
#[test]
fn strings_are_written_in_the_code_page_they_fit() {
    assert_eq!(windows_1252("aé€Ÿ\u{81}"), [b'a', 0xE9, 0x80, 0x9F, 0x81]);
    assert_eq!(windows_1252("日x"), b"?x");
    assert_eq!(
        Text::for_message(Format::Ansi, ["Grüße €"].into_iter()),
        Text::Ansi(CodePage::Windows1252)
    );
    assert_eq!(
        Text::for_message(Format::Ansi, ["ok", "Ελληνικά"].into_iter()),
        Text::Ansi(CodePage::Utf8)
    );
    assert_eq!(
        Text::for_message(Format::Unicode, ["Ελληνικά"].into_iter()),
        Text::Unicode
    );
    let categories = ["Rot", "Läuft"];
    match Text::Ansi(CodePage::Windows1252)
        .narrow(UnicodePstValue::MultipleUnicode(&categories).property_value())
    {
        PropertyValue::MultipleString8(values) => {
            let values: Vec<_> = values.iter().map(|value| value.buffer().to_vec()).collect();
            assert_eq!(values, [b"Rot\0".to_vec(), windows_1252("Läuft\0")]);
        }
        other => panic!("not narrowed: {other:?}"),
    }
}
