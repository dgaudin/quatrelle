//! The reading pane (the open message with its conversation and attachments) and the inline
//! compose form that replaces it while a message is being written.

use egui::{Align, Color32, Margin, Rect, RichText, Sense, Ui, Vec2, vec2};
use quatrelle_engine::mime::{self, AttachmentKind};
use quatrelle_engine::model::contacts::Contact;
use quatrelle_engine::model::{MessageKey, addr, dates};
use quatrelle_engine::{InviteProblem, Session, model::EmailAddress};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::rich_edit;
use crate::theme::{self, Tokens};
use crate::{Action, UiState, widgets};

mod assistant;
mod attachments;
mod body;
mod check_names;
mod compose;
mod header;
mod people;
mod pictures;
mod plain;
mod security;
mod sender;
use attachments::*;
pub(crate) use attachments::{big_picture, decode};
pub(crate) use body::CellHeights;
use body::*;
pub use compose::compose_window;
#[cfg(test)]
pub(crate) use compose::picture_alone;
use pictures::*;
/// A picture cut to a circle: its middle square is kept (a picture that isn't square would
/// otherwise fill a square box stretched, its circle an oval), then everything outside the
/// circle goes, with a pixel of smoothing at its edge.
fn round_pixels(img: image::RgbaImage) -> ([usize; 2], Vec<u8>) {
    let (w, h) = (img.width(), img.height());
    let side = w.min(h);
    let square = if side == 0 || (w == side && h == side) {
        img
    } else {
        image::imageops::crop_imm(&img, (w - side) / 2, (h - side) / 2, side, side).to_image()
    };
    let (w, h) = (square.width() as f32, square.height() as f32);
    let mut ci = egui::ColorImage::from_rgba_unmultiplied([square.width() as usize, square.height() as usize], square.as_raw());
    let (cx, cy, r) = (w / 2.0, h / 2.0, w.min(h) / 2.0);
    for (i, px) in ci.pixels.iter_mut().enumerate() {
        let (x, y) = ((i % square.width() as usize) as f32 + 0.5, (i / square.width() as usize) as f32 + 0.5);
        let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
        if d > r {
            px[3] = 0;
        } else if d > r - 1.0 {
            px[3] = (f32::from(px[3]) * (r - d)) as u8;
        }
    }
    let bytes = ci.pixels.iter().flat_map(|p| p.to_array()).collect();
    ([square.width() as usize, square.height() as usize], bytes)
}

/// A picture cut to a circle: what a person's picture looks like in a round avatar.
pub(crate) fn decode_round(ctx: &egui::Context, name: &str, data: &[u8], max: u32) -> Option<egui::TextureHandle> {
    decode_with(ctx, name, data, max, true)
}

