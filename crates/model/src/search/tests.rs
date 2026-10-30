//! The search model's tests: queries, the index, and what's kept of it.

use super::*;
use crate::{EmailAddress, Flags};

fn sample() -> Summary {
    Summary {
        subject: "Project kickoff: storyboards".into(),
        from: EmailAddress::new("Zoe Park", "zoe@studio.example"),
        to: vec![EmailAddress::new("Ann", "ann@studio.example")],
        preview: "Bring the sketches for the opening scene".into(),
        has_attachments: true,
        flags: Flags { seen: false, flagged: true, ..Default::default() },
        ..Default::default()
    }
}

fn q(s: &str, m: &Summary) -> bool {
    Query::parse(s).matches(&Doc { summary: m, body: None, indexed: false, awaiting: false })
}

/// Whitespace between `field:` and its value is tolerated.
#[test]
fn whitespace_between_field_and_value() {
    let m = sample();
    assert_eq!(Query::parse("from: zoe").clauses, Query::parse("from:zoe").clauses);
    assert_eq!(Query::parse("FROM:   \"zoe park\"  -is:  read").clauses, Query::parse("from:\"zoe park\" -is:read").clauses);
    assert!(q("from: zoe", &m) && !q("from: ann", &m) && q("to: ann", &m) && q("-is: read", &m));
    // Unknown prefixes stay plain words, and so does a trailing `from:`.
    assert_eq!(Query::parse("note: zoe").clauses.len(), 2);
    assert_eq!(Query::parse("zoe from:").clauses.len(), 2);
    assert!(Query::parse("from:\t\n").clauses.len() == 1);
}

/// Sizes, and lists matching any of their names.
#[test]
fn sizes_and_lists() {
    let mut m = sample();
    m.size = 300 * 1024;
    assert!(q("larger:100KB", &m) && q("larger:100kb", &m) && !q("larger:1MB", &m) && q("smaller:0.5mb", &m) && !q("smaller:300K", &m));
    assert!(q("-larger:1MB", &m) && q("larger:307199", &m) && !q("larger:307200", &m));
    // Not sizes: plain words, never a crash.
    for junk in ["larger:", "larger:big", "larger:-5", "larger:1e999MB", "larger:NaN", "smaller:12XB", "larger:99999999999999999999GB"] {
        let _ = q(junk, &m);
    }
    assert_eq!(Query::parse("larger:99999999999999999999GB").clauses[0].term, Term::Size(true, 1024u64.pow(4)));
    assert!(q("from:ana,zoe", &m) && q("from:\"zoe park\",ann", &m) && !q("from:ana,jun", &m) && q("to:x,ann", &m));
    assert!(!q("-from:ana,zoe", &m) && q("from:zoe,", &m) && q("from:,zoe", &m));
    assert_eq!(Query::parse("from:,,").clauses.len(), 1, "commas alone: a phrase to look for");
    assert!(matches!(&Query::parse("from:a,b,c").clauses[0].term, Term::AnyOf(v) if v.len() == 3));
}

#[test]
fn parsing() {
    let p = Query::parse("from:zoe \"opening scene\" -is:read has:attachment");
    assert_eq!(p.clauses.len(), 4);
    assert_eq!(p.clauses[0].term, Term::Words(Field::From, vec!["zoe".into()]));
    assert_eq!(p.clauses[1].term, Term::Phrase(Field::Any, "opening scene".into()));
    assert!(p.clauses[2].negated);
    assert!(Query::parse("   ").is_empty());
    assert_eq!(Query::parse("subject:\"a b\"").clauses[0].term, Term::Phrase(Field::Subject, "a b".into()));
}

