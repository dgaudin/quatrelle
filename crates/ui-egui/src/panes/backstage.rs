//! The File page: everything about accounts and the app that isn't about one message. It
//! replaces the ribbon and panes until the user goes back: Info (the account's settings, its
//! automatic replies, the space it uses and its rules), Open & Export (Outlook data files),
//! Options and About.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use egui::{Margin, RichText, Sense, Ui, vec2};
use quatrelle_engine::{AccountState, ReadingPane, Session, VacationState};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog, UiState, widgets};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Info,
    Open,
    Options,
    About,
}

#[derive(Clone, Debug)]
pub struct Backstage {
    pub page: Page,
    /// The account Info shows (the first one until another is picked).
    pub account: Option<String>,
    /// Accounts whose automatic replies were asked for since the page opened.
    asked: HashSet<String>,
    /// Bytes cached per account, measured once per visit.
    cached: HashMap<String, u64>,
}

impl Backstage {
    pub fn new(page: Page) -> Backstage {
        Backstage { page, account: None, asked: HashSet::new(), cached: HashMap::new() }
    }
}

/// What kind of account it is, for the picker.
fn kind(a: &AccountState) -> &'static str {
    if a.is_data_file() {
        "Outlook data file"
    } else if a.is_local() {
        "Sample mailbox"
    } else if a.config.graph.is_some() {
        "Exchange (Microsoft Graph)"
    } else if a.config.ews.is_some() {
        "Exchange server (EWS)"
    } else if a.config.jmap.is_some() {
        "JMAP"
    } else if a.config.pop.is_some() {
        "POP and SMTP"
    } else {
        match a.config.oauth.as_deref() {
            Some("google") => "Google (IMAP)",
            Some("microsoft") => "Microsoft 365 (IMAP)",
            _ => "IMAP and SMTP",
        }
    }
}

pub fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let Some(mut b) = st.backstage.take() else { return };
    let mut close = false;
    egui::Panel::left("backstage-rail").exact_size(200.0).frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(0, 10))).show(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let r = rail_item(ui, &t, Icon::Back, "Back", false);
            if r.clicked() {
                close = true;
            }
            ui.add_space(8.0);
            for (page, icon, label) in [(Page::Info, Icon::Info, "Info"), (Page::Open, Icon::Folder, "Open & Export")] {
                if rail_item(ui, &t, icon, label, b.page == page).clicked() {
                    b.page = page;
                }
            }
            // The open message, as Outlook's File › Save As, Save Attachments and Print.
            ui.add_space(8.0);
            let open = s.reading.as_ref().and_then(|r| r.parsed.as_ref());
            let attachments = open.is_some_and(|p| !p.attachments.is_empty());
            for (icon, label, on, action) in [
                (Icon::File, "Save As", open.is_some(), Action::SaveMessage),
                (Icon::Attach, "Save Attachments", attachments, Action::SaveAll),
                (Icon::Print, "Print", open.is_some(), Action::Print),
            ] {
                let r = ui.add_enabled_ui(on, |ui| rail_item(ui, &t, icon, label, false)).inner;
                if on && r.clicked() {
                    acts.push(action);
                    close = true;
                }
            }
            ui.add_space(8.0);
            for (page, icon, label) in [(Page::Options, Icon::Gear, "Options"), (Page::About, Icon::Star, "About")] {
                if rail_item(ui, &t, icon, label, b.page == page).clicked() {
                    b.page = page;
                }
            }
            // Close Quatrelle (the desktop app; a browser tab is closed by the browser).
            if st.has_files && rail_item(ui, &t, Icon::Close, "Exit", false).clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        },
    );
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink(false).id_salt("backstage").show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin { left: 36, right: 36, top: 24, bottom: 32 }).show(ui, |ui| {
                ui.set_max_width(820.0);
                match b.page {
                    Page::Info => info(s, &t, st.has_files, ui, &mut b, acts),
                    Page::Open => open(s, &t, ui, &mut b, acts),
                    Page::Options => options(s, st, ui, acts),
                    Page::About => about(s, &t, st.pending_update.as_deref(), ui, acts),
                }
            });
        });
    });
    if !close {
        st.backstage = Some(b);
    }
}

