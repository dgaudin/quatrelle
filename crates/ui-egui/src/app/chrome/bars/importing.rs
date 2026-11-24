// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The status bar's line for a data file coming in: where it is, a thin bar when the messages
//! were counted, and a button to stop it.

use crate::*;

/// The words of an import's line: `{name}: {folder} — {done} of {total} messages`, shorter when
/// the folder isn't known yet or the messages weren't counted.
pub(crate) fn import_text(name: &str, i: &quatrelle_engine::ImportState) -> String {
    let (done, folder) = (i.done, i.folder.as_str());
    if i.stopping {
        return tf!("{name}: stopping the import…", name = name);
    }
    match (i.total, folder.is_empty()) {
        (Some(total), false) => tf!("{name}: {folder} — {done} of {total} messages", name = name, folder = folder, done = done, total = total),
        (Some(total), true) => tf!("{name}: {done} of {total} messages", name = name, done = done, total = total),
        (None, false) => tf!("{name}: {folder} — {done} messages", name = name, folder = folder, done = done),
        (None, true) => tf!("{name}: {done} messages", name = name, done = done),
    }
}

/// A line for each import running, in the status bar's left part; the accounts whose Stop button
/// was clicked.
pub(crate) fn import_lines(ui: &mut Ui, s: &Session, t: &Tokens) -> Vec<String> {
    let mut stop = Vec::new();
    for a in &s.accounts {
        let Some(i) = s.import_of(&a.config.id) else { continue };
        let text = import_text(&a.config.email, i);
        ui.label(RichText::new(text.clone()).color(t.accent).small());
        if let Some(total) = i.total.filter(|n| *n > 0) {
            // Counts are cast to floats for a fraction only: a few million fit exactly.
            let fraction = (i.done.min(total) as f32 / total as f32).clamp(0.0, 1.0);
            let bar = ui.add(egui::ProgressBar::new(fraction).desired_width(90.0).desired_height(5.0).fill(t.accent));
            bar.widget_info(|| {
                let mut info = egui::WidgetInfo::labeled(egui::Role::ProgressIndicator, true, &text);
                info.value = Some((f64::from(fraction) * 100.0).floor());
                info
            });
        }
        let button = ui.add_enabled(!i.stopping, egui::Button::new(RichText::new(tl!("Stop")).small()).small());
        widgets::named(ui, &button, &tf!("Stop the import of {name}", name = a.config.email), !i.stopping, None);
        if button.on_hover_text(tl!("Stop the import")).clicked() {
            stop.push(a.config.id.clone());
        }
    }
    stop
}
