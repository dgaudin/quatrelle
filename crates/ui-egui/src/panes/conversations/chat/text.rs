// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A message's text drawn ([`ChatText`], as the worker read it from Teams' HTML: the window never
//! reads that HTML): its blocks (paragraphs, quotes, code, list items) of runs in their styles,
//! line breaks inside a run, links and mentions; at most [`MAX_BLOCKS`] blocks, then "…" (a copy
//! kept before the reader's cap can hold more).
//!
//! What is hostile here: the words, the links and the names were the sender's. Control and bidi
//! characters drop out of what's drawn (a line break stays); a link opens only on a click, through
//! [`Action::OpenUrl`], and only one a browser may be handed, as written and as the window opens
//! it, out of its wrappers when links are cleaned (`parts::openable`: `http(s)`, at most 2,048
//! bytes, a plain host): any other is drawn as plain text, never as a link that does nothing. The
//! pointer on a link shows where it goes (`parts::destination`), never only its wrapper. A
//! mention shows the member's own name when it names a member, so a sender can't write another
//! name over someone's mention.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, Sense, Stroke, Ui, vec2};
use quatrelle_engine::ChatMembers;
use quatrelle_engine::model::pim::teams::{Block, BlockKind, ChatText, MAX_BLOCKS, Mention, Run, is_bidi, one_line};

use crate::Action;
use crate::theme::{self, Tokens};

/// The text's size.
const SIZE: f32 = 14.0;

/// What the body is drawn against: the user's own id, the chat's members, the bubble's colour
/// for text, and whether links are cleaned (File › Options › Safety).
pub(super) struct Context<'a> {
    pub me: &'a str,
    pub members: &'a ChatMembers,
    pub color: Color32,
    pub clean_links: bool,
}

/// A run's text as it's drawn: no control or bidi character, line breaks kept.
fn clean(text: &str) -> String {
    text.chars().filter(|&c| c == '\n' || !(c.is_control() || is_bidi(c))).collect()
}

/// Whose a mention is (of the message's `mentions`), and the name it shows: the member's own name
/// when it names a member, else the text the message gave; whether it names the user.
fn mention_of(run: &Run, mentions: &[Mention], cx: &Context) -> (String, bool) {
    let mention = run.mention.and_then(|id| mentions.iter().find(|x| x.id == id));
    let user = mention.and_then(|x| x.user_id.as_deref());
    let mine = user.is_some_and(|u| !cx.me.is_empty() && u.eq_ignore_ascii_case(cx.me));
    let member = user.and_then(|u| cx.members.get(u)).map(|x| one_line(&x.name)).filter(|n| !n.is_empty());
    (member.unwrap_or_else(|| one_line(&run.text)), mine)
}

/// A message's text (with its `mentions`) block by block, at most [`MAX_BLOCKS`] of them, `width`
/// wide; links open on a click.
pub(super) fn body(ui: &mut Ui, t: &Tokens, text: &ChatText, mentions: &[Mention], cx: &Context, width: f32, acts: &mut Vec<Action>) {
    let mut number = 0u32;
    for block in text.blocks.iter().take(MAX_BLOCKS) {
        number = match block.kind {
            BlockKind::Item { ordered: true, .. } => number.saturating_add(1),
            _ => 0,
        };
        let Some((job, links)) = layout(t, mentions, cx, block, number, width) else { continue };
        match block.kind {
            BlockKind::Quote => {
                ui.horizontal(|ui| {
                    let (bar, _) = ui.allocate_exact_size(vec2(3.0, 18.0), Sense::hover());
                    ui.painter().rect_filled(bar, 1.0, t.border);
                    ui.add_space(6.0);
                    draw(ui, t, job, &links, cx.clean_links, acts);
                });
            }
            BlockKind::Code => {
                egui::Frame::new().fill(code_fill(t)).corner_radius(4).inner_margin(6).show(ui, |ui| draw(ui, t, job, &links, cx.clean_links, acts));
            }
            BlockKind::Item { depth, .. } => {
                ui.horizontal(|ui| {
                    ui.add_space(f32::from(depth.clamp(1, 6)) * 14.0);
                    draw(ui, t, job, &links, cx.clean_links, acts);
                });
            }
            BlockKind::Paragraph => draw(ui, t, job, &links, cx.clean_links, acts),
        }
    }
    if text.blocks.len() > MAX_BLOCKS {
        ui.label(egui::RichText::new("…").size(SIZE).color(cx.color));
    }
}

