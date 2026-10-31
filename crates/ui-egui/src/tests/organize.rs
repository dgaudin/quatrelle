//! UI tests: rules, junk lists, Search Folders and Quick Steps.

use quatrelle_engine::FieldId;
use quatrelle_engine::model::organize::{Condition, Rule, RuleAction};

use super::*;
use crate::dialogs::people_field::{dialog_field, egui_id};
use crate::dialogs::{RuleCond, RuleTest, rule_field};
use crate::look::people_field::{Piece, input_id};

#[test]
fn organising_renders_and_rules_save() {
    let (mut a, dir) = app("organise");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[1].key.clone();
    a.run("list.select", json!({"key": k.to_id()}));
    a.run("mail.categorize", json!({"category": "Work"}));
    a.run("mail.pin", json!({}));
    a.run("view.focused", json!({"on": true}));
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    a.run("list.focusTab", json!({"value": "other"}));
    frame(&ctx, &mut a, vec![]);
    // The rule editor saves a rule; the Rules dialog lists it.
    a.ui.dialog = Some(Dialog::rule_from("zoe@studio.example", None, false, None));
    frame(&ctx, &mut a, vec![]);
    if let Some(Dialog::EditRule(f)) = a.ui.dialog.as_mut() {
        f.actions = vec![crate::dialogs::RuleAct { action: crate::dialogs::RuleDo::Categorize, target: "Work".into() }];
    }
    a.act(crate::Action::Submit(
        "rules.set",
        json!({"rules": [{"name": "Zoe", "conditions": [{"if": "from", "value": "zoe@"}], "actions": [{"do": "categorize", "value": "Work"}]}]}),
    ));
    assert_eq!(a.session.rules.len(), 1);
    // One the server refused: the list says so, and the editor says why.
    a.session.rules[0].server = true;
    a.session.rules[0].account = Some("sample".into());
    a.session.rules[0].refused = Some("the server refused: Outlook keeps its own copy of the rules".into());
    let edit = crate::dialogs::RuleForm::from_rule(0, &a.session.rules[0]);
    assert!(edit.refused.is_some());
    a.ui.dialog = Some(Dialog::EditRule(Box::new(edit.clone())));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(edit.to_rule().unwrap().refused, None, "saved, it goes to the server again");
    // A rule the server keeps that Quatrelle didn't make (Outlook's own): listed, read-only.
    a.session.foreign_rules.push(quatrelle_engine::model::organize::ForeignRule {
        account: "sample".into(),
        id: "R9".into(),
        name: "Outlook rule".into(),
        enabled: true,
        runnable: false,
        rule: None,
        summary: "subject \"x\" → delete".into(),
    });
    a.ui.dialog = Some(Dialog::Rules);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A rule whose name and description are longer than its line is cut at its buttons, not drawn
/// under them and out of the dialog (one with a long sender ran 350 points past it, 29/10).
#[test]
fn a_long_rule_stays_inside_the_rules_dialog() {
    let (mut a, dir) = app("rule-long");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let name = "New message — from: postmaster@a-rather-long-mail-domain-name.example";
    a.act(crate::Action::Submit(
        "rules.set",
        json!({"rules": [{"name": name, "conditions": [{"if": "from", "value": "postmaster@a-rather-long-mail-domain-name.example"}], "actions": [{"do": "delete"}]}]}),
    ));
    // And one of the server's, made in Outlook, as long.
    a.session.foreign_rules.push(quatrelle_engine::model::organize::ForeignRule {
        account: "sample".into(),
        id: "R9".into(),
        name: "Outlook rule: everything from the monitoring robots of the datacentre to its folder, then a copy to the team's archive, flagged"
            .into(),
        enabled: true,
        runnable: false,
        rule: None,
        summary: "from robots → move".into(),
    });
    a.ui.dialog = Some(Dialog::Rules);
    let size = egui::vec2(1280.0, 800.0);
    let title = settled_text_at(&ctx, &mut a, "Rules", size).expect("the dialog's title");
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    let ends = |start: &str| {
        shapes
            .iter()
            .find_map(|s| match s {
                egui::Shape::Text(t) if t.galley.text().starts_with(start) => Some(t.pos.x + t.galley.rect.max.x),
                _ => None,
            })
            .unwrap_or_else(|| panic!("the line of {start}"))
    };
    // The dialog's content is 560 points wide, from about where its title starts.
    for start in ["New message", "Outlook rule"] {
        let line = ends(start);
        assert!(line < title.x + 560.0, "{start}: the line ends at {line}, the dialog's content at about {}", title.x + 560.0);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Create rule… on a message in another folder: the form offers to run the rule on that folder's
/// mail (the Inbox alone seemed to do nothing), and saving runs it there; from the Inbox, or
/// with no folder, on the Inbox.
#[test]
fn a_rule_made_from_a_folder_runs_there() {
    let (mut a, dir) = app("rule-folder");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Some(Dialog::rule_from("zoe@studio.example", Some("sample"), false, Some("Archive")));
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::EditRule(mut f)) = a.ui.dialog.clone() else { panic!("the rule editor is open") };
    assert_eq!(f.run_in.as_deref(), Some("Archive"));
    assert_eq!(f.run_now_folder(&a.session).as_deref(), Some("Archive"), "named as the folder pane names it");
    f.actions = vec![crate::dialogs::RuleAct { action: crate::dialogs::RuleDo::Flag, target: String::new() }];
    let acts = f.save_actions(&a.session).unwrap();
    assert!(matches!(&acts[0], crate::Action::Submit("rules.set", _)));
    assert!(matches!(&acts[1], crate::Action::Run("rules.run", p) if p["index"] == 0 && p["account"] == "sample" && p["folder"] == "Archive"));
    for act in acts {
        a.act(act);
    }
    assert_eq!(a.session.rules.len(), 1);
    assert!(a.session.notices.last().is_some_and(|n| n.text.contains("matched")), "ran on Archive's mail");
    // From the Inbox itself: the Inbox wording, and the Inbox run.
    let inbox = rule_form(Dialog::rule_from("zoe@studio.example", Some("sample"), false, Some("INBOX")));
    assert_eq!(inbox.run_now_folder(&a.session), None);
    // A folder that's gone: the Inbox. No account: no folder to run in.
    let mut gone = rule_form(Dialog::rule_from("zoe@studio.example", Some("sample"), false, Some("Gone")));
    gone.actions = vec![crate::dialogs::RuleAct { action: crate::dialogs::RuleDo::Flag, target: String::new() }];
    assert!(matches!(&gone.save_actions(&a.session).unwrap()[1], crate::Action::Run("rules.run", p) if p["inbox"] == true));
    assert_eq!(rule_form(Dialog::rule_from("zoe@studio.example", None, false, Some("Archive"))).run_in, None);
    let _ = std::fs::remove_dir_all(dir);
}

fn rule_form(d: Dialog) -> crate::dialogs::RuleForm {
    match d {
        Dialog::EditRule(f) => *f,
        other => panic!("not the rule editor: {other:?}"),
    }
}

/// Junk Email: the menu blocks the sender, and the lists in Options draw, long ones in part.
#[test]
fn junk_lists_block_and_draw() {
    let (mut a, dir) = app("junk-lists");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("list.open", json!({"key": "sample:1:1:INBOX"}));
    a.act(crate::Action::Run("junk.blockSender", json!({})));
    assert!(a.session.prefs.blocked_senders.contains("zoe@studio.example"));
    for n in 0..1500 {
        a.run("junk.edit", json!({"list": if n % 2 == 0 { "blocked" } else { "safe" }, "entry": format!("@d{n}.example")}));
    }
    assert_eq!(a.session.prefs.blocked_senders.len() + a.session.prefs.safe_senders.len(), 1501);
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.backstage.is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// Search Folders: made from the dialog (a kind, or people), opened from the pane, the heading
/// its name and the search box left empty; the dialog shows the reason when refused.
#[test]
fn search_folders_are_made_from_the_dialog_and_open() {
    let (mut a, dir) = app("search-folders");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let mut f = crate::dialogs::SearchFolderForm::new();
    assert_eq!(f.final_query(), "is:unread");
    let before = f.preset;
    f.preset = 8;
    f.apply_preset(before);
    assert_eq!(f.name, "Mail from specific people", "the name follows the kind until one is typed");
    f.people = "zoe@studio.example; Ana Lima, , \"x".into();
    assert_eq!(f.final_query(), "from:zoe@studio.example,\"Ana Lima\",x");
    f.name = "Zoe and Ana".into();
    a.ui.dialog = Some(Dialog::SearchFolder(Box::new(f.clone())));
    frame(&ctx, &mut a, vec![]);
    a.ui.search_text = "typed before".into();
    a.act(crate::Action::Submit("searchFolder.create", json!({"name": f.name, "query": f.final_query()})));
    assert!(a.ui.dialog.is_none());
    assert_eq!(crate::view_title(&a.session), "Zoe and Ana");
    assert!(a.ui.search_text.is_empty());
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.search_text.is_empty(), "the box isn't filled with its query");
    assert!(!a.session.list.is_empty() && a.session.list.iter().all(|m| m.from.email.starts_with("zoe") || m.from.name.contains("Ana")));
    // The same name again: the dialog stays, with why.
    a.ui.dialog = Some(Dialog::SearchFolder(Box::new(f.clone())));
    a.act(crate::Action::Submit("searchFolder.create", json!({"name": "zoe AND ana", "query": "is:unread"})));
    assert!(matches!(&a.ui.dialog, Some(Dialog::SearchFolder(f)) if f.error.is_some()));
    // Edited: a preset's search shows as that kind, anything else as Custom.
    let edit = crate::dialogs::SearchFolderForm::edit(&a.session.prefs.search_folders[0]);
    assert_eq!((edit.preset, edit.final_query().as_str()), (8, "from:zoe@studio.example,\"Ana Lima\",x"));
    a.ui.dialog = Some(Dialog::SearchFolder(Box::new(edit)));
    frame(&ctx, &mut a, vec![]);
    // Many, with long names, draw.
    for n in 0..60 {
        a.run("searchFolder.create", json!({"name": format!("{} {n}", "Quite a long Search Folder name ".repeat(3)), "query": "is:flagged"}));
    }
    a.ui.dialog = None;
    frame(&ctx, &mut a, vec![]);
    a.run("search.run", json!({"query": "storyboard"}));
    assert!(crate::view_title(&a.session).contains("storyboard"));
    let _ = std::fs::remove_dir_all(dir);
}

/// Quick Steps: made from the dialog, done with Ctrl+Shift+1, changed and deleted from it.
#[test]
fn quick_steps_are_made_and_done_with_their_keys() {
    let (mut a, dir) = app("quick-steps");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let mut f = crate::dialogs::QuickStepForm::new(&a.session);
    assert_eq!(f.actions(), vec![json!({"do": "markRead"}), json!({"do": "moveToRole", "value": "archive"})], "Done, as Outlook's");
    f.name = "Done".into();
    f.flag = true;
    a.ui.dialog = Some(Dialog::QuickStep(Box::new(f.clone())));
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Submit("quickStep.create", json!({"name": f.name, "actions": f.actions()})));
    assert!(a.ui.dialog.is_none() && a.session.prefs.quick_steps.len() == 1);
    // Ctrl+Shift+1 on the first message: flagged, read, archived.
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    let m = a.session.selected()[0].clone();
    // As Windows sends it: Shift makes the key `!`; where it is says 1.
    let shifted = Event::Key {
        key: Key::Exclamationmark,
        physical_key: Some(Key::Num1),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND | Modifiers::SHIFT,
    };
    frame(&ctx, &mut a, vec![shifted]);
    let archive = a.session.account("sample").and_then(|x| x.folder(quatrelle_engine::model::FolderRole::Archive)).map(|x| x.path.clone()).unwrap();
    let there = a.session.folder_state("sample", &archive).messages.iter().find(|x| x.message_id == m.message_id).cloned().unwrap();
    assert!(there.flags.seen && there.flags.flagged);
    // Opened again, it shows what it does; renamed to a taken name: the reason in the dialog.
    a.run("quickStep.create", json!({"name": "Later", "actions": [{"do": "pin"}]}));
    let mut edit = crate::dialogs::QuickStepForm::edit(&a.session, &a.session.prefs.quick_steps[0]);
    assert!(edit.read && edit.flag && edit.move_to == crate::dialogs::StepMove::Role(quatrelle_engine::model::FolderRole::Archive));
    edit.name = "later".into();
    a.ui.dialog = Some(Dialog::QuickStep(Box::new(edit.clone())));
    a.act(crate::Action::Submit("quickStep.edit", json!({"name": "Done", "newName": edit.name, "actions": edit.actions()})));
    assert!(matches!(&a.ui.dialog, Some(Dialog::QuickStep(f)) if f.error.is_some()));
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Submit("quickStep.delete", json!({"name": "Done"})));
    assert!(a.ui.dialog.is_none());
    assert_eq!(a.session.prefs.quick_steps.len(), 1);
    // Nothing to do: the dialog can't be saved.
    assert!(crate::dialogs::QuickStepForm { name: "x".into(), ..Default::default() }.actions().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// Issue #4: the search box's recent searches (those with what's typed so far; none, closed)
/// and its advanced search (filled from the box, written back to it), drawn in a frame; Esc
/// closes them; where it looks named in every language's terms.
#[test]
fn the_search_box_drops_down_recent_and_advanced_search() {
    let (mut a, dir) = app("search-box");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.session.prefs.recent_searches = vec!["budget".into(), "storyboard".into()];
    a.ui.search_drop = crate::SearchDrop::Recent;
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.search_drop, crate::SearchDrop::Recent);
    a.ui.search_text = "story".into();
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.search_drop, crate::SearchDrop::Recent, "storyboard has it");
    a.ui.search_text = "zzz".into();
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.search_drop, crate::SearchDrop::None, "nothing to show: closed");

    a.ui.search_text = "budget from:ana".into();
    let form = quatrelle_engine::model::search::Advanced::from_query(&a.ui.search_text);
    assert_eq!((form.words.as_str(), form.from.as_str()), ("budget", "ana"));
    a.ui.search_drop = crate::SearchDrop::Advanced(form.clone());
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.search_drop, crate::SearchDrop::Advanced(form), "stays open, the form as it was");
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert_eq!(a.ui.search_drop, crate::SearchDrop::None);
    for scope in quatrelle_engine::SearchScope::ALL {
        assert!(!crate::scope_label(scope).is_empty());
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Outlook's 25 category colours each have one of their own here (after Quatrelle's eight), and
/// the calendars keep theirs: the first eight in turn, as before there were more.
#[test]
fn categories_have_outlooks_colours_and_calendars_keep_theirs() {
    use crate::theme::category_color;
    use quatrelle_engine::model::organize::{CATEGORY_COLORS, outlook_color};
    let all: Vec<egui::Color32> = (0..CATEGORY_COLORS).map(|i| category_color(i, false)).collect();
    let mut distinct = all.clone();
    distinct.sort_by_key(|c| c.to_array());
    distinct.dedup();
    assert_eq!(distinct.len(), usize::from(CATEGORY_COLORS));
    let mapped: Vec<u8> = (0..25).filter_map(outlook_color).collect();
    let mut once = mapped.clone();
    once.sort();
    once.dedup();
    assert_eq!(once.len(), 25, "each of Outlook's its own");
    assert_eq!((outlook_color(-1), outlook_color(25)), (None, None));
    let t = crate::theme::Tokens::light();
    for i in 1..40u8 {
        assert_eq!(crate::panes::calendar::color(i, &t), category_color((i - 1) % 8, false), "calendar colour {i}");
    }
}

