//! The dialogs' contacts fields, made ready before they draw: Add calendar's (with its address
//! book), Open another mailbox's, a rule's conditions' and the Search Folder's, numbered apart as
//! dialog window 0 ([`DIALOG_WINDOWS`]).

use quatrelle_engine::Session;

use super::{DIALOG_WINDOWS, PeopleFields, accepts_of, dialog_field};
use crate::dialogs::{Dialog, rule_field};

/// The windows the open dialog's fields belong to, kept live while it's open (`settle` lets the
/// others' fields, and an OK held for them, go).
pub(crate) fn dialog_windows(d: &Dialog) -> Vec<u64> {
    match d {
        Dialog::OpenSharedCalendar(_) | Dialog::OpenSharedMailbox { .. } => vec![DIALOG_WINDOWS],
        Dialog::EditRule(f) => f.conditions.iter().enumerate().filter_map(|(i, c)| rule_field(i, c.test)).map(|field| field.window).collect(),
        Dialog::SearchFolder(f) if f.needs().0 => vec![DIALOG_WINDOWS],
        _ => Vec::new(),
    }
}

/// The open dialog's field made ready (its text parsed when it changed, the names the engine
/// settled taken, suggestions for what's typed), and Add calendar's address book listed. Each
/// frame before the dialog draws, and before a held OK is looked at (`send_when_resolved`), so a
/// name the directory has just answered is taken before the wait concludes.
pub(crate) fn prepare(people: &mut PeopleFields, s: &mut Session, d: &mut Dialog) {
    match d {
        Dialog::OpenSharedCalendar(f) => {
            f.search.refresh(s);
            let account = s.directory_account(None);
            people.prepare(s, dialog_field(0, "calendar"), account.as_deref(), accepts_of("calendar"), &f.text);
        }
        Dialog::OpenSharedMailbox { account, email, .. } => {
            people.prepare(s, dialog_field(0, "mailbox"), Some(account), accepts_of("mailbox"), email);
        }
        Dialog::EditRule(f) => {
            let account = s.directory_account(f.account.as_deref());
            for (i, c) in f.conditions.iter().enumerate() {
                if let Some(field) = rule_field(i, c.test) {
                    people.prepare(s, field, account.as_deref(), accepts_of(field.name), &c.text);
                }
            }
        }
        Dialog::SearchFolder(f) if f.needs().0 => {
            let account = s.directory_account(None);
            people.prepare(s, dialog_field(0, "search-people"), account.as_deref(), accepts_of("search-people"), &f.people);
        }
        _ => {}
    }
}
