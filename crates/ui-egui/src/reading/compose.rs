//! The message being written: its form (from, to, cc, bcc, subject), its ribbon and its body.

use quatrelle_engine::FieldId;
use quatrelle_engine::model::addr::Accepts;

use super::*;
use crate::dialogs::people_field::COMPOSE_FIELDS;

/// Height of an attachment chip.
pub(super) const CHIP_H: f32 = 42.0;

/// When to send a held-back message: in an hour, tomorrow or Monday morning, a time of one's
/// own, or at once again.
pub(crate) fn delay_menu(ui: &mut Ui, t: &Tokens, c: &mut quatrelle_engine::Compose, clock: &dates::Clock) {
    let today = dates::today(clock);
    let at = |day: i64, minutes: i64| clock.utc(day.saturating_mul(86_400).saturating_add(minutes * 60));
    let weekday = (today + 3).rem_euclid(7);
    let presets = [
        ("In 1 hour", (clock.now + 3600) / 60 * 60),
        ("Tomorrow morning", at(today + 1, 8 * 60)),
        ("Monday morning", at(today + 7 - weekday, 8 * 60)),
    ];
    for (label, when) in presets {
        let clicked = ui
            .horizontal(|ui| {
                let r = ui.selectable_label(c.send_after == Some(when), tl!(label));
                ui.label(RichText::new(crate::i18n::date(&dates::full_time(when, clock))).color(t.faint).small());
                r.clicked()
            })
            .inner;
        if clicked {
            c.send_after = Some(when);
            ui.close();
        }
    }
    ui.separator();
    // A day and time of one's own (local), kept while the menu is open.
    let id = egui::Id::unique(("delay-custom", c.id));
    let (mut day, mut time) = ui.data(|d| d.get_temp::<(String, String)>(id)).unwrap_or_else(|| {
        let c0 = dates::civil(c.send_after.unwrap_or(clock.now + 3600), clock);
        (format!("{:04}-{:02}-{:02}", c0.year, c0.month, c0.day), format!("{:02}:{:02}", c0.hour, c0.minute))
    });
    ui.label(RichText::new(tl!("Don't send before")).color(t.dim).small());
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut day).desired_width(90.0).hint_text("YYYY-MM-DD"));
        ui.add(egui::TextEdit::singleline(&mut time).desired_width(50.0).hint_text("HH:MM"));
        let parsed = parse_local(&day, &time).map(|(d, m)| at(d, m));
        if ui.add_enabled(parsed.is_some_and(|w| w > clock.now), egui::Button::new(tl!("Set"))).clicked() {
            chosen = parsed;
        }
    });
    ui.data_mut(|d| d.insert_temp(id, (day, time)));
    if let Some(w) = chosen {
        c.send_after = Some(w);
        ui.close();
    }
    if c.send_after.is_some() {
        ui.separator();
        if ui.button(tl!("Send right away")).clicked() {
            c.send_after = None;
            ui.close();
        }
    }
}

