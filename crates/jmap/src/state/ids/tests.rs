use super::*;

/// An id shaped like Exchange's: a mailbox's bytes, then a folder's and a message's numbers.
fn exchange(mailbox: u8, folder: u32, message: u32) -> String {
    let mut b: Vec<u8> = vec![0, 3, 36, 0];
    b.extend(format!("{mailbox:02x}6f2c1d-5e4a-4b3c-9d8e-7f6a5b4c3d2e").bytes());
    b.extend([0, 70, 0, 0, 0, 0]);
    b.extend((0..22u8).map(|i| i.wrapping_mul(mailbox | 1)));
    b.extend(folder.to_be_bytes());
    b.extend([7, 0]);
    b.extend((0..22u8).map(|i| i.wrapping_mul(13)));
    b.extend(message.to_be_bytes());
    b.extend(message.wrapping_mul(2_654_435_761).to_le_bytes());
    b.extend([0, 0]);
    base64(&b)
}

#[test]
fn a_code_is_sixteen_bytes() {
    assert_eq!(std::mem::size_of::<Code>(), 16);
}

/// Exchange's ids are kept as their differences from a few templates, and every one gives its
/// number and comes back exactly.
#[test]
fn exchange_ids_kept_small_and_exact() {
    let mut ids = Ids::default();
    let mut n = 0;
    let mut all = Vec::new();
    for mailbox in 0..3u8 {
        for folder in 0..20u32 {
            for message in 0..200u32 {
                n += 1;
                let id = exchange(mailbox, folder, message * 7919);
                ids.insert(&id, n);
                all.push((id, n));
            }
        }
    }
    assert!(ids.whole.is_empty(), "none kept whole: {ids:?}");
    assert!(ids.templates.len() <= 12, "a few templates: {ids:?}");
    for (id, n) in &all {
        assert_eq!(ids.get(id), Some(*n));
        assert_eq!(ids.id(*n).as_deref(), Some(id.as_str()));
    }
    assert_eq!(ids.len(), all.len());
    assert_eq!(ids.get(&exchange(1, 3, 5)), None, "one never numbered has no number");
}

/// An id told against a template still finds its number once later ones are made, even one that
/// would suit it better: the first that fits is taken, and templates are only added to.
#[test]
fn later_templates_never_change_a_code() {
    let mut ids = Ids::default();
    let first = exchange(0, 1, 1);
    ids.insert(&first, 1);
    let close = exchange(0, 1, 2);
    ids.insert(&close, 2);
    let before = ids.told(&close);
    // Many others, some of them templates of their own.
    let mut n = 2;
    for mailbox in 0..40u8 {
        for message in 0..5u32 {
            n += 1;
            ids.insert(&exchange(mailbox, mailbox.into(), message), n);
        }
    }
    assert!(ids.templates.len() > 1);
    assert_eq!(ids.told(&close), before, "the same code");
    assert_eq!((ids.get(&first), ids.get(&close)), (Some(1), Some(2)));
}

/// Ids that aren't base64 as written, short ones and the URL's alphabet: each comes back as it
/// was, and none takes another's number.
#[test]
fn every_other_id_comes_back_as_it_was() {
    let long = exchange(0, 1, 1);
    let others = [
        "id-a".to_string(),
        "M1".to_string(),
        String::new(),
        "exactly-15-char".to_string(),
        "sixteen-chars-xx".to_string(),
        // Base64 a decoder reads but wouldn't write: no padding, a line break, stray bits.
        long.trim_end_matches('=').to_string(),
        format!("{}\r\n{}", &long[..76], &long[76..]),
        format!("{}B=", &long[..long.len() - 2]),
        // The URL's alphabet, and both mixed.
        long.replace('+', "-").replace('/', "_"),
        format!("-{}", &long[1..]),
        format!("+-{}", &long[2..]),
        "é".repeat(20),
        "x".repeat(400),
        base64(&[9u8; 300]),
    ];
    let mut ids = Ids::default();
    ids.insert(&long, 1);
    for (i, id) in others.iter().enumerate() {
        ids.insert(id, 10 + i as u32);
    }
    assert_eq!(ids.get(&long), Some(1));
    for (i, id) in others.iter().enumerate() {
        assert_eq!(ids.get(id), Some(10 + i as u32), "{id:?}");
        assert_eq!(ids.id(10 + i as u32).as_deref(), Some(id.as_str()), "{id:?}");
    }
    assert_eq!(ids.len(), others.len() + 1);
}

