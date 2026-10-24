//! Outlook's Appointment Recurrence: how often it repeats (every few days or every working day,
//! weekly on some days, monthly on a day or on the nth weekday, yearly the same ways) and until
//! when (no end, after so many times, by a date), kept as the event's RRULE (RFC 5545 §3.3.10),
//! what Exchange and Microsoft Graph carry both ways. A rule this can't show (BYSETPOS, hourly…)
//! is left as it was.

use quatrelle_engine::model::dates::{civil_from_days, days_from_civil};

use super::*;

/// How often.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Freq {
    Daily,
    #[default]
    Weekly,
    Monthly,
    Yearly,
}

/// Until when.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ends {
    #[default]
    Never,
    /// After this many times.
    After(u32),
    /// On this day (days since 1970), the last one it can fall on.
    By(i64),
}

/// The weekdays as RRULE names them, Monday first.
const CODES: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];
/// Their names as the interface says them.
const WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
/// The nth weekday of a month: the first to the fourth, and the last.
const NTH: [(i32, &str); 5] = [(1, "first"), (2, "second"), (3, "third"), (4, "fourth"), (-1, "last")];

/// A repeat as the dialog shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recurrence {
    pub freq: Freq,
    /// Every this many days, weeks, months or years (1 and up).
    pub interval: u32,
    /// Daily: every working day (Monday to Friday) rather than every `interval` days.
    pub weekdays_only: bool,
    /// Weekly: the days it falls on, Monday first.
    pub days: [bool; 7],
    /// Monthly and yearly: on the `nth` (1 to 4, or -1 for the last) `weekday` of the month,
    /// rather than on day `month_day`.
    pub relative: bool,
    pub month_day: u32,
    pub nth: i32,
    pub weekday: usize,
    /// Yearly: its month (1 to 12).
    pub month: u32,
    pub ends: Ends,
}

/// The weekday (0: Monday) of a day since 1970 (a Thursday).
fn weekday_of(day: i64) -> usize {
    usize::try_from((day + 3).rem_euclid(7)).unwrap_or(0)
}

impl Recurrence {
    /// The usual one for an event starting on `day`: every week on that weekday, its day of
    /// the month and its month ready for the other kinds.
    pub fn starting(day: i64) -> Recurrence {
        let (_, m, d) = civil_from_days(day);
        let wd = weekday_of(day);
        let mut days = [false; 7];
        if let Some(x) = days.get_mut(wd) {
            *x = true;
        }
        let nth = i32::try_from((d - 1) / 7 + 1).unwrap_or(1);
        Recurrence { interval: 1, days, month_day: d, nth: if nth > 4 { -1 } else { nth }, weekday: wd, month: m, ..Default::default() }
    }

