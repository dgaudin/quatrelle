//! Tasks, as Outlook's Tasks module: the To-Do List grouped by due date (Overdue, Today,
//! Tomorrow, This Week…), a box at its top to type a new one in, a tick to mark one complete.
//! The one picked shows beside the list, written on in place ([`form`]). Exchange accounts keep
//! them in the mailbox's Tasks folder, others on this computer.

use egui::{Margin, RichText, Sense, Ui, pos2, vec2};
use quatrelle_engine::model::dates::{self, civil_from_days};
use quatrelle_engine::model::pim::tasks::{DueGroup, due_group};
use quatrelle_engine::{Session, TaskRef};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, UiState, i18n, widgets};

mod form;

/// What Tasks keeps between frames.
#[derive(Clone, Debug, Default)]
pub struct TasksUi {
    /// The search (from the box at the top of the window).
    pub query: String,
    /// The task picked (its uid).
    pub selected: Option<String>,
    /// What's typed in the box at the top of the list, and whether it's to get the keyboard
    /// (New Task).
    pub new_text: String,
    pub focus_new: bool,
    /// The completed ones listed too.
    pub completed: bool,
    /// The picked one's form, as typed.
    form: Option<form::Form>,
}

/// The side pane: New Task, how many are left to do (and late), Show completed tasks.
pub fn nav(s: &Session, st: &mut UiState, ui: &mut Ui, _acts: &mut Vec<Action>) {
    let t = st.tokens;
    if widgets::accent_button(ui, Icon::Plus, "New task", &t, !s.accounts.is_empty()).clicked() {
        st.tasks.focus_new = true;
    }
    ui.add_space(8.0);
    let today = dates::today(&s.clock);
    let open = s.tasks("", false);
    let late = open.iter().filter(|r| r.task.is_overdue(today)).count();
    ui.label(RichText::new(tf!("{n} tasks to do", n = open.len())).color(t.faint).small());
    if late > 0 {
        ui.label(RichText::new(tf!("{n} overdue", n = late)).color(t.danger).small());
    }
    ui.add_space(8.0);
    ui.checkbox(&mut st.tasks.completed, tl!("Show completed tasks"));
}

/// The Home tab in Tasks: New Task; Delete and Mark Complete for the one picked, its
/// priority; whether the completed ones show.
pub(crate) fn ribbon(s: &Session, st: &mut UiState, ui: &mut Ui, groups: &mut widgets::Groups, acts: &mut Vec<Action>) {
    let t = st.tokens;
    if widgets::big_accent(ui, Icon::Tasks, "New task", &t, !s.accounts.is_empty()).clicked() {
        st.tasks.focus_new = true;
    }
    groups.end(ui, &t, "New");
    let picked = st.tasks.selected.as_deref().and_then(|u| s.find_task(u));
    if widgets::big_button(ui, Icon::Delete, "Delete", &t, picked.is_some()).clicked()
        && let Some(p) = &picked
    {
        st.tasks.selected = None;
        acts.push(Action::Run("task.delete", json!({"uid": p.task.uid})));
    }
    let done = picked.as_ref().is_some_and(|p| p.task.is_complete());
    if widgets::big_toggle(ui, Icon::Complete, "Mark Complete", &t, done).clicked()
        && let Some(p) = &picked
    {
        acts.push(Action::Run("task.complete", json!({"uid": p.task.uid, "done": !done})));
    }
    groups.end(ui, &t, "Manage Task");
    let importance = picked.as_ref().map_or(0, |p| p.task.importance);
    for (icon, label, level) in [(Icon::Important, "High Importance", 1), (Icon::LowImportance, "Low Importance", -1)] {
        if widgets::big_toggle(ui, icon, label, &t, picked.is_some() && importance == level).clicked()
            && let Some(p) = &picked
        {
            let to = if importance == level { 0 } else { level };
            acts.push(Action::Run("task.update", json!({"uid": p.task.uid, "importance": to})));
        }
    }
    groups.end(ui, &t, "Tags");
    if widgets::big_toggle(ui, Icon::Funnel, "Show completed", &t, st.tasks.completed).clicked() {
        st.tasks.completed = !st.tasks.completed;
    }
    groups.last(ui, &t, "Current View");
}

