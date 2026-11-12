// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Teams' messages read into [`ChatText`]: the subset of HTML a chat message may hold (bold,
//! italic, underline, strike, `http(s)` links, lists, quotes, code, `<at>` mentions, `<emoji>` as
//! its alt text, pictures, `<attachment>` placeholders). Any other tag goes but its text stays;
//! scripts and styles go with their content.
//!
//! The sender writes this HTML: it is read within caps on what comes in ([`TEAMS_HTML_MAX`]),
//! on how deep it nests ([`TEAMS_DEPTH`]) and on what is kept ([`TEAMS_RUNS`], and the same
//! bytes as came in). A picture is this message's own hosted image only when its address is
//! exactly Graph's for this chat and this message; any other picture on the web is an outside
//! one, which waits for a click. Nothing here fetches anything.

use std::cell::RefCell;

use html5ever::TokenizerResult;
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts};
use quatrelle_model::pim::teams::{Block, BlockKind, ChatText, MAX_BODY, MAX_ID, Run, is_bidi};

/// The most bytes of a message's HTML read: the rest is left out (cut where a character starts).
pub const TEAMS_HTML_MAX: usize = 256 << 10;
/// Elements open one in another past which the text is kept flat (their styles left out).
pub const TEAMS_DEPTH: usize = 32;
/// The most runs kept of a message.
pub const TEAMS_RUNS: usize = 20_000;
/// The longest link or picture address kept.
const ADDRESS_MAX: usize = 4096;
/// The most characters of an emoji's alt text kept.
const EMOJI_CHARS: usize = 32;
/// Where Graph keeps a message's hosted images.
const HOSTED: &str = "https://graph.microsoft.com/v1.0/chats/";

/// A message's HTML body (`contentType: html`) as its text, `chat` and `message` its own ids.
///
/// The caller guards it: it reads with html5ever, a third-party parser, so the caller calls it
/// behind `catch_unwind` at the message's boundary (the network layer reading a message or a
/// chat's preview), which stores a message that panics as one that can't be shown.
pub fn teams_html(html: &str, chat: &str, message: &str) -> ChatText {
    read(cut(html, TEAMS_HTML_MAX), chat, message)
}

/// A message's plain-text body (`contentType: text`): one paragraph, its lines as they came,
/// control characters out but line breaks and tabs. The caller guards it, as [`teams_html`].
pub fn teams_text(text: &str) -> ChatText {
    let text: String = cut(text, MAX_BODY).chars().filter(|&c| c == '\n' || c == '\t' || !c.is_control()).collect();
    if text.trim().is_empty() {
        return ChatText::default();
    }
    ChatText { blocks: vec![Block { kind: BlockKind::Paragraph, runs: vec![Run { text, ..Run::default() }] }] }
}

/// `s` cut to at most `max` bytes, where a character starts.
fn cut(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    s.get(..end).unwrap_or_default()
}

fn read(html: &str, chat: &str, message: &str) -> ChatText {
    let reader = Reader {
        chat,
        message,
        base: Context::default(),
        stack: Vec::new(),
        raw: None,
        blocks: Vec::new(),
        block: None,
        space: None,
        line_start: true,
        budget: TEAMS_HTML_MAX,
        runs: 0,
        full: false,
    };
    let tok = Tokenizer::new(Sink(RefCell::new(reader)), TokenizerOpts::default());
    let queue = BufferQueue::default();
    queue.push_back(StrTendril::from(html));
    while let TokenizerResult::Script(()) = tok.feed(&queue) {}
    tok.end();
    let mut reader = tok.sink.0.into_inner();
    reader.flush();
    ChatText { blocks: reader.blocks }
}

/// What the text inside an element is: its style, and the block it goes in.
#[derive(Clone, Debug, Default)]
struct Context {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    code: bool,
    link: Option<String>,
    mention: Option<u32>,
    /// In a link or a mention, whose text loses bidi characters too.
    vouched: bool,
    /// Lists open (`ul`, `ol`), and whether the innermost is numbered.
    lists: u8,
    ordered: bool,
    item: bool,
    quote: bool,
    code_block: bool,
    /// In an emoji or an attachment placeholder, whose content is left out.
    hidden: bool,
}

impl Context {
    fn kind(&self) -> BlockKind {
        if self.code_block {
            BlockKind::Code
        } else if self.item {
            BlockKind::Item { ordered: self.ordered, depth: self.lists.max(1) }
        } else if self.quote {
            BlockKind::Quote
        } else {
            BlockKind::Paragraph
        }
    }

