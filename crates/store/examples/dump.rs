//! Read a packed data file (redb) outside the app: `cargo run -p quatrelle-store --example dump -- <file> [path]`.
//!
//! With no path, every file's path and size is listed; with one, that file is written to stdout
//! (unpacked: the deflate marker is taken off, a file kept in pieces is put back together).

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

const FILES: TableDefinition<&str, &[u8]> = TableDefinition::new("files");
const PIECES: TableDefinition<(&str, u32), &[u8]> = TableDefinition::new("pieces");
const PIECED: &[u8] = b"\0Quatrelle:pieces\0";
const PIECED_OLD: &[u8] = b"\0MailCraft:pieces\0";
const DEFLATED: &[u8] = b"\0Quatrelle:deflate\0";
const DEFLATED_OLD: &[u8] = b"\0MailCraft:deflate\0";

/// A file's bytes as written: the deflate marker taken off when it's there.
fn unpack_kept(kept: &[u8]) -> Vec<u8> {
    let mark = if kept.starts_with(DEFLATED) {
        DEFLATED.len()
    } else if kept.starts_with(DEFLATED_OLD) {
        DEFLATED_OLD.len()
    } else {
        return kept.to_vec();
    };
    let mut out = Vec::new();
    let mut d = flate2::read::DeflateDecoder::new(&kept[mark..]);
    let _ = std::io::Read::read_to_end(&mut d, &mut out);
    out
}

/// A file's bytes, from the FILES table or its pieces, deflate taken off.
fn read_file(tx: &redb::ReadTransaction, name: &str) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
    let files = tx.open_table(FILES)?;
    let Some(value) = files.get(name)?.map(|v| v.value().to_vec()) else { return Ok(None) };
    if value.starts_with(PIECED) || value.starts_with(PIECED_OLD) {
        let _mark = if value.starts_with(PIECED) { PIECED.len() } else { PIECED_OLD.len() };
        let pieces = tx.open_table(PIECES)?;
        let mut whole = Vec::new();
        for n in 0.. {
            let Some(p) = pieces.get((name, n))? else { break };
            whole.extend_from_slice(p.value());
        }
        return Ok(Some(unpack_kept(&whole).clone()));
    }
    Ok(Some(unpack_kept(&value)))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("a data file");
    let wanted = args.next();
    let db = Database::open(std::path::Path::new(&path))?;
    let tx = db.begin_read()?;
    let files = tx.open_table(FILES)?;
    match wanted {
        None => {
            for row in files.iter()? {
                let (k, v) = row?;
                println!("{}\t{}", k.value(), v.value().len());
            }
        }
        // `@states`: each folder's state, as a line of its own (how much of it is kept here).
        Some(name) if name == "@states" => {
            let mut rows: Vec<(String, usize, i64)> = Vec::new();
            for row in files.iter()? {
                let (k, v) = row?;
                let key = k.value();
                if !key.ends_with("/state.json") {
                    continue;
                }
                let _ = v;
                let Some(raw) = read_file(&tx, key)? else { continue };
                let Ok(json) = serde_json::from_slice::<serde_json::Value>(&raw) else { continue };
                let messages = json.get("messages").and_then(|m| m.as_array()).cloned().unwrap_or_default();
                let oldest = messages.iter().filter_map(|m| m.get("date").or_else(|| m.get("received")).and_then(|d| d.as_i64())).min().unwrap_or(0);
                rows.push((key.to_string(), messages.len(), oldest));
            }
            rows.sort_by_key(|r| std::cmp::Reverse(r.1));
            for (key, n, oldest) in rows {
                println!("{key}\t{n}\t{oldest}");
            }
        }
        Some(name) => {
            let value = files.get(name.as_str())?.map(|v| v.value().to_vec());
            let Some(mut bytes) = value else {
                eprintln!("no such file: {name}");
                std::process::exit(1);
            };
            if bytes.starts_with(PIECED) || bytes.starts_with(PIECED_OLD) {
                let mark = if bytes.starts_with(PIECED) { PIECED.len() } else { PIECED_OLD.len() };
                let len = u64::from_le_bytes(bytes[mark..mark + 8].try_into()?) as usize;
                let pieces = tx.open_table(PIECES)?;
                let mut whole = Vec::with_capacity(len);
                for n in 0.. {
                    let Some(p) = pieces.get((name.as_str(), n))? else { break };
                    whole.extend_from_slice(p.value());
                }
                bytes = whole;
            }
            if bytes.starts_with(DEFLATED) || bytes.starts_with(DEFLATED_OLD) {
                let mark = if bytes.starts_with(DEFLATED) { DEFLATED.len() } else { DEFLATED_OLD.len() };
                let mut out = Vec::new();
                let mut d = flate2::read::DeflateDecoder::new(&bytes[mark..]);
                std::io::Read::read_to_end(&mut d, &mut out)?;
                bytes = out;
            }
            std::io::Write::write_all(&mut std::io::stdout(), &bytes)?;
        }
    }
    Ok(())
}
