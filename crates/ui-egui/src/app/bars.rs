//! The top bar (search, sync, settings), the status bar, and notices.

use crate::*;

impl MailApp {
    pub(crate) fn top_bar(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        ui.horizontal_centered(|ui| {
            // The app's own mark, the only colour up here: the bar itself is plain, as the title
            // bars around it are.
            let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
            ui.painter().rect_filled(r, 4.0, t.accent);
            icons::paint(ui.painter(), icons::centered(r, 14.0), Icon::Unread, t.on_accent);
            ui.add_space(4.0);
            // Quick access, as Outlook's title bar: Send / Receive all folders, and Undo the last
            // move, delete, flag or category (Ctrl+Z).
            let can_sync = self.session.registry().get("mail.sync").is_some_and(|c| (c.enabled)(&self.session).is_none());
            if can_sync && widgets::bar_icon_button(ui, Icon::Sync, tl!("Send / Receive all folders (F9)"), &t).clicked() {
                acts.push(Action::Run("mail.sync", json!({})));
            }
            if !self.session.undo.is_empty() && widgets::bar_icon_button(ui, Icon::Undo, tl!("Undo (Ctrl+Z)"), &t).clicked() {
                acts.push(Action::Run("edit.undo", json!({})));
            }
            ui.add_space(10.0);
            // Centred in the window, whatever is left of it.
            let avail = ui.available_width();
            let search_w = (avail * 0.45).clamp(220.0, 620.0);
            let left = ui.min_rect().width();
            let full = left + avail;
            ui.add_space(((full - search_w) / 2.0 - left).clamp(0.0, (avail - search_w - 80.0).max(0.0)));
            let frame = egui::Frame::new()
                .fill(if t.dark { t.surface } else { Color32::WHITE })
                .stroke(egui::Stroke::new(1.0, t.border))
                .corner_radius(4)
                .inner_margin(Margin::symmetric(8, 3));
            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    icons::paint(ui.painter(), r, Icon::Search, t.dim);
                    // In People, Tasks and Notes it searches those (as Outlook's box searches where
                    // you are).
                    let module = self.session.module;
                    if matches!(module, Module::People | Module::Tasks | Module::Notes) {
                        let focus = std::mem::take(&mut self.ui.focus_search);
                        let (query, hint) = match module {
                            Module::People => (&mut self.ui.people_query, tl!("Search people")),
                            Module::Tasks => (&mut self.ui.tasks.query, tl!("Search tasks")),
                            _ => (&mut self.ui.notes_query, tl!("Search notes")),
                        };
                        let resp = ui.add(
                            egui::TextEdit::singleline(query)
                                .hint_text(hint)
                                .frame(egui::Frame::NONE)
                                .desired_width(search_w - 60.0)
                                .text_color(t.text),
                        );
                        if focus {
                            resp.request_focus();
                        }
                        if !query.is_empty() && widgets::tiny_icon_button(ui, Icon::Close, "Clear search", &t).clicked() {
                            query.clear();
                        }
                        return;
                    }
                    self.search_box(ui, search_w, acts);
                });
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let dark = t.dark;
                let (icon, tip) = if dark { (Icon::Sun, tl!("Switch to light mode")) } else { (Icon::Moon, tl!("Switch to dark mode")) };
                if widgets::bar_icon_button(ui, icon, tip, &t).clicked() {
                    acts.push(Action::SetDark(Some(!dark)));
                }
                if widgets::bar_icon_button(ui, Icon::Gear, "Accounts and settings", &t).clicked() {
                    acts.push(Action::Backstage(backstage::Page::Info));
                }
            });
        });
    }

    pub(crate) fn status_bar(&mut self, ui: &mut Ui) {
        let t = self.ui.tokens;
        let s = &self.session;
        let mut again: Option<String> = None;
        let mut restart = false;
        let mut zoom_to: Option<u16> = None;
        ui.horizontal_centered(|ui| {
            // The items of what's shown: the list's mail, or (in People) the contacts.
            if s.module == Module::People {
                ui.label(RichText::new(tf!("Items: {n}", n = people::shown(s, &self.ui).len())).color(t.dim).small());
            } else if s.module == Module::Mail {
                let unread = s.list.iter().filter(|m| !m.flags.seen).count();
                ui.label(RichText::new(tf!("Items: {n}", n = s.list.len())).color(t.dim).small());
                if unread > 0 {
                    ui.label(RichText::new(tf!("Unread: {n}", n = unread)).color(t.dim).small());
                }
            }
            // A search's index is still being built: say so, as Outlook's search does, rather
            // than showing a short list with no word about it.
            if s.search_pending {
                ui.label(RichText::new(tl!("Searching…")).color(t.accent).small());
            }
            // Download all, as Outlook's status bar shows it while mail comes down.
            if let Some((done, total, _)) = s.download_progress() {
                ui.label(RichText::new(tf!("Downloading {done} of {total}", done = done, total = total)).color(t.accent).small());
            }
            if !s.outbox.is_empty() {
                ui.label(RichText::new(tf!("Outbox: {n}", n = s.outbox.len())).color(t.warn).small());
            }
            if s.prefs.work_offline {
                ui.label(RichText::new(tl!("Working offline")).color(t.warn).small());
            }
            // A test build's next version: how far it's downloaded, then restart into it.
            restart = self.update_status(ui);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // The message read's zoom, at the far right as in Outlook's status bar.
                if s.module == Module::Mail && s.reading.is_some() {
                    zoom_to = zoom_control(ui, &t, s.prefs.reading_zoom);
                    ui.add_space(12.0);
                }
                let dot = |ui: &mut Ui, color: egui::Color32| {
                    let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 4.0, color);
                };
                // One account: its own status. Several: one status for them all (a line per
                // account didn't fit), with each on hover; only those that need you stand out.
                if s.accounts.len() <= 1 {
                    for a in &s.accounts {
                        let (color, text) = account_status(s, &t, a);
                        // "Password needed" is a link: click it to enter the password again.
                        if needs_you(a) {
                            if ui.link(RichText::new(text).color(t.danger).small()).on_hover_text(why(a)).clicked() {
                                again = Some(a.config.id.clone());
                            }
                        } else {
                            ui.label(RichText::new(text).color(t.dim).small()).on_hover_text(&a.config.email);
                        }
                        dot(ui, color);
                    }
                    return;
                }
                let each: Vec<String> =
                    s.accounts.iter().map(|a| tf!("{account}: {status}", account = a.config.email, status = account_status(s, &t, a).1)).collect();
                let each = each.join("\n");
                let busy = s.accounts.iter().any(|a| matches!(a.status, SyncStatus::Syncing { .. }));
                let connecting = s.accounts.iter().any(|a| matches!(a.status, SyncStatus::Connecting));
                let failed = s.accounts.iter().filter(|a| matches!(a.status, SyncStatus::Error { .. }) && !needs_you(a)).count();
                let online = s.accounts.iter().any(|a| matches!(a.status, SyncStatus::Online { .. }));
                let (color, text) = if busy {
                    (t.accent, tl!("Updating…").to_string())
                } else if connecting {
                    (t.accent, tl!("Connecting…").to_string())
                } else if failed > 0 {
                    (t.warn, tf!("{n} accounts disconnected", n = failed))
                } else if online {
                    (t.ok, tl!("All folders are up to date").to_string())
                } else {
                    (t.faint, tl!("Offline").to_string())
                };
                ui.label(RichText::new(text).color(t.dim).small()).on_hover_text(&each);
                dot(ui, color);
                // Accounts waiting for you: a link each (it signs in again), as many as fit beside
                // the counts on the left; the rest as "+N", each on hover (a dozen of them ran over
                // "Items" and "Unread").
                let waiting: Vec<&quatrelle_engine::AccountState> = s.accounts.iter().rev().filter(|a| needs_you(a)).collect();
                let mut shown = 0;
                for a in &waiting {
                    let text = tf!("{account}: {status}", account = a.config.email, status = account_status(s, &t, a).1);
                    let width = ui.painter().layout_no_wrap(text.clone(), egui::TextStyle::Small.resolve(ui.style()), t.danger).size().x;
                    // Room for this one and a "+N" after it.
                    if ui.available_width() < width + 70.0 {
                        break;
                    }
                    ui.add_space(10.0);
                    if ui.link(RichText::new(text).color(t.danger).small()).on_hover_text(why(a)).clicked() {
                        again = Some(a.config.id.clone());
                    }
                    dot(ui, t.danger);
                    shown += 1;
                }
                let rest: Vec<&&quatrelle_engine::AccountState> = waiting.iter().skip(shown).collect();
                if let Some(first) = rest.first() {
                    ui.add_space(10.0);
                    let each: Vec<String> =
                        rest.iter().map(|a| tf!("{account}: {status}", account = a.config.email, status = account_status(s, &t, a).1)).collect();
                    if ui
                        .link(RichText::new(format!("+{}", rest.len())).color(t.danger).small())
                        .on_hover_text(each.join(
                            "
",
                        ))
                        .clicked()
                    {
                        again = Some(first.config.id.clone());
                    }
                    dot(ui, t.danger);
                }
            });
        });
        if let Some(id) = again {
            self.sign_in_again(&id);
        }
        if restart {
            self.act(Action::RestartToUpdate);
        }
        if let Some(percent) = zoom_to {
            self.act(Action::Run("view.zoom", json!({"percent": percent})));
        }
    }

    pub(crate) fn toasts(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if self.session.notices.len() < self.ui.seen_notices {
            self.ui.seen_notices = 0;
        }
        if let Some(n) = self.session.notices.get(self.ui.seen_notices..).and_then(|v| v.last()) {
            self.ui.toast = Some((n.text.clone(), n.error, now));
        }
        self.ui.seen_notices = self.session.notices.len();
        let Some((text, error, at)) = self.ui.toast.clone() else { return };
        let ttl = if error { 7.0 } else { 3.5 };
        if now - at > ttl {
            self.ui.toast = None;
            return;
        }
        let t = self.ui.tokens;
        // In: up from a little below, settling; out: fading over its last moments.
        let since = (now - at) as f32;
        let (enter, leave) = if motion::enabled(ctx) {
            (
                motion::decelerate((since / motion::GENTLE).clamp(0.0, 1.0)),
                1.0 - motion::accelerate(((since - (ttl as f32 - motion::GENTLE)) / motion::GENTLE).clamp(0.0, 1.0)),
            )
        } else {
            (1.0, 1.0)
        };
        if enter < 1.0 || leave < 1.0 {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((at + ttl - motion::GENTLE as f64 - now).max(0.0)));
        }
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(egui::Align2::RIGHT_BOTTOM, vec2(-16.0, -36.0 + 14.0 * (1.0 - enter)))
            .order(egui::Order::Foreground)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.multiply_opacity(enter.min(leave));
                egui::Frame::new()
                    .fill(if error { t.danger } else { t.text })
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(14, 10))
                    .shadow(egui::epaint::Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(60) })
                    .show(ui, |ui| {
                        ui.set_max_width(420.0);
                        // Notices from the engine are English; the ones with a fixed text have a translation.
                        ui.label(RichText::new(tl!(&text)).color(if error { Color32::WHITE } else { t.surface }));
                    });
            });
    }
}

