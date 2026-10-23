//! A plain-text message as it reads: selectable as a whole, the web and mail addresses written
//! in it (`https://…`, `www.…`, `name@host`) shown as links and opened with a click, as
//! Outlook makes them links.

use super::*;

/// A plain-text body, in the reader's font, its addresses as links.
pub(super) fn plain_body(t: &Tokens, ui: &mut Ui, text: &str, acts: &mut Vec<Action>) {
    let links = mime::find_links(text);
    // The editor counts characters, the text's ranges bytes: each address in characters too.
    let mut chars: Vec<std::ops::Range<usize>> = Vec::with_capacity(links.len());
    let (mut byte, mut count) = (0usize, 0usize);
    for (r, _) in &links {
        count += text.get(byte..r.start).map_or(0, |s| s.chars().count());
        let len = text.get(r.clone()).map_or(0, |s| s.chars().count());
        chars.push(count..count + len);
        count += len;
        byte = r.end;
    }
    let font = theme::regular(14.5);
    let (fg, accent) = (t.text, t.accent);
    let ranges: Vec<std::ops::Range<usize>> = links.iter().map(|(r, _)| r.clone()).collect();
    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let s = buf.as_str();
        let plain = egui::TextFormat { font_id: font.clone(), color: fg, ..Default::default() };
        let link = egui::TextFormat { font_id: font.clone(), color: accent, underline: egui::Stroke::new(1.0, accent), ..Default::default() };
        let mut job = egui::text::LayoutJob::default();
        let mut at = 0;
        for r in &ranges {
            if let (Some(before), Some(address)) = (s.get(at..r.start), s.get(r.clone())) {
                job.append(before, 0.0, plain.clone());
                job.append(address, 0.0, link.clone());
                at = r.end;
            }
        }
        job.append(s.get(at..).unwrap_or(""), 0.0, plain);
        job.wrap.max_width = wrap;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let mut shown = text;
    let out = egui::TextEdit::multiline(&mut shown).frame(egui::Frame::NONE).desired_width(f32::INFINITY).layouter(&mut layouter).show(ui);
    if links.is_empty() {
        return;
    }
    // Where each address starts, for the tests to point at.
    #[cfg(test)]
    {
        let at: Vec<egui::Rect> =
            chars.iter().map(|r| out.galley.pos_from_cursor(egui::text::CCursor::new(r.start)).translate(out.galley_pos.to_vec2())).collect();
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("plain-links"), at));
    }
    // The address under the pointer: a hand, its address told, opened on a click (a drag still
    // selects, as in the rest of the text).
    let under = |p: egui::Pos2| {
        let at = out.galley.cursor_from_pos(p - out.galley_pos).index.0;
        chars.iter().position(|r| r.contains(&at)).and_then(|i| links.get(i)).map(|(_, url)| url.clone())
    };
    let hovered = out.response.hover_pos().and_then(under);
    if let Some(url) = &hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        egui::Response::clone(&out.response).on_hover_text_at_pointer(url);
    }
    if out.response.clicked()
        && let Some(url) = out.response.interact_pointer_pos().and_then(under)
    {
        acts.push(Action::OpenUrl(url));
    }
}