/// Options › Assistant: the page asks the server for its models, and says how many it has (or
/// why there are none) under the model's box.
#[test]
fn the_assistants_options_list_the_servers_models() {
    use quatrelle_engine::model::Event as MailEvent;
    let (mut a, dir) = app("llm-models");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let url = "http://localhost:11434/v1";
    a.run("assistant.settings", json!({"url": url, "model": "llama3.1"}));
    a.session.llm_models = None;
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    let tall = egui::vec2(1280.0, 5000.0);
    let texts = |shapes: &[egui::Shape]| -> Vec<String> {
        shapes.iter().filter_map(|s| if let egui::Shape::Text(t) = s { Some(t.galley.text().to_string()) } else { None }).collect()
    };
    let shown = texts(&frame_shapes(&ctx, &mut a, vec![], tall));
    assert!(a.session.llm_models.as_ref().is_some_and(|m| m.asking && m.url == url), "the page asks for them");
    let shown = [shown, texts(&frame_shapes(&ctx, &mut a, vec![], tall))].concat();
    assert!(shown.iter().any(|t| t == "Looking for the server's models…"), "{shown:?}");
    a.session.apply(MailEvent::LlmModels { url: url.into(), models: vec!["gemma2".into(), "llama3.1".into()], error: None });
    let shown = texts(&frame_shapes(&ctx, &mut a, vec![], tall));
    assert!(shown.iter().any(|t| t == "Models on this server: 2"), "{shown:?}");
    a.run("assistant.models", json!({}));
    a.session.apply(MailEvent::LlmModels { url: url.into(), models: Vec::new(), error: Some("HTTP 404".into()) });
    let shown = texts(&frame_shapes(&ctx, &mut a, vec![], tall));
    assert!(shown.iter().any(|t| t == "The models couldn't be listed: HTTP 404"), "{shown:?}");
    assert_eq!(a.session.llm_models.as_ref().map(|m| m.names.len()), Some(2), "the list before stays to pick from");
    let _ = std::fs::remove_dir_all(dir);
}

