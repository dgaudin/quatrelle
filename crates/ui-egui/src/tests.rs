//! Headless UI tests: real egui frames over the sample mailbox, in every pane layout and dialog,
//! with keyboard shortcuts sent as input events.

use egui::{Event, Key, Modifiers, RawInput};

use quatrelle_engine::model::dates::Clock;

use quatrelle_engine::model::{Event as MailEvent, MessageKey, Summary, SyncStatus};

use quatrelle_engine::store::{FolderState, Store};

use quatrelle_engine::{ReadingPane, Session, sample};

use serde_json::json;

use crate::{Dialog, MailApp, NullBackend, NullServices};

const NOW: i64 = 1_791_979_200;

fn app(name: &str) -> (MailApp, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("quatrelle-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    sample::seed_if_empty(&store, NOW).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    (MailApp::new(s, Box::new(NullBackend), Box::new(NullServices(NOW))), dir)
}

/// The app over a big mailbox (`sample::seed_big_mailbox`): `folders` folders of `per_folder`
/// messages, each from someone else.
fn app_big(name: &str, folders: usize, per_folder: u32) -> (MailApp, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("quatrelle-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    sample::seed_big_mailbox(&store, folders, per_folder, NOW).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    (MailApp::new(s, Box::new(NullBackend), Box::new(NullServices(NOW))), dir)
}

/// The app over the sample mailbox with one more message in its Inbox: an HTML one, as a
/// sender's program would write it.
fn app_with_html(name: &str, subject: &str, html: &str) -> (MailApp, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("quatrelle-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    sample::seed_if_empty(&store, NOW).unwrap();
    let raw = format!(
        "From: Studio Shop <shop@studio.example>\r\nTo: Riley Moreno <riley@studio.example>\r\nSubject: {subject}\r\n\
         Date: Wed, 22 Oct 2026 10:00:00 +0200\r\nMessage-ID: <html-{name}@studio.example>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/html; charset=utf-8\r\n\r\n{html}"
    );
    let account = quatrelle_engine::sample::SAMPLE_ACCOUNT;
    let mut fs = store.load_folder(account, "INBOX").unwrap();
    let uid = fs.uid_next.max(1);
    let key = MessageKey { account: account.into(), folder: "INBOX".into(), validity: fs.validity.max(1), uid };
    store.write_body(&key, raw.as_bytes()).unwrap();
    let flags = quatrelle_engine::model::Flags { seen: true, ..Default::default() };
    fs.messages.push(quatrelle_engine::mime::summary_from_full(raw.as_bytes(), key, flags, raw.len() as u32, NOW));
    fs.uid_next = uid + 1;
    store.save_folder(account, "INBOX", &fs).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    (MailApp::new(s, Box::new(NullBackend), Box::new(NullServices(NOW))), dir)
}

fn frame(ctx: &egui::Context, app: &mut MailApp, events: Vec<Event>) {
    frame_sized(ctx, app, events, egui::vec2(1280.0, 800.0));
}

/// A frame in a window of `size`.
fn frame_sized(ctx: &egui::Context, app: &mut MailApp, events: Vec<Event>, size: egui::Vec2) {
    frame_shapes(ctx, app, events, size);
}

/// A frame in a window of `size`, and what it drew, every grouped shape taken apart.
fn frame_shapes(ctx: &egui::Context, app: &mut MailApp, events: Vec<Event>, size: egui::Vec2) -> Vec<egui::Shape> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| app.ui(ui));
    // No renderer here: drop the texture uploads deliberately.
    out.textures_delta.clear();
    fn flat(s: egui::Shape, out: &mut Vec<egui::Shape>) {
        match s {
            egui::Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
            s => out.push(s),
        }
    }
    let mut shapes = Vec::new();
    for s in out.shapes {
        flat(s.shape, &mut shapes);
    }
    shapes
}

/// The Home ribbon fits the window: at a width that hasn't room for every group's large
/// buttons (1320 px, since Quick Steps came in, and 1100), the last groups are drawn compact —
/// small buttons stacked — as many as it takes, and every group ends within the window; given
/// the room again, they're large again.
#[test]
fn the_ribbon_fits_the_window_by_drawing_its_last_groups_compact() {
    let (mut a, dir) = app("ribbon-fit");
    let ctx = egui::Context::default();
    let fit = |a: &MailApp| crate::RibbonGroups::of(&a.ui, quatrelle_engine::Module::Mail, crate::RibbonTab::Home).cloned().unwrap_or_default();
    let mut compact_at = Vec::new();
    for width in [1320.0, 1100.0] {
        // The first frame measures; the next draws the fit; one more proves it steady.
        for _ in 0..3 {
            frame_sized(&ctx, &mut a, vec![], egui::vec2(width, 800.0));
        }
        let f = fit(&a);
        assert!(f.overflow <= 0.5, "{width} px: the ribbon reaches {} px past the window", f.overflow);
        assert!(f.compact_from >= 1, "New mail stays large");
        compact_at.push(f.compact_from.min(f.columns.len()));
        let again = fit(&a);
        frame_sized(&ctx, &mut a, vec![], egui::vec2(width, 800.0));
        assert_eq!(fit(&a).compact_from, again.compact_from, "{width} px: steady from one frame to the next");
    }
    let f = fit(&a);
    assert!(f.compact_from < f.columns.len(), "1100 px: some groups are compact ({f:?})");
    assert!(compact_at[1] <= compact_at[0], "narrower: at least as many compact groups");
    for _ in 0..3 {
        frame_sized(&ctx, &mut a, vec![], egui::vec2(2200.0, 800.0));
    }
    let f = fit(&a);
    assert_eq!(f.compact_from, f.columns.len(), "room for all: every group large again ({f:?})");
    assert!(f.overflow <= 0.5);
    let _ = std::fs::remove_dir_all(dir);
}

fn key(k: Key, modifiers: Modifiers) -> Event {
    Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers }
}

