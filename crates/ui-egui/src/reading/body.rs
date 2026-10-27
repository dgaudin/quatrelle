//! The reading pane: a message's text, laid out as its HTML lays it out, with its pictures.

use super::*;

mod table;
use table::*;

/// Above an HTML message: whether its web pictures are hidden, and a switch to plain text.
pub(super) fn html_bar(s: &Session, t: &Tokens, ui: &mut Ui, p: &mime::Parsed, acts: &mut Vec<Action>) {
    let web = s.remote_pictures();
    let formatted = s.prefs.show_html && !p.rich.is_empty();
    if !web.is_empty() && !s.pictures_allowed() && formatted {
        egui::Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(
            ui,
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    icons::paint(ui.painter(), ir, Icon::Image, t.dim);
                    ui.label(RichText::new(tl!("Pictures from the web are hidden to protect your privacy.")).color(t.dim));
                    if ui.link(tl!("Show pictures")).clicked() {
                        acts.push(Action::Run("message.showPictures", json!({})));
                    }
                    let who = p.from.email.clone();
                    if !who.is_empty() && ui.link(tf!("Always show from {sender}", sender = who)).clicked() {
                        acts.push(Action::Run("message.showPictures", json!({"always": true})));
                    }
                });
            },
        );
        ui.add_space(6.0);
    }
    // What reports back to the sender: never-loaded pictures, links that count clicks.
    let tr = &p.trackers;
    if !tr.is_empty() {
        let line = match (tr.pictures.len(), tr.links) {
            (0, links) => tf!("Links that report clicks: {links}", links = links),
            (pictures, 0) => tf!("Tracking pictures, never loaded: {pictures}", pictures = pictures),
            (pictures, links) => {
                tf!("Tracking pictures, never loaded: {pictures} · Links that report clicks: {links}", pictures = pictures, links = links)
            }
        };
        let about = if s.prefs.clean_links {
            tl!("Links open without their tracking tags, and out of redirects that carry the real address")
        } else {
            tl!("Links open as they are (File › Options › Safety)")
        };
        ui.horizontal_wrapped(|ui| {
            let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            icons::paint(ui.painter(), ir, Icon::Info, t.faint);
            ui.label(RichText::new(line).color(t.faint).small()).on_hover_text(about);
        });
    }
    ui.horizontal(|ui| {
        if formatted {
            if ui.link(RichText::new(tl!("Show as plain text")).small()).clicked() {
                acts.push(Action::Run("view.html", json!({"on": false})));
            }
            if !web.is_empty() && s.pictures_allowed() && s.prefs.trusted_senders.contains(&p.from.email.to_ascii_lowercase()) {
                ui.label(RichText::new("·").color(t.faint).small());
                if ui.link(RichText::new(tl!("Stop showing pictures from this sender")).small()).clicked() {
                    acts.push(Action::Run("message.blockPictures", json!({})));
                }
            }
        } else if !p.rich.is_empty() && ui.link(RichText::new(tl!("Show formatted")).small()).clicked() {
            acts.push(Action::Run("view.html", json!({"on": true})));
        } else if p.rich.is_empty() {
            ui.label(RichText::new(tl!("Shown as text.")).color(t.faint).small());
        }
    });
    ui.add_space(6.0);
}

