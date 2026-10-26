//! The ribbon: its tabs and their buttons.

use crate::*;

impl MailApp {
    pub(crate) fn ribbon(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        let s = &self.session;
        // The Search tab goes when the search does (and the box isn't being typed in).
        let on_ribbon = self.ui.ribbon_rect.zip(ui.ctx().pointer_latest_pos()).is_some_and(|(r, p)| r.contains(p));
        if self.ui.tab == RibbonTab::Search && s.search.is_none() && !self.ui.search_focused && !on_ribbon {
            self.ui.tab = RibbonTab::Home;
        }
        let searching = self.ui.tab == RibbonTab::Search || s.search.is_some();
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new(tl!("File")).color(t.dim).font(theme::regular(13.5))).frame_when_inactive(false)).clicked() {
                acts.push(Action::Backstage(backstage::Page::Info));
            }
            let tabs =
                [(RibbonTab::Home, "Home"), (RibbonTab::SendReceive, "Send / Receive"), (RibbonTab::View, "View"), (RibbonTab::Search, "Search")];
            for (tab, label) in tabs.into_iter().filter(|(tab, _)| *tab != RibbonTab::Search || searching) {
                let on = self.ui.tab == tab;
                let r = ui.add(
                    egui::Button::new(RichText::new(tl!(label)).color(if on { t.accent } else { t.dim }).font(if on {
                        theme::bold(13.5)
                    } else {
                        theme::regular(13.5)
                    }))
                    .frame_when_inactive(false),
                );
                if on {
                    // The underline glides from the tab left to the one chosen.
                    let rect = r.rect;
                    let ctx = ui.ctx().clone();
                    let left = motion::glide(&ctx, egui::Id::unique("ribbon-tab-left"), rect.left() + 6.0, motion::NORMAL);
                    let right = motion::glide(&ctx, egui::Id::unique("ribbon-tab-right"), rect.right() - 6.0, motion::NORMAL);
                    ui.painter().line_segment(
                        [egui::pos2(left, rect.bottom() + 1.0), egui::pos2(right, rect.bottom() + 1.0)],
                        egui::Stroke::new(2.0, t.accent),
                    );
                }
                if r.clicked() {
                    self.ui.tab = tab;
                    // A tab clicked while the ribbon is folded away brings it back.
                    self.ui.ribbon_collapsed = false;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (icon, tip) = if self.ui.ribbon_collapsed {
                    (Icon::ChevronDown, "Show the ribbon (Ctrl+F1)")
                } else {
                    (Icon::ChevronUp, "Collapse the ribbon (Ctrl+F1)")
                };
                if widgets::tiny_icon_button(ui, icon, tip, &t).clicked() {
                    self.ui.ribbon_collapsed = !self.ui.ribbon_collapsed;
                }
            });
        });
        if self.ui.ribbon_collapsed {
            return;
        }
        ui.add_space(2.0);
        let enabled = |id: &str| s.registry().get(id).is_some_and(|c| (c.enabled)(s).is_none());
        if s.module != Module::Mail {
            ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let mut groups = widgets::Groups::new(ui);
                if s.module == Module::Calendar {
                    if widgets::big_accent(ui, Icon::Calendar, "New event", &t, !s.accounts.is_empty()).clicked() {
                        acts.push(Action::Dialog(Dialog::new_event(s, None)));
                    }
                    groups.end(ui, &t, "New");
                    if widgets::big_button(ui, Icon::CalendarToday, "Today", &t, true).clicked() {
                        acts.push(Action::Run("calendar.goto", json!({"date": "today"})));
                    }
                    groups.end(ui, &t, "Go to");
                    for (mode, icon, label, code) in [
                        (CalMode::Day, Icon::CalendarDay, "Day", "day"),
                        (CalMode::WorkWeek, Icon::CalendarWorkWeek, "Work week", "workWeek"),
                        (CalMode::Week, Icon::CalendarWeek, "Week", "week"),
                        (CalMode::Month, Icon::CalendarMonth, "Month", "month"),
                    ] {
                        // The view shown stays lit, as Outlook's.
                        let r = widgets::big_toggle(ui, icon, label, &t, s.cal_mode == mode);
                        if r.clicked() {
                            acts.push(Action::Run("calendar.view", json!({"mode": code})));
                        }
                    }
                    // Each calendar in a column of its own (Outlook's side by side), or together;
                    // and Outlook's Open Calendar: someone's calendar shared with an Exchange account.
                    let several = s.pim.values().map(|p| p.calendars.len()).sum::<usize>() > 1;
                    let exchange = s.accounts.iter().any(|a| a.config.graph.is_some() || a.config.ews.is_some());
                    if several || exchange {
                        groups.end(ui, &t, "Arrange");
                        if exchange
                            && widgets::big_button(ui, Icon::CalendarAdd, "Open calendar", &t, true)
                                .on_hover_text(tl!("Open someone's calendar shared with you in Exchange"))
                                .clicked()
                        {
                            acts.push(Action::Dialog(Dialog::OpenSharedCalendar { email: String::new(), error: None }));
                        }
                        if several && widgets::big_toggle(ui, Icon::Calendar, "Side by side", &t, s.prefs.calendars_side_by_side).clicked() {
                            acts.push(Action::Run("calendar.sideBySide", json!({})));
                        }
                        groups.last(ui, &t, "Manage calendars");
                    } else {
                        groups.last(ui, &t, "Arrange");
                    }
                } else if s.module == Module::Tasks {
                    tasks::ribbon(s, &mut self.ui, ui, &mut groups, acts);
                } else if s.module == Module::Notes {
                    if widgets::big_accent(ui, Icon::Note, "New note", &t, !s.accounts.is_empty()).clicked() {
                        notes::new_note(&mut self.ui, acts);
                    }
                    groups.end(ui, &t, "New");
                    let editing = self.ui.note_edit.as_ref().map(|(u, _)| u.clone());
                    if widgets::big_button(ui, Icon::Delete, "Delete", &t, editing.is_some()).clicked()
                        && let Some(uid) = editing.clone()
                    {
                        self.ui.note_edit = None;
                        acts.push(Action::Run("note.delete", json!({"uid": uid})));
                    }
                    // The note's colour, as Outlook's Categorize on a note.
                    let r = widgets::big_button(ui, Icon::Tag, "Color", &t, editing.is_some());
                    egui::Popup::menu(&r).show(|ui| {
                        for (i, name) in notes::NAMES.iter().enumerate() {
                            if ui.button(tl!(name)).clicked()
                                && let Some(uid) = &editing
                            {
                                acts.push(Action::Run("note.update", json!({"uid": uid, "color": i})));
                                ui.close();
                            }
                        }
                    });
                    // How they're listed: the last changed first, A to Z, or by colour.
                    groups.end(ui, &t, "Actions");
                    let r = widgets::big_button(ui, Icon::ListBullet, "Arrange by", &t, true);
                    egui::Popup::menu(&r).show(|ui| {
                        use quatrelle_engine::NotesOrder as O;
                        for (order, label, code) in
                            [(O::Modified, "Last changed", "modified"), (O::Title, "Title (A to Z)", "title"), (O::Color, "Color", "color")]
                        {
                            if ui.radio(s.prefs.notes_order == order, tl!(label)).clicked() {
                                acts.push(Action::Run("note.arrange", json!({"by": code})));
                                ui.close();
                            }
                        }
                    });
                    groups.last(ui, &t, "Arrange");
                } else {
                    if widgets::big_accent(ui, Icon::Person, "New contact", &t, !s.accounts.is_empty()).clicked() {
                        acts.push(Action::Dialog(Dialog::new_contact()));
                    }
                    groups.end(ui, &t, "New");
                    let picked = self.ui.person.as_deref().and_then(|u| s.find_person(u));
                    // Someone in a shared (read-only) address book can't be changed here.
                    let picked = picked.filter(|p| !s.person_read_only(&p.card.uid));
                    if widgets::big_button(ui, Icon::Drafts, "Edit", &t, picked.is_some()).clicked()
                        && let Some(p) = &picked
                    {
                        acts.push(Action::Dialog(Dialog::edit_contact(&p.card)));
                    }
                    if widgets::big_button(ui, Icon::Delete, "Delete", &t, picked.is_some()).clicked()
                        && let Some(p) = &picked
                    {
                        acts.push(Action::Dialog(Dialog::ConfirmDeleteContact { uid: p.card.uid.clone(), name: p.card.display() }));
                    }
                    // Outlook's Change View: the list with a card, business cards, a phone list.
                    groups.end(ui, &t, "Actions");
                    for v in people::PeopleView::ALL {
                        if widgets::big_toggle(ui, v.icon(), v.label(), &t, self.ui.people_view == v).clicked() {
                            self.ui.people_view = v;
                        }
                    }
                    // The list's columns (also from its header's right-click).
                    if self.ui.people_view == people::PeopleView::Phone {
                        let r = widgets::big_button(ui, Icon::Layout, "Columns", &t, true);
                        egui::Popup::menu(&r)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .show(|ui| people::column_menu(ui, &mut self.ui.people_list));
                    }
                    groups.last(ui, &t, "Current view");
                }
            });
            return;
        }
        let confirm_delete = self.needs_delete_confirm();
        let ust = &mut self.ui;
        // A narrow window scrolls the ribbon sideways rather than cutting buttons off; each tab
        // keeps its own place (one shared place opened View scrolled as far as Home was, which
        // gridcraft's ribbon found).
        egui::ScrollArea::horizontal().id_salt(("ribbon", format!("{:?}", ust.tab))).show(ui, |ui| {
            ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let mut groups = widgets::Groups::new(ui);
                match ust.tab {
                    RibbonTab::Home => {
                        if widgets::big_accent(ui, Icon::NewMail, "New mail", &t, enabled("mail.new"))
                            .on_hover_text(tl!("New mail (Ctrl+N)"))
                            .clicked()
                        {
                            acts.push(Action::Run("mail.new", json!({})));
                        }
                        // New items of other kinds, as Outlook's New Email drop-down.
                        let r = widgets::big_more(ui, "New items", &t);
                        egui::Popup::menu(&r).show(|ui| {
                            if ui.button(tl!("Email message")).clicked() {
                                acts.push(Action::Run("mail.new", json!({})));
                                ui.close();
                            }
                            if ui.button(tl!("Event")).clicked() {
                                acts.push(Action::Dialog(Dialog::new_event(s, None)));
                                ui.close();
                            }
                            if ui.button(tl!("Contact")).clicked() {
                                acts.push(Action::Dialog(Dialog::new_contact()));
                                ui.close();
                            }
                        });
                        groups.end(ui, &t, "New");
                        if widgets::big_button(ui, Icon::Delete, "Delete", &t, enabled("mail.delete")).on_hover_text(tl!("Delete (Del)")).clicked() {
                            if confirm_delete {
                                acts.push(Action::Dialog(Dialog::ConfirmDelete { count: s.selection.len() }));
                            } else {
                                acts.push(Action::Run("mail.delete", json!({})));
                            }
                        }
                        // Ignore the conversation, as Outlook's Delete drop-down.
                        let r = widgets::big_more(ui, "More delete options", &t);
                        egui::Popup::menu(&r).show(|ui| {
                            let ignoring = !s.selection.is_empty() && s.selected().iter().all(|m| s.is_ignored(m));
                            let label = if ignoring { "Stop ignoring the conversation" } else { "Ignore the conversation" };
                            if ui
                                .add_enabled(enabled("mail.ignore"), egui::Button::new(tl!(label)))
                                .on_hover_text(tl!("Its messages, and replies that arrive later, go to Deleted Items"))
                                .clicked()
                            {
                                acts.push(Action::Run("mail.ignore", json!({})));
                                ui.close();
                            }
                            // Clean Up: what a later reply quotes in full goes to Deleted Items.
                            ui.separator();
                            let tip = tl!("Messages quoted in full by a later reply go to Deleted Items");
                            if ui.add_enabled(enabled("mail.cleanUp"), egui::Button::new(tl!("Clean up conversation"))).on_hover_text(tip).clicked() {
                                acts.push(Action::Run("mail.cleanUp", json!({})));
                                ui.close();
                            }
                            if ui.add_enabled(enabled("folder.cleanUp"), egui::Button::new(tl!("Clean up folder"))).on_hover_text(tip).clicked() {
                                acts.push(Action::Run("folder.cleanUp", json!({})));
                                ui.close();
                            }
                        });
                        if widgets::big_button(ui, Icon::Archive, "Archive", &t, enabled("mail.archive"))
                            .on_hover_text(tl!("Archive (Backspace)"))
                            .clicked()
                        {
                            acts.push(Action::Run("mail.archive", json!({})));
                        }
                        let in_junk = s.view_role() == Some(quatrelle_engine::model::FolderRole::Junk);
                        let (jid, jl) = if in_junk { ("mail.notJunk", "Not junk") } else { ("mail.junk", "Report junk") };
                        if widgets::big_button(ui, Icon::Junk, jl, &t, enabled(jid)).clicked() {
                            acts.push(Action::Run(jid, json!({})));
                        }
                        // The Junk drop-down: the sender's lists, as Outlook's.
                        let r = widgets::big_more(ui, "Junk options", &t);
                        egui::Popup::menu(&r).show(|ui| junk_menu(ui, s, acts));
                        groups.end(ui, &t, "Delete");
                        if widgets::big_button(ui, Icon::Move, "Move to", &t, enabled("mail.move"))
                            .on_hover_text(tl!("Move to a folder (Ctrl+Shift+V)"))
                            .clicked()
                        {
                            acts.push(Action::Dialog(Dialog::move_to()));
                        }
                        groups.end(ui, &t, "Move");
                        for (id, icon, label, tip) in [
                            ("mail.reply", Icon::Reply, "Reply", "Reply (Ctrl+R)"),
                            ("mail.replyAll", Icon::ReplyAll, "Reply all", "Reply all (Ctrl+Shift+R)"),
                            ("mail.forward", Icon::Forward, "Forward", "Forward (Ctrl+F)"),
                        ] {
                            if widgets::big_button(ui, icon, label, &t, enabled(id)).on_hover_text(tl!(tip)).clicked() {
                                acts.push(Action::Run(id, json!({})));
                            }
                        }
                        groups.end(ui, &t, "Respond");
                        // Quick Steps: each done to the selection in one click (Ctrl+Shift+1…9).
                        let r = widgets::big_button(ui, Icon::Steps, "Quick steps", &t, true);
                        egui::Popup::menu(&r).show(|ui| quick_steps_menu(ui, s, acts));
                        groups.end(ui, &t, "Quick steps");
                        let first_unread = s.selected().first().is_some_and(|m| !m.flags.seen);
                        let (ricon, rlabel) = if first_unread { (Icon::Read, "Mark read") } else { (Icon::Unread, "Mark unread") };
                        if widgets::big_button(ui, ricon, rlabel, &t, enabled("mail.toggleRead"))
                            .on_hover_text("Read / Unread (Ctrl+Q / Ctrl+U)")
                            .clicked()
                        {
                            acts.push(Action::Run("mail.toggleRead", json!({})));
                        }
                        if widgets::big_button(ui, Icon::Flag, "Flag", &t, enabled("mail.flag"))
                            .on_hover_text(tl!("Flag / Unflag (Insert)"))
                            .clicked()
                        {
                            acts.push(Action::Run("mail.flag", json!({})));
                        }
                        // Follow Up: when it's due, as Outlook's flag drop-down.
                        let r = widgets::big_more(ui, "Follow up", &t);
                        egui::Popup::menu(&r).show(|ui| organize::follow_up_menu(ui, s, &t, acts));
                        let sel = !s.selection.is_empty();
                        let r = widgets::big_button(ui, Icon::Tag, "Categorize", &t, sel);
                        egui::Popup::menu(&r).show(|ui| organize::categorize_menu(ui, s, &t, acts));
                        let pinned = s.selected().first().is_some_and(|m| m.flags.pinned());
                        if widgets::big_button(ui, Icon::Pin, if pinned { "Unpin" } else { "Pin" }, &t, sel).clicked() {
                            acts.push(Action::Run("mail.pin", json!({"on": !pinned})));
                        }
                        let r = widgets::big_button(ui, Icon::Clock, "Snooze", &t, sel);
                        egui::Popup::menu(&r).show(|ui| organize::snooze_menu(ui, s, &t, acts));
                        groups.end(ui, &t, "Tags");
                        // Show only some mail, as Outlook's Filter Email.
                        let r = widgets::big_button(ui, Icon::Funnel, "Filter", &t, true);
                        egui::Popup::menu(&r).show(|ui| {
                            for f in quatrelle_engine::ListFilter::EVERY {
                                if ui.selectable_label(s.filter == f, tl!(f.label())).clicked() {
                                    acts.push(Action::Run("list.filter", json!({"value": f.code()})));
                                    ui.close();
                                }
                            }
                        });
                        groups.last(ui, &t, "Find");
                    }
                    RibbonTab::SendReceive => {
                        if widgets::big_button(ui, Icon::Sync, "Send / Receive all folders", &t, enabled("mail.sync")).on_hover_text("F9").clicked() {
                            acts.push(Action::Run("mail.sync", json!({})));
                        }
                        // Outlook's Download All: every message on this computer, to read and
                        // search it away from the server.
                        if widgets::big_button(ui, Icon::Storage, "Download all", &t, enabled("account.downloadAll")).clicked() {
                            acts.push(Action::Run("account.downloadAll", json!({})));
                        }
                        if widgets::big_button(ui, Icon::Folder, "Update folder", &t, enabled("folder.sync")).on_hover_text("Shift+F9").clicked() {
                            acts.push(Action::Run("folder.sync", json!({})));
                        }
                        if widgets::big_button(ui, Icon::Send, "Send all", &t, enabled("outbox.send") && !s.prefs.work_offline)
                            .on_hover_text(tl!("Send what's waiting in the Outbox"))
                            .clicked()
                        {
                            acts.push(Action::Run("outbox.send", json!({})));
                        }
                        // Working offline: no connections; mail you send waits in the Outbox.
                        if ust.has_files {
                            groups.end(ui, &t, "Send & Receive");
                            let off = s.prefs.work_offline;
                            if widgets::big_toggle(ui, Icon::Away, "Work offline", &t, off)
                                .on_hover_text(tl!("Close every connection: changes, mail you send and drafts wait on this computer"))
                                .clicked()
                            {
                                acts.push(Action::Run("mail.workOffline", json!({"on": !off})));
                            }
                            groups.last(ui, &t, "Preferences");
                        } else {
                            groups.last(ui, &t, "Send & Receive");
                        }
                    }
                    RibbonTab::Search => {
                        let query = s.search.as_ref().map(|(q, _)| q.clone()).unwrap_or_else(|| ust.search_text.clone());
                        let current = s.search.as_ref().map_or(s.prefs.search_scope, |(_, scope)| *scope);
                        for pair in [[SearchScope::All, SearchScope::Mailbox], [SearchScope::Current, SearchScope::Subfolders]] {
                            widgets::stack(ui, |ui| {
                                for scope in pair {
                                    if ui.radio(scope == current, scope_label(scope)).clicked() && scope != current {
                                        acts.push(Action::Run("search.run", json!({"query": query, "scope": scope.code()})));
                                    }
                                }
                            });
                        }
                        groups.end(ui, &t, "Look in");
                        // Terms to type a name after: put in the box, ready for it.
                        for (icon, label, term) in
                            [(Icon::Person, "From", "from:"), (Icon::Send, "Sent To", "to:"), (Icon::Read, "Subject", "subject:")]
                        {
                            if widgets::big_button(ui, icon, label, &t, true).clicked() {
                                let text = &mut ust.search_text;
                                if !text.is_empty() && !text.ends_with(' ') {
                                    text.push(' ');
                                }
                                text.push_str(term);
                                ust.focus_search = true;
                                ust.search_cursor_end = true;
                            }
                        }
                        // Terms that narrow at once: on, or off again.
                        for (icon, label, term) in [
                            (Icon::Attach, "Has Attachments", "has:attachment"),
                            (Icon::Unread, "Unread", "is:unread"),
                            (Icon::Tag, "Categorized", "is:categorized"),
                            (Icon::Flag, "Flagged", "is:flagged"),
                            (Icon::Warning, "Important", "is:important"),
                            (Icon::Send, "Awaiting reply", "is:awaiting"),
                        ] {
                            let words: Vec<&str> = query.split_whitespace().collect();
                            let on = words.iter().any(|w| w.eq_ignore_ascii_case(term));
                            if widgets::big_toggle(ui, icon, label, &t, on).clicked() {
                                let q = if on {
                                    words.iter().filter(|w| !w.eq_ignore_ascii_case(term)).copied().collect::<Vec<_>>().join(" ")
                                } else if query.trim().is_empty() {
                                    term.to_string()
                                } else {
                                    format!("{} {term}", query.trim_end())
                                };
                                ust.search_text = q.clone();
                                if q.trim().is_empty() {
                                    acts.push(Action::Run("search.clear", json!({})));
                                } else {
                                    acts.push(Action::Run("search.run", json!({"query": q, "scope": current.code()})));
                                }
                            }
                        }
                        groups.end(ui, &t, "Refine");
                        if widgets::big_button(ui, Icon::Close, "Close Search", &t, true).clicked() {
                            acts.push(Action::Run("search.clear", json!({})));
                            ust.search_text.clear();
                            ust.tab = RibbonTab::Home;
                        }
                        groups.last(ui, &t, "Close");
                    }
                    RibbonTab::View => {
                        // Layout: where the reading pane is, and the folder pane shown or not.
                        let r = widgets::big_button(ui, Icon::Layout, "Reading pane", &t, true);
                        egui::Popup::menu(&r).show(|ui| {
                            for (v, label) in [(ReadingPane::Right, "Right"), (ReadingPane::Bottom, "Bottom"), (ReadingPane::Off, "Off")] {
                                if ui.radio(s.prefs.reading_pane == v, tl!(label)).clicked() {
                                    acts.push(Action::Run("view.readingPane", json!({"value": label.to_lowercase()})));
                                    ui.close();
                                }
                            }
                        });
                        // The list as cards or a table (Automatic: a table when the reading pane
                        // is at the bottom or off, as Outlook switches).
                        let r = widgets::big_button(ui, Icon::ListBullet, "Message list", &t, true);
                        egui::Popup::menu(&r).show(|ui| {
                            use quatrelle_engine::ListLayout as L;
                            for (v, label, tip) in [
                                (L::Auto, "Automatic", "A table when the reading pane is at the bottom or off"),
                                (L::Cards, "Cards", ""),
                                (L::Table, "Table", ""),
                            ] {
                                let r = ui.radio(s.prefs.list_layout == v, tl!(label));
                                let r = if tip.is_empty() { r } else { r.on_hover_text(tl!(tip)) };
                                if r.clicked() {
                                    acts.push(Action::Run("view.listLayout", json!({"value": v.code()})));
                                    ui.close();
                                }
                            }
                        });
                        if widgets::big_toggle(ui, Icon::Folder, "Folder pane", &t, !s.prefs.folder_pane_minimized).clicked() {
                            acts.push(Action::Run("view.folderPane", json!({})));
                        }
                        if widgets::big_toggle(ui, Icon::CalendarToday, "To-Do bar", &t, s.prefs.todo_bar)
                            .on_hover_text(tl!("The month and your coming appointments beside the mail"))
                            .clicked()
                        {
                            acts.push(Action::Run("view.todoBar", json!({})));
                        }
                        groups.end(ui, &t, "Layout");
                        // Messages: small boxes stacked, as Outlook's.
                        widgets::stack(ui, |ui| {
                            for (on, label, tip, id) in [
                                (s.prefs.conversations, "Conversations", "Group messages by conversation", "view.conversations"),
                                (s.prefs.show_preview, "Message preview", "", "view.preview"),
                                (s.prefs.focused_inbox, "Focused Inbox", "Put newsletters and automatic mail in Other", "view.focused"),
                            ] {
                                let mut v = on;
                                let r = ui.checkbox(&mut v, tl!(label));
                                let r = if tip.is_empty() { r } else { r.on_hover_text(tl!(tip)) };
                                if r.clicked() {
                                    acts.push(Action::Run(id, json!({})));
                                }
                            }
                        });
                        widgets::stack(ui, |ui| {
                            let mut v = s.prefs.compact;
                            if ui.checkbox(&mut v, tl!("Compact")).clicked() {
                                acts.push(Action::Run("view.compact", json!({})));
                            }
                        });
                        groups.end(ui, &t, "Messages");
                        widgets::stack(ui, |ui| {
                            let r = ui.button(tf!("Arrange by: {field}", field = tl!(s.prefs.arrange.label())));
                            egui::Popup::menu(&r).show(|ui| list::arrange_menu(ui, s, acts));
                            let mut v = s.prefs.reverse_sort;
                            if ui.checkbox(&mut v, tl!("Reverse sort")).clicked() {
                                acts.push(Action::Run("view.reverseSort", json!({})));
                            }
                        });
                        groups.end(ui, &t, "Arrangement");
                        widgets::stack(ui, |ui| {
                            for (v, label) in [(None, "System"), (Some(false), "Light"), (Some(true), "Dark")] {
                                if ui.radio(ust.dark == v, tl!(label)).clicked() {
                                    acts.push(Action::SetDark(v));
                                }
                            }
                        });
                        groups.last(ui, &t, "Theme");
                    }
                }
            });
        });
    }
}