/// A picture read, within limits: anything bigger than this is hostile, not a picture.
fn decoded(data: &[u8]) -> Option<image::DynamicImage> {
    if data.len() > 30 << 20 {
        return None;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    limits.max_alloc = Some(256 << 20);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format().ok()?;
    reader.limits(limits);
    reader.decode().ok()
}

/// A picture's pixels, whole (to copy it).
pub(crate) fn color_image(data: &[u8]) -> Option<egui::ColorImage> {
    let img = decoded(data)?.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
}

/// Pixels pasted (a screenshot) as a PNG, to go in a message.
pub(crate) fn png_of(img: &egui::ColorImage) -> Option<Vec<u8>> {
    let [w, h] = img.size;
    if w == 0 || h == 0 || w > 12_000 || h > 12_000 {
        return None;
    }
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect();
    let rgba = image::RgbaImage::from_raw(u32::try_from(w).ok()?, u32::try_from(h).ok()?, bytes)?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(rgba).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}

fn decode_with(ctx: &egui::Context, name: &str, data: &[u8], max: u32, round: bool) -> Option<egui::TextureHandle> {
    decode_sized_with(ctx, name, data, max, round).map(|(tex, _)| tex)
}

/// [`attachments::decode`], with the picture's own size (in pixels, the texture's being smaller
/// when it was shrunk).
pub(crate) fn decode_sized(ctx: &egui::Context, name: &str, data: &[u8], max: u32) -> Option<(egui::TextureHandle, Vec2)> {
    decode_sized_with(ctx, name, data, max, false)
}

fn decode_sized_with(ctx: &egui::Context, name: &str, data: &[u8], max: u32, round: bool) -> Option<(egui::TextureHandle, Vec2)> {
    let decoded = decoded(data)?;
    let own = vec2(decoded.width() as f32, decoded.height() as f32);
    // Never bigger than the graphics card takes (a software or browser one may take 2048 only).
    let max = max.min(ctx.input(|i| i.max_texture_side).try_into().unwrap_or(u32::MAX)).max(1);
    // Shrink large pictures; never enlarge small ones (the painter scales them crisply).
    let img = if decoded.width() > max || decoded.height() > max { decoded.thumbnail(max, max) } else { decoded }.to_rgba8();
    let (size, pixels) = if round { round_pixels(img) } else { ([img.width() as usize, img.height() as usize], img.into_raw()) };
    let ci = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    // Pixel art and tiny swatches stay crisp when scaled up.
    let filter = if size[0].max(size[1]) < 96 { egui::TextureOptions::NEAREST } else { egui::TextureOptions::LINEAR };
    Some((ctx.load_texture(name, ci, filter), own))
}

/// HTML written by hand (a signature) as a message shows it: its layout, text and links, pictures
/// on the web only as their place (nothing is loaded). The text above is its plain twin.
pub fn html_preview(s: &Session, st: &mut UiState, ui: &mut Ui, html: &str) {
    let raw = format!("Content-Type: text/html; charset=utf-8\r\n\r\n{html}");
    let Some(p) = mime::parse(raw.as_bytes()) else { return };
    let t = st.tokens;
    let key = MessageKey::default();
    let mut shown: Vec<String> = Vec::new();
    let mut acts: Vec<Action> = Vec::new();
    let width = ui.available_width();
    let faces = crate::look::typefaces::loaded(ui.ctx());
    let mut body = Body { s, key: &key, p: &p, allowed: false, shown: &mut shown, acts: &mut acts, faces, zoom: 1.0, cells: 0 };
    if p.layout.is_empty() {
        body.lines(st, ui, &p.rich, width, theme::surface_rgb(&t), Align::Min);
    } else {
        body.blocks(st, ui, &p.layout, width, theme::surface_rgb(&t), mime::layout::Align::Left, 0);
    }
}

/// Voting buttons: Outlook's usual sets, in the interface's language, or the sender's own,
/// separated by semicolons.
pub(crate) fn voting_menu(ui: &mut Ui, t: &Tokens, c: &mut quatrelle_engine::Compose) {
    ui.set_min_width(260.0);
    for set in [["Approve", "Reject", ""], ["Yes", "No", ""], ["Yes", "No", "Maybe"]] {
        let choices: Vec<String> = set.iter().filter(|x| !x.is_empty()).map(|x| tl!(x).to_string()).collect();
        let joined = choices.join(";");
        if ui.radio(c.voting == joined, choices.join(" · ")).clicked() {
            c.voting = joined;
            ui.close();
        }
    }
    ui.separator();
    ui.label(RichText::new(tl!("Your own choices, separated by semicolons")).color(t.dim).small());
    ui.add(egui::TextEdit::singleline(&mut c.voting).desired_width(240.0).char_limit(500));
    if !c.voting.is_empty() && ui.button(tl!("No voting buttons")).clicked() {
        c.voting.clear();
        ui.close();
    }
}

/// A one-line field whose misspelt words are underlined in red, as the message's are (the
/// subject: the words people read first), checked as `check` says (the message's language).
fn underlined_spelling(ui: &mut Ui, t: &Tokens, id: egui::Id, value: &mut String, hint: &str, check: &crate::spellcheck::Check) -> egui::Response {
    let colour = t.danger;
    let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let text = buf.as_str().to_string();
        let font = egui::TextStyle::Body.resolve(ui.style());
        let mut job = egui::text::LayoutJob::simple(text.clone(), font, ui.visuals().text_color(), wrap);
        crate::spellcheck::underline_with(&mut job, &text, colour, check);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    underlined(ui, t, egui::TextEdit::singleline(value).id(id).hint_text(tl!(hint)).layouter(&mut layouter))
}

pub(crate) use compose::delay_menu;
#[cfg(test)]
pub(crate) use compose::parse_local;
use compose::*;
use people::*;
use sender::*;

pub fn show(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    if s.compose.as_ref().is_some_and(|c| crate::compose_inline(c, ui.ctx())) {
        compose(s, st, ui, acts, false);
        return;
    }
    let t = st.tokens;
    if s.reading.is_none() {
        empty(ui, &t, if s.list.is_empty() { "" } else { tl!("Select a message to read it") });
        return;
    }
    // A message just opened comes in: fading up from a few pixels below, quickly.
    let key = s.reading.as_ref().map(|r| r.key.to_id()).unwrap_or_default();
    let k = crate::motion::entrance(ui.ctx(), egui::Id::unique(("reading", key)), crate::motion::NORMAL);
    ui.scope(|ui| {
        ui.multiply_opacity(k);
        ui.add_space(6.0 * (1.0 - k));
        message(s, st, ui, acts, false);
    });
}

/// A message in a window of its own. What it asks for applies to that message: it becomes the
/// open one first.
pub fn window(s: &mut Session, st: &mut UiState, ui: &mut Ui, w: quatrelle_engine::Reading, acts: &mut Vec<Action>) {
    let key = w.key.clone();
    let saved = s.reading.replace(w);
    let mut mine = Vec::new();
    message(s, st, ui, &mut mine, true);
    s.reading = saved;
    if !mine.is_empty() {
        acts.push(Action::Open(key));
        acts.extend(mine);
    }
}

/// The sender's voting buttons, above the message: a button per choice, which sends that answer
/// at once; once answered, which one (and the buttons stay, to vote again).
fn vote_bar(ui: &mut Ui, t: &Tokens, key: &MessageKey, choices: &[String], voted: &str, sending: Option<&str>, acts: &mut Vec<Action>) {
    egui::Frame::new().fill(t.accent_soft).corner_radius(6).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), ir, Icon::Info, t.accent);
            let text = match sending {
                Some(c) => tf!("Sending your vote: {choice}…", choice = c),
                None if !voted.is_empty() => tf!("You voted: {choice}. Another button sends a new vote.", choice = voted),
                None => tl!("The sender asks for a vote. Choose your answer:").to_string(),
            };
            ui.label(RichText::new(text).color(t.text));
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for c in choices {
                let chosen = c == voted;
                let label = RichText::new(c).color(if chosen { t.on_accent } else { t.text });
                let b = egui::Button::new(label).fill(if chosen { t.accent } else { t.surface }).stroke(egui::Stroke::new(1.0, t.accent));
                if ui.add_enabled(sending.is_none(), b).on_hover_text(tl!("Send this answer to the sender")).clicked() {
                    acts.push(Action::Run("mail.vote", json!({"key": key.to_id(), "choice": c})));
                }
            }
        });
    });
}