/// Every pane layout, state and dialog, in light and dark (also run in the pseudo-language by
/// the translation tests).
pub(crate) fn tour() {
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (mut a, dir) = app(&format!("states-{n}"));
    let ctx = egui::Context::default();
    for dark in [Some(false), Some(true)] {
        a.ui.dark = dark;
        frame(&ctx, &mut a, vec![]);
        let k = a.session.list.first().map(|m| m.key.clone()).unwrap();
        a.run("list.open", json!({"key": k.to_id()}));
        frame(&ctx, &mut a, vec![]);
        for pane in [ReadingPane::Bottom, ReadingPane::Off, ReadingPane::Right] {
            a.session.prefs.reading_pane = pane;
            frame(&ctx, &mut a, vec![]);
        }
        a.run("view.conversations", json!({}));
        a.run("view.compact", json!({}));
        frame(&ctx, &mut a, vec![]);
        a.run("view.compact", json!({}));
        a.run("mail.reply", json!({}));
        frame(&ctx, &mut a, vec![]);
        a.run("compose.attach", json!({"name": "a.txt", "data": [1, 2]}));
        a.session.prefs.reading_pane = ReadingPane::Off;
        frame(&ctx, &mut a, vec![]);
        a.session.prefs.reading_pane = ReadingPane::Right;
        a.run("compose.discard", json!({}));
        a.run("message.popOut", json!({"key": k.to_id()}));
        frame(&ctx, &mut a, vec![]);
        a.run("message.closeWindow", json!({"key": k.to_id()}));
        backstage_tour(&mut a, &ctx);
        pim_tour(&mut a, &ctx);
        let projects = a.session.folder("sample", "Projects").cloned().unwrap();
        a.session.oauth_available = vec!["google".into()];
        for d in [
            Dialog::add_account(),
            Dialog::move_to(),
            Dialog::ConfirmDelete { count: 2 },
            Dialog::LostOnClose { unsent: 2, changes: 1 },
            Dialog::ConfirmDeleteContact { uid: "zoe".into(), name: "Zoe Park".into() },
            Dialog::EmptySubject,
            Dialog::SendCheck(vec![
                quatrelle_engine::SendCheck::Lookalike { address: "orders@printsh0p.example".into(), like: "printshop.example".into() },
                quatrelle_engine::SendCheck::Exposed { count: 12 },
            ]),
            Dialog::edit_account(&a.session.accounts[0].config),
            Dialog::vacation(
                "sample",
                &quatrelle_engine::model::job::Vacation {
                    enabled: true,
                    subject: "Away".into(),
                    text: "Back soon".into(),
                    from: Some("2026-10-15".into()),
                    until: None,
                    ..Default::default()
                },
                false,
            ),
            Dialog::vacation(
                "sample",
                &quatrelle_engine::model::job::Vacation {
                    enabled: true,
                    text: "Back soon".into(),
                    external: Some("Out of office".into()),
                    outside: "known".into(),
                    ..Default::default()
                },
                true,
            ),
            Dialog::ConfirmEmpty { account: "sample".into(), folder: "Trash".into(), name: "Deleted Items".into(), for_good: true },
            Dialog::ConfirmEmpty { account: "sample".into(), folder: "INBOX".into(), name: "Inbox".into(), for_good: false },
            Dialog::folder_size(&a.session, "sample"),
            Dialog::FolderProperties {
                name: "Inbox".into(),
                id: "sample".into(),
                account: "riley@studio.example".into(),
                location: "INBOX".into(),
                total: 7,
                unread: 3,
                size: 20_480,
            },
            Dialog::OpenSharedCalendar { email: "ana@contoso.example".into(), error: Some("no access".into()) },
            Dialog::OpenSharedMailbox {
                account: "a1".into(),
                email: "support@contoso.example".into(),
                calendar: true,
                error: Some("no access".into()),
            },
            Dialog::RemoveAccount { account: "sample".into(), email: "x".into() },
            Dialog::MailboxClosed {
                account: "sample".into(),
                email: "zoe@hotmail.com".into(),
                error: format!("{}. Turn on IMAP on Outlook.com.", quatrelle_engine::model::MICROSOFT_IMAP_CLOSED),
            },
            Dialog::add_account_with("zoe@hotmail.com", true),
            Dialog::Signature(Box::new(crate::dialogs::SignatureForm::new(&a.session, "sample", false))),
            Dialog::Signature(Box::new(crate::dialogs::SignatureForm::new(&a.session, "sample", true))),
            Dialog::Password { account: "sample".into(), email: "x".into(), password: String::new() },
            Dialog::Certificate { account: "sample".into(), path: "riley.p12".into(), data: None, password: String::new() },
            Dialog::Certificate { account: "sample".into(), path: String::new(), data: None, password: String::new() },
            Dialog::SmartCard { account: "sample".into(), module: "/usr/lib/opensc-pkcs11.so".into(), pin: "1234".into() },
            Dialog::new_folder("sample", Some("Projects")),
            Dialog::new_folder("sample", None),
            Dialog::rename_folder(&projects),
            Dialog::move_folder(&projects),
            Dialog::delete_folder(&a.session, &projects),
            Dialog::NewCategory { name: "Clients".into(), color: 3, error: Some("taken".into()) },
            Dialog::Rules,
            Dialog::rule_from("zoe@studio.example", Some("sample"), true, Some("Archive")),
            Dialog::spelling(),
            Dialog::add_account_with("zoe@gmail.com", true),
            jmap_form(),
            graph_form(),
            pop_form(),
            Dialog::Preview {
                title: "notes.txt".into(),
                header: vec![("From".into(), "Zoe".into())],
                text: "line one
line two"
                    .into(),
                warning: Some(quatrelle_engine::model::lookalike::SenderWarning::Lookalike {
                    domain: "printsh0p.example".into(),
                    like: "printshop.example".into(),
                }),
            },
            Dialog::MoveEvent(Box::new(crate::dialogs::MoveForm {
                uid: "weekly".into(),
                summary: "Stand-up".into(),
                recurring: true,
                occurrence_allowed: false,
                meeting: true,
                quiet_allowed: false,
                ..Default::default()
            })),
            Dialog::MoveEvent(Box::new(crate::dialogs::MoveForm {
                uid: "review".into(),
                summary: "Review".into(),
                meeting: true,
                quiet_allowed: true,
                error: Some("no".into()),
                ..Default::default()
            })),
        ] {
            a.ui.dialog = Some(d);
            frame(&ctx, &mut a, vec![]);
            frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
            assert!(a.ui.dialog.is_none(), "Escape closes dialogs");
        }
        a.ui.search_text = "storyboard".into();
        a.run("search.run", json!({"query": "storyboard"}));
        frame(&ctx, &mut a, vec![]);
        a.run("search.run", json!({"query": "zzzz-nothing"}));
        frame(&ctx, &mut a, vec![]);
        a.run("search.clear", json!({}));
        a.run("folder.open", json!({"account": "sample", "folder": "Sent"}));
        frame(&ctx, &mut a, vec![]);
        a.run("folder.open", json!({"account": "sample", "folder": "Trash"}));
        frame(&ctx, &mut a, vec![]);
        a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    }
    // The Outbox: its folder row, list and reading pane.
    let store = a.session.store.clone().unwrap();
    let item = quatrelle_engine::store::OutboxItem {
        id: 5,
        from: "riley@studio.example".into(),
        recipients: vec!["ann@x.example".into()],
        queued: NOW,
        error: Some("network: refused".into()),
        ..Default::default()
    };
    store.outbox_add("sample", &item, b"From: riley@studio.example\r\nTo: ann@x.example\r\nSubject: Waiting\r\n\r\nLater.\r\n").unwrap();
    a.session.refresh_outbox();
    frame(&ctx, &mut a, vec![]);
    a.run("folder.outbox", json!({}));
    let k = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.subject.as_str()), Some("Waiting"));
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    // Statuses and a notice toast.
    for st in [
        SyncStatus::Connecting,
        SyncStatus::NeedsPassword { error: "no".into() },
        SyncStatus::Error { error: "x".repeat(300) },
        SyncStatus::Online { at: NOW },
    ] {
        a.session.apply(MailEvent::Status { account: "sample".into(), status: st });
        frame(&ctx, &mut a, vec![]);
    }
    let _ = std::fs::remove_dir_all(dir);
}

