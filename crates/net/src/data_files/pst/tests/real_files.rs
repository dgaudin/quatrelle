// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A real data file, imported when one is named (counts only).

/// Import a real data file named by `QUATRELLE_PST` (counts only).
#[test]
#[ignore]
fn imports_the_file_in_quatrelle_pst() {
    let Ok(path) = std::env::var("QUATRELLE_PST") else { return };
    let dir = std::env::temp_dir().join(format!("quatrelle-real-pst-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = quatrelle_store::Store::open(&dir).unwrap();
    let t = std::time::Instant::now();
    let r = super::import(std::path::Path::new(&path), &store, "r1", &mut |_| {});
    match r {
        Ok(done) => {
            eprintln!("messages {} contacts {} events {} skipped {} in {:?}", done.messages, done.contacts, done.events, done.skipped, t.elapsed());
            eprintln!("files left out {:?}, {:?}", done.files_left_out, done.reading);
            let (mut parsed, mut html, mut files, mut dated) = (0, 0, 0, 0);
            // Attached messages by level (0 = attached to an item) and the files inside them.
            let (mut attached, mut inner) = ([0usize; 4], 0);
            for f in &done.folders {
                eprintln!("  {:30} {:?} total {} unread {}", f.path, f.role, f.total, f.unread);
                for m in store.load_folder("r1", &f.path).unwrap().messages {
                    dated += usize::from(m.date > 0);
                    let raw = store.read_body(&m.key).unwrap_or_default();
                    if let Some(p) = quatrelle_mime::parse(&raw) {
                        parsed += 1;
                        html += usize::from(p.had_html);
                        files += p.attachments.len();
                        count_attached(&raw, 0, &mut attached, &mut inner);
                    }
                }
            }
            eprintln!("parsed {parsed} (html {html}, attachments {files}, dated {dated})");
            eprintln!("attached messages by level {attached:?}, files inside them {inner}");
        }
        Err(e) => eprintln!("FAILED: {e}"),
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Counts the attached messages in `raw`, `level` down, and the other files inside them.
fn count_attached(raw: &[u8], level: usize, attached: &mut [usize; 4], inner: &mut usize) {
    let Some(p) = quatrelle_mime::parse(raw) else { return };
    for a in &p.attachments {
        if a.mime.eq_ignore_ascii_case("message/rfc822") {
            if let Some(n) = attached.get_mut(level) {
                *n += 1;
            }
            if let Some(data) = quatrelle_mime::attachment_data(raw, a.index) {
                count_attached(&data, level + 1, attached, inner);
            }
        } else if level > 0 {
            *inner += 1;
        }
    }
}