    /// From a rule, for an event starting on `day`; `None` for one this can't show.
    pub fn from_rule(rule: &str, day: i64) -> Option<Recurrence> {
        let mut r = Recurrence::starting(day);
        let mut byday: Vec<(i32, usize)> = Vec::new();
        let mut freq = None;
        for part in rule.split(';').filter(|p| !p.trim().is_empty()) {
            let (k, v) = part.split_once('=')?;
            let v = v.trim().to_ascii_uppercase();
            match k.trim().to_ascii_uppercase().as_str() {
                "FREQ" => {
                    freq = Some(match v.as_str() {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        "MONTHLY" => Freq::Monthly,
                        "YEARLY" => Freq::Yearly,
                        _ => return None,
                    })
                }
                "INTERVAL" => r.interval = v.parse().ok().filter(|n| (1..=999).contains(n))?,
                "BYDAY" => {
                    for d in v.split(',') {
                        let code = d.get(d.len().checked_sub(2)?..)?;
                        let n = d.get(..d.len() - 2).filter(|n| !n.is_empty()).map_or(Some(0), |n| n.parse::<i32>().ok())?;
                        byday.push((n, CODES.iter().position(|c| *c == code)?));
                    }
                }
                "BYMONTHDAY" => r.month_day = v.parse().ok().filter(|n| (1..=31).contains(n))?,
                "BYMONTH" => r.month = v.parse().ok().filter(|n| (1..=12).contains(n))?,
                "COUNT" => r.ends = Ends::After(v.parse().ok().filter(|n| *n >= 1)?),
                "UNTIL" => {
                    let (y, m, d) = (v.get(0..4)?.parse().ok()?, v.get(4..6)?.parse().ok()?, v.get(6..8)?.parse().ok()?);
                    r.ends = Ends::By(days_from_civil(y, m, d));
                }
                "WKST" => {}
                _ => return None,
            }
        }
        r.freq = freq?;
        match r.freq {
            Freq::Daily if !byday.is_empty() => return None,
            Freq::Weekly => {
                if byday.iter().any(|(n, _)| *n != 0) {
                    return None;
                }
                if !byday.is_empty() {
                    r.days = [false; 7];
                    for (_, d) in &byday {
                        if let Some(x) = r.days.get_mut(*d) {
                            *x = true;
                        }
                    }
                }
                // Monday to Friday, every week: Outlook's "every weekday".
                if r.interval == 1 && r.days == [true, true, true, true, true, false, false] {
                    r.freq = Freq::Daily;
                    r.weekdays_only = true;
                }
            }
            Freq::Monthly | Freq::Yearly => match byday.as_slice() {
                [] => {}
                [(n, d)] if NTH.iter().any(|(x, _)| x == n) => {
                    r.relative = true;
                    (r.nth, r.weekday) = (*n, *d);
                }
                _ => return None,
            },
            Freq::Daily => {}
        }
        Some(r)
    }

    /// Whether it falls on `day` (days since 1970), its interval aside.
    fn falls_on(&self, day: i64) -> bool {
        let (y, m, d) = civil_from_days(day);
        let wd = weekday_of(day);
        let next_month = if m == 12 { days_from_civil(y + 1, 1, 1) } else { days_from_civil(y, m + 1, 1) };
        let nth = if day + 7 >= next_month && self.nth == -1 { -1 } else { i32::try_from(d.saturating_sub(1) / 7 + 1).unwrap_or(0) };
        let in_month = if self.relative { wd == self.weekday && nth == self.nth } else { d == self.month_day };
        match self.freq {
            Freq::Daily => !self.weekdays_only || wd < 5,
            Freq::Weekly => self.days.get(wd) == Some(&true),
            Freq::Monthly => in_month,
            Freq::Yearly => in_month && m == self.month,
        }
    }

    /// The first day on or after `day` it falls on, where Outlook moves the appointment (RFC
    /// 5545 wants the start to be one of its occurrences); `day` itself if none in four years.
    pub fn first_from(&self, day: i64) -> i64 {
        (day..day.saturating_add(4 * 366 + 31)).find(|d| self.falls_on(*d)).unwrap_or(day)
    }

    /// As a rule.
    pub fn rule(&self) -> String {
        let interval = self.interval.max(1);
        let mut out = match (self.freq, self.weekdays_only) {
            (Freq::Daily, true) => "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR".to_string(),
            (Freq::Daily, false) => "FREQ=DAILY".to_string(),
            (Freq::Weekly, _) => "FREQ=WEEKLY".to_string(),
            (Freq::Monthly, _) => "FREQ=MONTHLY".to_string(),
            (Freq::Yearly, _) => "FREQ=YEARLY".to_string(),
        };
        if interval > 1 && !(self.freq == Freq::Daily && self.weekdays_only) {
            out.push_str(&format!(";INTERVAL={interval}"));
        }
        let code = |d: usize| CODES.get(d).copied().unwrap_or("MO");
        match self.freq {
            Freq::Weekly => {
                let days: Vec<&str> = (0..7).filter(|d| self.days.get(*d) == Some(&true)).map(code).collect();
                if !days.is_empty() {
                    out.push_str(&format!(";BYDAY={}", days.join(",")));
                }
            }
            Freq::Monthly | Freq::Yearly => {
                if self.freq == Freq::Yearly {
                    out.push_str(&format!(";BYMONTH={}", self.month.clamp(1, 12)));
                }
                if self.relative {
                    out.push_str(&format!(";BYDAY={}{}", self.nth, code(self.weekday)));
                } else {
                    out.push_str(&format!(";BYMONTHDAY={}", self.month_day.clamp(1, 31)));
                }
            }
            Freq::Daily => {}
        }
        match self.ends {
            Ends::Never => {}
            Ends::After(n) => out.push_str(&format!(";COUNT={}", n.max(1))),
            Ends::By(day) => {
                let (y, m, d) = civil_from_days(day);
                out.push_str(&format!(";UNTIL={y:04}{m:02}{d:02}"));
            }
        }
        out
    }