/// An account that waits for you to do something: sign in (again, or with Microsoft rather than
/// a password), or choose what to do about a mailbox Microsoft won't open over IMAP.
pub(crate) fn needs_you(a: &quatrelle_engine::AccountState) -> bool {
    match &a.status {
        SyncStatus::NeedsPassword { .. } => true,
        SyncStatus::Error { error } => {
            error.starts_with(quatrelle_engine::model::MICROSOFT_IMAP_CLOSED) || error.starts_with(quatrelle_engine::model::MICROSOFT_NO_PASSWORDS)
        }
        _ => false,
    }
}

/// An account's address, and what's wrong with it when something is (for hovering its status).
pub(crate) fn why(a: &quatrelle_engine::AccountState) -> String {
    match &a.status {
        SyncStatus::Error { error } | SyncStatus::NeedsPassword { error } if !error.is_empty() => {
            format!("{}\n{}", a.config.email, tl!(error))
        }
        _ => a.config.email.clone(),
    }
}

/// An account's status for the status bar: its colour and words.
pub(crate) fn account_status(s: &Session, t: &Tokens, a: &quatrelle_engine::AccountState) -> (egui::Color32, String) {
    match &a.status {
        SyncStatus::Error { error } if error.starts_with(quatrelle_engine::model::MICROSOFT_NO_PASSWORDS) => {
            (t.danger, tl!("Sign in with Microsoft").to_string())
        }
        SyncStatus::Error { error } if error.starts_with(quatrelle_engine::model::MICROSOFT_IMAP_CLOSED) => {
            (t.danger, tl!("Mailbox won't open").to_string())
        }
        SyncStatus::Online { at } => (t.ok, tf!("Updated {time}", time = i18n::date(&quatrelle_engine::model::dates::list_time(*at, &s.clock)))),
        SyncStatus::Syncing { .. } if a.is_data_file() => (t.accent, tf!("Opening {file}…", file = a.config.email)),
        SyncStatus::Syncing { folder } => {
            (t.accent, tf!("Updating {folder}…", folder = s.folder(&a.config.id, folder).map(i18n::folder).unwrap_or_default()))
        }
        SyncStatus::Connecting => (t.accent, tl!("Connecting…").to_string()),
        SyncStatus::Offline if a.is_data_file() => (t.faint, tl!("Outlook data file").to_string()),
        SyncStatus::Offline if a.is_local() => (t.faint, tl!("Sample mailbox (offline)").to_string()),
        SyncStatus::Offline => (t.faint, tl!("Offline").to_string()),
        // An account that signs in in the browser has no password to ask for.
        SyncStatus::NeedsPassword { .. } if a.config.oauth.is_some() => (t.danger, tl!("Sign in again").to_string()),
        SyncStatus::NeedsPassword { .. } => (t.danger, tl!("Password needed").to_string()),
        SyncStatus::Error { error } => (t.warn, tf!("Disconnected: {error}", error = widgets::shorten(error, 60))),
    }
}