/// Issue #11: Favorites can be hidden from the top of the folder pane (View › Favorites), not
/// only collapsed, and shown again.
#[test]
fn favorites_can_be_hidden() {
    let (mut a, dir) = app("favorites-hidden");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let headings = |a: &mut MailApp| {
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
        nodes.iter().filter(|(_, n)| n.label().is_some_and(|l| l.contains("Favorites"))).count()
    };
    headings(&mut a);
    assert!(headings(&mut a) > 0, "shown at first");
    a.run("view.favorites", json!({}));
    headings(&mut a);
    assert_eq!(headings(&mut a), 0, "hidden");
    a.run("view.favorites", json!({}));
    headings(&mut a);
    assert!(headings(&mut a) > 0, "shown again");
    let _ = std::fs::remove_dir_all(dir);
}

/// What the engine says with something filled in shows in the interface's language, the
/// wording of the error in it too: a rule Exchange refused was said in English in a French
/// window (« the server's rules weren't changed: the server refused: Outlook keeps its own
/// copy… »). A server's own words stay as they came.
#[test]
fn a_notice_shows_in_the_interfaces_language() {
    let (mut a, dir) = app("notice-fr");
    let ctx = egui::Context::default();
    a.ui.language = "fr".into();
    frame(&ctx, &mut a, vec![]);
    a.session.notice_with("the server's rules weren't changed: {error}", &[("error", "the server refused: Mailbox is full")], true);
    let said = "Les règles du serveur n’ont pas été modifiées : le serveur a refusé : Mailbox is full";
    assert!(settled_text_at(&ctx, &mut a, said, egui::vec2(1280.0, 800.0)).is_some(), "{:?}", a.ui.toast);
    // From the network, with no account to name: said whole.
    a.session.apply(MailEvent::Notice { account: String::new(), message: "export: the server closed the connection: bye".into() });
    let said = "exportation : le serveur a fermé la connexion : bye";
    assert!(settled_text_at(&ctx, &mut a, said, egui::vec2(1280.0, 800.0)).is_some(), "{:?}", a.ui.toast);
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let _ = std::fs::remove_dir_all(dir);
}

