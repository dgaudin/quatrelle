//! The contacts field drawn alone (`look/people_field.rs`), with no `Session`: its keys, its
//! chips and their kinds, the suggestion list under the input, and a chip's card.

use egui::accesskit::Node;
use quatrelle_engine::model::addr::Accepts;
use quatrelle_engine::model::{Addressee, AddresseeKind};
use quatrelle_engine::{Source, Suggestion};

use super::*;
use crate::look::people_field::{
    CHIP_ROW_H, FieldInput, FieldState, MAX_LINES, PeopleEvent, Piece, PieceError, input_id, kind_name, people_field, source_name, text_of,
};
use crate::theme::Tokens;

/// What one frame of the field gave: its events, the rect it took, what was drawn, and what egui
/// was told (the clipboard, the screen reader's tree).
struct Drawn {
    events: Vec<PeopleEvent>,
    rect: egui::Rect,
    shapes: Vec<egui::Shape>,
    out: egui::PlatformOutput,
}

/// One frame of the field alone, `width` wide, at the top left of a window 1000 by 700.
fn draw_in(ctx: &egui::Context, st: &mut FieldState, input: &FieldInput<'_>, events: Vec<Event>, width: f32) -> Drawn {
    let raw = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))), events, ..Default::default() };
    let t = Tokens::light();
    let mut got = (Vec::new(), egui::Rect::NOTHING);
    let mut out = ctx.run_ui(raw, |ui| {
        ui.allocate_ui(egui::vec2(width, 650.0), |ui| {
            let r = ui.scope(|ui| people_field(ui, &t, st, input));
            got = (r.inner, r.response.rect);
        });
    });
    out.textures_delta.clear();
    fn flat(s: egui::Shape, out: &mut Vec<egui::Shape>) {
        match s {
            egui::Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
            s => out.push(s),
        }
    }
    let mut shapes = Vec::new();
    for s in out.shapes {
        flat(s.shape, &mut shapes);
    }
    Drawn { events: got.0, rect: got.1, shapes, out: out.platform_output }
}

/// One frame of the field alone, 600 wide: its events.
fn draw(ctx: &egui::Context, st: &mut FieldState, input: &FieldInput<'_>, events: Vec<Event>) -> Vec<PeopleEvent> {
    draw_in(ctx, st, input, events, 600.0).events
}

fn person(name: &str, address: &str) -> Addressee {
    Addressee::new(name, address, AddresseeKind::Person)
}

fn recent(who: Addressee) -> Suggestion {
    Suggestion { who, source: Source::Recent }
}

/// The field `id`'s input, asking for the focus (as a message window does once, when it opens).
fn field<'a>(id: &str, suggestions: &'a [Suggestion]) -> FieldInput<'a> {
    FieldInput {
        id: egui::Id::unique(id),
        label: "To",
        hint: "Add recipients",
        accepts: &Accepts::RECIPIENTS,
        suggestions,
        can_expand: false,
        focus: true,
        pending: 0,
    }
}

/// The suggestion list's rect, when it shows.
fn list_rect(ctx: &egui::Context) -> Option<egui::Rect> {
    ctx.memory(|m| m.areas().visible_layer_ids().into_iter().filter(|l| l.order == egui::Order::Foreground).find_map(|l| m.area_rect(l.id)))
}

