//! Outlook's address book: what's typed to find someone, the book looked in and who's found,
//! shared by Select Names (a message's To, Cc and Bcc, here) and Select Attendees and Resources
//! (a meeting's, over its window). An Exchange account's directory (the Global Address List) is
//! one of the books: it's asked on the server for the name typed.

use quatrelle_engine::FieldId;
use quatrelle_engine::model::EmailAddress;
use quatrelle_engine::model::addr::Accepts;

use super::people_field::{BOOK_FIELDS, PeopleFields};
use super::*;

/// The address book being looked at.
#[derive(Clone, Debug, Default)]
pub struct AddressSearch {
    pub query: String,
    /// The address book shown: `all` (everyone known), `people` (every contact) or one book
    /// (`book:<account>:<id>`); empty until the one it opens with is known.
    pub source: String,
    /// The one it opens with.
    pub default_source: String,
    /// Each account's address books: (source, name, the account's address).
    pub books: Vec<(String, String, String)>,
    pub list: Vec<EmailAddress>,
    /// The list must be read again (the search or the book changed).
    pub stale: bool,
    /// The session's address book as it was listed ([`Session::contacts_revision`]): listed
    /// again as it fills in.
    pub revision: u64,
    pub selected: Option<usize>,
    /// The directory (`directory:<account>`): still being asked, has more than it gave, or
    /// couldn't be asked.
    pub searching: bool,
    pub more: bool,
    pub error: Option<String>,
}

impl AddressSearch {
    /// One to open: listed at the next frame.
    pub fn new() -> AddressSearch {
        AddressSearch { stale: true, ..Default::default() }
    }

    /// The list from what the session's address book knows now: when the search changed, or
    /// the book did (it's read a slice a frame, the first time). The first time, with the book
    /// to open with (one gone is everyone).
    pub fn refresh(&mut self, s: &mut Session) {
        if !self.stale && self.revision == s.contacts_revision() {
            return;
        }
        if self.source.is_empty() {
            self.books = s.address_books();
            self.default_source = s.prefs.address_book.clone();
            let known = self.default_source == "people" || self.books.iter().any(|(b, _, _)| *b == self.default_source);
            self.source = if known { self.default_source.clone() } else { "all".into() };
        }
        self.list = s.address_book_now(&self.source, &self.query);
        (self.searching, self.more, self.error) = match self.source.strip_prefix("directory:") {
            Some(account) => {
                let found = s.directory_found(account, &self.query);
                (s.directory_searching(account, &self.query), found.is_some_and(|f| f.more), found.and_then(|f| f.error.clone()))
            }
            None => (false, false, None),
        };
        self.revision = s.contacts_revision();
        self.stale = false;
    }

    /// The person picked in the list.
    pub fn picked(&self) -> Option<&EmailAddress> {
        self.selected.and_then(|i| self.list.get(i))
    }
}

/// The address book for a message, and its To, Cc and Bcc as they'll go back to it. The rows are
/// contacts fields of the message's window (`book-to`, `book-cc`, `book-bcc`).
#[derive(Clone, Debug, Default)]
pub struct NamesForm {
    pub search: AddressSearch,
    /// The message it was opened for, and its account (whose directory).
    pub compose: u64,
    pub account: String,
    /// The field a double-click adds to (`to`, `cc`, `bcc`).
    pub target: &'static str,
    pub to: String,
    pub cc: String,
    pub bcc: String,
}

/// Add `a` to a recipient field (`a; b` as Outlook writes them), unless it's there.
pub(crate) fn add_to(field: &mut String, a: &EmailAddress) {
    if field.to_lowercase().contains(&a.email.to_lowercase()) {
        return;
    }
    let f = field.trim().trim_end_matches([';', ',']).trim().to_string();
    *field = if f.is_empty() { format!("{a}; ") } else { format!("{f}; {a}; ") };
}

