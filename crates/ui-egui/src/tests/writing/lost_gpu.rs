// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the window's GPU device lost (a driver reset after the computer slept), what's
//! being written is kept as a close keeps it, and the window closes without asking.

use quatrelle_engine::model::{Folder, FolderRole, Job};

use super::*;

/// The jobs the app handed its backend (`NullBackend` drops them).
type Handed = std::rc::Rc<std::cell::RefCell<Vec<Job>>>;

struct Recorder(Handed);

impl crate::Backend for Recorder {
    fn submit(&mut self, job: Job) {
        self.0.borrow_mut().push(job);
    }

    fn poll(&mut self) -> Vec<MailEvent> {
        Vec::new()
    }
}

/// The GPU device goes: nothing can be drawn, so nobody can answer "keep what you're writing?".
/// Both messages being written on an Exchange account, online, are kept the way a close keeps
/// them: on this computer as the job is taken (`KeepDraft`), for Drafts at the next connection;
/// never as a save to the server (`SaveDraft`), which goes to the account's thread that the close
/// stops. Each once, however often the loss is said; the window closes without asking.
#[test]
fn a_lost_gpu_keeps_whats_written_and_closes() {
    let (mut a, dir) = app("gpu-lost");
    a.session.prefs.new_mail_window = false;
    let handed = Handed::default();
    a.backend = Box::new(Recorder(handed.clone()));
    let ews = with_exchange(&mut a);
    let folder = |path: &str, role| Folder { account: ews.clone(), path: path.into(), role, selectable: true, ..Default::default() };
    a.session
        .apply(MailEvent::Folders { account: ews.clone(), folders: vec![folder("Inbox", FolderRole::Inbox), folder("Drafts", FolderRole::Drafts)] });
    a.session.apply(MailEvent::Status { account: ews.clone(), status: SyncStatus::Online { at: NOW } });
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    for subject in ["First thoughts", "Second thoughts"] {
        a.run("mail.new", json!({}));
        a.run("compose.update", json!({"account": ews, "subject": subject, "body": "Not sent yet."}));
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(a.session.all_composes().filter(|c| c.account == ews).count(), 2);
    handed.borrow_mut().clear();
    a.graphics_lost(&ctx);
    a.graphics_lost(&ctx);
    assert!(a.ui.graphics_lost && a.ui.closing_anyway && a.ui.close_now && a.ui.dialog.is_none());
    let jobs = handed.borrow();
    let kept: Vec<&[u8]> = jobs
        .iter()
        .filter_map(|j| match j {
            Job::KeepDraft { account, raw, .. } if *account == ews => Some(raw.as_slice()),
            _ => None,
        })
        .collect();
    let saved = jobs.iter().filter(|j| matches!(j, Job::SaveDraft { .. })).count();
    assert_eq!((kept.len(), saved), (2, 0), "kept on this computer, each once; none left to the account's thread");
    for subject in ["First thoughts", "Second thoughts"] {
        let has = |raw: &[u8]| raw.windows(subject.len()).any(|w| w == subject.as_bytes());
        assert_eq!(kept.iter().filter(|raw| has(raw)).count(), 1, "{subject} kept once");
    }
    let _ = std::fs::remove_dir_all(dir);
}