fn message(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>, in_window: bool) {
    let t = st.tokens;
    let Some(r) = s.reading.clone() else { return };
    let summary = s.summary(&r.key).cloned();
    // Voting buttons the sender put on it, the answer sent from here, and one on its way.
    let voting = s.voting(&r.key);
    let vote_sending = s.vote_sending.as_ref().filter(|(k, _)| *k == r.key).map(|(_, c)| c.clone());
    // An attachment shown in place of the text (its chip clicked); Escape goes back.
    let viewing = st
        .attachment_view
        .as_ref()
        .filter(|(k, _)| *k == r.key)
        .map(|(_, i)| *i)
        .filter(|i| r.parsed.as_ref().is_some_and(|p| p.attachments.iter().any(|a| a.index == *i)));
    if viewing.is_some() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        close_attachment_view(st);
    }
    let viewing = viewing.filter(|_| st.attachment_view.is_some());
    // As Outlook's, the header stays put while the message scrolls under it: the subject, who
    // it's from and to, and what can be done with it (scrolling itself when it's long).
    let headed = viewing.is_none() && r.parsed.is_some();
    if let Some(p) = r.parsed.as_ref().filter(|_| headed) {
        let most = (ui.available_height() * 0.45).max(140.0);
        egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(most).id_salt(("reading-head", in_window, r.key.to_id())).show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin { left: 24, right: 24, top: 18, bottom: 4 }).show(ui, |ui| {
                header::header(s, t, ui, &r, p, &summary, in_window, acts);
            });
        });
    }
    let top = ui.cursor().top();
    // Zoomed in, a picture can be wider than the pane: it scrolls sideways (the text still wraps).
    let zoomed = s.prefs.reading_scale() > 1.0;
    let scrolled = egui::ScrollArea::new([zoomed, true]).auto_shrink(false).id_salt(("reading", in_window, r.key.to_id(), viewing)).show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin { left: 24, right: 24, top: if headed { 8 } else { 18 }, bottom: 24 }).show(ui, |ui| {
            let Some(p) = &r.parsed else {
                ui.add_space(30.0);
                match &r.error {
                    Some(e) => {
                        ui.horizontal(|ui| {
                            let (ir, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                            icons::paint(ui.painter(), ir, Icon::Warning, t.warn);
                            ui.label(RichText::new(tf!("This message couldn't be downloaded: {error}", error = e)).color(t.dim));
                        });
                        if ui.button(tl!("Try again")).clicked() {
                            acts.push(Action::Open(r.key.clone()));
                        }
                    }
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new(tl!("Downloading the message…")).color(t.dim));
                        });
                    }
                }
                return;
            };
            // Previewing an attachment: back to the message, and what can be done with the file.
            if let Some(index) = viewing
                && let Some(a) = p.attachments.iter().find(|a| a.index == index)
            {
                preview_header(&t, ui, a, acts, &mut st.attachment_view);
                if st.attachment_view.is_none() {
                    close_attachment_view(st);
                }
            }
            if viewing.is_none() {
                // Conversation.
                // Not with View › Conversations off.
                if r.thread.len() > 1 && !in_window && s.prefs.conversations {
                    thread_list(s, &t, ui, &r.thread, &r.key, acts);
                    ui.add_space(10.0);
                }

                assistant::panel(s, &t, ui, &r.key, acts);
                if let Some((choices, voted)) = &voting {
                    vote_bar(ui, &t, &r.key, choices, voted, vote_sending.as_deref(), acts);
                    ui.add_space(8.0);
                }
                if !in_window {
                    invite_card(s, st, ui, &r.key, acts);
                }
            }

            // An encrypted message not decrypted: nothing of it (its smime.p7m) is shown.
            if r.security.as_ref().is_some_and(security::still_locked) {
                return;
            }

            // Attachments.
            // Pictures the formatted message shows in place aren't listed again as files.
            let formatted = s.prefs.show_html && !p.rich.is_empty();
            let shown: Vec<usize> = if formatted { p.images.iter().filter_map(|i| mime::picture_attachment(p, i)).collect() } else { Vec::new() };
            let in_place = |a: &mime::AttachmentInfo| shown.contains(&a.index);
            // An invitation is shown as its card above, not as a file.
            let invitation = st.invite.as_ref().is_some_and(|(k, _, i)| k == &r.key && i.is_some()) && !in_window;
            let is_invite_file = |a: &mime::AttachmentInfo| {
                invitation && (a.mime.to_ascii_lowercase().starts_with("text/calendar") || a.name.to_ascii_lowercase().ends_with(".ics"))
            };
            let files: Vec<_> = p
                .attachments
                .iter()
                .filter(|a| (!a.inline || a.kind() == AttachmentKind::Image) && !in_place(a) && !is_invite_file(a))
                .cloned()
                .collect();
            if !files.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if files.len() == 1 { tl!("1 attachment").to_string() } else { tf!("{n} attachments", n = files.len()) })
                            .color(t.dim)
                            .small(),
                    );
                    if files.len() > 1 && ui.link(RichText::new(tl!("Save all…")).small()).clicked() {
                        acts.push(Action::SaveAll);
                    }
                });
                // Wrapped into rows, scrolling past three, as Outlook's are.
                widgets::chip_grid(ui, egui::Id::unique(("attachments", r.key.to_id())), files.len(), 240.0, 400.0, CHIP_H, 3, |ui, i, w| {
                    if let Some(a) = files.get(i) {
                        attachment_card(s, st, ui, &r.key, a, w, acts);
                    }
                });
                ui.add_space(10.0);
            }
            if let Some(index) = viewing
                && let Some(a) = p.attachments.iter().find(|a| a.index == index)
            {
                let sep = ui.available_rect_before_wrap();
                ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));
                ui.add_space(12.0);
                attachment_preview(s, st, ui, &r.key, a, acts);
                return;
            }
            if p.hidden_words > 0 {
                hidden_bar(&t, ui, &r.key, p);
            }
            if p.had_html {
                html_bar(s, &t, ui, p, acts);
            }
            if let Some(u) = &p.unsubscribe {
                ui.label(RichText::new(tf!("Mailing list: unsubscribe at {address}", address = widgets::shorten(u, 80))).color(t.faint).small());
                ui.add_space(6.0);
            }
            let sep = ui.available_rect_before_wrap();
            ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));
            ui.add_space(12.0);
            if han_missing(ui, &p.text) {
                st.han_missing = true;
            }
            if s.prefs.show_html && !p.rich.is_empty() {
                rich_body(s, st, ui, &r.key, p, acts);
            } else if crate::bidi::has_rtl(&p.text) {
                // Right-to-left text, against the right edge where it starts; selected with the
                // pointer as it reads, and right-click copies it as written too.
                let r = crate::bidi::selectable_label(ui, &p.text, theme::regular(14.5), t.text);
                rtl_copy_menu(&r, None, &p.text);
            } else {
                plain::plain_body(&t, ui, &p.text, acts);
            }
        });
    });
    // Ctrl+wheel (or a pinch) over the message zooms it, as Outlook's.
    if r.parsed.is_some()
        && viewing.is_none()
        && ui.rect_contains_pointer(scrolled.inner_rect)
        && let Some(percent) = zoom_step(ui.ctx(), s.prefs.reading_zoom)
    {
        acts.push(Action::Run("view.zoom", json!({"percent": percent})));
    }
    // The message scrolled under the header: a line between them.
    if headed && scrolled.state.clamped_offset().y > 0.5 {
        let r = scrolled.inner_rect;
        ui.painter().line_segment([egui::pos2(r.left(), top), egui::pos2(r.right(), top)], egui::Stroke::new(1.0, t.border));
    }
}

