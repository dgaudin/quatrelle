//! Modal dialogs: add an account, sign in again, move to a folder, confirmations, settings.

use egui::{Margin, Rect, RichText, Sense, Ui, vec2};
use quatrelle_engine::Compose;
use quatrelle_engine::Session;
use quatrelle_engine::model::organize::{Condition, Rule, RuleAction};
use quatrelle_engine::model::{AccountConfig, Folder, FolderRole, Security};
use serde_json::json;

use crate::theme::{self, Tokens};
use crate::{Action, Icon, widgets};

mod accounts;
mod folder_sizes;
mod folders;
mod mailbox;
mod moving;
mod organize;
mod other;
pub(crate) mod pim_dialogs;
mod writing;

pub use accounts::*;
pub use folder_sizes::SizeSort;
pub use mailbox::*;
pub use moving::MoveForm;
pub use organize::*;
pub use writing::*;

#[derive(Clone, Debug)]
pub enum Dialog {
    AddAccount(Box<AccountForm>),
    /// Outlook's Select Names: the address book, to add people to To, Cc and Bcc.
    SelectNames(Box<NamesForm>),
    /// Outlook's Attach Item: a message, a contact or an appointment, to go along as a file.
    AttachItem(Box<ItemForm>),
    /// Closing a message window with something written in it.
    SaveChanges,
    Password {
        account: String,
        email: String,
        password: String,
    },
    ConfirmDelete {
        count: usize,
    },
    /// Closing Quatrelle with messages being written (how many): save them as drafts?
    SaveOnClose(usize),
    /// Closing would lose what online-only accounts hold in memory only (issue #4).
    LostOnClose {
        unsent: usize,
        changes: usize,
    },
    /// Confirmation before `contact.delete`, which can't be undone.
    ConfirmDeleteContact {
        uid: String,
        name: String,
    },
    /// A link to a network share on a server not trusted yet: opened once the user says so
    /// (`link`: `file://server/share/…`; `path`: as Windows writes it), and from then on
    /// without asking when `always` is ticked.
    OpenShare {
        link: String,
        server: String,
        path: String,
        always: bool,
    },
    MoveTo {
        filter: String,
    },
    EmptySubject,
    /// What `compose.send` held the message for (a lookalike recipient, many people seeing each
    /// other's addresses).
    SendCheck(Vec<quatrelle_engine::SendCheck>),
    RemoveAccount {
        account: String,
        email: String,
    },
    /// Microsoft signed the account in but won't open its mailbox over IMAP: why, and the way out.
    MailboxClosed {
        account: String,
        email: String,
        error: String,
    },
    Signature(Box<SignatureForm>),
    /// An account's name, user name and servers.
    EditAccount(Box<EditForm>),
    /// An account's automatic replies.
    Vacation(Box<VacationForm>),
    /// Empty Deleted Items or Junk for good (`for_good`), or Delete All in another folder:
    /// everything to Deleted Items.
    ConfirmEmpty {
        account: String,
        folder: String,
        name: String,
        for_good: bool,
    },
    /// Open another mailbox in an Exchange account: a shared one, or someone's who made this
    /// account a delegate (Outlook's Open Other Mailbox).
    OpenSharedMailbox {
        account: String,
        email: String,
        calendar: bool,
        error: Option<String>,
    },
    /// Open someone else's calendar by their address (Outlook's Open Calendar).
    OpenSharedCalendar {
        email: String,
        error: Option<String>,
    },
    /// The password of a certificate file being imported (S/MIME): from `path`, or `data` as the
    /// browser's picker gives it; neither, the one the browser keeps, unlocked.
    Certificate {
        account: String,
        path: String,
        data: Option<Vec<u8>>,
        password: String,
    },
    /// A certificate on a smart card (its PKCS #11 module and PIN), or in the system's keychain.
    SmartCard {
        account: String,
        module: String,
        pin: String,
    },
    /// Word count of the message being written (or its selection: the `bool`).
    WordCount(crate::rich_edit::Counts, bool),
    /// A folder's Properties: where it is, how many messages, how much is on this computer;
    /// `id`: its account's, for Folder Size.
    FolderProperties {
        name: String,
        id: String,
        account: String,
        location: String,
        total: usize,
        unread: usize,
        size: u64,
    },
    /// Outlook's Folder Size: each of an account's folders' sizes on this computer, or on its
    /// server (`server`), as the session has them; `kept`: what the account takes here in all.
    FolderSize {
        account: String,
        server: bool,
        sort: SizeSort,
        kept: u64,
    },
    Event(Box<crate::pim_dialogs::EventForm>),
    Contact(Box<crate::pim_dialogs::ContactForm>),
    /// An appointment dragged to another time: Outlook's questions before it moves.
    MoveEvent(Box<MoveForm>),
    /// New folder (in `parent`), or rename `rename`.
    FolderName {
        account: String,
        parent: Option<String>,
        rename: Option<String>,
        name: String,
        error: Option<String>,
    },
    MoveFolder {
        account: String,
        folder: String,
        filter: String,
        error: Option<String>,
    },
    DeleteFolder {
        account: String,
        folder: String,
        name: String,
        permanent: bool,
        error: Option<String>,
    },
    NewCategory {
        name: String,
        color: u8,
        error: Option<String>,
    },
    Rules,
    /// Before Outlook's copy of an Exchange account's rules is replaced: what's lost.
    ReplaceOutlookRules {
        account: String,
        email: String,
    },
    EditRule(Box<RuleForm>),
    /// A Search Folder made or changed (Outlook's New Search Folder).
    SearchFolder(Box<SearchFolderForm>),
    /// A Quick Step made or changed.
    QuickStep(Box<QuickStepForm>),
    /// A picture attachment, larger.
    PicturePreview {
        key: quatrelle_engine::model::MessageKey,
        index: usize,
        name: String,
    },
    /// An attachment shown as text (a file, or an attached email's header and text).
    Preview {
        title: String,
        header: Vec<(String, String)>,
        text: String,
        /// An email whose sender doesn't add up.
        warning: Option<quatrelle_engine::model::lookalike::SenderWarning>,
    },
    /// Outlook's Spelling: the words the dictionaries don't know, one at a time.
    Spelling(Box<SpellForm>),
}

