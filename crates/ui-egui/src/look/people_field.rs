//! The contacts field (To, Cc, Bcc, and later a meeting's attendees, a rule's senders…), drawn the
//! way Outlook draws it: the book button naming the field, each recognised entry a chip with its
//! kind's icon and any text not yet recognised beside them, the input after them, the suggestions
//! in a list under the input, and a clicked chip's card. The field wraps and grows to
//! [`MAX_LINES`] lines, then scrolls inside itself.
//!
//! Pure UI: it's handed its state and the suggestions already worked out, and says what happened
//! as [`PeopleEvent`]s. It never sees the `Session` (`dialogs/people_field.rs` is its glue). It
//! changes the input and its own view of the field (the list, the selection, the card); the
//! pieces are the glue's to change, from the events.

use egui::{EventFilter, FocusDirection, Id, Key, KeyboardShortcut, Modifiers, Ui, vec2};
use quatrelle_engine::model::addr::{Accepts, Addressee, AddresseeKind};
use quatrelle_engine::{Source, Suggestion};

use crate::icons::Icon;
use crate::theme::Tokens;

mod card;
mod chips;
mod list;

/// The lines the field grows to before it scrolls.
pub const MAX_LINES: usize = 4;
/// The height of a line of chips, the gap under it included.
pub const CHIP_ROW_H: f32 = 30.0;
/// A chip's slot in its line (its pill is drawn inside), and the input's height.
const CHIP_H: f32 = CHIP_ROW_H - 4.0;

/// Why a piece of text didn't become a chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PieceError {
    /// It has an `@` but isn't an address.
    NotAnAddress,
    /// Nothing matched the name.
    NotFound,
    /// The directory couldn't be asked (offline, unreachable).
    Unreached,
    /// A room among a meeting's guests that its Rooms can't take, the meeting booking the most
    /// rooms it can (this many): kept among them as typed rather than lost.
    RoomsFull(usize),
    /// What was asked of it couldn't be done (a calendar that didn't open), and why (as said,
    /// already translated or the server's own words). It isn't asked again.
    Failed(String),
}

/// One entry of the field: a chip, or text not (yet) recognised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    /// Someone or something recognised; `refused` is why the place doesn't take what it is
    /// (translated), drawn as a red chip that blocks OK or Send until it's removed.
    Chip {
        who: Addressee,
        refused: Option<String>,
    },
    Text {
        text: String,
        error: Option<PieceError>,
    },
}

impl Piece {
    /// A chip the place takes.
    pub fn chip(who: Addressee) -> Piece {
        Piece::Chip { who, refused: None }
    }

    /// Whether the entry is red: a chip its place refuses, or text with an error. It stops the
    /// field's OK or Send until it's removed.
    pub fn is_red(&self) -> bool {
        matches!(self, Piece::Chip { refused: Some(_), .. } | Piece::Text { error: Some(_), .. })
    }
}

/// A field's state between frames.
#[derive(Default)]
pub struct FieldState {
    pub pieces: Vec<Piece>,
    /// What is being typed after the pieces.
    pub input: String,
    /// The piece selected (Backspace in an empty input, a click): Backspace or Delete removes it.
    pub selected: Option<usize>,
    /// Whether the suggestions are wanted (something typed, not dismissed since).
    pub open: bool,
    /// The suggestion highlighted, among those shown.
    pub highlighted: usize,
    /// The piece whose card is open.
    pub card: Option<usize>,
    /// The input the list was closed for with Esc: it stays closed until the input changes.
    pub dismissed_for: Option<String>,
    /// How many entries a paste or a text fed in left out (past the field's cap).
    pub left_out: Option<usize>,
    /// The text last written back (the glue's).
    pub synced: String,
    /// How many times the text was parsed (the glue's).
    pub syncs: u32,
}