/// The zoom Ctrl+wheel (or a pinch) asks for this frame, from `percent`: ten percent a step. egui
/// spreads a wheel's notch over a dozen frames, so a step is taken once it has moved enough and
/// the rest of it let go (until it stops, or for 0.3 s): a notch is one step whatever its size, a
/// pinch a step every 0.3 s.
fn zoom_step(ctx: &egui::Context, percent: u16) -> Option<u16> {
    let (delta, now) = ctx.input(|i| (i.zoom_delta(), i.time));
    let frame = ctx.cumulative_frame_nr();
    let id = egui::Id::unique("reading-zoom");
    // The frame last looked at (egui can draw one twice), the movement added up, and when the
    // last step was taken.
    let (seen, pending, stepped) = ctx.data(|d| d.get_temp::<(u64, f32, Option<f64>)>(id)).unwrap_or((u64::MAX, 0.0, None));
    if seen == frame {
        return None;
    }
    let keep = |pending: f32, stepped: Option<f64>| ctx.data_mut(|d| d.insert_temp(id, (frame, pending, stepped)));
    if !delta.is_finite() || delta <= 0.0 || (delta - 1.0).abs() < 1e-4 {
        // Stopped: the next movement starts afresh.
        keep(0.0, None);
        return None;
    }
    if stepped.is_some_and(|t| now - t < 0.3) {
        keep(0.0, stepped);
        return None;
    }
    let pending = pending + delta.ln();
    if pending.abs() < 1.05f32.ln() {
        keep(pending, stepped);
        return None;
    }
    keep(0.0, Some(now));
    let tens = percent.saturating_add(5) / 10;
    let next = if pending > 0.0 { tens.saturating_add(1) } else { tens.saturating_sub(1) }.saturating_mul(10);
    let (low, high) = (*quatrelle_engine::READING_ZOOM.start(), *quatrelle_engine::READING_ZOOM.end());
    Some(next.clamp(low, high)).filter(|n| *n != percent)
}

