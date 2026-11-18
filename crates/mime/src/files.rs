// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
//! Attachment names from a message, before anything is written to disk or handed to another app:
//! [`sanitize_filename`] turns whatever the sender wrote into a plain name we can save under,
//! [`attachment_name`] gives it the extension its type has when it has none, [`runs_code`] says
//! whether opening that file would start a program, and [`TYPES`] goes between a file's extension
//! and its MIME type.

mod alt_code;
pub use alt_code::{alt_code_char, alt_code_pages, alt_code_system_pages};

/// Longest name we keep, in characters (file systems allow 255 bytes; this leaves room for UTF-8
/// and for a ` (2)` added on a clash).
const LONGEST_NAME: usize = 180;

/// What is saved when nothing usable is left of the sender's name.
const FALLBACK_NAME: &str = "attachment";

/// Makes the sender's file name safe to save under. Only the part after the last `/` or `\` is
/// kept, so the name can never point into another folder. Control characters and the ones Windows
/// refuses in names become `_`. Characters that disguise a name are dropped: a right-to-left
/// override turns `invoice\u{202E}fdp.exe` into what looks like `invoiceexe.pdf`. A name Windows
/// reserves for a device (`CON`, `nul.txt`, `COM1`) gets a leading `_`, since no such file can be
/// created there. An empty result becomes `attachment`.
pub fn sanitize_filename(raw: &str) -> String {
    let last_part = raw.split(['/', '\\']).next_back().unwrap_or_default();
    let mapped: String = last_part.chars().filter_map(plain_char).collect();
    // Leading and trailing spaces and dots go; Windows drops them silently anyway.
    let mapped = mapped.trim();
    let mapped = mapped.trim_matches('.');
    let mapped = mapped.trim();
    let mut name: String = mapped.chars().take(LONGEST_NAME).collect();
    if reserved_by_windows(&name) {
        name.insert(0, '_');
    }
    if name.is_empty() {
        name.push_str(FALLBACK_NAME);
    }
    name
}

/// How one character of a name is kept: dropped (`None`) when it only changes how the name is
/// shown, `_` when a file system would refuse it, otherwise as it is.
fn plain_char(c: char) -> Option<char> {
    const REFUSED: &str = "<>:\"|?*";
    if disguises(c) {
        None
    } else if c.is_control() || REFUSED.contains(c) {
        Some('_')
    } else {
        Some(c)
    }
}

/// Characters with no glyph of their own that alter how the text around them reads.
fn disguises(c: char) -> bool {
    match c {
        // Zero-width space, non-joiner and joiner; left-to-right and right-to-left marks.
        '\u{200B}'..='\u{200F}' => true,
        // Bidirectional embeddings and overrides (LRE, RLE, PDF, LRO, RLO).
        '\u{202A}'..='\u{202E}' => true,
        // Bidirectional isolates (LRI, RLI, FSI, PDI).
        '\u{2066}'..='\u{2069}' => true,
        // Word joiner and the invisible operators.
        '\u{2060}'..='\u{2064}' => true,
        // Line and paragraph separators.
        '\u{2028}' | '\u{2029}' => true,
        // The byte-order mark, which inside a name is a zero-width no-break space.
        '\u{FEFF}' => true,
        // Arabic letter mark; Mongolian vowel separator.
        '\u{61C}' | '\u{180E}' => true,
        _ => false,
    }
}

/// Whether Windows keeps this name for a device. Only the part before the first dot counts, so
/// `prn.txt` and `COM1.log` are taken too, while `Console.txt` and `COM10.txt` are not.
fn reserved_by_windows(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or_default().trim_end().to_ascii_uppercase();
    if ["AUX", "CON", "CONIN$", "CONOUT$", "NUL", "PRN"].contains(&base.as_str()) {
        return true;
    }
    // A serial or printer port: COM or LPT and one digit, 0 to 9, or a superscript ¹ ² ³, which
    // Windows reserves too (craft-libs' file names found these).
    let port = base.strip_prefix("COM").or_else(|| base.strip_prefix("LPT"));
    let mut digit = port.unwrap_or_default().chars();
    port.is_some() && matches!((digit.next(), digit.next()), (Some('0'..='9' | '¹' | '²' | '³'), None))
}