/// The main pane: the list, and the one picked beside it.
pub fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let today = dates::today(&s.clock);
    let list = s.tasks(&st.tasks.query, st.tasks.completed);
    // The one picked went (deleted, or done with the completed ones hidden): none is.
    let picked = st.tasks.selected.as_deref().and_then(|u| list.iter().find(|r| r.task.uid == u)).cloned();
    if picked.is_none() {
        st.tasks.selected = None;
        st.tasks.form = None;
    }
    if let Some(p) = &picked {
        let end = if i18n::current().rtl() { egui::Panel::left("task-form") } else { egui::Panel::right("task-form") };
        end.resizable(true)
            .default_size(380.0)
            .size_range(300.0..=560.0)
            .frame(egui::Frame::new().fill(t.surface).inner_margin(Margin::same(16)).stroke(egui::Stroke::new(1.0, t.border)))
            .show(ui, |ui| form::show(s, &mut st.tasks, &t, ui, p, today, acts));
    }
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| list_pane(s, &mut st.tasks, &t, ui, &list, today, acts));
}

/// The box to type a new task in, then the tasks under their groups' headings.
fn list_pane(s: &Session, tu: &mut TasksUi, t: &Tokens, ui: &mut Ui, list: &[TaskRef], today: i64, acts: &mut Vec<Action>) {
    egui::Frame::new().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
        let r = ui.add_enabled(
            !s.accounts.is_empty(),
            egui::TextEdit::singleline(&mut tu.new_text).hint_text(tl!("Type a new task")).desired_width(f32::INFINITY),
        );
        if std::mem::take(&mut tu.focus_new) {
            r.request_focus();
        }
        // Enter adds it, and the box stays to type the next one.
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let subject = tu.new_text.trim().to_string();
            if !subject.is_empty() {
                acts.push(Action::Run("task.create", json!({"subject": subject})));
                tu.new_text.clear();
                r.request_focus();
            }
        }
    });
    let line = ui.available_rect_before_wrap();
    ui.painter().line_segment([line.left_top(), line.right_top()], egui::Stroke::new(1.0, t.border));
    if list.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.3);
                let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::Tasks, t.border);
                ui.add_space(8.0);
                let empty = tu.query.trim().is_empty();
                ui.label(RichText::new(tl!(if empty { "Nothing to do: type a new task above." } else { "No task matches." })).color(t.faint));
            });
        });
        return;
    }
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("tasks").show(ui, |ui| {
        let mut group: Option<DueGroup> = None;
        for r in list {
            let g = due_group(&r.task, today);
            if group != Some(g) {
                group = Some(g);
                let n = list.iter().filter(|x| due_group(&x.task, today) == g).count();
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    let color = if g == DueGroup::Overdue { t.danger } else { t.dim };
                    ui.label(RichText::new(format!("{}  {n}", tl!(g.label()))).font(theme::bold(12.5)).color(color));
                });
                ui.add_space(2.0);
            }
            row(s, tu, t, ui, r, today, acts);
        }
        ui.add_space(12.0);
    });
}

/// A due day as a list shows it: `Tue 27/10`.
pub(crate) fn short_day(day: i64) -> String {
    let (_, m, d) = civil_from_days(day);
    let wd = usize::try_from((day + 3).rem_euclid(7)).unwrap_or(0);
    let name = crate::calendar::DAYS.get(wd).copied().unwrap_or("");
    tf!("{weekday} {day}/{month}", weekday = tl!(name), day = d, month = format!("{m:02}"))
}