    /// In words, as Outlook says it under the times ("Every 2 weeks on Monday, Wednesday").
    pub fn describe(&self) -> String {
        let n = self.interval.max(1);
        let day_name = |d: usize| tl!(WEEKDAYS.get(d).copied().unwrap_or("Monday")).to_string();
        let nth = NTH.iter().find(|(x, _)| *x == self.nth).map_or("first", |(_, s)| *s);
        let month = crate::calendar::MONTHS.get(self.month.clamp(1, 12) as usize - 1).copied().unwrap_or("");
        let month = crate::i18n::tc("month", month);
        let what = match self.freq {
            Freq::Daily if self.weekdays_only => tl!("Every weekday").to_string(),
            Freq::Daily if n == 1 => tl!("Every day").to_string(),
            Freq::Daily => tf!("Every {n} days", n = n),
            Freq::Weekly => {
                let days: Vec<String> = (0..7).filter(|d| self.days.get(*d) == Some(&true)).map(day_name).collect();
                let days = days.join(", ");
                if n == 1 { tf!("Every week on {days}", days = days) } else { tf!("Every {n} weeks on {days}", n = n, days = days) }
            }
            Freq::Monthly if self.relative => {
                let (nth, weekday) = (tl!(nth), day_name(self.weekday));
                if n == 1 {
                    tf!("The {nth} {weekday} of every month", nth = nth, weekday = weekday)
                } else {
                    tf!("The {nth} {weekday} of every {n} months", nth = nth, weekday = weekday, n = n)
                }
            }
            Freq::Monthly if n == 1 => tf!("Day {day} of every month", day = self.month_day),
            Freq::Monthly => tf!("Day {day} of every {n} months", day = self.month_day, n = n),
            Freq::Yearly if self.relative => {
                let (nth, weekday) = (tl!(nth), day_name(self.weekday));
                if n == 1 {
                    tf!("The {nth} {weekday} of {month}, every year", nth = nth, weekday = weekday, month = month)
                } else {
                    tf!("The {nth} {weekday} of {month}, every {n} years", nth = nth, weekday = weekday, month = month, n = n)
                }
            }
            Freq::Yearly if n == 1 => tf!("Every year on {day} {month}", day = self.month_day, month = month),
            Freq::Yearly => tf!("Every {n} years on {day} {month}", n = n, day = self.month_day, month = month),
        };
        match self.ends {
            Ends::Never => what,
            Ends::After(c) => tf!("{what}, {n} times", what = what, n = c),
            Ends::By(day) => tf!("{what}, until {date}", what = what, date = crate::calendar::long_date(day)),
        }
    }
}

/// The Appointment Recurrence dialog's state, while it's open over the appointment's window.
#[derive(Clone, Debug, Default)]
pub struct RecurrenceUi {
    pub r: Recurrence,
    /// The interval, the counts and the day as typed.
    pub interval: String,
    pub month_day: String,
    pub count: String,
    pub until: String,
}

impl RecurrenceUi {
    /// For an event's rule (its repeat as the form keeps it), starting on `day`.
    pub fn open(repeat: &str, day: i64) -> RecurrenceUi {
        let rule = match repeat {
            "daily" | "weekly" | "monthly" | "yearly" => format!("FREQ={}", repeat.to_ascii_uppercase()),
            r => r.to_string(),
        };
        let r = Recurrence::from_rule(&rule, day).unwrap_or_else(|| Recurrence::starting(day));
        let until = match r.ends {
            Ends::By(d) => d,
            _ => day + 90,
        };
        let (y, m, d) = civil_from_days(until);
        RecurrenceUi {
            interval: r.interval.max(1).to_string(),
            month_day: r.month_day.to_string(),
            count: match r.ends {
                Ends::After(n) => n.to_string(),
                _ => "10".to_string(),
            },
            until: format!("{y:04}-{m:02}-{d:02}"),
            r,
        }
    }

