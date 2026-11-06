// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
//! Several attachments from one parse ([`attachments_data`]): the reading pane takes a message's
//! pictures so, once, instead of parsing the message again for each.

use super::*;
use proptest::prelude::*;

/// A message with these attachments (name, bytes).
fn with_files(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let d = Draft {
        subject: "Files".into(),
        body: "Here.".into(),
        attachments: files
            .iter()
            .map(|(name, data)| OutAttachment {
                name: name.clone(),
                mime: "application/octet-stream".into(),
                data: data.clone(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    build(&d, &EmailAddress::new("Sam", "sam@studio.example"), 1_700_000_000, "<files@x>").unwrap_or_default()
}

#[test]
fn several_attachments_come_from_one_parse_within_their_caps() {
    let files: Vec<(String, Vec<u8>)> = (0..5u8).map(|i| (format!("f{i}.bin"), vec![i; 100 * (usize::from(i) + 1)])).collect();
    let raw = with_files(&files);
    // In the order asked, each once; one the message hasn't is left out.
    let got = attachments_data(&raw, &[3, 0, 3, 99], usize::MAX, usize::MAX);
    assert_eq!(got.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [3, 0]);
    assert_eq!(got[0].1, files[3].1);
    assert_eq!(got[1].1, files[0].1);
    // Each at most 250 bytes: 0 and 1 (100 and 200), not 2 (300).
    let got = attachments_data(&raw, &[0, 1, 2], 250, usize::MAX);
    assert_eq!(got.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [0, 1]);
    // All together at most 600: 4 (500) fits, then 1 (200) doesn't, 0 (100) does.
    let got = attachments_data(&raw, &[4, 1, 0], usize::MAX, 600);
    assert_eq!(got.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [4, 0]);
    // Nothing asked, or no message: nothing, and no parse.
    assert!(attachments_data(&raw, &[], usize::MAX, usize::MAX).is_empty());
    assert!(attachments_data(b"", &[0], usize::MAX, usize::MAX).is_empty());
    assert!(attachments_data(&raw, &[usize::MAX], 1 << 20, 4 << 20).is_empty());
    // Only so many asked at once.
    let many: Vec<usize> = (0..10_000).collect();
    assert_eq!(attachments_data(&raw, &many, usize::MAX, usize::MAX).len(), 5);
}

/// Files moved out of one parse ([`attachments_moved`]): each once, the same bytes as read alone,
/// a text file's and a binary one's alike; an index asked twice once, one it doesn't have none.
#[test]
fn files_moved_out_of_one_parse_are_the_files() {
    let mut files: Vec<(String, Vec<u8>)> = (0..4u8).map(|i| (format!("f{i}.bin"), vec![i; 3000 + usize::from(i)])).collect();
    files.push(("note.txt".into(), b"caf\xe9 in Latin-1".to_vec()));
    let raw = with_files(&files);
    let mut all = Vec::new();
    attachments_moved(&raw, &[4, 0, 3, 1, 2, 0, 9], &mut |index, data| all.push((index, data)));
    all.sort();
    let each: Vec<_> = (0..5).map(|i| (i, attachment_data(&raw, i).unwrap())).collect();
    assert_eq!(all, each);
    assert_eq!(attachment_data_moved(&raw, 2), attachment_data(&raw, 2));
    assert_eq!(attachment_data_moved(&raw, 9), None);
    let mut none = 0;
    attachments_moved(b"", &[0], &mut |_, _| none += 1);
    attachments_moved(&raw, &[], &mut |_, _| none += 1);
    assert_eq!(none, 0);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// One parse gives each attachment's bytes as reading it alone does.
    #[test]
    fn one_parse_gives_what_each_read_alone_gives(
        sizes in proptest::collection::vec(0usize..2000, 0..8),
        wanted in proptest::collection::vec(0usize..10, 0..12),
    ) {
        let files: Vec<(String, Vec<u8>)> =
            sizes.iter().enumerate().map(|(i, n)| (format!("f{i}.bin"), (0..*n).map(|k| (k * 7 + i) as u8).collect())).collect();
        let raw = with_files(&files);
        let got = attachments_data(&raw, &wanted, usize::MAX, usize::MAX);
        for (i, data) in &got {
            prop_assert_eq!(Some(data.clone()), attachment_data(&raw, *i));
        }
        let mut asked: Vec<usize> = wanted.iter().copied().filter(|i| attachment_data(&raw, *i).is_some()).collect();
        asked.sort_unstable();
        asked.dedup();
        let mut had: Vec<usize> = got.iter().map(|(i, _)| *i).collect();
        had.sort_unstable();
        prop_assert_eq!(had, asked);
    }
}