/// One task: its tick, its subject (struck through once done, red once late), and on the far
/// side its due day, priority, reminder and categories. A click picks it; right-click gives
/// Mark Complete and Delete.
fn row(s: &Session, tu: &mut TasksUi, t: &Tokens, ui: &mut Ui, r: &TaskRef, today: i64, acts: &mut Vec<Action>) {
    let task = &r.task;
    let rtl = i18n::current().rtl();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let picked = tu.selected.as_deref() == Some(task.uid.as_str());
    if picked || resp.hovered() {
        ui.painter().rect_filled(rect.shrink2(vec2(6.0, 1.0)), 4.0, if picked { t.accent_soft } else { t.hover });
    }
    // Positions from the row's start (its right in a right-to-left language).
    let at = |from: f32| if rtl { rect.right() - from } else { rect.left() + from };
    let done = task.is_complete();
    let late = task.is_overdue(today);
    // The tick: a ring, filled with a tick once done (or shown faintly under the pointer).
    let tick = egui::Rect::from_center_size(pos2(at(26.0), rect.center().y), vec2(18.0, 18.0));
    let tick_resp = ui.interact(tick.expand(3.0), ui.scope_id().with(("task-tick", &task.uid)), Sense::click()).on_hover_text(tl!(if done {
        "Mark as not complete"
    } else {
        "Mark Complete"
    }));
    if done {
        ui.painter().circle_filled(tick.center(), 9.0, t.accent);
        icons::paint(ui.painter(), tick.shrink(3.0), Icon::Read, t.on_accent);
    } else {
        ui.painter().circle_stroke(tick.center(), 8.5, egui::Stroke::new(1.5, if late { t.danger } else { t.dim }));
        if tick_resp.hovered() {
            icons::paint(ui.painter(), tick.shrink(3.0), Icon::Read, t.faint);
        }
    }
    if tick_resp.clicked() {
        acts.push(Action::Run("task.complete", json!({"uid": task.uid, "done": !done})));
    }
    // The far side, from the edge in: due day, then the marks.
    let mut edge = 18.0;
    let side = |from: f32, w: f32| {
        let x = if rtl { rect.left() + from } else { rect.right() - from - w };
        egui::Rect::from_min_size(pos2(x, rect.center().y - 9.0), vec2(w, 18.0))
    };
    if let Some(due) = task.due {
        let text = short_day(due);
        let color = if late {
            t.danger
        } else if done {
            t.faint
        } else {
            t.dim
        };
        let g = widgets::one_line(ui, &text, theme::regular(12.0), color, 110.0);
        let w = g.size().x;
        ui.painter().galley(side(edge, w).left_center() - vec2(0.0, g.size().y / 2.0), g, color);
        edge += w + 12.0;
    }
    let mut mark = |icon: Icon, color: egui::Color32, tip: &str| {
        let r = side(edge, 16.0);
        icons::paint(ui.painter(), r, icon, color);
        ui.interact(r, ui.scope_id().with(("task-mark", &task.uid, tip)), Sense::hover()).on_hover_text(tl!(tip));
        edge += 22.0;
    };
    if task.reminder.is_some() {
        mark(Icon::Reminder, t.dim, "Reminder");
    }
    match task.importance {
        1.. => mark(Icon::Important, t.danger, "High Importance"),
        ..0 => mark(Icon::LowImportance, t.accent, "Low Importance"),
        0 => {}
    }
    for c in task.categories.iter().take(3) {
        let dot = side(edge, 10.0);
        ui.painter().circle_filled(dot.center(), 5.0, theme::category_color(s.category_color_of(c), t.dark));
        ui.interact(dot, ui.scope_id().with(("task-category", &task.uid, c)), Sense::hover()).on_hover_text(c);
        edge += 16.0;
    }
    // The subject, in what's left.
    let room = (rect.width() - 48.0 - edge).max(40.0);
    let color = if done {
        t.faint
    } else if late {
        t.danger
    } else {
        t.text
    };
    let g = widgets::one_line(ui, &task.subject, theme::regular(14.0), color, room);
    let size = g.size();
    let x = if rtl { at(48.0) - size.x } else { at(48.0) };
    let top = pos2(x, rect.center().y - size.y / 2.0);
    if done {
        let y = rect.center().y;
        ui.painter().line_segment([pos2(top.x, y), pos2(top.x + size.x, y)], egui::Stroke::new(1.0, t.faint));
    }
    ui.painter().galley(top, g, color);
    if resp.clicked() {
        tu.selected = Some(task.uid.clone());
    }
    resp.context_menu(|ui| {
        if ui.button(tl!(if done { "Mark as not complete" } else { "Mark Complete" })).clicked() {
            acts.push(Action::Run("task.complete", json!({"uid": task.uid, "done": !done})));
            ui.close();
        }
        if ui.button(tl!("Delete")).clicked() {
            if picked {
                tu.selected = None;
            }
            acts.push(Action::Run("task.delete", json!({"uid": task.uid})));
            ui.close();
        }
    });
}
