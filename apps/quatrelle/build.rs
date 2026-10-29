//! Windows only: embed the app icon, version info (VERSIONINFO) and the application manifest
//! (per-monitor DPI awareness: `src/windows_manifest.rs`) into `quatrelle.exe`, so it
//! shows in Explorer, the taskbar, the Start menu and Alt-Tab.
//!
//! On every other target this does nothing. A missing resource compiler is a warning, so a
//! cross-compile from macOS or Linux still links, unless `QUATRELLE_REQUIRE_WINRES=1` turns it
//! into an error (release builds: packaging/windows/package.ps1).

#[path = "src/windows_manifest.rs"]
mod windows_manifest;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/windows_manifest.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/quatrelle.ico");
    println!("cargo:rerun-if-env-changed=QUATRELLE_REQUIRE_WINRES");
    println!("cargo:rerun-if-env-changed=QUATRELLE_BUILD_SHA");
    println!("cargo:rerun-if-env-changed=QUATRELLE_BUILD_DATE");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/quatrelle.ico")
        .set_manifest(windows_manifest::WINDOWS_MANIFEST)
        .set("ProductName", "Quatrelle")
        .set("FileDescription", "Quatrelle email")
        .set("CompanyName", "Didier Gaudin")
        .set("LegalCopyright", "Copyright (c) the Quatrelle authors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "quatrelle.exe")
        .set("InternalName", "quatrelle");
    if let Err(e) = res.compile() {
        if std::env::var_os("QUATRELLE_REQUIRE_WINRES").is_some() {
            eprintln!("embedding Windows resources failed: {e}");
            std::process::exit(1);
        }
        println!("cargo:warning=quatrelle.exe built without icon/version resources: {e}");
    }
}
