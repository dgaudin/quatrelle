// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the sender check as it's shown — the reading pane's warning and External tag, the
//! message list's warning sign, the sender's address and the External tag, and File › Options ›
//! Safety's three rows.

use quatrelle_engine::model::lookalike::SenderWarning;

use super::*;

/// A message from `from` put at the top of the sample Inbox (unread), and its key.
fn deliver(a: &mut MailApp, uid: u32, from: (&str, &str), subject: &str, body: &str) -> MessageKey {
    let d = quatrelle_engine::mime::Draft { subject: subject.into(), body: body.into(), ..Default::default() };
    let raw =
        quatrelle_engine::mime::build(&d, &quatrelle_engine::model::EmailAddress::new(from.0, from.1), NOW, &format!("<senders{uid}@x>")).unwrap();
    let store = a.session.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid };
    inbox.messages.insert(0, quatrelle_engine::mime::summary_from_full(&raw, key.clone(), Default::default(), 0, NOW));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, &raw).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    key
}

/// What a screen reader is told in a frame of `size` (AccessKit on): every node's name or value.
fn spoken(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2) -> Vec<String> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
    tree.nodes.iter().filter_map(|(_, n)| n.label().or(n.value()).map(str::to_string)).collect()
}

/// Frames until the address book has read the mailbox (the sender check is gathered again then).
fn settle(ctx: &egui::Context, a: &mut MailApp) {
    for _ in 0..200 {
        frame(ctx, a, vec![]);
        if !a.session.contacts_pending() {
            break;
        }
    }
    frame(ctx, a, vec![]);
}

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// The reading pane says each case in its own words, above the message: Zoe Park's name from a
/// Gmail address (a contact's name on someone else's address) as the check finds it, then each of
/// the four as the engine could hand them up, with the same advice under each.
#[test]
fn each_banner_says_its_case() {
    let (mut a, dir) = app("banner-cases");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let key = deliver(&mut a, 9800, ("Zoe Park", "zoe.park.director@gmail.com"), "Quick favour", "Are you at your desk?");
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.run("list.open", json!({"key": key.to_id()}));
    settle(&ctx, &mut a);
    let shown = spoken(&ctx, &mut a, SIZE);
    assert!(
        shown.iter().any(|l| l == "Careful: you know Zoe Park as zoe@studio.example, but this comes from zoe.park.director@gmail.com."),
        "{shown:?}"
    );
    let cases = [
        (
            SenderWarning::Lookalike { domain: "studio-example.com".into(), like: "studio.example".into() },
            "Careful: this comes from studio-example.com, which looks like studio.example but isn't it.",
        ),
        (
            SenderWarning::NameShowsOther { shown: "zoe@studio.example".into(), real: "billing@pay.example".into() },
            "Careful: the sender's name shows zoe@studio.example, but this comes from billing@pay.example.",
        ),
        (
            SenderWarning::NameOfKnownPerson { name: "Zoe Park".into(), known: "zoe@studio.example".into(), real: "zoe.park@outlook.com".into() },
            "Careful: you know Zoe Park as zoe@studio.example, but this comes from zoe.park@outlook.com.",
        ),
        (
            SenderWarning::SignatureShowsOther { shown: "zoe@studio.example".into(), real: "zoe.park@outlook.com".into() },
            "Careful: the signature gives zoe@studio.example, but this comes from zoe.park@outlook.com.",
        ),
    ];
    for (w, says) in cases {
        if let Some(r) = a.session.reading.as_mut() {
            r.warning = Some(w);
        }
        let shown = spoken(&ctx, &mut a, SIZE);
        assert!(shown.iter().any(|l| l == says), "{says}: {shown:?}");
        assert!(shown.iter().any(|l| l.starts_with("Don't open its links or attachments")), "{shown:?}");
    }
    // In French, as the spec words it.
    a.ui.language = "fr".into();
    frame(&ctx, &mut a, vec![]);
    let shown = spoken(&ctx, &mut a, SIZE);
    assert!(
        shown.iter().any(|l| l == "Attention : la signature donne zoe@studio.example, mais ceci vient de l'adresse zoe.park@outlook.com."),
        "{shown:?}"
    );
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let _ = std::fs::remove_dir_all(dir);
}