fn press(at: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

/// Where `text` was drawn (the one on top), in the middle of it.
fn drawn_at(shapes: &[egui::Shape], text: &str) -> Option<egui::Rect> {
    shapes.iter().rev().find_map(|s| match s {
        egui::Shape::Text(t) if t.galley.text() == text => Some(t.galley.rect.translate(t.pos.to_vec2())),
        _ => None,
    })
}

/// What was put on the clipboard this frame.
fn copied(out: &egui::PlatformOutput) -> Option<String> {
    out.commands.iter().find_map(|c| match c {
        egui::OutputCommand::CopyText(s) => Some(s.clone()),
        _ => None,
    })
}

/// Typing opens the list; ↑/↓ move in it, Enter takes, Tab takes while it's open and lets the
/// focus move on once it's closed, Esc closes it for the text typed, Delete forgets a Recent
/// one, Ctrl+K resolves.
#[test]
fn typing_opens_suggestions_and_the_keys_take_move_and_close() {
    let ctx = egui::Context::default();
    let two = [recent(person("Zoe Park", "zoe@studio.example")), recent(person("Zoey Lane", "zoey@studio.example"))];
    let mut st = FieldState::default();
    assert_eq!(draw(&ctx, &mut st, &field("to", &two), vec![]), vec![]);
    // From here the field keeps the focus by itself.
    let input = FieldInput { focus: false, ..field("to", &two) };

    assert_eq!(draw(&ctx, &mut st, &input, vec![Event::Text("zo".into())]), vec![PeopleEvent::Changed]);
    assert!(st.open && st.input == "zo" && st.highlighted == 0);
    draw(&ctx, &mut st, &input, vec![]);
    assert!(list_rect(&ctx).is_some(), "the list shows under the input");
    assert_eq!(draw(&ctx, &mut st, &input, vec![key(Key::ArrowDown, Modifiers::NONE)]), vec![]);
    assert_eq!(st.highlighted, 1);
    draw(&ctx, &mut st, &input, vec![key(Key::ArrowDown, Modifiers::NONE), key(Key::ArrowUp, Modifiers::NONE)]);
    assert_eq!(st.highlighted, 1, "down wraps to the first, up back to the second");

    // Esc closes the list for the text typed; the field keeps the focus.
    assert_eq!(draw(&ctx, &mut st, &input, vec![key(Key::Escape, Modifiers::NONE)]), vec![]);
    assert!(!st.open);
    assert_eq!(st.dismissed_for.as_deref(), Some("zo"));
    draw(&ctx, &mut st, &input, vec![]);
    assert!(list_rect(&ctx).is_none(), "closed");
    assert!(ctx.memory(|m| m.has_focus(input_id(input.id))), "Esc closed the list, not the field");

    // Typing again opens it again; Enter takes the one highlighted.
    assert_eq!(draw(&ctx, &mut st, &input, vec![Event::Text("e".into())]), vec![PeopleEvent::Changed]);
    assert!(st.open && st.dismissed_for.is_none() && st.highlighted == 0);
    draw(&ctx, &mut st, &input, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    assert_eq!(draw(&ctx, &mut st, &input, vec![key(Key::Enter, Modifiers::NONE)]), vec![PeopleEvent::Take(1)]);
    assert!(st.input.is_empty() && !st.open, "the text typed became the suggestion taken");

    // Tab takes while the list is open.
    draw(&ctx, &mut st, &input, vec![Event::Text("zo".into())]);
    draw(&ctx, &mut st, &input, vec![]);
    assert_eq!(draw(&ctx, &mut st, &input, vec![key(Key::Tab, Modifiers::NONE)]), vec![PeopleEvent::Take(0)]);

    // Delete forgets a Recent one; Ctrl+K resolves.
    draw(&ctx, &mut st, &input, vec![Event::Text("zo".into())]);
    draw(&ctx, &mut st, &input, vec![]);
    assert_eq!(draw(&ctx, &mut st, &input, vec![key(Key::Delete, Modifiers::NONE)]), vec![PeopleEvent::Forget("zoe@studio.example".into())]);
    assert_eq!(draw(&ctx, &mut st, &input, vec![key(Key::K, Modifiers::COMMAND)]), vec![PeopleEvent::Resolve]);
    // Delete on one from Contacts forgets nothing (Recent ones only are forgotten).
    let saved = [Suggestion { who: person("Zoe Park", "zoe@studio.example"), source: Source::Contacts }];
    assert_eq!(draw(&ctx, &mut st, &FieldInput { focus: false, ..field("to", &saved) }, vec![key(Key::Delete, Modifiers::NONE)]), vec![]);

    // With no suggestions, Tab takes nothing and the focus moves on, closing what was typed.
    let none = FieldInput { focus: false, ..field("to", &[]) };
    draw(&ctx, &mut st, &none, vec![]);
    assert_eq!(draw(&ctx, &mut st, &none, vec![key(Key::Tab, Modifiers::NONE)]), vec![PeopleEvent::Closed("zo".into())]);
    assert!(!ctx.memory(|m| m.has_focus(input_id(none.id))), "Tab moved the focus on");
    // The focus back (the Tab left it to give on: the book button takes it first, then the input).
    draw(&ctx, &mut st, &field("to", &[]), vec![]);
    draw(&ctx, &mut st, &field("to", &[]), vec![]);
    assert!(ctx.memory(|m| m.has_focus(input_id(none.id))));
    assert_eq!(draw(&ctx, &mut st, &none, vec![key(Key::Tab, Modifiers::NONE)]), vec![], "nothing typed: nothing to close");
    assert!(!ctx.memory(|m| m.has_focus(input_id(none.id))), "Tab moved the focus on");
}

/// `;` (or `,`) closes what's typed, outside quotes; Backspace in an empty input selects the last
/// chip and a second removes it; a pasted list is closed whole; leaving the input closes its
/// text; Ctrl+C copies a chip, or every entry.
#[test]
fn a_separator_closes_the_text_and_backspace_takes_a_chip_in_two_presses() {
    let ctx = egui::Context::default();
    let mut st = FieldState::default();
    draw(&ctx, &mut st, &field("cc", &[]), vec![]);
    let input = FieldInput { focus: false, ..field("cc", &[]) };
    assert_eq!(draw(&ctx, &mut st, &input, vec![Event::Text("ann;".into())]), vec![PeopleEvent::Closed("ann".into())]);
    assert!(st.input.is_empty());
    // A comma inside quotes is part of the name.
    assert_eq!(draw(&ctx, &mut st, &input, vec![Event::Text("\"Doe, John\" <j".into())]), vec![PeopleEvent::Changed]);
    assert_eq!(
        draw(&ctx, &mut st, &input, vec![Event::Text("@x.fr>, b".into())]),
        vec![PeopleEvent::Closed("\"Doe, John\" <j@x.fr>".into()), PeopleEvent::Changed]
    );
    assert_eq!(st.input, "b");
    draw(&ctx, &mut st, &input, vec![key(Key::Backspace, Modifiers::NONE)]);
    assert!(st.input.is_empty() && st.selected.is_none(), "a letter goes first");

    st.pieces = vec![Piece::Chip(person("Ann Lee", "ann@studio.example"))];
    assert_eq!(draw(&ctx, &mut st, &input, vec![key(Key::Backspace, Modifiers::NONE)]), vec![]);
    assert_eq!(st.selected, Some(0));
    // Ctrl+C copies the chip selected.
    let d = draw_in(&ctx, &mut st, &input, vec![Event::Copy], 600.0);
    assert_eq!(copied(&d.out).as_deref(), Some("Ann Lee <ann@studio.example>"));
    assert_eq!(draw(&ctx, &mut st, &input, vec![key(Key::Backspace, Modifiers::NONE)]), vec![PeopleEvent::RemoveChip(0)]);
    assert_eq!(st.selected, None);

    // Nothing selected and nothing typed: Ctrl+C copies every entry (a contact group by its name).
    let mut group = Addressee::new("Dupont family", "", AddresseeKind::ContactGroup);
    group.item_id = "AAMk=".into();
    st.pieces = vec![Piece::Chip(person("Ann Lee", "ann@studio.example")), Piece::Chip(group), Piece::Text { text: "dupont".into(), error: None }];
    let d = draw_in(&ctx, &mut st, &input, vec![Event::Copy], 600.0);
    assert_eq!(copied(&d.out).as_deref(), Some("Ann Lee <ann@studio.example>; Dupont family; dupont"));
    st.pieces.clear();

    // A paste of a list is closed whole.
    assert_eq!(
        draw(&ctx, &mut st, &input, vec![Event::Paste("a@x.fr; John Doe <b@y.fr>".into())]),
        vec![PeopleEvent::Closed("a@x.fr; John Doe <b@y.fr>".into())]
    );
    assert!(st.input.is_empty());

    // Text left in the input when the focus goes elsewhere is closed.
    draw(&ctx, &mut st, &input, vec![Event::Text("bob".into())]);
    let far = egui::pos2(590.0, 590.0);
    let mut seen = draw(&ctx, &mut st, &input, vec![Event::PointerMoved(far), press(far, true)]);
    seen.extend(draw(&ctx, &mut st, &input, vec![press(far, false)]));
    assert_eq!(seen, vec![PeopleEvent::Closed("bob".into())]);
    assert!(st.input.is_empty());
}

/// Sixty chips in 400 pixels wrap, the field grows to four lines and scrolls past them, and the
/// list opened by typing sits under the input.
#[test]
fn chips_wrap_and_scroll_past_four_lines() {
    let ctx = egui::Context::default();
    let one = [recent(person("Pat Pierce", "pat@studio.example"))];
    let input = field("bcc", &one);
    let edit_rect = || ctx.read_response(input_id(input.id)).map(|r| r.rect).expect("the input is drawn");
    let mut st = FieldState::default();
    draw_in(&ctx, &mut st, &input, vec![], 400.0);
    // Where a line starts: where the input is with nothing before it.
    let line_start = edit_rect().left();
    st.pieces = vec![Piece::Chip(person("Pat Pierce", "pat@studio.example"))];
    let d = draw_in(&ctx, &mut st, &input, vec![], 400.0);
    assert!(d.rect.height() <= CHIP_ROW_H + 8.0, "one line: {}", d.rect.height());

    // Sixty chips, or a few fewer so that the input shares the last line with chips.
    let chips = |n: usize| (0..n).map(|i| Piece::Chip(person(&format!("Person {i}"), &format!("p{i}@studio.example")))).collect::<Vec<_>>();
    let mut d = draw_in(&ctx, &mut st, &input, vec![], 400.0);
    for n in (54..=60).rev() {
        st.pieces = chips(n);
        for _ in 0..3 {
            draw_in(&ctx, &mut st, &input, vec![], 400.0);
        }
        d = draw_in(&ctx, &mut st, &input, vec![], 400.0);
        if edit_rect().left() > line_start + 20.0 {
            break;
        }
    }
    assert!(edit_rect().left() > line_start + 20.0, "the input follows a chip on its line: {:?}", edit_rect());
    assert!(d.rect.height() <= MAX_LINES as f32 * CHIP_ROW_H + 8.0, "four lines at most: {}", d.rect.height());
    assert!(d.rect.height() >= (MAX_LINES - 1) as f32 * CHIP_ROW_H, "it grew: {}", d.rect.height());
    assert!(d.rect.width() <= 400.0);

    draw_in(&ctx, &mut st, &input, vec![Event::Text("pa".into())], 400.0);
    draw_in(&ctx, &mut st, &input, vec![], 400.0);
    draw_in(&ctx, &mut st, &input, vec![], 400.0);
    let edit = edit_rect();
    let list = list_rect(&ctx).expect("typing opened the list");
    assert!(edit.top() > d.rect.top() + CHIP_ROW_H, "the input follows the chips, lines down: {edit:?} in {:?}", d.rect);
    assert!((list.top() - edit.bottom()).abs() <= 3.0, "the list under the input: {list:?}, the input {edit:?}");
    assert!((list.left() - edit.left()).abs() <= 1.0, "the list starts at the input, not at the line's start: {list:?}, {edit:?}");
}

/// A chip says its name, kind and how to remove it to a screen reader; a refused piece is drawn
/// red-underlined; a click opens the chip's card, which offers Expand for a list when the account
/// can expand one.
#[test]
fn a_chip_is_a_named_widget_with_its_kind_and_a_refused_one_is_red() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let input = field("rooms", &[]);
    let seine = Addressee::new("Salle Seine", "room.seine@contoso.example", AddresseeKind::Room);
    let refused = PieceError::Refused("This isn't a room".into());
    let mut st = FieldState { pieces: vec![Piece::Chip(seine), Piece::Text { text: "Seine B".into(), error: Some(refused) }], ..Default::default() };
    draw_in(&ctx, &mut st, &input, vec![], 600.0);
    let d = draw_in(&ctx, &mut st, &input, vec![], 600.0);
    let tree = d.out.accesskit_update.as_ref().expect("AccessKit is on");
    let label = |n: &Node| n.label().map(str::to_string);
    assert!(
        tree.nodes.iter().any(|(_, n)| label(n).as_deref() == Some("Salle Seine, Room, remove with Delete")),
        "{:?}",
        tree.nodes.iter().filter_map(|(_, n)| label(n)).collect::<Vec<_>>()
    );
    let text = drawn_at(&d.shapes, "Seine B").expect("the refused piece is drawn");
    let danger = Tokens::light().danger;
    let under = d.shapes.iter().any(|s| match s {
        egui::Shape::LineSegment { points, stroke } => {
            stroke.color == danger && points[0].y >= text.center().y && points[0].y <= text.bottom() + 6.0 && points[1].x > text.left()
        }
        _ => false,
    });
    assert!(under, "a red line under the refused piece");

    // A click on the chip opens its card: Remove, and no Expand for a room.
    let chip = drawn_at(&d.shapes, "Salle Seine").expect("the chip's name is drawn").center();
    draw(&ctx, &mut st, &input, vec![Event::PointerMoved(chip), press(chip, true)]);
    draw(&ctx, &mut st, &input, vec![press(chip, false)]);
    assert_eq!(st.card, Some(0));
    let d = draw_in(&ctx, &mut st, &input, vec![], 600.0);
    assert!(drawn_at(&d.shapes, "Remove").is_some() && drawn_at(&d.shapes, "Expand").is_none());
    assert!(drawn_at(&d.shapes, "room.seine@contoso.example").is_some() && drawn_at(&d.shapes, "Room").is_some(), "the address and the kind");

    // A list's card offers Expand when the account can, and it asks for the expansion.
    let list = Addressee::new("Design team", "design@contoso.example", AddresseeKind::DistributionList);
    st.pieces[0] = Piece::Chip(list);
    let d = draw_in(&ctx, &mut st, &input, vec![], 600.0);
    assert!(drawn_at(&d.shapes, "Expand").is_none(), "no Expand without a directory");
    let can = FieldInput { can_expand: true, ..field("rooms", &[]) };
    draw_in(&ctx, &mut st, &can, vec![], 600.0);
    let d = draw_in(&ctx, &mut st, &can, vec![], 600.0);
    let expand = drawn_at(&d.shapes, "Expand").expect("Expand for a list").center();
    let mut seen = draw(&ctx, &mut st, &can, vec![Event::PointerMoved(expand), press(expand, true)]);
    seen.extend(draw(&ctx, &mut st, &can, vec![press(expand, false)]));
    assert_eq!(seen, vec![PeopleEvent::Expand(0)]);
    assert_eq!(st.card, None, "the card closes");
    assert_eq!(kind_name(AddresseeKind::Unknown), "Address");
    assert_eq!(source_name(Source::Directory), "Directory (cached)");
}

/// The field's text is what Check Names writes: its entries joined by "; ", then "; " and what's
/// typed.
#[test]
fn text_of_round_trips_check_names_output() {
    let zoe = person("Zoe Park", "zoe@studio.example");
    let pieces = [Piece::Chip(zoe), Piece::Text { text: "nobody here".into(), error: Some(PieceError::NotFound) }];
    assert_eq!(text_of(&pieces, ""), "Zoe Park <zoe@studio.example>; nobody here; ");
    assert_eq!(text_of(&pieces, "ann"), "Zoe Park <zoe@studio.example>; nobody here; ann");
    assert_eq!(text_of(&[], "dup"), "dup");
    let mut group = Addressee::new("Dupont family", "", AddresseeKind::ContactGroup);
    group.item_id = "AAMk=".into();
    assert_eq!(text_of(&[Piece::Chip(group)], ""), "Dupont family <group:AAMk=>; ");
}
