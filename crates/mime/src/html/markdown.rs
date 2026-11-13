// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Markdown pasted into a message: text that is plainly Markdown (a heading and a list, bold and
//! a link…) goes in formatted. pulldown-cmark reads it (CommonMark, with its tables and
//! strikethrough) and Quatrelle writes the HTML itself, which is then pasted the way HTML from
//! another program is ([`pasted_html`]): the editor's own styles and its cleaning apply.
//!
//! What is hostile in it: the clipboard's text, of any size and nesting, with raw HTML and links
//! to anything. Only [`MARKDOWN_MAX`] bytes of it are read, nesting past [`NESTING_MAX`] is read
//! flat, raw HTML is dropped (never written out), a picture is its description (nothing is
//! fetched), a link is kept only to `http`, `https` or `mailto`, and the reader runs behind
//! `catch_unwind`.

use super::*;
use pulldown_cmark::{Event, Options, Parser, Tag};

/// The most clipboard text read as Markdown, in bytes: past it, text is pasted as it is.
pub const MARKDOWN_MAX: usize = 256 * 1024;

/// The most HTML written of pasted Markdown, in bytes: past it, the text is pasted as it is.
pub const MARKDOWN_HTML_MAX: usize = 4 * MARKDOWN_MAX;

/// Lists, quotes and other boxes open one in another past which Markdown is read flat (their
/// text kept, their boxes not).
const NESTING_MAX: usize = 24;

/// The most table cells read as tables in one text, counting those pulldown-cmark fills in
/// (it pads every row to its header's width, up to a quarter of a million cells a table, all
/// built before the first is read): past it, tables are read as the text they are written as.
/// Real tables are far below it (a hundred rows of twenty columns are 2,100 cells).
const TABLE_CELLS_MAX: usize = 1 << 16;

/// Where a line starts in the HTML written for pasting, and how the editor lays it out: a list's
/// bullet or number, its indent, a quote. html2text keeps the character in the line, after its
/// own prefixes ("• ", "1. ", indents), which [`lay_out`] takes away with it.
const MARK_FIRST: u32 = 0xE040;
const MARK_LAST: u32 = 0xE06F;

/// The marks Quatrelle's conversions put in text (`html.rs`, `fonts.rs`, `rules.rs` and the line
/// marks here): taken out of the clipboard's text, so none can be forged.
fn is_own_mark(c: char) -> bool {
    ('\u{E000}'..='\u{E06F}').contains(&c)
}

/// The first `max` bytes of `text`, cut at a character's edge.
pub(super) fn cut(text: &str, max: usize) -> &str {
    let mut end = max.min(text.len());
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text.get(..end).unwrap_or("")
}

/// Whether `text` is plainly Markdown: at least two different signs of it among a heading line
/// (`# `), a list line (`- `, `* `, `1. `), `**bold**` or `__bold__`, `` `code` `` or a fence,
/// a link to the web or to mail (`[text](https://…)`), a quote line (`>`), and a table row
/// (`| … |`) with its rule (`|---|`) under it. One sign alone is ordinary text (a dash in a
/// sentence, a line that starts with a number).
pub fn looks_like_markdown(text: &str) -> bool {
    let text = cut(text, MARKDOWN_MAX);
    // Heading, list, bold, code, link, quote, table.
    let mut signs = [false; 7];
    let mut row_above = false;
    for line in text.lines() {
        let t = line.trim_start();
        let shallow = line.len() - t.len() <= 3;
        signs[0] |= shallow && heading_line(t);
        signs[1] |= list_line(t);
        signs[2] |= paired(t, "**") || paired(t, "__");
        signs[3] |= (shallow && t.starts_with("```")) || paired(t, "`");
        signs[4] |= web_link(t);
        signs[5] |= shallow && t.starts_with('>');
        let rule = table_rule(t);
        signs[6] |= row_above && rule;
        row_above = !rule && t.len() >= 2 && t.starts_with('|') && t.trim_end().ends_with('|');
        if signs.iter().filter(|s| **s).count() >= 2 {
            return true;
        }
    }
    false
}