/// Extensions whose files start code when opened, grouped by where they do it.
const EXECUTABLE_EXTENSIONS: &[&[&str]] = &[
    // Windows: programs and the things that load into them.
    &["exe", "com", "pif", "scr", "dll", "ocx", "cpl", "msc", "sys", "drv", "gadget", "xll"],
    // Windows: installers, app packages and click-to-run deployments.
    &["msi", "msp", "mst", "msix", "msixbundle", "appx", "appxbundle", "appinstaller"],
    &["application", "appref-ms", "xbap", "vsix"],
    // Windows: batch files, PowerShell and the Windows Script Host.
    &["bat", "cmd", "ps1", "psm1", "psd1", "ps1xml", "psc1"],
    &["vb", "vbs", "vbe", "js", "jse", "wsf", "wsc", "wsh", "hta", "sct"],
    // Windows: registry and setup files, help files, Access databases with code in them.
    &["reg", "inf", "ins", "isp", "chm", "hlp"],
    &["mde", "mda", "mdz", "ade", "adp", "accde"],
    // Windows: shortcuts, links and shell files that launch something.
    &["lnk", "url", "website", "scf", "library-ms", "searchconnector-ms", "settingcontent-ms", "diagcab"],
    // Windows: settings files that connect somewhere or start a command, update packages and
    // cabinets, Office add-ins and the data files that fetch or run (web queries, SYLK).
    &["rdp", "search-ms", "theme", "themepack", "wsb", "ws", "ps2", "msu", "cab"],
    &["xlam", "ppam", "xla", "ppa", "wll", "iqy", "slk"],
    // Windows: disk images that mount and run.
    &["iso", "img", "vhd", "vhdx"],
    // macOS: app bundles, disk images, installers, AppleScript and Automator.
    &["app", "dmg", "pkg", "mpkg", "applescript", "scpt", "workflow", "action", "command", "tool"],
    // macOS: link files that open a place or a program, and Terminal's settings files.
    &["inetloc", "fileloc", "webloc", "terminal"],
    // Linux and other Unix systems: packages, self-running images, launchers and shells.
    &["deb", "rpm", "appimage", "run", "bin", "desktop", "sh", "bash", "zsh", "csh", "ksh"],
    // Anywhere: interpreters and runtimes that an operating system may set up to open.
    &["jar", "jnlp", "class", "apk", "py", "pyw", "pyc", "pyz", "pyzw", "pl", "rb", "php", "lua"],
];

/// The extension of the picture or PDF that `data` is, from its first bytes: for one sent as just
/// bytes (`application/octet-stream`) with nothing else to go by.
fn sniffed_ext(data: &[u8]) -> &'static str {
    match data {
        [0x89, b'P', b'N', b'G', ..] => ".png",
        [0xFF, 0xD8, 0xFF, ..] => ".jpg",
        [b'G', b'I', b'F', b'8', ..] => ".gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => ".webp",
        [b'%', b'P', b'D', b'F', ..] => ".pdf",
        _ => "",
    }
}

/// Whether a (plain) name ends in an extension: a dot and up to 8 letters or digits.
fn has_extension(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && (1..=8).contains(&ext.len()) && ext.chars().all(|c| c.is_alphanumeric()))
}

/// A received attachment's name: the sender's, made plain, with the extension its type has when
/// it has none (`photo` sent as `image/webp` is `photo.webp`: without one the system couldn't open
/// it, nor the file it was saved as, issue #11); with no name, `attachment-N` with that
/// extension. A type that says nothing (`application/octet-stream`) goes by the first bytes.
pub(crate) fn attachment_name(given: Option<&str>, mime: &str, data: &[u8], index: usize) -> String {
    let ext = match ext_for_mime(mime) {
        "" => sniffed_ext(data),
        e => e,
    };
    match given {
        Some(g) => {
            let name = sanitize_filename(g);
            if ext.is_empty() || has_extension(&name) { name } else { format!("{name}{ext}") }
        }
        None => format!("attachment-{}{ext}", index.saturating_add(1)),
    }
}

/// Whether opening this attachment would run code: a program, script, installer, shortcut, disk
/// image and so on. Quatrelle never asks another app to open such a file; the user may still save
/// it. As with the operating system, only the last extension decides, so `report.pdf.exe` counts.
pub fn runs_code(name: &str) -> bool {
    let cleaned = sanitize_filename(name).to_ascii_lowercase();
    let Some((_, extension)) = cleaned.rsplit_once('.') else { return false };
    let extension = extension.trim();
    EXECUTABLE_EXTENSIONS.iter().any(|group| group.contains(&extension))
}