/// A number given again to another id (Exchange gives a moved message a new id) stands for the
/// new one, and the old id still finds it, as before.
#[test]
fn a_number_given_again_stands_for_its_latest_id() {
    let mut ids = Ids::default();
    let (old, new) = (exchange(0, 1, 1), exchange(0, 2, 1));
    ids.insert(&old, 5);
    ids.insert(&new, 5);
    assert_eq!(ids.id(5).as_deref(), Some(new.as_str()));
    assert_eq!((ids.get(&old), ids.get(&new)), (Some(5), Some(5)));
    // So in a copy that takes another's.
    let mut main = Ids::default();
    main.insert(&old, 5);
    main.absorb(&ids);
    assert_eq!(main.id(5).as_deref(), Some(new.as_str()));
    // And whole ids alike.
    ids.insert("not base64 at all, and long", 5);
    assert_eq!(ids.id(5).as_deref(), Some("not base64 at all, and long"));
    ids.insert(&old, 5);
    assert_eq!(ids.id(5).as_deref(), Some(old.as_str()));
}

/// Written, the ids themselves (what earlier versions wrote and read); read back, the same.
#[test]
fn written_as_the_ids_themselves() {
    let mut ids = Ids::default();
    let a = exchange(0, 1, 1);
    ids.insert(&a, 1);
    ids.insert("short", 2);
    ids.insert("not base64, kept whole", 3);
    let json = serde_json::to_value(&ids).unwrap_or_default();
    assert_eq!(json, serde_json::json!({a.clone(): 1, "short": 2, "not base64, kept whole": 3}));
    let back: Ids = serde_json::from_value(json).unwrap_or_default();
    assert_eq!((back.get(&a), back.get("short"), back.get("not base64, kept whole")), (Some(1), Some(2), Some(3)));
    assert_eq!(back.id(1).as_deref(), Some(a.as_str()));
    // A damaged list reads as none.
    assert!(serde_json::from_value::<Ids>(serde_json::json!({"a": "one"})).is_err());
}

/// Past the last template, ids unlike them all are kept whole, and still come back.
#[test]
fn past_the_last_template_ids_are_kept_whole() {
    let mut ids = Ids::default();
    // Each unlike the others nearly everywhere (a generator's bytes, after its own number).
    let unlike: Vec<String> = (0..300u32)
        .map(|i| {
            let mut x = u64::from(i).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
            let mut b = i.to_be_bytes().to_vec();
            b.extend((0..116).map(|_| {
                x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                (x >> 56) as u8
            }));
            base64(&b)
        })
        .collect();
    for (i, id) in unlike.iter().enumerate() {
        ids.insert(id, i as u32);
    }
    assert_eq!(ids.templates.len(), TEMPLATES);
    assert!(!ids.whole.is_empty());
    for (i, id) in unlike.iter().enumerate() {
        assert_eq!(ids.get(id), Some(i as u32));
        assert_eq!(ids.id(i as u32).as_deref(), Some(id.as_str()));
    }
}

/// Forgetting numbers forgets their ids, both ways.
#[test]
fn retain_forgets_both_ways() {
    let mut ids = Ids::default();
    let (a, b) = (exchange(0, 1, 1), exchange(0, 1, 2));
    ids.insert(&a, 1);
    ids.insert(&b, 2);
    ids.insert("kept whole, as it's not base64", 3);
    ids.retain(|n| n == 2);
    assert_eq!((ids.get(&a), ids.get(&b), ids.get("kept whole, as it's not base64")), (None, Some(2), None));
    assert_eq!((ids.id(1), ids.id(3)), (None, None));
    assert_eq!(ids.numbers().collect::<Vec<u32>>(), vec![2]);
}