#[test]
fn matching() {
    let m = sample();
    assert!(q("proj", &m));
    assert!(q("from:zoe", &m));
    assert!(!q("from:ann", &m));
    assert!(q("to:ann", &m));
    assert!(q("subject:kickoff is:unread is:flagged", &m));
    assert!(q("hasattachments:yes", &m));
    assert!(!q("is:read", &m));
    assert!(q("\"opening scene\"", &m));
    assert!(!q("\"scene opening\"", &m));
    assert!(q("-from:ann", &m));
    assert!(q("zoe@studio.example", &m));
    assert!(q("body:sketches", &m));
    assert!(!q("is:important", &m) && !q("is:categorized", &m));
    let mut m2 = m.clone();
    m2.importance = 1;
    m2.flags.keywords = vec!["$Pinned".into(), "work".into()];
    assert!(q("is:important is:categorized", &m2));
    let label = Query { clauses: vec![Clause { term: Term::Categorized(vec!["$label1".into()]), negated: false }] };
    assert!(!label.matches(&Doc { summary: &m2, body: None, indexed: false, awaiting: false }));
    assert!(Query::parse("").matches(&Doc { summary: &m, body: None, indexed: false, awaiting: false }));
}

#[test]
fn index_narrows_without_missing() {
    let a = sample();
    let mut b = sample();
    b.subject = "Invoice October".into();
    b.preview = "Payment due".into();
    let mut ix = Index::new();
    ix.add(0, &summary_text(&a));
    ix.add(1, &summary_text(&b));
    assert_eq!(ix.candidates(&Query::parse("invo")), BTreeSet::from([1]));
    assert_eq!(ix.candidates(&Query::parse("zoe proj")), BTreeSet::from([0]));
    assert_eq!(ix.candidates(&Query::parse("is:unread")).len(), 2);
    assert!(ix.candidates(&Query::parse("nothing-here")).is_empty());
    ix.add(1, "late body text");
    assert_eq!(ix.candidates(&Query::parse("late")), BTreeSet::from([1]));
}

/// A search carried on while the index grows asks only about the documents added since: the
/// same answer, from a number on.
#[test]
fn candidates_from_a_number_on() {
    let mut ix = Index::new();
    for d in 0..10u32 {
        ix.add(d, if d % 3 == 0 { "storyboard review" } else { "budget story" });
    }
    let q = Query::parse("story");
    assert_eq!(ix.candidates_from(&q, 0), ix.candidates(&q));
    assert_eq!(ix.candidates_from(&q, 7).into_iter().collect::<Vec<_>>(), [7, 8, 9]);
    let q = Query::parse("storyboard review");
    assert_eq!(ix.candidates_from(&q, 4).into_iter().collect::<Vec<_>>(), [6, 9]);
    assert!(ix.candidates_from(&q, 10).is_empty());
    assert_eq!(ix.candidates_from(&Query::parse("is:unread"), 8).into_iter().collect::<Vec<_>>(), [8, 9]);
    assert!(ix.candidates_from(&Query::parse("budget"), u32::MAX).is_empty());
}

/// The index keeps each word's documents sorted and once, whatever order they come in; plain
/// words found through it need no body, while phrases, missing words and addresses do.
#[test]
fn the_index_settles_plain_words() {
    let mut ix = Index::new();
    ix.add(5, "storyboard review");
    ix.add(2, "storyboard storyboard");
    ix.add(9, "budget");
    ix.add(5, "storyboard again");
    assert_eq!(ix.prefix("story").into_iter().collect::<Vec<_>>(), [2, 5]);
    let q = Query::parse("storyboard review");
    assert_eq!(ix.candidates(&q).into_iter().collect::<Vec<_>>(), [5]);
    assert!(!q.needs_body());
    let m = Summary { subject: "Weekly".into(), ..Default::default() };
    assert!(q.matches(&Doc { summary: &m, body: None, indexed: true, awaiting: false }), "the index found both words");
    assert!(!q.matches(&Doc { summary: &m, body: None, indexed: false, awaiting: false }), "without it, the words aren't in the summary");
    for needs in ["\"storyboard review\"", "-budget storyboard", "body:storyboard", "zoe@studio.example"] {
        assert!(Query::parse(needs).needs_body(), "{needs}");
    }
    for not in ["from:zoe storyboard", "is:unread budget", "subject:weekly"] {
        assert!(!Query::parse(not).needs_body(), "{not}");
    }
}

