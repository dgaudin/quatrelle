//! The mailbox over EWS: the folder list, searching, voting, public folders and shared mailboxes.

use super::*;

/// Gap #7: searching Exchange through EWS (`QueryString`, and a `Restriction` for the read
/// state) for mail past the window.
#[test]
fn ews_is_searched_past_the_window() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let mut ids = Vec::new();
    for n in 1..=25 {
        ids.push(
            d.deliver(
                &inbox,
                &format!(
                    "From: Zoe Park <zoe@x.example>\r\nTo: me@x.example\r\nSubject: Message {n}\r\nMessage-ID: <s{n}@x>\r\n\r\nBody tag{n}.\r\n"
                )
                .into_bytes(),
                n > 5,
                &format!("2024-05-{n:02}T09:00:00Z"),
            ),
        );
    }
    let server = serve(d);
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    let data = server.data.clone();
    crate::fake::search_the_server_past_the_window("ews-search", account, PASSWORD, &|n, read| {
        let mut d = data.lock().unwrap();
        let id = ids[n as usize - 1].clone();
        if read {
            d.items.iter_mut().filter(|m| m.id == id).for_each(|m| m.read = true);
        } else {
            d.items.retain(|m| m.id != id);
        }
    });
}

/// A folder flagged hidden is not offered as mail, nor is anything nested in it at any
/// depth; its siblings and their subfolders still are.
#[test]
fn exchanges_own_folders_are_left_out_of_the_list() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let projects = d.folder("Projects", None, None, "IPF.Note");
    let working = d.folder("Working Set", None, None, "IPF.Note");
    let inner = d.folder("Inner", Some(&working), None, "IPF.Note");
    d.folder("Deeper", Some(&inner), None, "IPF.Note");
    d.folder("2024", Some(&projects), None, "IPF.Note");
    d.flagged_hidden.push(working);
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let mut names: Vec<String> = c.mailboxes().unwrap().into_iter().map(|m| m.name).collect();
    names.sort();
    assert_eq!(names, ["2024", "Inbox", "Projects"]);
}

/// Voting buttons chosen while writing go to Exchange as Outlook's verb stream, and the
/// header that carried them to the connection doesn't go out with the message.
#[test]
fn voting_buttons_go_out_as_outlook_reads_them() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let sent = d.folder("Sent Items", None, Some("sentitems"), "IPF.Note");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let draft = quatrelle_mime::Draft {
        to: quatrelle_model::addr::parse_list("Zoe <zoe@contoso.example>").unwrap(),
        subject: "Lunch on Friday?".into(),
        body: "Say which.".into(),
        voting: "Oui ; Non;Peut-être".into(),
        ..Default::default()
    };
    let from = quatrelle_model::addr::parse_list("Riley <riley@contoso.example>").unwrap().remove(0);
    let raw = quatrelle_mime::build(&draft, &from, 1_792_000_000, "lunch-1@contoso.example").unwrap();
    assert!(String::from_utf8_lossy(&raw).contains(quatrelle_mime::VOTING_HEADER));
    crate::ews::send(&c, &raw, &["zoe@contoso.example".into()]).unwrap();
    let data = server.data.lock().unwrap();
    let item = data.items.iter().find(|i| i.folder == sent).unwrap();
    assert_eq!(quatrelle_model::voting::choices(&item.voting), ["Oui", "Non", "Peut-être"]);
    assert!(data.sent.iter().all(|m| !String::from_utf8_lossy(m).contains(quatrelle_mime::VOTING_HEADER)));
    assert!(String::from_utf8_lossy(&item.raw).contains("Lunch on Friday?"));
}