    /// Whether a run's text can go on with text of this context.
    fn styles(&self, r: &Run) -> bool {
        r.hosted.is_none()
            && r.outside.is_none()
            && r.attachment.is_none()
            && (r.bold, r.italic, r.underline, r.strike, r.code, r.mention)
                == (self.bold, self.italic, self.underline, self.strike, self.code, self.mention)
            && r.link == self.link
    }

    fn run(&self) -> Run {
        Run {
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            strike: self.strike,
            code: self.code,
            link: self.link.clone(),
            mention: self.mention,
            ..Run::default()
        }
    }
}

/// An element open, and what the text inside it is.
struct Open {
    name: String,
    inside: Context,
}

/// What a picture or a placeholder is.
enum Object {
    Hosted(String),
    Outside(String),
    Attachment(String),
}

struct Reader<'a> {
    chat: &'a str,
    message: &'a str,
    /// The context outside every element.
    base: Context,
    stack: Vec<Open>,
    /// The element whose content is being left out (`script`, `style`…).
    raw: Option<String>,
    blocks: Vec<Block>,
    block: Option<Block>,
    /// A space seen and not yet written (in the context it was seen in): written only before
    /// more text on the same line.
    space: Option<Context>,
    /// Whether nothing has been written yet on this line of the block.
    line_start: bool,
    /// Bytes left to keep: text, block breaks, and the addresses and ids runs carry.
    budget: usize,
    runs: usize,
    /// Whether a cap is reached: nothing more is kept.
    full: bool,
}