/// What the spelling dialog keeps from one word to the next: the places passed over (Ignore
/// once), and the suggestions and the one picked for the word it shows (none for the next).
#[derive(Clone, Debug, Default)]
pub struct SpellForm {
    /// Words passed over while this dialog is open: the field and where the word starts.
    passed: Vec<(&'static str, usize)>,
    /// The suggestion picked, when one is.
    chosen: Option<String>,
    /// The word `chosen` was picked for: a new word comes with nothing picked.
    shown: String,
    /// What could take `shown`, worked out once for it.
    suggestions: Vec<String>,
}

impl Dialog {
    pub fn add_account() -> Dialog {
        Dialog::AddAccount(Box::default())
    }

    /// The address book for the message being written; a double-click adds to `field`.
    pub fn select_names(c: &quatrelle_engine::Compose, field: &'static str) -> Dialog {
        Dialog::SelectNames(Box::new(NamesForm {
            target: field,
            to: c.to.clone(),
            cc: c.cc.clone(),
            bcc: c.bcc.clone(),
            stale: true,
            ..Default::default()
        }))
    }

    /// Attach Item, open on a kind of item; messages from the Inbox of the account writing.
    pub fn attach_item(s: &Session, c: &Compose, kind: quatrelle_engine::ItemKind) -> Dialog {
        let many = s.accounts.len() > 1;
        let folders: Vec<(String, String, String)> = s
            .accounts
            .iter()
            .flat_map(|a| {
                a.folders.iter().filter(|f| f.selectable).map(move |f| {
                    let name = f.display_name();
                    let label = if many { format!("{name} ({})", a.config.email) } else { name };
                    (a.config.id.clone(), f.path.clone(), label)
                })
            })
            .collect();
        let inbox = s.account(&c.account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| (c.account.clone(), f.path.clone()));
        let (account, folder) = inbox.or_else(|| folders.first().map(|(a, f, _)| (a.clone(), f.clone()))).unwrap_or_default();
        Dialog::AttachItem(Box::new(ItemForm { kind, account, folder, folders, stale: true, ..Default::default() }))
    }

    /// Dialogs that belong to the message being written (shown in its window when it has one).
    pub fn for_compose(&self) -> bool {
        matches!(
            self,
            Dialog::SelectNames(_)
                | Dialog::AttachItem(_)
                | Dialog::SaveChanges
                | Dialog::EmptySubject
                | Dialog::SendCheck(_)
                | Dialog::Spelling(_)
                | Dialog::WordCount(..)
        ) || matches!(self, Dialog::Signature(f) if f.insert)
    }

