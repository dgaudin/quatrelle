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
            let (mut parsed, mut html, mut files, mut dated) = (0, 0, 0, 0);
            for f in &done.folders {
                eprintln!("  {:30} {:?} total {} unread {}", f.path, f.role, f.total, f.unread);
                for m in store.load_folder("r1", &f.path).unwrap().messages {
                    dated += usize::from(m.date > 0);
                    if let Some(p) = store.read_body(&m.key).and_then(|raw| quatrelle_mime::parse(&raw)) {
                        parsed += 1;
                        html += usize::from(p.had_html);
                        files += p.attachments.len();
                    }
                }
            }
            eprintln!("parsed {parsed} (html {html}, attachments {files}, dated {dated})");
        }
        Err(e) => eprintln!("FAILED: {e}"),
    }
    let _ = std::fs::remove_dir_all(dir);
}