/// An index kept between runs comes back with the same words for the same documents, and a
/// folder's index with the same messages.
#[test]
fn an_index_is_kept_and_read_back() {
    let mut a = Index::new();
    a.add(0, &summary_text(&sample()));
    a.add(1, "invoice payment due");
    a.add(2, "invoice again, storyboard review");
    a.add(7, ""); // a document with no word at all still exists
    let bytes = a.to_bytes();
    let b = Index::from_bytes(&bytes).expect("it reads back");
    assert_eq!(b.len(), 4);
    for q in ["invo", "storyboard review", "opening scene", "nothing-here"] {
        assert_eq!(a.candidates(&Query::parse(q)), b.candidates(&Query::parse(q)), "{q}");
    }
    // Nothing left over, and no word kept twice.
    assert_eq!(b.to_bytes(), bytes);
}

#[test]
fn merging_indexes_numbers_them_apart() {
    let mut folder = Index::new();
    folder.add(0, "storyboard");
    folder.add(1, "budget");
    let mut whole = Index::new();
    whole.add(0, "storyboard review");
    whole.absorb(folder, 1);
    assert_eq!(whole.len(), 3);
    assert_eq!(whole.candidates(&Query::parse("storyboard")).into_iter().collect::<Vec<_>>(), [0, 1]);
    assert_eq!(whole.candidates(&Query::parse("budget")).into_iter().collect::<Vec<_>>(), [2]);
    // The joined index is itself kept and read back.
    let back = Index::from_bytes(&whole.to_bytes()).expect("it reads back");
    assert_eq!(back.candidates(&Query::parse("storyboard")).into_iter().collect::<Vec<_>>(), [0, 1]);
}

/// Bytes that aren't an index — cut short, from another format, with a lying count or a
/// posting out of order — are refused rather than trusted.
#[test]
fn damaged_index_bytes_are_refused() {
    let mut ix = Index::new();
    ix.add(0, "storyboard review");
    ix.add(3, "budget");
    let good = ix.to_bytes();
    assert!(Index::from_bytes(&good).is_some());
    for cut in 0..good.len() {
        assert!(Index::from_bytes(&good[..cut]).is_none(), "cut at {cut}");
    }
    let mut trailing = good.clone();
    trailing.push(0);
    assert!(Index::from_bytes(&trailing).is_none());
    let mut magic = good.clone();
    magic[0] = b'X';
    assert!(Index::from_bytes(&magic).is_none());
    // A word count of four billion in four bytes.
    let mut lying = good[..6].to_vec();
    lying.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0x0f]);
    assert!(Index::from_bytes(&lying).is_none());
    // A posting list with the same document twice (its gaps must grow).
    let mut twice = Vec::new();
    twice.extend_from_slice(INDEX_MAGIC);
    twice.push(INDEX_VERSION);
    put_varint(&mut twice, 2);
    put_varint(&mut twice, 5);
    put_varint(&mut twice, 1);
    put_varint(&mut twice, 1);
    twice.push(1);
    twice.push(b'w');
    put_varint(&mut twice, 2);
    put_varint(&mut twice, 5);
    put_varint(&mut twice, 0);
    assert!(Index::from_bytes(&twice).is_none());
}

#[test]
fn a_folder_index_is_kept_and_read_back() {
    let mut fi = FolderIndex::new(42);
    for uid in 1..=3u32 {
        fi.keys.push(MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid });
        fi.has_body.push(uid == 2);
    }
    fi.index.add(0, "storyboard");
    fi.index.add(1, "body words, budget");
    fi.index.add(2, "storyboard");
    let bytes = fi.to_bytes();
    let back = FolderIndex::from_bytes("a1", "INBOX", &bytes).expect("it reads back");
    assert_eq!(back.stamp, 42);
    assert_eq!(back.keys, fi.keys);
    assert_eq!(back.has_body, [false, true, false]);
    assert_eq!(back.index.candidates(&Query::parse("storyboard")).into_iter().collect::<Vec<_>>(), [0, 2]);
    // The folder isn't kept in the file: it's the one it's read for.
    let other = FolderIndex::from_bytes("a1", "Sent", &bytes).expect("it reads back");
    assert_eq!(other.keys[0].folder, "Sent");
    for cut in [0, 3, 6, 12, bytes.len() - 1] {
        assert!(FolderIndex::from_bytes("a1", "INBOX", &bytes[..cut]).is_none(), "cut at {cut}");
    }
    let mut version = bytes.clone();
    version[4] = FOLDER_VERSION + 1;
    assert!(FolderIndex::from_bytes("a1", "INBOX", &version).is_none());
}
