// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The index read back from the file at start, in slices: the chats listed, then one chat's
//! name and words a slice.

use super::*;

impl Copy {
    /// One slice of the index read back: the chats listed, or one chat's name and words. `false`
    /// when it's read whole.
    pub(crate) fn index_step(&self) -> bool {
        let mut ix = self.index.borrow_mut();
        let next = ix.todo.as_mut().map(Vec::pop);
        match next {
            None => {
                let mut hashes = match self.store.account_files(TEAMS_ACCOUNT, "chats") {
                    Ok(names) => names,
                    Err(StoreError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                    Err(_) => {
                        log::warn!("Teams search: the copy's chats couldn't be listed; nothing is searched");
                        Vec::new()
                    }
                };
                hashes
                    .retain(|n| n.strip_suffix(".json").is_some_and(|h| h.len() == 32 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))));
                hashes.sort_unstable();
                hashes.truncate(MAX_CHATS);
                // Popped from the end: read in the listing's order.
                let todo: Vec<String> = hashes.iter().rev().filter_map(|n| n.strip_suffix(".json").map(str::to_string)).collect();
                ix.partial = self.read(PARTIAL).is_some();
                ix.todo = Some(todo);
                true
            }
            Some(None) => false,
            Some(Some(hash)) => {
                if let Some(r) = self.record_by_hash(&hash) {
                    ix.name(&r.chat);
                }
                // A chat written before its turn is held already, its orphans with it.
                if !ix.segments.contains_key(&hash) {
                    match self.chat_index(&mut ix, &hash) {
                        Ok(Some(t)) => ix.put_chat(hash, t),
                        Ok(None) => {}
                        Err(_) => {
                            log::warn!("Teams search: a chat's words couldn't be read; it isn't searched");
                            self.mark_partial(&mut ix);
                        }
                    }
                }
                // Held to the bounds as it's read, not once it's whole: a planted file can't
                // make it hold more meanwhile.
                self.bound(&mut ix, None);
                true
            }
        }
    }

    /// A chat's record by its hash, when it's that chat's.
    fn record_by_hash(&self, hash: &str) -> Option<ChatRecord> {
        let r: ChatRecord = serde_json::from_slice(&self.read(&format!("chats/{hash}.json"))?).ok()?;
        (id_ok(&r.chat.id) && chat_hash(&r.chat.id) == hash).then_some(r)
    }

    /// A chat's words read back from the file, held to the caps: its tranches behind the stub (by
    /// number, at most [`SEGMENT_MAX`] bytes read, a file that isn't a tranche of the chat under
    /// its number an orphan, a message held twice or out of its tranche's range freed), or its
    /// words in the old format, cut into tranches in memory. `None` when the file has nothing of
    /// it; `Err` when the store can't say (the chat is then neither searched nor written).
    pub(super) fn chat_index(&self, ix: &mut Index, hash: &str) -> Result<Option<Tranches>> {
        let most = usize::try_from(TRANCHES_MAX).unwrap_or(usize::MAX);
        // Its tranches' numbers, ascending: names as a number is written (others never read nor
        // written); past TRANCHES_MAX, orphans.
        let names = match self.store.account_files(TEAMS_ACCOUNT, &tranches_key(hash)) {
            Ok(names) => names,
            Err(StoreError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        let mut numbers: Vec<u32> = names
            .iter()
            .filter_map(|name| {
                let n = name.strip_suffix(".json")?;
                n.parse::<u32>().ok().filter(|k| k.to_string() == n)
            })
            .collect();
        numbers.sort_unstable();
        numbers.dedup();
        let mut orphans = numbers.split_off(numbers.partition_point(|n| *n < TRANCHES_MAX));
        orphans.truncate(most);
        // Its head: the stub, or its words in the old format. One it can't read (past SEGMENT_MAX,
        // damaged, another chat's, a tranche's shape), or none beside tranches (an old version
        // takes the head away as it lets a chat's words go, maybe after it deleted a message),
        // reads as an old segment with nothing in it.
        let head = self.store.account_file(TEAMS_ACCOUNT, &segment_key(hash))?;
        let len = head.as_ref().map_or(0, Vec::len);
        let (old, chat) = match head.as_deref().and_then(|b| Draft::read(b, hash, SEGMENT_MAX)) {
            Some((d, Form::Stub)) => (None, d.chat),
            Some((d, Form::Old)) => (Some((d, len)), String::new()),
            _ if head.is_none() && numbers.is_empty() && orphans.is_empty() => return Ok(None),
            _ => {
                log::warn!("Teams search: a chat's words without a head that can be read: taken for a rollback");
                self.mark_partial(ix);
                (Some((Draft::default(), 0)), String::new())
            }
        };
        if let Some((d, len)) = old {
            if !numbers.is_empty() || !orphans.is_empty() {
                // Tranches beside a segment of the old format: the old version wrote it after
                // them, and they may hold the words of a message it deleted since.
                if self.store.remove_account_files(TEAMS_ACCOUNT, &tranches_key(hash)).is_ok() {
                    orphans.clear();
                } else {
                    log::warn!("Teams search: a chat's tranches from before a rollback couldn't leave the file");
                    orphans.append(&mut numbers);
                    orphans.sort_unstable();
                    orphans.truncate(most);
                }
                self.mark_partial(ix);
            }
            let held = d.held();
            let mut t = Tranches::from_legacy(d, len, ix.tranche_cap());
            if t.list.iter().map(|s| usize::try_from(s.held).unwrap_or(usize::MAX)).fold(0, usize::saturating_add) < held {
                self.mark_partial(ix);
            }
            t.orphans = orphans;
            return Ok(Some(t));
        }
        // Each tranche in order, to the chat's SEGMENT_MAX: past it, this one and the rest
        // orphans.
        let mut list = Vec::new();
        let mut read = 0usize;
        for (i, n) in numbers.iter().copied().enumerate() {
            let Some(bytes) = self.store.account_file(TEAMS_ACCOUNT, &tranche_key(hash, n))? else { continue };
            read = read.saturating_add(bytes.len());
            if read > SEGMENT_MAX {
                log::warn!("Teams search: a chat's words past {SEGMENT_MAX} bytes: its highest-numbered tranches passed over");
                orphans.extend(numbers.get(i..).unwrap_or_default());
                self.mark_partial(ix);
                break;
            }
            match Draft::read(&bytes, hash, TRANCHE_READ_MAX) {
                Some((d, Form::Tranche(m))) if m == n && d.held() > 0 => list.push(d.settle(n, bytes.len())),
                _ => orphans.push(n),
            }
        }
        // Held as a write finds them: a message a lower-numbered tranche holds freed, then, in
        // the chat's order, one made past the next tranche's first. A tranche freed of messages is
        // written again at the chat's next write; one left holding nothing is an orphan.
        free_held_below(&mut list);
        list.retain(|s| {
            if s.held == 0 {
                orphans.push(s.key);
            }
            s.held > 0
        });
        by_first(&mut list);
        free_past_next(&mut list);
        orphans.sort_unstable();
        orphans.dedup();
        orphans.truncate(most);
        Ok(Some(Tranches { chat, list, legacy: false, orphans }))
    }
}

/// Tranches by number, each freed of the messages a lower-numbered one holds.
fn free_held_below(list: &mut [Segment]) {
    let mut again: Vec<(usize, HashSet<String>)> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for (i, seg) in list.iter().enumerate() {
        // A tranche holds an id once (`Draft::read`).
        let ids = seg.messages.iter().map(|(id, _)| &**id).filter(|id| !id.is_empty());
        let twice: HashSet<String> = ids.clone().filter(|id| seen.contains(id)).map(str::to_string).collect();
        seen.extend(ids);
        if !twice.is_empty() {
            again.push((i, twice));
        }
    }
    for (i, twice) in again {
        let ids: HashSet<&str> = twice.iter().map(String::as_str).collect();
        if let Some(seg) = list.get_mut(i) {
            *seg = seg.freed(&ids);
        }
    }
}

/// A chat's tranches in its order ([`by_first`]), each freed of the messages made after the next
/// one's first (strictly: of two that begin in one millisecond, the first may hold only that
/// millisecond). Each keeps its first message, so the order holds.
fn free_past_next(list: &mut [Segment]) {
    let firsts: Vec<i64> = list.iter().map(|s| s.first).collect();
    for (seg, next) in list.iter_mut().zip(firsts.iter().skip(1)) {
        if seg.newest <= *next {
            continue;
        }
        let past: HashSet<&str> = seg.messages.iter().filter(|(id, at)| !id.is_empty() && at > next).map(|(id, _)| &**id).collect();
        let fixed = seg.freed(&past);
        *seg = fixed;
    }
}