/// A message that hides a lot of text from its reader: said, with the text to look at.
pub(super) fn hidden_bar(t: &Tokens, ui: &mut Ui, key: &MessageKey, p: &mime::Parsed) {
    let id = egui::Id::unique(("hidden-text", key.to_id()));
    let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    egui::Frame::new().fill(t.warn.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Warning, t.warn);
            ui.vertical(|ui| {
                ui.label(RichText::new(tf!("This message hides text from you ({n} words).", n = p.hidden_words)).color(t.text));
                ui.label(
                    RichText::new(tl!(
                        "Hidden text is a way to slip past spam filters, or to give instructions to tools and assistants that read mail for you."
                    ))
                    .color(t.dim)
                    .small(),
                );
                if ui.link(RichText::new(if open { tl!("Hide it again") } else { tl!("Show the hidden text") }).small()).clicked() {
                    open = !open;
                }
            });
        });
        if open {
            ui.add_space(6.0);
            egui::ScrollArea::vertical().id_salt(id).max_height(180.0).show(ui, |ui| {
                let mut text = p.hidden_text.as_str();
                ui.add(
                    egui::TextEdit::multiline(&mut text).font(egui::FontId::monospace(12.0)).desired_width(f32::INFINITY).frame(egui::Frame::NONE),
                );
            });
        }
    });
    ui.data_mut(|d| d.insert_temp(id, open));
    ui.add_space(8.0);
}

/// An HTML message, formatted: emphasis, code, links (opened in the browser on click) and
/// pictures: those inside the message always, those on the web only once allowed. Mail laid out
/// with tables keeps its columns ([`mime::layout`]).
pub(super) fn rich_body(s: &Session, st: &mut UiState, ui: &mut Ui, key: &MessageKey, p: &mime::Parsed, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let allowed = s.pictures_allowed();
    let mut shown: Vec<String> = Vec::new();
    let width = ui.available_width();
    // The message's typefaces, loaded once each (it reads in them from the next frame).
    for name in crate::look::typefaces::wanted(ui.ctx(), message_typefaces(p)) {
        acts.push(Action::LoadTypeface(name));
    }
    let faces = crate::look::typefaces::loaded(ui.ctx());
    let zoom = s.prefs.reading_scale();
    let mut body = Body { s, key, p, allowed, shown: &mut shown, acts: &mut *acts, faces, zoom, inline: &[], cells: 0 };
    if p.layout.is_empty() {
        body.lines(st, ui, &p.rich, width, theme::surface_rgb(&t), Align::Min);
    } else {
        body.blocks(st, ui, &p.layout, width, theme::surface_rgb(&t), mime::layout::Align::Left, 0);
    }
    // Pictures without a description have no place in the text: show them after it.
    if allowed {
        let rest: Vec<&String> = p.images.iter().filter(|u| !shown.contains(u)).collect();
        if !rest.is_empty() {
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                for src in rest {
                    picture(s, st, ui, key, p, src, allowed, width, zoom, acts);
                }
            });
        }
    }
}

/// What drawing a message body needs.
pub(super) struct Body<'a> {
    pub(super) s: &'a Session,
    pub(super) key: &'a MessageKey,
    pub(super) p: &'a mime::Parsed,
    pub(super) allowed: bool,
    /// Pictures drawn in place.
    pub(super) shown: &'a mut Vec<String>,
    pub(super) acts: &'a mut Vec<Action>,
    /// The typefaces loaded, to draw the message's own in ([`span_font_in`]).
    pub(super) faces: std::collections::BTreeSet<String>,
    /// The message's zoom (1.0 as written): its text, pictures and set widths ([`Prefs::reading_scale`]).
    pub(super) zoom: f32,
    /// Pictures carried along with what's drawn (a signature's), by their `cid:`.
    pub(super) inline: &'a [mime::InlinePicture],
    /// Table cells drawn so far: each one's number, the same from frame to frame, keys its
    /// height as last drawn ([`CellHeights`]) for its row's alignment.
    pub(super) cells: usize,
}

/// The heights of the table cells of the messages drawn (their content and padding, as last
/// drawn, by message and cell number): a row aligns its cells to its tallest on the next frame
/// from what this frame measured, which costs nothing per frame beyond a lookup. A few
/// messages at a time (the reading pane and the windows open), so much as a newsletter holds.
#[derive(Default)]
pub(crate) struct CellHeights {
    by_message: Vec<(MessageKey, Vec<Option<f32>>)>,
}

impl CellHeights {
    const MESSAGES: usize = 8;
    const CELLS: usize = 20_000;

