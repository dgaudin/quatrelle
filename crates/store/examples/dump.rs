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