/// A window tall enough for the rule editor.
const SIZE: egui::Vec2 = egui::vec2(1280.0, 900.0);

fn frames(ctx: &egui::Context, a: &mut MailApp, n: usize) {
    for _ in 0..n {
        frame_sized(ctx, a, vec![], SIZE);
    }
}

/// The dialog `d` open over the main window, a few frames on.
fn open(ctx: &egui::Context, a: &mut MailApp, d: Dialog) {
    frames(ctx, a, 1);
    a.ui.dialog = Some(d);
    frames(ctx, a, 3);
}

/// `text` typed in the field, its input given the keyboard first; a few frames after.
fn type_in(ctx: &egui::Context, a: &mut MailApp, field: FieldId, text: &str) {
    ctx.memory_mut(|m| m.request_focus(input_id(egui_id(field))));
    frame_sized(ctx, a, vec![], SIZE);
    frame_sized(ctx, a, vec![Event::Text(text.into())], SIZE);
    frames(ctx, a, 3);
}

/// The kinds of a field's pieces, as the tests compare them: a chip's address and name, or a text.
fn shown(a: &MailApp, field: FieldId) -> Vec<String> {
    a.ui.people
        .pieces(field)
        .iter()
        .map(|p| match p {
            Piece::Chip { who, refused: None } => format!("chip {} <{}>", who.name, who.address),
            Piece::Chip { who, refused: Some(why) } => format!("refused {} ({why})", who.name),
            Piece::Text { text, error: None } => format!("text {text}"),
            Piece::Text { text, error: Some(e) } => format!("error {text} ({e:?})"),
        })
        .collect()
}