/// What a field is drawn with this frame.
pub struct FieldInput<'a> {
    /// The field's id: its input, list, card and scroll are kept under it.
    pub id: egui::Id,
    /// The book button's label and the input's hint (English, translated here). An empty label:
    /// the field sits in a form's row whose own label names it (a meeting's Required), drawn
    /// under [`label_id`]; the field draws no button of its own.
    pub label: &'a str,
    pub hint: &'a str,
    /// The button opens the address book (a field with no book to open keeps the label alone).
    pub book: bool,
    pub accepts: &'a Accepts,
    /// The suggestions for the input, best first (only the kinds `accepts` takes are shown).
    pub suggestions: &'a [Suggestion],
    /// Whether a list can be expanded (an account with a directory): the card offers Expand.
    pub can_expand: bool,
    /// Give the input the keyboard focus.
    pub focus: bool,
    /// Names still being looked up: a spinner shows while there are.
    pub pending: usize,
}

/// What happened in the field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeopleEvent {
    /// The input's text changed by typing (the glue asks for its suggestions).
    Changed,
    /// Text closed by `;` or `,`, a paste of a list or an address, or the input losing the focus
    /// with text in it: the glue makes it into pieces. The input is left with what follows.
    Closed(String),
    /// A suggestion taken (by its index in `suggestions`): Enter, Tab while the list is open, or a
    /// click. The input is emptied for it.
    Take(usize),
    /// A Recent suggestion to forget, by its address (Delete on it, or its ×).
    Forget(String),
    /// Remove the piece (Backspace twice, Delete, or the card's Remove).
    RemoveChip(usize),
    /// Expand the list or contact group (its card's Expand).
    Expand(usize),
    /// Ctrl+K with the focus in the field: check the names typed.
    Resolve,
    /// The book button: open the address book for the field.
    OpenBook,
}

/// The id of a field's input (`TextEdit`), from the field's id.
pub fn input_id(field: Id) -> Id {
    field.with("people-input")
}

/// The id of a field's suggestion list (an area of its own), from the field's id.
pub fn list_id(field: Id) -> Id {
    field.with("people-list")
}

/// The id of a field's chip card (an area of its own), from the field's id.
pub fn card_id(field: Id) -> Id {
    field.with("people-card")
}

/// Whether `pos` is on the field's suggestion list or chip card, as drawn last frame: a menu
/// holding the field doesn't take a click there for one outside it.
pub fn over_popups(ctx: &egui::Context, field: Id, pos: egui::Pos2) -> bool {
    ctx.memory(|m| {
        [list_id(field), card_id(field)].into_iter().any(|id| {
            m.areas().visible_last_frame(&egui::LayerId::new(egui::Order::Foreground, id)) && m.area_rect(id).is_some_and(|r| r.contains(pos))
        })
    })
}

/// The id of the label a form draws for a field with none of its own (an empty
/// [`FieldInput::label`]): it names the input to a screen reader.
pub fn label_id(field: Id) -> Id {
    field.with("people-label")
}

/// The icon for a kind of entry.
pub fn kind_icon(kind: AddresseeKind) -> Icon {
    match kind {
        AddresseeKind::Person => Icon::Person,
        AddresseeKind::DistributionList => Icon::DistributionList,
        AddresseeKind::ContactGroup => Icon::ContactGroup,
        AddresseeKind::Room => Icon::Room,
        AddresseeKind::Equipment => Icon::Equipment,
        AddresseeKind::SharedMailbox => Icon::SharedMailbox,
        AddresseeKind::PublicFolder => Icon::PublicFolder,
        AddresseeKind::Microsoft365Group => Icon::Group365,
        AddresseeKind::Unknown => Icon::Address,
    }
}

/// A kind's name, translated (an unknown kind is an address alone).
pub fn kind_name(kind: AddresseeKind) -> &'static str {
    match kind {
        AddresseeKind::Person => tl!("Person"),
        AddresseeKind::DistributionList => tl!("Distribution list"),
        AddresseeKind::ContactGroup => tl!("Contact group"),
        AddresseeKind::Room => tl!("Room"),
        AddresseeKind::Equipment => tl!("Equipment"),
        AddresseeKind::SharedMailbox => tl!("Shared mailbox"),
        AddresseeKind::PublicFolder => tl!("Public folder"),
        AddresseeKind::Microsoft365Group => tl!("Microsoft 365 group"),
        // An e-mail address, where the plain "Address" may be a web address (Chinese).
        AddresseeKind::Unknown => crate::i18n::tc("kind", "Address"),
    }
}

