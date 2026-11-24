// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What gives memory back to the system: jemalloc's background thread, the log of what the process
//! holds, and fewer arenas for the system libraries' own malloc.

/// jemalloc purges freed pages only when the thread that freed them allocates again, so a sync
/// thread that ends, or a window left alone, kept them; its background thread purges them after
/// ten seconds unused. One is enough.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) fn give_back_freed_memory() {
    let _ = tikv_jemalloc_ctl::max_background_threads::write(1);
    match tikv_jemalloc_ctl::background_thread::write(true) {
        Ok(()) => log::info!("memory: freed memory goes back to the system"),
        Err(e) => log::warn!("memory: freed memory stays with the app (no background thread: {e})"),
    }
    // What the memory is, in the log now and then: what's in use, and the rest of what the
    // process holds (pictures drawn, fonts, the program itself).
    let _ = std::thread::Builder::new().name("memory".into()).spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(60));
        loop {
            log_memory();
            std::thread::sleep(std::time::Duration::from_secs(10 * 60));
        }
    });
}

/// One line on the memory: the process's resident size, and the allocator's share of it.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn log_memory() {
    use tikv_jemalloc_ctl::{epoch, stats};
    if epoch::advance().is_err() {
        return;
    }
    let mb = |r: Result<usize, tikv_jemalloc_ctl::Error>| r.map_or(0, |b| b >> 20);
    let rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)).and_then(|k| k.parse::<usize>().ok()))
        .map_or(0, |kb| kb >> 10);
    log::info!(
        "memory: {rss} MB resident; the allocator's {} MB resident, {} MB in pages used, {} MB in use; {} MB on the graphics card",
        mb(stats::resident::read()),
        mb(stats::active::read()),
        mb(stats::allocated::read()),
        graphics_memory() >> 10
    );
}

/// What the graphics driver holds for this process (KiB), as the kernel tells it for each of its
/// clients (`/proc/self/fdinfo`, `drm-total-*`): an integrated GPU's memory is the computer's.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn graphics_memory() -> u64 {
    let Ok(fds) = std::fs::read_dir("/proc/self/fdinfo") else { return 0 };
    let mut clients = std::collections::HashMap::new();
    for fd in fds.flatten().take(4096) {
        let Ok(info) = std::fs::read_to_string(fd.path()) else { continue };
        let Some(client) = info.lines().find_map(|l| l.strip_prefix("drm-client-id:")).map(|c| c.trim().to_string()) else { continue };
        let kib: u64 = info.lines().filter(|l| l.starts_with("drm-total-")).filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()).sum();
        clients.insert(client, kib);
    }
    clients.values().sum()
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub(crate) fn give_back_freed_memory() {}

/// The system libraries' malloc (glibc's: the graphics driver's, X11's) keeps an arena for each
/// thread that ever allocated, each holding on to what it freed: 25 of them held 54 MB in an
/// Exchange user's Quatrelle, none of it Quatrelle's own (that's jemalloc's). Two are enough.
/// glibc reads `MALLOC_ARENA_MAX` only as a process starts, so Quatrelle starts again with it,
/// once, before anything else; one set already is left as it is, and the programs Quatrelle
/// opens inherit it.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub(crate) fn fewer_malloc_arenas() {
    use std::os::unix::process::CommandExt;
    if std::env::var_os("MALLOC_ARENA_MAX").is_some() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    // Returns only when it couldn't start again: then it goes on as it is.
    let _ = std::process::Command::new(exe).args(std::env::args_os().skip(1)).env("MALLOC_ARENA_MAX", "2").exec();
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub(crate) fn fewer_malloc_arenas() {}

#[cfg(test)]
mod tests {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    use super::*;

    /// The allocator gives freed memory back by itself (Linux on x86-64).
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn freed_memory_goes_back_to_the_system() {
        give_back_freed_memory();
        assert_eq!(tikv_jemalloc_ctl::background_thread::read().ok(), Some(true));
        assert_eq!(tikv_jemalloc_ctl::max_background_threads::read().ok(), Some(1));
    }
}
