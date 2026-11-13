// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
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
mod calendars;
mod closing;
mod folder_sizes;
mod folders;
mod mailbox;
mod move_across;
mod moving;
mod names;
mod organize;
mod other;
pub(crate) mod people_field;
mod pgp;
mod recovering;
pub use recovering::RecoverForm;
pub(crate) use recovering::{link as recover_link, open as open_recovery, recover_blocked};
pub(crate) mod pim_dialogs;
mod writing;

pub use accounts::*;
#[cfg(test)]
pub(crate) use calendars::CalendarAsk;
pub use calendars::{CalendarForm, CalendarsForm};
pub use folder_sizes::SizeSort;
pub use mailbox::*;
pub use moving::MoveForm;
pub use names::{AddressSearch, NamesForm};
pub use organize::*;
pub use other::FileHolds;
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
    /// A server proposed another domain (Autodiscover, a calendar's discovery, JMAP's session):
    /// the account's password or sign-in goes there only if the user allows it.
    TrustDomain {
        account: String,
        site: String,
    },
    /// The data file of an import has changed since it began: bring in what is new?
    ImportChanged {
        account: String,
    },
    ConfirmDelete {
        count: usize,
        /// All of them on Exchange (EWS or Microsoft 365), whose server keeps them a while among
        /// its recoverable items: the question says so, not "this can't be undone".
        exchange: bool,
    },
    /// Closing Quatrelle with messages being written (how many): save them as drafts?
    SaveOnClose(usize),
    /// Closing would lose what online-only accounts hold in memory only (issue #4), or messages
    /// still on their way to the server (`sending`).
    LostOnClose {
        unsent: usize,
        changes: usize,
        sending: usize,
    },
    /// Closing with an update about to be installed, or still downloading: what becomes of it.
    UpdateOnClose(crate::UpdateState),
    /// Delete pressed on an appointment of a series (its uid, the occurrence's start, its
    /// subject): this occurrence, or the series?
    DeleteOccurrence {
        uid: String,
        occurrence: i64,
        name: String,
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
    /// A calendar or contacts file opened with Quatrelle: what it holds and where it would go,
    /// added once the user says so.
    AddFromFile(Box<FileHolds>),
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
        /// The account is Exchange (EWS or Microsoft 365), whose server keeps what is emptied
        /// a while among its recoverable items: the question says so, not "this can't be undone".
        exchange: bool,
    },
    /// Open another mailbox in an Exchange account: a shared one, or someone's who made this
    /// account a delegate (Outlook's Open Other Mailbox).
    OpenSharedMailbox {
        account: String,
        email: String,
        calendar: bool,
        error: Option<String>,
    },
    /// Open other people's calendars, from the address book or typed (Outlook's Open Calendar).
    OpenSharedCalendar(Box<CalendarsForm>),
    /// A calendar made, renamed or deleted, from its menu in the side pane.
    CalendarChange(Box<CalendarForm>),
    /// Subscribe to a calendar by its link (Outlook's Add Calendar › From Internet, issue #7).
    SubscribeCalendar {
        url: String,
        name: String,
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
    /// OpenPGP: the passphrase of a secret key file being imported, from `path`.
    PgpKey {
        account: String,
        path: String,
        passphrase: quatrelle_engine::model::Secret,
    },
    /// OpenPGP: the passphrase (typed twice) a backup of key `fingerprint` is saved behind, at
    /// `path`.
    PgpBackup {
        account: String,
        fingerprint: String,
        path: String,
        passphrase: quatrelle_engine::model::Secret,
        again: quatrelle_engine::model::Secret,
    },
    /// OpenPGP: before forgetting one of the account's keys (`who` it's for; whether a backup
    /// of it was saved).
    PgpForget {
        account: String,
        fingerprint: String,
        who: String,
        backed_up: bool,
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
    /// Recover deleted items: what the server keeps of an account's deletions.
    Recover(Box<RecoverForm>),
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

impl Dialog {
    /// Dialogs that belong to the message being written (shown in its window when it has one).
    /// A question that must be answered before going on, or a warning: read as an alert.
    pub fn is_alert(&self) -> bool {
        self.about_closing()
            || matches!(self, Dialog::CalendarChange(f) if f.deleting())
            || matches!(
                self,
                Dialog::ConfirmDelete { .. }
                    | Dialog::ConfirmDeleteContact { .. }
                    | Dialog::DeleteOccurrence { .. }
                    | Dialog::ConfirmEmpty { .. }
                    | Dialog::DeleteFolder { .. }
                    | Dialog::RemoveAccount { .. }
                    | Dialog::PgpForget { .. }
                    | Dialog::MailboxClosed { .. }
                    | Dialog::TrustDomain { .. }
                    | Dialog::ImportChanged { .. }
                    | Dialog::EmptySubject
                    | Dialog::SaveChanges
                    | Dialog::SendCheck(_)
            )
    }

    /// A question asked as Quatrelle closes.
    pub fn about_closing(&self) -> bool {
        matches!(self, Dialog::SaveOnClose(_) | Dialog::LostOnClose { .. } | Dialog::UpdateOnClose(_))
    }

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
            Dialog::OpenSharedCalendar(f) => f.error = Some(e),
            Dialog::CalendarChange(f) => f.error = Some(e),
            Dialog::SubscribeCalendar { error, .. } | Dialog::OpenSharedMailbox { error, .. } => *error = Some(e),
            _ => {}
        }
    }
}