    pub(crate) fn get(&self, key: &MessageKey, cell: usize) -> Option<f32> {
        self.by_message.iter().find(|(k, _)| k == key).and_then(|(_, h)| h.get(cell).copied().flatten())
    }

    fn set(&mut self, key: &MessageKey, cell: usize, height: f32) {
        if cell >= Self::CELLS || !height.is_finite() {
            return;
        }
        if !self.by_message.iter().any(|(k, _)| k == key) {
            if self.by_message.len() >= Self::MESSAGES {
                self.by_message.remove(0);
            }
            self.by_message.push((key.clone(), Vec::new()));
        }
        let Some((_, heights)) = self.by_message.iter_mut().find(|(k, _)| k == key) else { return };
        if heights.len() <= cell {
            heights.resize(cell + 1, None);
        }
        if let Some(slot) = heights.get_mut(cell) {
            *slot = Some(height);
        }
    }
}

/// The typefaces a message's text is set in (`font-family`), a few at most: loaded from the
/// system once each, so it reads in them (a signature in Arial, a body in Calibri).
pub(super) fn message_typefaces(p: &mime::Parsed) -> Vec<String> {
    const MOST: usize = 8;
    fn add(out: &mut Vec<String>, lines: &mime::RichText) {
        for sp in lines.iter().flatten() {
            if out.len() >= MOST {
                return;
            }
            if let Some(f) = &sp.font
                && !out.contains(f)
            {
                out.push(f.clone());
            }
        }
    }
    fn blocks(out: &mut Vec<String>, b: &[mime::layout::Block], depth: usize) {
        for b in b {
            match b {
                mime::layout::Block::Text(lines) => add(out, lines),
                mime::layout::Block::Table(t) if depth < 16 => {
                    for c in t.rows.iter().flatten() {
                        blocks(out, &c.blocks, depth + 1);
                    }
                }
                mime::layout::Block::Table(_) => {}
            }
        }
    }
    let mut out = Vec::new();
    add(&mut out, &p.rich);
    blocks(&mut out, &p.layout, 0);
    out
}

/// A span's font: the size the message gave it (points, as a page shows them), in its typeface
/// when that's loaded (`faces`), else the reader's; `zoom` times bigger.
pub(super) fn span_font_in(sp: &mime::Span, faces: &std::collections::BTreeSet<String>, zoom: f32) -> egui::FontId {
    let mut font = span_font_written(sp, faces);
    font.size *= zoom;
    font
}

fn span_font_written(sp: &mime::Span, faces: &std::collections::BTreeSet<String>) -> egui::FontId {
    if let Some(pt) = sp.size.filter(|_| !sp.code) {
        let px = (f32::from(pt) * 4.0 / 3.0).clamp(8.0, 72.0);
        let bold = sp.bold || sp.heading > 0;
        let own = sp.font.as_deref().and_then(|name| crate::look::typefaces::family(faces, name, bold));
        return match own {
            Some(family) => egui::FontId::new(px, family),
            None if bold => theme::bold(px),
            None => theme::regular(px),
        };
    }
    if let Some(family) =
        sp.font.as_deref().filter(|_| !sp.code && sp.heading == 0).and_then(|name| crate::look::typefaces::family(faces, name, sp.bold))
    {
        return egui::FontId::new(14.5, family);
    }
    if sp.heading > 0 {
        theme::bold(match sp.heading {
            1 => 22.0,
            2 => 19.0,
            3 => 17.0,
            _ => 15.5,
        })
    } else if sp.code {
        egui::FontId::monospace(13.5)
    } else if sp.bold {
        theme::bold(14.5)
    } else {
        theme::regular(14.5)
    }
}

