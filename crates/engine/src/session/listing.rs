//! The message list: what it shows, its filters, how it's arranged, and its rows.

use super::*;

/// What the message list shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum View {
    /// Nothing yet (no accounts).
    Empty,
    Folder {
        account: String,
        folder: String,
    },
    /// The same folder of every account (Favorites › All Inboxes).
    Unified {
        role: FolderRole,
    },
    /// Messages waiting to be sent, from every account.
    Outbox,
}

/// The pseudo-folder of Outbox message keys (NUL never appears in a server's folder names).
pub const OUTBOX: &str = "\u{0}outbox";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ListFilter {
    #[default]
    All,
    Unread,
    Flagged,
    Attachments,
    /// Carrying a category.
    Categorized,
    /// Marked high importance by the sender.
    Important,
    /// Sent to the account's address, or copied to it.
    ToMe,
}

impl ListFilter {
    /// The filters shown as tabs above the list.
    pub const ALL: [ListFilter; 4] = [ListFilter::All, ListFilter::Unread, ListFilter::Flagged, ListFilter::Attachments];
    /// Every filter (the Home tab's Filter menu).
    pub const EVERY: [ListFilter; 7] = [
        ListFilter::All,
        ListFilter::Unread,
        ListFilter::Flagged,
        ListFilter::Attachments,
        ListFilter::Categorized,
        ListFilter::Important,
        ListFilter::ToMe,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ListFilter::All => "All",
            ListFilter::Unread => "Unread",
            ListFilter::Flagged => "Flagged",
            ListFilter::Attachments => "Has files",
            ListFilter::Categorized => "Categorized",
            ListFilter::Important => "Important",
            ListFilter::ToMe => "To or Cc me",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            ListFilter::All => "all",
            ListFilter::Unread => "unread",
            ListFilter::Flagged => "flagged",
            ListFilter::Attachments => "attachments",
            ListFilter::Categorized => "categorized",
            ListFilter::Important => "important",
            ListFilter::ToMe => "toMe",
        }
    }
}

/// What the message list is sorted and grouped by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Arrange {
    /// Newest first, under Today, Yesterday… (conversations, when on, apply here).
    #[default]
    Date,
    /// By sender, A to Z.
    From,
    /// By the first recipient, A to Z.
    To,
    /// By subject (without Re:, Fwd:…), A to Z.
    Subject,
    /// Largest first, under size ranges.
    Size,
    /// Flagged first.
    Flag,
    /// With attachments first.
    Attachments,
}

impl Arrange {
    pub const ALL: [Arrange; 7] = [Arrange::Date, Arrange::From, Arrange::To, Arrange::Subject, Arrange::Size, Arrange::Flag, Arrange::Attachments];

    pub fn code(self) -> &'static str {
        match self {
            Arrange::Date => "date",
            Arrange::From => "from",
            Arrange::To => "to",
            Arrange::Subject => "subject",
            Arrange::Size => "size",
            Arrange::Flag => "flag",
            Arrange::Attachments => "attachments",
        }
    }

    pub fn from_code(s: &str) -> Option<Arrange> {
        Arrange::ALL.into_iter().find(|a| a.code() == s)
    }

    /// The name shown in the View tab and above the list.
    pub fn label(self) -> &'static str {
        match self {
            Arrange::Date => "Date",
            Arrange::From => "From",
            Arrange::To => "To",
            Arrange::Subject => "Subject",
            Arrange::Size => "Size",
            Arrange::Flag => "Flag status",
            Arrange::Attachments => "Attachments",
        }
    }
}

/// Who a message is shown under: a name, else the address, else nothing.
pub(super) fn person(a: Option<&quatrelle_model::EmailAddress>) -> String {
    a.map(|a| if a.name.trim().is_empty() { a.email.trim().to_string() } else { a.name.trim().to_string() }).unwrap_or_default()
}

/// The fixed headings an arranged list shows (for translations).
pub const ARRANGE_HEADINGS: [&str; 13] = [
    "Under 10 KB",
    "10 to 25 KB",
    "25 to 100 KB",
    "100 to 500 KB",
    "500 KB to 1 MB",
    "1 to 5 MB",
    "Over 5 MB",
    "Flagged",
    "Not flagged",
    "With attachments",
    "Without attachments",
    "(no subject)",
    "(nobody)",
];

/// The size range a message is shown under.
pub(super) fn size_group(size: u32) -> &'static str {
    match size {
        0..10_240 => "Under 10 KB",
        10_240..25_600 => "10 to 25 KB",
        25_600..102_400 => "25 to 100 KB",
        102_400..512_000 => "100 to 500 KB",
        512_000..1_048_576 => "500 KB to 1 MB",
        1_048_576..5_242_880 => "1 to 5 MB",
        _ => "Over 5 MB",
    }
}

