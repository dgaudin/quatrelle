# Fuzzing

Fuzz targets for the parsers that read network and disk input (CLAUDE.md, "Never crash"). This
is its own Cargo workspace, outside the main one, so the main build never needs nightly or
libFuzzer. `.github/workflows/fuzz.yml` runs every target weekly on Linux.

| Target | What it reads |
|---|---|
| `mime_parse` | a message source: parse, list summary, search text, HTML body, attachments; told signed or encrypted from its header alone, never wrongly told it isn't |
| `index_text` | a message source read for search: boundaries found, text parts decoded, files counted and named but never decoded, pictures the HTML shows told from files |
| `html` | an HTML body: text, formatted lines, columns, CSS, links and pictures; Quatrelle's markup |
| `ical` | an iCalendar file, written back and read again; a WebDAV `multistatus` answer |
| `vcard` | a vCard file, each card written back and read again |
| `http` | an HTTP answer read whole and streamed to a file (lengths, chunks, to the end), a download's `Content-Range`; GitHub's release list, `SHA256SUMS.txt`, its quota header and tags |
| `imap` | everything an IMAP server sends through a session (greeting to MOVE, with a folder sync and a message's text first) |
| `jmap` | a JMAP server's answers (split at form feeds) through sign-in, folder list and folder sync; push events; URLs |
| `search` | the search box: the query language, a query checked against a message and its text, folding, the index, the Advanced form, JMAP filters, modified UTF-7; lookalike senders, known names on other addresses, External |
| `signature` | a sender's name, address and message text (split at the first two line breaks): the author's own part, the signature's addresses and sites, the name asked of the directory, the signature checked against the sender |
| `pst` | an Outlook data file: a good exported file with the input as byte patches, or a whole file |
| `pst_item` | a `.pst` mail item rebuilt as a message from hostile property values: the headers made from them (Thread-Index, Thread-Topic, Sensitivity) stay on clean lines of their own |
| `pst_props` | text in a data file's properties: an HTML body or an 8-bit string in its code page (a declared charset, byte-order marks), and the bytes in every code page Quatrelle reads |
| `trust` | the URLs and hosts servers hand out (DAV hrefs, `EwsUrl`, `Location`): `url_host` and `site`, a site staying itself and reading back |
| `text_first` | Exchange's `GetItem` answer (XML), Graph's message (JSON) or an IMAP server's answers to a text first (`BODYSTRUCTURE`, then the header and text parts), picked by the first byte, built into a message's text and read back with `parse_text_first` and `parse`; each of Exchange's response messages read as a refusal (`check_message`, the fields its `MessageXml` names) |
| `data_file` | an account's data file as a version before R3 may have left it, damaged any way (files whole and in pieces, raw or compressed, `META` and its cursor): `META` read, compressed again as Compact Now does (stopped after a batch, gone on with, then once more), and every file that read back before reads back the same |
| `tnef` | Outlook's `winmail.dat`: its files, body and attached messages, compressed RTF and the HTML or text in it, and as a message's attachment |

## Running

Linux or macOS (or WSL), with a nightly toolchain and `cargo install cargo-fuzz`:

```bash
cd fuzz
cargo +nightly fuzz run -O imap corpus/imap seeds/imap -- -dict=dicts/imap.dict -max_total_time=300
```

`seeds/` holds a hand-written valid input per target and `dicts/` the protocol's keywords, both
to get the fuzzer past the first line quickly; `corpus/` and `artifacts/` stay out of git. A
crash leaves its input in `artifacts/<target>/`: replay it with
`cargo +nightly fuzz run -O <target> <file>`, fix the code, and add the input to a regression test
next to the code (never only to the corpus).

Every panic counts as a crash, even one Quatrelle's own `catch_unwind` fences would catch (the
HTML converter's, for instance): there the message would lose its formatting, so it is worth a
fix. The exception is `pst`, where `pst::import` fences the outlook-pst reader and reports the
file as damaged, the right answer for a damaged file: that target aborts only on a panic in
Quatrelle's own code, and a panic in the reader (built from the fixed copy pinned in
`[patch.crates-io]`) is not one. Its seeds are the patches' starting point and a whole file whose
first attachment holds a message and whose second is a zero-byte file (as `pst/tests/reader.rs`
makes it), so the reader's paths for those are fuzzed too.