    /// The repeat as typed, if every field reads.
    fn read(&self) -> Option<Recurrence> {
        let mut r = self.r.clone();
        r.interval = self.interval.trim().parse().ok().filter(|n| (1..=999).contains(n))?;
        if !r.relative && matches!(r.freq, Freq::Monthly | Freq::Yearly) {
            r.month_day = self.month_day.trim().parse().ok().filter(|n| (1..=31).contains(n))?;
        }
        r.ends = match r.ends {
            Ends::Never => Ends::Never,
            Ends::After(_) => Ends::After(self.count.trim().parse().ok().filter(|n| (1..=999).contains(n))?),
            Ends::By(_) => {
                let mut it = self.until.trim().split('-');
                let (y, m, d) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
                Ends::By(days_from_civil(y, m, d))
            }
        };
        (r.freq != Freq::Weekly || r.days.iter().any(|d| *d)).then_some(r)
    }
}

/// What the dialog was closed with.
pub enum Closed {
    /// The repeat chosen.
    Rule(Recurrence),
    /// Remove Recurrence: it no longer repeats.
    Removed,
    Cancelled,
}

/// The dialog, over the window numbered `window`; what it was closed with, once it is.
pub fn dialog(ctx: &egui::Context, st: &mut RecurrenceUi, t: &Tokens, window: u64, today: i64) -> Option<Closed> {
    let mut closed = None;
    egui::Modal::new(egui::Id::unique(("appointment-recurrence", window))).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(RichText::new(tl!("Appointment Recurrence")).font(theme::bold(15.0)).color(t.text));
        ui.add_space(8.0);
        ui.label(RichText::new(tl!("Recurrence pattern")).color(t.dim));
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                for (f, name) in [(Freq::Daily, "Daily"), (Freq::Weekly, "Weekly"), (Freq::Monthly, "Monthly"), (Freq::Yearly, "Yearly")] {
                    ui.radio_value(&mut st.r.freq, f, tl!(name));
                }
            });
            ui.add_space(16.0);
            ui.vertical(|ui| pattern(ui, st));
        });
        ui.add_space(10.0);
        ui.label(RichText::new(tl!("Range of recurrence")).color(t.dim));
        ui.radio_value(&mut st.r.ends, Ends::Never, tl!("No end date"));
        ui.horizontal(|ui| {
            if ui.radio(matches!(st.r.ends, Ends::After(_)), tl!("End after")).clicked() {
                st.r.ends = Ends::After(1);
            }
            ui.add(egui::TextEdit::singleline(&mut st.count).desired_width(44.0));
            ui.label(tl!("occurrences"));
        });
        ui.horizontal(|ui| {
            if ui.radio(matches!(st.r.ends, Ends::By(_)), tl!("End by")).clicked() {
                st.r.ends = Ends::By(0);
            }
            if super::pickers::date(ui, t, "recurrence-until", &mut st.until, today) {
                st.r.ends = Ends::By(0);
            }
        });
        let read = st.read();
        if let Some(r) = &read {
            ui.add_space(6.0);
            ui.label(RichText::new(r.describe()).color(t.dim).small());
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(read.is_some(), egui::Button::new(tl!("OK"))).clicked()
                && let Some(r) = &read
            {
                closed = Some(Closed::Rule(r.clone()));
            }
            if ui.button(tl!("Cancel")).clicked() {
                closed = Some(Closed::Cancelled);
            }
            if ui.button(tl!("Remove Recurrence")).clicked() {
                closed = Some(Closed::Removed);
            }
        });
    });
    closed
}