/// Who the message says it's from doesn't add up: said plainly, above everything it asks for.
pub(crate) fn sender_warning(t: &Tokens, ui: &mut Ui, w: &quatrelle_engine::model::lookalike::SenderWarning) {
    use quatrelle_engine::model::lookalike::SenderWarning;
    let what = match w {
        SenderWarning::Lookalike { domain, like } => {
            tf!("Careful: this comes from {domain}, which looks like {like} but isn't it.", domain = domain, like = like)
        }
        SenderWarning::NameShowsOther { shown, real } => {
            tf!("Careful: the sender's name shows {shown}, but this comes from {real}.", shown = shown, real = real)
        }
    };
    egui::Frame::new().fill(t.danger.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Warning, t.danger);
            ui.vertical(|ui| {
                ui.label(RichText::new(what).color(t.text));
                ui.label(
                    RichText::new(tl!("Don't open its links or attachments, or answer it, unless you're sure who sent it.")).color(t.dim).small(),
                );
            });
        });
    });
    ui.add_space(8.0);
}

/// A meeting invitation (or an answer to one, or a cancellation) carried by the message.
fn invite_card(s: &Session, st: &mut UiState, ui: &mut Ui, key: &MessageKey, acts: &mut Vec<Action>) {
    let fresh = st.invite.as_ref().is_some_and(|(k, rev, _)| k == key && *rev == s.pim_revision);
    if !fresh {
        st.invite = Some((key.clone(), s.pim_revision, s.invite()));
    }
    let Some(inv) = st.invite.as_ref().and_then(|(_, _, i)| i.clone()) else { return };
    let t = st.tokens;
    let e = &inv.event;
    egui::Frame::new().fill(t.accent_soft).corner_radius(8).inner_margin(Margin::same(12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Calendar, t.accent);
            ui.vertical(|ui| {
                ui.label(RichText::new(&e.summary).font(theme::bold(15.0)).color(t.text));
                let when = if e.all_day {
                    crate::i18n::date(&dates::full_time(s.clock.utc(e.start), &s.clock)).split(' ').take(2).collect::<Vec<_>>().join(" ")
                } else {
                    format!("{} – {}", crate::i18n::date(&dates::full_time(e.start, &s.clock)), crate::calendar::hm(e.end, s))
                };
                ui.label(RichText::new(when).color(t.dim));
                if !e.location.is_empty() {
                    ui.label(RichText::new(&e.location).color(t.dim));
                }
            });
        });
        ui.add_space(6.0);
        // One that can't be taken in says why, and offers nothing to do.
        if let Some(why) = inv.problem {
            let text = match why {
                InviteProblem::NotFromOrganizer => tl!("This update wasn't sent by the meeting's organizer, so it can't change your calendar."),
                InviteProblem::NotFromGuest => tl!("This answer wasn't sent by the guest it's from, so it can't be recorded."),
                InviteProblem::OutOfDate => tl!("This invitation is out of date: your calendar has a newer version."),
            };
            ui.label(RichText::new(text).color(t.danger));
            return;
        }
        match inv.method.as_str() {
            "REQUEST" => {
                ui.horizontal_wrapped(|ui| {
                    let current = match inv.answer.as_str() {
                        "ACCEPTED" => Some("You accepted."),
                        "TENTATIVE" => Some("You said maybe."),
                        "DECLINED" => Some("You declined."),
                        _ => None,
                    };
                    if let Some(c) = current {
                        ui.label(RichText::new(tl!(c)).color(t.text));
                    }
                    for (label, code, icon) in
                        [("Accept", "accepted", Icon::Read), ("Tentative", "tentative", Icon::Clock), ("Decline", "declined", Icon::Close)]
                    {
                        if widgets::pill_button(ui, icon, label, &t).clicked() {
                            acts.push(Action::Run("invite.respond", json!({"answer": code})));
                        }
                    }
                    if inv.in_calendar && ui.link(tl!("Show in calendar")).clicked() {
                        show_day(e.start, s, acts);
                    }
                });
            }
            "CANCEL" => {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(tl!("This event was cancelled.")).color(t.danger));
                    if inv.in_calendar && widgets::pill_button(ui, Icon::Delete, "Remove from calendar", &t).clicked() {
                        acts.push(Action::Run("invite.apply", json!({})));
                    }
                });
            }
            _ => {
                if let Some(who) = &inv.replier {
                    let name = if who.name.is_empty() { who.email.clone() } else { who.name.clone() };
                    let line = match who.partstat.as_str() {
                        "ACCEPTED" => tf!("{name} accepted.", name = name),
                        "DECLINED" => tf!("{name} declined.", name = name),
                        "TENTATIVE" => tf!("{name} said maybe.", name = name),
                        _ => tf!("{name} answered.", name = name),
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(line).color(t.text));
                        if inv.in_calendar && widgets::pill_button(ui, Icon::Calendar, "Record the answer", &t).clicked() {
                            acts.push(Action::Run("invite.apply", json!({})));
                        }
                    });
                }
            }
        }
    });
    ui.add_space(10.0);
}