/// A row whose sender doesn't add up carries a warning sign, which a screen reader hears as
/// "Check who sent it" (« Vérifiez qui l'a envoyé ») with the row; Zoe's own mail doesn't. With the
/// tag on, the stranger's row is External, Zoe's (the organisation's) isn't, and the reading pane
/// shows the tag beside the sender's name.
#[test]
fn a_warned_row_has_its_sign() {
    let (mut a, dir) = app("warned-row");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let key = deliver(&mut a, 9801, ("Zoe Park", "zoe.park.director@gmail.com"), "Quick favour", "Are you at your desk?");
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.run("list.open", json!({"key": key.to_id()}));
    settle(&ctx, &mut a);
    // A row is read as "who, subject, when, …" (the reading pane's heading is the subject alone).
    let row = |shown: &[String], subject: &str| shown.iter().find(|l| l.contains(&format!(", {subject}, "))).cloned().unwrap_or_default();
    let own_subject =
        a.session.list.iter().find(|m| m.from.email == "zoe@studio.example").map(|m| m.subject.clone()).expect("a message from Zoe in the sample");
    let shown = spoken(&ctx, &mut a, SIZE);
    let warned = row(&shown, "Quick favour");
    assert!(warned.contains("Check who sent it"), "{warned}");
    assert!(!warned.contains("External") && !warned.contains('<'), "both off at first: {warned}");
    let own = row(&shown, &own_subject);
    assert!(!own.is_empty() && !own.contains("Check who sent it"), "{own}");
    // The tag on: outside the organisation (studio.example) is External, in the list and above
    // the message; the address shown always.
    a.run("view.externalTag", json!({"on": true}));
    a.run("view.senderAddresses", json!({"show": "always"}));
    frame(&ctx, &mut a, vec![]);
    let shown = spoken(&ctx, &mut a, SIZE);
    let warned = row(&shown, "Quick favour");
    assert!(warned.contains("Zoe Park <zoe.park.director@gmail.com>") && warned.contains("External"), "{warned}");
    let own = row(&shown, &own_subject);
    assert!(!own.contains("External"), "{own}");
    assert!(shown.iter().any(|l| l == "External"), "the tag above the message: {shown:?}");
    // In French.
    a.ui.language = "fr".into();
    frame(&ctx, &mut a, vec![]);
    let shown = spoken(&ctx, &mut a, SIZE);
    let warned = row(&shown, "Quick favour");
    assert!(warned.contains("Vérifiez qui l'a envoyé") && warned.contains("Externe"), "{warned}");
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let _ = std::fs::remove_dir_all(dir);
}