/// The pattern's choices, for the kind picked.
fn pattern(ui: &mut Ui, st: &mut RecurrenceUi) {
    let every = |ui: &mut Ui, text: &mut String, unit: &str| {
        ui.horizontal(|ui| {
            ui.label(tl!("Every"));
            ui.add(egui::TextEdit::singleline(text).desired_width(40.0));
            ui.label(tl!(unit));
        });
    };
    match st.r.freq {
        Freq::Daily => {
            if ui.radio(!st.r.weekdays_only, tl!("Every few days")).clicked() {
                st.r.weekdays_only = false;
            }
            every(ui, &mut st.interval, "day(s)");
            ui.radio_value(&mut st.r.weekdays_only, true, tl!("Every weekday"));
        }
        Freq::Weekly => {
            every(ui, &mut st.interval, "week(s) on:");
            ui.horizontal_wrapped(|ui| {
                for (i, name) in WEEKDAYS.iter().enumerate() {
                    if let Some(on) = st.r.days.get_mut(i) {
                        ui.checkbox(on, tl!(name));
                    }
                }
            });
        }
        Freq::Monthly | Freq::Yearly => {
            let yearly = st.r.freq == Freq::Yearly;
            if yearly {
                every(ui, &mut st.interval, "year(s)");
            }
            ui.horizontal(|ui| {
                ui.radio_value(&mut st.r.relative, false, tl!("On day"));
                ui.add(egui::TextEdit::singleline(&mut st.month_day).desired_width(36.0));
                if yearly {
                    month_choice(ui, &mut st.r.month, "recurrence-month-a");
                }
            });
            ui.horizontal(|ui| {
                ui.radio_value(&mut st.r.relative, true, tl!("On the"));
                egui::ComboBox::from_id_salt("recurrence-nth")
                    .width(90.0)
                    .selected_text(tl!(NTH.iter().find(|(x, _)| *x == st.r.nth).map_or("first", |(_, s)| *s)))
                    .show_ui(ui, |ui| {
                        for (n, name) in NTH {
                            ui.selectable_value(&mut st.r.nth, n, tl!(name));
                        }
                    });
                egui::ComboBox::from_id_salt("recurrence-weekday")
                    .width(110.0)
                    .selected_text(tl!(WEEKDAYS.get(st.r.weekday).copied().unwrap_or("Monday")))
                    .show_ui(ui, |ui| {
                        for (i, name) in WEEKDAYS.iter().enumerate() {
                            ui.selectable_value(&mut st.r.weekday, i, tl!(name));
                        }
                    });
                if yearly {
                    month_choice(ui, &mut st.r.month, "recurrence-month-b");
                }
            });
            if !yearly {
                every(ui, &mut st.interval, "month(s)");
            }
        }
    }
}

