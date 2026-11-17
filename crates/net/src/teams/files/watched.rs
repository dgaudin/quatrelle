// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A transfer's connection, watched: every read and write first looks at the transfer's cancel
//! flag and at the floor of speed, so a lane hears "stop" and gives up on a server that trickles.
//!
//! The lanes' sockets read and write in short slices ([`super::LANE_IO`]): a slice in which
//! nothing came (`WouldBlock`, `TimedOut`, `Interrupted` from the connection) is nothing moved,
//! and the cancel flag and the floor are looked at again before the next. A cancelled transfer
//! ends with an error of its own ("cancelled", never `Interrupted`, which `write_all` and
//! `read_exact` would try again for ever).

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// What a cancelled transfer's next read or write says.
pub(super) const CANCELLED: &str = "cancelled";
/// What a transfer slower than its floor says.
pub(super) const TOO_SLOW: &str = "too slow";

/// A connection whose transfer can be cancelled and must keep a floor of speed: at least
/// `floor` bytes a second over each `window`, read and written together.
pub(super) struct Watched<S> {
    inner: S,
    cancel: Arc<AtomicBool>,
    /// The bytes a whole window must move.
    least: u64,
    window: Duration,
    /// When the window began, and what moved in it.
    began: Instant,
    moved: u64,
}

impl<S> Watched<S> {
    pub(super) fn new(inner: S, cancel: Arc<AtomicBool>, floor: u64, window: Duration) -> Self {
        let least = u128::from(floor).saturating_mul(window.as_millis()) / 1000;
        Watched { inner, cancel, least: u64::try_from(least).unwrap_or(u64::MAX), window, began: Instant::now(), moved: 0 }
    }

    /// Before each call: cancelled, or a whole window gone by with less than its floor moved.
    /// A window that has passed is judged once; the next one starts then.
    fn check(&mut self) -> io::Result<()> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(io::Error::other(CANCELLED));
        }
        let now = Instant::now();
        if now.saturating_duration_since(self.began) >= self.window {
            let slow = self.moved < self.least;
            self.began = now;
            self.moved = 0;
            if slow {
                return Err(io::Error::new(io::ErrorKind::TimedOut, TOO_SLOW));
            }
        }
        Ok(())
    }

    /// `op` on the connection until something moves or the transfer is given up: a slice in
    /// which nothing came is looked past.
    fn watch(&mut self, mut op: impl FnMut(&mut S) -> io::Result<usize>) -> io::Result<usize> {
        loop {
            self.check()?;
            match op(&mut self.inner) {
                Ok(n) => {
                    self.moved = self.moved.saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
                    return Ok(n);
                }
                Err(e) if quiet(&e) => {}
                Err(e) => return Err(e),
            }
        }
    }
}

/// A slice of the connection in which nothing came: its own timeout (`WouldBlock`, or `TimedOut`
/// on Windows), or a signal.
fn quiet(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted)
}

impl<S: Read> Read for Watched<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.watch(|s| s.read(buf))
    }
}

impl<S: Write> Write for Watched<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.watch(|s| s.write(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.watch(|s| s.flush().map(|()| 0)).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    const WINDOW: Duration = Duration::from_millis(100);

    fn flag(on: bool) -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(on))
    }

    /// A connection on which nothing ever comes: each read or write waits a slice of 10 ms, as a
    /// socket's own timeout would, then says so.
    struct Quiet;

    impl Read for Quiet {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            std::thread::sleep(Duration::from_millis(10));
            Err(io::ErrorKind::WouldBlock.into())
        }
    }

    impl Write for Quiet {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            std::thread::sleep(Duration::from_millis(10));
            Err(io::ErrorKind::TimedOut.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// H4: 1,000 bytes a second over 100 ms is 100 bytes a window. Ten bytes, then a read once the
    /// window is over: given up, and the next window starts then. Two hundred bytes in the window:
    /// it goes on.
    #[test]
    fn a_transfer_slower_than_the_floor_is_given_up() {
        let mut w = Watched::new(Cursor::new(vec![7u8; 1000]), flag(false), 1000, WINDOW);
        assert_eq!(w.read(&mut [0u8; 10]).unwrap(), 10);
        std::thread::sleep(Duration::from_millis(150));
        let e = w.read(&mut [0u8; 10]).unwrap_err();
        assert_eq!((e.kind(), e.to_string().as_str()), (io::ErrorKind::TimedOut, TOO_SLOW));
        assert_eq!(w.read(&mut [0u8; 10]).unwrap(), 10, "judged once: the next window starts");

        let mut w = Watched::new(Cursor::new(vec![7u8; 1000]), flag(false), 1000, WINDOW);
        assert_eq!(w.read(&mut [0u8; 200]).unwrap(), 200);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(w.read(&mut [0u8; 10]).unwrap(), 10, "enough moved in the window");

        // Read and written together: 60 bytes each way are the window's 100.
        let mut w = Watched::new(Cursor::new(vec![7u8; 1000]), flag(false), 1000, WINDOW);
        assert_eq!(w.read(&mut [0u8; 60]).unwrap(), 60);
        assert_eq!(w.write(&[1u8; 60]).unwrap(), 60);
        std::thread::sleep(Duration::from_millis(150));
        assert!(w.read(&mut [0u8; 10]).is_ok());
    }

    /// A connection on which nothing comes is given up at the floor, not waited on for ever.
    #[test]
    fn a_quiet_connection_is_given_up_at_the_floor() {
        let start = Instant::now();
        let e = Watched::new(Quiet, flag(false), 1000, WINDOW).read(&mut [0u8; 10]).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(1), "{:?}", start.elapsed());
        let e = Watched::new(Quiet, flag(false), 1000, WINDOW).write(&[0u8; 10]).unwrap_err();
        assert_eq!(e.to_string(), TOO_SLOW);
    }

    /// Cancelled, the next read or write fails at once, before a byte moves; and a read waiting
    /// on a quiet connection hears it within a slice.
    #[test]
    fn a_cancelled_transfer_stops_at_its_next_byte() {
        let cancel = flag(false);
        let mut w = Watched::new(Cursor::new(vec![7u8; 1000]), cancel.clone(), 1, Duration::from_secs(60));
        assert_eq!(w.read(&mut [0u8; 10]).unwrap(), 10);
        cancel.store(true, Ordering::SeqCst);
        let e = w.read(&mut [0u8; 10]).unwrap_err();
        assert_eq!(e.to_string(), CANCELLED);
        assert_ne!(e.kind(), io::ErrorKind::Interrupted, "write_all would try again for ever");
        assert_eq!(w.write(&[1u8; 10]).unwrap_err().to_string(), CANCELLED);
        assert_eq!(w.flush().unwrap_err().to_string(), CANCELLED);
        assert_eq!(w.inner.position(), 10, "nothing more read");

        let cancel = flag(false);
        let later = cancel.clone();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            later.store(true, Ordering::SeqCst);
        });
        let start = Instant::now();
        let e = Watched::new(Quiet, cancel, 1, Duration::from_secs(60)).read(&mut [0u8; 10]).unwrap_err();
        assert_eq!(e.to_string(), CANCELLED);
        assert!(start.elapsed() < Duration::from_secs(1), "{:?}", start.elapsed());
        stopper.join().unwrap();
    }
}