fn code_fill(t: &Tokens) -> Color32 {
    if t.dark { Color32::from_rgb(0x2B, 0x2F, 0x36) } else { Color32::from_rgb(0xEE, 0xF0, 0xF3) }
}

/// A block's runs laid out: none when it has nothing to draw (only pictures, say). The links,
/// by the characters they cover.
fn layout(
    t: &Tokens,
    mentions: &[Mention],
    cx: &Context,
    block: &Block,
    number: u32,
    width: f32,
) -> Option<(LayoutJob, Vec<(std::ops::Range<usize>, String)>)> {
    let mut job = LayoutJob::default();
    let mut links = Vec::new();
    let mut at = 0usize;
    let plain = TextFormat { font_id: theme::regular(SIZE), color: cx.color, ..Default::default() };
    if let BlockKind::Item { ordered, .. } = block.kind {
        let mark = if ordered { format!("{number}. ") } else { "• ".to_string() };
        at += mark.chars().count();
        job.append(&mark, 0.0, plain.clone());
    }
    let mut drawn = false;
    for run in &block.runs {
        // Pictures are drawn under the text.
        if run.hosted.is_some() || run.outside.is_some() {
            continue;
        }
        let mut f = plain.clone();
        let text = if run.mention.is_some() {
            let (name, mine) = mention_of(run, mentions, cx);
            f.font_id = theme::bold(SIZE);
            (f.color, f.background) = if mine { (t.on_accent, t.accent) } else { (t.accent, t.accent_soft) };
            name
        } else {
            clean(&run.text)
        };
        if text.is_empty() {
            continue;
        }
        if run.bold {
            f.font_id = theme::bold(SIZE);
        }
        f.italics = run.italic;
        if run.underline {
            f.underline = Stroke::new(1.0, f.color);
        }
        if run.strike {
            f.strikethrough = Stroke::new(1.0, f.color);
        }
        if run.code || block.kind == BlockKind::Code {
            f.font_id = egui::FontId::monospace(SIZE - 1.0);
            if block.kind != BlockKind::Code {
                f.background = code_fill(t);
            }
        }
        let n = text.chars().count();
        if let Some(url) = run.link.as_deref().filter(|u| super::parts::openable(u, cx.clean_links)) {
            f.color = if f.background == t.accent { t.on_accent } else { t.accent };
            f.underline = Stroke::new(1.0, f.color);
            links.push((at..at + n, url.to_string()));
        }
        at += n;
        job.append(&text, 0.0, f);
        drawn = true;
    }
    if !drawn {
        return None;
    }
    job.wrap.max_width = width;
    Some((job, links))
}

/// A block laid out drawn; a link under the pointer shows where it goes (out of its wrappers when
/// links are `clean`ed), and opens on a click.
fn draw(ui: &mut Ui, t: &Tokens, job: LayoutJob, links: &[(std::ops::Range<usize>, String)], clean: bool, acts: &mut Vec<Action>) {
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let sense = if links.is_empty() { Sense::hover() } else { Sense::click() };
    let (rect, resp) = ui.allocate_exact_size(galley.size(), sense);
    ui.painter().galley(rect.min, galley.clone(), t.text);
    if links.is_empty() {
        return;
    }
    let under = |p: egui::Pos2| {
        let at = galley.cursor_from_pos(p - rect.min).index.0;
        links.iter().find(|(r, _)| r.contains(&at)).map(|(_, url)| url.clone())
    };
    if let Some(url) = resp.hover_pos().and_then(under) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        resp.clone().on_hover_text_at_pointer(super::parts::shown(&super::parts::destination(&url, clean)));
    }
    if resp.clicked()
        && let Some(url) = resp.interact_pointer_pos().and_then(under)
    {
        acts.push(Action::OpenUrl(url));
    }
}