/// Show the calendar on the day of a time.
fn show_day(t: i64, s: &Session, acts: &mut Vec<Action>) {
    let c = dates::civil(t, &s.clock);
    acts.push(Action::Run("view.module", json!({"name": "calendar"})));
    acts.push(Action::Run("calendar.view", json!({"mode": "day"})));
    acts.push(Action::Run("calendar.goto", json!({"date": format!("{:04}-{:02}-{:02}", c.year, c.month, c.day)})));
}

fn empty(ui: &mut Ui, t: &Tokens, msg: &str) {
    let msg = tl!(msg);
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.35);
            let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Read, t.border);
            ui.add_space(8.0);
            ui.label(RichText::new(msg).color(t.faint));
        });
    });
}

fn thread_list(s: &Session, t: &Tokens, ui: &mut Ui, thread: &[MessageKey], open: &MessageKey, acts: &mut Vec<Action>) {
    egui::Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8).inner_margin(Margin::symmetric(10, 8)).show(
        ui,
        |ui| {
            // Folded to one line (a long conversation's list would push the message down), with
            // the previous and next message at hand; unfolded on a click, and kept so.
            let id = egui::Id::unique("conversation-open");
            let mut unfolded = ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
            let at = thread.iter().position(|k| k == open);
            ui.horizontal(|ui| {
                let label = match at {
                    Some(i) => tf!("Message {i} of {n} in this conversation", i = i + 1, n = thread.len()),
                    None => tf!("{n} messages in this conversation", n = thread.len()),
                };
                let r = ui
                    .add(egui::Button::new(RichText::new(format!("{} {label}", if unfolded { "▾" } else { "▸" })).color(t.dim).small()).frame(false))
                    .on_hover_text(tl!(if unfolded { "Hide the conversation's messages" } else { "Show the conversation's messages" }));
                if r.clicked() {
                    unfolded = !unfolded;
                    ui.ctx().data_mut(|d| d.insert_temp(id, unfolded));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let next = at.and_then(|i| thread.get(i + 1));
                    if ui.add_enabled(next.is_some(), egui::Button::new("›").small()).on_hover_text(tl!("Next in the conversation")).clicked()
                        && let Some(k) = next
                    {
                        acts.push(Action::Open(k.clone()));
                    }
                    let prev = at.and_then(|i| i.checked_sub(1)).and_then(|i| thread.get(i));
                    if ui.add_enabled(prev.is_some(), egui::Button::new("‹").small()).on_hover_text(tl!("Previous in the conversation")).clicked()
                        && let Some(k) = prev
                    {
                        acts.push(Action::Open(k.clone()));
                    }
                });
            });
            if !unfolded {
                return;
            }
            for k in thread {
                let Some(m) = s.summary(k) else { continue };
                let current = k == open;
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                if current || resp.hovered() {
                    ui.painter().rect_filled(rect, 4.0, if current { t.accent_soft } else { t.hover_row });
                }
                let font = if !m.flags.seen { theme::bold(13.0) } else { theme::regular(13.0) };
                let p = ui.painter_at(rect);
                p.text(rect.left_center() + vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, m.from.display(), font, t.text);
                p.text(
                    rect.left_center() + vec2(170.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    widgets::shorten(s.preview_of(m), 70),
                    theme::regular(12.5),
                    t.dim,
                );
                p.text(
                    rect.right_center() - vec2(8.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    crate::i18n::date(&dates::list_time(m.date, &s.clock)),
                    theme::regular(12.0),
                    t.dim,
                );
                if resp.clicked() && !current {
                    acts.push(Action::Open(k.clone()));
                }
            }
        },
    );
}

