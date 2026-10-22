//! The window, frame by frame: what comes in, the panes, the windows, what was asked.

use crate::*;

use super::ribbon::RIBBON_H;

impl MailApp {
    /// Every wake, the window shown or not. eframe runs no `ui` while the window is minimized,
    /// hidden or covered, only this: the workers' news is taken in, new mail and reminders are
    /// told on the desktop, and jobs go out. When all of it lived in `ui`, a minimized Quatrelle
    /// announced nothing (and synced nothing new into view) until it was shown again.
    pub fn logic(&mut self, ctx: &egui::Context) {
        self.ui.logic_ran = true;
        if self.pump() {
            ctx.request_repaint();
        }
        self.announce(ctx);
        // Calendar reminders: the desktop told of each as it comes, whatever the window shows.
        for r in reminders::refresh(&mut self.ui.reminders, &self.session, false) {
            let start = quatrelle_engine::model::dates::civil(r.start, &self.session.clock);
            let when = if r.all_day { String::new() } else { format!("{:02}:{:02}", start.hour, start.minute) };
            self.services.notify(&r.summary, &if r.location.is_empty() { when } else { format!("{when} · {}", r.location) });
        }
        self.flush_jobs();
        // Woken by the workers when they have news; this is for the clock (reminders, snoozed
        // mail coming back) when nothing else happens.
        ctx.request_repaint_after(std::time::Duration::from_secs(15));
    }

    /// The whole window.
    pub fn ui(&mut self, ui: &mut Ui) {
        // The switch in Options, wherever it was changed from (a command, another window).
        spellcheck::set_on(self.session.prefs.spell_check);
        let ctx = ui.ctx().clone();
        // Typefaces a message being written asked for (Arial, Calibri…), from the system.
        for name in std::mem::take(&mut self.ui.load_typefaces) {
            crate::look::typefaces::load(&ctx, &mut *self.services, &name);
        }
        let mut main = self.ui.places.get("main").copied();
        after_open(ui, main);
        WindowPlace::note(&mut main, ui);
        if let Some(p) = main {
            self.ui.places.insert("main".into(), p);
        }
        if !self.ui.fonts_ready {
            // Fonts take effect from the next frame; drawing now would miss the bold family.
            self.install_fonts(&ctx, None, None);
            ctx.request_repaint();
            return;
        }
        self.apply_language(&ctx);
        motion::set_enabled(&ctx, self.session.prefs.animations);
        if !std::mem::take(&mut self.ui.logic_ran) {
            self.logic(&ctx);
            self.ui.logic_ran = false;
        }
        self.settle_composes(&ctx);
        self.ui.pending_update = self.services.pending_update();
        // Closing with online-only accounts' unsent mail or changes held in memory: ask first.
        if self.ui.close_now {
            self.ui.close_now = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if ctx.input(|i| i.viewport().close_requested()) && !self.ui.closing_anyway {
            // Messages being written: kept as drafts if wanted, as Outlook asks; never lost.
            let written = self.session.all_composes().filter(|c| self.session.written(c) && !c.sending).count();
            let (unsent, changes) = self.session.lost_on_close();
            if written > 0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.ui.dialog = Some(Dialog::SaveOnClose(written));
                self.ui.dialog_for = None;
            } else if unsent + changes > 0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.ui.dialog = Some(Dialog::LostOnClose { unsent, changes });
            }
        }
        // A search started by a command or agent shows its query in the box.
        if let Some((q, _)) = &self.session.search
            && self.ui.search_text.is_empty()
            && !ctx.text_edit_focused()
            && self.session.open_search_folder_name().is_none()
        {
            self.ui.search_text = q.clone();
        }
        let system = self.services.system_style();
        let dark = self.ui.dark.or(system.dark).unwrap_or_else(|| ctx.system_theme() == Some(egui::Theme::Dark));
        let tokens = Tokens::for_dark(dark).with_accent(system.accent.filter(|_| self.ui.system_accent));
        if dark != self.ui.tokens.dark || tokens.accent != self.ui.tokens.accent || ctx.global_style().visuals.panel_fill != self.ui.tokens.surface {
            self.ui.tokens = tokens;
            theme::apply(&ctx, &self.ui.tokens);
        }
        // The title bar the system draws goes light or dark with the window: the system's
        // when following it, else the one chosen.
        if self.ui.window_theme != Some(self.ui.dark) {
            self.ui.window_theme = Some(self.ui.dark);
            let theme = match self.ui.dark {
                Some(true) => egui::SystemTheme::Dark,
                Some(false) => egui::SystemTheme::Light,
                None => egui::SystemTheme::SystemDefault,
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(theme));
        }
        self.shortcuts(&ctx);