fn rule_of(a: &MailApp) -> Result<Rule, String> {
    match &a.ui.dialog {
        Some(Dialog::EditRule(f)) => f.to_rule(),
        other => panic!("not the rule editor: {other:?}"),
    }
}

/// A rule editor with `conditions`, flagging what it finds.
fn rule_with(conditions: Vec<RuleCond>) -> Dialog {
    let Dialog::EditRule(mut f) = Dialog::rule_from("", Some("sample"), false, None) else { panic!("the rule editor") };
    f.name = "People".into();
    f.conditions = conditions;
    f.actions = vec![crate::dialogs::RuleAct { action: crate::dialogs::RuleDo::Flag, target: String::new() }];
    Dialog::EditRule(f)
}

fn cond(test: RuleTest, text: &str) -> RuleCond {
    RuleCond { test, text: text.into() }
}

/// A rule's From conditions take addresses alone — `Doe, John <j@x.fr>` is `j@x.fr` — and any other
/// entry as typed (`@proginov.com` looks for a domain), with no error and no word of a name not
/// found.
#[test]
fn a_rule_matches_addresses_alone_and_text_as_typed() {
    let (mut a, dir) = app("rule-people-typed");
    let ctx = egui::Context::default();
    open(&ctx, &mut a, rule_with(vec![cond(RuleTest::FromAny, "")]));
    let f = rule_field(0, RuleTest::FromAny).unwrap();
    assert_eq!(f, dialog_field(0, "rule-any"));
    type_in(&ctx, &mut a, f, "Doe, John <j@x.fr>; @proginov.com; ");
    let p = shown(&a, f);
    assert!(p.len() == 2 && p[0].starts_with("chip ") && p[0].ends_with("<j@x.fr>") && p[1] == "text @proginov.com", "{p:?}");
    assert!(!a.ui.people.blocked(f));
    let rule = rule_of(&a).unwrap();
    assert_eq!(rule.conditions, [Condition::FromAny(vec!["j@x.fr".into(), "@proginov.com".into()])]);
    let _ = std::fs::remove_dir_all(dir);
}