/// Whether `text` has Chinese characters (or kana, hangul) that no font installed now can draw:
/// in the browser, which brings only some fonts, the Simplified Chinese one is then fetched. The
/// first few thousand letters are looked at, each distinct character once.
pub(crate) fn han_missing(ui: &Ui, text: &str) -> bool {
    let cjk = |c: char| matches!(u32::from(c), 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF);
    let mut seen: Vec<char> = Vec::new();
    for c in text.chars().take(4000).filter(|c| cjk(*c)) {
        if seen.len() >= 64 {
            break;
        }
        if !seen.contains(&c) {
            seen.push(c);
        }
    }
    if seen.is_empty() {
        return false;
    }
    let font = egui::FontId::proportional(14.5);
    ui.fonts_mut(|f| seen.iter().any(|c| !f.has_glyph(&font, *c)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture that isn't square is cut to its middle square before the circle: drawn in a
    /// square box, a wide one was stretched and its "circle" came out an oval.
    #[test]
    fn a_round_picture_keeps_its_middle_square() {
        let wide = image::RgbaImage::from_pixel(8, 4, image::Rgba([200, 100, 50, 255]));
        let (size, pixels) = round_pixels(wide);
        assert_eq!(size, [4, 4], "cut to the middle square, not stretched");
        assert_eq!(pixels.len(), 4 * 4 * 4, "one RGBA for each pixel of the square");
        let alpha = |i: usize| pixels[i * 4 + 3];
        assert_eq!((alpha(0), alpha(3)), (0, 0), "the corners fall outside the circle");
        assert!(alpha(5) > 0 || alpha(6) > 0, "the middle is kept");
        // A square picture is left as it is.
        let square = image::RgbaImage::from_pixel(16, 16, image::Rgba([1, 2, 3, 255]));
        assert_eq!(round_pixels(square).0, [16, 16]);
    }
}