fn rail_item(ui: &mut Ui, t: &Tokens, icon: Icon, label: &str, on: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
    if on || resp.hovered() {
        ui.painter().rect_filled(r, 0.0, if on { t.accent_soft } else { t.hover_row });
    }
    if on {
        ui.painter().rect_filled(egui::Rect::from_min_size(r.min, vec2(3.0, r.height())), 0.0, t.accent);
    }
    let ir = egui::Rect::from_center_size(r.left_center() + vec2(26.0, 0.0), vec2(18.0, 18.0));
    let enabled = ui.is_enabled();
    icons::paint(
        ui.painter(),
        ir,
        icon,
        if on {
            t.accent
        } else if enabled {
            t.dim
        } else {
            t.faint
        },
    );
    ui.painter().text(
        r.left_center() + vec2(46.0, 0.0),
        egui::Align2::LEFT_CENTER,
        tl!(label),
        if on { theme::bold(14.0) } else { theme::regular(14.0) },
        if enabled { t.text } else { t.faint },
    );
    resp
}

fn heading(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.label(RichText::new(tl!(text)).font(theme::bold(22.0)).color(t.text));
    ui.add_space(14.0);
}

/// A big square button with an icon, and its title, description and anything else beside it
/// (the fork's look for the File page's entries, as Outlook's Backstage).
fn tile(ui: &mut Ui, t: &Tokens, icon: Icon, label: &str, title: &str, about: &str, extra: impl FnOnce(&mut Ui)) -> egui::Response {
    let mut out = None;
    ui.horizontal_top(|ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(104.0, 92.0), Sense::click());
        let fill = if resp.hovered() { t.hover } else { t.surface };
        ui.painter().rect(r, 6.0, fill, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        // The icon over its label, the two centred in the tile together (a label in two lines,
        // as some languages need, moves the icon up), each line of the label centred.
        let mut job = egui::text::LayoutJob::simple(tl!(label).to_string(), theme::regular(12.5), t.text, 100.0);
        job.halign = egui::Align::Center;
        let galley = ui.painter().layout_job(job);
        let block = 28.0 + 8.0 + galley.size().y;
        let top = r.center().y - block / 2.0;
        icons::paint(ui.painter(), egui::Rect::from_center_size(egui::pos2(r.center().x, top + 14.0), vec2(28.0, 28.0)), icon, t.accent);
        ui.painter().galley(egui::pos2(r.center().x, top + 36.0), galley, t.text);
        ui.add_space(14.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(tl!(title)).font(theme::bold(16.0)).color(t.text));
            ui.label(RichText::new(tl!(about)).color(t.dim));
            ui.add_space(4.0);
            extra(ui);
        });
        out = Some(resp);
    });
    ui.add_space(18.0);
    out.unwrap_or_else(|| ui.label(""))
}

/// A card on a File page: its title and a line about it, then its rows.
fn card(ui: &mut Ui, t: &Tokens, title: &str, about: Option<&str>, body: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(t.surface)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(10)
        .inner_margin(Margin { left: 20, right: 20, top: 16, bottom: 10 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).font(theme::bold(15.0)).color(t.text));
            if let Some(about) = about {
                ui.add_space(2.0);
                ui.label(RichText::new(about).color(t.dim).size(12.5));
            }
            ui.add_space(6.0);
            body(ui);
        });
    ui.add_space(14.0);
}

/// One row of a card: its name (and a line under it) on the left, its controls against the
/// right edge, centred on each other; a thin line above every row but the first.
fn setting_row(ui: &mut Ui, t: &Tokens, first: bool, label: RichText, about: Option<RichText>, control: impl FnOnce(&mut Ui)) {
    if !first {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().rect_filled(r, 0.0, t.border.gamma_multiply(0.6));
    }
    ui.add_space(7.0);
    let left = (ui.available_width() - 300.0).max(ui.available_width() * 0.45);
    egui::Sides::new().height(if about.is_some() { 38.0 } else { 28.0 }).show(
        ui,
        |ui| match about {
            // One line, cut short before the controls (all of it on hover).
            None => {
                ui.scope(|ui| {
                    ui.set_max_width(left);
                    ui.add(egui::Label::new(label).truncate());
                });
            }
            Some(about) => {
                ui.vertical(|ui| {
                    ui.set_max_width(left);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(label);
                    ui.label(about);
                });
            }
        },
        control,
    );
    ui.add_space(7.0);
}

