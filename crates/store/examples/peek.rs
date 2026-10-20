//! Temporary: what the Outbox holds (read-only inspection of a real cache).
fn main() {
    let root = std::env::args().nth(1).expect("root");
    let files = std::env::args().nth(2).expect("files");
    let s = mailcraft_store::Store::open_packed(&root, &files).expect("open");
    for a in ["a1"] {
        let items = s.outbox(a);
        println!("{a}: {} queued", items.len());
        for i in &items {
            println!("  id={} not_before={} attempts={} error={:?}", i.id, i.not_before, i.attempts, i.error);
        }
    }
}