/// A local `YYYY-MM-DD` and `HH:MM` as (day, minutes into it).
pub(crate) fn parse_local(day: &str, time: &str) -> Option<(i64, i64)> {
    if !quatrelle_engine::model::job::is_day(day.trim()) {
        return None;
    }
    let d = day.trim();
    let num = |a: usize, z: usize| d.get(a..z).and_then(|x| x.parse::<u32>().ok());
    let days = dates::days_from_civil(i64::from(num(0, 4)?), num(5, 7)?, num(8, 10)?);
    let (h, m) = time.trim().split_once(':')?;
    let (h, m) = (h.trim().parse::<i64>().ok()?, m.trim().parse::<i64>().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some((days, h * 60 + m))
}

/// Width of the labels and label buttons left of the message's fields.
pub(super) const LABEL_W: f32 = crate::widgets::FIELD_LABEL_W;

pub(super) const ROW_H: f32 = crate::widgets::FIELD_ROW_H;

/// A field box drawn the way Outlook's are: no frame, a line under it (the accent while typing).
pub(super) fn underlined(ui: &mut Ui, t: &Tokens, edit: egui::TextEdit<'_>) -> egui::Response {
    let r = ui.add_sized(vec2(ui.available_width(), ROW_H - 4.0), edit.frame(egui::Frame::NONE).align(egui::Align2::LEFT_CENTER));
    let y = r.rect.bottom();
    let color = if r.has_focus() { t.accent } else { t.border };
    ui.painter().line_segment([egui::pos2(r.rect.left(), y), egui::pos2(r.rect.right(), y)], egui::Stroke::new(1.0, color));
    r
}

/// A label button that opens a menu: its label and a small drawn chevron, centred together.
pub(super) fn label_button_menu(ui: &mut Ui, t: &Tokens, label: &str, enabled: bool, menu: bool) -> egui::Response {
    crate::widgets::field_button(ui, t, label, enabled, menu)
}

/// Outlook's tall Send button beside the address rows.
pub(super) fn send_button(ui: &mut Ui, t: &Tokens, height: f32, sending: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(66.0, height), if sending { Sense::hover() } else { Sense::click() });
    crate::widgets::named(ui, &resp, if sending { tl!("Sending…") } else { tl!("Send") }, !sending, None);
    let fill = if resp.hovered() && !sending { t.hover } else { t.surface };
    ui.painter().rect(rect, 4.0, fill, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let icon = egui::Rect::from_center_size(rect.center() - vec2(0.0, 10.0), vec2(24.0, 24.0));
    icons::paint(ui.painter(), icon, Icon::Send, if sending { t.dim } else { t.accent });
    let label = if sending { tl!("Sending…") } else { tl!("Send") };
    ui.painter().text(rect.center() + vec2(0.0, 16.0), egui::Align2::CENTER_CENTER, label, theme::regular(12.5), t.text);
    resp.on_hover_text(tl!("Send (Ctrl+Enter)"))
}

/// A labelled field; its spelling is checked when `spell` says how.
pub(super) fn field(ui: &mut Ui, t: &Tokens, label: &str, value: &mut String, hint: &str, focus: bool, spell: Option<&crate::spellcheck::Check>) {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(LABEL_W, ROW_H), egui::Label::new(RichText::new(tl!(label)).color(t.dim)));
        let id = ui.make_persistent_id(("field", label));
        let r = match spell {
            Some(check) => underlined_spelling(ui, t, id, value, hint, check),
            None => underlined(ui, t, egui::TextEdit::singleline(value).id(id).hint_text(tl!(hint))),
        };
        if focus {
            r.request_focus();
        }
    });
}

/// To, Cc, Bcc or Replies to: the contacts field over the message's own text (`name` is the
/// field's, as Check Names knows it). Whether its book button was pressed.
#[allow(clippy::too_many_arguments)]
fn people_row(
    ui: &mut Ui,
    st: &mut UiState,
    c: &mut quatrelle_engine::Compose,
    name: &'static str,
    label: &str,
    hint: &str,
    focus: bool,
    can_expand: bool,
    acts: &mut Vec<Action>,
) -> bool {
    let t = st.tokens;
    let field = FieldId { window: c.id, name };
    let value = match name {
        "to" => &mut c.to,
        "cc" => &mut c.cc,
        "bcc" => &mut c.bcc,
        _ => &mut c.reply_to,
    };
    // Select Names has rows for To, Cc and Bcc: Replies to keeps its label alone.
    let book = name != "replyTo";
    st.people.draw(ui, &t, field, Some(&c.account), &Accepts::RECIPIENTS, label, hint, value, focus, can_expand, book, acts)
}

/// The message being written, in a window of its own.
pub fn compose_window(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    compose(s, st, ui, acts, true);
}

