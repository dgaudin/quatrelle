// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The calendar's search (issue #33): what's typed in the search box in the Calendar lists the
//! events it finds in place of the days, as Outlook's calendar search does: each with its date
//! and time, subject, place and calendar's colour; upcoming ones first, then past ones, or only
//! either. Clicking one goes to its day and opens it.

use egui::{Align2, Margin, Rect, RichText, Sense, Ui, pos2, vec2};
use quatrelle_engine::{EventHit, EventWhen, Session};
use serde_json::json;

use std::collections::HashSet;

use super::{color, day_of, hm, long_date};
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog, UiState};

/// What the calendar's search box holds, and what it found (kept while nothing changes).
#[derive(Clone, Debug, Default)]
pub struct CalSearch {
    pub query: String,
    pub when: EventWhen,
    /// One calendar (account, calendar id) to look in; `None`: every one shown.
    pub scope: Option<(String, String)>,
    found: Option<(Key, Vec<EventHit>)>,
    /// The calendars that can be searched (account, id, name), and what they were worked out
    /// from (the calendars' state, how many are unticked).
    searchable: Option<((u64, usize), Vec<(String, String, String)>)>,
    /// When a slow search was last said in the log.
    slow_said: Option<web_time::Instant>,
}

/// What a search's results depend on: the query, the switches, the calendars' state, the minute.
type Key = (String, EventWhen, Option<(String, String)>, u64, i64);

/// The most results listed.
const SHOWN: usize = 500;
/// A result's height.
const ROW: f32 = 46.0;
/// A heading's (Upcoming, Past).
const HEADING: f32 = 30.0;
/// A search this long or longer is said in the log (once every ten seconds at most).
const SLOW: web_time::Duration = web_time::Duration::from_millis(50);

impl CalSearch {
    /// Searching: the box has something in it.
    pub fn active(&self) -> bool {
        !self.query.trim().is_empty()
    }

    /// What the search finds, searched again only when the query, the switches, the calendars
    /// or the minute have changed.
    pub(crate) fn results(&mut self, s: &Session) -> &[EventHit] {
        // A calendar since closed or unsubscribed from, or one not kept here (unticked, never
        // read): every calendar, rather than nothing.
        if let Some((a, c)) = self.scope.clone()
            && !self.calendars(s).iter().any(|(x, y, _)| *x == a && *y == c)
        {
            self.scope = None;
        }
        let key: Key = (self.query.clone(), self.when, self.scope.clone(), s.pim_revision, s.clock.now.div_euclid(60));
        if self.found.as_ref().is_none_or(|(k, _)| *k != key) {
            let (account, calendar) = match &self.scope {
                Some((a, c)) => (Some(a.as_str()), Some(c.as_str())),
                None => (None, None),
            };
            let began = web_time::Instant::now();
            let hits = s.find_events(&self.query, self.when, account, calendar, SHOWN);
            let took = began.elapsed();
            if took >= SLOW && self.slow_said.is_none_or(|t| t.elapsed().as_secs() >= 10) {
                self.slow_said = Some(web_time::Instant::now());
                log::info!("calendar: searched in {} ms, {} found", took.as_millis(), hits.len());
            }
            self.found = Some((key, hits));
        }
        self.found.as_ref().map_or(&[], |(_, h)| h.as_slice())
    }

    /// The calendars that can be searched ([`searchable`]), worked out again only when the
    /// calendars or which are ticked have changed.
    pub(crate) fn calendars(&mut self, s: &Session) -> &[(String, String, String)] {
        let key = (s.pim_revision, s.prefs.hidden_calendars.len());
        if self.searchable.as_ref().is_none_or(|(k, _)| *k != key) {
            self.searchable = Some((key, searchable(s)));
        }
        self.searchable.as_ref().map_or(&[], |(_, l)| l.as_slice())
    }
}

impl crate::MailApp {
    /// Searches the calendar for `query` (the box's text), upcoming, past or both, and shows the
    /// Calendar (the start-up `--cmd ui.calendarSearch`).
    pub fn search_calendar(&mut self, query: &str, when: EventWhen) {
        self.run("view.module", json!({"name": "calendar"}));
        self.ui.cal_search.query = query.to_string();
        self.ui.cal_search.when = when;
    }
}