/// A system line as it's drawn: on one line, without control or bidi characters, what it quotes
/// (a name, a topic) cut to 80 characters, the whole to 300.
pub(super) fn system_line(text: &str) -> String {
    let mut out = String::new();
    let mut quoted = String::new();
    let mut inside = false;
    for c in text.chars() {
        match c {
            '"' | '“' | '”' => {
                if inside {
                    out.push_str(&one_line(&quoted));
                    quoted.clear();
                }
                inside = !inside;
                out.push(c);
            }
            c if inside => quoted.push(c),
            c => out.push(c),
        }
    }
    if inside {
        out.push_str(&one_line(&quoted));
    }
    let line: String = out.split_whitespace().collect::<Vec<_>>().join(" ").chars().filter(|&c| !(c.is_control() || is_bidi(c))).collect();
    if line.chars().count() > 300 {
        let mut cut: String = line.chars().take(299).collect();
        cut.push('…');
        return cut;
    }
    line
}

#[cfg(test)]
mod tests {
    use quatrelle_engine::model::pim::teams::{ChatMessage, Member};

    use super::*;

    /// What a system line quotes is cut to 80 characters, so it can't pass for another line.
    #[test]
    fn a_system_line_cuts_what_it_quotes() {
        let long = "x".repeat(200);
        let line = system_line(&format!("Riley renamed the conversation to \"{long}\""));
        assert!(line.starts_with("Riley renamed the conversation to \""), "{line}");
        assert_eq!(line.chars().filter(|&c| c == 'x').count(), 79, "{line}");
        assert!(line.ends_with("…\""), "{line}");
        assert_eq!(system_line("Ana\u{202E} joined\n\nthe chat"), "Ana joined the chat");
        assert_eq!(system_line(&"y ".repeat(400)).chars().count(), 300);
    }

    fn message(runs: Vec<Run>) -> ChatMessage {
        ChatMessage {
            body: quatrelle_engine::model::pim::teams::ChatText { blocks: vec![Block { kind: BlockKind::Paragraph, runs }] },
            ..Default::default()
        }
    }

    /// A link a browser may be handed is one; a longer one (past 2,048 bytes) is plain text,
    /// never a link that does nothing; a line break inside a run stays in what's laid out.
    #[test]
    fn a_link_too_long_to_hand_over_is_plain_text() {
        let t = Tokens::light();
        let none = ChatMembers::default();
        let cx = Context { me: "", members: &none, color: t.text, clean_links: true };
        let link = |url: String| Run { text: "the plan".into(), link: Some(url), ..Default::default() };
        let short = message(vec![Run { text: "one\ntwo ".into(), ..Default::default() }, link("https://contoso.example/plan".into())]);
        let block = &short.body.blocks[0];
        let (job, links) = layout(&t, &short.mentions, &cx, block, 0, 400.0).unwrap();
        assert_eq!(links, vec![(8..16, "https://contoso.example/plan".to_string())]);
        assert!(job.text.contains("one\ntwo"), "the line break kept: {:?}", job.text);
        let long = message(vec![link(format!("https://contoso.example/{}", "a".repeat(2100)))]);
        let (job, links) = layout(&t, &long.mentions, &cx, &long.body.blocks[0], 0, 400.0).unwrap();
        assert!(links.is_empty(), "too long: plain text");
        assert_eq!(job.text, "the plan");
        let script = message(vec![link("javascript:alert(1)".into())]);
        assert!(layout(&t, &script.mentions, &cx, &script.body.blocks[0], 0, 400.0).unwrap().1.is_empty(), "not http(s)");
    }

    /// A mention shows the member's own name, whatever the message wrote over it (and whatever
    /// capitals Graph wrote the member's id in).
    #[test]
    fn a_mention_shows_the_members_own_name() {
        let t = Tokens::light();
        let members: ChatMembers = [Member { user_id: "U-Ana".into(), name: "Ana Exemple".into(), ..Default::default() }].into_iter().collect();
        let cx = Context { me: "u-me", members: &members, color: t.text, clean_links: true };
        let mut m = message(vec![Run { text: "The boss".into(), mention: Some(3), ..Default::default() }]);
        m.mentions = vec![quatrelle_engine::model::pim::teams::Mention { id: 3, text: "The boss".into(), user_id: Some("u-ana".into()) }];
        let (job, _) = layout(&t, &m.mentions, &cx, &m.body.blocks[0], 0, 400.0).unwrap();
        assert_eq!(job.text, "Ana Exemple");
    }

    /// Control and bidi characters drop out of a run; a line break (`<br>`) stays.
    #[test]
    fn a_run_keeps_its_line_breaks_only() {
        assert_eq!(clean("one\u{202E}\u{7}\ntwo\u{2066}"), "one\ntwo");
    }
}
