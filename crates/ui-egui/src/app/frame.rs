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
        // An update ready while the window is minimized or behind others: the system says so
        // (and a close below says what becomes of it).
        self.follow_update(ctx);
        // A close, decided here too: `ui` doesn't run while the window is minimized. In a pass
        // that draws, `ui` then leaves it be (egui gives `logic` alone the last pass's input time).
        self.ui.close_checked = Some(ctx.input(|i| i.time));
        self.decide_close(ctx);
        self.tell_textures(ctx);
        if self.pump() {
            ctx.request_repaint();
        }
        self.announce(ctx);
        // Reminders answered from their notification: as from the window's buttons; clicked:
        // the window comes forward, the Reminders window with it. (Before the due list is
        // worked out again below, so a snooze takes effect in this very frame.)
        for (key, what) in self.services.take_reminder_actions() {
            // Answered already (in the window, or on another notification for it), or its time
            // has passed: its buttons have nothing left to do, and nothing to complain about.
            let due = || self.session.due_reminders(self.session.clock.now).iter().any(|r| r.key == key);
            if matches!(what, ReminderAction::Snooze | ReminderAction::Dismiss) && !due() {
                log::info!("reminder {key}: answered from a notification it no longer had");
                continue;
            }
            match what {
                ReminderAction::Snooze => self.run("reminders.snooze", json!({"key": key, "minutes": 5})),
                ReminderAction::Dismiss => self.run("reminders.dismiss", json!({"key": key})),
                ReminderAction::Open => {
                    self.ui.reminders.closed.remove(&key);
                    self.ui.reminders.selected = Some(key);
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Minimized(false));
                    self.ui.focus_main = true;
                }
            }
            ctx.request_repaint();
        }
        // Calendar reminders: the desktop told of each as it comes, whatever the window shows,
        // with Snooze and Dismiss to answer it from there.
        for r in reminders::refresh(&mut self.ui.reminders, &self.session, false) {
            let start = quatrelle_engine::model::dates::civil(r.start, &self.session.clock);
            let when = if r.all_day { String::new() } else { format!("{:02}:{:02}", start.hour, start.minute) };
            let body = if r.location.is_empty() { when } else { format!("{when} · {}", r.location) };
            let (snooze, dismiss) = (tl!("Snooze 5 minutes").to_string(), tl!("Dismiss").to_string());
            self.services.notify_reminder(&r.key, &r.summary, &body, &snooze, &dismiss);
        }
        // Answered or past: their notifications go, so none is left with buttons that fail.
        for key in std::mem::take(&mut self.ui.reminders.gone) {
            self.services.withdraw_reminder(&key);
        }
        // Minimized or covered, the window draws nothing, so the Reminders window — a window of
        // its own, on top of the others — couldn't open: a frame is asked for while something
        // is due, and once that window is up eframe keeps drawing for it.
        if self.session.prefs.reminders_on_top && reminders::showing(&self.ui.reminders) && ctx.input(|i| i.viewport().visible() == Some(false)) {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::RequestPaintWhileHidden);
        }
        self.flush_jobs();
        // Woken by the workers when they have news; this is for the clock (reminders, snoozed
        // mail coming back) when nothing else happens.
        ctx.request_repaint_after(std::time::Duration::from_secs(15));
    }

    /// Where the platform has a quit of its own (macOS: `terminate:`, `quit` returning true), a
    /// close of the main window that nothing held open goes that way instead: call it last in
    /// `logic`. Leaving a Mac by closing the eframe window can crash in AppKit's Touch Bar code
    /// once the window is gone, and `terminate:` ends the process before that runs (craft-libs'
    /// quit crate). Whether it took the close over.
    pub fn quit_instead_of_close(ctx: &egui::Context, quit: impl FnOnce() -> bool) -> bool {
        if ctx.viewport_id() != egui::ViewportId::ROOT || !ctx.input(|i| i.viewport().close_requested()) {
            return false;
        }
        // A question (save what's being written?) holds the window open.
        if ctx.viewport(|v| v.commands.contains(&egui::ViewportCommand::CancelClose)) {
            return false;
        }
        if !quit() {
            return false;
        }
        // The window stays for the quit to close.
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        true
    }

    /// Closing with messages being written, online-only accounts' unsent mail or changes held in
    /// memory, or an update waiting: ask first. Decided in [`MailApp::logic`], which eframe runs
    /// with the window minimized too, and closes the window after unless it cancels (craft-libs'
    /// quit crate found this: a close from the taskbar lost the messages being written).
    fn decide_close(&mut self, ctx: &egui::Context) {
        self.forget_cancelled_close();
        // Closing with online-only accounts' unsent mail or changes held in memory: ask first.
        if self.ui.close_now {
            self.ui.close_now = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if ctx.input(|i| i.viewport().close_requested()) {
            // Messages being written: kept as drafts if wanted, as Outlook asks; never lost.
            let written = self.session.all_composes().filter(|c| self.session.written(c) && !c.sending).count();
            let (unsent, changes) = self.session.lost_on_close();
            let asked = if self.ui.closing_anyway {
                false
            } else if written > 0 {
                self.ui.dialog = Some(Dialog::SaveOnClose(written));
                self.ui.dialog_for = None;
                true
            } else if unsent + changes > 0 {
                self.ui.dialog = Some(Dialog::LostOnClose { unsent, changes });
                true
            } else if self.ask_about_update() {
                // An update about to be installed, or still downloading: said first.
                self.ask_update_on_close();
                true
            } else {
                false
            };
            if asked {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.ui.close_asked = true;
                // Closed from the taskbar or the Dock while minimized: the window comes back to ask.
                if ctx.input(|i| i.viewport().minimized == Some(true)) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            } else if self.ui.restart_wanted {
                // Restart now, and nothing stopped the close: the new version opens once installed.
                self.services.restart_to_update();
            }
        }
    }

    /// The pictures the window keeps on the graphics card (the glyphs, mail pictures, icons), in
    /// the log every ten minutes, beside the memory the app logs.
    fn tell_textures(&mut self, ctx: &egui::Context) {
        let now = self.session.clock.now;
        // The first a minute after the start, once the window has drawn what it shows.
        if self.ui.textures_told == 0 {
            self.ui.textures_told = now.saturating_sub(540);
        }
        if now.saturating_sub(self.ui.textures_told) < 600 {
            return;
        }
        self.ui.textures_told = now;
        let tm = ctx.tex_manager();
        let tm = tm.read();
        let (mut count, mut bytes, mut biggest) = (0usize, 0usize, (0usize, [0usize; 2]));
        for (_, meta) in tm.allocated() {
            count += 1;
            bytes = bytes.saturating_add(meta.bytes_used());
            if meta.bytes_used() > biggest.0 {
                biggest = (meta.bytes_used(), meta.size);
            }
        }
        log::info!("memory: {count} pictures on the graphics card, {} MB (the biggest {}×{})", bytes >> 20, biggest.1[0], biggest.1[1]);
    }

    /// The address book (recipient suggestions, Select Names, the Focused Inbox), built the way
    /// the search index is: a slice per frame, the window drawing between slices. Asked for
    /// while a message is being written, ahead of the first name typed; kept going once started.
    /// Built whole at that first name, it read every folder of a big mailbox from disk at once
    /// and froze the window for seconds.
    fn build_contacts(&mut self, ctx: &egui::Context) {
        if (self.session.all_composes().next().is_some() || self.session.contacts_pending()) && !self.session.contacts_step(CONTACTS_BUDGET) {
            ctx.request_repaint();
        }
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
        // Ctrl+Q is Mark as read, as Outlook's: egui's own Ctrl+Q closed the window too. On a
        // Mac, Cmd+Q quits, as every Mac app does.
        if !cfg!(target_os = "macos") && ctx.options(|o| !o.quit_shortcuts.is_empty()) {
            ctx.options_mut(|o| o.quit_shortcuts.clear());
        }
        if !std::mem::take(&mut self.ui.logic_ran) {
            self.logic(&ctx);
            self.ui.logic_ran = false;
        }
        self.settle_composes(&ctx);
        if self.ui.close_checked != Some(ctx.input(|i| i.time)) {
            self.decide_close(&ctx);
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
            self.build_contacts(&ctx);
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
                    ui.vertical_centered(|ui| super::bars::module_rail(ui, &t, self.session.module, &mut acts));
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
                    // And its groups of large buttons: the last ones drawn compact (small
                    // buttons stacked), as many as it takes, worked out the same way — per
                    // tab, as each has its own groups.
                    let key = format!("{:?}/{:?}", self.session.module, self.ui.tab);
                    let groups = self.ui.ribbon_groups.get(&key).cloned().unwrap_or_default();
                    let gid = widgets::group_fit_id();
                    let room_right = ui.max_rect().right();
                    ui.ctx().data_mut(|d| d.insert_temp(gid, widgets::GroupFit::start(groups.compact_from, groups.columns)));
                    self.ribbon(ui, &mut acts);
                    let fit = ui.ctx().data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<widgets::RibbonFit>(id)));
                    let gfit = ui.ctx().data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<widgets::GroupFit>(gid)));
                    self.ui.ribbon_rect = Some(ui.min_rect());
                    let from = fit.fit(room_right);
                    if from != self.ui.ribbon_icons_from.min(fit.count) {
                        self.ui.ribbon_icons_from = from;
                        ui.ctx().request_repaint();
                    }
                    let next = RibbonGroups {
                        compact_from: gfit.fit(room_right),
                        columns: gfit.groups.iter().map(|g| g.2.clone()).collect(),
                        overflow: gfit.right - room_right,
                    };
                    if next.compact_from != self.ui.ribbon_groups.get(&key).map_or(usize::MAX, |g| g.compact_from).min(gfit.groups.len())
                        || next.columns != self.ui.ribbon_groups.get(&key).map(|g| g.columns.clone()).unwrap_or_default()
                    {
                        ui.ctx().request_repaint();
                    }
                    self.ui.ribbon_groups.insert(key, next);
                });
            // A newer Quatrelle ready to install (or a copy left from before): a bar of its own.
            if self.update_bar_wanted() {
                egui::Panel::top("update-bar")
                    .frame(egui::Frame::new().fill(t.accent_soft).inner_margin(Margin::symmetric(12, 6)).stroke(egui::Stroke::new(1.0, t.border)))
                    .show(ui, |ui| self.update_bar(ui, &mut acts));
            }
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
                    Module::Tasks => tasks::nav(&self.session, &mut self.ui, ui, &mut acts),
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
                (Module::Tasks, _) => {
                    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                        tasks::show(&self.session, &mut self.ui, ui, &mut acts);
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
        self.event_windows(&ctx, &mut acts);
        self.compacting_window(&ctx);

        // The address book reads the session's people: what is known now, listed again as the
        // book fills in (built a slice per frame, below).
        if let Some(Dialog::SelectNames(f)) = self.ui.dialog.as_mut() {
            f.search.refresh(&mut self.session);
        }
        // Attach Item's list: the items of the kind and folder chosen, matching the words typed.
        if let Some(Dialog::AttachItem(f)) = self.ui.dialog.as_mut()
            && f.stale
        {
            f.rows = self.session.item_choices(f.kind, &f.account, &f.folder, &f.query);
            f.picked.retain(|id| f.rows.iter().any(|r| r.id == *id));
            f.stale = false;
            ctx.request_repaint();
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
                                    let pictures = self.session.signature_pictures(&f.account);
                                    reading::html_preview(&self.session, &mut self.ui, ui, &html, &pictures);
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
        self.build_contacts(&ctx);
        // The rows drawn without a preview: a few worked out, drawn in the next frame (which
        // asks for the rest).
        let previews = std::mem::take(&mut self.ui.want_previews);
        if !previews.is_empty() {
            self.session.fill_previews(&previews);
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