/// A row's name and the line under it, in the page's colours.
fn name(t: &Tokens, s: &str) -> RichText {
    RichText::new(s).color(t.text)
}

fn note(t: &Tokens, s: &str) -> RichText {
    RichText::new(s).color(t.faint).size(12.0)
}

fn info(s: &Session, t: &Tokens, has_files: bool, ui: &mut Ui, b: &mut Backstage, acts: &mut Vec<Action>) {
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

    // Mailbox: space used on the server and here, where the data file is, emptying.
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
                if let Some(frac) = frac {
                    ui.add(egui::ProgressBar::new(frac).desired_width(200.0).fill(if frac > 0.9 { t.danger } else { t.accent }));
                }
            });
            first = false;
        }
        // Where, as Outlook shows a data file's place; and a copy as one.
        let online = a.config.keeps_online();
        let file = if has_files && !online { s.store.as_ref().map(|store| store.account_path(&id)) } else { None };
        let here = match &file {
            _ if online => tl!("Online only: none of this account's mail is kept on this computer.").to_string(),
            Some(f) => format!("{}\n{}", tf!("Kept on this computer: {size}", size = size(cached)), f.display()),
            None => tf!("Kept on this computer: {size}", size = size(cached)),
        };
        setting_row(ui, t, first, name(t, tl!("On this computer")), Some(note(t, &here)), |ui| {
            if let Some(file) = &file {
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

/// Open & Export: Outlook data files, and the rules as a file.
fn open(s: &Session, t: &Tokens, ui: &mut Ui, b: &mut Backstage, acts: &mut Vec<Action>) {
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

fn options(s: &Session, st: &UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    heading(ui, &t, "Options");
    ui.set_max_width(760.0);
    // A card per subject, one setting per row (the helpers Info and Open & Export use too).
    let group = |ui: &mut Ui, title: &str, about: Option<&str>, body: &mut dyn FnMut(&mut Ui)| card(ui, &t, title, about, |ui| body(ui));
    let setting = |ui: &mut Ui, first: bool, label: &str, about: Option<&str>, control: &mut dyn FnMut(&mut Ui)| {
        setting_row(ui, &t, first, name(&t, label), about.map(|a| note(&t, a)), |ui| control(ui));
    };
    let switch = |ui: &mut Ui, on: bool, label: &str, id: &'static str, acts: &mut Vec<Action>| {
        let mut v = on;
        if widgets::switch(ui, &t, &mut v, label).changed() {
            acts.push(Action::Run(id, json!({"on": v})));
        }
    };

    group(ui, tl!("General"), None, &mut |ui| {
        setting(ui, true, tl!("Theme"), None, &mut |ui| {
            let options = [tl!("System"), tl!("Light"), tl!("Dark")];
            let values = [None, Some(false), Some(true)];
            let now = values.iter().position(|v| *v == st.dark).unwrap_or(0);
            if let Some(i) = widgets::segmented(ui, &t, "theme", &options, now)
                && let Some(v) = values.get(i)
            {
                acts.push(Action::SetDark(*v));
            }
        });
        let mut accent = st.system_accent;
        if ui.checkbox(&mut accent, tl!("Use the system's accent colour")).changed() {
            acts.push(Action::SetSystemAccent(accent));
        }
        setting(ui, false, tl!("Language"), None, &mut |ui| {
            let language = st.language.as_str();
            let current = crate::i18n::Lang::from_code(language).map_or_else(|| tl!("Same as the system").to_string(), |l| l.name().to_string());
            egui::ComboBox::from_id_salt("language").width(200.0).selected_text(current).show_ui(ui, |ui| {
                if ui.selectable_label(language == crate::i18n::AUTO, tl!("Same as the system")).clicked() {
                    acts.push(Action::SetLanguage(crate::i18n::AUTO.into()));
                }
                for l in crate::i18n::Lang::all() {
                    if ui.selectable_label(language == l.code(), l.name()).clicked() {
                        acts.push(Action::SetLanguage(l.code().into()));
                    }
                }
            });
        });
    });

    // Where the data files are (one per account, as Outlook's .pst files); movable as those are.
    if let Some((dir, movable)) = &st.data_folder {
        group(
            ui,
            tl!("Data files"),
            Some(tl!(
                "Each account's mail, contacts and calendar are kept in a Quatrelle data file (.quatrelle) in this folder, as Outlook keeps each account in a .pst in Outlook Files. An Outlook data file you open gets a .quatrelle file of its own here; Outlook's file is left as it was. To have a copy as a .pst, use Save as an Outlook data file under Info."
            )),
            &mut |ui| {
                let path = dir.display().to_string();
                setting(ui, true, tl!("Folder"), Some(&path), &mut |ui| {
                    if *movable && ui.button(tl!("Move…")).clicked() {
                        acts.push(Action::MoveDataFolder);
                    }
                    if ui.button(tl!("Open folder")).clicked() {
                        acts.push(Action::OpenFolder(dir.clone()));
                    }
                });
                // The account list, settings and log are in the app folder, not with the data
                // files (they're easy to look for in Documents).
                if let Some(app) = s.store.as_ref().map(|st| st.root().to_path_buf()).filter(|r| r != dir) {
                    let path = app.display().to_string();
                    setting(ui, false, tl!("Accounts and settings"), Some(&path), &mut |ui| {
                        if ui
                            .button(tl!("Open folder"))
                            .on_hover_text(tl!("The account list (no passwords), the window's settings and the log"))
                            .clicked()
                        {
                            acts.push(Action::OpenFolder(app.clone()));
                        }
                    });
                }
                if !*movable && st.pending_move.is_none() {
                    let why = if s.portable {
                        tl!("Portable: everything is kept in QuatrelleData beside Quatrelle, and passwords only until it closes.")
                    } else {
                        tl!("Chosen when Quatrelle was started (--data or QUATRELLE_DATA), so it can't be moved here.")
                    };
                    ui.label(RichText::new(why).color(t.faint).small());
                }
                if let Some(to) = &st.pending_move {
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(tf!("Moves to {folder} the next time Quatrelle starts.", folder = to.display().to_string()))
                                .color(t.warn)
                                .small(),
                        );
                        if ui.small_button(tl!("Cancel")).clicked() {
                            acts.push(Action::CancelDataMove);
                        }
                    });
                }
                ui.add_space(8.0);
            },
        );
    }

    group(ui, tl!("Reading"), None, &mut |ui| {
        setting(ui, true, tl!("Reading pane"), None, &mut |ui| {
            let values = [(ReadingPane::Right, "right"), (ReadingPane::Bottom, "bottom"), (ReadingPane::Off, "off")];
            let now = values.iter().position(|(v, _)| *v == s.prefs.reading_pane).unwrap_or(0);
            if let Some(i) = widgets::segmented(ui, &t, "reading-pane", &[tl!("Right"), tl!("Bottom"), tl!("Off")], now)
                && let Some((_, code)) = values.get(i)
            {
                acts.push(Action::Run("view.readingPane", json!({"value": code})));
            }
        });
        setting(ui, false, tl!("Group messages by conversation"), None, &mut |ui| {
            switch(ui, s.prefs.conversations, tl!("Group messages by conversation"), "view.conversations", acts);
        });
        setting(ui, false, tl!("Show a line of each message in the list"), None, &mut |ui| {
            switch(ui, s.prefs.show_preview, tl!("Show a line of each message in the list"), "view.preview", acts);
        });
        setting(ui, false, tl!("Compact list"), None, &mut |ui| {
            switch(ui, s.prefs.compact, tl!("Compact list"), "view.compact", acts);
        });
        // When the reading pane marks mail read, as Outlook's Reading Pane options: when shown
        // (after a wait), once another message is chosen, or not at all (only a message opened
        // in its own window is then).
        let on = s.prefs.mark_read_on_open;
        setting(
            ui,
            false,
            tl!("Mark messages as read in the reading pane"),
            Some(tl!("A message opened in its own window (double-click) is always marked as read.")),
            &mut |ui| {
                let mut v = on;
                if widgets::switch(ui, &t, &mut v, tl!("Mark messages as read in the reading pane")).changed() {
                    acts.push(Action::Run("view.readingOptions", json!({"markRead": v})));
                }
            },
        );
        if on {
            ui.indent("read-when", |ui| {
                let shown = s.prefs.read_in_pane == quatrelle_engine::ReadInPane::Shown;
                ui.horizontal(|ui| {
                    if ui.radio(shown, tl!("When it's shown, after")).clicked() {
                        acts.push(Action::Run("view.readingOptions", json!({"when": "shown"})));
                    }
                    let mut secs = s.prefs.read_after_secs;
                    if ui.add_enabled(shown, egui::DragValue::new(&mut secs).range(0..=999).speed(0.2)).changed() {
                        acts.push(Action::Run("view.readingOptions", json!({"after": secs})));
                    }
                    ui.label(tl!("seconds"));
                });
                if ui.radio(!shown, tl!("When I choose another message")).clicked() {
                    acts.push(Action::Run("view.readingOptions", json!({"when": "left"})));
                }
            });
            ui.add_space(8.0);
        }
        setting(ui, false, tl!("Write new messages in a window of their own"), None, &mut |ui| {
            switch(ui, s.prefs.new_mail_window, tl!("Write new messages in a window of their own"), "view.newMailWindow", acts);
        });
        setting(ui, false, tl!("Check the spelling as you type"), None, &mut |ui| {
            switch(ui, s.prefs.spell_check, tl!("Check the spelling as you type"), "view.spelling", acts);
        });
        // The words added with "Add to dictionary", each taken back out here (they belong to the
        // preferences, so they stay between starts).
        if !s.prefs.spelling.is_empty() {
            setting(ui, false, tl!("Your words"), None, &mut |ui| {
                ui.horizontal_wrapped(|ui| {
                    for w in s.prefs.spelling.clone() {
                        let label = format!("{w}  ✕");
                        if ui.button(label).on_hover_text(tl!("Remove")).clicked() {
                            acts.push(Action::Run("spelling.remove", serde_json::json!({"word": w.clone()})));
                            crate::spellcheck::remove_word(&w);
                        }
                    }
                });
            });
        }
        setting(ui, false, tl!("Show HTML mail formatted (otherwise as plain text)"), None, &mut |ui| {
            switch(ui, s.prefs.show_html, tl!("Show HTML mail formatted (otherwise as plain text)"), "view.html", acts);
        });
    });

    // How much mail is kept on this computer, as Outlook's "Mail to keep offline".
    group(
        ui,
        tl!("Mail kept on this computer"),
        Some(tl!("New mail always comes in. Older mail past these stays on the server; the rest of each message is downloaded when it's opened.")),
        &mut |ui| {
            setting(ui, true, tl!("Of each folder, keep"), None, &mut |ui| {
                let values = [1000u32, 5000, 20000, 0];
                let now = values.iter().position(|v| *v == s.prefs.keep_messages).unwrap_or(0);
                if let Some(i) = widgets::segmented(ui, &t, "keep", &["1,000", "5,000", "20,000", tl!("All")], now)
                    && let Some(n) = values.get(i)
                {
                    acts.push(Action::Run("account.keepMail", json!({"messages": n})));
                }
            });
            setting(ui, false, tl!("Download in full for reading offline"), None, &mut |ui| {
                let values = [40u32, 200, 1000];
                let now = values.iter().position(|v| *v == s.prefs.keep_offline).unwrap_or(0);
                if let Some(i) = widgets::segmented(ui, &t, "offline", &["40", "200", "1,000"], now)
                    && let Some(n) = values.get(i)
                {
                    acts.push(Action::Run("account.keepMail", json!({"offline": n})));
                }
            });
            // Issue #4: an account kept online only has none of its mail here.
            let servers: Vec<&quatrelle_engine::model::AccountConfig> =
                s.accounts.iter().map(|a| &a.config).filter(|c| c.can_be_online_only()).collect();
            for (i, c) in servers.iter().enumerate() {
                let label = tf!("Online only: {email}", email = c.email);
                setting(ui, false, &label, (i == 0).then(crate::dialogs::online_only_about), &mut |ui| {
                    let mut on = c.keeps_online();
                    if widgets::switch(ui, &t, &mut on, &label).changed() {
                        acts.push(Action::Run("account.onlineOnly", json!({"account": c.id, "on": on})));
                    }
                });
            }
        },
    );

    group(ui, tl!("Inbox and notifications"), None, &mut |ui| {
        setting(ui, true, tl!("Show a desktop notification when new mail arrives"), None, &mut |ui| {
            switch(ui, s.prefs.notify, tl!("Show a desktop notification when new mail arrives"), "view.notifications", acts);
        });
        setting(ui, false, tl!("Focused Inbox: put newsletters and automatic mail in Other"), None, &mut |ui| {
            switch(ui, s.prefs.focused_inbox, tl!("Focused Inbox: put newsletters and automatic mail in Other"), "view.focused", acts);
        });
        setting(ui, false, tl!("Ask before deleting for good"), None, &mut |ui| {
            switch(ui, s.prefs.confirm_permanent_delete, tl!("Ask before deleting for good"), "view.confirmDelete", acts);
        });
        setting(
            ui,
            false,
            tl!("Play animations"),
            Some(tl!("Hovers, selections and panes ease into place; off, everything changes at once")),
            &mut |ui| {
                switch(ui, s.prefs.animations, tl!("Play animations"), "view.animations", acts);
            },
        );
    });

    // Junk Email: the blocked and safe senders, each list with a box to add to it.
    for (list, title, about) in [
        ("blocked", "Blocked senders", "Mail from these addresses and domains goes to Junk Email as it arrives."),
        ("safe", "Safe senders", "Mail from these addresses and domains is never treated as junk, and its pictures load."),
    ] {
        let entries = if list == "blocked" { &s.prefs.blocked_senders } else { &s.prefs.safe_senders };
        group(ui, tl!(title), Some(tl!(about)), &mut |ui| {
            let id = egui::Id::unique(("junk-add", list));
            let mut text: String = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_default();
            let mut add = false;
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut text).hint_text(tl!("name@example.com or @example.com")).desired_width(320.0));
                add |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                add |= ui.add_enabled(!text.trim().is_empty(), egui::Button::new(tl!("Add"))).clicked();
            });
            if add && !text.trim().is_empty() {
                acts.push(Action::Run("junk.edit", json!({"list": list, "entry": text.trim()})));
                // One that can't be added stays to be corrected (the reason shows as a notice).
                if quatrelle_engine::junk_entry(&text).is_some() {
                    text.clear();
                }
            }
            ui.data_mut(|d| d.insert_temp(id, text));
            if entries.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(tl!("No senders yet.")).color(t.faint).small());
                ui.add_space(8.0);
            }
            // A long list is shown in part: the first ones, and how many more.
            for (i, who) in entries.iter().take(200).enumerate() {
                setting(ui, i == 0, who, None, &mut |ui| {
                    if ui.button(tl!("Remove")).clicked() {
                        acts.push(Action::Run("junk.edit", json!({"list": list, "entry": who, "on": false})));
                    }
                });
            }
            if entries.len() > 200 {
                ui.label(RichText::new(tf!("And {n} more", n = entries.len() - 200)).color(t.faint).small());
            }
        });
    }

    group(ui, tl!("Privacy"), Some(tl!("Pictures from the web load only when you ask. These senders' pictures always load:")), &mut |ui| {
        if s.prefs.trusted_senders.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(tl!("No senders yet.")).color(t.faint).small());
            ui.add_space(8.0);
        }
        for (i, who) in s.prefs.trusted_senders.iter().enumerate() {
            setting(ui, i == 0, who, None, &mut |ui| {
                if ui.button(tl!("Remove")).clicked() {
                    acts.push(Action::Run("view.trustSender", json!({"address": who, "on": false})));
                }
            });
        }
    });
}