impl Body<'_> {
    /// Formatted lines `width` wide, on a background of `page` (for the text's contrast).
    pub(super) fn lines(&mut self, st: &mut UiState, ui: &mut Ui, lines: &mime::RichText, width: f32, page: [u8; 3], align: Align) {
        let t = st.tokens;
        for line in lines {
            if line.is_empty() {
                ui.add_space(9.0 * self.zoom);
                continue;
            }
            // A line with right-to-left text is laid out whole, egui putting it in order (its links
            // show, but as text).
            if line.iter().any(|sp| crate::bidi::has_rtl(&sp.text)) && line.iter().all(|sp| sp.image.is_none()) {
                let mut job = egui::text::LayoutJob::default();
                for sp in line {
                    let (fg, bg) = theme::mail_colors_on(sp.color, sp.background, page, &t, if sp.link.is_some() { t.accent } else { t.text });
                    let format = egui::TextFormat {
                        font_id: span_font_in(sp, &self.faces, self.zoom),
                        color: fg,
                        background: bg.unwrap_or(Color32::TRANSPARENT),
                        italics: sp.italic,
                        strikethrough: if sp.strike { egui::Stroke::new(1.0, fg) } else { egui::Stroke::NONE },
                        underline: if sp.link.is_some() || sp.underline { egui::Stroke::new(1.0, fg) } else { egui::Stroke::NONE },
                        ..Default::default()
                    };
                    job.append(&sp.text, 0.0, format);
                }
                let r = ui
                    .scope(|ui| {
                        ui.set_max_width(width);
                        crate::bidi::add_selectable_label(ui, job)
                    })
                    .inner;
                let written: String = line.iter().map(|sp| sp.text.as_str()).collect();
                rtl_copy_menu(&r, Some(&written), &self.p.text);
                continue;
            }
            // Centred or right-aligned: pictures in the middle (or at the end), and a line that
            // fits moved over (a wrapped line keeps to the left).
            if align != Align::Min {
                if line.iter().all(|sp| sp.image.is_some() || sp.text.trim().is_empty()) {
                    ui.allocate_ui_with_layout(vec2(width, 0.0), egui::Layout::top_down(align), |ui| {
                        ui.set_width(width);
                        for sp in line {
                            self.span(st, ui, sp, width, page);
                        }
                    });
                    continue;
                }
                let line_w: f32 = line
                    .iter()
                    .map(|sp| ui.fonts_mut(|f| f.layout_no_wrap(sp.text.clone(), span_font_in(sp, &self.faces, self.zoom), Color32::WHITE).size().x))
                    .sum();
                if line.iter().all(|sp| sp.image.is_none()) && line_w < width {
                    let space = if align == Align::Center { (width - line_w) / 2.0 } else { width - line_w };
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.add_space(space.max(0.0));
                        for sp in line {
                            self.span(st, ui, sp, width, page);
                        }
                    });
                    continue;
                }
            }
            let layout = egui::Layout::left_to_right(Align::BOTTOM).with_main_wrap(true);
            ui.allocate_ui_with_layout(vec2(width, 0.0), layout, |ui| {
                ui.set_max_width(width);
                ui.spacing_mut().item_spacing.x = 0.0;
                for sp in line {
                    self.span(st, ui, sp, width, page);
                }
            });
        }
    }

    /// One span: text (a link when it has one) or a picture.
    pub(super) fn span(&mut self, st: &mut UiState, ui: &mut Ui, sp: &mime::Span, width: f32, page: [u8; 3]) {
        let t = st.tokens;
        if let Some(src) = &sp.image {
            self.shown.push(src.clone());
            if let Some(pic) = src.strip_prefix("cid:").and_then(|cid| self.inline.iter().find(|p| p.cid == cid)) {
                carried_picture(st, ui, pic, width, self.zoom);
                return;
            }
            if !picture(self.s, st, ui, self.key, self.p, src, self.allowed, width, self.zoom, self.acts) {
                let alt = if sp.text.trim().is_empty() { "picture" } else { sp.text.trim() };
                ui.label(RichText::new(format!("[{alt}]")).color(t.faint));
            }
            return;
        }
        let mut rt = RichText::new(&sp.text).font(span_font_in(sp, &self.faces, self.zoom));
        if sp.italic {
            rt = rt.italics();
        }
        if sp.strike {
            rt = rt.strikethrough();
        }
        if sp.underline {
            rt = rt.underline();
        }
        let (fg, bg) = theme::mail_colors_on(sp.color, sp.background, page, &t, if sp.link.is_some() { t.accent } else { t.text });
        if let Some(bg) = bg {
            rt = rt.background_color(bg);
        }
        match &sp.link {
            Some(link) => {
                // Hovered, a link says where it will open (cleaned, when links are).
                let shown = if self.s.prefs.clean_links { mime::clean_link(link) } else { link.clone() };
                let r = ui.add(egui::Link::new(rt.color(fg))).on_hover_text(shown);
                if r.clicked() {
                    self.acts.push(Action::OpenUrl(link.clone()));
                }
            }
            None => {
                ui.label(rt.color(fg));
            }
        }
    }

    /// Laid-out parts `width` wide.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn blocks(
        &mut self,
        st: &mut UiState,
        ui: &mut Ui,
        blocks: &[mime::layout::Block],
        width: f32,
        page: [u8; 3],
        align: mime::layout::Align,
        depth: usize,
    ) {
        use mime::layout::{Align as A, Block};
        let main_align = |a: A| match a {
            A::Left => Align::Min,
            A::Center => Align::Center,
            A::Right => Align::Max,
        };
        for b in blocks {
            match b {
                Block::Text(lines) => self.lines(st, ui, lines, width, page, main_align(align)),
                Block::Table(table) if depth < 16 => self.table(st, ui, table, width, page, align, depth),
                Block::Table(_) => {}
            }
        }
    }

    /// A picture to put behind a box, when it can be shown (one inside the message, or one on the
    /// web once pictures are allowed); counted as shown either way, so it isn't added after the text.
    pub(super) fn background(&mut self, st: &mut UiState, ctx: &egui::Context, src: &str) -> Option<egui::TextureHandle> {
        if !self.shown.iter().any(|x| x == src) {
            self.shown.push(src.to_string());
        }
        match picture_texture(self.s, st, ctx, self.key, self.p, src, self.allowed, 1.0) {
            Pic::Ready(tex, _) => Some(tex),
            Pic::Loading | Pic::Missing => None,
        }
    }

    /// A box placed over `over` (its container's rectangle); where it ends at the bottom.
    pub(super) fn overlay(&mut self, st: &mut UiState, ui: &mut Ui, o: &mime::layout::Overlay, over: egui::Rect, page: [u8; 3], depth: usize) -> f32 {
        use mime::layout::Width;
        const PAD: f32 = 4.0;
        let x = o.x as f32 * self.zoom;
        let w = match o.width {
            Some(Width::Px(px)) => px as f32 * self.zoom,
            Some(Width::Percent(pc)) => over.width() * f32::from(pc) / 100.0,
            // As wide as its text (CSS's shrink-to-fit), at most what's left of the box.
            None => natural_width(ui, &o.blocks, self.zoom).map_or(over.width() - x, |n| n + 2.0 * PAD + 1.0).min(over.width() - x).max(24.0),
        }
        .clamp(1.0, over.width().max(1.0));
        let left = if o.from_right { over.right() - x - w } else { over.left() + x }.max(over.left() - w);
        let bg = theme::box_color(o.background, page, &st.tokens);
        let fill = bg.map_or(Color32::TRANSPARENT, |b| Color32::from_rgb(b[0], b[1], b[2]));
        let draw = |this: &mut Self, st: &mut UiState, ui: &mut Ui, top: f32, measure: bool| -> f32 {
            let mut b = egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::pos2(left, top), vec2(w, f32::INFINITY)));
            if measure {
                b = b.sizing_pass().invisible();
            }
            let mut child = ui.new_child(b);
            egui::Frame::new().fill(fill).inner_margin(PAD).show(&mut child, |ui| {
                let inner = (w - 2.0 * PAD).max(1.0);
                ui.set_width(inner);
                this.blocks(st, ui, &o.blocks, inner, bg.unwrap_or(page), mime::layout::Align::Left, depth + 1);
            });
            child.min_rect().height()
        };
        // From the bottom: its height first, out of sight.
        let top = if o.from_bottom {
            let h = draw(self, st, ui, over.top(), true);
            (over.bottom() - o.y as f32 - h).max(over.top())
        } else {
            over.top() + o.y as f32
        };
        let h = draw(self, st, ui, top, false);
        top + h
    }
}