/// `# Title` to `###### Title`.
fn heading_line(t: &str) -> bool {
    let hashes = t.bytes().take_while(|b| *b == b'#').count();
    (1..=6).contains(&hashes) && t.get(hashes..).is_some_and(|r| r.starts_with(' ') && !r.trim().is_empty())
}

/// `- item`, `* item`, `+ item`, `1. item`, `1) item`.
fn list_line(t: &str) -> bool {
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    let rest = match digits {
        0 => t.strip_prefix(['-', '*', '+']),
        1..=9 => t.get(digits..).and_then(|r| r.strip_prefix(['.', ')'])),
        _ => None,
    };
    rest.is_some_and(|r| r.starts_with(' ') && !r.trim().is_empty())
}

/// `mark` around some text in `t` (`**this**`): the text starts and ends with something other
/// than white space. Each `mark` found is looked past once.
fn paired(t: &str, mark: &str) -> bool {
    let mut from = 0;
    while let Some(i) = t.get(from..).and_then(|r| r.find(mark)).map(|i| i + from) {
        let inner = i + mark.len();
        let opens = t.get(inner..).and_then(|r| r.chars().next()).is_some_and(|c| !c.is_whitespace() && !mark.starts_with(c));
        if opens && let Some(j) = t.get(inner..).and_then(|r| r.find(mark)).map(|j| j + inner) {
            if t.get(..j).and_then(|r| r.chars().next_back()).is_some_and(|c| !c.is_whitespace()) {
                return true;
            }
            from = j;
            continue;
        }
        from = inner;
    }
    false
}

/// `[text](https://…)`, `(http://…)` or `(mailto:…)`.
fn web_link(t: &str) -> bool {
    let (Some(open), Some(close)) = (t.find('['), t.rfind(')')) else { return false };
    let mut from = 0;
    while let Some(i) = t.get(from..).and_then(|r| r.find("](")).map(|i| i + from) {
        let to = t.get(i + 2..).unwrap_or("").get(..8).unwrap_or("").to_ascii_lowercase();
        if open < i && close > i && (to.starts_with("http://") || to.starts_with("https://") || to.starts_with("mailto:")) {
            return true;
        }
        from = i + 2;
    }
    false
}

/// A table's rule under its first row: `|---|:---:|`.
fn table_rule(t: &str) -> bool {
    let t = t.trim_end();
    t.contains('|') && t.contains("---") && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// Markdown as HTML that Quatrelle writes itself: headings, emphasis, code, lists, quotes, links
/// to the web or to mail, a table one line a row (its cells between " · ", the first row bold).
/// Raw HTML is dropped, pictures are their description. Empty when it can't be read.
pub fn markdown_html(md: &str) -> String {
    written(md, false).unwrap_or_default()
}

/// Markdown pasted into a message, as text and styles: `None` when it isn't plainly Markdown
/// ([`looks_like_markdown`]), it is longer than [`MARKDOWN_MAX`] or it holds no text, to be
/// pasted as it is. Its lists are the editor's lists and its quotes indented and grey, as the
/// editor writes them.
pub fn pasted_markdown(text: &str) -> Option<(String, rich::Styles)> {
    if text.len() > MARKDOWN_MAX || !looks_like_markdown(text) {
        return None;
    }
    let html = written(text, true)?;
    let (text, styles) = pasted_html(&html);
    let (text, styles) = lay_out(&text, &styles);
    (!text.trim().is_empty()).then_some((text, styles))
}

/// How many table cells pulldown-cmark could make of `md`, counted generously (past
/// [`TABLE_CELLS_MAX`] the count stops): each table rule's columns (its `|` and one) for its
/// header, its rule and every line after it up to a blank one, quote marks and indents before
/// them aside.
fn table_cells(md: &str) -> usize {
    let mut cells = 0usize;
    // The columns of the table the lines are in, none outside one.
    let mut columns = 0usize;
    for line in md.lines() {
        let t = line.trim_start_matches(|c: char| c == '>' || c.is_whitespace()).trim_end();
        if t.is_empty() {
            columns = 0;
            continue;
        }
        if columns > 0 {
            cells = cells.saturating_add(columns);
        } else if t.contains('|') && t.contains('-') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t')) {
            columns = t.matches('|').count().saturating_add(1);
            cells = cells.saturating_add(columns.saturating_mul(2));
        }
        if cells > TABLE_CELLS_MAX {
            break;
        }
    }
    cells
}