pub(super) fn compose(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>, in_window: bool) {
    let t = st.tokens;
    attach_dropped(ui, &t, acts);
    // Where there are no windows of their own (the browser), the other messages being written
    // are a click away above this one.
    if !in_window && !s.composes.is_empty() && ui.ctx().embed_viewports() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tl!("Also being written:")).color(t.dim).small());
            for c in &s.composes {
                let subject = if c.subject.trim().is_empty() { tl!("Untitled").to_string() } else { c.subject.trim().to_string() };
                if ui.small_button(subject).clicked() {
                    acts.push(Action::Run("compose.activate", json!({"id": c.id})));
                }
            }
        });
    }
    let senders = s.senders();
    // Who the other address typed in From may be; the message's fields made ready (chips from
    // their text, the names the directory settled, suggestions for what's typed), and the
    // choice a name several people fit waits for.
    let others = match s.compose.as_ref().map(|c| (c.id, c.account.clone())) {
        Some((id, account)) => {
            st.people.prepare_compose(s, id, &account);
            st.people.choice_dialog(ui.ctx(), &t, s, id, &COMPOSE_FIELDS, acts);
            candidates(s, ui.ctx(), id, &account)
        }
        None => Candidates::default(),
    };
    // A list's card offers Expand only where there's a directory to ask (an Exchange account).
    let can_expand = s.compose.as_ref().is_some_and(|c| s.directory_account(Some(&c.account)).is_some());
    let clock = s.clock;
    // Delay Delivery holds mail in the Outbox, which only a server account's sends from.
    let can_delay = s.compose.as_ref().and_then(|c| s.account(&c.account)).is_some_and(|a| !a.is_local());
    let can_vote = s.compose.as_ref().and_then(|c| s.account(&c.account)).is_some_and(|a| a.config.ews.is_some());
    // S/MIME's Encrypt and Sign, for an account with a certificate.
    let has_cert = s.compose.as_ref().is_some_and(|c| s.identity(&c.account).is_some());
    let assistant = !s.prefs.llm_url.trim().is_empty();
    let recent_emoji = s.prefs.recent_emoji.clone();
    let signature = s.compose.as_ref().and_then(|c| s.account(&c.account)).map(|a| a.config.signature.clone()).unwrap_or_default();
    let Some(c) = s.compose.as_mut() else { return };
    // The editor's state belongs to one message.
    if st.editor.as_ref().is_none_or(|e| e.id != c.id) {
        st.editor = Some(rich_edit::Editor::new(c.id));
    }
    let Some(mut ed) = st.editor.take() else { return };
    // Why it couldn't go: over the message, in its own window when it has one.
    if let Some(msg) = st.compose_alert.clone() {
        let modal = egui::Modal::new(egui::Id::unique(("compose-alert", c.id))).frame(crate::dialogs::dialog_frame(&t)).show(ui.ctx(), |ui| {
            ui.set_width(380.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::Warning, t.warn);
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(tl!("Quatrelle")).font(theme::bold(16.0)).color(t.text));
                    let mut text = if msg == "add at least one recipient" {
                        tl!("Add at least one name in To, Cc or Bcc before sending.").to_string()
                    } else if msg == "Looking for these recipients' certificates in the Exchange address book: send it again in a moment." {
                        tl!("Looking for these recipients' certificates in the Exchange address book: send it again in a moment.").to_string()
                    } else {
                        msg.clone()
                    };
                    if let Some(first) = text.get(..1) {
                        text = first.to_uppercase() + text.get(1..).unwrap_or_default();
                    }
                    if !text.ends_with('.') {
                        text.push('.');
                    }
                    ui.add(egui::Label::new(RichText::new(text).color(t.text)).wrap());
                });
            });
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.button(tl!("OK")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter))
            })
            .inner
        });
        if modal.inner || modal.should_close() {
            st.compose_alert = None;
        }
    }
    // A newly opened message: start in To (new mail) or the body (replies).
    let fresh = st.compose_shown.insert(c.id);
    let focus_to = fresh && c.to.trim().is_empty();
    let focus_body = fresh && !focus_to;
    let send = |c: &quatrelle_engine::Compose, acts: &mut Vec<Action>| {
        if c.subject.trim().is_empty() {
            acts.push(Action::Dialog(crate::Dialog::EmptySubject));
        } else {
            acts.push(Action::Run("compose.send", json!({})));
        }
    };
    // Alt+S sends, as in Outlook; in a window of its own the main window's shortcuts don't reach
    // it, so Ctrl+Enter and Ctrl+S are the message's own there.
    if !c.sending {
        let (alt_s, send_key, save_key, spelling) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::ALT, egui::Key::S),
                in_window && i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter),
                in_window && i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
                i.consume_key(egui::Modifiers::NONE, egui::Key::F7),
            )
        });
        if alt_s || send_key {
            send(c, acts);
        }
        if save_key {
            acts.push(Action::Run("compose.saveDraft", json!({})));
        }
        if spelling {
            acts.push(Action::Dialog(crate::Dialog::spelling()));
        }
    }
    // The typefaces the text is set in (Arial, Calibri…), from the system when it has them, so
    // the text shows as it will be read.
    let typefaces = c.styles.palette.iter().filter_map(|s| s.font.clone());
    for name in crate::look::typefaces::wanted(ui.ctx(), typefaces) {
        acts.push(Action::LoadTypeface(name));
    }
    // The subject's spelling is checked as the message's is (its language, the words let be).
    let subject_check = crate::spellcheck::Check { skip: Vec::new(), ..rich_edit::check_for(c) };
    egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 8, bottom: 12 }).show(ui, |ui| {
        // The ribbon, as Outlook's message window has it: its tabs, with Save (and in a window
        // Pop in and Discard) small at their right; where windows can't open (the browser),
        // messages stay in the reading pane.
        let input = rich_edit::RibbonInput {
            signature: &signature,
            clock: &clock,
            can_delay,
            can_vote,
            has_cert,
            in_window,
            can_pop: !ui.ctx().embed_viewports(),
            assistant,
            recent_emoji: &recent_emoji,
        };
        // In the reading pane, Pop out and Discard where they're seen, as Outlook's reply there
        // has them (a window has them small beside its tabs).
        if !in_window {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if input.can_pop
                    && widgets::pill_button(ui, Icon::PopOut, "Pop out", &t).on_hover_text(tl!("Write it in a window of its own")).clicked()
                {
                    acts.push(Action::Run("compose.popOut", json!({})));
                }
                if widgets::pill_button(ui, Icon::Delete, "Discard", &t).clicked() {
                    acts.push(Action::Run("compose.discard", json!({})));
                }
            });
            ui.add_space(6.0);
        }
        egui::Frame::new().fill(t.chrome).corner_radius(6).inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            rich_edit::compose_ribbon(ui, &t, &mut ed, c, &input, acts);
        });
        super::assistant::rewrite_panel(&t, ui, c, acts);
        if let Some(e) = &c.error {
            ui.add_space(4.0);
            egui::Frame::new().fill(t.danger.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                ui.label(RichText::new(tf!("Not sent: {error}", error = e)).color(t.danger));
            });
        }
        // Ctrl+K: Outlook's Check Names on every field, anywhere but in the text (where it makes
        // a link), the other sender's box, and a contacts field (which checks itself).
        if !c.sending
            && st.people.open_field().is_none()
            && !ui.memory(|m| m.has_focus(rich_edit::body_id(c)) || m.has_focus(box_id(c.id)))
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K))
        {
            acts.push(Action::Run("compose.checkNames", json!({})));
        }
        ui.add_space(6.0);
        ui.add_enabled_ui(!c.sending, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            // Send beside the address rows (From, To, Cc and Bcc when shown).
            let rows = if c.show_cc { 4.0 } else { 3.0 };
            let height = rows * ROW_H + (rows - 1.0) * 6.0;
            ui.horizontal_top(|ui| {
                if send_button(ui, &t, height, c.sending).clicked() {
                    send(c, acts);
                }
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| from_row(ui, &t, c, &senders, &others));
                    // To, Cc and Bcc are the contacts field; their buttons open the address book,
                    // as in Outlook.
                    let mut book = None;
                    if people_row(ui, st, c, "to", "To", "Names <addresses>, separated by ; or ,", focus_to, can_expand, acts) {
                        book = Some("to");
                    }
                    if people_row(ui, st, c, "cc", "Cc", "", false, can_expand, acts) {
                        book = Some("cc");
                    }
                    if c.show_cc && people_row(ui, st, c, "bcc", "Bcc", "Hidden from the other recipients", false, can_expand, acts) {
                        book = Some("bcc");
                    }
                    if let Some(name) = book {
                        acts.push(Action::Dialog(crate::Dialog::select_names(c, name)));
                    }
                });
            });
            if c.show_reply_to || !c.reply_to.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(76.0);
                    ui.vertical(|ui| {
                        people_row(ui, st, c, "replyTo", "Replies to", "Where replies go instead of to you", false, can_expand, acts);
                    });
                });
            }
            ui.horizontal(|ui| {
                ui.add_space(76.0);
                ui.vertical(|ui| field(ui, &t, "Subject", &mut c.subject, "Add a subject", false, Some(&subject_check)));
            });
            if !c.attachments.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(76.0 + LABEL_W);
                    let mut remove = None;
                    ui.vertical(|ui| {
                        // Wrapped into rows, scrolling past two, as Outlook's are.
                        let n = c.attachments.len();
                        widgets::chip_grid(ui, egui::Id::unique(("compose-attachments", c.id)), n, 220.0, 320.0, 28.0, 2, |ui, i, w| {
                            let Some(a) = c.attachments.get(i) else { return };
                            let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
                            let x_rect = Rect::from_center_size(egui::pos2(rect.right() - 15.0, rect.center().y), vec2(22.0, 22.0));
                            let x = ui.interact(x_rect, resp.id.with("remove"), Sense::click()).on_hover_text(tl!("Remove"));
                            widgets::named(ui, &x, &format!("{}: {}", tl!("Remove"), a.name), true, None);
                            let p = ui.painter();
                            p.rect(
                                rect,
                                4.0,
                                if resp.hovered() { t.hover_row } else { t.surface },
                                egui::Stroke::new(1.0, t.faint.gamma_multiply(0.55)),
                                egui::StrokeKind::Inside,
                            );
                            if x.hovered() {
                                p.rect_filled(x_rect, 3.0, t.hover);
                            }
                            icons::paint(p, icons::centered(x_rect, 11.0), Icon::Close, if x.hovered() { t.text } else { t.dim });
                            let ir = Rect::from_center_size(egui::pos2(rect.left() + 15.0, rect.center().y), vec2(14.0, 14.0));
                            icons::paint(p, ir, Icon::Attach, t.dim);
                            let text = format!("{} ({})", a.name, widgets::human_size(a.data.len()));
                            let g = widgets::one_line(ui, &text, theme::regular(12.0), t.text, x_rect.left() - 4.0 - (ir.right() + 6.0));
                            p.galley(egui::pos2(ir.right() + 6.0, rect.center().y - g.size().y / 2.0), g, t.text);
                            resp.on_hover_text(a.name.clone());
                            if x.clicked() {
                                remove = Some(i);
                            }
                        });
                        // A photo that says where it was taken (looked at once per set of files).
                        let size: usize = c.attachments.iter().map(|a| a.data.len()).sum();
                        let id = egui::Id::unique(("photo-place", c.id, n, size));
                        let placed = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or_else(|| {
                            let v = c.attachments.iter().any(|a| mime::without_location(&a.data).is_some());
                            ui.data_mut(|d| d.insert_temp(id, v));
                            v
                        });
                        if placed {
                            let note = if s.prefs.keep_photo_location {
                                tl!("These photos say where they were taken, and go that way (File › Options › Safety)")
                            } else {
                                tl!("Where these photos were taken is left out as they're sent")
                            };
                            ui.label(RichText::new(note).color(t.faint).small());
                        }
                    });
                    if let Some(i) = remove {
                        acts.push(Action::Run("compose.removeAttachment", json!({"index": i})));
                    }
                });
            }
            ui.add_space(4.0);
            let sep = ui.available_rect_before_wrap();
            ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));
            ui.add_space(8.0);
            let body_id = rich_edit::body_id(c);
            // A picture pasted goes in as a picture: a screenshot (the window reads it off the
            // clipboard when there's no text there, and Ctrl+V comes as the picture), or one
            // copied from a web page, which comes with its HTML alone (a picture, no text).
            if ui.memory(|m| m.has_focus(body_id)) {
                let shots: Vec<Vec<u8>> = ui.input(|i| {
                    i.events
                        .iter()
                        .filter_map(|e| match e {
                            egui::Event::PasteImage(img) => crate::reading::png_of(img),
                            _ => None,
                        })
                        .collect()
                });
                acts.extend(shots.into_iter().map(Action::PasteImage));
                if ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))))
                    && crate::clipboard_html().is_some_and(|h| picture_alone(&h))
                {
                    ui.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Paste(_))));
                    acts.push(Action::PastePicture);
                }
            }
            egui::ScrollArea::vertical().auto_shrink(false).id_salt("compose-body").show(ui, |ui| {
                // Replies start above the quoted text.
                rich_edit::body(ui, &t, &mut ed, c, focus_body.then_some(0), acts);
            });
        });
    });
    // A signature written in HTML, drawn where it stands in the text (with the whole session at
    // hand again).
    if let Some(c) = s.compose.as_ref() {
        rich_edit::draw_signature(s, st, ui, &mut ed, c);
    }
    st.editor = Some(ed);
}

/// Whether HTML copied holds a picture and no text (a picture copied from a web page): the
/// picture is what's meant, not its address.
pub(crate) fn picture_alone(html: &str) -> bool {
    html.to_ascii_lowercase().contains("<img")
        && quatrelle_engine::mime::pasted_html(html).0.replace(quatrelle_engine::mime::rich::PICTURE, "").trim().is_empty()
}

/// Files dragged from the desktop onto the message are attached (read from where they are, as
/// Attach File reads them); while they're held over it, its edge says it takes them. In the
/// browser a dropped file has no path to read from, and nothing is attached.
fn attach_dropped(ui: &mut Ui, t: &Tokens, acts: &mut Vec<Action>) {
    let (hovering, dropped) = ui.ctx().input(|i| (!i.raw.hovered_files.is_empty(), i.raw.dropped_files.clone()));
    if hovering {
        ui.painter().rect_stroke(ui.max_rect().shrink(2.0), 6.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    }
    for f in dropped.iter().take(100).filter(|f| f.path().is_absolute()) {
        acts.push(Action::Run("compose.attach", json!({"path": f.path().to_string_lossy()})));
    }
}