fn month_choice(ui: &mut Ui, month: &mut u32, id: &str) {
    let name = |m: u32| crate::i18n::tc("month", crate::calendar::MONTHS.get(m.clamp(1, 12) as usize - 1).copied().unwrap_or(""));
    egui::ComboBox::from_id_salt(id).width(110.0).selected_text(name(*month)).show_ui(ui, |ui| {
        for m in 1..=12 {
            ui.selectable_value(month, m, name(m));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-27, a Tuesday.
    fn tuesday() -> i64 {
        days_from_civil(2026, 10, 27)
    }

    /// Every kind the dialog shows goes to a rule and comes back the same, and what Outlook and
    /// Exchange write reads as the dialog would set it.
    #[test]
    fn every_kind_both_ways() {
        let day = tuesday();
        for rule in [
            "FREQ=DAILY",
            "FREQ=DAILY;INTERVAL=3;COUNT=10",
            "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR",
            "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;UNTIL=20261231",
            "FREQ=MONTHLY;BYMONTHDAY=27",
            "FREQ=MONTHLY;INTERVAL=2;BYDAY=4TU",
            "FREQ=MONTHLY;BYDAY=-1FR;COUNT=6",
            "FREQ=YEARLY;BYMONTH=3;BYMONTHDAY=14",
            "FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU",
        ] {
            let r = Recurrence::from_rule(rule, day).unwrap_or_else(|| panic!("{rule}"));
            assert_eq!(r.rule(), rule, "{r:?}");
        }
        let weekdays = Recurrence::from_rule("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR", day).unwrap();
        assert!(weekdays.freq == Freq::Daily && weekdays.weekdays_only, "Outlook's every weekday");
        // Its own: weekly on the day it starts, the day of the month and the month ready.
        let usual = Recurrence::starting(day);
        assert_eq!((usual.rule().as_str(), usual.month_day, usual.nth, usual.weekday, usual.month), ("FREQ=WEEKLY;BYDAY=TU", 27, 4, 1, 10));
        assert_eq!(Recurrence::starting(days_from_civil(2026, 10, 30)).nth, -1, "the 30th is the last Friday");
        // What it can't show is left alone.
        for odd in ["FREQ=MONTHLY;BYDAY=MO,TU;BYSETPOS=-1", "FREQ=HOURLY", "FREQ=WEEKLY;BYDAY=2MO", "garbage", "FREQ=DAILY;INTERVAL=0"] {
            assert!(Recurrence::from_rule(odd, day).is_none(), "{odd}");
        }
    }

    /// The dialog's fields read into a rule; nonsense keeps OK greyed out.
    #[test]
    fn the_dialog_reads_what_was_typed() {
        let mut st = RecurrenceUi::open("weekly", tuesday());
        assert_eq!(st.read().map(|r| r.rule()).as_deref(), Some("FREQ=WEEKLY;BYDAY=TU"));
        st.interval = "2".into();
        st.r.days = [true, false, true, false, false, false, false];
        st.r.ends = Ends::After(0);
        st.count = "5".into();
        assert_eq!(st.read().map(|r| r.rule()).as_deref(), Some("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=5"));
        st.interval = "zero".into();
        assert!(st.read().is_none());
        st.interval = "1".into();
        st.r.days = [false; 7];
        assert!(st.read().is_none(), "a week with no day");
        let st = RecurrenceUi::open("FREQ=MONTHLY;BYDAY=-1FR;UNTIL=20270331", tuesday());
        assert!(st.r.relative && st.r.nth == -1 && st.r.weekday == 4 && st.until == "2027-03-31");
        assert!(!st.r.describe().is_empty());
    }

    /// The appointment moves to the first day the repeat falls on, as Outlook moves it.
    #[test]
    fn it_starts_on_its_first_occurrence() {
        let day = tuesday();
        let rule = |r: &str| Recurrence::from_rule(r, day).unwrap();
        let first = |r: &str| civil_from_days(rule(r).first_from(day));
        assert_eq!(first("FREQ=WEEKLY;BYDAY=TU"), (2026, 10, 27), "already on it");
        assert_eq!(first("FREQ=WEEKLY;BYDAY=MO,FR"), (2026, 10, 30));
        assert_eq!(first("FREQ=MONTHLY;BYMONTHDAY=3"), (2026, 11, 3));
        assert_eq!(first("FREQ=MONTHLY;BYDAY=2TU"), (2026, 11, 10));
        assert_eq!(first("FREQ=MONTHLY;BYDAY=-1FR"), (2026, 10, 30));
        assert_eq!(first("FREQ=MONTHLY;BYDAY=4TU"), (2026, 10, 27), "the 27th is the fourth Tuesday");
        assert_eq!(first("FREQ=MONTHLY;BYDAY=-1TU"), (2026, 10, 27), "and the last");
        assert_eq!(first("FREQ=YEARLY;BYMONTH=3;BYMONTHDAY=14"), (2027, 3, 14));
        assert_eq!(first("FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU"), (2027, 10, 31), "this October's is past");
        assert_eq!(civil_from_days(rule("FREQ=DAILY").first_from(days_from_civil(2026, 10, 31))), (2026, 10, 31));
        let weekdays = rule("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR");
        assert_eq!(civil_from_days(weekdays.first_from(days_from_civil(2026, 10, 31))), (2026, 11, 2), "Saturday to Monday");
        // Day 31 every month: November has none, so December.
        assert_eq!(civil_from_days(rule("FREQ=MONTHLY;BYMONTHDAY=31").first_from(days_from_civil(2026, 11, 1))), (2026, 12, 31));
    }
}
