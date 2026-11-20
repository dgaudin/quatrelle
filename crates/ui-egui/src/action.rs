// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What a pane, a dialog or a window asks the app to do (`Action`): drawn first, done after
//! (`app/actions.rs`), so that drawing only borrows what it reads.

use crate::*;

/// Something a pane asks for; run after drawing so panes only borrow what they read.
pub enum Action {
    Run(&'static str, Value),
    /// Close Quatrelle (File › Exit), asking first as any close does.
    Quit,
    /// Close Quatrelle though online-only accounts hold unsent mail or changes (issue #4).
    CloseAnyway,
    /// Close, install the downloaded version, and open it.
    RestartToUpdate,
    /// Closing, with the update asked about: installed as Quatrelle closes (true), or kept for
    /// later (false).
    UpdateOnClose(bool),
    Open(MessageKey),
    /// Delete the mail selected (Delete; Shift+Delete, `for_good`), asked first where that's
    /// for good and the option says so: one answer for the keys, the ribbon, a row's button and
    /// menu, and the reading pane's buttons, taken once what's asked before it (a row selected)
    /// is done.
    DeleteMail {
        for_good: bool,
    },
    Dialog(Dialog),
    /// An action from one message's window: done to that message (several can be written at
    /// once, each in a window of its own).
    ForCompose(u64, Box<Action>),
    /// A command from an appointment's window (its number): the window closes once it's done,
    /// or says why it couldn't be.
    ForEvent(u64, &'static str, Value),
    /// A Webex meeting asked for from an appointment's window (its number), with
    /// `webex.createMeeting`'s params: the window waits for the answer, or says why not.
    ForWebex(u64, Value),
    /// A Teams meeting asked for from an appointment's window (its number), with
    /// `event.teamsMeeting`'s params (`{"add": …}`): the window waits for the answer, or says why
    /// not. A window waiting for one already asks nothing (T5).
    ForTeamsMeeting(u64, Value),
    OpenAttachment(usize),
    /// Open an attachment whose page or macros `Action::OpenAttachment` asked about first.
    OpenAttachmentNow(usize),
    SaveAttachment(usize),
    /// Open or save one of an appointment's or a contact's files.
    OpenItemFile(pim_dialogs::ItemFile),
    /// Open the file `Action::OpenItemFile` asked about first (a page, a document with macros).
    OpenItemFileNow(pim_dialogs::ItemFile),
    SaveItemFile(pim_dialogs::ItemFile),
    AttachFiles,
    /// Attach Item, open on a kind of item.
    AttachItem(quatrelle_engine::ItemKind),
    FocusSearch,
    /// Open the reading pane's find bar (More actions › Find in message).
    FindInMessage,
    /// Make a note and write on it.
    NewNote,
    /// Show a task in Tasks (from its reminder).
    OpenTask(String),
    /// Search every folder for this (shown in the box, with the Search tab).
    Search(String),
    SetDark(Option<bool>),
    /// Show one message formatted (`true`) or as plain text, whatever the setting for all mail
    /// says (that stays as it is).
    ShowFormatted(MessageKey, bool),
    /// Show one message's formatting as Markdown text (`true`), or as before.
    ShowMarkdown(MessageKey, bool),
    SetSystemAccent(bool),
    /// Print the open message: the system's print window, else a PDF opened in the viewer.
    Print,
    /// The open message as a PDF opened in the viewer, to print there (no print window here).
    PrintAsPdf,
    /// Save the open message as a PDF, or as its `.eml` source.
    ExportPdf,
    SaveMessage,
    /// Open an attachment in a Craft app (its command name).
    OpenInCraft(usize, &'static str),
    /// Save every attachment of the open message into a folder.
    SaveAll,
    /// Save some attachments of the open message (by index) into a folder.
    SaveAttachments(Vec<usize>),
    /// The messages selected in the list, as `.eml` files (one where asked; several in a folder).
    SaveSelected,
    /// These messages as `.eml` files (Save As on those selected, once the place is said).
    SaveMessages(Vec<MessageKey>),
    /// The rules to a file, or rules from one (Outlook's Export and Import Rules).
    ExportRules,
    ImportRules,
    /// Pick a `.p12` / `.pfx` file for an account's certificate, then ask its password.
    ImportCertificate(String),
    /// The browser: open the account's certificate it keeps, with its password, for this page.
    UnlockCertificate(String),
    /// Choose a certificate on a smart card (or in the system's keychain) for an account.
    UseSmartCard(String),
    /// The OpenPGP card's: a key file to pick or a place to save one, then its dialog or command.
    Pgp(backstage::PgpAsk),
    /// Sign an account in again: the browser for OAuth accounts, else the password dialog.
    SignInAgain(String),
    /// Insert a picture at the cursor of the message being written (from a file).
    InsertPicture,
    /// A picture into the signature being made (a logo), from a file.
    SignaturePicture,
    /// The account's signature at the cursor of the message being written.
    InsertSignature,
    /// Paste a picture from the clipboard, if there is one.
    PastePicture,
    /// Put a picture pasted (a screenshot: its PNG) in the message being written.
    PasteImage(Vec<u8>),
    /// Choose the interface language (`auto` or a code).
    SetLanguage(String),
    /// Open the signature editor for an account (`insert`: from a message, putting it in).
    EditSignature {
        account: String,
        insert: bool,
    },
    /// A link in a message was clicked (web pages open in the browser; `mailto:` starts a message;
    /// a network share asks first, unless its server is trusted).
    OpenUrl(String),
    /// A meeting's link was clicked in an event or meeting (its Join button, its notes): a Teams
    /// meeting joins in a browser's app window, as File › Options › Applications says; any other
    /// link opens as [`Action::OpenUrl`] does.
    OpenMeeting(String),
    /// File › Options › Applications' "Create a profile for meetings": the meeting browser opens
    /// Teams in Quatrelle's own profile, which it makes, for the user to sign in to once; once it
    /// has opened, that profile is the one meetings open in.
    MakeMeetingProfile,
    /// A network share's link the user said to open ([`Dialog::OpenShare`]).
    OpenShare(String),
    /// A calendar or contacts file the user said to add ([`Dialog::AddFromFile`]): its path.
    AddFile(String),
    /// Run a command for the open dialog: it closes on success and shows the error otherwise.
    Submit(&'static str, Value),
    /// Server settings' Save with a domain typed and not added: `account.trustDomain`'s params,
    /// then `account.update`'s. A domain refused leaves the dialog open, saying why, nothing saved.
    SaveWithDomain(Value, Value),
    /// The open dialog's OK once its field's names are settled (Add calendar, Open another mailbox).
    DialogOk,
    /// Bring the main window to the front (after acting from a message window).
    FocusMain,
    /// Open the File page at a page.
    Backstage(backstage::Page),
    /// Open File › Options, scrolled to the Webex card.
    WebexOptions,
    /// Open File › Options, scrolled to the Teams card.
    TeamsOptions,
    /// Read a typeface the message being written uses (Arial, Calibri…) from the system, so
    /// the editor draws it as it will be read.
    LoadTypeface(String),
    /// An attachment is being dragged out of the window: hand it to the system as a file.
    DragOut(usize),
    /// Choose an Outlook data file and open it.
    OpenDataFile,
    /// Go on with an account's unfinished import (the file dialog asks where the file is when it
    /// isn't where it was).
    ResumeImport(String),
    /// Choose an existing data file for the account being added (its mail comes in first).
    PickAccountDataFile,
    /// An action from a message's own window, on that message (its ribbon's and header's buttons,
    /// its attachments, Save As, Print), whatever the main window has selected.
    InWindow(MessageKey, Box<Action>),
    /// Choose where to write an account's mail as an Outlook data file (a new one, by Save as).
    ExportDataFile(String),
    /// Choose an Outlook data file already there to add an account's mail to (by Open).
    AddToDataFile(String),
    /// Show a folder in the system's file manager.
    OpenFolder(PathBuf),
    /// Choose a folder to move Quatrelle's data into (at the next start).
    MoveDataFolder,
    /// Forget a move asked for.
    CancelDataMove,
    /// Say why something can't be done now, as a command's refusal is said (a message whose
    /// text came first: its attachments are still coming).
    Refuse(String),
    /// What a Teams conversation's box asks of the window: the file dialog, the clipboard's picture.
    Teams(conversations::TeamsAct),
}