/// Exchange's hidden folders (Working Set…) aren't listed: Outlook doesn't show them, and
/// Quatrelle's sync stalled on one. Someone's calendar asked for by an alias opens by the
/// address the directory has.
#[test]
fn hidden_folders_stay_out_and_an_alias_finds_the_calendar() {
    use quatrelle_model::pim::{Pim, shared_calendar_id};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let ws = d.folder("Working Set", None, None, "IPF.Note");
    d.hidden.push(ws);
    d.folder("Yammer Root", None, None, "");
    d.folder("Projects", None, None, "IPF.Note");
    let review = parse(
        "<t:CalendarItem><t:Subject>Budget review</t:Subject><t:Start>2026-10-06T13:00:00Z</t:Start><t:End>2026-10-06T14:00:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>budget-1</t:UID></t:CalendarItem>",
    );
    d.pim.push(("S1".into(), "mailbox:ana.lima@contoso.example".into(), review.kids[0].clone()));
    d.directory = vec![("ana@contoso.example".into(), "Ana.Lima@contoso.example".into())];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let names: Vec<String> = c.mailboxes().unwrap().into_iter().map(|m| m.name).collect();
    assert!(names.contains(&"Projects".to_string()) && names.contains(&"Inbox".to_string()), "{names:?}");
    assert!(!names.iter().any(|n| n == "Working Set" || n == "Yammer Root"), "{names:?}");
    let pim = Pim { shared: vec!["ana@contoso.example".into()], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    let id = shared_calendar_id("ana@contoso.example");
    assert!(sync.events.iter().any(|e| e.calendar == id && e.event.summary == "Budget review"), "{:?}", sync.errors);
}

#[test]
fn public_folders_list_under_their_own_folder_and_sync() {
    // Exchange 2013 and later: the hierarchy and the content each in a public folder mailbox
    // of their own, reached only when routed there.
    let mut d =
        Data { pf_mailbox: Some("PFContent@contoso.example".into()), pf_hierarchy: Some("PFHierarchy@contoso.example".into()), ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let root = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
    let team = d.folder("Team", Some(&root), None, "IPF.Note");
    let news = d.folder("Announcements", Some(&team), None, "IPF.Note");
    let rooms = d.folder("Rooms", Some(&root), None, "IPF.Appointment");
    let shared = d.folder("Company contacts", Some(&team), None, "IPF.Contact");
    d.pim.push((
        "PC1".into(),
        shared,
        parse("<t:Contact><t:ItemId Id=\"PC1\" ChangeKey=\"k\"/><t:DisplayName>Front desk</t:DisplayName><t:EmailAddresses><t:Entry Key=\"EmailAddress1\">desk@contoso.example</t:Entry></t:EmailAddresses></t:Contact>")
            .kids[0]
            .clone(),
    ));
    d.pim.push((
        "PE1".into(),
        rooms,
        parse("<t:CalendarItem><t:Subject>Room 2 booked</t:Subject><t:Start>2026-10-16T09:00:00Z</t:Start><t:End>2026-10-16T10:00:00Z</t:End><t:UID>room-1</t:UID></t:CalendarItem>")
            .kids[0]
            .clone(),
    ));
    d.deliver(
        &news,
        b"From: hr@contoso.example\r\nSubject: Holiday hours\r\nMessage-ID: <h@x>\r\n\r\nClosed Friday.\r\n",
        false,
        "2026-10-14T09:00:00Z",
    );
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let boxes = c.mailboxes().unwrap();
    let names: Vec<&str> = boxes.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["Inbox", "Public Folders", "Team", "Announcements"], "the room calendar isn't mail");
    let pf = boxes.iter().find(|m| m.name == "Public Folders").unwrap();
    let team_box = boxes.iter().find(|m| m.name == "Team").unwrap();
    assert_eq!((pf.parent.as_deref(), team_box.parent.as_deref()), (None, Some(pf.id.as_str())));
    assert_eq!(boxes.iter().find(|m| m.name == "Announcements").unwrap().parent.as_deref(), Some(team_box.id.as_str()));
    // Their mail syncs as any folder's.
    let store = crate::fake::tests_store("public-folders");
    let mut st = crate::jmap::State::default();
    let folders = crate::ews::list_folders(&c, &mut st, "a1").unwrap();
    let news_path = folders.iter().find(|f| f.path.ends_with("Announcements")).map(|f| f.path.clone()).unwrap();
    crate::ews::sync_folder(&c, &mut st, &store, "a1", &news_path, &crate::sync::SyncOptions::default()).unwrap();
    let fs = store.load_folder("a1", &news_path).unwrap();
    assert_eq!(fs.messages.len(), 1);
    assert_eq!(fs.messages[0].subject, "Holiday hours");
    let _ = std::fs::remove_dir_all(store.root());
    // Issue #1: the organisation's shared contacts and calendars come in, read-only.
    let sync = crate::ews_pim::sync(&c, &quatrelle_model::pim::Pim::default(), 0).unwrap();
    let books = sync.books.clone().unwrap_or_default();
    let book = books.iter().find(|b| b.id.starts_with("public:")).unwrap();
    assert_eq!((book.name.as_str(), book.read_only), ("Team › Company contacts", true));
    assert!(sync.cards.iter().any(|k| k.book == book.id && k.card.display() == "Front desk"));
    let cals = sync.calendars.clone().unwrap_or_default();
    let room = cals.iter().find(|c| c.id.starts_with("public:")).unwrap();
    assert_eq!((room.name.as_str(), room.read_only), ("Rooms", true));
    assert!(sync.events.iter().any(|e| e.calendar == room.id && e.event.summary == "Room 2 booked"));
    // A server without public folders lists the mailbox's alone.
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    assert_eq!(c.mailboxes().unwrap().len(), 1);
}

/// A message whose source the server won't give doesn't stop the folder: the others
/// download, and the folder syncs.
#[test]
fn one_unreadable_message_holds_up_nothing() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.deliver(&inbox, &mail("one", "m1"), false, "2026-10-01T09:00:00Z");
    let bad = d.deliver(&inbox, &mail("two", "m2"), false, "2026-10-01T10:00:00Z");
    d.deliver(&inbox, &mail("three", "m3"), false, "2026-10-01T11:00:00Z");
    d.unreadable.push(bad);
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let store = crate::fake::tests_store("ews-unreadable");
    let mut st = crate::jmap::State::default();
    crate::ews::list_folders(&c, &mut st, "a1").unwrap();
    crate::ews::sync_folder(&c, &mut st, &store, "a1", "INBOX", &crate::sync::SyncOptions::default()).unwrap();
    let fs = store.load_folder("a1", "INBOX").unwrap();
    assert_eq!(fs.messages.len(), 3);
    let with_body: Vec<&str> = fs.messages.iter().filter(|m| store.has_body(&m.key)).map(|m| m.subject.as_str()).collect();
    assert_eq!(with_body, ["three", "one"], "the others downloaded");
    let _ = std::fs::remove_dir_all(store.root());
}