/// Where a suggestion comes from, translated.
pub fn source_name(source: Source) -> &'static str {
    match source {
        Source::Recent => tl!("Recent"),
        Source::Contacts => tl!("Contacts"),
        Source::Directory => tl!("Directory (cached)"),
        Source::RoomList => tl!("Room list"),
    }
}

/// What a piece of text that didn't become a chip says, translated.
pub fn error_text(text: &str, error: &PieceError) -> String {
    match error {
        PieceError::NotAnAddress => tf!("Not an address: {what}", what = text),
        PieceError::NotFound => tf!("Not found: {name}", name = text),
        PieceError::Unreached => tl!("The directory can't be reached").to_string(),
        PieceError::Failed(why) => why.clone(),
        PieceError::RoomsFull(max) => tf!("{room} can't be booked: a meeting books {max} rooms at most", room = text, max = max),
    }
}

/// The field's text: its entries (a chip as [`Addressee::text`] writes it, text as typed) joined
/// by `"; "`, then `"; "` and the input when there are entries, as Check Names writes it.
pub fn text_of(pieces: &[Piece], input: &str) -> String {
    let mut out = String::new();
    for p in pieces {
        match p {
            Piece::Chip { who, .. } => out.push_str(&who.text()),
            Piece::Text { text, .. } => out.push_str(text),
        }
        out.push_str("; ");
    }
    out.push_str(input);
    out
}

/// A piece as Ctrl+C copies it: the text it is written as, a contact group by its name (its
/// reference means nothing outside Quatrelle).
fn copy_text(p: &Piece) -> String {
    match p {
        Piece::Chip { who, .. } if who.kind == AddresseeKind::ContactGroup && who.address.is_empty() => who.display().to_string(),
        Piece::Chip { who, .. } => who.text(),
        Piece::Text { text, .. } => text.clone(),
    }
}

/// Where the last `;` or `,` outside quotes and `<…>` is (a byte offset).
fn last_separator(s: &str) -> Option<usize> {
    let (mut quoted, mut angle, mut last) = (false, false, None);
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '<' if !quoted => angle = true,
            '>' if !quoted => angle = false,
            ';' | ',' if !quoted && !angle => last = Some(i),
            _ => {}
        }
    }
    last
}

/// Closed text as it's handed on: trimmed, without separators at its end.
fn closed(s: &str) -> String {
    s.trim_end_matches(|c: char| c == ';' || c == ',' || c.is_whitespace()).trim_start().to_string()
}

/// Whether a paste is closed at once: a list, or an address (a name alone is typed in, for its
/// suggestions).
fn closes(paste: &str) -> bool {
    paste.contains([';', ',', '\n', '\r', '@'])
}