/// The calendars that can be searched (account, id, name), for the scope menu: those whose
/// appointments are read and kept here, every one ticked and those unticked that still have
/// what was read of them; not a colleague's never read (the engine's `not_read`, unticked).
fn searchable(s: &Session) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for a in &s.accounts {
        let id = &a.config.id;
        let Some(p) = s.pim.get(id) else { continue };
        // Which calendars have appointments kept here: only asked when some are unticked.
        let kept: HashSet<&str> = if p.calendars.iter().all(|c| s.calendar_shown(id, &c.id)) {
            HashSet::new()
        } else {
            p.events.iter().map(|e| e.calendar.as_str()).collect()
        };
        for c in p.calendars.iter().filter(|c| s.calendar_shown(id, &c.id) || kept.contains(c.id.as_str())) {
            out.push((id.clone(), c.id.clone(), format!("{} · {}", calendar_name(c), a.config.email)));
        }
    }
    out
}

/// A calendar's name, as the side pane gives it.
fn calendar_name(c: &quatrelle_engine::model::pim::CalendarInfo) -> String {
    if c.id == quatrelle_engine::model::pim::LOCAL { tl!("Calendar").to_string() } else { c.name.clone() }
}

/// The results, in place of the days.
pub(crate) fn found(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    egui::Frame::new().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
        ui.horizontal(|ui| {
            for (when, label) in [(EventWhen::All, tl!("All")), (EventWhen::Upcoming, tl!("Upcoming")), (EventWhen::Past, tl!("Past"))] {
                if ui.selectable_label(st.cal_search.when == when, label).clicked() {
                    st.cal_search.when = when;
                }
            }
            ui.add_space(12.0);
            scope_menu(s, st, ui);
        });
    });
    // The results taken out to draw (not copied each frame), then put back.
    st.cal_search.results(s);
    let found = st.cal_search.found.take();
    let hits = found.as_ref().map_or(&[][..], |(_, h)| h.as_slice());
    list(s, st, ui, hits, acts);
    if st.cal_search.found.is_none() {
        st.cal_search.found = found;
    }
}

/// Where to look, as the mail search box's menu: every calendar shown, or one that can be searched.
fn scope_menu(s: &Session, st: &mut UiState, ui: &mut Ui) {
    st.cal_search.calendars(s);
    let kept = st.cal_search.searchable.take();
    let all = kept.as_ref().map_or(&[][..], |(_, l)| l.as_slice());
    let shown = match &st.cal_search.scope {
        Some((a, c)) => all.iter().find(|(x, y, _)| x == a && y == c).map_or_else(|| tl!("All calendars").to_string(), |x| x.2.clone()),
        None => tl!("All calendars").to_string(),
    };
    egui::ComboBox::from_id_salt("calendar-search-scope").selected_text(shown).show_ui(ui, |ui| {
        if ui.selectable_label(st.cal_search.scope.is_none(), tl!("All calendars")).clicked() {
            st.cal_search.scope = None;
        }
        for (a, c, name) in all {
            let on = st.cal_search.scope.as_ref().is_some_and(|(x, y)| x == a && y == c);
            if ui.selectable_label(on, name).clicked() {
                st.cal_search.scope = Some((a.clone(), c.clone()));
            }
        }
    });
    if st.cal_search.searchable.is_none() {
        st.cal_search.searchable = kept;
    }
}

/// A line of the results: a heading (upcoming ones, or past ones), or a result.
enum Line {
    Heading(bool),
    Hit(usize),
}

/// The results under their headings, only those in view drawn.
fn list(s: &Session, st: &mut UiState, ui: &mut Ui, hits: &[EventHit], acts: &mut Vec<Action>) {
    let t = st.tokens;
    if hits.is_empty() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Calendar, t.border);
            ui.add_space(8.0);
            ui.label(RichText::new(tl!("No events match.")).color(t.faint));
        });
        return;
    }
    // Upcoming ones, then past ones, each under its heading: each line with its top.
    let mut lines: Vec<(f32, Line)> = Vec::with_capacity(hits.len() + 2);
    let mut y = 0.0;
    let mut heading: Option<bool> = None;
    for (i, h) in hits.iter().enumerate() {
        if st.cal_search.when == EventWhen::All && heading != Some(h.upcoming) {
            heading = Some(h.upcoming);
            lines.push((y, Line::Heading(h.upcoming)));
            y += HEADING;
        }
        lines.push((y, Line::Hit(i)));
        y += ROW;
    }
    let total = y + 10.0;
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("calendar-found").show_viewport(ui, |ui, view| {
        ui.set_height(total);
        let origin = ui.min_rect().min + vec2(14.0, 0.0);
        let width = (ui.available_width() - 28.0).max(0.0);
        let first = lines.partition_point(|(top, line)| top + if matches!(line, Line::Heading(_)) { HEADING } else { ROW } < view.min.y);
        for (top, line) in lines.iter().skip(first) {
            if *top > view.max.y {
                break;
            }
            match line {
                Line::Heading(upcoming) => {
                    let r = Rect::from_min_size(origin + vec2(0.0, *top), vec2(width, HEADING));
                    let label = if *upcoming { tl!("Upcoming") } else { tl!("Past") };
                    ui.painter().text(pos2(r.min.x, r.max.y - 12.0), Align2::LEFT_CENTER, label, theme::bold(13.0), t.dim);
                }
                Line::Hit(i) => {
                    let Some(h) = hits.get(*i) else { continue };
                    let r = Rect::from_min_size(origin + vec2(0.0, *top), vec2(width, ROW));
                    let clicked =
                        ui.scope_builder(egui::UiBuilder::new().id_salt(("calendar-found", *i)).max_rect(r), |ui| row(s, &t, ui, h).clicked()).inner;
                    if clicked {
                        open_hit(s, st, h, acts);
                    }
                }
            }
        }
    });
}