/// What opening an attachment in its usual program can do that looking at it can't: a page's
/// scripts run and reach the internet, a document's macros run, and a file whose name doesn't say
/// what it is goes to whatever the system takes it for. A file that starts a program
/// ([`runs_code`]) is never opened at all, so it is no mere risk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenRisk {
    /// A page (or a drawing a browser shows): its scripts run and it can load what it likes.
    WebPage,
    /// A document whose macros run as it opens, or an old binary Office format carrying them.
    Macros,
    /// A name that doesn't say what the file is (no extension, or one of no type known here):
    /// the system opens it by what's in it, a page as like as not.
    Unknown,
}

/// Why opening this attachment is worth asking about first, if it is: what its name says (only
/// the last extension counts, as it does for the system that would open it; the name is cleaned
/// first, so one that disguises its extension is read as it would be saved), what the type the
/// sender `declared` says whatever the name, and a name that says nothing (no extension, or one
/// of no type known here), which leaves the system to go by what's in the file. A file that
/// starts a program ([`runs_code`]) is never opened at all, so it is no mere risk.
pub fn open_risk(name: &str, declared: &str) -> Option<OpenRisk> {
    if runs_code(name) {
        return None;
    }
    let cleaned = sanitize_filename(name).to_ascii_lowercase();
    let extension = cleaned.rsplit_once('.').map_or("", |(_, e)| e.trim());
    let declared = declared.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    if WEB_PAGE_EXTENSIONS.contains(&extension) || WEB_PAGE_TYPES.contains(&declared.as_str()) {
        Some(OpenRisk::WebPage)
    } else if MACRO_EXTENSIONS.contains(&extension) || macro_type(&declared) {
        Some(OpenRisk::Macros)
    } else if extension.is_empty() || mime_from_name(&cleaned) == "application/octet-stream" {
        Some(OpenRisk::Unknown)
    } else {
        None
    }
}

/// Extensions a browser opens as a page: what it shows can run scripts and load remote content
/// (an SVG included: to a browser, a drawing is a document; XML too, through its style sheets).
const WEB_PAGE_EXTENSIONS: &[&str] = &["html", "htm", "xhtml", "xht", "shtml", "shtm", "svg", "svgz", "mht", "mhtml", "xml", "xsl", "xslt"];

/// Types a sender declares for a page a browser runs (the same as [`WEB_PAGE_EXTENSIONS`]).
const WEB_PAGE_TYPES: &[&str] =
    &["text/html", "application/xhtml+xml", "image/svg+xml", "text/xml", "application/xml", "application/xslt+xml", "multipart/related"];

/// Extensions of documents whose macros run on opening: Office's macro-enabled formats, the old
/// binary Office formats that carry macros too (Excel's macro sheets and workspaces, Word's
/// backups), LibreOffice's documents and templates, whose macros run too, RTF, which carries
/// objects that do, and the databases and publications whose code runs. Add-ins that run
/// (`.xlam`, `.xla`, `.ppam`, `.ppa`) are never opened at all ([`runs_code`]).
const MACRO_EXTENSIONS: &[&str] = &[
    "docm", "dotm", "xlsm", "xltm", "xlsb", "pptm", "potm", "ppsm", "sldm", "doc", "dot", "xls", "xlt", "xlm", "xlw", "ppt", "pot", "pps", "wbk",
    "vsd", "vss", "vst", "mdb", "accdb", "pub", "rtf", "odt", "ods", "odp", "odg", "odb", "ott", "ots", "otp", "otg", "fodt", "fods", "fodp", "fodg",
];

/// Whether a type a sender declares is one of a document whose macros run: Office's
/// macro-enabled and old binary formats, LibreOffice's, RTF.
fn macro_type(declared: &str) -> bool {
    declared.contains("macroenabled")
        || declared.starts_with("application/vnd.oasis.opendocument.")
        || matches!(
            declared,
            "application/msword"
                | "application/vnd.ms-excel"
                | "application/vnd.ms-powerpoint"
                | "application/vnd.visio"
                | "application/rtf"
                | "text/rtf"
        )
}