/// Check for updates, and what it found: a newer release to download (its installer for this
/// system, else its page) and what's new in it. Nothing installs by itself.
fn updates(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    use quatrelle_engine::UpdateCheck;
    if s.jmap_only {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        let checking = s.update == UpdateCheck::Checking;
        if ui.add_enabled(!checking, egui::Button::new(tl!("Check for updates"))).clicked() {
            acts.push(Action::Run("app.checkUpdates", json!({})));
        }
        match &s.update {
            UpdateCheck::NotAsked => {}
            UpdateCheck::Checking => {
                ui.spinner();
                ui.label(RichText::new(tl!("Checking…")).color(t.dim));
            }
            UpdateCheck::UpToDate(_) => {
                ui.label(RichText::new(tl!("You have the newest version.")).color(t.dim));
            }
            UpdateCheck::Available(r) => {
                let name = r.tag.trim_start_matches('v').to_string();
                ui.label(RichText::new(tf!("Quatrelle {version} is available.", version = name)).color(t.text).strong());
                let target = if r.download.is_empty() { r.page.clone() } else { r.download.clone() };
                if widgets::accent_button(ui, Icon::Sync, "Download", t, true).on_hover_text(&target).clicked() {
                    acts.push(Action::OpenUrl(target));
                }
                if ui.link(tl!("What's new")).on_hover_text(&r.page).clicked() {
                    acts.push(Action::OpenUrl(r.page.clone()));
                }
            }
            UpdateCheck::Failed(e) => {
                ui.label(RichText::new(tf!("Couldn't check for updates: {error}", error = e)).color(t.danger));
            }
        }
    });
}

