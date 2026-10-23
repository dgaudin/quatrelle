//! The fake IMAP server's commands, each answered from the shared state.

use super::*;

impl Fake {
    pub(super) fn command(&mut self, line: &str, literal: Option<Vec<u8>>) {
        let upper_line = line.to_ascii_uppercase();
        if let Some(i) = upper_line.find(" AUTHENTICATE XOAUTH2") {
            let tag = line.get(..i).unwrap_or("").to_string();
            match line.get(i + " AUTHENTICATE XOAUTH2".len()..).map(str::trim).filter(|r| !r.is_empty()) {
                Some(ir) => self.sasl_step(&tag, Some(ir)),
                None => {
                    self.say("+ ");
                    self.sasl = Some((tag, true));
                }
            }
            return;
        }
        let (tag, rest) = line.split_once(' ').unwrap_or((line, ""));
        if self.refused {
            self.say(&format!("{tag} BAD User is authenticated but not connected."));
            return;
        }
        if upper_line.contains(" SEARCH ") && upper_line.contains(" TEXT ") {
            let delay = self.st.lock().map(|s| s.search_delay).unwrap_or_default();
            std::thread::sleep(delay);
        }
        let st = self.st.clone();
        let mut st = st.lock().unwrap();
        st.log.push(rest.to_string());
        let upper = rest.to_ascii_uppercase();
        let ok = |s: &mut Fake, extra: &str| s.say(&format!("{tag} OK {extra}done"));
        if let Some((on, subject)) = st.deliver_on.clone()
            && rest.contains(on.as_str())
        {
            st.deliver_on = None;
            st.add("INBOX", &subject, &[]);
        }
        if upper == "IDLE" {
            // RFC 2177, without pushing anything: what came before IDLE isn't said again.
            self.idling = Some(tag.to_string());
            self.say("+ idling");
            return;
        }
        if upper == "NOOP"
            && let Some(flood) = st.flood.clone()
        {
            self.flood = Some(flood);
            return;
        }
        if upper == "CAPABILITY" {
            self.say(&format!("* CAPABILITY {}", st.caps));
            ok(self, "");
        } else if upper.starts_with("LOGIN ") {
            if let Some(why) = &st.refuse_passwords {
                let why = why.clone();
                self.say(&format!("{tag} NO {why}"));
            } else if rest.ends_with("\"pw\"") {
                ok(self, &format!("[CAPABILITY {}] ", st.caps));
            } else {
                self.say(&format!("{tag} NO [AUTHENTICATIONFAILED] wrong password"));
            }
        } else if upper.starts_with("GETQUOTAROOT") {
            let used: usize = st.boxes.iter().flat_map(|b| b.msgs.iter()).map(|m| m.raw.len()).sum::<usize>() / 1024 + 1;
            self.say("* QUOTAROOT INBOX \"\"");
            self.say(&format!("* QUOTA \"\" (STORAGE {used} 1024)"));
            ok(self, "");
        } else if upper.starts_with("ENABLE") {
            if upper.contains("QRESYNC") && st.caps.contains("QRESYNC") {
                self.say("* ENABLED QRESYNC");
            } else {
                self.say("* ENABLED CONDSTORE");
            }
            ok(self, "");
        } else if upper.starts_with("LIST") {
            for b in &st.boxes {
                self.say(&format!("* LIST (\\HasNoChildren{}{}) \"/\" \"{}\"", if b.attr.is_empty() { "" } else { " " }, b.attr, b.name));
            }
            ok(self, "");
        } else if upper.starts_with("SELECT ") || upper.starts_with("EXAMINE ") {
            let name = quoted_arg(rest);
            let modseq = st.modseq;
            // `(QRESYNC (validity modseq …))`
            let qresync: Option<(u32, u64)> = upper.split("(QRESYNC (").nth(1).and_then(|r| {
                let mut w = r.split([' ', ')']);
                Some((w.next()?.parse().ok()?, w.next()?.parse().ok()?))
            });
            match st.mailbox(&name) {
                Some(b) => {
                    let (n, v, next) = (b.msgs.len(), b.validity, b.uid_next);
                    self.say(&format!("* {n} EXISTS"));
                    self.say(&format!("* OK [UIDVALIDITY {v}] ok"));
                    self.say(&format!("* OK [UIDNEXT {next}] ok"));
                    self.say(&format!("* OK [HIGHESTMODSEQ {modseq}] ok"));
                    if let Some((qv, qm)) = qresync.filter(|(qv, _)| *qv == v) {
                        let _ = qv;
                        let gone: Vec<String> = b.expunged.iter().filter(|(_, m)| *m > qm).map(|(u, _)| u.to_string()).collect();
                        let changed: Vec<String> = b
                            .msgs
                            .iter()
                            .enumerate()
                            .filter(|(_, m)| m.modseq > qm)
                            .map(|(i, m)| format!("* {} FETCH (UID {} FLAGS ({}) MODSEQ ({}))", i + 1, m.uid, m.flags.join(" "), m.modseq))
                            .collect();
                        if !gone.is_empty() {
                            self.say(&format!("* VANISHED (EARLIER) {}", gone.join(",")));
                        }
                        for c in changed {
                            self.say(&c);
                        }
                    }
                    self.selected = Some(name);
                    ok(self, "[READ-WRITE] ");
                }
                None => {
                    self.selected = None;
                    self.say(&format!("{tag} NO no such mailbox"));
                }
            }
        } else if upper.starts_with("CREATE ") {
            let name = quoted_arg(rest);
            if st.mailbox(&name).is_some() {
                self.say(&format!("{tag} NO [ALREADYEXISTS] exists"));
            } else {
                let validity = 500 + u32::try_from(st.boxes.len()).unwrap_or(0);
                st.boxes.push(Mailbox { name, attr: "", validity, uid_next: 1, msgs: Vec::new(), expunged: Vec::new() });
                ok(self, "");
            }
        } else if upper.starts_with("RENAME ") {
            let mut names = rest.split('"').skip(1).step_by(2).map(str::to_string);
            let (from, to) = (names.next().unwrap_or_default(), names.next().unwrap_or_default());
            if st.mailbox(&from).is_none() || st.mailbox(&to).is_some() {
                self.say(&format!("{tag} NO cannot rename"));
            } else {
                for b in &mut st.boxes {
                    if b.name == from {
                        b.name = to.clone();
                    } else if let Some(rest) = b.name.strip_prefix(&format!("{from}/")) {
                        b.name = format!("{to}/{rest}");
                    }
                }
                ok(self, "");
            }
        } else if upper.starts_with("DELETE ") {
            let name = quoted_arg(rest);
            let before = st.boxes.len();
            st.boxes.retain(|b| b.name != name);
            if st.boxes.len() == before {
                self.say(&format!("{tag} NO [NONEXISTENT] no such mailbox"));
            } else {
                ok(self, "");
            }
        } else if upper.starts_with("SUBSCRIBE ") || upper.starts_with("UNSUBSCRIBE ") || upper == "UNSELECT" {
            if upper == "UNSELECT" {
                self.selected = None;
            }
            ok(self, "");
        } else if upper.starts_with("APPEND ") {
            let name = quoted_arg(rest);
            let flags = paren_arg(rest);
            st.modseq += 1;
            let modseq = st.modseq;
            match st.mailbox(&name) {
                Some(b) => {
                    let uid = b.uid_next;
                    b.uid_next += 1;
                    b.msgs.push(Msg { uid, flags, raw: literal.unwrap_or_default(), modseq, received: RECEIVED });
                    let v = b.validity;
                    ok(self, &format!("[APPENDUID {v} {uid}] "));
                }
                None => self.say(&format!("{tag} NO [TRYCREATE] no such mailbox")),
            }
        } else if upper.starts_with("UID ") || upper == "EXPUNGE" || upper == "NOOP" || upper == "LOGOUT" {
            let sel = self.selected.clone().unwrap_or_default();
            let modseq_now = st.modseq;
            let mut lines: Vec<Vec<u8>> = Vec::new();
            let mut extra = String::new();
            let mut moved: Vec<Msg> = Vec::new();
            let mut dest = String::new();
            // Gmail mode: an EXPUNGE in a label folder (anything but Trash/Spam) drops the label
            // and archives the message into "[Gmail]/All Mail".
            let labels = st.caps.contains("X-GM-EXT-1");
            let mut copied_from = 0;
            let mut copyuid = String::new();
            {
                let Some(b) = st.boxes.iter_mut().find(|b| b.name == sel) else {
                    if upper == "NOOP" || upper == "LOGOUT" {
                        ok(self, "");
                    } else {
                        self.say(&format!("{tag} BAD no mailbox selected"));
                    }
                    return;
                };
                b.msgs.sort_by_key(|m| m.uid);
                let max = b.msgs.last().map_or(0, |m| m.uid);
                let words: Vec<&str> = rest.split_whitespace().collect();
                let verb = words.get(1).map(|s| s.to_ascii_uppercase()).unwrap_or_default();
                if upper == "NOOP" {
                    // What a real server says to a NOOP: the folder's size, now.
                    lines.push(format!("* {} EXISTS", b.msgs.len()).into_bytes());
                } else if upper == "LOGOUT" {
                } else if verb == "SEARCH" {
                    // The keys after `SEARCH`, a literal last (as Quatrelle sends a UTF-8 word).
                    let after = rest.splitn(3, ' ').nth(2).unwrap_or("");
                    let mut keys = search_tokens(after);
                    if let Some(l) = &literal {
                        keys.push(String::from_utf8_lossy(l).into_owned());
                    }
                    let exists = u32::try_from(b.msgs.len()).unwrap_or(u32::MAX);
                    let hits: Vec<String> = b
                        .msgs
                        .iter()
                        .enumerate()
                        .filter(|(i, m)| search_matches(m, u32::try_from(*i + 1).unwrap_or(0), &keys, max, exists))
                        .map(|(_, m)| m.uid.to_string())
                        .collect();
                    lines.push(format!("* SEARCH {}\r\n", hits.join(" ")).trim_end().as_bytes().to_vec());
                } else if verb == "FETCH" {
                    let set = words.get(2).copied().unwrap_or("");
                    let since: u64 = rest.split("CHANGEDSINCE ").nth(1).and_then(|s| s.trim_end_matches(')').trim().parse().ok()).unwrap_or(0);
                    for (i, m) in b.msgs.iter().enumerate() {
                        if !in_set(m.uid, set, max) || m.modseq <= since {
                            continue;
                        }
                        let mut item = format!("* {} FETCH (UID {} FLAGS ({}) MODSEQ ({})", i + 1, m.uid, m.flags.join(" "), m.modseq).into_bytes();
                        if upper.contains("RFC822.SIZE") {
                            item.extend(format!(" RFC822.SIZE {}", m.raw.len()).as_bytes());
                        }
                        if upper.contains("INTERNALDATE") {
                            item.extend(format!(" INTERNALDATE \"{}\"", internal_date(m.received)).as_bytes());
                        }
                        if upper.contains("BODY.PEEK[HEADER]") {
                            let end = m.raw.windows(4).position(|w| w == b"\r\n\r\n").map_or(m.raw.len(), |p| p + 4);
                            item.extend(format!(" BODY[HEADER] {{{end}}}\r\n").as_bytes());
                            item.extend(&m.raw[..end]);
                        }
                        if upper.contains("BODY.PEEK[]") {
                            item.extend(format!(" BODY[] {{{}}}\r\n", m.raw.len()).as_bytes());
                            item.extend(&m.raw);
                        }
                        item.push(b')');
                        lines.push(item);
                    }
                } else if verb == "STORE" {
                    let set = words.get(2).copied().unwrap_or("");
                    let add = words.get(3).is_some_and(|w| w.starts_with('+'));
                    let flags = paren_arg(rest);
                    for m in b.msgs.iter_mut().filter(|m| in_set(m.uid, set, max)) {
                        for f in &flags {
                            m.flags.retain(|x| !x.eq_ignore_ascii_case(f));
                            if add {
                                m.flags.push(f.clone());
                            }
                        }
                        m.modseq = modseq_now + 1;
                    }
                } else if verb == "MOVE" || verb == "COPY" {
                    let set = words.get(2).copied().unwrap_or("");
                    dest = quoted_arg(rest);
                    if verb == "MOVE" {
                        let (go, stay): (Vec<Msg>, Vec<Msg>) = std::mem::take(&mut b.msgs).into_iter().partition(|m| in_set(m.uid, set, max));
                        b.msgs = stay;
                        b.expunged.extend(go.iter().map(|m| (m.uid, modseq_now + 1)));
                        moved = go;
                    } else {
                        moved = b
                            .msgs
                            .iter()
                            .filter(|m| in_set(m.uid, set, max))
                            .map(|m| Msg { uid: m.uid, flags: m.flags.clone(), raw: m.raw.clone(), modseq: 0, received: m.received })
                            .collect();
                    }
                    copied_from = b.validity;
                } else if verb == "EXPUNGE" || upper == "EXPUNGE" {
                    let set = words.get(2).copied().unwrap_or("1:*");
                    let gone: Vec<u32> =
                        b.msgs.iter().filter(|m| in_set(m.uid, set, max) && m.flags.iter().any(|f| f == "\\Deleted")).map(|m| m.uid).collect();
                    if labels && !matches!(b.attr, "\\Trash" | "\\Junk") {
                        moved = b
                            .msgs
                            .iter()
                            .filter(|m| gone.contains(&m.uid))
                            .map(|m| Msg { uid: m.uid, flags: Vec::new(), raw: m.raw.clone(), modseq: 0, received: m.received })
                            .collect();
                        dest = "[Gmail]/All Mail".into();
                    }
                    b.msgs.retain(|m| !gone.contains(&m.uid));
                    b.expunged.extend(gone.iter().map(|u| (*u, modseq_now + 1)));
                } else {
                    extra = "BAD".into();
                }
            }
            if !moved.is_empty() || !dest.is_empty() {
                st.modseq += 1;
                let ms = st.modseq;
                if let Some(d) = st.mailbox(&dest) {
                    let (mut from, mut to) = (Vec::new(), Vec::new());
                    for mut m in moved {
                        from.push(m.uid.to_string());
                        m.uid = d.uid_next;
                        to.push(m.uid.to_string());
                        d.uid_next += 1;
                        m.modseq = ms;
                        d.msgs.push(m);
                    }
                    if copied_from != 0 && !from.is_empty() {
                        copyuid = format!("[COPYUID {} {} {}] ", d.validity, from.join(","), to.join(","));
                    }
                }
            } else if upper.contains("STORE") || upper.contains("EXPUNGE") {
                st.modseq += 1;
            }
            for l in lines {
                self.out.extend(l);
                self.out.extend(b"\r\n");
            }
            if extra == "BAD" {
                self.say(&format!("{tag} BAD unsupported"));
            } else {
                if upper == "LOGOUT" {
                    self.say("* BYE bye");
                }
                ok(self, &copyuid);
            }
        } else {
            self.say(&format!("{tag} BAD unsupported"));
        }
    }
}

fn quoted_arg(s: &str) -> String {
    s.split('"').nth(1).unwrap_or_default().to_string()
}

fn paren_arg(s: &str) -> Vec<String> {
    let Some(open) = s.find('(') else { return Vec::new() };
    let close = s[open..].find(')').map_or(s.len(), |c| open + c);
    s[open + 1..close].split_whitespace().map(str::to_string).collect()
}
