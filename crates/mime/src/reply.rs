//! Replies and forwards: who they go to, their subject, and the original quoted under them with
//! its header in the language they're written in.

use super::*;

/// The words the original's header is written with under a reply or forward, in the language
/// the reply is written in (the interface's: the app fills them from its catalogs). English by
/// default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuoteWords {
    pub from: String,
    pub sent: String,
    pub to: String,
    pub cc: String,
    pub subject: String,
    /// Monday first.
    pub weekdays: [String; 7],
    pub months: [String; 12],
    /// The day written out, from `{weekday}`, `{day}`, `{month}` and `{year}` (the time follows).
    pub date: String,
}

impl Default for QuoteWords {
    fn default() -> Self {
        let weekdays = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"].map(String::from);
        let months = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"]
            .map(String::from);
        QuoteWords {
            from: "From:".into(),
            sent: "Sent:".into(),
            to: "To:".into(),
            cc: "Cc:".into(),
            subject: "Subject:".into(),
            weekdays,
            months,
            date: "{weekday}, {day} {month} {year}".into(),
        }
    }
}

impl QuoteWords {
    /// When `t` was, written out: `Monday, 26 October 2026 10:07`.
    pub fn when(&self, t: i64, clock: &Clock) -> String {
        let c = quatrelle_model::dates::civil(t, clock);
        let weekday = self.weekdays.get(c.weekday as usize).map_or("", String::as_str);
        let month = self.months.get(c.month.saturating_sub(1) as usize).map_or("", String::as_str);
        let day = self
            .date
            .replace("{weekday}", weekday)
            .replace("{day}", &c.day.to_string())
            .replace("{month}", month)
            .replace("{year}", &c.year.to_string());
        format!("{day} {:02}:{:02}", c.hour, c.minute)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReplyMode {
    Reply,
    ReplyAll,
    Forward,
}

fn prefixed(prefix: &str, subject: &str) -> String {
    let base = quatrelle_model::thread::base_subject(subject);
    let already = subject.trim().to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase());
    if already { subject.trim().to_string() } else { format!("{prefix} {base}") }
}

/// The original under a reply or forward, the way Outlook lays it out: a rule, then who sent it
/// to whom and when (labels in bold, in `words`), then the original with its own formatting.
fn quoted_original(p: &Parsed, clock: &Clock, words: &QuoteWords) -> (String, rich::Styles) {
    let mut lines: Vec<(&str, String)> = vec![(&words.from, p.from.to_string()), (&words.sent, words.when(p.date, clock))];
    if !p.to.is_empty() {
        lines.push((&words.to, quatrelle_model::addr::format_list(&p.to)));
    }
    if !p.cc.is_empty() {
        lines.push((&words.cc, quatrelle_model::addr::format_list(&p.cc)));
    }
    lines.push((&words.subject, p.subject.clone()));
    // The rule: an empty paragraph drawn as a line.
    let mut text = String::new();
    let mut styles = rich::Styles::plain("");
    if let Some(first) = styles.paras.first_mut() {
        first.rule = true;
    }
    let bold = rich::Style { bold: true, ..Default::default() };
    for (label, value) in lines {
        let line = format!("\n{label} {value}");
        let mut st = rich::Styles::plain(&line);
        st.restyle(&line, 1..1 + label.chars().count(), |_| bold.clone());
        rich::append(&mut text, &mut styles, &line, &st);
    }
    let (orig, ost) = if p.had_html && !p.rich.is_empty() { rich::from_rich(&p.rich) } else { (p.text.clone(), rich::Styles::plain(&p.text)) };
    rich::append(&mut text, &mut styles, "\n\n", &rich::Styles::plain("\n\n"));
    rich::append(&mut text, &mut styles, &orig, &ost);
    (text, styles)
}

/// A reply or forward of `p`, as plain text. `me` are the user's own addresses, left out of
/// Reply All.
pub fn reply(p: &Parsed, mode: ReplyMode, me: &[String], clock: &Clock, signature: &str, words: &QuoteWords) -> Draft {
    let (mut d, styles) = reply_doc(p, mode, me, clock, signature, words);
    d.body = rich::to_text(&d.body, &styles);
    d
}

/// A reply or forward of `p` to edit: the draft (its body the editable text) and the text's
/// styles. `signature` is the signature as it goes into the message (after a `-- ` line when
/// the writer wants one; Outlook adds none), or empty. The original's header is written in
/// `words`.
pub fn reply_doc(p: &Parsed, mode: ReplyMode, me: &[String], clock: &Clock, signature: &str, words: &QuoteWords) -> (Draft, rich::Styles) {
    let mine = |a: &EmailAddress| me.iter().any(|m| m.eq_ignore_ascii_case(a.email.trim()));
    let sig = if signature.trim().is_empty() { String::new() } else { format!("\n\n{}", signature.trim_end()) };
    // The cursor's line, the signature, a blank line, then the original under its rule.
    let mut body = sig;
    let mut styles = rich::Styles::plain(&body);
    rich::append(&mut body, &mut styles, "\n\n", &rich::Styles::plain("\n\n"));
    let (orig, ost) = quoted_original(p, clock, words);
    rich::append(&mut body, &mut styles, &orig, &ost);
    let mut d = Draft::default();
    match mode {
        ReplyMode::Forward => {
            d.subject = prefixed("Fwd:", &p.subject);
        }
        ReplyMode::Reply | ReplyMode::ReplyAll => {
            d.subject = prefixed("Re:", &p.subject);
            let primary = if p.reply_to.is_empty() { vec![p.from.clone()] } else { p.reply_to.clone() };
            // Replying to your own sent message goes back to its recipients.
            d.to = if primary.iter().all(&mine) && !p.to.is_empty() { p.to.clone() } else { primary };
            if mode == ReplyMode::ReplyAll {
                let mut cc: Vec<EmailAddress> = Vec::new();
                for a in p.to.iter().chain(&p.cc) {
                    let dup = d.to.iter().chain(&cc).any(|x| x.email.eq_ignore_ascii_case(&a.email));
                    if !mine(a) && !dup {
                        cc.push(a.clone());
                    }
                }
                d.cc = cc;
            }
            if !p.message_id.is_empty() {
                d.in_reply_to = p.message_id.clone();
                d.references = p.references.clone();
                d.references.push(p.message_id.clone());
                // RFC 5322 §3.6.4 allows trimming; keep the first and the last 20.
                if d.references.len() > 21 {
                    let tail = d.references.split_off(d.references.len() - 20);
                    d.references.truncate(1);
                    d.references.extend(tail);
                }
            }
        }
    }
    d.body = body;
    (d, styles)
}
