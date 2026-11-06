// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Printing through the system's print window (`Services::print`), nothing saved first.
//!
//! - **Linux and the BSDs:** the XDG print portal (`org.freedesktop.portal.Print`), which GNOME
//!   (its `gtk` portal) and KDE answer. `PreparePrint` shows the print window (printer, pages,
//!   copies, two-sided) and, on Print, answers with a token; `Print` then prints the document
//!   with the settings chosen, handed over as a file descriptor open for reading. Each answer is
//!   a `Response` signal on a `Request` object, waited for on a thread of its own: the window
//!   goes on drawing meanwhile, and looks for the outcome each frame.
//! - **Windows and macOS:** no print window here yet; the window falls back on the PDF opened
//!   in the viewer.
//!
//! The PDF is written to a temporary file only long enough to open it: the file is removed at
//! once, and the open descriptor keeps its bytes for the portal to read.

use std::sync::mpsc;

use quatrelle_ui_egui::PrintOutcome;

/// The printing under way, if any: its outcome to come from the thread that waits on it.
#[derive(Default)]
pub(crate) struct Printing {
    answer: Option<mpsc::Receiver<PrintOutcome>>,
}

impl Printing {
    /// Print `pdf`, called `title`, through the print window, on a thread of its own. False when
    /// there's no print window here, or the thread couldn't start.
    #[cfg(all(unix, not(target_os = "macos")))]
    pub(crate) fn start(&mut self, title: &str, pdf: Vec<u8>) -> bool {
        let (tx, rx) = mpsc::channel();
        let title = title.to_string();
        let spawned = std::thread::Builder::new().name("print".into()).spawn(move || {
            let _ = tx.send(portal::print(&title, &pdf));
        });
        match spawned {
            Ok(_) => {
                self.answer = Some(rx);
                true
            }
            Err(e) => {
                log::warn!("printing: no thread for the print window: {e}");
                false
            }
        }
    }

    #[cfg(not(all(unix, not(target_os = "macos"))))]
    pub(crate) fn start(&mut self, _title: &str, _pdf: Vec<u8>) -> bool {
        false
    }

    /// The outcome, once the thread has it; a thread gone without one failed.
    pub(crate) fn take(&mut self) -> Option<PrintOutcome> {
        let outcome = match self.answer.as_ref()?.try_recv() {
            Ok(o) => o,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => PrintOutcome::Failed("the print thread stopped".into()),
        };
        self.answer = None;
        Some(outcome)
    }
}

/// Write `pdf` to a new file in `dir`, open it for reading, and remove it: the open file keeps
/// the bytes, and nothing is left behind (Unix keeps a removed file while it's open).
#[cfg(unix)]
fn unlinked_copy(dir: &std::path::Path, pdf: &[u8]) -> Result<std::fs::File, String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = dir.join(format!("quatrelle-print-{}-{n}.pdf", std::process::id()));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .and_then(|mut f| f.write_all(pdf).and_then(|()| f.sync_all()))
        .and_then(|()| std::fs::File::open(&path));
    // Removed whatever happened, before anything else can fail.
    let _ = std::fs::remove_file(&path);
    written.map_err(|e| format!("the PDF couldn't be made ready to print: {e}"))
}

/// The XDG print portal.
#[cfg(all(unix, not(target_os = "macos")))]
mod portal {
    use std::collections::HashMap;
    use std::time::Duration;

    use quatrelle_ui_egui::PrintOutcome;
    use zbus::zvariant::{Fd, OwnedObjectPath, OwnedValue, Value};

    const DESKTOP: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const PRINT: &str = "org.freedesktop.portal.Print";
    const REQUEST: &str = "org.freedesktop.portal.Request";

