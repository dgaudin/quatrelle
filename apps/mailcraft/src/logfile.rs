//! The app's log: what the connections to the mail servers did and why something failed, in
//! `mailcraft.log` beside the account list (started afresh each run, and capped), and on the
//! terminal when there is one. Never a password or a message's content.

use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

/// The log stops growing past this.
const MAX_BYTES: u64 = 8 * 1024 * 1024;

struct FileLog {
    file: Mutex<Option<(std::fs::File, u64)>>,
    start: Instant,
}

impl log::Log for FileLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        // MailCraft's own lines from Info; other crates' only when they warn.
        let ours = m.target().starts_with("mailcraft");
        m.level() <= if ours { log::Level::Info } else { log::Level::Warn }
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!("{:>8.1}s {:<5} {}: {}\n", self.start.elapsed().as_secs_f32(), r.level(), r.target(), r.args());
        eprint!("{line}");
        if let Ok(mut slot) = self.file.lock()
            && let Some((f, written)) = slot.as_mut()
            && *written < MAX_BYTES
            && f.write_all(line.as_bytes()).is_ok()
        {
            *written = written.saturating_add(line.len() as u64);
        }
    }

    fn flush(&self) {
        if let Ok(mut slot) = self.file.lock()
            && let Some((f, _)) = slot.as_mut()
        {
            let _ = f.flush();
        }
    }
}

/// Start logging to `dir/mailcraft.log` (and the terminal); its path.
pub fn start(dir: Option<&Path>) -> Option<std::path::PathBuf> {
    let path = dir.map(|d| {
        let _ = std::fs::create_dir_all(d);
        d.join("mailcraft.log")
    });
    let file = path.as_ref().and_then(|p| std::fs::File::create(p).ok()).map(|f| (f, 0));
    let logger = Box::new(FileLog { file: Mutex::new(file), start: Instant::now() });
    if log::set_boxed_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    path
}
