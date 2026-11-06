# outlook-pst-rw for MailCraft

This branch, `craft/rw-1.2.14`, is Dong Shin's Outlook data file writer,
[outlook-pst-rw](https://crates.io/crates/outlook-pst-rw) 1.2.14 (the tag `outlook-pst-rw-v1.2.14`,
commit `4799f5d0`, of [dongs0104/outlook-pst-rs](https://github.com/dongs0104/outlook-pst-rs), MIT,
`LICENSE`), with the fixes below. That crate is itself a fork of Microsoft's reader,
[outlook-pst](https://crates.io/crates/outlook-pst): it keeps its own copy of the reader's modules
and the reader's library name, `outlook_pst`, so the reader's fixes from this repository's
`craft/1.2.0` branch are carried over here as well. [MailCraft](https://github.com/ZIDAN44/mailcraft)
reads and writes `.pst` files with this crate alone (`craft/1.2.0`, which it read with before, is
superseded by this branch), building it through `[patch.crates-io]` in its `Cargo.toml` and in
`fuzz/Cargo.toml`, pinned to a revision of this branch; its `cargo xtask ci` checks that both pin
the same one.

Only `crates/pst` changes. Each fix is its own commit, follows MailCraft's rules all the same (no
new panics, checked sizes), and is proved by the tests beside it (`cargo test -p outlook-pst-rw`).

To move to a newer release: fetch `dongs0104`'s tags
(`git remote add dongs0104 https://github.com/dongs0104/outlook-pst-rs.git`,
`git fetch dongs0104 --tags`), make `craft/rw-<version>` from the release's tag
(`outlook-pst-rw-v<version>`), cherry-pick this branch's commits onto it, dropping the fixes the
release already has, and pin MailCraft to it. Never rewrite a pushed branch: MailCraft's history
pins its revisions. Drop the fork once a release has all of the fixes.

## Fixes

Every changed line is marked `MailCraft:` in a comment, with a test beside it. Fixes 1 to 8 are
the reader's, from `craft/1.2.0` (commits `560024b`, `4832e88`, `33c5231`, `647f6fd`, `38765ba`,
`b3a2ed1`), cherry-picked with their authors onto the writer's copy of the reader; 9 and up
are the writer's own.

1. **Reads are capped** (`ndb/block.rs`, `ndb/page.rs`, `ndb/read_write.rs`, `ltp/heap.rs`,
   `ltp/prop_context.rs`, `ltp/table_context.rs`, `messaging/message.rs`, `messaging/attachment.rs`,
   `messaging/store.rs`, `messaging/folder.rs`, `messaging/named_prop.rs`, `messaging/search.rs`,
   `lib.rs`):
   - every data tree is read up to the size its XBLOCK declares: `DataTree::reader` stops there,
     and `blocks()` and the reader's list of blocks stop naming blocks once that much is named,
     so blocks named over and over (an XXBLOCK's thousand entries naming one XBLOCK whose
     thousand name one block) can't make a value, a heap or a table's rows bigger than that;
   - `UnicodePstFile::set_max_value_size` (and the Ansi twin) sets a limit for the store's, a
     folder's, the name-to-id map's, a message's and an attachment's values
     (`PropertyContextReadWrite::read_property_capped`; nothing in the crate calls the uncapped
     `read_property` any more), a table's column
     kept in a subnode and its row matrix, `read_block` and the search update queue
     (`DataTree::read_capped`): one declared bigger is an error before any of it is read;
     MailCraft sets 256 MB. A heap that declares more than 2^16 blocks can hold is an error;
   - a data tree is level 1 or 2 and a subnode tree level 0 or 1, each block points one level
     down, and so does each node and block B-tree page (`RootBTreePage::find_entry`), so a tree
     that points back at itself is an error, not a stack overflow;
   - an intermediate block's entry count is checked in `u32`, and a block smaller than its header
     is an error, where both overflowed `u16`.

   Tests: `ndb::block::tests`.
2. **`PtypObject` reads** (`ltp/prop_type.rs`): `PropertyType::try_from` had no arm for `0x000D`,
   though the enum and the record reader have it, so an attached message's 0x3701 didn't parse.
   Test: `ltp::prop_type::tests`.
3. **A BTH record that doesn't parse is skipped** (`ltp/tree.rs`): each leaf record is read from
   its own fixed-size slot (key and value sizes from the BTH header), and one that fails is left
   out with a warning. The scan used to stop there, so every property after it was lost (an
   attachment kept 0x0E20 and 0x3001 and lost its method). A BTH that yields more records than
   its key size has keys (65,536 for a property context), or more than 2^24, is an error, and
   so is one whose index records, counted over all its levels (the header may declare 255),
   come to more than that: records that point back up the tree can't keep the walk going.
   Test: `ltp::tree::tests`.
4. **The object an attachment holds is looked up in its own subnode tree**
   (`messaging/attachment.rs`, `AttachmentInner::object_node`), where MS-PST keeps it, then in the
   message's, where this crate looked; for attached messages and OLE storage alike. An own tree
   that can't be read is passed over with a warning.
5. **An attached message is read once the file's lock is let go** (`messaging/attachment.rs`):
   `read_embedded` locks the file again, and `std::sync::Mutex` isn't re-entrant, so the first
   attached message hung the reader for good.
6. **A by-value attachment with no value reads as a zero-byte file** (`messaging/attachment.rs`):
   such a file is kept as 0x3701 with HNID 0, which reads as `PtypNull`, and was an error.
7. **`UnicodeAttachment::embedded_message()` and `AnsiAttachment::embedded_message()`** hand back
   the attached message as the file's own message type (kept beside the `Rc<dyn Message>` in
   `AttachmentData::Message`), so its own attachments read with `UnicodeAttachment::read`.