/// Mail, Calendar, People, Tasks and Notes down the window's edge, as Outlook's: the module shown
/// lit, with a bar at its edge.
pub(super) fn module_rail(ui: &mut Ui, t: &Tokens, module: Module, acts: &mut Vec<Action>) {
    ui.spacing_mut().item_spacing.y = 4.0;
    for (m, icon, label, code) in [
        (Module::Mail, Icon::Unread, "Mail", "mail"),
        (Module::Calendar, Icon::Calendar, "Calendar", "calendar"),
        (Module::People, Icon::People, "People", "people"),
        (Module::Tasks, Icon::Tasks, "Tasks", "tasks"),
        (Module::Notes, Icon::Note, "Notes", "notes"),
    ] {
        let (r, resp) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::click());
        let on = m == module;
        widgets::named(ui, &resp, tl!(label), true, Some(on));
        if on || resp.hovered() {
            ui.painter().rect_filled(r, 4.0, if on { t.accent_soft } else { t.hover });
        }
        if on {
            let edge = if i18n::current().rtl() { r.right() - 3.0 } else { r.left() };
            ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(edge, r.top() + 8.0), vec2(3.0, r.height() - 16.0)), 1.5, t.accent);
        }
        icons::paint(ui.painter(), icons::centered(r, 22.0), icon, if on { t.accent } else { t.dim });
        if resp.on_hover_text(tl!(label)).clicked() {
            acts.push(Action::Run("view.module", json!({"name": code})));
        }
    }
}

/// The status bar's zoom of the message read (drawn right to left: +, the percent, −); what it
/// was changed to. The percent goes back to 100.
fn zoom_control(ui: &mut Ui, t: &Tokens, percent: u16) -> Option<u16> {
    let (low, high) = (*quatrelle_engine::READING_ZOOM.start(), *quatrelle_engine::READING_ZOOM.end());
    let button = |text: String| egui::Button::new(RichText::new(text).color(t.dim).small()).frame_when_inactive(false);
    let tens = percent.saturating_add(5) / 10;
    let mut to = None;
    if ui.add_enabled(percent < high, button("+".into())).on_hover_text(tl!("Zoom in")).clicked() {
        to = Some(tens.saturating_add(1).saturating_mul(10));
    }
    if ui.add(button(tf!("{percent}%", percent = percent))).on_hover_text(tl!("Message zoom: click for 100%")).clicked() {
        to = Some(100);
    }
    if ui.add_enabled(percent > low, button("−".into())).on_hover_text(tl!("Zoom out")).clicked() {
        to = Some(tens.saturating_sub(1).saturating_mul(10));
    }
    to.map(|n| n.clamp(low, high)).filter(|n| *n != percent)
}
