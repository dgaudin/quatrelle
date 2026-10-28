//! Files dropped on the main window where nothing is being written: a new message with them
//! attached, as Outlook starts one (from upstream 865a712). Files dropped on a message being
//! written are its own (`reading::compose`), and files copied in a file manager are attached as
//! they're pasted into one.

use crate::*;

/// Most files taken from one drop.
const MAX_FILES: usize = 100;

impl MailApp {
    /// The main window's files from the desktop, with no message written in it: while they're
    /// held over it, it says what letting go does; let go, a new message with them attached.
    pub(crate) fn files_dropped_on_window(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        let writing_here = self.session.compose.as_ref().is_some_and(|c| compose_inline(c, ctx));
        // Somewhere else to put them first: the message written here, the File page, a dialog,
        // no account yet.
        if writing_here
            || self.session.module != Module::Mail
            || self.ui.backstage.is_some()
            || self.ui.dialog.is_some()
            || self.session.accounts.is_empty()
        {
            return;
        }
        let (held, dropped) = ctx.input(|i| (!i.raw.hovered_files.is_empty(), i.raw.dropped_files.clone()));
        if held {
            let rect = ctx.content_rect();
            let t = self.ui.tokens;
            let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("files-held-over")));
            p.rect_stroke(rect.shrink(3.0), 6.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
            let g = p.layout_no_wrap(tl!("Drop to attach them to a new message").to_string(), theme::regular(15.0), t.text);
            let card = egui::Rect::from_center_size(rect.center(), g.size() + vec2(32.0, 20.0));
            p.rect(card, 8.0, t.surface, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
            p.galley(card.center() - g.size() / 2.0, g, t.text);
        }
        let files: Vec<String> = dropped
            .iter()
            .filter(|f| f.path().is_absolute() && f.path().is_file())
            .take(MAX_FILES)
            .map(|f| f.path().to_string_lossy().to_string())
            .collect();
        // Folders aren't attached: none, no message.
        if files.is_empty() {
            return;
        }
        acts.push(Action::Run("mail.new", json!({})));
        for f in files {
            acts.push(Action::Run("compose.attach", json!({"path": f})));
        }
    }
}