    /// A method's options (`a{sv}`).
    pub(super) type Options<'a> = HashMap<&'static str, Value<'a>>;
    /// `PreparePrint`'s arguments: the parent window, the title, the print settings, the page
    /// setup, the options.
    pub(super) type PrepareBody<'a> = (&'a str, &'a str, Options<'a>, Options<'a>, Options<'a>);
    /// `Print`'s: the parent window, the title, the document, the options.
    pub(super) type PrintBody<'a> = (&'a str, &'a str, Fd<'a>, Options<'a>);

    /// Each print's requests named apart.
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    /// What `PreparePrint` answered.
    #[derive(Debug, PartialEq, Eq)]
    pub(super) enum Prepared {
        /// Print was pressed: `Print` with this token prints with the settings chosen.
        Print(u32),
        Cancelled,
        Failed(String),
    }

    /// Print `pdf` through the portal: the print window, then the printing.
    pub(super) fn print(title: &str, pdf: &[u8]) -> PrintOutcome {
        match run(title, pdf) {
            Ok(outcome) => outcome,
            Err(e) => {
                log::warn!("printing through the portal: {e}");
                PrintOutcome::Failed(e)
            }
        }
    }

    fn run(title: &str, pdf: &[u8]) -> Result<PrintOutcome, String> {
        // A method call is answered at once (the window's answer is a signal): one that isn't
        // within the minute has no portal behind it.
        let bus = zbus::blocking::connection::Builder::session()
            .map(|b| b.method_timeout(Duration::from_secs(60)))
            .and_then(|b| b.build())
            .map_err(|e| format!("no session bus: {e}"))?;
        let unique = bus.unique_name().map(|n| n.to_string()).ok_or("no name on the session bus")?;
        // The portal started if it's only started when asked for: its Response is listened for
        // from its owner, which must be there first.
        let _ = bus.call_method(Some(DESKTOP), PATH, Some("org.freedesktop.DBus.Peer"), "Ping", &());
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let prepare = format!("quatrelle_prepare_print_{n}");
        let body: PrepareBody<'_> = ("", title, Options::new(), Options::new(), prepare_options(&prepare));
        let (code, results) =
            ask(&bus, &unique, "PreparePrint", &prepare, || bus.call_method(Some(DESKTOP), PATH, Some(PRINT), "PreparePrint", &body))?;
        let token = match prepared(code, &results) {
            Prepared::Print(token) => token,
            Prepared::Cancelled => return Ok(PrintOutcome::Cancelled),
            Prepared::Failed(e) => return Err(e),
        };
        let file = super::unlinked_copy(&std::env::temp_dir(), pdf)?;
        let handle = format!("quatrelle_print_{n}");
        let body: PrintBody<'_> = ("", title, Fd::from(&file), print_options(&handle, token));
        let (code, _) = ask(&bus, &unique, "Print", &handle, || bus.call_method(Some(DESKTOP), PATH, Some(PRINT), "Print", &body))?;
        Ok(printed(code))
    }

    /// Call the portal's `method` (`call`), its request named `token`, and wait for the
    /// request's `Response`: its code and results. The signal is listened for before the call,
    /// at the request's path worked out ahead, so an answer can't come before it's listened for.
    fn ask(
        bus: &zbus::blocking::Connection,
        unique: &str,
        method: &str,
        token: &str,
        call: impl FnOnce() -> zbus::Result<zbus::Message>,
    ) -> Result<(u32, HashMap<String, OwnedValue>), String> {
        let listen = |path: String| {
            zbus::blocking::Proxy::new(bus, DESKTOP, path, REQUEST).and_then(|p| p.receive_signal("Response")).map_err(|e| e.to_string())
        };
        let expected = request_path(unique, token);
        let mut responses = listen(expected.clone())?;
        let reply = call().map_err(|e| format!("{method}: {e}"))?;
        let handle: OwnedObjectPath = reply.body().deserialize().map_err(|e| format!("{method}: {e}"))?;
        // An older portal names the request its own way.
        if handle.as_str() != expected {
            responses = listen(handle.to_string())?;
        }
        let response = responses.next().ok_or_else(|| format!("{method}: the portal went away"))?;
        response.body().deserialize::<(u32, HashMap<String, OwnedValue>)>().map_err(|e| format!("{method}: {e}"))
    }

    /// The path of the request a call with `handle_token` makes (the portal's documented
    /// scheme): the caller's unique bus name without its colon, dots made underscores.
    pub(super) fn request_path(unique: &str, token: &str) -> String {
        let sender = unique.trim_start_matches(':').replace('.', "_");
        format!("{PATH}/request/{sender}/{token}")
    }

    /// `PreparePrint`'s options: the request's name, and a window that waits for an answer.
    pub(super) fn prepare_options(handle: &str) -> Options<'_> {
        HashMap::from([("handle_token", Value::from(handle)), ("modal", Value::from(true))])
    }

    /// `Print`'s options: the request's name, and the token `PreparePrint` gave, so it prints
    /// as chosen there without asking again.
    pub(super) fn print_options(handle: &str, token: u32) -> Options<'_> {
        let mut o = prepare_options(handle);
        o.insert("token", Value::from(token));
        o
    }

    /// `PreparePrint`'s response: 0 is Print (with its token), 1 Cancel, anything else a failure.
    pub(super) fn prepared(code: u32, results: &HashMap<String, OwnedValue>) -> Prepared {
        match code {
            0 => match results.get("token").and_then(|t| u32::try_from(t).ok()) {
                Some(token) => Prepared::Print(token),
                None => Prepared::Failed("PreparePrint: no token in its answer".into()),
            },
            1 => Prepared::Cancelled,
            n => Prepared::Failed(format!("PreparePrint: answered {n}")),
        }
    }

    /// `Print`'s response: 0 printed, 1 cancelled, anything else a failure.
    pub(super) fn printed(code: u32) -> PrintOutcome {
        match code {
            0 => PrintOutcome::Sent,
            1 => PrintOutcome::Cancelled,
            n => PrintOutcome::Failed(format!("Print: answered {n}")),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn owned(v: Value<'_>) -> OwnedValue {
            v.try_to_owned().unwrap()
        }

        #[test]
        fn the_request_path_follows_the_portals_scheme() {
            assert_eq!(request_path(":1.42", "quatrelle_print_7"), "/org/freedesktop/portal/desktop/request/1_42/quatrelle_print_7");
        }

        #[test]
        fn the_options_name_the_request_and_carry_the_token() {
            let p = prepare_options("quatrelle_prepare_print_7");
            assert_eq!(p.get("handle_token"), Some(&Value::from("quatrelle_prepare_print_7")));
            assert_eq!(p.get("modal"), Some(&Value::from(true)));
            assert_eq!(p.len(), 2);
            let o = print_options("quatrelle_print_7", 99);
            assert_eq!(o.get("handle_token"), Some(&Value::from("quatrelle_print_7")));
            assert_eq!(o.get("token"), Some(&Value::U32(99)));
            // The arguments' D-Bus signatures, as the portal's methods take them.
            use zbus::zvariant::Type;
            assert_eq!(<PrepareBody<'_> as Type>::SIGNATURE.to_string(), "(ssa{sv}a{sv}a{sv})");
            assert_eq!(<PrintBody<'_> as Type>::SIGNATURE.to_string(), "(ssha{sv})");
        }

        #[test]
        fn the_answers_are_read() {
            let token = HashMap::from([("token".to_string(), owned(Value::U32(5))), ("settings".to_string(), owned(Value::from("x")))]);
            assert_eq!(prepared(0, &token), Prepared::Print(5));
            assert!(matches!(prepared(0, &HashMap::new()), Prepared::Failed(_)), "no token");
            let wrong = HashMap::from([("token".to_string(), owned(Value::from("5")))]);
            assert!(matches!(prepared(0, &wrong), Prepared::Failed(_)), "a token of the wrong type");
            assert_eq!(prepared(1, &token), Prepared::Cancelled);
            assert!(matches!(prepared(2, &token), Prepared::Failed(_)));
            assert_eq!(printed(0), PrintOutcome::Sent);
            assert_eq!(printed(1), PrintOutcome::Cancelled);
            assert!(matches!(printed(2), PrintOutcome::Failed(_)));
            assert!(matches!(printed(u32::MAX), PrintOutcome::Failed(_)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The PDF handed over reads back whole from its descriptor, and no file is left.
    #[cfg(unix)]
    #[test]
    fn the_pdf_is_handed_over_and_no_file_is_left() {
        use std::io::Read;
        let dir = std::env::temp_dir().join(format!("quatrelle-print-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pdf = b"%PDF-1.7\n% a message\n%%EOF\n".repeat(100);
        let mut file = unlinked_copy(&dir, &pdf).unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "removed at once");
        let mut back = Vec::new();
        file.read_to_end(&mut back).unwrap();
        assert_eq!(back, pdf);
        assert!(unlinked_copy(&dir.join("gone"), &pdf).is_err(), "no folder: an error, not a panic");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The outcome comes once; a thread gone without one is a failure, not a wait for ever.
    #[test]
    fn the_outcome_comes_once() {
        let mut p = Printing::default();
        assert_eq!(p.take(), None);
        let (tx, rx) = mpsc::channel();
        p.answer = Some(rx);
        assert_eq!(p.take(), None, "still open");
        tx.send(PrintOutcome::Cancelled).unwrap();
        assert_eq!(p.take(), Some(PrintOutcome::Cancelled));
        assert_eq!(p.take(), None);
        let (tx, rx) = mpsc::channel::<PrintOutcome>();
        p.answer = Some(rx);
        drop(tx);
        assert!(matches!(p.take(), Some(PrintOutcome::Failed(_))));
    }
}