    /// The add-account dialog with the address filled in (servers guessed from it).
    pub fn add_account_with(email: &str, advanced: bool) -> Dialog {
        let mut f = AccountForm { email: email.trim().to_string(), advanced, ..Default::default() };
        if !f.email.is_empty() {
            f.guess();
        }
        Dialog::AddAccount(Box::new(f))
    }

    pub fn move_to() -> Dialog {
        Dialog::MoveTo { filter: String::new() }
    }

    /// Outlook's Spelling, over the message being written.
    pub fn spelling() -> Dialog {
        Dialog::Spelling(Box::default())
    }

    /// A new event (at a time, or the next hour).
    pub fn new_event(s: &Session, at: Option<i64>) -> Dialog {
        Dialog::Event(Box::new(crate::pim_dialogs::EventForm::new(s, at)))
    }

    /// A meeting about a message, as Outlook's Reply with Meeting: its subject, its sender and
    /// the others it went to as guests (not the user), and its text as the description.
    pub fn meeting_from(s: &Session, p: &quatrelle_engine::mime::Parsed) -> Dialog {
        let mut f = crate::pim_dialogs::EventForm::new(s, None);
        f.summary = p.subject.clone();
        let mine = |e: &str| s.accounts.iter().any(|a| a.config.email.eq_ignore_ascii_case(e.trim()));
        let mut guests: Vec<quatrelle_engine::model::EmailAddress> = Vec::new();
        for a in std::iter::once(&p.from).chain(&p.to).chain(&p.cc) {
            if !a.email.trim().is_empty() && !mine(&a.email) && !guests.iter().any(|g| g.email.eq_ignore_ascii_case(&a.email)) {
                guests.push(a.clone());
            }
        }
        f.guests = quatrelle_engine::model::addr::format_list(&guests);
        f.description = p.text.chars().take(4000).collect();
        Dialog::Event(Box::new(f))
    }

    /// A folder's Properties, from what this computer keeps of it.
    pub fn folder_properties(s: &Session, f: &quatrelle_engine::model::Folder) -> Dialog {
        let kept = s.store.as_ref().and_then(|st| st.load_folder(&f.account, &f.path).ok()).unwrap_or_default();
        let total = if kept.messages.is_empty() { f.total as usize } else { kept.messages.len() };
        let unread = if kept.messages.is_empty() { f.unread as usize } else { kept.messages.iter().filter(|m| !m.flags.seen).count() };
        let email = s.account(&f.account).map(|a| a.config.email.clone()).unwrap_or_default();
        Dialog::FolderProperties {
            name: crate::i18n::folder(f),
            id: f.account.clone(),
            location: {
                let decoded = quatrelle_engine::model::utf7::decode(&f.path);
                match f.delimiter {
                    Some(d) => decoded.split(d).collect::<Vec<_>>().join(" › "),
                    None => decoded,
                }
            },
            account: email,
            total,
            unread,
            size: kept.messages.iter().map(|m| u64::from(m.size)).sum(),
        }
    }

    /// Folder Size for an account, open on its server's side when it has a server.
    pub fn folder_size(s: &Session, account: &str) -> Dialog {
        let server = s.account(account).is_some_and(|a| !a.is_local());
        Dialog::FolderSize { account: account.to_string(), server, sort: SizeSort::Place, kept: s.cache_size(account) }
    }

    /// An event, opened at one of its occurrences.
    pub fn open_event(s: &Session, uid: &str, occurrence: i64) -> Option<Dialog> {
        crate::pim_dialogs::EventForm::open(s, uid, occurrence).map(|f| Dialog::Event(Box::new(f)))
    }

    pub fn new_contact() -> Dialog {
        Dialog::Contact(Box::default())
    }

    pub fn edit_contact(c: &quatrelle_engine::model::vcard::Card) -> Dialog {
        Dialog::Contact(Box::new(crate::pim_dialogs::ContactForm::edit(c)))
    }

    /// A new contact for someone in a message, their name and address filled in.
    pub fn contact_from(a: &quatrelle_engine::model::EmailAddress) -> Dialog {
        Dialog::Contact(Box::new(crate::pim_dialogs::ContactForm::from_address(a)))
    }