/// Issue #1: a mailbox made in Dutch showed "Inbox", "Sent Items"… where Outlook shows the
/// names Exchange gave them ("Postvak IN", "Verzonden items"); they're kept now, and the
/// folders are still the account's Inbox, Sent Items… A folder of the user's own is as it was.
#[test]
fn special_folders_keep_the_names_exchange_gives_them() {
    let mut d = Data::default();
    d.folder("Postvak IN", None, Some("inbox"), "IPF.Note");
    d.folder("Verzonden items", None, Some("sentitems"), "IPF.Note");
    d.folder("Verwijderde items", None, Some("deleteditems"), "IPF.Note");
    d.folder("Afgehandeld", None, None, "IPF.Note");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let folders = crate::ews::list_folders(&c, &mut crate::jmap::State::default(), "a1").unwrap();
    let named = |role: quatrelle_model::FolderRole| folders.iter().find(|f| f.role == role).map(|f| (f.path.clone(), f.display_name())).unwrap();
    assert_eq!(named(quatrelle_model::FolderRole::Inbox), ("INBOX".to_string(), "Postvak IN".to_string()));
    assert_eq!(named(quatrelle_model::FolderRole::Sent).1, "Verzonden items");
    assert_eq!(named(quatrelle_model::FolderRole::Trash).1, "Verwijderde items");
    let own = folders.iter().find(|f| f.path == "Afgehandeld").unwrap();
    assert_eq!((own.name.as_deref(), own.display_name().as_str()), (None, "Afgehandeld"));
}

