//! The File page: Info — the account's settings, its automatic replies, the space it uses and
//! its rules.

use super::*;

pub(super) fn info(s: &Session, t: &Tokens, has_files: bool, ui: &mut Ui, b: &mut Backstage, acts: &mut Vec<Action>) {
    heading(ui, t, "Account information");
    ui.set_max_width(760.0);
    let Some(a) = b.account.as_deref().and_then(|id| s.account(id)).or_else(|| s.accounts.first()) else {
        ui.label(RichText::new(tl!("Add an email account")).color(t.dim));
        if ui.button(tl!("Add account…")).clicked() {
            acts.push(Action::Dialog(Dialog::add_account()));
        }
        return;
    };
    let id = a.config.id.clone();
    // The account picker.
    ui.horizontal(|ui| {
        let current = format!("{}   ·   {}", a.config.email, tl!(kind(a)));
        egui::ComboBox::from_id_salt("backstage-account").width(420.0).selected_text(current).show_ui(ui, |ui| {
            for x in &s.accounts {
                if ui.selectable_label(x.config.id == id, format!("{}   ·   {}", x.config.email, tl!(kind(x)))).clicked() {
                    b.account = Some(x.config.id.clone());
                }
            }
        });
        if widgets::pill_button(ui, Icon::Plus, "Add account…", t).clicked() {
            acts.push(Action::Dialog(Dialog::add_account()));
        }
    });
    ui.add_space(16.0);
    if !a.is_local() && b.asked.insert(id.clone()) {
        acts.push(Action::Run("account.vacation", json!({"account": id})));
    }

    // Account settings: where it connects, and every change to it, each with its own button.
    let servers = if a.is_data_file() {
        tl!("Opened from an Outlook data file, kept on this computer only").to_string()
    } else if a.is_local() {
        tl!("Sample mailbox, kept on this computer only").to_string()
    } else if let Some(u) = &a.config.graph {
        tf!("Server: {url} (Microsoft Graph)", url = u)
    } else if let Some(u) = &a.config.jmap {
        tf!("Server: {url} (JMAP)", url = u)
    } else if let Some(u) = &a.config.ews {
        tf!("Server: {url} (EWS)", url = u)
    } else if let Some(p) = &a.config.pop {
        tf!(
            "Servers: {pop} (POP {pop_port}), {smtp} (SMTP {smtp_port})",
            pop = p.host,
            pop_port = p.port,
            smtp = a.config.smtp.host,
            smtp_port = a.config.smtp.port
        )
    } else {
        tf!(
            "Servers: {imap} (IMAP {imap_port}), {smtp} (SMTP {smtp_port})",
            imap = a.config.imap.host,
            imap_port = a.config.imap.port,
            smtp = a.config.smtp.host,
            smtp_port = a.config.smtp.port
        )
    };
    card(ui, t, tl!("Account settings"), Some(tl!("Change this account's name, servers and sign-in, or its signature.")), |ui| {
        setting_row(ui, t, true, name(t, &a.config.email), Some(note(t, &servers)), |ui| {
            if !a.is_local() && ui.button(tl!(if a.config.oauth.is_some() { "Sign in again…" } else { "Password…" })).clicked() {
                acts.push(Action::SignInAgain(id.clone()));
            }
            if !a.is_local() && a.config.graph.is_none() && ui.button(tl!("Server settings…")).clicked() {
                acts.push(Action::Dialog(Dialog::edit_account(&a.config)));
            }
        });
        setting_row(ui, t, false, name(t, tl!("Signature")), None, |ui| {
            if ui.button(tl!("Signature…")).clicked() {
                acts.push(Action::EditSignature { account: id.clone(), insert: false });
            }
        });
        setting_row(ui, t, false, name(t, tl!("Remove account")), None, |ui| {
            if ui.button(RichText::new(tl!("Remove account…")).color(t.danger)).clicked() {
                acts.push(Action::Dialog(Dialog::RemoveAccount { account: id.clone(), email: a.config.email.clone() }));
            }
        });
    });
    // S/MIME, for accounts that send mail.
    if !a.is_data_file() {
        super::security::security(s, t, ui, a, acts);
    }

    // Automatic replies: on or off, and a button to change them where the server has them.
    let (color, status) = match &a.vacation {
        _ if a.is_data_file() => (t.faint, tl!("Not available: a data file has no server.").to_string()),
        _ if a.is_local() => (t.faint, tl!("Not available: the sample mailbox has no server.").to_string()),
        VacationState::Unknown | VacationState::Loading => (t.faint, tl!("Checking with the server…").to_string()),
        VacationState::Loaded(v) if v.enabled => (
            t.ok,
            match (&v.from, &v.until) {
                (Some(f), Some(u)) => tf!("On from {from} until {until}", from = f, until = u),
                (Some(f), None) => tf!("On from {from}", from = f),
                (None, Some(u)) => tf!("On until {until}", until = u),
                (None, None) => tl!("On").to_string(),
            },
        ),
        VacationState::Loaded(_) => (t.dim, tl!("Off").to_string()),
        VacationState::Failed(e) => (t.warn, tf!("Not available: {error}", error = tl!(e))),
    };
    let can_change = !a.is_local() && !matches!(a.vacation, VacationState::Failed(_));
    card(
        ui,
        t,
        tl!("Automatic replies"),
        Some(tl!("Tell people you're away: each sender gets one reply while they're on, even when this computer is off.")),
        |ui| {
            setting_row(ui, t, true, RichText::new(&status).color(color), None, |ui| {
                if ui.add_enabled(can_change, egui::Button::new(tl!("Change…"))).clicked() {
                    let current = match &a.vacation {
                        VacationState::Loaded(v) => v.clone(),
                        _ => Default::default(),
                    };
                    // Exchange keeps one reply for inside the organisation and one for outside.
                    let exchange = a.config.ews.is_some() || a.config.graph.is_some();
                    acts.push(Action::Dialog(Dialog::vacation(&id, &current, exchange)));
                }
                if matches!(a.vacation, VacationState::Loading) {
                    ui.spinner();
                }
            });
        },
    );

    // Mailbox: space used on the server and here, where the data file is, compacting it,
    // emptying. Its size is looked at again once it's been compacted.
    if s.compacting.as_deref() == Some(id.as_str()) {
        b.cached.remove(&id);
    }
    let cached = *b.cached.entry(id.clone()).or_insert_with(|| s.cache_size(&id));
    let size = |n: u64| widgets::human_size(usize::try_from(n).unwrap_or(usize::MAX));
    card(ui, t, tl!("Mailbox settings"), Some(tl!("See how much space your mail takes, and empty Deleted Items and Junk Email.")), |ui| {
        let mut first = true;
        if !a.is_local() {
            let (line, frac) = match a.quota {
                Some((used, limit)) if limit > 0 => (
                    tf!("{free} free of {total}", free = size(limit.saturating_sub(used)), total = size(limit)),
                    Some((used as f32 / limit as f32).clamp(0.0, 1.0)),
                ),
                Some((used, _)) => (tf!("{used} used on the server", used = size(used)), None),
                None => (tl!("The server doesn't say how much space is used.").to_string(), None),
            };
            setting_row(ui, t, true, name(t, tl!("On the server")), Some(note(t, &line)), |ui| {
                if ui.button(tl!("Folder Size…")).on_hover_text(tl!("How much space each folder takes, here and on the server")).clicked() {
                    acts.push(Action::Run("account.folderSizes", json!({"account": id})));
                    acts.push(Action::Dialog(Dialog::folder_size(s, &id)));
                }
                if let Some(frac) = frac {
                    widgets::meter(ui, t, frac, 200.0);
                }
            });
            first = false;
        }
        // Where, as Outlook shows a data file's place; and a copy as one.
        let online = a.config.keeps_online();
        let file = if has_files && !online { s.store.as_ref().map(|store| store.account_path(&id)) } else { None };
        let mut here = match &file {
            _ if online => tl!("Online only: none of this account's mail is kept on this computer.").to_string(),
            Some(f) => format!("{}\n{}", tf!("Kept on this computer: {size}", size = size(cached)), f.display()),
            None => tf!("Kept on this computer: {size}", size = size(cached)),
        };
        if let Some((_, before, after)) = s.compacted.as_ref().filter(|(a, _, _)| *a == id) {
            here.push('\n');
            here.push_str(&tf!("Compacted from {before} to {after}", before = size(*before), after = size(*after)));
        }
        setting_row(ui, t, first, name(t, tl!("On this computer")), Some(note(t, &here)), |ui| {
            // An account without a server has the button here.
            if first && !online && ui.button(tl!("Folder Size…")).on_hover_text(tl!("How much space each folder takes on this computer")).clicked()
            {
                acts.push(Action::Run("account.folderSizes", json!({"account": id})));
                acts.push(Action::Dialog(Dialog::folder_size(s, &id)));
            }
            if let Some(file) = &file {
                let busy = s.compacting.is_some();
                if ui
                    .add_enabled(!busy, egui::Button::new(tl!("Compact Now")))
                    .on_hover_text(tl!("Give the space freed inside the data file back to the disk"))
                    .clicked()
                {
                    acts.push(Action::Run("account.compact", json!({"account": id})));
                }
                if ui.button(tl!("Save as an Outlook data file…")).clicked() {
                    acts.push(Action::ExportDataFile(id.clone()));
                }
                if ui.button(tl!("Open folder")).clicked() {
                    // The folder the data file is in (or the account's own folder).
                    let dir = if file.is_dir() { file.clone() } else { file.parent().map_or_else(|| file.clone(), Path::to_path_buf) };
                    acts.push(Action::OpenFolder(dir));
                }
            }
        });
        setting_row(ui, t, false, name(t, tl!("Clean up")), None, |ui| {
            for (role, label) in [
                (quatrelle_engine::model::FolderRole::Junk, "Empty Junk Email…"),
                (quatrelle_engine::model::FolderRole::Trash, "Empty Deleted Items…"),
            ] {
                if let Some(f) = a.folder(role)
                    && ui.add_enabled(f.total > 0 || a.is_local(), egui::Button::new(tl!(label))).clicked()
                {
                    acts.push(Action::Dialog(Dialog::ConfirmEmpty {
                        account: id.clone(),
                        folder: f.path.clone(),
                        name: crate::i18n::folder(f),
                        for_good: true,
                    }));
                }
            }
        });
    });

    // Rules.
    let n = s.rules.len();
    card(ui, t, tl!("Rules"), Some(tl!("Sort new mail as it arrives: move it, flag it, categorize it.")), |ui| {
        let count = if n == 0 { tl!("No rules yet.").to_string() } else { tf!("Rules: {n}", n = n) };
        setting_row(ui, t, true, name(t, &count), None, |ui| {
            if ui.button(tl!("Manage rules")).clicked() {
                acts.push(Action::Dialog(Dialog::Rules));
            }
        });
    });
}