    pub fn edit_account(a: &AccountConfig) -> Dialog {
        Dialog::EditAccount(Box::new(EditForm {
            account: a.id.clone(),
            email: a.email.clone(),
            name: a.name.clone(),
            username: a.username.clone(),
            jmap: a.jmap.is_some(),
            jmap_url: a.jmap.clone().unwrap_or_default(),
            ews: a.ews.clone(),
            pop: a.pop.as_ref().map(|p| (p.host.clone(), p.port.to_string(), p.security)),
            pop_keep: PopKeep::of(a),
            imap_host: a.imap.host.clone(),
            imap_port: a.imap.port.to_string(),
            imap_security: a.imap.security,
            smtp_host: a.smtp.host.clone(),
            smtp_port: a.smtp.port.to_string(),
            smtp_security: a.smtp.security,
            smtp_sign_in: SmtpSignIn::of(a),
            online_only: a.online_only,
            shared_kept: !a.shared_online,
            public_favorites_kept: a.public_favorites_kept,
            oauth: a.oauth.clone(),
            client_id: a.oauth_client_id.clone().unwrap_or_default(),
            tenant: a.oauth_tenant.clone().unwrap_or_default(),
            error: None,
        }))
    }

    /// Automatic replies; `exchange`: the server keeps a reply for outside the organisation too.
    pub fn vacation(account: &str, v: &quatrelle_engine::model::job::Vacation, exchange: bool) -> Dialog {
        Dialog::Vacation(Box::new(VacationForm {
            account: account.to_string(),
            enabled: v.enabled,
            subject: v.subject.clone(),
            text: v.text.clone(),
            dated: v.from.is_some() || v.until.is_some(),
            from: v.from.clone().unwrap_or_default(),
            until: v.until.clone().unwrap_or_default(),
            exchange,
            split: v.external.is_some(),
            external: v.external.clone().unwrap_or_default(),
            outside: if v.outside.is_empty() { "all".into() } else { v.outside.clone() },
            error: None,
        }))
    }

    pub fn password(a: &AccountConfig) -> Dialog {
        Dialog::Password { account: a.id.clone(), email: a.email.clone(), password: String::new() }
    }

    /// Set the error shown in the add-account form.
    pub fn set_error(&mut self, e: String) {
        match self {
            Dialog::AddAccount(f) => f.error = Some(e),
            Dialog::FolderName { error, .. }
            | Dialog::MoveFolder { error, .. }
            | Dialog::DeleteFolder { error, .. }
            | Dialog::NewCategory { error, .. } => *error = Some(e),
            Dialog::EditRule(f) => f.error = Some(e),
            Dialog::SearchFolder(f) => f.error = Some(e),
            Dialog::QuickStep(f) => f.error = Some(e),
            Dialog::EditAccount(f) => f.error = Some(e),
            Dialog::Event(f) => f.error = Some(e),
            Dialog::Contact(f) => f.error = Some(e),
            Dialog::MoveEvent(f) => f.error = Some(e),
            Dialog::Vacation(f) => f.error = Some(e),
            Dialog::OpenSharedCalendar { error, .. } | Dialog::OpenSharedMailbox { error, .. } => *error = Some(e),
            _ => {}
        }
    }

    /// A new rule for mail like this: from its sender, moved to a folder to choose, as Outlook's
    /// Create Rule. Made from a message, it's that message's account's (whose folders it offers),
    /// and it runs on the server too when the server keeps rules (`server`): Exchange's rules work
    /// while Quatrelle is closed. (It used to belong to no account, filed into an Archive folder
    /// an Exchange mailbox may not have — the rule then did nothing.) Once saved it runs on the
    /// mail already in `folder` (the message's: made from one in another folder, it ran on the
    /// Inbox alone, and seemed to do nothing), or the Inbox.
    pub fn rule_from(sender: &str, account: Option<&str>, server: bool, folder: Option<&str>) -> Dialog {
        Dialog::EditRule(Box::new(RuleForm {
            index: None,
            remote: None,
            name: if sender.is_empty() { String::new() } else { tf!("Mail from {sender}", sender = sender) },
            enabled: true,
            match_all: true,
            conditions: vec![RuleCond { test: RuleTest::From, text: sender.to_string() }],
            actions: vec![RuleAct { action: if account.is_some() { RuleDo::MoveTo } else { RuleDo::Archive }, target: String::new() }],
            account: account.map(str::to_string),
            stop: false,
            server: server && account.is_some(),
            run_now: true,
            run_in: folder.filter(|_| account.is_some()).map(str::to_string),
            refused: None,
            error: None,
        }))
    }