fn jmap_form() -> Dialog {
    let mut d = Dialog::add_account_with("zoe@fastmail.example", true);
    if let Dialog::AddAccount(f) = &mut d {
        f.jmap = true;
        f.jmap_url = " https://api.fastmail.example/jmap/session".into();
        f.password = "pw".into();
    }
    d
}

fn pop_form() -> Dialog {
    let mut d = Dialog::add_account_with("ana@example.org", true);
    if let Dialog::AddAccount(f) = &mut d {
        f.imap_host = "imap.example.org".into();
        f.smtp_host = "smtp.example.org".into();
        f.pop = true;
        f.pop_host = "pop.example.org".into();
        f.pop_port = "995".into();
        f.pop_security = quatrelle_engine::model::Security::Tls;
        f.pst = "C:/Mail/old.pst".into();
        f.password = "pw".into();
    }
    d
}

fn graph_form() -> Dialog {
    let mut d = Dialog::add_account_with("ana@contoso.example", true);
    if let Dialog::AddAccount(f) = &mut d {
        f.graph = true;
    }
    d
}

/// The browser's services: a popped-out message goes to a browser window of its own.
struct PageServices(std::rc::Rc<std::cell::RefCell<Vec<String>>>);

impl crate::Services for PageServices {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
    fn save_path(&mut self, _: &str) -> Option<std::path::PathBuf> {
        None
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Err("no".into())
    }
    fn open_message_window(&mut self, key: &str) -> bool {
        self.0.borrow_mut().push(key.to_string());
        true
    }
    fn close_page(&mut self) {
        self.0.borrow_mut().push("closed".into());
    }
}