fn about(s: &Session, t: &Tokens, pending: Option<&str>, ui: &mut Ui, acts: &mut Vec<Action>) {
    heading(ui, t, "About Quatrelle");
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
        ui.painter().rect_filled(r, 12.0, t.accent);
        icons::paint(ui.painter(), icons::centered(r, 34.0), Icon::Unread, egui::Color32::WHITE);
        ui.add_space(10.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(tf!("Quatrelle {version}", version = env!("CARGO_PKG_VERSION"))).font(theme::bold(18.0)).color(t.text));
            ui.label(RichText::new(tl!("An ArtCraft app · MIT OR Apache-2.0")).color(t.dim));
        });
    });
    ui.add_space(10.0);
    updates(s, t, ui, acts);
    // A test build that updates itself, with the next one downloaded.
    if let Some(tag) = pending {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let version = tag.trim_start_matches('v');
            ui.label(RichText::new(tf!("Quatrelle {version} is downloaded. It installs when you close Quatrelle.", version = version)).color(t.text));
            if widgets::accent_button(ui, Icon::Sync, "Restart to update", t, true).clicked() {
                acts.push(Action::RestartToUpdate);
            }
        });
    }
    ui.add_space(14.0);
    for line in [
        "Local-first: mail is cached on this computer for fast offline search. Passwords stay in the system keychain.",
        "Quatrelle talks only to your own mail servers, over encrypted connections. Pictures from the web load only when you ask.",
        "Built in the open from the email standards: no code, icons or assets from other mail apps.",
    ] {
        ui.label(RichText::new(tl!(line)).color(t.dim));
        ui.add_space(4.0);
    }
    ui.add_space(8.0);
    if ui.link(tl!("Source code and issues")).clicked() {
        acts.push(Action::OpenUrl("https://github.com/dgaudin/quatrelle".into()));
    }
    // Fonts built in from craft-fonts travel with their licences.
    if !crate::craft_fonts::CRAFT_FONTS.is_empty() {
        ui.add_space(12.0);
        ui.label(RichText::new(tl!("Fonts")).font(theme::bold(14.0)).color(t.text));
        for f in crate::craft_fonts::CRAFT_FONTS {
            egui::CollapsingHeader::new(RichText::new(format!("{} {} · {}", f.family, f.style, f.licence)).color(t.dim))
                .id_salt(("craft-font", f.family, f.style))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(220.0).id_salt(("craft-font-licence", f.family, f.style)).show(ui, |ui| {
                        ui.label(RichText::new(f.licence_text).small().color(t.dim));
                    });
                });
        }
    }
}