/// Draw the field; what happened in it.
pub fn people_field(ui: &mut Ui, t: &Tokens, st: &mut FieldState, input: &FieldInput<'_>) -> Vec<PeopleEvent> {
    let mut ev = Vec::new();
    let edit = input_id(input.id);
    if st.selected.is_some_and(|i| i >= st.pieces.len()) {
        st.selected = None;
    }
    if st.card.is_some_and(|i| i >= st.pieces.len()) {
        st.card = None;
    }
    if input.focus && !ui.memory(|m| m.has_focus(edit)) {
        ui.memory_mut(|m| m.request_focus(edit));
    }
    let focused = ui.memory(|m| m.has_focus(edit));
    // The list as it stands before this frame's keys and clicks: they act on what was shown.
    let rows = list::rows(st, input, focused);
    st.highlighted = st.highlighted.min(rows.len().saturating_sub(1));
    let mut refocus = false;
    if focused {
        refocus |= keys(ui, st, input, &rows, &mut ev);
    }
    let typed_from = st.input.clone();
    let mut laid = None;
    ui.horizontal_top(|ui| {
        let named_by = if input.label.is_empty() {
            label_id(input.id)
        } else {
            let book = crate::widgets::field_button(ui, t, input.label, input.book, false);
            if input.book && book.clicked() {
                ev.push(PeopleEvent::OpenBook);
            }
            book.id
        };
        laid = Some(ui.vertical(|ui| chips::body(ui, t, st, input, edit, named_by)).inner);
    });
    let Some(laid) = laid else { return ev };
    refocus |= laid.refocus;
    typed(st, &typed_from, &mut ev);

    // The list under the input (inside the lines shown, when the input is scrolled out of them).
    let mut on_popup = false;
    if !rows.is_empty() && !refocus {
        let at = egui::pos2(laid.edit.rect.left().max(laid.view.left()), laid.edit.rect.bottom().min(laid.view.bottom()) + 2.0);
        let (picked, hit) = list::show(ui, t, st, input, &rows, at, laid.edit.rect.width(), &mut ev);
        on_popup |= hit;
        if picked {
            st.input.clear();
            st.open = false;
            st.highlighted = 0;
            refocus = true;
        }
    }
    on_popup |= card::show(ui, t, st, input, &laid.rects, &mut ev);
    // egui takes the focus from the input when a click ends elsewhere, so a press on the list or
    // the card holds the field until the click is over: the input keeps the keyboard and its text.
    let hold = input.id.with("people-held");
    let held = ui.data(|d| d.get_temp::<bool>(hold)).unwrap_or(false);
    let clicking = ui.input(|i| i.pointer.any_down() || i.pointer.any_released());
    ui.data_mut(|d| d.insert_temp(hold, on_popup || (held && clicking)));
    refocus |= on_popup || held;

    // Leaving the field lets go of its card and selection, and closes the text left in the input
    // (it's checked as the field is left).
    if !refocus && laid.edit.lost_focus() {
        st.card = None;
        st.selected = None;
        let text = closed(&st.input);
        st.input.clear();
        st.open = false;
        if !text.is_empty() {
            ev.push(PeopleEvent::Closed(text));
        }
    }
    if refocus && !ui.memory(|m| m.has_focus(edit)) {
        ui.memory_mut(|m| m.request_focus(edit));
    }
    // While the list, a card or a selection is up, Tab and Esc are the field's (taking, closing),
    // not egui's (moving the focus, dropping it).
    let listing = !list::rows(st, input, true).is_empty();
    let held = listing || st.card.is_some() || st.selected.is_some();
    ui.memory_mut(|m| m.set_focus_lock_filter(edit, EventFilter { tab: listing, horizontal_arrows: true, vertical_arrows: true, escape: held }));

    if let Some(n) = st.left_out {
        ui.horizontal(|ui| {
            ui.add_space(label_w(ui, input.label));
            let n = crate::widgets::count_text(n as u64);
            ui.label(egui::RichText::new(tf!("and {n} more were left out", n = n)).color(t.warn).font(crate::theme::regular(12.0)));
        });
    }
    ev
}

/// How far a word under the field is set in, to line up with its chips: past its label, when it
/// has one of its own.
pub fn label_w(ui: &Ui, label: &str) -> f32 {
    if label.is_empty() { 0.0 } else { crate::widgets::field_button_w(ui, label, false) + 8.0 }
}