/// A result clicked: its day in the view the calendar was in (the search box emptied), and the
/// event open at that occurrence.
pub(crate) fn open_hit(s: &Session, st: &mut UiState, h: &EventHit, acts: &mut Vec<Action>) {
    let (y, m, d) = quatrelle_engine::model::dates::civil_from_days(day_of(h.start, s));
    acts.push(Action::Run("calendar.goto", json!({"date": format!("{y:04}-{m:02}-{d:02}")})));
    if let Some(dialog) = Dialog::open_event(s, &h.uid, h.occurrence) {
        acts.push(Action::Dialog(dialog));
    }
    st.cal_search.query.clear();
}

/// One result: its colour, its date and time, its subject (a lock when private, a mark when it
/// repeats), its place, categories and calendar.
fn row(s: &Session, t: &Tokens, ui: &mut Ui, h: &EventHit) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover_row);
    }
    // Its first category's colour, else its calendar's, in the strip a chip of the days has.
    let c = h.category.map_or_else(|| color(h.color, t), |i| theme::category_color(i, t.dark));
    ui.painter().rect_filled(Rect::from_min_size(rect.min + vec2(2.0, 6.0), vec2(3.0, ROW - 12.0)), 2.0, c);
    let p = ui.painter().with_clip_rect(rect);
    let date_x = rect.min.x + 16.0;
    let day = day_of(h.start, s);
    p.text(pos2(date_x, rect.min.y + 14.0), Align2::LEFT_CENTER, long_date(day), theme::regular(12.5), t.text);
    let time = if h.all_day { tl!("All day").to_string() } else { format!("{} – {}", hm(h.start, s), hm(h.end, s)) };
    p.text(pos2(date_x, rect.min.y + 32.0), Align2::LEFT_CENTER, time, theme::regular(12.0), t.dim);
    // The subject and what's under it, after the date's column (narrower on a narrow pane).
    let x = date_x + (rect.width() * 0.3).clamp(150.0, 240.0);
    let mut sx = x;
    if h.private {
        icons::paint(&p, Rect::from_min_size(pos2(sx, rect.min.y + 8.0), vec2(12.0, 12.0)), Icon::Lock, t.dim);
        sx += 17.0;
    }
    if h.recurring {
        icons::paint(&p, Rect::from_min_size(pos2(sx, rect.min.y + 8.0), vec2(12.0, 12.0)), Icon::Sync, t.dim);
        sx += 17.0;
    }
    let title = if h.summary.trim().is_empty() { tl!("(no title)").to_string() } else { h.summary.clone() };
    p.with_clip_rect(rect.with_min_x(sx).shrink2(vec2(0.0, 1.0)).with_max_x(rect.max.x - 8.0)).text(
        pos2(sx, rect.min.y + 14.0),
        Align2::LEFT_CENTER,
        &title,
        theme::bold(13.0),
        t.text,
    );
    let calendar = s.pim.get(&h.account).and_then(|p| p.calendars.iter().find(|c| c.id == h.calendar)).map(calendar_name).unwrap_or_default();
    let under: Vec<String> = [h.location.clone(), h.categories.join(", "), calendar].into_iter().filter(|x| !x.trim().is_empty()).collect();
    p.with_clip_rect(rect.with_min_x(x).with_max_x(rect.max.x - 8.0)).text(
        pos2(x, rect.min.y + 32.0),
        Align2::LEFT_CENTER,
        under.join(" · "),
        theme::regular(12.0),
        t.dim,
    );
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], egui::Stroke::new(1.0, t.border));
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    crate::widgets::named(ui, &resp, &format!("{} {title}", long_date(day)), true, None);
    resp
}