/// The search box, the address book to look in and the people found, one picked by a click
/// (`id`: the dialog's, two can be open in different windows). `true`: one was double-clicked
/// (to go to the field the dialog was opened from).
pub(crate) fn search(ui: &mut Ui, id: egui::Id, f: &mut AddressSearch, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Search")).strong());
        let r = ui.add(egui::TextEdit::singleline(&mut f.query).desired_width(240.0).hint_text(tl!("Name or address")));
        if ui.memory(|m| m.focused().is_none()) {
            r.request_focus();
        }
        if r.changed() {
            f.stale = true;
            f.selected = None;
        }
        ui.add_space(12.0);
        ui.label(RichText::new(tl!("Address Book")).strong());
        // Everyone (the people written to or heard from, and contacts), every contact, or one
        // address book (each account's Contacts, an organisation's shared ones).
        let accounts = f.books.iter().map(|(_, _, a)| a).collect::<std::collections::BTreeSet<_>>().len();
        let mut choices: Vec<(String, String, &'static str)> = vec![
            ("all".into(), tl!("All addresses").to_string(), tl!("Everyone you've written to or heard from, and your contacts")),
            ("people".into(), tl!("All contacts").to_string(), tl!("Every contact, in every address book")),
        ];
        // Each account's address books when there are several, and each Exchange account's
        // directory.
        let books = f.books.iter().filter(|(s, _, _)| !s.starts_with("directory:")).count();
        for (source, name, account) in &f.books {
            let directory = source.starts_with("directory:");
            if !directory && books < 2 {
                continue;
            }
            let name = match name.as_str() {
                _ if directory => tl!("Global Address List").to_string(),
                "" | "Contacts" => tl!("Contacts").to_string(),
                _ => name.clone(),
            };
            let tip = if directory { tl!("Your organisation's directory, searched on the server for the name typed") } else { "" };
            choices.push((source.clone(), if accounts > 1 { format!("{name} ({account})") } else { name }, tip));
        }
        let shown = choices.iter().find(|(s, _, _)| *s == f.source).map_or_else(|| tl!("All addresses").to_string(), |(_, l, _)| l.clone());
        egui::ComboBox::from_id_salt(id.with("address-book")).width(220.0).selected_text(shown).show_ui(ui, |ui| {
            for (source, label, tip) in &choices {
                let mut r = ui.selectable_label(f.source == *source, label);
                if !tip.is_empty() {
                    r = r.on_hover_text(*tip);
                }
                if r.clicked() && f.source != *source {
                    f.source = source.clone();
                    f.stale = true;
                    f.selected = None;
                }
            }
        });
    });
    // Outlook's "Show this address list first".
    if !f.source.is_empty() && f.source != f.default_source {
        ui.horizontal(|ui| {
            ui.add_space(ui.available_width() - 260.0);
            if ui.link(RichText::new(tl!("Always open with this address book")).small()).clicked() {
                acts.push(Action::Run("contact.defaultBook", json!({"source": f.source})));
                f.default_source = f.source.clone();
            }
        });
    }
    ui.add_space(6.0);
    let mut chosen = false;
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(4).show(ui, |ui| {
        ui.set_min_size(vec2(ui.available_width(), 230.0));
        egui::Grid::new(id.with("names-head")).num_columns(2).min_col_width(250.0).show(ui, |ui| {
            ui.label(RichText::new(tl!("Name")).color(t.dim).small());
            ui.label(RichText::new(tl!("Email Address")).color(t.dim).small());
        });
        ui.separator();
        egui::ScrollArea::vertical().id_salt(id.with("names")).max_height(210.0).auto_shrink([false, false]).show(ui, |ui| {
            let directory = f.source.starts_with("directory:");
            if f.list.is_empty() && !f.stale {
                let said = match &f.error {
                    _ if directory && f.query.trim().is_empty() => tl!("Type a name or an address to search the directory.").to_string(),
                    _ if f.searching => tl!("Searching the directory…").to_string(),
                    Some(e) => tf!("The directory couldn't be searched: {error}", error = e),
                    None => tl!("No one found.").to_string(),
                };
                ui.label(RichText::new(said).color(t.faint));
            }
            for (i, a) in f.list.iter().enumerate() {
                let on = f.selected == Some(i);
                // A row: name and address, left-aligned as in Outlook's list.
                let r = ui.horizontal(|ui| {
                    let name = if a.name.is_empty() { a.email.clone() } else { a.name.clone() };
                    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), egui::Sense::click());
                    if on || r.hovered() {
                        ui.painter().rect_filled(rect, 3.0, if on { t.accent_soft } else { t.hover_row });
                    }
                    // Each in its column, a long one cut short with "…" (the whole of it on hover).
                    let cell = |ui: &Ui, text: &str, x: f32, w: f32, c: egui::Color32| {
                        let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), theme::regular(13.5), c);
                        job.wrap.max_width = w;
                        job.wrap.max_rows = 1;
                        job.wrap.break_anywhere = true;
                        job.wrap.overflow_character = Some('…');
                        let g = ui.fonts_mut(|f| f.layout_job(job));
                        let cut = g.rows.first().is_some_and(|row| row.glyphs.len() < text.chars().count());
                        let pos = egui::pos2(rect.min.x + x, rect.center().y - g.size().y / 2.0);
                        ui.painter()
                            .with_clip_rect(Rect::from_min_size(egui::pos2(rect.min.x + x, rect.min.y), vec2(w + 2.0, rect.height())))
                            .galley(pos, g, c);
                        cut
                    };
                    let name_w = 244.0;
                    let cut_name = cell(ui, &name, 6.0, name_w, t.text);
                    let cut_email = cell(ui, &a.email, 256.0, (rect.width() - 262.0).max(40.0), t.dim);
                    if cut_name || cut_email { r.on_hover_text(format!("{name} <{}>", a.email)) } else { r }
                });
                if r.inner.clicked() {
                    f.selected = Some(i);
                }
                if r.inner.double_clicked() {
                    f.selected = Some(i);
                    chosen = true;
                }
            }
            if directory && f.more && !f.list.is_empty() {
                ui.label(RichText::new(tl!("More people match: type more of the name.")).color(t.faint).small());
            }
        });
    });
    chosen
}