/// A box's frame: its fill, and its corners rounded when the message rounds them (a
/// newsletter's button), with its border as the frame's stroke then, when it's the same all
/// round (`true`: the edges are drawn by the frame, not after it).
fn framed_box(fill: Color32, borders: &mime::layout::Edges, radius: u8, page: [u8; 3], t: &Tokens) -> (egui::Frame, bool) {
    let mut frame = egui::Frame::new().fill(fill);
    let round = radius > 0 && (borders.is_none() || borders.uniform().is_some());
    if round {
        frame = frame.corner_radius(radius);
        if let Some(b) = borders.uniform() {
            frame = frame.stroke(egui::Stroke::new(f32::from(b.width), theme::line_color(b.color, page, t)));
        }
    }
    (frame, round)
}

/// A box's borders along `rect`, on whole pixels: the top and left lines just outside it, the
/// right and bottom ones just inside, so two bordered cells side by side draw one line between
/// them (as a collapsed table does) and a table's own border falls on its cells'.
fn edges(ui: &Ui, rect: Rect, e: &mime::layout::Edges, page: [u8; 3], t: &Tokens) {
    if e.is_none() {
        return;
    }
    let ppp = ui.ctx().pixels_per_point().max(0.1);
    let snap = |v: f32| (v * ppp).round() / ppp;
    let (left, top, right, bottom) = (snap(rect.left()), snap(rect.top()), snap(rect.right()), snap(rect.bottom()));
    let width = |b: Option<mime::layout::Border>| b.map_or(0.0, |b| f32::from(b.width));
    let (tw, lw) = (width(e.top), width(e.left));
    let paint = |r: Rect, b: mime::layout::Border| {
        if r.width() > 0.0 && r.height() > 0.0 {
            ui.painter().rect_filled(r, 0.0, theme::line_color(b.color, page, t));
        }
    };
    if let Some(b) = e.top {
        paint(Rect::from_min_max(egui::pos2(left - lw, top - tw), egui::pos2(right, top)), b);
    }
    if let Some(b) = e.left {
        paint(Rect::from_min_max(egui::pos2(left - lw, top - tw), egui::pos2(left, bottom)), b);
    }
    if let Some(b) = e.right {
        paint(Rect::from_min_max(egui::pos2(right - width(Some(b)), top - tw), egui::pos2(right, bottom)), b);
    }
    if let Some(b) = e.bottom {
        paint(Rect::from_min_max(egui::pos2(left - lw, bottom - width(Some(b))), egui::pos2(right, bottom)), b);
    }
}

/// Right-click on right-to-left text (selectable too): copy the line, or the whole message, as
/// written.
pub(super) fn rtl_copy_menu(r: &egui::Response, line: Option<&str>, whole: &str) {
    r.interact(Sense::click()).context_menu(|ui| {
        if let Some(line) = line
            && ui.button(tl!("Copy")).clicked()
        {
            ui.ctx().copy_text(line.trim().to_string());
            ui.close();
        }
        if ui.button(tl!("Copy the whole message")).clicked() {
            ui.ctx().copy_text(whole.trim().to_string());
            ui.close();
        }
    });
}