/// The File page: every page, in the tour's languages and themes.
pub(crate) fn backstage_tour(a: &mut MailApp, ctx: &egui::Context) {
    for page in [crate::backstage::Page::Info, crate::backstage::Page::Open, crate::backstage::Page::Options, crate::backstage::Page::About] {
        a.act(crate::Action::Backstage(page));
        frame(ctx, a, vec![]);
    }
    // Compact Now: the window while it goes on, and the page with its sizes after.
    if let Some(id) = a.session.accounts.first().map(|x| x.config.id.clone()) {
        a.act(crate::Action::Backstage(crate::backstage::Page::Info));
        a.session.compacting = Some(id.clone());
        frame(ctx, a, vec![]);
        a.session.compacting = None;
        a.session.compacted = Some((id, 40_000_000_000, 12_000_000_000));
        frame(ctx, a, vec![]);
        a.session.compacted = None;
    }
    a.ui.backstage = None;
}

/// Calendar (each view, an event open) and People (someone picked, the form open).
pub(crate) fn pim_tour(a: &mut MailApp, ctx: &egui::Context) {
    a.run("view.module", json!({"name": "calendar"}));
    for mode in ["day", "workWeek", "week", "month"] {
        a.run("calendar.view", json!({"mode": mode}));
        frame(ctx, a, vec![]);
    }
    let item =
        a.session.agenda(a.session.local_midnight(a.session.cal_days().0), a.session.local_midnight(a.session.cal_days().1)).into_iter().next();
    if let Some(it) = item {
        a.ui.dialog = Dialog::open_event(&a.session, &it.uid, it.occurrence);
        frame(ctx, a, vec![]);
    }
    a.ui.dialog = Some(Dialog::new_event(&a.session, None));
    frame(ctx, a, vec![]);
    // A meeting with guests and a room, its Room Finder open and its Scheduling Assistant
    // showing everyone's hours (the rooms and hours still being read too).
    let mut d = Dialog::new_event(&a.session, None);
    if let Dialog::Event(f) = &mut d {
        f.guests = "Zoe Park <zoe@studio.example>".into();
        f.add_room("Sound Lab", "sound.lab@studio.example");
        f.finder = Some(crate::pim_dialogs::RoomFinderUi::default());
        f.scheduling = true;
    }
    a.ui.dialog = Some(d);
    frame(ctx, a, vec![]);
    frame(ctx, a, vec![]);
    a.session.rooms.insert("sample".into(), quatrelle_engine::Fetched::Loading);
    a.session.free_busy.clear();
    frame(ctx, a, vec![]);
    a.session.rooms.insert("sample".into(), quatrelle_engine::Fetched::Failed("no directory".into()));
    frame(ctx, a, vec![]);
    a.ui.dialog = None;
    a.run("view.module", json!({"name": "people"}));
    frame(ctx, a, vec![]);
    a.ui.person = a.session.people("").first().map(|p| p.card.uid.clone());
    frame(ctx, a, vec![]);
    if let Some(p) = a.session.people("").first() {
        a.ui.dialog = Some(Dialog::edit_contact(&p.card));
        frame(ctx, a, vec![]);
    }
    a.ui.dialog = None;
    a.ui.people_query = "nobody-at-all".into();
    frame(ctx, a, vec![]);
    a.ui.people_query.clear();
    a.run("view.module", json!({"name": "mail"}));
    frame(ctx, a, vec![]);
}