/// File types: an extension (with its dot) and its MIME type. A type's first extension is the one an attachment
/// sent without a name (or with a name without one) is saved with; the types after a type's own
/// are other names senders use for it.
const TYPES: &[(&str, &str)] = &[
    (".png", "image/png"),
    (".jpg", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".gif", "image/gif"),
    (".webp", "image/webp"),
    (".bmp", "image/bmp"),
    (".tif", "image/tiff"),
    (".tiff", "image/tiff"),
    (".heic", "image/heic"),
    (".avif", "image/avif"),
    (".svg", "image/svg+xml"),
    (".pdf", "application/pdf"),
    (".txt", "text/plain"),
    (".md", "text/plain"),
    (".html", "text/html"),
    (".htm", "text/html"),
    (".csv", "text/csv"),
    (".ics", "text/calendar"),
    (".vcf", "text/vcard"),
    (".zip", "application/zip"),
    (".mp4", "video/mp4"),
    (".mov", "video/quicktime"),
    (".mp3", "audio/mpeg"),
    (".wav", "audio/wav"),
    (".docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
    (".xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
    (".pptx", "application/vnd.openxmlformats-officedocument.presentationml.presentation"),
    (".doc", "application/msword"),
    (".xls", "application/vnd.ms-excel"),
    (".ppt", "application/vnd.ms-powerpoint"),
    (".odt", "application/vnd.oasis.opendocument.text"),
    (".ods", "application/vnd.oasis.opendocument.spreadsheet"),
    (".odp", "application/vnd.oasis.opendocument.presentation"),
    (".rtf", "application/rtf"),
    (".eml", "message/rfc822"),
    // Other names for the types above.
    (".jpg", "image/jpg"),
    (".jpg", "image/pjpeg"),
    (".png", "image/x-png"),
    (".bmp", "image/x-ms-bmp"),
    (".bmp", "image/x-bmp"),
    (".wav", "audio/x-wav"),
    (".vcf", "text/x-vcard"),
    (".zip", "application/x-zip-compressed"),
    (".heic", "image/heif"),
    (".ico", "image/x-icon"),
    (".ico", "image/vnd.microsoft.icon"),
];

/// The MIME type for an attachment, from its file name.
pub fn mime_from_name(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    TYPES.iter().find(|(e, _)| e.get(1..) == Some(ext.as_str())).map_or("application/octet-stream", |(_, m)| m)
}

/// The extension, with its dot, a file of this MIME type is saved with (empty for a type not
/// known here).
pub(crate) fn ext_for_mime(mime: &str) -> &'static str {
    let mime = mime.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    TYPES.iter().find(|(_, m)| *m == mime).map_or("", |(e, _)| e)
}

/// The name a picture the HTML shows goes out under: its own, made safe to save under, with the
/// extension of its type when it has none (a picture pasted in is named `local3`); none when it
/// has no name.
pub(crate) fn picture_name(name: &str, mime: &str) -> Option<String> {
    if name.trim().is_empty() {
        return None;
    }
    let mut name = sanitize_filename(name);
    if !has_extension(&name) {
        name.push_str(ext_for_mime(mime));
    }
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #11: an attachment with no name, or a name without an extension, gets the one its
    /// type has (else the one its first bytes say), so the system can open it and the file saved;
    /// a name with one keeps it, and nothing that runs code is ever added.
    #[test]
    fn attachments_get_the_extension_their_type_has() {
        let webp = b"RIFF\x10\0\0\0WEBPVP8 ";
        assert_eq!(attachment_name(None, "image/webp", webp, 0), "attachment-1.webp");
        assert_eq!(attachment_name(Some("photo"), "image/webp", webp, 0), "photo.webp");
        assert_eq!(attachment_name(Some("photo.webp"), "image/webp", webp, 0), "photo.webp");
        assert_eq!(attachment_name(Some("photo.jpg"), "image/webp", webp, 0), "photo.jpg", "the sender's extension stays");
        assert_eq!(attachment_name(None, "application/octet-stream", webp, 2), "attachment-3.webp");
        assert_eq!(attachment_name(Some("scan"), "application/octet-stream", b"%PDF-1.7", 0), "scan.pdf");
        assert_eq!(attachment_name(Some("Q3 v1.2 final"), "image/png", b"\x89PNG", 0), "Q3 v1.2 final.png");
        assert_eq!(attachment_name(Some("setup"), "application/x-msdownload", b"MZ\x90\0", 0), "setup", "nothing that runs");
        assert_eq!(attachment_name(None, "application/octet-stream", b"", usize::MAX), format!("attachment-{}", usize::MAX));
        assert_eq!(attachment_name(Some("../../x"), "image/gif", b"GIF89a", 0), "x.gif");
        assert!(!runs_code(&attachment_name(Some("tool"), "application/octet-stream", b"MZ", 0)));
    }

    /// Paths, refused characters, disguising characters and device names all come out plain;
    /// ordinary names, accents and dashes included, are left alone.
    #[test]
    fn sender_names_become_plain_file_names() {
        let cases = [
            ("../tmp\\x/../q|r*s.doc", "q_r_s.doc"),
            ("C:\\Users\\pat\\bed<3>.csv", "bed_3_.csv"),
            (". .", "attachment"),
            ("", "attachment"),
            ("photo\u{202E}gpj.scr", "photogpj.scr"),
            ("\u{200D}plan\u{2060}s\u{FEFF}.odt", "plans.odt"),
            ("\u{2067}\u{2068}\u{180E}", "attachment"),
            ("aux", "_aux"),
            ("Prn.html", "_Prn.html"),
            ("LPT9.dat", "_LPT9.dat"),
            ("conout$", "_conout$"),
            ("Auxiliary.txt", "Auxiliary.txt"),
            ("LPT0.txt", "_LPT0.txt"),
            ("com¹.txt", "_com¹.txt"),
            ("LPT³", "_LPT³"),
            ("COM⁴.txt", "COM⁴.txt"),
            ("COM12", "COM12"),
            ("Ångström – notes 3.md", "Ångström – notes 3.md"),
        ];
        for (raw, want) in cases {
            assert_eq!(sanitize_filename(raw), want, "{raw:?}");
        }
        let long = "y".repeat(400) + ".txt";
        assert_eq!(sanitize_filename(&long).chars().count(), LONGEST_NAME);
    }

    /// Whatever a name pretends to be first, its last extension says whether it runs.
    #[test]
    fn programs_and_scripts_are_told_from_documents() {
        let run = [
            "Installer.EXE",
            "minutes.docx.exe",
            "Minutes.DOCX.Exe  ",
            "photo\u{202E}gpj.scr",
            "go.cmd",
            "tidy.bat",
            "fix.ps1",
            "help.hta",
            "Report.lnk",
            "link.url",
            "image.iso",
            "Tool.app",
            "game.jar",
            "macro.vbs",
            "loader.js",
            "update.msi",
            "plugin.xll",
            // A link or a settings file that starts a program, a connection or a download.
            "Invoice.rdp",
            "viewer.jnlp",
            "open.inetloc",
            "open.fileloc",
            "site.webloc",
            "find.search-ms",
            "look.theme",
            "look.themepack",
            "shell.terminal",
            "box.wsb",
            "rates.iqy",
            "sheet.slk",
            // Packages, add-ins and compiled code.
            "patch.msu",
            "drivers.cab",
            "addin.xlam",
            "addin.ppam",
            "addin.wll",
            "Main.class",
            "tool.pyz",
            "tool.pyzw",
            "tool.pyc",
            "old.ps2",
            "job.ws",
        ];
        for name in run {
            assert!(runs_code(name), "{name} should count as code");
        }
        let safe = [
            "minutes.docx",
            "sunset.png",
            "readme.txt",
            "sums.ods",
            "talk.odp",
            "bundle.tar.gz",
            "msi",
            "the.exe.plan.md",
            "meeting.ics",
            "pat.vcf",
            "saved.eml",
            "",
        ];
        for name in safe {
            assert!(!runs_code(name), "{name} should not count as code");
        }
    }

    /// A page a browser would run scripts from, and a document whose macros run, ask first; an
    /// ordinary document, a picture and a PDF don't, and what runs code is no mere risk.
    #[test]
    fn pages_and_macro_documents_ask_before_they_open() {
        for name in ["invite.html", "Page.HTM", "plan.xhtml", "logo.svg", "drawing.SVGZ", "mail.mhtml", "photo\u{202E}gpj.svg"] {
            assert_eq!(open_risk(name, ""), Some(OpenRisk::WebPage), "{name}");
        }
        for name in
            ["budget.docm", "BUDGET.DOCM", "letter.dotm", "sheet.xlsm", "book.xlsb", "deck.pptm", "notes.doc", "sums.xls", "talk.ppt", "base.accdb"]
        {
            assert_eq!(open_risk(name, ""), Some(OpenRisk::Macros), "{name}");
        }
        for name in ["report.pdf", "photo.png", "notes.txt", "minutes.docx", "sums.xlsx", "setup.exe", "macro.vbs", "print.html.exe"] {
            assert_eq!(open_risk(name, ""), None, "{name}");
        }
    }

    /// The question comes too when the name doesn't say what the file is (no extension, or one of
    /// no type known here: the system then goes by what's in it), and when the type the sender
    /// declared is a page or a document whose macros run, whatever the name says. A known name
    /// with a type that says nothing asks nothing; what runs code is never opened at all.
    #[test]
    fn a_file_whose_name_says_nothing_or_whose_type_is_risky_asks() {
        let octets = "application/octet-stream";
        for name in ["facture", "facture.htmlx", "no-extension", ""] {
            assert_eq!(open_risk(name, octets), Some(OpenRisk::Unknown), "{name}");
        }
        for (name, declared) in [
            ("report.pdf", "text/html"),
            ("report.pdf", "Text/HTML; charset=utf-8"),
            ("drawing.png", "image/svg+xml"),
            ("page.txt", "application/xhtml+xml"),
        ] {
            assert_eq!(open_risk(name, declared), Some(OpenRisk::WebPage), "{name} as {declared}");
        }
        for (name, declared) in [("budget.xlsx", "application/vnd.ms-excel.sheet.macroEnabled.12"), ("letter.docx", "application/msword")] {
            assert_eq!(open_risk(name, declared), Some(OpenRisk::Macros), "{name} as {declared}");
        }
        for (name, declared) in [("report.pdf", "application/pdf"), ("report.pdf", octets), ("photo.png", "image/png"), ("setup.exe", octets)] {
            assert_eq!(open_risk(name, declared), None, "{name} as {declared}");
        }
    }

    /// More documents whose code runs: LibreOffice's (their macros run), XML and its style
    /// sheets (a browser runs their scripts), Excel's macro sheets and workspaces, Word's backups,
    /// RTF; Excel's and PowerPoint's old add-ins run, and are never opened at all.
    #[test]
    fn more_documents_whose_code_runs_ask_first() {
        for name in [
            "plan.odt",
            "sums.ods",
            "talk.odp",
            "sketch.odg",
            "letter.ott",
            "sheet.ots",
            "flat.fodt",
            "flat.fods",
            "macros.xlm",
            "space.xlw",
            "backup.wbk",
            "letter.rtf",
        ] {
            assert_eq!(open_risk(name, ""), Some(OpenRisk::Macros), "{name}");
        }
        for name in ["data.xml", "style.xsl", "style.xslt"] {
            assert_eq!(open_risk(name, ""), Some(OpenRisk::WebPage), "{name}");
        }
        for name in ["addin.xla", "addin.ppa"] {
            assert!(runs_code(name), "{name}");
        }
    }

    /// Types from names and names from types, the other names senders use for a type included;
    /// a name keeps an extension it has, and gets its type's when it has none.
    #[test]
    fn extensions_and_types_go_both_ways() {
        assert_eq!(mime_from_name("Beach.WEBP"), "image/webp");
        assert_eq!(mime_from_name("scan.bmp"), "image/bmp");
        assert_eq!(mime_from_name("notes.md"), "text/plain");
        assert_eq!(mime_from_name("no-extension"), "application/octet-stream");
        assert_eq!(ext_for_mime("image/webp"), ".webp");
        assert_eq!(ext_for_mime("Image/WebP; name=x"), ".webp");
        assert_eq!(ext_for_mime("image/jpeg"), ".jpg");
        assert_eq!(ext_for_mime("image/jpg"), ".jpg");
        assert_eq!(ext_for_mime("text/plain"), ".txt");
        assert_eq!(ext_for_mime("application/x-unknown"), "");
        assert_eq!(attachment_name(Some("holiday"), "image/heic", b"", 0), "holiday.heic");
        assert_eq!(attachment_name(Some("report.final.pdf"), "image/webp", b"", 0), "report.final.pdf");
        assert_eq!(attachment_name(Some("data"), "application/octet-stream", b"", 0), "data");
        // Every type's own extension names it, and is named by it.
        for (ext, mime) in TYPES.iter().filter(|(e, m)| TYPES.iter().find(|(_, m2)| m2 == m).is_some_and(|(e2, _)| e2 == e)) {
            assert_eq!(ext_for_mime(mime), *ext);
        }
    }
}