/// The same in the "Mail from specific people" Search Folder.
#[test]
fn a_search_folder_matches_addresses_alone_and_text_as_typed() {
    let (mut a, dir) = app("search-people-typed");
    let ctx = egui::Context::default();
    let mut f = crate::dialogs::SearchFolderForm::new();
    f.preset = 8;
    f.name = "People".into();
    open(&ctx, &mut a, Dialog::SearchFolder(Box::new(f)));
    let field = dialog_field(0, "search-people");
    type_in(&ctx, &mut a, field, "Doe, John <j@x.fr>; @proginov.com; ");
    let p = shown(&a, field);
    assert!(p.len() == 2 && p[0].ends_with("<j@x.fr>") && p[1] == "text @proginov.com", "{p:?}");
    let Some(Dialog::SearchFolder(f)) = &a.ui.dialog else { panic!("the dialog is open") };
    assert_eq!(f.final_query(), "from:j@x.fr,@proginov.com");
    let _ = std::fs::remove_dir_all(dir);
}

fn saved(conditions: Vec<Condition>) -> Rule {
    Rule {
        name: "Saved".into(),
        enabled: true,
        account: None,
        conditions,
        match_all: true,
        actions: vec![RuleAction::Flag],
        stop: false,
        server: false,
        remote_id: None,
        priority: None,
        refused: None,
    }
}

/// A saved rule reopens with an address the book knows as a chip (named by the book) and what it
/// doesn't as text, and saves the same.
#[test]
fn a_saved_rule_reopens_with_a_known_address_as_a_chip_and_text_as_text() {
    let (mut a, dir) = app("rule-people-reopen");
    a.session.contacts_step(usize::MAX);
    let ctx = egui::Context::default();
    let rule = saved(vec![Condition::FromAny(vec!["zoe@studio.example".into(), "Dupont".into()])]);
    open(&ctx, &mut a, Dialog::EditRule(Box::new(crate::dialogs::RuleForm::from_rule(0, &rule))));
    frames(&ctx, &mut a, 2);
    let f = rule_field(0, RuleTest::FromAny).unwrap();
    let p = shown(&a, f);
    assert!(p.len() == 2 && p[0].ends_with("<zoe@studio.example>") && p[1] == "text Dupont", "{p:?}");
    assert!(p[0].starts_with("chip Zoe Park "), "her name from the book: {p:?}");
    assert_eq!(rule_of(&a).unwrap().conditions, rule.conditions);
    let _ = std::fs::remove_dir_all(dir);
}

