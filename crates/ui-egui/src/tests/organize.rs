//! UI tests: rules, junk lists, Search Folders and Quick Steps.

use super::*;

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
    assert_eq!((edit.preset, edit.query.as_str()), (9, "from:zoe@studio.example,\"Ana Lima\",x"));
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
