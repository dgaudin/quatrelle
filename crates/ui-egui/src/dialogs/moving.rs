//! Moving an appointment by drag and drop: what Outlook asks before it moves — one occurrence
//! of a series or the whole series, and whether the guests of a meeting the user organizes are
//! sent the update — then `event.move`.

use super::*;

/// An appointment dragged to another time, waiting on the user's answers.
#[derive(Clone, Debug, Default)]
pub struct MoveForm {
    pub uid: String,
    pub summary: String,
    /// The occurrence dragged (its start as the series' rule has it), for moving it alone.
    pub occurrence: i64,
    pub recurring: bool,
    /// One occurrence can move on its own in this account (not on Microsoft 365, which reads
    /// whole series back).
    pub occurrence_allowed: bool,
    /// Where it goes (the event's own times).
    pub start: i64,
    pub end: i64,
    /// A meeting the user organizes: the guests may be told.
    pub meeting: bool,
    /// The change can be saved without telling them (not on Microsoft 365, whose server tells
    /// them whatever is asked).
    pub quiet_allowed: bool,
    /// The first answer: this occurrence alone (`Some(true)`) or the series.
    pub this_occurrence: Option<bool>,
    pub error: Option<String>,
}

impl MoveForm {
    /// The command's parameters, once the answers are in.
    pub(crate) fn params(&self, send: bool) -> serde_json::Value {
        let mut p = json!({"uid": self.uid, "start": self.start, "end": self.end, "send": send});
        if self.this_occurrence == Some(true) {
            p["occurrence"] = json!(self.occurrence);
        }
        p
    }
}

/// The questions, one after the other; whether the dialog stays open.
pub(super) fn draw(ui: &mut Ui, f: &mut MoveForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(440.0);
    let mut keep = true;
    if f.recurring && f.this_occurrence.is_none() {
        title(ui, t, "Move this occurrence or the whole series?");
        ui.label(RichText::new(&f.summary).color(t.dim));
        if !f.occurrence_allowed {
            ui.add_space(4.0);
            ui.label(RichText::new(tl!("On Microsoft 365, only the whole series can be moved from here.")).color(t.dim).small());
        }
        ui.add_space(10.0);
        let mut picked = None;
        button_row(ui, |ui| {
            if ui.button(tl!("Cancel")).clicked() {
                keep = false;
            }
            if widgets::primary_button(ui, "The whole series", t, true).clicked() {
                picked = Some(false);
            }
            if f.occurrence_allowed && ui.button(tl!("This occurrence")).clicked() {
                picked = Some(true);
            }
        });
        if let Some(one) = picked {
            f.this_occurrence = Some(one);
            // Nobody to tell: done.
            if !f.meeting {
                acts.push(Action::Submit("event.move", f.params(true)));
            }
        }
    } else if f.meeting {
        title(ui, t, "Send the update to the attendees?");
        ui.label(RichText::new(&f.summary).color(t.dim));
        if !f.quiet_allowed {
            ui.add_space(4.0);
            ui.label(RichText::new(tl!("Microsoft 365 tells the attendees of the change itself.")).color(t.dim).small());
        }
        ui.add_space(10.0);
        button_row(ui, |ui| {
            if ui.button(tl!("Cancel")).clicked() {
                keep = false;
            }
            if widgets::primary_button(ui, "Send update", t, true).clicked() {
                acts.push(Action::Submit("event.move", f.params(true)));
            }
            if f.quiet_allowed && ui.button(tl!("Save without sending")).clicked() {
                acts.push(Action::Submit("event.move", f.params(false)));
            }
        });
    } else {
        // Nothing to ask (the dialog was opened with the answers given).
        acts.push(Action::Submit("event.move", f.params(true)));
    }
    if let Some(e) = &f.error {
        ui.add_space(6.0);
        ui.label(RichText::new(e).color(t.danger));
    }
    keep
}
