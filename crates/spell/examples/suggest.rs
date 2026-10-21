//! What the dictionaries say about a word: `cargo run -p quatrelle-spell --example suggest -- vai`

fn main() {
    let word = std::env::args().nth(1).unwrap_or_else(|| "vai".into());
    let max: usize = std::env::args().nth(2).and_then(|n| n.parse().ok()).unwrap_or(20);
    let s = quatrelle_spell::Spell::built_in().expect("dictionaries");
    println!("known: {}", s.check(&word));
    for (i, w) in s.suggest(&word, max).iter().enumerate() {
        println!("{i:2} {w}");
    }
}