    /// The preview of an attachment, from `attachment.preview`'s answer; or a message's
    /// headers, from `message.headers`'.
    pub fn preview(v: &serde_json::Value, clock: &quatrelle_engine::model::dates::Clock) -> Dialog {
        let get = |k: &str| v.get(k).and_then(serde_json::Value::as_str).unwrap_or("").to_string();
        let mut header = Vec::new();
        if get("kind") == "message" {
            header.push((tl!("From").to_string(), get("from")));
            header.push((tl!("To").to_string(), get("to")));
            if let Some(d) = v.get("date").and_then(serde_json::Value::as_i64).filter(|d| *d > 0) {
                header.push((tl!("Date").to_string(), crate::i18n::date(&quatrelle_engine::model::dates::full_time(d, clock))));
            }
        }
        let mut text = get("text");
        if v.get("truncated").and_then(serde_json::Value::as_bool).unwrap_or(false) {
            text.push_str("\n… (the rest isn't shown: open the file to see it all)");
        }
        let title = match get("kind").as_str() {
            "headers" => tl!("Message headers").to_string(),
            "message" if !get("subject").is_empty() => get("subject"),
            _ => get("name"),
        };
        let warning = v.get("warning").and_then(|w| serde_json::from_value(w.clone()).ok());
        Dialog::Preview { title, header, text, warning }
    }

    pub fn new_folder(account: &str, parent: Option<&str>) -> Dialog {
        Dialog::FolderName { account: account.into(), parent: parent.map(str::to_string), rename: None, name: String::new(), error: None }
    }

    pub fn rename_folder(f: &Folder) -> Dialog {
        Dialog::FolderName { account: f.account.clone(), parent: None, rename: Some(f.path.clone()), name: f.display_name(), error: None }
    }

    pub fn move_folder(f: &Folder) -> Dialog {
        Dialog::MoveFolder { account: f.account.clone(), folder: f.path.clone(), filter: String::new(), error: None }
    }

    /// Delete a folder: into Deleted Items, or for good when it's already there.
    pub fn delete_folder(s: &Session, f: &Folder) -> Dialog {
        let trash = s.account(&f.account).and_then(|a| a.folder(quatrelle_engine::model::FolderRole::Trash)).map(|t| t.path.clone());
        let permanent = trash.is_none_or(|t| quatrelle_engine::model::is_within(&f.path, &t, f.delimiter));
        Dialog::DeleteFolder { account: f.account.clone(), folder: f.path.clone(), name: f.display_name(), permanent, error: None }
    }
}

fn title(ui: &mut Ui, t: &Tokens, s: &str) {
    ui.label(RichText::new(tl!(s)).font(theme::bold(18.0)).color(t.text));
    ui.add_space(6.0);
}

/// A dialog's bottom row, right-aligned and one row tall: a layout given all the room left made
/// every dialog as tall as the last one shown, with its buttons floating halfway down.
fn button_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 30.0), egui::Layout::right_to_left(egui::Align::Center), add).inner
}

/// A Close button on its own row; whether the dialog stays open.
fn close_row(ui: &mut Ui) -> bool {
    button_row(ui, |ui| !ui.button(tl!("Close")).clicked())
}

fn buttons(ui: &mut Ui, t: &Tokens, ok: &str, ok_enabled: bool) -> (bool, bool) {
    let mut res = (false, false);
    ui.add_space(10.0);
    button_row(ui, |ui| {
        if widgets::primary_button(ui, ok, t, ok_enabled).clicked() {
            res.0 = true;
        }
        if ui.button(tl!("Cancel")).clicked() {
            res.1 = true;
        }
    });
    res
}

/// Show the dialog; returns whether it stays open.
pub fn show(ctx: &egui::Context, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    let frame = dialog_frame(t);
    // Each kind of dialog keeps its own size: one id for all let a small one open at the size
    // of the last big one.
    // Never taller than the window (issue #1: an invitation's notes put the dialog's title and
    // buttons off a 2160-pixel screen): what doesn't fit scrolls.
    let max = (ctx.content_rect().height() - 48.0).max(200.0);
    let modal = egui::Modal::new(egui::Id::unique(("dialog", std::mem::discriminant(d)))).frame(frame).show(ctx, |ui| {
        // As tall as it needs, up to the window: only then does it scroll.
        egui::ScrollArea::vertical().id_salt("dialog-body").max_height(max).min_scrolled_height(max).show(ui, |ui| body(ui, d, s, t, acts)).inner
    });
    let keep = modal.inner;
    keep && !modal.should_close()
}

