// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A message's source in a window of its own (View message source, issue #27): read only, its
//! lines selectable, the whole of it copied with a button, its header alone at a click. Only
//! the lines in view are laid out, so a long source scrolls as quickly as a short one. Its bar
//! is the message windows' (the chrome colour, a line under it, buttons with their icons).

use crate::*;

impl MailApp {
    /// Each message source asked for, in a native window (in the browser, a window in the page).
    pub(crate) fn source_windows(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        let (sources, st) = (&self.session.sources, &mut self.ui);
        for v in sources {
            let key = v.key.clone();
            let subject = if v.subject.trim().is_empty() { tl!("(no subject)").to_string() } else { v.subject.trim().to_string() };
            let id = egui::ViewportId::from_hash_of(("source-window", key.to_id()));
            let mut builder =
                egui::ViewportBuilder::default().with_title(tf!("{subject} – Message source", subject = subject)).with_min_inner_size([420.0, 300.0]);
            // Where the last one was, and its size: at its opening only (see `first_frame`).
            if first_frame(ctx, id) {
                builder = match st.places.get("source") {
                    Some(p) => p.apply(builder),
                    None => builder.with_inner_size([860.0, 680.0]),
                };
            }
            let close = ctx.show_viewport_immediate(id, builder, |ui, _| {
                let mut place = st.places.get("source").copied();
                super::windows::after_open(ui, place);
                trace_window(ui, "source");
                super::windows::WindowPlace::note(&mut place, ui);
                if let Some(p) = place {
                    st.places.insert("source".into(), p);
                }
                if st.raise_source.as_ref() == Some(&key) {
                    st.raise_source = None;
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                let mut closing = ui.input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape));
                let header_only = st.source_headers.contains(&key);
                egui::Panel::top(egui::Id::unique(("source-window-bar", key.to_id())))
                    .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(8, 6)).stroke(egui::Stroke::new(1.0, t.border)))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            let parts = [tl!("Whole message"), tl!("Header only")];
                            if let Some(i) = widgets::segmented(ui, &t, "source-part", &parts, usize::from(header_only)) {
                                if i == 1 {
                                    st.source_headers.insert(key.clone());
                                } else {
                                    st.source_headers.remove(&key);
                                }
                            }
                            widgets::group_sep(ui, &t);
                            let ready = !v.pending && v.error.is_none();
                            if widgets::tool_button(ui, Icon::Copy, "Copy all", &t, ready).clicked() && ready {
                                ui.ctx().copy_text(v.text(header_only));
                            }
                            if ready {
                                ui.add_space(6.0);
                                ui.label(RichText::new(widgets::human_size(v.size)).color(t.dim).small());
                            }
                            // Drawn inside the page (the browser), the window has no title bar.
                            if ui.ctx().embed_viewports() {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if widgets::tool_button(ui, Icon::Close, "Close", &t, true).clicked() {
                                        closing = true;
                                    }
                                });
                            }
                        });
                        if v.truncated && !header_only {
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(tf!(
                                    "Only the first {shown} of {size} are shown. Save as email (.eml) keeps all of it.",
                                    shown = widgets::human_size(quatrelle_engine::SOURCE_SHOWN.min(v.size)),
                                    size = widgets::human_size(v.size)
                                ))
                                .color(t.dim)
                                .small(),
                            );
                        }
                    });
                egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface).inner_margin(Margin::same(8))).show(ui, |ui| {
                    source_text(ui, &t, v, header_only);
                });
                closing
            });
            if close {
                super::windows::forget_open(ctx, id);
                st.source_headers.remove(&key);
                acts.push(Action::Run("message.closeSource", json!({"key": key.to_id()})));
            }
        }
    }
}

/// The source's lines (or its header's), only those in view laid out.
fn source_text(ui: &mut Ui, t: &Tokens, v: &quatrelle_engine::SourceView, header_only: bool) {
    if let Some(e) = &v.error {
        ui.label(RichText::new(tf!("The source couldn't be downloaded: {error}", error = e)).color(t.danger));
        return;
    }
    if v.pending {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(14.0));
            ui.label(RichText::new(tl!("Downloading the message…")).color(t.dim));
        });
        return;
    }
    let n = if header_only { v.header_lines.min(v.lines.len()) } else { v.lines.len() };
    let font = egui::FontId::monospace(12.5);
    let row = ui.fonts_mut(|f| f.row_height(&font));
    egui::ScrollArea::both().auto_shrink(false).id_salt(("source", v.key.to_id())).show_rows(ui, row, n, |ui, rows| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for line in v.lines.get(rows).unwrap_or_default() {
            ui.add(egui::Label::new(RichText::new(line).font(font.clone()).color(t.text)).wrap_mode(egui::TextWrapMode::Extend).selectable(true));
        }
    });
}
