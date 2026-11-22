// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A dialog's OK that its command refuses.

use super::*;

/// A dialog's OK refused by the command is said, even by a dialog with no error line of its
/// own (Select Names, Ctrl+Enter's question), and with no dialog open at all.
#[test]
fn a_refused_dialog_submit_is_said() {
    let (mut a, dir) = app("names-refused");
    let ctx = egui::Context::default();
    a.session.prefs.new_mail_window = false;
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.clone().unwrap();
    a.ui.dialog = Some(Dialog::select_names(&c, "to"));
    frame(&ctx, &mut a, vec![]);
    // The message went to be sent while the names were being picked: OK can't change it.
    if let Some(c) = a.session.compose.as_mut() {
        c.sending = true;
    }
    let before = a.session.notices.len();
    a.act(crate::Action::Submit("compose.update", json!({"to": "zoe@studio.example", "cc": "", "bcc": ""})));
    assert!(matches!(a.ui.dialog, Some(Dialog::SelectNames(_))), "the dialog stays, to be put right or cancelled");
    let said = a.session.notices.get(before).unwrap_or_else(|| panic!("nothing said: {:?}", a.session.notices));
    assert!(said.error && said.text.contains("being sent"), "{said:?}");
    // A question with no error line says it the same way.
    a.ui.dialog = Some(Dialog::CtrlEnter { remember: false });
    a.act(crate::Action::Submit("compose.update", json!({"to": "zoe@studio.example", "cc": "", "bcc": ""})));
    assert_eq!(a.session.notices.len(), before + 2, "{:?}", a.session.notices);
    assert!(a.session.notices.last().is_some_and(|n| n.error && n.text.contains("being sent")));
    // No dialog open: said all the same.
    a.ui.dialog = None;
    a.act(crate::Action::Submit("rules.set", json!({"rules": "not a list"})));
    assert_eq!(a.session.notices.len(), before + 3, "{:?}", a.session.notices);
    assert!(a.session.notices.last().is_some_and(|n| n.error));
    // A dialog with its own error line keeps showing it there, and only there.
    a.ui.dialog = Some(Dialog::rule_from("zoe@studio.example", None, false, None));
    a.act(crate::Action::Submit("rules.set", json!({"rules": "not a list"})));
    assert_eq!(a.session.notices.len(), before + 3, "{:?}", a.session.notices);
    let Some(Dialog::EditRule(f)) = &a.ui.dialog else { panic!("no dialog") };
    assert!(f.error.as_ref().is_some_and(|e| !e.starts_with("bad parameters")), "{:?}", f.error);
    let _ = std::fs::remove_dir_all(dir);
}