/// The ribbon's height (its tabs and its row of buttons), the same whichever tab is shown.
pub(crate) const RIBBON_H: f32 = 65.0;

/// The Quick Steps menu: each step (done to the selection), then making and changing them.
pub(crate) fn quick_steps_menu(ui: &mut egui::Ui, s: &Session, acts: &mut Vec<Action>) {
    let has = !s.selection.is_empty();
    if s.prefs.quick_steps.is_empty() {
        ui.label(RichText::new(tl!("No Quick Steps yet.")).weak());
    }
    for (i, q) in s.prefs.quick_steps.iter().enumerate() {
        let mut b = egui::Button::new(q.name.as_str());
        if i < 9 {
            b = b.shortcut_text(format!("Ctrl+Shift+{}", i + 1));
        }
        if ui.add_enabled(has, b).clicked() {
            acts.push(Action::Run("quickStep.run", json!({"index": i})));
            ui.close();
        }
    }
    ui.separator();
    if ui.button(tl!("New Quick Step…")).clicked() {
        acts.push(Action::Dialog(Dialog::QuickStep(Box::new(crate::dialogs::QuickStepForm::new(s)))));
        ui.close();
    }
    if !s.prefs.quick_steps.is_empty() {
        ui.menu_button(tl!("Edit a Quick Step"), |ui| {
            for q in &s.prefs.quick_steps {
                if ui.button(q.name.as_str()).clicked() {
                    acts.push(Action::Dialog(Dialog::QuickStep(Box::new(crate::dialogs::QuickStepForm::edit(s, q)))));
                    ui.close();
                }
            }
        });
    }
}

/// The Junk menu: block the selection's senders, or never block them (or their domain), and
/// the lists themselves (File › Options).
pub(crate) fn junk_menu(ui: &mut egui::Ui, s: &Session, acts: &mut Vec<Action>) {
    let has = !s.selection.is_empty();
    for (id, label, tip) in [
        ("junk.blockSender", "Block sender", "Their mail goes to Junk Email from now on"),
        ("junk.neverBlockSender", "Never block sender", "Their mail is never treated as junk"),
        ("junk.neverBlockDomain", "Never block sender's domain", "Mail from anyone at their domain is never treated as junk"),
    ] {
        if ui.add_enabled(has, egui::Button::new(tl!(label))).on_hover_text(tl!(tip)).clicked() {
            acts.push(Action::Run(id, json!({})));
            ui.close();
        }
    }
    ui.separator();
    if ui.button(tl!("Junk Email options…")).clicked() {
        acts.push(Action::Backstage(backstage::Page::Options));
        ui.close();
    }
}