/// The keys the field takes before its input sees them; whether the input is to get the focus
/// back (a suggestion was taken).
fn keys(ui: &mut Ui, st: &mut FieldState, input: &FieldInput<'_>, rows: &[usize], ev: &mut Vec<PeopleEvent>) -> bool {
    let n = rows.len();
    let mut took = false;
    let mut back = false;
    let mut copy = None;
    ui.input_mut(|i| {
        if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::K)) {
            ev.push(PeopleEvent::Resolve);
        }
        if n > 0 {
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                st.highlighted = (st.highlighted + 1) % n;
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                st.highlighted = (st.highlighted + n - 1) % n;
            }
            // Shift+Tab goes back to the field before, taking nothing (a plain Tab's key would match it).
            if i.consume_key(Modifiers::SHIFT, Key::Tab) {
                st.open = false;
                back = true;
            } else if (i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Tab))
                && let Some(&k) = rows.get(st.highlighted)
            {
                ev.push(PeopleEvent::Take(k));
                st.input.clear();
                st.open = false;
                st.highlighted = 0;
                took = true;
            } else if i.consume_key(Modifiers::NONE, Key::Escape) {
                st.open = false;
                st.dismissed_for = Some(st.input.clone());
            } else if let Some(s) = rows.get(st.highlighted).and_then(|&k| input.suggestions.get(k))
                && s.source == Source::Recent
                && i.consume_key(Modifiers::NONE, Key::Delete)
            {
                ev.push(PeopleEvent::Forget(s.who.address.clone()));
            }
        } else if (st.card.is_some() || st.selected.is_some()) && i.consume_key(Modifiers::NONE, Key::Escape) {
            st.card = None;
            st.selected = None;
        }
        if n == 0 && st.input.is_empty() {
            // Backspace selects the last piece, a second removes it; Delete removes the one selected.
            if let Some(k) = st.selected {
                if i.consume_key(Modifiers::NONE, Key::Backspace) || i.consume_key(Modifiers::NONE, Key::Delete) {
                    ev.push(PeopleEvent::RemoveChip(k));
                    st.selected = None;
                    st.card = None;
                }
            } else if !st.pieces.is_empty() && i.consume_key(Modifiers::NONE, Key::Backspace) {
                st.selected = Some(st.pieces.len() - 1);
            }
        }
        // Ctrl+C: the piece selected, or every entry when nothing is typed (text selected in the
        // input is the input's to copy).
        if i.events.iter().any(|e| matches!(e, egui::Event::Copy)) {
            copy = match st.selected.and_then(|k| st.pieces.get(k)) {
                Some(p) => Some(copy_text(p)),
                None if st.input.is_empty() && !st.pieces.is_empty() => Some(st.pieces.iter().map(copy_text).collect::<Vec<_>>().join("; ")),
                None => None,
            };
            if copy.is_some() {
                i.events.retain(|e| !matches!(e, egui::Event::Copy));
            }
        }
        // A pasted list or address is closed at once, after what was typed.
        let pasted = i.events.iter().find_map(|e| match e {
            egui::Event::Paste(p) if closes(p) => Some(p.clone()),
            _ => None,
        });
        if let Some(p) = pasted {
            i.events.retain(|e| !matches!(e, egui::Event::Paste(_)));
            let text = closed(&format!("{}{}", st.input, p.replace("\r\n", "\n").replace(['\n', '\r'], "; ")));
            st.input.clear();
            st.open = false;
            st.selected = None;
            if !text.is_empty() {
                ev.push(PeopleEvent::Closed(text));
            }
        }
    });
    if let Some(text) = copy {
        ui.ctx().copy_text(text);
    }
    if back {
        ui.memory_mut(|m| m.move_focus(FocusDirection::Previous));
    }
    took
}

/// After the input saw this frame's typing: text closed by a separator goes on as `Closed`, the
/// rest stays typed; a changed input opens the list again.
fn typed(st: &mut FieldState, from: &str, ev: &mut Vec<PeopleEvent>) {
    if st.input == from {
        return;
    }
    if let Some(at) = last_separator(&st.input) {
        let text = closed(st.input.get(..at).unwrap_or_default());
        let rest = st.input.get(at + 1..).unwrap_or_default().trim_start().to_string();
        if !text.is_empty() {
            ev.push(PeopleEvent::Closed(text));
        }
        st.input = rest;
    }
    if st.input != from {
        ev.push(PeopleEvent::Changed);
        st.open = !st.input.trim().is_empty();
        st.highlighted = 0;
        st.dismissed_for = None;
        st.selected = None;
        st.card = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separators_count_outside_quotes_and_angle_brackets() {
        assert_eq!(last_separator("ann;"), Some(3));
        assert_eq!(last_separator("\"Doe, John\" <j"), None);
        assert_eq!(last_separator("<a,b@x>"), None);
        assert_eq!(last_separator("a; b, c"), Some(4));
        assert_eq!(closed(" a@x.fr; ;"), "a@x.fr");
        assert!(closes("a@x.fr") && closes("a\nb") && !closes("Dupont"));
    }
}
