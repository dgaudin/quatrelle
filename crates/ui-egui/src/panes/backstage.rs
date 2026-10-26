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

mod about;
mod info;
mod open;
mod options;
mod security;

use about::about;
use info::info;
use open::open;
use options::options;

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
            // The open message, as Outlook's File › Save As, Save Attachments, Print, and the
            // headers its Properties show.
            ui.add_space(8.0);
            let open = s.reading.as_ref().and_then(|r| r.parsed.as_ref());
            let attachments = open.is_some_and(|p| !p.attachments.is_empty());
            for (icon, label, on, action) in [
                (Icon::File, "Save As", open.is_some(), Action::SaveMessage),
                (Icon::Attach, "Save Attachments", attachments, Action::SaveAll),
                (Icon::Print, "Print", open.is_some(), Action::Print),
                (Icon::Info, "Message headers", open.is_some(), Action::Run("message.headers", json!({}))),
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
                    Page::About => about(s, &t, &st.update, ui, acts),
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
    crate::widgets::named(ui, &resp, tl!(label), true, Some(on));
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
        crate::widgets::named(ui, &resp, tl!(label), true, None);
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