/// Sort a list as `arrange` says (each one's natural order; `reverse` the other way), newest
/// first among equals.
pub fn arrange_list<M: std::borrow::Borrow<Summary>>(list: &mut [M], arrange: Arrange, reverse: bool) {
    let text = |s: String| s.to_lowercase();
    list.sort_by(|a, b| {
        let (a, b) = (a.borrow(), b.borrow());
        let primary = match arrange {
            Arrange::Date => b.date.cmp(&a.date),
            Arrange::From => text(person(Some(&a.from))).cmp(&text(person(Some(&b.from)))),
            Arrange::To => text(person(a.to.first())).cmp(&text(person(b.to.first()))),
            Arrange::Subject => text(a.base_subject()).cmp(&text(b.base_subject())),
            Arrange::Size => b.size.cmp(&a.size),
            Arrange::Flag => b.flags.flagged.cmp(&a.flags.flagged),
            Arrange::Attachments => b.has_attachments.cmp(&a.has_attachments),
        };
        let primary = if reverse { primary.reverse() } else { primary };
        primary.then_with(|| b.date.cmp(&a.date)).then_with(|| b.key.cmp(&a.key))
    });
}

/// A folder of an account, by path.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FolderRef {
    pub account: String,
    pub folder: String,
}

/// A line in the message list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A date heading (`Today`, `Last week`…), or another heading of a fixed text.
    Header(&'static str),
    /// A heading that's a name (a sender, a recipient, a subject), shown as it is.
    Group(String),
    /// A message (index into [`Session::list`]); with conversations on, the newest message of a
    /// conversation and how many messages (and unread ones) it holds.
    Message { index: usize, count: usize, unread: usize },
}

/// Previews worked out in one go: a screenful takes a few frames, the window drawn between them
/// (reading a big message's text out takes milliseconds).
const PREVIEW_STEP: usize = 6;
/// The sources read for previews in one go, at most (a bigger one goes alone).
const PREVIEW_STEP_BYTES: usize = 4 << 20;
/// Previews kept in memory (a couple of hundred bytes each); past it they're worked out again.
const PREVIEWS_KEPT: usize = 5_000;
/// A message whose source wasn't here is looked at again after this long (downloading ahead
/// brings it meanwhile).
const PREVIEW_AGAIN_SECS: i64 = 60;

/// A list row's preview, worked out from its source.
#[derive(Clone, Debug)]
pub(super) enum Preview {
    Text(String),
    /// No source on this computer when looked (at that time).
    Missing(i64),
}

impl Session {
    /// A message's preview in the list: its list entry's, or the one worked out from its source
    /// when it came without one ([`Session::fill_previews`]).
    pub fn preview_of<'a>(&'a self, m: &'a Summary) -> &'a str {
        if !m.preview.is_empty() {
            return &m.preview;
        }
        match self.previews.get(&m.key) {
            Some(Preview::Text(p)) => p,
            _ => "",
        }
    }

    /// Whether a row's preview is still to be worked out from its source.
    pub fn preview_wanted(&self, m: &Summary) -> bool {
        m.preview.is_empty()
            && m.key.folder != OUTBOX
            && match self.previews.get(&m.key) {
                None => true,
                Some(Preview::Text(_)) => false,
                Some(Preview::Missing(at)) => self.clock.now.saturating_sub(*at) >= PREVIEW_AGAIN_SECS,
            }
    }

    /// Work out the previews of rows shown without one from their sources here, as Outlook
    /// writes a message's preview from its offline copy for the rows it draws: mail downloaded
    /// ahead comes without one (a big folder's list isn't written again for it). A few at a
    /// time; whether some of `keys` are left.
    pub fn fill_previews(&mut self, keys: &[MessageKey]) -> bool {
        let Some(store) = self.store.clone() else { return false };
        if self.previews.len() >= PREVIEWS_KEPT {
            self.previews.clear();
        }
        let (mut done, mut bytes) = (0usize, 0usize);
        for key in keys {
            if done >= PREVIEW_STEP || bytes >= PREVIEW_STEP_BYTES {
                return true;
            }
            let Some(raw) = store.read_body(key) else {
                self.previews.insert(key.clone(), Preview::Missing(self.clock.now));
                continue;
            };
            done += 1;
            bytes = bytes.saturating_add(raw.len());
            let text = mime::parse(&raw).map(|p| mime::preview(&p.text)).unwrap_or_default();
            self.previews.insert(key.clone(), Preview::Text(text));
        }
        false
    }
}
