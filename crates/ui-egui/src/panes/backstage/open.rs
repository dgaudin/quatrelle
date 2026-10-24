//! The File page: Open & Export — Outlook data files, and the rules as a file.

use super::*;

/// Open & Export: Outlook data files, and the rules as a file.
pub(super) fn open(s: &Session, t: &Tokens, ui: &mut Ui, b: &mut Backstage, acts: &mut Vec<Action>) {
    let _ = b;
    heading(ui, t, "Open & Export");
    ui.set_max_width(760.0);
    let r = tile(
        ui,
        t,
        Icon::Folder,
        "Open data file",
        "Open a data file",
        "Bring in an Outlook data file (.pst) or a Quatrelle one (.quatrelle). It opens as a mailbox of its own, with its folders, kept on this computer.",
        |ui| {
            let files: Vec<&str> = s.accounts.iter().filter(|a| a.is_data_file()).map(|a| a.config.email.as_str()).collect();
            if !files.is_empty() {
                ui.label(RichText::new(tf!("Open now: {files}", files = files.join(", "))).color(t.faint).small());
            }
        },
    );
    if r.clicked() {
        acts.push(Action::OpenDataFile);
    }
    // Outlook's Other User's Folder: another mailbox opened in an Exchange account (a shared
    // one, or someone's who made this account a delegate), its calendar beside it.
    let exchange: Vec<&AccountState> = s.accounts.iter().filter(|a| a.config.graph.is_some() || a.config.ews.is_some()).collect();
    if let Some(first) = exchange.first() {
        let into = s
            .current_account()
            .filter(|a| a.config.graph.is_some() || a.config.ews.is_some())
            .map_or_else(|| first.config.id.clone(), |a| a.config.id.clone());
        let r = tile(
            ui,
            t,
            Icon::People,
            "Other user's folder",
            "Open another mailbox",
            "A shared mailbox, or someone's who made you a delegate: its folders open under this account, as far as Exchange lets you in.",
            |ui| {
                let open: Vec<&str> = exchange.iter().flat_map(|a| a.config.shared_mailboxes.iter().map(String::as_str)).collect();
                if !open.is_empty() {
                    ui.label(RichText::new(tf!("Open now: {files}", files = open.join(", "))).color(t.faint).small());
                }
            },
        );
        if r.clicked() {
            acts.push(Action::Dialog(Dialog::OpenSharedMailbox { account: into, email: String::new(), calendar: true, error: None }));
        }
    }
    let current = s.current_account().map(|a| a.config.id.clone());
    let r = tile(
        ui,
        t,
        Icon::Outbox,
        "Export",
        "Export to Outlook data file",
        "Write an account's mail, with its folders, to an Outlook data file (.pst) that Outlook and other mail apps can open. Mail that was never downloaded is left out.",
        |ui| {
            ui.label(RichText::new(tl!("Its calendar and contacts go beside it, as .ics and .vcf files.")).color(t.faint).small());
            ui.horizontal_wrapped(|ui| {
                for a in &s.accounts {
                    if ui.button(RichText::new(&a.config.email).small()).clicked() {
                        acts.push(Action::ExportDataFile(a.config.id.clone()));
                    }
                }
            });
        },
    );
    if r.clicked()
        && let Some(id) = current
    {
        acts.push(Action::ExportDataFile(id));
    }
    // Rules to a file and back, as Outlook's Import and Export Rules (the shared version's card,
    // drawn as the fork's tile).
    tile(
        ui,
        t,
        Icon::Inbox,
        "Rules",
        "Rules as a file",
        "Save your rules in a file, to keep them or to use them on another computer, and add the ones from a file saved by Quatrelle. A rule for an account this computer doesn't have comes in turned off.",
        |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!s.rules.is_empty(), egui::Button::new(RichText::new(tl!("Export…")).small())).clicked() {
                    acts.push(Action::ExportRules);
                }
                if ui.button(RichText::new(tl!("Import…")).small()).clicked() {
                    acts.push(Action::ImportRules);
                }
            });
        },
    );
}
