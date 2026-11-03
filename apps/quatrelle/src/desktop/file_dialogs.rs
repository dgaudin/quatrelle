// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The system's file dialogs (attach, save as, pick a folder…), opened without holding the
//! window. A blocking dialog stopped the window's event loop for as long as it was open: the
//! window answered nothing, and GNOME, calling it "not responding", offered to kill it (and
//! did, with a message being written, 03/11).
//!
//! These are rfd's async dialogs, asked from the window's own thread and polled each frame:
//! on Windows and Linux (the XDG portal) rfd runs the dialog on a thread of its own; on macOS,
//! where AppKit's panels belong to the main thread, it begins the panel there as a sheet on
//! the window and answers through the main loop, which goes on running. Each dialog belongs to
//! Quatrelle's window (`set_parent`), above it and modal to it where the system allows.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

/// A dialog's answer to come: the paths picked (none on Cancel).
type Answer = Pin<Box<dyn Future<Output = Vec<PathBuf>> + Send>>;

/// The file dialog open, if one is.
#[derive(Default)]
pub(crate) struct FileDialogs {
    /// What every dialog starts from: Quatrelle's window as its parent (none in tests).
    pub(crate) base: rfd::AsyncFileDialog,
    /// The dialog open: its answer to come, and whether its folder is the next one's
    /// (`Some(true)`: the folder picked; `Some(false)`: the folder of the file picked).
    open: Option<(Answer, Option<bool>)>,
}

impl FileDialogs {
    /// Dialogs that belong to Quatrelle's window.
    pub(crate) fn over(window: &eframe::CreationContext<'_>) -> Self {
        Self { base: rfd::AsyncFileDialog::new().set_parent(window), open: None }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The dialog whose answer is `answer` is open (on Windows and Linux, rfd starts its thread
    /// at the first look, the next frame's).
    pub(crate) fn start(&mut self, answer: Answer, remember: Option<bool>) {
        self.open = Some((answer, remember));
    }

    /// The open dialog's answer once it's given, and where the next dialog opens (as
    /// [`FileDialogs::start`] was told); `None` while it's still open, or when none is.
    pub(crate) fn poll(&mut self) -> Option<(Vec<PathBuf>, Option<bool>)> {
        let (answer, _) = self.open.as_mut()?;
        // Nothing to wake: the window looks again each frame while a dialog is open.
        let mut cx = Context::from_waker(Waker::noop());
        match answer.as_mut().poll(&mut cx) {
            Poll::Ready(paths) => self.open.take().map(|(_, remember)| (paths, remember)),
            Poll::Pending => None,
        }
    }
}

/// The answer of a dialog that picks several files.
pub(crate) fn many(f: impl Future<Output = Option<Vec<rfd::FileHandle>>> + Send + 'static) -> Answer {
    Box::pin(async move { f.await.unwrap_or_default().iter().map(|h| h.path().to_path_buf()).collect() })
}

/// The answer of a dialog that picks one file or folder, or where to save.
pub(crate) fn one(f: impl Future<Output = Option<rfd::FileHandle>> + Send + 'static) -> Answer {
    Box::pin(async move { f.await.map(|h| h.path().to_path_buf()).into_iter().collect() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An answer is given once, then the dialog is closed; none open answers nothing.
    #[test]
    fn an_answer_closes_the_dialog() {
        let mut d = FileDialogs::default();
        assert!(d.poll().is_none() && !d.is_open());
        d.start(Box::pin(async { vec![PathBuf::from("/tmp/a.txt")] }), Some(false));
        assert!(d.is_open());
        assert_eq!(d.poll(), Some((vec![PathBuf::from("/tmp/a.txt")], Some(false))));
        assert!(!d.is_open() && d.poll().is_none());
        // Still open: asked again later.
        d.start(Box::pin(std::future::pending()), None);
        assert!(d.poll().is_none() && d.is_open());
    }
}