8. **The root folder's hierarchy table reads** (`messaging/store.rs`): upstream's fix
   (microsoft/outlook-pst-rs#55, commit `d0f9f001`) is already in outlook-pst-rw 1.2.14, so
   only its test comes over from `craft/1.2.0`. `Store::root_hierarchy_table` held the file's
   lock while `TableContextReadWrite::read` locked it again, so every call hung for good.
   Test: `messaging::store::tests` (fails on its time limit without the fix).
9. **The writer reads a file it adds to as the reader does** (`lib.rs`, `ltp/heap.rs`,
   `ndb/block.rs`; with the reader's limit in force since fix 12):
   - its own walks of a data tree (`data_tree_leaves`, `read_data_block`) read data blocks with
     the file's `bCryptMethod`, where they read them as if unencoded, take only XBLOCKs over
     leaves and XXBLOCKs over XBLOCKs, and stop naming leaves at the size the top block
     declares, like `DataTree::blocks`; a tree that names itself was a stack overflow;
   - its walk of the node and block B-trees (`BTreeLayout::read`) takes each page a level below
     the one that names it and each page once, where one page named twice was listed twice and
     pages that named more pages at their own level were walked 500^8 times;
   - a block size no block can have (over 8,192 bytes with its trailer) is an error in
     `DataTree::read` and `SubNodeTree::read`, where `block_size` asserted and the sum
     overflowed `u16`;
   - a heap allocation placed past its block is an error (`HeapNode::find_entry`,
     `writer_pages`), and so are a table row too short for its ID and a column past its row
     (`encode_table_row`, `patch_table_row`), an allocation map bit before the first map or
     past its own map (`mark_page_allocation`, `mark_block_allocation`), a row matrix with no
     blocks (`append_table_rows`) and an end of file before the first map: each one panicked
     or overflowed;
   - `internal_block_children` returns an error where it had `unreachable!`, and the row
     index reads lose their `unwrap`s.

   Tests: `existing_file_tests`, `ltp::heap::tests`.
10. **A permute- or cyclic-encoded file is added to** (`lib.rs`): files Outlook makes are
    permute-encoded (`bCryptMethod` 0x01) by default, and `append*`, `create_*_in_folder` and
    `set_store_display_name` refused every file that wasn't unencoded. Their new data blocks
    are now written encoded with the file's method (`BlockWriter`, `write_new_block`; cyclic
    is keyed on each block's ID), and only data blocks: XBLOCKs, XXBLOCKs, SLBLOCKs and
    SIBLOCKs never are (MS-PST 5.1, 5.2). Existing blocks are read decoded (fix 9). Files the
    writer creates stay unencoded.

    Test: `create_tests::appends_to_permute_and_cyclic_encoded_unicode_psts` adds to Outlook's
    own fixture as Outlook made it (permute) and re-encoded cyclic: a renamed store, a message
    with an 8 KB+ body and a 20,000-byte attachment, a batch of 40 that appends the contents
    table's rows in place, and a new nested folder, all read back with the crate's reader; no
    data block's raw bytes show the body's text.
11. **A damaged allocation map is not trusted** (`lib.rs`, `FileSpace::read`): new blocks and
    pages go where the file's allocation map marks space free, and the map was taken whenever
    the header called it valid, checking only each page's type byte. A map page now has to read
    as the reader reads one (type and its repeat, CRC, a zero signature, its own offset as its
    ID), and no space it marks free may hold a block the block B-tree lists, a page of either
    B-tree or a map page; otherwise the map is treated as invalid and new blocks go at the end
    of the file. A map with live blocks marked free put new blocks over them: the append
    succeeded and old mail no longer read. Outlook's own maps still pass, so their free space
    is still reused.

    Test: `create_tests::a_damaged_allocation_map_is_not_trusted` (Outlook's fixture after 60
    appends, its last map page marking live blocks free with a matching or a stale CRC; every
    message still reads after one more append).
12. **A file the writer adds to is read within a limit** (`lib.rs`, `open_to_add`): the writer
    reads the file it adds to through the reader's capped paths (fix 1), but the limit was
    `u64::MAX` and nothing set it, so a damaged file could still have it build a value, a
    table's rows or a column kept in a subnode of whatever size the file declared, up to 4 GiB
    from a few KB of blocks named over and over. `append*`, `create_*_in_folder` and
    `set_store_display_name` now open it with `ADD_MAX_VALUE_SIZE` (256 MB, MailCraft's own
    limit) set; anything declared bigger is an error before any of it is read.

    Test: `create_tests::a_file_added_to_is_read_within_the_limit` (with the limit lowered on
    the test's thread, an append to a folder whose tables hold more is an error and leaves the
    file as it was).

13. **A message has its own class and any tagged property** (`extra.rs`, `lib.rs`):
    `UnicodePstMessage` takes `message_class` (`PR_MESSAGE_CLASS`, which was always `IPM.Note`)
    and `properties`, tagged properties of the types `UnicodePstValue` has (16- and 32-bit
    integers, booleans, times, strings, binaries, and multi-valued strings and binaries) that
    replace the writer's own of the same ID. They go in the message's property context and in
    the cells of its folder's contents table for the columns that table has (`PR_MESSAGE_FLAGS`,
    `PR_IMPORTANCE`, `PR_CLIENT_SUBMIT_TIME`…). `PR_MESSAGE_FLAGS`, always 1 (read), is the
    one given, its `mfHasAttach` kept as the message is; a message that goes in unread counts in
    its folder's `PR_CONTENT_UNREAD` (0x3603) and in that folder's row of its parent's
    hierarchy table, beside the content count. An empty class, an ID of 0 or from 0x8000, one
    the writer works out itself (class, size, has-attachments, row ID and version) or one given
    twice is an `InvalidInput` error before the file is touched.

    Tests: `extra::tests`.

14. **Named properties** (`extra/named.rs`, `extra.rs`, `lib.rs`): an added property's key can
    be `UnicodePstPropertyKey::Named(set, name)`, a property set's GUID and a LID or a string
    (`UnicodePstName`). Before a batch is written, each name is looked up in the file's
    name-to-ID map (`NID_NAME_TO_ID_MAP`, MS-PST 2.4.7) and the ones it doesn't have are added:
    the set to `PidTagNameidStreamGuid` unless it is PS_MAPI or PS_PUBLIC_STRINGS, a string to
    `PidTagNameidStreamString` (its size, UTF-16, padding to 4 bytes), a `NAMEID` to
    `PidTagNameidStreamEntry` with the next `wPropIdx`, and the same record, with a string's CRC
    in place of its offset, to its hash bucket (`0x1000` + (LID or CRC XOR
    `(wGuid << 1) | N`) mod `PidTagNameidBucketCount`). The property is then written with ID
    `0x8000 + wPropIdx`, in the message and in a contents table column of that ID. Names the map
    has keep their IDs; the map's node gets a new property context only when one is added. An
    empty name, one over 1,024 UTF-16 units or a name given twice is an `InvalidInput` error, a
    full map (`wPropIdx` past 0x7FFE) an `InvalidData` one.

    Tests: `extra::named::tests` (Outlook's own map in `examples/Empty.pst` is hashed as the
    writer hashes; names added to it and found again by later appends, read back with the
    reader).

15. **Folders' container classes and the special folders a file records** (`extra/folders.rs`,
    `lib.rs`): folders were always made `IPF.Note`. `UnicodePstFile::create_folder` makes the
    missing folders of a path, the last with the `PR_CONTAINER_CLASS` given (the others, and
    every folder `append*_in_folder` makes, `IPF.Note`), in the folder and in its row of the
    parent's hierarchy table; folders already there keep theirs. `set_special_folder` records a
    folder (made when missing) as one of `UnicodePstSpecialFolder` (MS-OXOSFLD 2.2): Deleted
    Items, Sent Items and the Outbox in the store's `PidTagIpmWastebasketEntryId`,
    `PidTagIpmSentMailEntryId` and `PidTagIpmOutboxEntryId`; Drafts in `PidTagIpmDraftsEntryId`
    and Junk Email as the fifth value of `PidTagAdditionalRenEntryIds`, the others kept, on the
    root folder, the IPM subtree and the Inbox. `special_folder` reads which folder a file
    records, as names from the IPM subtree, so a caller adding to a file can use the ones it
    has; `default_name` gives Outlook's name for a new file's.

    Tests: `extra::folders::tests` (Outlook's empty file records Deleted Items, and keeps it and
    the other `PidTagAdditionalRenEntryIds` values when Junk Email is recorded).