fn title(ui: &mut Ui, t: &Tokens, s: &str) {
    let r = ui.label(RichText::new(tl!(s)).font(theme::bold(18.0)).color(t.text));
    // What the dialog is called, to a screen reader ([`show`] names the dialog after it).
    ui.ctx().data_mut(|d| d.insert_temp(title_id(), Some(r.id)));
    ui.add_space(6.0);
}

fn title_id() -> egui::Id {
    egui::Id::unique("dialog-title")
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

pub(crate) fn buttons(ui: &mut Ui, t: &Tokens, ok: &str, ok_enabled: bool) -> (bool, bool) {
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

/// Show the dialog; returns whether it stays open. `people` draws the contacts fields a dialog
/// has (Select Names' rows, Add calendar's and Open another mailbox's).
pub(crate) fn show(
    ctx: &egui::Context,
    d: &mut Dialog,
    s: &Session,
    people: &mut people_field::PeopleFields,
    pics: &mut crate::reading::Pictures,
    t: &Tokens,
    acts: &mut Vec<Action>,
) -> bool {
    let frame = dialog_frame(t);
    // Each kind of dialog keeps its own size: one id for all let a small one open at the size
    // of the last big one.
    // Never taller than the window (issue #1: an invitation's notes put the dialog's title and
    // buttons off a 2160-pixel screen): what doesn't fit scrolls.
    let max = (ctx.content_rect().height() - 48.0).max(200.0);
    // A screen reader hears the dialog's title as it opens; a question that must be answered
    // (delete for good? save before closing?) is an alert, as wordcraft's save prompt is.
    let id = egui::Id::unique(("dialog", std::mem::discriminant(d)));
    let role = if d.is_alert() { egui::accesskit::Role::AlertDialog } else { egui::accesskit::Role::Dialog };
    let modal = egui::Modal::new(id).frame(frame).show(ctx, |ui| {
        ui.scope_builder(egui::UiBuilder::new().accessibility_role(role), |ui| {
            // As tall as it needs, up to the window: only then does it scroll.
            let keep = egui::ScrollArea::vertical()
                .id_salt("dialog-body")
                .max_height(max)
                .min_scrolled_height(max)
                .show(ui, |ui| body(ui, d, s, people, pics, t, acts))
                .inner;
            (keep, ui.unique_id())
        })
        .inner
    });
    // Named by its title.
    let (keep, node_id) = modal.inner;
    let named = ctx.data_mut(|data| data.remove_temp::<Option<egui::Id>>(title_id()).flatten());
    ctx.accesskit_node_builder(node_id, |node| {
        node.set_modal();
        if let Some(title) = named {
            node.push_labelled_by(title.accesskit_id());
        }
    });
    // Escape, or a click outside, is a no: the safe answer to a question about a password.
    let closing = modal.should_close();
    if closing && let Dialog::TrustDomain { account, site } = d {
        acts.push(Action::Run("account.refuseDomain", json!({"account": account, "domain": site})));
    }
    // The question about purging: Escape or a click outside is a no, back to the list.
    if closing && let Dialog::Recover(f) = d {
        if f.confirming.take().is_some() {
            return keep;
        }
        acts.push(Action::Run("folder.recoverClose", json!({})));
    }
    keep && !closing
}

fn body(
    ui: &mut Ui,
    d: &mut Dialog,
    s: &Session,
    people: &mut people_field::PeopleFields,
    pics: &mut crate::reading::Pictures,
    t: &Tokens,
    acts: &mut Vec<Action>,
) -> bool {
    // Each kind is drawn by its group; listing them all here keeps the match exhaustive.
    match d {
        Dialog::LostOnClose { .. } | Dialog::SaveOnClose(..) | Dialog::UpdateOnClose(..) => closing::draw(ui, d, s, t, acts),
        Dialog::AddAccount(..)
        | Dialog::Password { .. }
        | Dialog::TrustDomain { .. }
        | Dialog::ImportChanged { .. }
        | Dialog::RemoveAccount { .. }
        | Dialog::MailboxClosed { .. }
        | Dialog::EditAccount(..)
        | Dialog::OpenSharedMailbox { .. } => accounts::draw(ui, d, s, people, t, acts),
        Dialog::Signature(..) | Dialog::Vacation(..) => mailbox::draw(ui, d, s, t, acts),
        Dialog::FolderSize { .. } => folder_sizes::draw(ui, d, s, t, acts),
        Dialog::Recover(..) => recovering::draw(ui, d, s, t, acts),
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
        | Dialog::QuickStep(..) => organize::draw(ui, d, s, people, t, acts),
        Dialog::SelectNames(..)
        | Dialog::AttachItem(..)
        | Dialog::SaveChanges
        | Dialog::EmptySubject
        | Dialog::SendCheck(_)
        | Dialog::WordCount(..)
        | Dialog::Spelling(..) => writing::draw(ui, d, s, people, t, acts),
        Dialog::ConfirmDeleteContact { .. }
        | Dialog::DeleteOccurrence { .. }
        | Dialog::OpenShare { .. }
        | Dialog::AddFromFile(_)
        | Dialog::PicturePreview { .. }
        | Dialog::Preview { .. }
        | Dialog::SubscribeCalendar { .. }
        | Dialog::Certificate { .. }
        | Dialog::SmartCard { .. }
        | Dialog::Event(..)
        | Dialog::Contact(..) => other::draw(ui, d, s, pics, t, acts),
        Dialog::PgpKey { .. } | Dialog::PgpBackup { .. } | Dialog::PgpForget { .. } => pgp::draw(ui, d, t, acts),
        Dialog::OpenSharedCalendar(f) => calendars::draw(ui, f, s, people, t, acts),
        Dialog::CalendarChange(f) => calendars::draw_change(ui, f, s, t, acts),
        Dialog::MoveEvent(f) => moving::draw(ui, f, t, acts),
    }
}

/// How every dialog looks (the main window's, and those over a message's own window).
pub(crate) fn dialog_frame(t: &Tokens) -> egui::Frame {
    egui::Frame::new().fill(t.surface).corner_radius(12).inner_margin(Margin::same(22)).stroke(egui::Stroke::new(1.0, t.border))
}

/// Whether the account is Exchange (EWS or Microsoft 365): its server keeps mail deleted for good
/// a while among the recoverable items.
pub(crate) fn is_exchange(s: &Session, account: &str) -> bool {
    s.account(account).is_some_and(|a| a.config.ews.is_some() || a.config.graph.is_some())
}
