# Vendored crates

Third-party crates copied into this repository as they are. They keep their own licence and
authors, are built as dependencies through `[patch.crates-io]`, and are not workspace members:
`Cargo.toml` excludes `vendor`, and `cargo xtask` (sizes, authors, assets, layers, fmt, clippy,
tests) leaves them alone. Never edit them here: fix them at their source, then copy them again.

## outlook-pst-rw

`outlook-pst-rw/` is the Outlook data file (`.pst`) reader and writer, `outlook-pst-rw` 1.2.14
(library `outlook_pst`, MIT, `outlook-pst-rw/LICENSE`): Dong Shin's writer over Microsoft's
reader, with the fixes Quatrelle needs, listed in `outlook-pst-rw/PATCHES.md`.

- **Where it comes from**: `crates/pst` of
  [dgaudin/outlook-pst-rs](https://github.com/dgaudin/outlook-pst-rs) (the owner's copy of
  MailCraft's `craft/rw-1.2.14` branch), commit `60cefd986b159d5f934d8291e4317dc96e0cd35b`, as
  `cargo package` makes it: the crate's files unchanged, `Cargo.toml` normalized (the source
  workspace's values written in; the original is `Cargo.toml.orig`), `.cargo_vcs_info.json` naming
  the commit, and the repository's `LICENSE` and `PATCHES.md` beside them. Its other crate,
  `compressed-rtf`, is only a dev-dependency there and is not copied.
- **Why it is here**: storytold's shared release workflow (`storytold/craft-common-actions`,
  job `git-sources`) fails before any build when `Cargo.lock` holds a git dependency outside
  `github.com/storytold/`. A path dependency in this repository passes it, and cargo-deny checks
  the copy like any other crate.
- **How to update it**: commit the new fixes in the owner's repository (its `PATCHES.md` says
  how to move to a newer release), then copy the crate again at that commit: a clone at the
  commit, `cargo package -p outlook-pst-rw --no-verify` there, the package unpacked in place of
  `vendor/outlook-pst-rw/` without its `Cargo.lock`, the repository's `LICENSE` and `PATCHES.md`
  added, and the commit above updated (the owner keeps a script that does these steps, outside
  this repository). Then `cargo xtask ci`, `cargo check --bins` in `fuzz/`, and commit the
  copy on its own. Both `Cargo.toml` and `fuzz/Cargo.toml` take this folder; `cargo xtask ci`
  checks they take the same one.

Drop the copy once a crates.io release of `outlook-pst-rw` has all of the fixes.