/// Services that remember the notifications shown.
struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>);

impl crate::Services for Recorder {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
    fn save_path(&mut self, _: &str) -> Option<std::path::PathBuf> {
        None
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    fn notify(&mut self, title: &str, body: &str) {
        self.0.lock().unwrap().push((title.into(), body.into()));
    }
    fn clipboard_picture(&mut self) -> Option<Vec<u8>> {
        Some(quatrelle_engine::sample::sample_png())
    }
    fn can_drag_out(&self) -> bool {
        true
    }
}

/// The browser's services: no files, so downloads and a picker that answers later.
#[derive(Default)]
struct WebLike {
    downloads: std::rc::Rc<std::cell::RefCell<Vec<(String, usize)>>>,
    picked: Vec<(String, String, Vec<u8>)>,
    asked: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
}

impl crate::Services for WebLike {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
        panic!("the browser can't pick files at once")
    }
    fn save_path(&mut self, _: &str) -> Option<std::path::PathBuf> {
        panic!("the browser has no paths")
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        panic!("the browser has no paths")
    }
    fn downloads(&self) -> bool {
        true
    }
    fn download(&mut self, name: &str, bytes: &[u8]) {
        self.downloads.borrow_mut().push((name.to_string(), bytes.len()));
    }
    fn pick_later(&mut self, purpose: &str) -> bool {
        self.asked.borrow_mut().push(purpose.to_string());
        self.picked.push((purpose.to_string(), "notes.txt".into(), b"remember".to_vec()));
        true
    }
    fn take_picked(&mut self) -> Vec<(String, String, Vec<u8>)> {
        std::mem::take(&mut self.picked)
    }
}