fn body(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    // Each kind is drawn by its group; listing them all here keeps the match exhaustive.
    match d {
        Dialog::AddAccount(..)
        | Dialog::Password { .. }
        | Dialog::LostOnClose { .. }
        | Dialog::SaveOnClose(..)
        | Dialog::RemoveAccount { .. }
        | Dialog::MailboxClosed { .. }
        | Dialog::EditAccount(..)
        | Dialog::OpenSharedMailbox { .. } => accounts::draw(ui, d, s, t, acts),
        Dialog::Signature(..) | Dialog::Vacation(..) => mailbox::draw(ui, d, s, t, acts),
        Dialog::FolderSize { .. } => folder_sizes::draw(ui, d, s, t, acts),
        Dialog::ConfirmDelete { .. }
        | Dialog::MoveTo { .. }
        | Dialog::FolderName { .. }
        | Dialog::MoveFolder { .. }
        | Dialog::DeleteFolder { .. }
        | Dialog::FolderProperties { .. }
        | Dialog::ConfirmEmpty { .. } => folders::draw(ui, d, s, t, acts),
        Dialog::NewCategory { .. }
        | Dialog::Rules
        | Dialog::ReplaceOutlookRules { .. }
        | Dialog::EditRule(..)
        | Dialog::SearchFolder(..)
        | Dialog::QuickStep(..) => organize::draw(ui, d, s, t, acts),
        Dialog::SelectNames(..)
        | Dialog::AttachItem(..)
        | Dialog::SaveChanges
        | Dialog::EmptySubject
        | Dialog::SendCheck(_)
        | Dialog::WordCount(..)
        | Dialog::Spelling(..) => writing::draw(ui, d, s, t, acts),
        Dialog::ConfirmDeleteContact { .. }
        | Dialog::OpenShare { .. }
        | Dialog::PicturePreview { .. }
        | Dialog::Preview { .. }
        | Dialog::OpenSharedCalendar { .. }
        | Dialog::Certificate { .. }
        | Dialog::SmartCard { .. }
        | Dialog::Event(..)
        | Dialog::Contact(..) => other::draw(ui, d, s, t, acts),
        Dialog::MoveEvent(f) => moving::draw(ui, f, t, acts),
    }
}

/// How every dialog looks (the main window's, and those over a message's own window).
pub(crate) fn dialog_frame(t: &Tokens) -> egui::Frame {
    egui::Frame::new().fill(t.surface).corner_radius(12).inner_margin(Margin::same(22)).stroke(egui::Stroke::new(1.0, t.border))
}

/// One test of a rule being written.
#[derive(Clone, Debug)]
pub struct RuleCond {
    pub test: RuleTest,
    pub text: String,
}

/// One action of a rule being written.
#[derive(Clone, Debug)]
pub struct RuleAct {
    pub action: RuleDo,
    /// A folder path (`Move to folder`) or a category name (`Categorize`).
    pub target: String,
}

impl RuleCond {
    fn of(c: &Condition) -> RuleCond {
        let (test, text) = match c {
            Condition::From(t) => (RuleTest::From, t.clone()),
            Condition::To(t) => (RuleTest::To, t.clone()),
            Condition::Subject(t) => (RuleTest::Subject, t.clone()),
            Condition::AnyText(t) => (RuleTest::AnyText, t.clone()),
            Condition::HasAttachment => (RuleTest::HasAttachment, String::new()),
            Condition::Bulk => (RuleTest::Bulk, String::new()),
            // "; " between them, so a subject that has a comma keeps it.
            Condition::FromAny(v) => (RuleTest::FromAny, v.join("; ")),
            Condition::ToAny(v) => (RuleTest::ToAny, v.join("; ")),
            Condition::SubjectAny(v) => (RuleTest::SubjectAny, v.join("; ")),
        };
        RuleCond { test, text }
    }
}

impl RuleAct {
    fn of(a: &RuleAction) -> RuleAct {
        let (action, target) = match a {
            RuleAction::MoveTo(f) => (RuleDo::MoveTo, f.clone()),
            RuleAction::MoveToRole(FolderRole::Trash) | RuleAction::Delete => (RuleDo::Delete, String::new()),
            RuleAction::MoveToRole(_) => (RuleDo::Archive, String::new()),
            RuleAction::Categorize(c) => (RuleDo::Categorize, c.clone()),
            RuleAction::MarkRead => (RuleDo::MarkRead, String::new()),
            RuleAction::Flag => (RuleDo::Flag, String::new()),
            RuleAction::Pin => (RuleDo::Pin, String::new()),
        };
        RuleAct { action, target }
    }
}