/// A saved From with a comma in it is one piece, and saves unchanged.
#[test]
fn a_saved_rule_with_a_comma_saves_unchanged() {
    let (mut a, dir) = app("rule-people-comma");
    let ctx = egui::Context::default();
    let rule = saved(vec![Condition::From("Doe, John".into())]);
    open(&ctx, &mut a, Dialog::EditRule(Box::new(crate::dialogs::RuleForm::from_rule(0, &rule))));
    frames(&ctx, &mut a, 2);
    let f = rule_field(0, RuleTest::From).unwrap();
    // One piece, written quoted as the field writes a text holding a comma.
    assert_eq!(shown(&a, f), ["text \"Doe, John\""]);
    frames(&ctx, &mut a, 2);
    assert_eq!(rule_of(&a).unwrap().conditions, rule.conditions);
    let _ = std::fs::remove_dir_all(dir);
}

/// The × buttons of the dialog, top to bottom.
fn remove_buttons(ctx: &egui::Context, a: &mut MailApp) -> Vec<egui::Pos2> {
    ctx.enable_accesskit();
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    let mut at: Vec<egui::Pos2> = nodes
        .iter()
        .filter(|(_, n)| n.label() == Some("Remove"))
        .filter_map(|(_, n)| n.bounds())
        .map(|r| egui::pos2(((r.x0 + r.x1) / 2.0) as f32, ((r.y0 + r.y1) / 2.0) as f32))
        .collect();
    at.sort_by(|p, q| p.y.total_cmp(&q.y));
    at
}

/// Taking a condition out keeps each field its own: the ones after it are made again from their
/// text, and the rule is what the fields show.
#[test]
fn removing_a_rule_condition_keeps_each_field_its_own() {
    let (mut a, dir) = app("rule-people-remove");
    let ctx = egui::Context::default();
    let three = ["a@x.fr", "b@x.fr", "c@x.fr"].map(|m| cond(RuleTest::FromAny, m)).to_vec();
    open(&ctx, &mut a, rule_with(three));
    frames(&ctx, &mut a, 2);
    let field = |i| rule_field(i, RuleTest::FromAny).unwrap();
    assert_eq!(shown(&a, field(1)).len(), 1);
    let buttons = remove_buttons(&ctx, &mut a);
    assert!(buttons.len() >= 3, "{buttons:?}");
    click_at(&ctx, &mut a, buttons[1], 1, SIZE);
    frames(&ctx, &mut a, 2);
    let (p0, p1) = (shown(&a, field(0)), shown(&a, field(1)));
    assert!(p0.len() == 1 && p0[0].ends_with("<a@x.fr>") && p1.len() == 1 && p1[0].ends_with("<c@x.fr>"), "{p0:?} {p1:?}");
    assert!(shown(&a, field(2)).is_empty(), "the third is gone");
    assert_eq!(rule_of(&a).unwrap().conditions, [Condition::FromAny(vec!["a@x.fr".into()]), Condition::FromAny(vec!["c@x.fr".into()])]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A saved "from these people" Search Folder reopens as people, as chips, and saves the same.
#[test]
fn a_people_search_folder_reopens_as_people_and_saves_the_same() {
    let (mut a, dir) = app("search-people-reopen");
    let ctx = egui::Context::default();
    let query = "from:zoe@studio.example,\"Ana Lima\",x";
    a.run("searchFolder.create", json!({"name": "People", "query": query}));
    let edit = crate::dialogs::SearchFolderForm::edit(&a.session.prefs.search_folders[0]);
    assert_eq!(edit.preset, 8);
    assert!(edit.query.is_empty());
    assert_eq!(edit.final_query(), query);
    open(&ctx, &mut a, Dialog::SearchFolder(Box::new(edit)));
    frames(&ctx, &mut a, 2);
    let p = shown(&a, dialog_field(0, "search-people"));
    assert_eq!(p.len(), 3, "{p:?}");
    let Some(Dialog::SearchFolder(f)) = &a.ui.dialog else { panic!("the dialog is open") };
    assert_eq!(f.final_query(), query);
    // A person with a comma in the name is one entry, quoted in the search and back.
    let with_comma = "from:\"Doe, John\",x@y.fr";
    a.run("searchFolder.create", json!({"name": "Comma", "query": with_comma}));
    let edit = crate::dialogs::SearchFolderForm::edit(&a.session.prefs.search_folders[1]);
    assert_eq!((edit.preset, edit.final_query().as_str()), (8, with_comma));
    let _ = std::fs::remove_dir_all(dir);
}