16. **Attached messages, and an attachment's own properties** (`extra/attach.rs`, `extra/named.rs`,
    `lib.rs`): every attachment was a file (`afByValue`). An attachment's `content` can now be
    `UnicodePstAttachmentContent::Message`, a message with its own attachments, written as
    Outlook keeps one (MS-OXCMSG 2.2.2.9, MS-PST 2.4.6.2): `PidTagAttachMethod` 5,
    `PidTagAttachDataObject` a `PtypObject` naming the message's node and its size, the message
    (its property context, recipient table 0x692, attachment table 0x671, attachments and values
    kept in subnodes) in the attachment's own subnode tree under that node, and its subject as
    the attachment's `PidTagDisplayName`; the attachment table's row has method 5 and no file
    name. Like Outlook, each attached message's node has a message ID no other node in the file
    has (`NID_TYPE_NORMAL_MESSAGE`, after the batch's own messages), and the file's next message
    ID is moved past them. Attached messages nest up to `MAX_EMBEDDED_DEPTH` (32); their classes,
    tagged and named properties are written as a message's (their names added to the file's map
    with the batch's). An attachment's `properties` are tagged properties of its own
    (`PidTagAttachmentHidden` 0x7FFE, `PidTagAttachmentFlags` 0x7FFD…) in place of the writer's
    of the same ID. Nesting past the limit, bytes given beside a message, an added property that
    is named, the writer's own (size, value, method), given twice or too big for the
    attachment's one heap page is an `InvalidInput` error before the file is touched, and so is
    an attachment whose property context is bigger than a block, which was written as a block
    no reader takes.

    Tests: `extra::attach::tests` (a message attached three deep with a file at the bottom,
    beside a file with properties of its own, read back with `UnicodeAttachment::embedded_message`;
    as deep as the limit and one past it).
17. **A file read from a source as it is written** (`extra/attach.rs`, `lib.rs`): an
    attachment's bytes were a slice, so a batch held every file it wrote whole, and the writer
    held each file's blocks again until its message was done. `UnicodePstAttachmentContent::Source`
    gives a file by a `UnicodePstAttachmentSource`, its size and a reader opened as it is
    written: its blocks go to the file 256 at a time (2 MB) as they are read, and its data tree,
    an XXBLOCK over XBLOCKs past 1,021 blocks, is up to 2 GiB less 1 MB (`PidTagAttachSize` is a
    signed 32-bit integer). A source that gives less or more than its size is an error, and the
    file keeps what it had (the blocks written are in space the file doesn't count as taken).

    Test: `extra::attach::tests::a_file_from_a_source_is_written_as_it_is_read` (20 MB through
    an XXBLOCK with no more than 4 MB of blocks waiting at once, where the same bytes given whole
    wait all together; read back byte for byte; sources that say more or less than they give).
18. **The Inbox is found by what it records** (`extra/folders.rs`): Drafts and Junk Email are
    recorded on the root folder, the IPM subtree and the Inbox, and the Inbox was the folder
    named "Inbox". In a file Outlook made in another language ("Posteingang", "Boîte de
    réception") there is none, so they were recorded on the other two alone. The Inbox is now
    the folder at the top of the IPM subtree that records what MS-OXOSFLD 2.2 records on the
    Inbox alone (the Calendar's, Contacts', Journal's, Notes', Tasks' and Drafts' entry IDs, or
    `PidTagAdditionalRenEntryIds`), whatever its name; the folder named Inbox when none does.

    Test: `extra::folders::tests::an_inbox_by_another_name_is_found_by_what_it_records` (Drafts
    and Junk Email recorded on a "Posteingang" that records the Calendar, the values it had kept,
    and not on a folder named Inbox beside it).
19. **A folder listed under itself is looked through once** (`extra/folders.rs`): `special_folder`
    finds a folder's names by walking the hierarchy down from the IPM subtree, up to 64 deep, and
    looked through a folder once per path to it. A damaged hierarchy table that lists a folder
    under itself twice made that 2^64 looks, so reading a damaged file's special folders never
    ended. Each folder is now looked through once.

    Test: `extra::folders::tests::a_folder_listed_under_itself_is_looked_through_once` (a folder
    whose hierarchy table names it twice, in its row index and its rows: looking for a folder that
    isn't there ends; without the fix it ran on past a minute).
20. **64-bit floats** (`extra.rs`): `UnicodePstValue` had no `PtypFloating64`, so a task's
    `PidLidPercentComplete` (a fraction of 1, MS-OXOTASK 2.2.2.2.3) couldn't be written.
    `UnicodePstValue::Floating64` writes one, in the message and in a contents table column of
    that type.

    Test: `extra::tests::a_message_keeps_its_class_and_added_properties` (0.4 read back).
21. **The Calendar, Contacts, the Journal, Notes and Tasks as special folders**
    (`extra/folders.rs`): `UnicodePstSpecialFolder` had the mail folders alone, so a file the
    writer made had nowhere Outlook would take for its contacts, notes or tasks. `Calendar`,
    `Contacts`, `Journal`, `Notes` and `Tasks` are recorded as MS-OXOSFLD 2.2 records them, in
    `PidTagIpmAppointmentEntryId`, `PidTagIpmContactEntryId`, `PidTagIpmJournalEntryId`,
    `PidTagIpmNoteEntryId` and `PidTagIpmTaskEntryId` on the root folder, the IPM subtree and the
    Inbox, and found by `special_folder` as the others are. `container_class` gives each role's
    `PR_CONTAINER_CLASS` (`IPF.Appointment`, `IPF.Contact`, `IPF.Journal`, `IPF.StickyNote`,
    `IPF.Task`; `IPF.Note` for the mail ones), and `set_special_folder` makes a missing folder with
    it; a folder already there keeps its own.

    Test: `extra::folders::tests::pim_folders_are_made_with_their_class_and_recorded`.
22. **A file with no message** (`lib.rs`): a new file was always made with its first message,
    in folders made `IPF.Note` on the way, so a file whose first item is a contact had its
    Contacts folder made as a folder of mail. `UnicodePstFile::create_empty` makes Outlook's empty
    file, unencoded, with an empty Inbox, as `create*` start from; `create_folder` and the
    `append*` calls go on from there.

    Test: `extra::folders::tests::an_empty_file_takes_a_folder_of_its_own_class_first`.
23. **A recipient's flags and track status** (`lib.rs`): a recipient row had its type, names and
    addresses alone, so a meeting's organizer couldn't be told from its attendees, nor what each
    attendee answered. `UnicodePstRecipient` takes `flags` (`PidTagRecipientFlags`, 0x5FFD:
    `recipSendable`, `recipOrganizer`…) and `track_status` (`PidTagRecipientTrackStatus`, 0x5FFF:
    accepted, tentative, declined, not answered), each in a column of the recipient table
    (MS-OXOCAL 2.2.4.10) whose cell is there when given; the row grows from 56 to 65 bytes. It is
    `Default` (a `To` recipient with neither) for callers that set what they need.

    Tests: `extra::tests::recipients_keep_their_flags_and_track_status` (a new file and one added
    to), `extra::attach::tests` (an attached message's recipient).
24. **Outlook 97–2002's ANSI files are added to** (`extra/format.rs`, `extra/format/layout.rs`,
    `extra/ansi.rs`, `lib.rs`, `extra/folders.rs`, `extra/named.rs`, `extra/attach.rs`;
    `ndb/header.rs`, `ndb/root.rs` and `ndb/page.rs` get the ANSI setters the Unicode types
    had): the writer wrote Unicode files alone. It keeps one model of a file, in the Unicode types, whose IDs and offsets are the
    wider; an ANSI file's header, B-tree pages and blocks are read with the reader's Ansi types
    and widened (`FileHeader`, `read_block_btree`, `read_tree_block`, `collect_subnode_entries`),
    and narrowed back where they are written, each BID and IB checked (`write_ansi_block`,
    `write_ansi_btree_page`): 12-byte trailers, 8,180 bytes of data in a block (the row matrix's
    rows per block and every data tree's blocks but the last, MS-PST 2.3.4.4), 2,043 BIDs to an
    XBLOCK, 4-byte SLBLOCK and SIBLOCK headers with 12- and 8-byte entries, 41 and 31 entries to
    a B-tree page. Above that layer the two differ in the strings, which are all 8-bit in an ANSI
    file (MS-PST 2.5: "for an ANSI PST, all string properties are stored in ANSI encoding";
    `Text`): a message's in Windows-1252 when its text, recipients and attachments' names are all
    in it, else in UTF-8, with `PidTagMessageCodepage` (MS-OXCMSG 2.2.1.4) saying which, and the
    store's and folders' names, which no property gives a code page for, in Windows-1252 (a `?`
    for a character it hasn't); named properties' names stay UTF-16, as MS-PST 2.4.7 keeps them
    in an ANSI file too. A table's row index keeps a 16-bit row number (MS-PST 2.3.4.3.1), so an
    ANSI folder holds 65,536 items at most, and the file stops at the end of the header's FPMap,
    the only one an ANSI file has (MS-PST 1.3.2.8: about 1.94 GB): both are an error before
    anything is written past them. `AnsiPstFile` takes `append_many_with_attachments`,
    `append_many_in_folder_with_attachments`, `create_folder`, `special_folder`,
    `set_special_folder` and `set_store_display_name`, the same calls as `UnicodePstFile`'s, and
    `create_empty`, Outlook's empty file written again in the ANSI format (each property context
    and table with 8-bit strings and 16-bit row numbers, the store not saying `STORE_UNICODE_OK`)
    with an Inbox the writer makes. Each type takes its own format: a file in the other one is an
    `InvalidInput` error before it is touched. Encoded ANSI files are added to as Unicode ones
    are (fix 10).

    Tests: `extra::ansi::tests` (a new ANSI file read with the reader, every block's trailer, CRC
    and signature checked and no two blocks or pages sharing space; messages with recipients, a
    body over a block, HTML and a 20,000-byte file in Windows-1252 and in UTF-8, a batch of 40
    whose rows go in place, read back; folders made and recorded as special folders; permute- and
    cyclic-encoded ANSI files added to; each format refused by the other's type; the 2 GB and
    65,536-row limits; the code pages).
25. **An ANSI file's folder found again by its name** (`lib.rs`, `extra/format.rs`): an ANSI
    file's folder names are written in Windows-1252, a `?` for a character it hasn't, and the
    writer found a folder by comparing the name asked for with the name the reader reads back,
    a byte to a character (Latin-1). A name with a character Windows-1252 lacks (Cyrillic,
    Greek, Polish `ł`) or keeps above 0x7F where Latin-1 has a control (an en dash, curly
    quotes, the euro sign) never matched, so each append to it made another folder of the same
    written name. `find_child_folder` compares an 8-bit name's bytes with the name as the writer
    writes it, or as the reader reads it back (`same_ansi_folder_name`), ASCII letters in either
    case; names written alike (`Письма` and `Отчёты` are both `??????`) are one folder, which a
    caller that needs them apart names apart.

    Test: `extra::ansi::tests::an_ansi_folder_is_found_again_by_its_name`.
26. **A new ANSI file leaves its neighbours alone** (`extra/ansi.rs`): `AnsiPstFile::create_empty`
    wrote Outlook's empty Unicode file, which it reads its nodes from, to `<path>.template` over
    any file of that name, then deleted it. The template is written to a new file in the
    temporary folder, opened `create_new` under a name of the process, the time and a count.

    Test: `extra::ansi::tests::a_new_ansi_file_leaves_its_neighbours_alone`.
27. **Messages taken out of a file** (`extra/delete.rs`): the writer could only add, so a caller
    that wrote an item again (a changed contact, say) left its earlier copy beside it.
    `UnicodePstFile::delete_messages` and `AnsiPstFile::delete_messages` take messages out by
    their node IDs, whatever folders they are in, in one commit, as MS-PST 2.6.1 lays a message
    out: each one's node leaves the node B-tree, so its property context's and subnode tree's
    blocks lose a reference each and those no node names any more leave the block B-tree with
    their children (`update_block_ref_counts`, checked in tests against a walk from every node);
    its row leaves its folder's contents table and any search folder's that lists it (each table
    written again in a heap of its own, its cells kept in subnodes going with their rows); the
    folder's `PidTagContentCount` and `PidTagContentUnreadCount` are counted again (the unread
    ones by their rows' `PidTagMessageFlags`), in its property context and in its row of its
    parent's hierarchy table; and a `SUD_MSG_DEL` for it goes in the search update queue
    (MS-PST 2.4.8.4.2). The header and the allocation map are written as after an append, so the
    space the freed blocks held is free in the new map, and the next blocks written take it. An
    ID that is not a message in a folder of the file, or given twice, is an `InvalidInput` error
    before the file is touched.

    Tests: `extra::delete::tests` (messages deleted from a new Unicode file, one by one until
    the folder is empty and then added to again, from an ANSI file and from Outlook's own
    permute-encoded fixture, one of them listed in its search folder; read back with the reader,
    the counts right, no two blocks or pages sharing space, every block's count the walk's, the
    allocation map trusted and a big message's space taken by the next one).
