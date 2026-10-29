//! Insert › Emoji (in the Message tab's Include group too): the emoji put in last, a search by
//! name or GitHub shortcode, and Unicode's groups, each emoji drawn in colour from the system's
//! emoji font as the rest of the text is (`look/emoji.rs`). Only the rows in view are laid out:
//! the 1,800 emoji don't slow the picker's opening.

use super::*;

use emojis::{Emoji, Group, UnicodeVersion};

/// The newest emoji offered: Unicode 14 (2021), which the colour fonts systems have draw; a
/// newer one would show as an empty box on many.
const NEWEST: UnicodeVersion = UnicodeVersion::new(14, 0);
const PER_ROW: usize = 10;
/// An emoji's square, and its size in it.
const CELL: f32 = 32.0;
const SIZE: f32 = 22.0;
/// The most a search lists.
const FOUND_MAX: usize = 200;

/// Unicode's groups, as the picker's tabs.
const GROUPS: [(Group, &str); 9] = [
    (Group::SmileysAndEmotion, "Smileys & emotion"),
    (Group::PeopleAndBody, "People & body"),
    (Group::AnimalsAndNature, "Animals & nature"),
    (Group::FoodAndDrink, "Food & drink"),
    (Group::TravelAndPlaces, "Travel & places"),
    (Group::Activities, "Activities"),
    (Group::Objects, "Objects"),
    (Group::Symbols, "Symbols"),
    (Group::Flags, "Flags"),
];

/// The picker's search and tab, kept while the message is written.
#[derive(Default)]
pub(crate) struct Picker {
    search: String,
    group: usize,
}

fn offered(e: &Emoji) -> bool {
    e.unicode_version() <= NEWEST
}

/// The emoji whose name or GitHub shortcode holds `query` (`thumbs`, `:+1:`), the ones found by
/// their name first.
pub(super) fn found(query: &str) -> Vec<&'static Emoji> {
    let q = query.trim().trim_matches(':').to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let named = |e: &Emoji| e.name().to_lowercase().contains(&q);
    let mut out: Vec<&'static Emoji> = emojis::iter().filter(|e| offered(e) && named(e)).collect();
    out.extend(emojis::iter().filter(|e| offered(e) && !named(e) && e.shortcodes().any(|s| s.contains(&q))));
    out.truncate(FOUND_MAX);
    out
}

/// The picker, under its button: the emoji clicked, if one was (it stays open for the next).
pub(super) fn picker(ui: &mut Ui, t: &Tokens, recent: &[String], st: &mut Picker) -> Option<String> {
    let width = PER_ROW as f32 * CELL;
    ui.set_width(width);
    let mut chosen = None;
    ui.add(egui::TextEdit::singleline(&mut st.search).hint_text(tl!("Search emoji")).desired_width(width));
    ui.add_space(4.0);
    let list: Vec<(&str, &str)> = if st.search.trim().is_empty() {
        let recent: Vec<(&str, &str)> = recent.iter().map(|r| (r.as_str(), emojis::get(r).map_or("", |e| e.name()))).collect();
        if !recent.is_empty() {
            ui.label(RichText::new(tl!("Recent")).color(t.dim).small());
            for row in recent.chunks(PER_ROW) {
                cells(ui, row, &mut chosen);
            }
            ui.add_space(4.0);
        }
        // The groups as tabs: each its first emoji, its name on hover.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, (g, name)) in GROUPS.iter().enumerate() {
                let face = g.emojis().next().map_or("", Emoji::as_str);
                if ui.selectable_label(st.group == i, RichText::new(face).size(18.0)).on_hover_text(tl!(name)).clicked() {
                    st.group = i;
                }
            }
        });
        let group = GROUPS.get(st.group).map(|(g, _)| *g);
        group.map(|g| g.emojis().filter(|e| offered(e)).map(|e| (e.as_str(), e.name())).collect()).unwrap_or_default()
    } else {
        found(&st.search).into_iter().map(|e| (e.as_str(), e.name())).collect()
    };
    if list.is_empty() {
        ui.label(RichText::new(tl!("No emoji found")).color(t.faint));
        return chosen;
    }
    let rows = list.len().div_ceil(PER_ROW);
    egui::ScrollArea::vertical().id_salt("emoji-grid").max_height(CELL * 7.0).auto_shrink([false, true]).show_rows(ui, CELL, rows, |ui, range| {
        for row in list.chunks(PER_ROW).skip(range.start).take(range.len()) {
            cells(ui, row, &mut chosen);
        }
    });
    chosen
}

/// A row of emoji, each a square button named on hover.
fn cells(ui: &mut Ui, row: &[(&str, &str)], chosen: &mut Option<String>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (emoji, name) in row {
            let b = egui::Button::new(RichText::new(*emoji).size(SIZE)).frame(false).min_size(vec2(CELL, CELL));
            let r = ui.add_sized(vec2(CELL, CELL), b);
            let r = if name.is_empty() { r } else { r.on_hover_text(*name) };
            if r.clicked() {
                *chosen = Some((*emoji).to_string());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_emoji_is_found_by_its_name_or_its_shortcode() {
        let thumbs: Vec<&str> = found("thumbs").iter().map(|e| e.as_str()).collect();
        assert!(thumbs.contains(&"👍") && thumbs.contains(&"👎"), "{thumbs:?}");
        assert_eq!(found(":+1:").first().map(|e| e.as_str()), Some("👍"));
        assert_eq!(found("  ").len(), 0);
        assert!(found("zzzz-nothing").is_empty());
        assert!(found("face").len() <= FOUND_MAX);
        // Only what the systems' colour fonts draw: nothing newer than Unicode 14.
        assert!(emojis::iter().filter(|e| offered(e)).all(|e| e.unicode_version() <= NEWEST));
        assert!(GROUPS.iter().all(|(g, _)| g.emojis().any(offered)));
    }
}
