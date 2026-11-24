// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The CLI's arguments as the system hands them over.

/// An argument that isn't Unicode (a file name in another encoding) panicked `std::env::args()`
/// before anything ran (WordCraft found); it's read lossily now.
#[test]
fn an_argument_that_isnt_unicode_doesnt_crash() {
    #[cfg(windows)]
    let odd = {
        use std::os::windows::ffi::OsStringExt;
        std::ffi::OsString::from_wide(&[0x66, 0xD800, 0x67])
    };
    #[cfg(unix)]
    let odd = {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(vec![b'f', 0xff, b'g'])
    };
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_quatrelle-cli")).arg("--version").arg(odd).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("quatrelle"));
}

/// Output into a pipe closed early (`quatrelle-cli … | head`) is passed over: `println!` panicked
/// there.
#[test]
fn a_closed_pipe_doesnt_crash() {
    use std::process::{Command, Stdio};
    for _ in 0..5 {
        let mut child =
            Command::new(env!("CARGO_BIN_EXE_quatrelle-cli")).arg("--help").stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        drop(child.stdout.take());
        let out = child.wait_with_output().unwrap();
        assert!(!String::from_utf8_lossy(&out.stderr).contains("panicked"), "{}", String::from_utf8_lossy(&out.stderr));
        assert_ne!(out.status.code(), Some(101), "a panic's exit code");
    }
}

/// What the CLI prints can't drive the terminal: an error that repeats what it was given shows
/// an escape in it as `\u{1b}`, never the byte itself. Mail it lists and reads, folders' names
/// and servers' errors go through the same printing.
#[test]
fn what_is_printed_cant_drive_the_terminal() {
    let data = std::env::temp_dir().join(format!("quatrelle-cli-printable-{}", std::process::id()));
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_quatrelle-cli"))
        .arg("--data")
        .arg(&data)
        .args(["run", "mail.\u{1b}]0;pwned\u{7}\u{1b}[31mred", "{}"])
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&data);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains('\u{1b}') && !err.contains('\u{7}'), "{err:?}");
    assert!(err.contains("\\u{1b}]0;pwned\\u{7}\\u{1b}[31mred"), "{err:?}");
}

/// Read as text, no source of the CLI starts a Windows program by its bare name: the current
/// folder is searched first, where a portable copy's `rundll32.exe` would run. A
/// `Command::new("…exe")` or a `("…exe", …)` tuple is refused (the app's `windows_programs.rs`
/// is the model; the CLI keeps a copy of it).
#[test]
fn windows_programs_are_never_started_by_their_bare_name() {
    let sources = [include_str!("../src/main.rs"), include_str!("../src/teams.rs")];
    for src in sources {
        for line in src.lines() {
            if line.contains(r#"::path(""#) {
                continue;
            }
            if let Some(at) = line.find(r#"Command::new(""#) {
                let lit = line[at + r#"Command::new(""#.len()..].split('"').next().unwrap_or("").to_ascii_lowercase();
                assert!(!(lit.ends_with(".exe") || lit.contains("powershell")), "a program by its bare name: {line}");
            }
            let mut at = 0;
            while let Some(rel) = line[at..].find(r#"(""#) {
                let open = at + rel;
                let before = line[..open].chars().next_back();
                let tuple_start = before.is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '.' || c == '!' || c == '>'));
                if tuple_start {
                    let lit = line[open + 2..].split('"').next().unwrap_or("");
                    let bare = !lit.contains('{') && lit.to_ascii_lowercase().ends_with(".exe");
                    assert!(!bare, "a program by its bare name: {line}");
                }
                at = open + 2;
            }
        }
    }
}

/// Everything the CLI writes to the terminal goes through `printable`: read back as text, no
/// `write!`/`writeln!` on stdout or stderr prints without it (a message's source comes from
/// whoever sent it, and the `source` command prints it).
#[test]
fn everything_printed_goes_through_printable() {
    for src in [include_str!("../src/main.rs"), include_str!("../src/teams.rs")] {
        for line in src.lines() {
            let direct = line.contains("write!(std::io::stdout()")
                || line.contains("writeln!(std::io::stdout()")
                || line.contains("write!(std::io::stderr()")
                || line.contains("writeln!(std::io::stderr()");
            if direct {
                assert!(line.contains("printable("), "printed without printable: {line}");
            }
        }
    }
}