/// Issue #1: a shared mailbox's calendar never showed (only calendars opened by hand were
/// read), and every account's own calendar was "Calendar" whatever the server called it.
/// The shared mailbox's calendar is read with its mail, named after it, editable as its
/// rights say; one without a calendar for the account is passed over quietly; the account's
/// own calendar and contacts keep the server's names.
#[test]
fn a_shared_mailboxs_calendar_is_read_with_it() {
    let mut d = Data::default();
    d.folder("Postvak IN", None, Some("inbox"), "IPF.Note");
    d.folder("Agenda", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contactpersonen", None, Some("contacts"), "IPF.Contact");
    d.shared_mailbox(SharedBox { address: "sales@contoso.example".into(), name: "Sales".into(), automap: true, legacy: false, granted: true });
    d.shared_mailbox(SharedBox { address: "boss@contoso.example".into(), name: "Boss".into(), automap: true, legacy: false, granted: false });
    let cal = d.id("S");
    d.folders.push(Fo {
        id: cal.clone(),
        name: "Calendar".into(),
        parent: String::new(),
        class: "IPF.Appointment".into(),
        known: Some("calendar".into()),
        owner: "sales@contoso.example".into(),
    });
    let plan = parse(
        "<t:CalendarItem><t:Subject>Team planning</t:Subject><t:Start>2026-10-20T09:00:00Z</t:Start><t:End>2026-10-20T10:00:00Z</t:End>             <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>team-plan-1</t:UID></t:CalendarItem>",
    );
    d.pim.push(("T1".into(), cal, plan.kids[0].clone()));
    // A Teams meeting in the account's own calendar, its join link longer than a text line.
    let agenda = d.folders.iter().find(|f| f.name == "Agenda").map(|f| f.id.clone()).unwrap();
    let long = format!("https://teams.microsoft.com/l/meetup-join/19%3ameeting_{}%40thread.v2/0?context=%7b%22Tid%22%7d", "Nj".repeat(300));
    let teams = parse(&format!(
        "<t:CalendarItem><t:Subject>Sync</t:Subject><t:Body BodyType=\"HTML\">&lt;p&gt;Hi&lt;/p&gt;&lt;a href=\"{}\"&gt;Join the meeting now&lt;/a&gt;</t:Body>             <t:Start>2026-10-20T13:00:00Z</t:Start><t:End>2026-10-20T13:30:00Z</t:End><t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>sync-1</t:UID></t:CalendarItem>",
        long.replace('&', "&amp;amp;")
    ));
    d.pim.push(("M1".into(), agenda, teams.kids[0].clone()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    crate::ews::list_folders(&c, &mut crate::jmap::State::default(), "a1").unwrap();
    // A second sync: the first one only lists the others' calendars (see above).
    let pim = quatrelle_model::pim::Pim {
        calendars: vec![quatrelle_model::pim::CalendarInfo { id: "shared:sales@contoso.example".into(), ..Default::default() }],
        ..Default::default()
    };
    let out = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    let cals = out.calendars.clone().unwrap();
    let names: Vec<(&str, &str)> = cals.iter().map(|c| (c.id.as_str(), c.name.as_str())).collect();
    assert_eq!(names, [(crate::ews_pim::CALENDAR, "Agenda"), ("shared:sales@contoso.example", "Sales")], "{names:?}");
    assert!(out.events.iter().any(|e| e.calendar == "shared:sales@contoso.example" && e.event.summary == "Team planning"));
    assert!(out.errors.is_empty(), "the boss's calendar is passed over quietly: {:?}", out.errors);
    assert_eq!(out.books.as_ref().unwrap()[0].name, "Contactpersonen");
    let sync = out.events.iter().find(|e| e.event.summary == "Sync").unwrap();
    assert_eq!(sync.event.join_link().as_deref(), Some(long.as_str()), "the whole join link");
}

/// Shared mailboxes (issue #1): those the server maps to the account open by themselves, one
/// named by its Exchange address only is looked up, one added by hand opens too; each under
/// a folder of its own, routed to it, its Outbox left out, its Inbox not the account's. One
/// without access, and the account's online archive, don't.
#[test]
fn shared_mailboxes_open_by_themselves_or_by_hand() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Sales", None, None, "IPF.Note");
    d.deliver(&inbox, &mail("Own", "o1"), false, "2026-10-19T08:00:00Z");
    let box_ = |address: &str, name: &str, automap: bool, legacy: bool, granted: bool| SharedBox {
        address: address.into(),
        name: name.into(),
        automap,
        legacy,
        granted,
    };
    let sales = d.shared_mailbox(box_("sales@contoso.example", "Sales", true, false, true));
    let quotes = d.shared_folder("sales@contoso.example", "Quotes", &sales);
    d.deliver(&sales, &mail("Order 17", "s1"), false, "2026-10-19T09:00:00Z");
    d.deliver(&quotes, &mail("Quote 4", "s2"), true, "2026-10-19T09:30:00Z");
    d.shared_mailbox(box_("support@contoso.example", "Support", true, true, true));
    d.shared_mailbox(box_("boss@contoso.example", "Boss", true, false, false));
    let team = d.shared_mailbox(box_("team@contoso.example", "Team", false, false, true));
    d.deliver(&team, &mail("Standup", "t1"), false, "2026-10-19T10:00:00Z");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    c.set_shared(&["team@contoso.example".to_string(), "riley@contoso.example".to_string()]);
    let mut st = crate::jmap::State::default();
    let folders = crate::ews::list_folders(&c, &mut st, "a1").unwrap();
    let paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
    // Sales collides with the account's own folder: named with its address.
    let sales_top = "Sales (sales@contoso.example)";
    for want in ["INBOX", "Sales", sales_top, &format!("{sales_top}/Inbox"), &format!("{sales_top}/Inbox/Quotes"), "Support/Inbox", "Team/Inbox"] {
        assert!(paths.contains(&want), "{want} in {paths:?}");
    }
    assert!(!paths.iter().any(|p| p.contains("Boss") || p.contains("Archive") || p.ends_with("Outbox")), "{paths:?}");
    assert!(!paths.iter().any(|p| p.contains("riley@")), "the account's own mailbox isn't a shared one");
    assert_eq!(folders.iter().filter(|f| f.role == quatrelle_model::FolderRole::Inbox).count(), 1, "only the account's own Inbox is its Inbox");
    // Each shared mailbox is a mailbox of its own (issue #1: it showed as one of the
    // account's folders): its folders say whose they are, and what each is there.
    let folder = |path: &str| folders.iter().find(|f| f.path == path).unwrap();
    let top = folder(sales_top);
    assert_eq!((top.mailbox.as_deref(), top.depth()), (Some("sales@contoso.example"), 0));
    let shared_inbox = folder(&format!("{sales_top}/Inbox"));
    assert_eq!(shared_inbox.mailbox.as_deref(), Some("sales@contoso.example"));
    assert_eq!((shared_inbox.role, shared_inbox.mailbox_role), (quatrelle_model::FolderRole::Other, quatrelle_model::FolderRole::Inbox));
    assert!(shared_inbox.is_fixed() && top.is_fixed(), "a shared mailbox's top folder and Inbox can't be renamed or deleted");
    let quotes = folder(&format!("{sales_top}/Inbox/Quotes"));
    assert_eq!((quotes.mailbox.as_deref(), quotes.mailbox_role), (Some("sales@contoso.example"), quatrelle_model::FolderRole::Other));
    assert!(!quotes.is_fixed());
    assert_eq!(folder("Team/Inbox").mailbox.as_deref(), Some("team@contoso.example"));
    assert!(folder("INBOX").mailbox.is_none() && folder("Sales").mailbox.is_none(), "the account's own folders are its own");
    // Shown after the account's own folders, each mailbox together, its Inbox first.
    let mut sorted = folders.clone();
    quatrelle_model::sort_folders(&mut sorted);
    let order: Vec<&str> = sorted.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        order,
        [
            "INBOX",
            "Sales",
            sales_top,
            &format!("{sales_top}/Inbox"),
            &format!("{sales_top}/Inbox/Quotes"),
            "Support",
            "Support/Inbox",
            "Team",
            "Team/Inbox"
        ],
    );
    // Their mail syncs as any folder's.
    let store = crate::fake::tests_store("shared-mailboxes");
    let r = crate::ews::sync_folder(&c, &mut st, &store, "a1", &format!("{sales_top}/Inbox"), &crate::sync::SyncOptions::default()).unwrap();
    assert_eq!(r.new, 1);
    assert_eq!(store.load_folder("a1", &format!("{sales_top}/Inbox")).unwrap().messages[0].subject, "Order 17");
    crate::ews::sync_folder(&c, &mut st, &store, "a1", "Team/Inbox", &crate::sync::SyncOptions::default()).unwrap();
    assert_eq!(store.load_folder("a1", "Team/Inbox").unwrap().messages[0].subject, "Standup");
    let anchors = server.data.lock().unwrap().anchors.clone();
    assert!(anchors.iter().any(|a| a == "sales@contoso.example") && anchors.iter().any(|a| a == "team@contoso.example"), "{anchors:?}");
    let _ = std::fs::remove_dir_all(store.root());
}
