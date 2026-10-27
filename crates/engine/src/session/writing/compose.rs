//! A message being written: its text and look, its attachments, and its signature's place in it.

use super::*;

/// A message being written.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Compose {
    pub id: u64,
    pub account: String,
    /// Who it's from when not the account's own address (`Name <address>`), as Outlook's From:
    /// a mailbox the account opens (a shared one), or another address it may send from.
    #[serde(default)]
    pub from: String,
    /// The signature in the text as it was put there (`-- ` and the signature): replaced when
    /// another goes in, while it's still there unchanged.
    #[serde(default)]
    pub signature: String,
    /// The signature written in HTML that stands where [`mime::rich::SIGNATURE`] is in the text
    /// (its pictures among [`Compose::inline`]).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature_html: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<OutAttachment>,
    pub in_reply_to: String,
    pub references: Vec<String>,
    pub mode: Option<ReplyMode>,
    /// The message replied to or forwarded (it gets `\Answered` / `$Forwarded` once sent).
    pub source: Option<MessageKey>,
    pub sending: bool,
    pub error: Option<String>,
    /// The copy saved in Drafts, replaced on the next save.
    pub draft: Option<MessageKey>,
    /// The `Message-ID` every copy saved in Drafts goes by: a server account's copy is found
    /// there by it once it's listed (its key isn't known when it's saved), to be replaced at the
    /// next save and removed once the message is sent or discarded.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub saved_id: String,
    /// What it held as it was started ([`Compose::fingerprint`]): a reply left as it came isn't
    /// kept when another message is read.
    #[serde(skip)]
    pub started: Option<u64>,
    /// The Bcc row is shown.
    pub show_cc: bool,
    /// Send as plain text only (lists written out, the rest of the formatting dropped).
    #[serde(default)]
    pub plain: bool,
    /// The pictures placed in the text (each where a [`mime::rich::PICTURE`] stands).
    #[serde(default)]
    pub inline: Vec<mime::InlinePicture>,
    /// How the text looks: a style for each character and paragraph.
    #[serde(default)]
    pub styles: mime::rich::Styles,
    /// Written in a window of its own rather than the reading pane.
    #[serde(default)]
    pub window: bool,
    /// High (1) or low (-1) importance.
    #[serde(default)]
    pub importance: i8,
    /// Ask the recipients for a read receipt.
    #[serde(default)]
    pub read_receipt: bool,
    /// Voting buttons offered (Exchange accounts), `;`-separated: Outlook's Use Voting Buttons.
    #[serde(default)]
    pub voting: String,
    /// Where replies go, when not to the sender (Outlook's "Have replies sent to").
    #[serde(default)]
    pub reply_to: String,
    /// Hold it in the Outbox until then (Unix seconds): Outlook's Delay Delivery.
    #[serde(default)]
    pub send_after: Option<i64>,
    /// The Replies to row is shown.
    #[serde(default)]
    pub show_reply_to: bool,
    /// S/MIME: signed, and encrypted to its recipients, as it's sent.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sign: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub encrypt: bool,
    /// The language its spelling is checked in (`fr`, `en`…, as the dictionaries are named;
    /// empty: the interface's, when there's a dictionary for it; `all`: every dictionary), as
    /// Outlook's Review › Language.
    #[serde(default)]
    pub language: String,
    /// The language model asked to rewrite the text written (not kept with the draft).
    #[serde(skip)]
    pub rewrite: Option<super::assistant::Rewrite>,
}