impl Reader<'_> {
    fn here(&self) -> &Context {
        self.stack.last().map_or(&self.base, |o| &o.inside)
    }

    /// Takes `cost` bytes of the budget, or marks the reader full.
    fn spend(&mut self, cost: usize) -> bool {
        match self.budget.checked_sub(cost) {
            Some(left) if !self.full => {
                self.budget = left;
                true
            }
            _ => {
                self.full = true;
                false
            }
        }
    }

    /// The block being written, opened in the kind of `ctx` if there is none.
    fn open_block(&mut self, ctx: &Context) -> bool {
        if self.block.is_some() {
            return true;
        }
        // A block after another is a line break in the text.
        if !self.spend(usize::from(!self.blocks.is_empty())) {
            return false;
        }
        self.block = Some(Block { kind: ctx.kind(), runs: Vec::new() });
        true
    }

    /// A new run in the block being written, or `None` when the runs are all used.
    fn new_run(&mut self, run: Run) -> Option<&mut Run> {
        let carried = run.link.as_ref().map_or(0, String::len);
        if self.runs >= TEAMS_RUNS || !self.spend(carried) {
            self.full = true;
            return None;
        }
        self.runs += 1;
        let block = self.block.as_mut()?;
        block.runs.push(run);
        block.runs.last_mut()
    }

    /// Writes `text` as it is, in the style of `ctx`, within the budget.
    fn put(&mut self, text: &str, ctx: &Context) {
        if text.is_empty() || self.full || !self.open_block(ctx) {
            return;
        }
        let text = cut(text, self.budget);
        if text.is_empty() {
            self.full = true;
            return;
        }
        let merge = self.block.as_ref().and_then(|b| b.runs.last()).is_some_and(|r| ctx.styles(r));
        let run = if merge { self.block.as_mut().and_then(|b| b.runs.last_mut()) } else { self.new_run(ctx.run()) };
        let Some(run) = run else { return };
        run.text.push_str(text);
        self.budget = self.budget.saturating_sub(text.len());
        self.line_start = text.ends_with('\n');
    }

    /// Writes the space waiting, if more is written on its line.
    fn settle_space(&mut self) {
        if let Some(ctx) = self.space.take()
            && !self.line_start
        {
            self.put(" ", &ctx);
        }
    }

    /// Text of the HTML: spaces collapsed (but in code blocks), control characters out, and bidi
    /// characters too in a link's or a mention's text.
    fn text(&mut self, s: &str) {
        let ctx = self.here().clone();
        if self.raw.is_some() || ctx.hidden || self.full {
            return;
        }
        let mut piece = String::new();
        for c in s.chars() {
            if !ctx.code_block && matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}') {
                self.put(&piece, &ctx);
                piece.clear();
                if self.space.is_none() {
                    self.space = Some(ctx.clone());
                }
                continue;
            }
            let kept_in_code = ctx.code_block && matches!(c, '\n' | '\t');
            if (c.is_control() && !kept_in_code) || (ctx.vouched && is_bidi(c)) {
                continue;
            }
            if self.space.is_some() {
                self.put(&piece, &ctx);
                piece.clear();
                self.settle_space();
            }
            piece.push(if c == '\u{a0}' { ' ' } else { c });
        }
        self.put(&piece, &ctx);
    }

    /// A line break in the block (none at its start).
    fn line_break(&mut self) {
        self.space = None;
        let ctx = self.here().clone();
        if !ctx.hidden && self.block.as_ref().is_some_and(|b| !b.runs.is_empty()) {
            self.put("\n", &ctx);
        }
    }

    /// A picture or a placeholder, a run of its own.
    fn object(&mut self, object: Object) {
        let ctx = self.here().clone();
        if ctx.hidden || self.full {
            return;
        }
        self.settle_space();
        let carried = match &object {
            Object::Hosted(s) | Object::Outside(s) | Object::Attachment(s) => s.len(),
        };
        if !self.open_block(&ctx) || !self.spend(carried) {
            return;
        }
        let mut run = ctx.run();
        match object {
            Object::Hosted(id) => run.hosted = Some(id),
            Object::Outside(address) => run.outside = Some(address),
            Object::Attachment(id) => run.attachment = Some(id),
        }
        if self.new_run(run).is_some() {
            self.line_start = false;
        }
    }

    /// Ends the block being written: kept if it shows anything, its trailing spaces and breaks out.
    fn flush(&mut self) {
        self.space = None;
        self.line_start = true;
        let Some(mut block) = self.block.take() else { return };
        while let Some(last) = block.runs.last_mut() {
            let shows = last.hosted.is_some() || last.outside.is_some() || last.attachment.is_some();
            if shows {
                break;
            }
            last.text.truncate(last.text.trim_end_matches([' ', '\n']).len());
            if !last.text.is_empty() {
                break;
            }
            block.runs.pop();
        }
        if block.runs.is_empty() {
            // Its line break given back.
            self.budget = self.budget.saturating_add(usize::from(!self.blocks.is_empty()));
        } else {
            self.blocks.push(block);
        }
    }

    fn start(&mut self, tag: &Tag) -> TokenSinkResult<()> {
        let name: &str = &tag.name;
        match name {
            "script" => {
                self.raw = Some(name.to_string());
                return TokenSinkResult::RawData(RawKind::ScriptData);
            }
            "style" | "title" | "textarea" | "xmp" | "iframe" | "noembed" | "noframes" => {
                self.raw = Some(name.to_string());
                return TokenSinkResult::RawData(RawKind::Rawtext);
            }
            "br" => self.line_break(),
            "img" => {
                if let Some(object) = attr(tag, "src").and_then(|src| self.picture(src)) {
                    self.object(object);
                }
            }
            "attachment" => {
                if let Some(id) = attr(tag, "id").map(str::trim).filter(|id| !id.is_empty() && id.len() <= MAX_ID && !id.contains(char::is_control)) {
                    self.object(Object::Attachment(id.to_string()));
                }
            }
            "emoji" => {
                let alt: String =
                    attr(tag, "alt").unwrap_or_default().chars().filter(|&c| !c.is_control() && !is_bidi(c)).take(EMOJI_CHARS).collect();
                self.text(&alt);
            }
            "td" | "th" if !self.line_start && self.space.is_none() => self.space = Some(self.here().clone()),
            _ => {}
        }
        if starts_block(name) {
            self.flush();
        }
        // What a new paragraph or list item ends: the one open.
        let ends = match name {
            "p" => self.stack.last().is_some_and(|o| o.name == "p").then(|| self.stack.len() - 1),
            "li" => {
                let item = self.stack.iter().rposition(|o| o.name == "li");
                let list = self.stack.iter().rposition(|o| o.name == "ul" || o.name == "ol");
                item.filter(|i| list.is_none_or(|l| l < *i))
            }
            _ => None,
        };
        if let Some(i) = ends {
            self.stack.truncate(i);
        }
        if tag.self_closing || is_void(name) || self.stack.len() >= TEAMS_DEPTH {
            return TokenSinkResult::Continue;
        }
        let mut inside = self.here().clone();
        match name {
            "b" | "strong" => inside.bold = true,
            "i" | "em" => inside.italic = true,
            "u" | "ins" => inside.underline = true,
            "s" | "strike" | "del" => inside.strike = true,
            "code" => inside.code = true,
            "a" => {
                inside.link = attr(tag, "href").and_then(web_address);
                inside.vouched = true;
            }
            "at" => {
                inside.mention = attr(tag, "id").and_then(|id| id.trim().parse().ok());
                inside.vouched = true;
            }
            "ul" | "ol" => {
                inside.lists = inside.lists.saturating_add(1);
                inside.ordered = name == "ol";
                inside.item = false;
            }
            "li" => inside.item = true,
            "blockquote" => inside.quote = true,
            "pre" | "codeblock" => inside.code_block = true,
            "emoji" | "attachment" => inside.hidden = true,
            _ => {}
        }
        self.stack.push(Open { name: name.to_string(), inside });
        TokenSinkResult::Continue
    }

    fn end(&mut self, tag: &Tag) {
        let name: &str = &tag.name;
        if self.raw.as_deref() == Some(name) {
            self.raw = None;
            return;
        }
        if name == "br" {
            self.line_break();
            return;
        }
        if starts_block(name) {
            self.flush();
        }
        if let Some(i) = self.stack.iter().rposition(|o| o.name == name) {
            self.stack.truncate(i);
        }
    }

    /// What a picture's address is: this message's hosted image, a picture on the web, or nothing.
    fn picture(&self, src: &str) -> Option<Object> {
        match hosted_id(src, self.chat, self.message) {
            Some(id) => Some(Object::Hosted(id)),
            None => web_address(src).map(Object::Outside),
        }
    }
}