/// What the writer closes at an element's end.
enum Open {
    /// Its closing tag (empty: nothing).
    Tag(&'static str),
    /// A list (its closing tag): taken off the lists open.
    List(&'static str),
    /// An item: taken off the items open.
    Item,
    /// A quote (its closing tag): one fewer open.
    Quote(&'static str),
    /// A code block.
    Code,
    /// A table row.
    Row,
}

/// The HTML writer's state.
struct Writer {
    out: String,
    /// Line marks for pasting ([`MARK_FIRST`]), and quotes without `> ` before each line.
    marks: bool,
    open: Vec<Open>,
    /// The lists open (numbered or not), and for each item open how many paragraphs it has had.
    lists: Vec<bool>,
    items: Vec<usize>,
    quotes: usize,
    in_code: bool,
    in_head: bool,
    in_link: bool,
    cells: usize,
}

impl Writer {
    /// The mark that starts a line of the given kind (0 none, 1 bullet, 2 number) at `indent`.
    fn mark(&mut self, kind: u32, indent: usize) {
        if !self.marks || (kind == 0 && indent == 0 && self.quotes == 0) {
            return;
        }
        let indent = u32::try_from((indent + self.quotes.min(1)).min(7)).unwrap_or(7);
        let quote = if self.quotes > 0 { 8 } else { 0 };
        if let Some(c) = char::from_u32(MARK_FIRST + kind * 16 + quote + indent) {
            self.out.push(c);
        }
    }

    /// A line that isn't an item's first: indented under the lists it is in.
    fn line_mark(&mut self) {
        let depth = self.lists.len();
        self.mark(0, depth);
    }

    fn text(&mut self, t: &str) {
        for c in t.chars().filter(|c| !is_own_mark(*c)) {
            match c {
                '&' => self.out.push_str("&amp;"),
                '<' => self.out.push_str("&lt;"),
                '>' => self.out.push_str("&gt;"),
                '"' => self.out.push_str("&quot;"),
                '\n' if self.in_code => {
                    self.out.push('\n');
                    self.line_mark();
                }
                c => self.out.push(c),
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        // Boxes open past the cap are read flat: their text stays, they don't.
        let deep = self.open.len() >= NESTING_MAX;
        let open = match tag {
            _ if deep => Open::Tag(""),
            Tag::Paragraph if self.items.last().is_some() && self.open.last().is_some_and(|o| matches!(o, Open::Item)) => {
                // An item's paragraphs: its first on the bullet's line, the others under it.
                if let Some(n) = self.items.last_mut() {
                    *n += 1;
                    if *n > 1 {
                        self.out.push_str("<br>");
                        self.line_mark();
                    }
                }
                Open::Tag("")
            }
            Tag::Paragraph => {
                self.out.push_str("<p>");
                self.line_mark();
                Open::Tag("</p>")
            }
            Tag::Heading { level, .. } => {
                let (open, close) = match level {
                    pulldown_cmark::HeadingLevel::H1 => ("<h1>", "</h1>"),
                    pulldown_cmark::HeadingLevel::H2 => ("<h2>", "</h2>"),
                    pulldown_cmark::HeadingLevel::H3 => ("<h3>", "</h3>"),
                    pulldown_cmark::HeadingLevel::H4 => ("<h4>", "</h4>"),
                    pulldown_cmark::HeadingLevel::H5 => ("<h5>", "</h5>"),
                    pulldown_cmark::HeadingLevel::H6 => ("<h6>", "</h6>"),
                };
                self.out.push_str(open);
                self.line_mark();
                Open::Tag(close)
            }
            Tag::BlockQuote(_) => {
                self.quotes += 1;
                // Pasted, a quote is indented and grey (its lines' marks say so), without the
                // `> ` html2text would write before each of them.
                if self.marks {
                    self.out.push_str("<div>");
                    Open::Quote("</div>")
                } else {
                    self.out.push_str("<blockquote>");
                    Open::Quote("</blockquote>")
                }
            }
            Tag::CodeBlock(_) => {
                self.out.push_str("<pre><code>");
                self.line_mark();
                self.in_code = true;
                Open::Code
            }
            Tag::List(first) => {
                self.lists.push(first.is_some());
                match first {
                    Some(n) if n != 1 => {
                        self.out.push_str(&format!("<ol start=\"{n}\">"));
                        Open::List("</ol>")
                    }
                    Some(_) => {
                        self.out.push_str("<ol>");
                        Open::List("</ol>")
                    }
                    None => {
                        self.out.push_str("<ul>");
                        Open::List("</ul>")
                    }
                }
            }
            Tag::Item => {
                self.out.push_str("<li>");
                let kind = if self.lists.last().copied().unwrap_or(false) { 2 } else { 1 };
                self.mark(kind, self.lists.len().saturating_sub(1));
                self.items.push(0);
                Open::Item
            }
            Tag::TableHead | Tag::TableRow => {
                self.out.push_str("<div>");
                self.line_mark();
                self.in_head = matches!(tag, Tag::TableHead);
                self.cells = 0;
                Open::Row
            }
            Tag::TableCell => {
                if self.cells > 0 {
                    self.out.push_str(" · ");
                }
                self.cells += 1;
                if self.in_head {
                    self.out.push_str("<strong>");
                    Open::Tag("</strong>")
                } else {
                    Open::Tag("")
                }
            }
            Tag::Emphasis => {
                self.out.push_str("<em>");
                Open::Tag("</em>")
            }
            Tag::Strong => {
                self.out.push_str("<strong>");
                Open::Tag("</strong>")
            }
            Tag::Strikethrough => {
                self.out.push_str("<del>");
                Open::Tag("</del>")
            }
            Tag::Link { dest_url, .. } if !self.in_link && safe_link(&dest_url) => {
                self.out.push_str("<a href=\"");
                self.text(dest_url.trim());
                self.out.push_str("\">");
                self.in_link = true;
                Open::Tag("</a>")
            }
            // A picture is its description (the text inside it); raw HTML blocks are dropped
            // (their HTML comes as events of its own, dropped too); a table, a link to anything
            // else and what isn't read here (footnotes, definitions) leave their text alone.
            _ => Open::Tag(""),
        };
        self.open.push(open);
    }

    fn end(&mut self) {
        match self.open.pop() {
            Some(Open::Tag(close)) => {
                if close == "</a>" {
                    self.in_link = false;
                }
                self.out.push_str(close);
            }
            Some(Open::List(close)) => {
                self.lists.pop();
                self.out.push_str(close);
            }
            Some(Open::Item) => {
                self.items.pop();
                self.out.push_str("</li>");
            }
            Some(Open::Quote(close)) => {
                self.quotes = self.quotes.saturating_sub(1);
                self.out.push_str(close);
            }
            Some(Open::Code) => {
                self.in_code = false;
                // The block's last line break (and the mark after it) would start an empty line.
                if self.out.ends_with(|c: char| (MARK_FIRST..=MARK_LAST).contains(&u32::from(c))) {
                    self.out.pop();
                }
                if self.out.ends_with('\n') {
                    self.out.pop();
                }
                self.out.push_str("</code></pre>");
            }
            Some(Open::Row) => {
                self.in_head = false;
                self.out.push_str("</div>");
            }
            None => {}
        }
    }
}

/// A link kept: to the web or to mail.
fn safe_link(url: &str) -> bool {
    let l = url.trim().get(..8).unwrap_or(url.trim()).to_ascii_lowercase();
    l.starts_with("https://") || l.starts_with("http://") || l.starts_with("mailto:")
}

/// [`markdown_html`], with line marks for pasting (`marks`). `None` when the reader failed, or
/// what it wrote went past [`MARKDOWN_HTML_MAX`].
fn written(md: &str, marks: bool) -> Option<String> {
    let md = cut(md, MARKDOWN_MAX);
    // Tables only while the cells they would make are within the budget.
    let mut options = Options::ENABLE_STRIKETHROUGH;
    if table_cells(md) <= TABLE_CELLS_MAX {
        options |= Options::ENABLE_TABLES;
    }
    let done = std::panic::catch_unwind(|| {
        let mut w = Writer {
            out: String::with_capacity(md.len() + md.len() / 2),
            marks,
            open: Vec::new(),
            lists: Vec::new(),
            items: Vec::new(),
            quotes: 0,
            in_code: false,
            in_head: false,
            in_link: false,
            cells: 0,
        };
        for event in Parser::new_ext(md, options) {
            if w.out.len() > MARKDOWN_HTML_MAX {
                return None;
            }
            match event {
                Event::Start(tag) => w.start(tag),
                Event::End(_) => w.end(),
                Event::Text(t) => w.text(&t),
                Event::Code(t) => {
                    w.out.push_str("<code>");
                    w.text(&t);
                    w.out.push_str("</code>");
                }
                Event::SoftBreak => w.out.push('\n'),
                Event::HardBreak => {
                    w.out.push_str("<br>");
                    w.line_mark();
                }
                Event::Rule => w.out.push_str("<hr>"),
                // Raw HTML is never written out; the rest isn't read here.
                Event::Html(_)
                | Event::InlineHtml(_)
                | Event::FootnoteReference(_)
                | Event::TaskListMarker(_)
                | Event::InlineMath(_)
                | Event::DisplayMath(_) => {}
            }
        }
        (w.out.len() <= MARKDOWN_HTML_MAX).then_some(w.out)
    });
    done.ok().flatten()
}

/// The text and styles html2text's reading of the marked HTML gave, laid out as its line marks
/// say: what is before a mark on its line (html2text's bullets, numbers and indents) taken away
/// with it, the line's paragraph made a list item or indented, a quote's text grey. One pass,
/// whatever the text's length.
fn lay_out(text: &str, styles: &rich::Styles) -> (String, rich::Styles) {
    const QUOTE_GREY: [u8; 3] = [0x55, 0x55, 0x55];
    let mut st = styles.clone();
    st.fit(text);
    let mut palette = st.palette.clone();
    let mut grey: HashMap<u16, u16> = HashMap::new();
    let mut out = String::with_capacity(text.len());
    let mut chars: Vec<u16> = Vec::with_capacity(st.chars.len());
    let mut paras = Vec::with_capacity(st.paras.len());
    let mut at = 0usize;
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            out.push('\n');
            chars.push(st.chars.get(at.saturating_sub(1)).copied().unwrap_or(0));
        }
        let line_chars: Vec<char> = line.chars().collect();
        let mut para = st.para(n);
        let mark = line_chars.iter().position(|c| (MARK_FIRST..=MARK_LAST).contains(&u32::from(*c)));
        let mut quote = false;
        if let Some(code) = mark.and_then(|p| line_chars.get(p)).map(|c| u32::from(*c) - MARK_FIRST) {
            para.list = match code / 16 {
                1 => rich::List::Bullet,
                2 => rich::List::Number,
                _ => rich::List::None,
            };
            quote = code % 16 >= 8;
            para.indent = u8::try_from(code % 8).unwrap_or(0);
        }
        let skip = mark.map_or(0, |p| p + 1);
        for (k, c) in line_chars.iter().enumerate().skip(skip) {
            if is_own_mark(*c) {
                continue;
            }
            let mut id = st.chars.get(at + k).copied().unwrap_or(0);
            if quote {
                id = *grey.entry(id).or_insert_with(|| {
                    let mut s = palette.get(usize::from(id)).cloned().unwrap_or_default();
                    s.color = s.color.or(Some(QUOTE_GREY));
                    match palette.iter().position(|p| *p == s) {
                        Some(i) => u16::try_from(i).unwrap_or(0),
                        None if palette.len() < usize::from(u16::MAX) => {
                            palette.push(s);
                            u16::try_from(palette.len() - 1).unwrap_or(0)
                        }
                        None => id,
                    }
                });
            }
            out.push(*c);
            chars.push(id);
        }
        paras.push(para);
        at += line_chars.len() + 1;
    }
    let mut laid = rich::Styles { chars, palette, paras };
    laid.fit(&out);
    (out, laid)
}

#[cfg(test)]
mod tests;