/// Services whose clock the test moves, remembering the notifications shown (reminders with
/// their key and buttons; taken back: `("withdrawn", key)`), and handing back what the test
/// "pressed" on them.
struct Ticking(
    std::sync::Arc<std::sync::Mutex<i64>>,
    std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
    std::sync::Arc<std::sync::Mutex<Vec<(String, crate::ReminderAction)>>>,
);

impl crate::Services for Ticking {
    fn clock(&self) -> (i64, i32) {
        (*self.0.lock().unwrap(), 0)
    }
    fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
    fn save_path(&mut self, _: &str) -> Option<std::path::PathBuf> {
        None
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    fn notify(&mut self, title: &str, body: &str) {
        self.1.lock().unwrap().push((title.into(), body.into()));
    }
    fn notify_reminder(&mut self, key: &str, title: &str, body: &str, snooze: &str, dismiss: &str) {
        self.1.lock().unwrap().push((title.into(), format!("{body} [{key}] {snooze}/{dismiss}")));
    }
    fn take_reminder_actions(&mut self) -> Vec<(String, crate::ReminderAction)> {
        std::mem::take(&mut *self.2.lock().unwrap())
    }
    fn withdraw_reminder(&mut self, key: &str) {
        self.1.lock().unwrap().push(("withdrawn".into(), key.into()));
    }
}

mod accounts;
mod organize;
mod pim;
mod reading;
mod scheduling;
mod windows;
mod writing;

/// The card the calendar shows when the pointer rests on an appointment, as Outlook's: when,
/// where, and the calendar it's in.
#[test]
fn the_calendar_card_says_when_and_where_an_appointment_is() {
    use crate::icons::Icon;
    let (a, dir) = app("calendar-card");
    let now = a.session.clock.now;
    let items = a.session.agenda(now - 86_400, now + 2 * 86_400);
    let lunch = items.iter().find(|i| i.summary == "Lunch with Jun").expect("the sample's lunch");
    let rows = crate::calendar::card_rows(&a.session, lunch);
    assert_eq!(rows.len(), 3);
    assert!(matches!(rows[0].0, Icon::Clock));
    assert!(rows[0].1.contains("12:30 – 13:30"), "{}", rows[0].1);
    assert_eq!(rows[1].1, "Café on the corner");
    assert_eq!(rows[2].1, "Calendar");
    // With a reminder, the card says when it sounds, as Outlook's does.
    let review = items.iter().find(|i| i.summary == "Storyboard review").expect("the sample's review");
    let rows = crate::calendar::card_rows(&a.session, review);
    assert!(rows.iter().any(|(icon, text)| matches!(icon, Icon::Reminder) && text == "15 minutes before"), "{rows:?}");
    assert_eq!(crate::calendar::reminder_text(1440), "1 day before", "days and hours say so, rather than 1440 minutes");
    assert_eq!(crate::calendar::reminder_text(0), "When it starts");
    // An all-day event with no place says the day, and has no place line.
    let poster = items.iter().find(|i| i.summary == "Poster to the print shop").expect("the sample's all-day event");
    let rows = crate::calendar::card_rows(&a.session, poster);
    assert_eq!(rows.len(), 2);
    assert!(rows[0].1.ends_with("All day"), "{}", rows[0].1);
    assert!(matches!(rows[1].0, Icon::Calendar));
    let _ = std::fs::remove_dir_all(dir);
}

/// The spelling dialog shows the word the dictionaries don't know, with what could replace it,
/// and Change is the command that puts the suggestion in its place.
#[test]
fn the_spelling_dialog_offers_a_correction() {
    let (mut a, dir) = app("spelling");
    assert!(crate::spellcheck::load_for_test(), "the dictionaries read");
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ann@x.example", "subject": "Plan", "body": "Le servveur de la salle"}));
    let ctx = egui::Context::default();
    a.ui.dialog = Some(Dialog::spelling());
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(a.ui.dialog.is_some(), "the dialog stays open");
    let words = crate::spellcheck::misspelt("Le servveur de la salle");
    let (range, word) = words.iter().find(|(_, w)| w == "servveur").cloned().expect("the word the dialog shows");
    assert!(!crate::spellcheck::suggest(&word, 8).is_empty(), "it has suggestions");
    // What Change runs.
    a.run("compose.replaceText", json!({"from": range.start, "to": range.end, "text": "serveur"}));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.compose.as_ref().map(|c| c.body.clone()).unwrap_or_default(), "Le serveur de la salle");
    // The subject is checked too, and corrected with the same command.
    a.run("compose.update", json!({"subject": "Le servveur est là"}));
    let subject = a.session.compose.as_ref().map(|c| c.subject.clone()).unwrap_or_default();
    let (sr, _) = crate::spellcheck::misspelt(&subject).into_iter().find(|(_, w)| w == "servveur").expect("the subject's word");
    a.run("compose.replaceText", json!({"from": sr.start, "to": sr.end, "text": "serveur", "field": "subject"}));
    assert_eq!(a.session.compose.as_ref().map(|c| c.subject.clone()).unwrap_or_default(), "Le serveur est là");
    // Underlining can be turned off; the dialog still checks.
    a.run("view.spelling", json!({"on": false}));
    crate::spellcheck::set_on(a.session.prefs.spell_check);
    let mut job = egui::text::LayoutJob::simple("servveur".into(), egui::FontId::default(), egui::Color32::WHITE, 400.0);
    crate::spellcheck::underline(&mut job, "servveur", egui::Color32::RED);
    assert!(job.sections.iter().all(|s| s.format.underline.width == 0.0), "nothing underlined when it's off");
    crate::spellcheck::set_on(true);
    // Added to the dictionary: no longer underlined, from the preferences' list.
    a.run("spelling.add", json!({"word": "Servveur"}));
    crate::spellcheck::add_word("servveur");
    assert!(crate::spellcheck::misspelt("Le servveur").is_empty());
    assert!(a.session.prefs.spelling.iter().any(|w| w == "servveur"));
    let _ = std::fs::remove_dir_all(dir);
}