impl Compose {
    /// What's been written, digested: the recipients, subject, text and attachments.
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        (&self.to, &self.cc, &self.bcc, &self.subject, &self.body).hash(&mut h);
        for a in &self.attachments {
            (&a.name, a.data.len()).hash(&mut h);
        }
        h.finish()
    }

    /// Nothing changed since it was started (a reply as it came).
    pub fn untouched(&self) -> bool {
        self.started == Some(self.fingerprint())
    }

    /// The text, HTML and pictures to send: formatted when the text uses formatting or has
    /// pictures (and the message isn't set to plain text), else plain text alone. A signature
    /// written in HTML goes as it was written where it stands, and as text in the text.
    pub fn bodies(&self) -> (String, Option<String>, Vec<mime::InlinePicture>) {
        let mut styles = self.styles.clone();
        styles.fit(&self.body);
        let mut used = mime::rich::pictures(&self.body, &styles);
        let signed = self.body.contains(mime::rich::SIGNATURE);
        if signed {
            used.extend(self.inline.iter().filter(|p| self.signature_html.contains(&format!("cid:{}", p.cid))).map(|p| p.cid.clone()));
        }
        let pictures: Vec<mime::InlinePicture> = self.inline.iter().filter(|p| used.contains(&p.cid)).cloned().collect();
        let mut text = mime::rich::to_text(&self.body, &styles);
        if signed {
            text = text.replace(mime::rich::SIGNATURE, &signature_text(&self.signature_html));
        }
        if self.plain || (styles.is_plain() && pictures.is_empty() && !signed) {
            (text, None, Vec::new())
        } else {
            let mut html = mime::rich::to_html(&self.body, &styles);
            if signed {
                html = html.replace(mime::rich::SIGNATURE, &format!("<div>{}</div>", self.signature_html));
            }
            (text, Some(html), pictures)
        }
    }

    /// Set the text (and forget its formatting).
    pub fn set_text(&mut self, text: String) {
        self.styles = mime::rich::Styles::plain(&text);
        self.body = text;
    }

    /// Replace a stretch of the text (byte offsets) with another, the look of the text it
    /// replaces kept: a misspelt word changed for a suggestion in the spelling dialog.
    pub fn replace_text(&mut self, from: usize, to: usize, with: &str) -> Result<(), CmdError> {
        if from > to || to > self.body.len() || !self.body.is_char_boundary(from) || !self.body.is_char_boundary(to) {
            return Err(CmdError::Params("that stretch of the text isn't there".into()));
        }
        let first = self.body[..from].chars().count();
        let was = self.body[from..to].chars().count();
        self.body.replace_range(from..to, with);
        let style = self.styles.chars.get(first).copied().unwrap_or(0);
        let end = (first + was).min(self.styles.chars.len());
        self.styles.chars.splice(first..end, std::iter::repeat_n(style, with.chars().count()));
        if self.styles.chars.len() != self.body.chars().count() {
            self.styles.fit(&self.body);
        }
        Ok(())
    }

    /// Replace a stretch of the subject (byte offsets): a misspelt word changed for a suggestion.
    pub fn replace_subject(&mut self, from: usize, to: usize, with: &str) -> Result<(), CmdError> {
        if from > to || to > self.subject.len() || !self.subject.is_char_boundary(from) || !self.subject.is_char_boundary(to) {
            return Err(CmdError::Params("that stretch of the subject isn't there".into()));
        }
        self.subject.replace_range(from..to, with);
        Ok(())
    }
}

/// A signature written in HTML as plain text (for a message's text part): its words, without
/// its pictures.
pub fn signature_text(html: &str) -> String {
    let (text, styles) = mime::signature_html(html);
    let text = mime::rich::to_text(&text, &styles).replace("[picture]", "");
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    lines.join("\n").trim().to_string()
}

/// A signature as it goes into a message: after the `-- ` line (RFC 3676's delimiter) when the
/// writer wants one (`dashes`), as it is otherwise, which is how Outlook adds it.
pub fn signature_block(sig: &str, dashes: bool) -> String {
    if dashes { format!("-- \n{}", sig.trim_end()) } else { sig.trim_end().to_string() }
}

/// The message as HTML with its signature's HTML where the signature's text stands (`None`
/// when that text was taken out or changed: the message is sent as written then).
pub(super) fn with_html_signature(c: &Compose, raw: &str) -> Option<String> {
    const MARK: char = '\u{E001}';
    if c.signature.is_empty() || c.body.contains(MARK) {
        return None;
    }
    let b = c.body.rfind(&c.signature)?;
    let start = c.body.get(..b)?.chars().count();
    let n = c.signature.chars().count();
    let (mut text, mut styles) = (c.body.clone(), c.styles.clone());
    styles.fit(&text);
    let mark = MARK.to_string();
    mime::rich::splice(&mut text, &mut styles, start..start + n, &mark, &mime::rich::Styles::plain(&mark));
    let html = mime::rich::to_html(&text, &styles);
    html.contains(MARK).then(|| html.replace(MARK, &format!("<div>{raw}</div>")))
}

/// Pictures a message gets with its signature (those it hasn't already).
pub fn add_pictures(c: &mut Compose, pictures: Vec<mime::InlinePicture>) {
    for p in pictures {
        if !c.inline.iter().any(|x| x.cid == p.cid) {
            c.inline.push(p);
            c.plain = false;
        }
    }
}

/// Give the signature in a message (`c.signature`, where it still stands as put there) the look
/// it has: `sig` is its text and styles, `dashes` whether a `-- ` line was put before it.
pub fn style_signature(c: &mut Compose, sig: &(String, mime::rich::Styles), dashes: bool) {
    let (text, styles) = sig;
    if c.signature.is_empty() || styles.is_plain() {
        return;
    }
    let mut block = if dashes { "-- \n".to_string() } else { String::new() };
    let mut block_styles = mime::rich::Styles::plain(&block);
    mime::rich::append(&mut block, &mut block_styles, text.trim_end(), styles);
    if block != c.signature {
        return;
    }
    let Some(b) = c.body.rfind(&c.signature) else { return };
    let start = c.body.get(..b).unwrap_or("").chars().count();
    let n = block.chars().count();
    c.styles.fit(&c.body);
    mime::rich::splice(&mut c.body, &mut c.styles, start..start + n, &block, &block_styles);
}