/// The table (the wide list, its columns) says the same in its From column: the warning sign,
/// heard with the row; with the options on, the address after the name and the External tag.
#[test]
fn a_warned_row_in_the_table_has_its_sign() {
    let (mut a, dir) = app("warned-table");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.run("view.listLayout", json!({"value": "table"}));
    frame(&ctx, &mut a, vec![]);
    let key = deliver(&mut a, 9803, ("Zoe Park", "zoe.park.director@gmail.com"), "Quick favour", "Are you at your desk?");
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.run("list.open", json!({"key": key.to_id()}));
    settle(&ctx, &mut a);
    assert!(a.session.list_as_table());
    let row = |shown: &[String]| shown.iter().find(|l| l.contains(", Quick favour, ")).cloned().unwrap_or_default();
    let warned = row(&spoken(&ctx, &mut a, SIZE));
    assert!(warned.contains("Check who sent it") && !warned.contains('<') && !warned.contains("External"), "{warned}");
    a.run("view.externalTag", json!({"on": true}));
    a.run("view.senderAddresses", json!({"show": "always"}));
    frame(&ctx, &mut a, vec![]);
    let warned = row(&spoken(&ctx, &mut a, SIZE));
    assert!(warned.contains("Zoe Park <zoe.park.director@gmail.com>") && warned.contains("External"), "{warned}");
    let drawn = |ctx: &egui::Context, a: &mut MailApp| -> Vec<String> {
        frame_shapes(ctx, a, vec![], SIZE)
            .into_iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect()
    };
    // Beside the reading pane the From column is narrow: the name keeps its share (it starts
    // "Zo"), the address is cut, and the tag, with no room left for it, isn't drawn in the row.
    let narrow = drawn(&ctx, &mut a);
    assert!(narrow.iter().any(|t| t.starts_with("Zo") && !t.contains('<')), "the name drawn: {narrow:?}");
    assert!(narrow.iter().any(|t| t.starts_with('<')), "the address drawn: {narrow:?}");
    assert_eq!(narrow.iter().filter(|t| *t == "External").count(), 1, "the tag above the message only: {narrow:?}");
    // The reading pane below and the From column dragged wider: room for the name, the address
    // and the tag.
    a.run("view.readingPane", json!({"value": "bottom"}));
    a.ui.list_columns.who = Some(420.0);
    frame(&ctx, &mut a, vec![]);
    let wide = drawn(&ctx, &mut a);
    assert!(wide.iter().any(|t| t == "<zoe.park.director@gmail.com>" || t == "<…@gmail.com>"), "the address drawn: {wide:?}");
    assert!(wide.iter().filter(|t| *t == "External").count() >= 2, "the tag in the row and above the message: {wide:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The warning sign is in the list from the start, nothing opened: the window starts reading the
/// address book (a slice a frame) and the check is gathered once it's read.
#[test]
fn the_sign_shows_before_any_message_is_opened() {
    let (mut a, dir) = app("warned-unopened");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    deliver(&mut a, 9804, ("Zoe Park", "zoe.park.director@gmail.com"), "Quick favour", "Are you at your desk?");
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    settle(&ctx, &mut a);
    assert!(a.session.reading.as_ref().is_none_or(|r| r.parsed.is_none()), "nothing opened");
    let shown = spoken(&ctx, &mut a, SIZE);
    let warned = shown.iter().find(|l| l.contains(", Quick favour, ")).cloned().unwrap_or_default();
    assert!(warned.contains("Check who sent it"), "{warned}");
    let _ = std::fs::remove_dir_all(dir);
}

/// With the sender's address shown in a short line (All Inboxes names each message's folder,
/// the External tag on), the address is cut to its domain and the name keeps its share: at least
/// nine twentieths of the line (the address, laid out first, took up to seven tenths of it and cut
/// the name to a few letters).
#[test]
fn the_address_is_cut_to_its_domain_and_the_name_keeps_its_share() {
    let (mut a, dir) = app("narrow-sender");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let name = "Alexandra Whitfield";
    let key = deliver(&mut a, 9802, (name, "alexandra.whitfield.accounts@example-billing.com"), "Invoice", "Attached.");
    a.run("folder.unified", json!({"role": "inbox"}));
    a.run("view.senderAddresses", json!({"show": "always"}));
    a.run("view.externalTag", json!({"on": true}));
    a.run("list.open", json!({"key": key.to_id()}));
    settle(&ctx, &mut a);
    let size = egui::vec2(700.0, 700.0);
    let mut drawn: Vec<(String, f32)> = Vec::new();
    for _ in 0..2 {
        drawn = frame_shapes(&ctx, &mut a, vec![], size)
            .into_iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) => Some((t.galley.job.text.clone(), t.galley.size().x)),
                _ => None,
            })
            .collect();
    }
    let width = |text: &str| drawn.iter().find(|(t, _)| t == text).map(|(_, w)| *w);
    let address = width("<…@example-billing.com>").unwrap_or_else(|| panic!("the address cut to its domain: {drawn:?}"));
    let named = width(name).unwrap_or_else(|| panic!("the name: {drawn:?}"));
    // Unread, the name is bold.
    let full = ctx.fonts_mut(|f| f.layout_no_wrap(name.into(), crate::theme::bold(14.0), egui::Color32::WHITE).size().x);
    assert!(named >= full * 0.6, "the name drawn {named} wide of {full}");
    // Name, 6 points, address: no more than the room the line had for them.
    let room = named + 6.0 + address;
    assert!(named + 0.5 >= full.min(room * 0.45), "the name {named} of {room}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The External tag stays one line in a pane too narrow for it (wrapped, its word stood in a
/// column a letter wide beside the sender's name), in English and Arabic.
#[test]
fn external_stays_on_one_line_in_a_narrow_pane() {
    let ctx = egui::Context::default();
    let t = crate::theme::Tokens::light();
    for lang in ["en", "ar", "fr"] {
        crate::i18n::set_current(crate::i18n::Lang::from_code(lang).unwrap());
        let mut tag = egui::Rect::NOTHING;
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            ui.allocate_ui(egui::vec2(12.0, 200.0), |ui| tag = crate::reading::external_tag(ui, &t).rect);
        });
        out.textures_delta.clear();
        assert!(tag.height() < 22.0, "{lang}: {tag:?}");
        assert!(tag.width() > 30.0, "{lang}: {tag:?}");
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

/// File › Options › Safety, in French as the spec words it: the sender addresses' choice and the
/// External switch; the organisation's domains always (Ruling 16: the signature check reads them
/// too, the switch off), its note saying what they serve and the field which they are when it's
/// left empty. Choosing « Personnes que vous ne connaissez pas » runs
/// `view.senderAddresses` with `strangers`.
#[test]
fn the_safety_rows() {
    let (mut a, dir) = app("safety-rows");
    let ctx = egui::Context::default();
    a.ui.language = "fr".into();
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    let size = egui::vec2(1100.0, 3800.0);
    let drawn = |ctx: &egui::Context, a: &mut MailApp| -> Vec<String> {
        frame_shapes(ctx, a, vec![], size)
            .into_iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect()
    };
    settled_text_at(&ctx, &mut a, "Adresses des expéditeurs dans la liste des messages", size).expect("the addresses' row");
    let texts = drawn(&ctx, &mut a);
    for want in ["Jamais", "Personnes que vous ne connaissez pas", "Toujours", "Signaler le courrier venant de l'extérieur de votre organisation"] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    assert!(texts.iter().any(|t| t == "Domaines de votre organisation"), "the domains with the tag off too: {texts:?}");
    let serves = "Sert à l'étiquette Externe et à la vérification de la signature";
    assert!(texts.iter().any(|t| t.starts_with(serves)), "what they serve: {texts:?}");
    assert_eq!(a.session.prefs.sender_addresses, quatrelle_engine::SenderAddresses::Never, "never at first");
    let strangers = settled_text_at(&ctx, &mut a, "Personnes que vous ne connaissez pas", size).expect("its choice");
    click_at(&ctx, &mut a, strangers, 1, size);
    frame_sized(&ctx, &mut a, vec![], size);
    assert_eq!(a.session.prefs.sender_addresses, quatrelle_engine::SenderAddresses::Strangers);
    a.run("view.externalTag", json!({"on": true}));
    settled_text_at(&ctx, &mut a, "Domaines de votre organisation", size).expect("the domains' row, with the tag on too");
    let texts = drawn(&ctx, &mut a);
    assert!(texts.iter().any(|t| t == "studio.example"), "left empty, the field says the account's domain: {texts:?}");
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let _ = std::fs::remove_dir_all(dir);
}