/// The id of this message's hosted image `src` is, when it is exactly
/// `https://graph.microsoft.com/v1.0/chats/{chat}/messages/{message}/hostedContents/{id}/$value`
/// with this message's own chat and id (compared %-decoded): at most [`MAX_ID`] characters of
/// the base64 alphabet and URL-safe marks, and not only dots (`.`, `..`: a path step), so the
/// address the worker builds from it is this one.
fn hosted_id(src: &str, chat: &str, message: &str) -> Option<String> {
    let rest = src.strip_prefix(HOSTED)?;
    let parts: Vec<&str> = rest.splitn(7, '/').collect();
    let [c, "messages", m, "hostedContents", id, "$value"] = parts.as_slice() else { return None };
    if percent_decoded(c)? != chat || percent_decoded(m)? != message {
        return None;
    }
    let id = percent_decoded(id)?;
    let safe = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '=' | '+' | '~');
    let only_dots = id.chars().all(|c| c == '.');
    (!id.is_empty() && !only_dots && id.len() <= MAX_ID && id.chars().all(safe)).then_some(id)
}

/// `s` with its %-escapes read; `None` when one is broken or the bytes aren't UTF-8.
fn percent_decoded(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A link or picture address on the web: `http` or `https` (the scheme written in lower case),
/// with no space, control or bidi character (one would turn round what the address shows), at
/// most [`ADDRESS_MAX`] bytes.
fn web_address(v: &str) -> Option<String> {
    let v = v.trim_matches(|c: char| c.is_ascii_whitespace());
    if v.len() > ADDRESS_MAX || v.contains(|c: char| c.is_whitespace() || c.is_control() || is_bidi(c)) {
        return None;
    }
    let (scheme, rest) = v.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    (matches!(scheme.as_str(), "http" | "https") && !rest.is_empty()).then(|| format!("{scheme}://{rest}"))
}

fn attr<'t>(tag: &'t Tag, name: &str) -> Option<&'t str> {
    tag.attrs.iter().find(|a| &*a.name.local == name).map(|a| &*a.value)
}

/// Tags that begin and end a block.
fn starts_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "ul"
            | "ol"
            | "li"
            | "blockquote"
            | "pre"
            | "codeblock"
            | "table"
            | "tr"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "hr"
            | "dl"
            | "dt"
            | "dd"
    )
}

/// Elements that never have content.
fn is_void(name: &str) -> bool {
    matches!(name, "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | "input" | "link" | "meta" | "source" | "track" | "wbr")
}

/// Hands html5ever's tokens to the reader.
struct Sink<'a>(RefCell<Reader<'a>>);

impl TokenSink for Sink<'_> {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let mut reader = self.0.borrow_mut();
        match token {
            Token::TagToken(tag) => match tag.kind {
                TagKind::StartTag => return reader.start(&tag),
                TagKind::EndTag => reader.end(&tag),
            },
            Token::CharacterTokens(t) => reader.text(&t),
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

#[cfg(test)]
mod tests;