/// The buttons beside the fields, as wide as the longest label (Required -> in some languages).
const FIELD_BUTTON_W: f32 = 112.0;

/// A button and the field it adds the person picked to (`To ->` and To), as Outlook lines them
/// up under the list. `true`: pressed.
pub(crate) fn field_row(ui: &mut Ui, label: &str, field: &mut String) -> bool {
    ui.horizontal(|ui| {
        let pressed = ui.add_sized(vec2(FIELD_BUTTON_W, 24.0), egui::Button::new(tl!(label))).clicked();
        ui.add(egui::TextEdit::singleline(field).desired_width(f32::INFINITY));
        pressed
    })
    .inner
}

pub(super) fn select_names(ui: &mut Ui, f: &mut NamesForm, s: &Session, people: &mut PeopleFields, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(620.0);
    title(ui, t, "Select Names");
    let mut add = search(ui, egui::Id::unique("select-names"), &mut f.search, t, acts).then_some(f.target);
    ui.add_space(8.0);
    // The rows are the contacts field (chips, suggestions, Ctrl+K); the button before each adds
    // the person picked to it, as Outlook's `To ->` does.
    let can_expand = s.directory_account(Some(&f.account)).is_some();
    for (name, target, label) in [("book-to", "to", "To ->"), ("book-cc", "cc", "Cc ->"), ("book-bcc", "bcc", "Bcc ->")] {
        let field = match target {
            "to" => &mut f.to,
            "cc" => &mut f.cc,
            _ => &mut f.bcc,
        };
        let id = FieldId { window: f.compose, name };
        if people.draw(ui, t, id, Some(&f.account), &Accepts::RECIPIENTS, label, "", field, false, can_expand, true, acts) {
            add = Some(target);
        }
    }
    people.choice_dialog(ui.ctx(), t, s, f.compose, &BOOK_FIELDS, acts);
    if let (Some(target), Some(a)) = (add, f.search.picked().cloned()) {
        let field = match target {
            "to" => &mut f.to,
            "cc" => &mut f.cc,
            _ => &mut f.bcc,
        };
        add_to(field, &a);
    }
    let (ok, cancel) = buttons(ui, t, "OK", true);
    if ok {
        acts.push(Action::Submit("compose.update", json!({"to": f.to.trim(), "cc": f.cc.trim(), "bcc": f.bcc.trim()})));
        return true;
    }
    !cancel
}