        let mut acts: Vec<Action> = Vec::new();
        let t = self.ui.tokens;
        if let Some(key) = self.ui.page_message.clone() {
            self.message_page(ui, &key, &mut acts);
            if let Some(mut d) = self.ui.dialog.take() {
                let keep = dialogs::show(&ctx, &mut d, &self.session, &t, &mut acts);
                if keep && self.ui.dialog.is_none() {
                    self.ui.dialog = Some(d);
                }
            }
            self.toasts(&ctx);
            for a in acts {
                self.act(a);
            }
            self.flush_jobs();
            return;
        }
        egui::Panel::top("topbar").exact_size(40.0).frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(10, 5))).show(ui, |ui| {
            self.top_bar(ui, &mut acts);
        });
        egui::Panel::bottom("status").exact_size(26.0).frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(10, 3))).show(
            ui,
            |ui| {
                self.status_bar(ui);
            },
        );
        // The modules down the window's edge, beside the ribbon, as Outlook's (the other edge
        // for right-to-left languages).
        if self.ui.backstage.is_none() && !self.session.accounts.is_empty() {
            let rail = if i18n::current().rtl() { egui::Panel::right("modules") } else { egui::Panel::left("modules") };
            rail.resizable(false)
                .exact_size(48.0)
                .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(4, 8)).stroke(egui::Stroke::new(1.0, t.border)))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| module_rail(ui, &t, self.session.module, &mut acts));
                });
        }
        if self.ui.backstage.is_none() {
            egui::Panel::top("ribbon")
                .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(8, 4)).stroke(egui::Stroke::new(1.0, t.border)))
                .show(ui, |ui| {
                    // Too wide for the window: its last buttons lose their labels, as few as it
                    // takes (worked out from this frame for the next).
                    // One height on every tab: switching tabs doesn't nudge the window below.
                    ui.set_min_height(RIBBON_H);
                    let id = widgets::ribbon_fit_id();
                    let start =
                        widgets::RibbonFit { active: true, icons_from: self.ui.ribbon_icons_from, right: ui.max_rect().left(), ..Default::default() };
                    ui.ctx().data_mut(|d| d.insert_temp(id, start));
                    self.ribbon(ui, &mut acts);
                    let fit = ui.ctx().data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<widgets::RibbonFit>(id)));
                    self.ui.ribbon_rect = Some(ui.min_rect());
                    let from = fit.fit(ui.max_rect().right());
                    if from != self.ui.ribbon_icons_from.min(fit.count) {
                        self.ui.ribbon_icons_from = from;
                        ui.ctx().request_repaint();
                    }
                });
        }

        if self.ui.backstage.is_some() {
            backstage::show(&self.session, &mut self.ui, ui, &mut acts);
        } else if self.session.accounts.is_empty() {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| {
                widgets::welcome(ui, &t, self.session.jmap_only, &mut acts);
            });
        } else {
            let module = self.session.module;
            // Right-to-left languages mirror the window: folders on the right, then the list,
            // the reading pane on the left.
            let rtl = i18n::current().rtl();
            let start = |id: &'static str| if rtl { egui::Panel::right(id) } else { egui::Panel::left(id) };
            // Minimized: a narrow strip with the way back, as Outlook's (the modules are on the
            // rail beside the ribbon). Between the two, the pane slides (or changes at once, with
            // animations off).
            let mut expanded = !self.session.prefs.folder_pane_minimized;
            let strip =
                start("nav-min").resizable(false).exact_size(32.0).frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(4, 8)));
            let full = start("nav")
                .resizable(true)
                .default_size(if module == Module::Mail { 230.0 } else { 260.0 })
                .size_range(170.0..=380.0)
                .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(6, 8)));
            egui::Panel::show_switched(ui, &mut expanded, strip, full, |ui, wide| {
                if !wide {
                    ui.vertical_centered(|ui| {
                        let chevron = if rtl { Icon::ChevronLeft } else { Icon::ChevronRight };
                        if widgets::tiny_icon_button(ui, chevron, "Show the folder pane", &t).clicked() {
                            acts.push(Action::Run("view.folderPane", json!({"minimized": false})));
                        }
                    });
                    return;
                }
                // Minimize, as Outlook's ‹ at the top of its folder pane.
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    let chevron = if rtl { Icon::ChevronRight } else { Icon::ChevronLeft };
                    if widgets::tiny_icon_button(ui, chevron, "Minimize the folder pane", &t).clicked() {
                        acts.push(Action::Run("view.folderPane", json!({"minimized": true})));
                    }
                });
                match module {
                    Module::Mail => nav::show(&self.session, &t, ui, &mut acts),
                    Module::Calendar => calendar::nav(&self.session, &mut self.ui, ui, &mut acts),
                    Module::People => people::nav(&self.session, &mut self.ui, ui, &mut acts),
                    Module::Notes => notes::nav(&self.session, &mut self.ui, ui, &mut acts),
                }
            });
            // Dragged narrower than it goes: minimized, as the button does.
            if expanded == self.session.prefs.folder_pane_minimized {
                acts.push(Action::Run("view.folderPane", json!({"minimized": !expanded})));
            }
            let pane = self.session.prefs.reading_pane;
            let list_frame = egui::Frame::new().fill(t.surface).stroke(egui::Stroke::new(1.0, t.border));
            // Beside the mail, the month and the coming appointments (Outlook's To-Do bar), on
            // the far side, where the window has room for it.
            let room = if pane == ReadingPane::Right { 1100.0 } else { 800.0 };
            if module == Module::Mail && self.session.prefs.todo_bar && ui.available_width() > room {
                let end = if rtl { egui::Panel::left("todo") } else { egui::Panel::right("todo") };
                end.resizable(true)
                    .default_size(230.0)
                    .size_range(190.0..=320.0)
                    .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(10, 8)))
                    .show(ui, |ui| calendar::todo_bar(&self.session, &mut self.ui, ui, &mut acts));
            }
            match (module, pane) {
                (Module::Calendar, _) => {
                    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                        calendar::show(&self.session, &mut self.ui, ui, &mut acts);
                    });
                }
                (Module::People, _) => {
                    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                        people::show(&self.session, &mut self.ui, ui, &mut acts);
                    });
                }
                (Module::Notes, _) => {
                    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                        notes::show(&self.session, &mut self.ui, ui, &mut acts);
                    });
                }
                (Module::Mail, pane) => match pane {
                    ReadingPane::Right => {
                        start("list").resizable(true).default_size(390.0).size_range(280.0..=720.0).frame(list_frame).show(ui, |ui| {
                            list::show(&self.session, &mut self.ui, ui, &mut acts);
                        });
                        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                            reading::show(&mut self.session, &mut self.ui, ui, &mut acts);
                        });
                    }
                    ReadingPane::Bottom => {
                        egui::Panel::top("list").resizable(true).default_size(300.0).size_range(140.0..=900.0).frame(list_frame).show(ui, |ui| {
                            list::show(&self.session, &mut self.ui, ui, &mut acts);
                        });
                        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                            reading::show(&mut self.session, &mut self.ui, ui, &mut acts);
                        });
                    }
                    ReadingPane::Off => {
                        egui::CentralPanel::default().frame(list_frame).show(ui, |ui| {
                            if self.session.compose.is_some() {
                                reading::show(&mut self.session, &mut self.ui, ui, &mut acts);
                            } else {
                                list::show(&self.session, &mut self.ui, ui, &mut acts);
                            }
                        });
                    }
                },
            }
        }

        self.message_windows(&ctx, &mut acts);
        self.compose_windows(&ctx, &mut acts);

        // The address book reads the session's people.
        if let Some(Dialog::SelectNames(f)) = self.ui.dialog.as_mut()
            && f.stale
        {
            // Opened: with the address book chosen to open with (one gone is everyone).
            if f.source.is_empty() {
                f.books = self.session.address_books();
                f.default_source = self.session.prefs.address_book.clone();
                let known = f.default_source == "people" || f.books.iter().any(|(b, _, _)| *b == f.default_source);
                f.source = if known { f.default_source.clone() } else { "all".into() };
            }
            f.list = self.session.address_book(&f.source, &f.query);
            f.stale = false;
        }
        // A message window's own dialogs show in that window.
        let in_window = self.session.compose.as_ref().is_some_and(|c| !compose_inline(c, &ctx));
        if let Some(mut d) = self.ui.dialog.take() {
            if in_window && d.for_compose() {
                self.ui.dialog = Some(d);
            } else {
                let keep = dialogs::show(&ctx, &mut d, &self.session, &t, &mut acts);
                // A signature written in HTML: how it looks, under the dialog.
                if let Dialog::Signature(f) = &d
                    && f.code
                    && !f.html_code.trim().is_empty()
                {
                    let html = quatrelle_engine::mime::layout::clean_html(&f.html_code);
                    let screen = ctx.content_rect();
                    egui::Area::new(egui::Id::unique("signature-preview"))
                        .order(egui::Order::Tooltip)
                        .pivot(egui::Align2::CENTER_BOTTOM)
                        .fixed_pos(egui::pos2(screen.center().x, screen.max.y - 16.0))
                        .show(&ctx, |ui| {
                            egui::Frame::new()
                                .fill(t.surface)
                                .stroke(egui::Stroke::new(1.0, t.border))
                                .corner_radius(8)
                                .inner_margin(egui::Margin::same(12))
                                .show(ui, |ui| {
                                    ui.set_width(560.0);
                                    ui.label(RichText::new(tl!("Preview (pictures on the web aren't loaded here)")).color(t.dim).small());
                                    ui.add_space(6.0);
                                    reading::html_preview(&self.session, &mut self.ui, ui, &html);
                                });
                        });
                }
                if keep && self.ui.dialog.is_none() {
                    self.ui.dialog = Some(d);
                }
            }
        }
        self.toasts(&ctx);
        // Calendar reminders' window (the desktop is told in `logic`).
        reminders::show(&ctx, &mut self.ui.reminders, &self.session, &t, &mut acts);
        if reminders::showing(&self.ui.reminders) {
            ctx.request_repaint_after(std::time::Duration::from_secs(15));
        }
        // Video thumbnails the cards asked for.
        for (path, frame) in self.ui.video_frames.iter_mut().filter(|(_, f)| **f == VideoFrame::Pending) {
            *frame = self.services.video_frame(path);
        }

        for a in acts {
            self.act(a);
        }
        // A big mailbox's search index goes on a slice at a time: keep going while there's more,
        // one slice per frame, with the window drawing between them (as Outlook's search does).
        if self.session.search_pending
            && let Some((q, _)) = self.session.search.clone()
        {
            self.run("search.run", json!({"query": q, "index": SEARCH_INDEX_BUDGET}));
            ctx.request_repaint();
        }
        self.flush_jobs();
        if std::mem::take(&mut self.ui.focus_main) {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        }
        let busy = self.session.accounts.iter().any(|a| matches!(a.status, SyncStatus::Connecting | SyncStatus::Syncing { .. }));
        ctx.request_repaint_after(std::time::Duration::from_secs(if busy || self.ui.toast.is_some() { 1 } else { 20 }));
    }
}
