// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The hits against the messages: a chat's messages added in any order, edited, deleted and made
//! at another millisecond, in tranches of eight with runs of one millisecond among them (made
//! over 40 ms, or over 4: runs longer than a tranche); after each write, a search finds what the
//! live messages say, and so does the copy opened again from the file. Only the messages' hits
//! are compared: A's name ("Budget review") and B's ("Ana") give their chats too, as
//! `finds_a_word_of_a_message_and_a_chat_name` tests. A write never leaves a tranche past the
//! next one's first, so what is freed as it's read back is tested on planted files
//! (`formats.rs`).

use std::collections::HashMap;

use proptest::prelude::*;
use proptest::sample::Index as Pick;

use super::tranches::msg_at;
use super::*;

/// The words a message's text is made of.
const WORDS: [&str; 5] = ["plan", "lunch", "budget", "review", "travel"];
/// What is looked for.
const QUERIES: [&str; 6] = ["plan", "pl", "lunch", "budget review", "ana", "travel lunch"];
/// The milliseconds the messages are made in, after T0, one or the other for each case: 60
/// messages over 4 make runs of one millisecond longer than a tranche.
const WIDEST: i64 = 40;
const SPREADS: [i64; 2] = [WIDEST, 4];

/// One write of messages of the pool, as they are now.
#[derive(Clone, Debug)]
enum Step {
    /// These, in this order.
    Add(Vec<Pick>),
    /// One's text changed to these words.
    Edit(Pick, Vec<usize>),
    Delete(Pick),
    /// One made this many milliseconds later, brought within 1 to spread − 1 (round the spread):
    /// at another millisecond.
    Move(Pick, i64),
}

/// A text's words, by their places in [`WORDS`].
fn words() -> impl Strategy<Value = Vec<usize>> {
    prop::collection::vec(0..WORDS.len(), 1..=3)
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        prop::collection::vec(any::<Pick>(), 1..=12).prop_map(Step::Add),
        (any::<Pick>(), words()).prop_map(|(k, w)| Step::Edit(k, w)),
        any::<Pick>().prop_map(Step::Delete),
        (any::<Pick>(), 1..WIDEST).prop_map(|(k, by)| Step::Move(k, by)),
    ]
}

fn says(words: &[usize]) -> String {
    words.iter().map(|w| WORDS[*w]).collect::<Vec<_>>().join(" ")
}

/// The words of a message's text.
fn text_of(m: &ChatMessage) -> Vec<&str> {
    m.body.blocks.iter().flat_map(|b| &b.runs).flat_map(|r| r.text.split_whitespace()).collect()
}

/// What a search finds among the messages as the copy last took them: the live ones with, for
/// each term, a word of their text or of their sender's name beginning with it; the newest first,
/// then by id; at most [`MAX_HITS`].
fn oracle(taken: &HashMap<String, ChatMessage>, query: &str) -> Vec<(String, String)> {
    let terms: Vec<&str> = query.split_whitespace().collect();
    let mut found: Vec<(i64, &str)> = taken
        .values()
        .filter(|m| m.deleted.is_none())
        .filter(|m| {
            let words: Vec<&str> = text_of(m).into_iter().chain(["ana", "lima"]).collect();
            terms.iter().all(|t| words.iter().any(|w| w.starts_with(t)))
        })
        .map(|m| (m.created, m.id.as_str()))
        .collect();
    found.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    found.truncate(MAX_HITS);
    found.into_iter().map(|(_, id)| (A.to_string(), id.to_string())).collect()
}

/// A search's hits of messages, the chats found by their names left out.
fn message_hits(copy: &Copy, query: &str) -> Vec<(String, String)> {
    hits(copy, query).into_iter().filter(|(_, m)| !m.is_empty()).collect()
}

proptest! {
    // Each case writes up to 40 times and opens the copy again after each.
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn the_same_hits_as_the_messages_say(
        spread in prop::sample::select(SPREADS.to_vec()),
        pool in prop::collection::vec((0..WIDEST, words()), 1..=60),
        steps in prop::collection::vec(step(), 1..=40),
    ) {
        let (copy, store) = copy();
        copy.index.borrow_mut().tranche_cap = Some(8);
        // Message k as it is now: `m{k:02}`, from Ana Exemple.
        let mut now: Vec<ChatMessage> =
            pool.iter().enumerate().map(|(k, (ms, w))| msg_at(A, &format!("m{k:02}"), T0 + ms % spread, &says(w))).collect();
        // Each message as the copy last took it: what a write says changed.
        let mut taken: HashMap<String, ChatMessage> = HashMap::new();
        for (i, step) in steps.iter().enumerate() {
            let sent = match step {
                Step::Add(ks) => ks.iter().map(|k| now[k.index(now.len())].clone()).collect(),
                Step::Edit(k, w) => {
                    let k = k.index(now.len());
                    let m = &mut now[k];
                    m.body = text(&says(w));
                    m.edited = Some(T0 + WIDEST + 1_000);
                    vec![m.clone()]
                }
                Step::Delete(k) => vec![ChatMessage { deleted: Some(T0 + WIDEST + 2_000), ..now[k.index(now.len())].clone() }],
                Step::Move(k, by) => {
                    let k = k.index(now.len());
                    let m = &mut now[k];
                    let by = 1 + (by - 1) % (spread - 1);
                    m.created = T0 + (m.created - T0 + by) % spread;
                    m.modified = m.created;
                    vec![m.clone()]
                }
            };
            for m in copy.upsert(A, &sent).unwrap() {
                taken.insert(m.id.clone(), m);
            }
            prop_assert_eq!(copy.index.borrow().within_caps(), Ok(()), "step {}: {:?}", i, step);
            let again = open(&store);
            for q in QUERIES {
                let want = oracle(&taken, q);
                prop_assert_eq!(&message_hits(&copy, q), &want, "{:?}, step {}: {:?}", q, i, step);
                prop_assert_eq!(&message_hits(&again, q), &want, "{:?} read back, step {}: {:?}", q, i, step);
            }
        }
    }
}
