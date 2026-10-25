//! Dialogs: Folder Size, as Outlook's: how much space each of an account's folders takes on this
//! computer (Local Data) and on its server (Server Data), with the mailbox's limits.

use std::collections::HashMap;

use quatrelle_engine::Sizes;
use quatrelle_engine::model::{FolderSize, MailboxQuota};

use super::*;

/// How the folders are listed: by their place, or the biggest first (by their own size, or with
/// their subfolders').
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SizeSort {
    #[default]
    Place,
    Size,
    Total,
}

/// A line of the list: where the folder is (its names from the top down), the space its own
/// items take, the same with its subfolders', and how many items it holds (0: not told).
struct Line {
    path: Vec<String>,
    size: u64,
    total: u64,
    items: u32,
}

const ROW: f32 = 22.0;
const ITEMS_W: f32 = 80.0;
const SIZE_W: f32 = 100.0;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    let Dialog::FolderSize { account, server, sort, kept } = d else { return true };
    ui.set_width(660.0);
    title(ui, t, "Folder Size");
    let a = s.account(account);
    ui.label(RichText::new(a.map_or("", |a| a.config.email.as_str())).color(t.dim));
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.selectable_value(server, false, tl!("Local Data"));
        ui.selectable_value(server, true, tl!("Server Data"));
    });
    ui.add_space(8.0);
    let v = s.sizes.as_ref().filter(|v| v.account == *account);
    let note = |ui: &mut Ui, text: &str| ui.label(RichText::new(text).color(t.dim));
    match v.map(|v| if *server { &v.server } else { &v.local }) {
        None | Some(Sizes::Loading) => {
            ui.horizontal(|ui| {
                ui.spinner();
                note(ui, tl!(if *server { "Asking the server…" } else { "Adding up what's kept here…" }));
            });
        }
        Some(Sizes::Loaded(folders)) => {
            let all = folders.iter().map(|f| f.size).fold(0u64, u64::saturating_add);
            if *server {
                server_summary(ui, t, all, v.and_then(|v| v.quota));
            } else {
                local_summary(ui, t, all, *kept);
            }
            ui.add_space(8.0);
            let lines = lines(s, account, folders, *server, *sort);
            table(ui, t, &lines, sort, *server);
        }
        Some(Sizes::NotKept) => {
            note(ui, tl!("Online only: none of this account's mail is kept on this computer."));
        }
        Some(Sizes::NoServer) => {
            let data_file = a.is_some_and(quatrelle_engine::AccountState::is_data_file);
            note(ui, tl!(if data_file { "Not available: a data file has no server." } else { "Not available: the sample mailbox has no server." }));
        }
        Some(Sizes::NotTold) => {
            note(ui, tl!("This server doesn't tell each folder's size: only Exchange does."));
            if let Some((used, limit)) = a.and_then(|a| a.quota) {
                ui.add_space(6.0);
                quota_bar(ui, t, used, limit);
            }
        }
        Some(Sizes::Failed(e)) => {
            ui.label(RichText::new(tf!("The sizes couldn't be read: {error}", error = e)).color(t.warn));
        }
    }
    ui.add_space(10.0);
    button_row(ui, |ui| {
        let close = ui.button(tl!("Close")).clicked();
        if ui.button(tl!("Refresh")).clicked() {
            acts.push(Action::Run("account.folderSizes", json!({"account": account})));
        }
        !close
    })
}

/// The server's side: the mailbox's size, and its limits when the server tells them.
fn server_summary(ui: &mut Ui, t: &Tokens, all: u64, quota: Option<MailboxQuota>) {
    let size = widgets::size_text;
    egui::Grid::new("sizes-server").num_columns(2).spacing(vec2(16.0, 4.0)).show(ui, |ui| {
        ui.label(RichText::new(tl!("Total size")).color(t.dim));
        ui.label(RichText::new(size(all)).strong());
        ui.end_row();
        if let Some(q) = quota {
            for (label, n) in [("Warning at", q.warning), ("Sending stops at", q.send), ("Receiving stops at", q.receive)] {
                if n > 0 {
                    ui.label(RichText::new(tl!(label)).color(t.dim));
                    ui.label(size(n));
                    ui.end_row();
                }
            }
        }
    });
    ui.add_space(6.0);
    match quota.map(|q| q.limit()).filter(|l| *l > 0) {
        Some(limit) => quota_bar(ui, t, all, limit),
        None => {
            ui.label(RichText::new(tl!("The server doesn't tell this mailbox's limit.")).color(t.faint).small());
        }
    }
}

/// How full the mailbox is, as the File page shows it.
fn quota_bar(ui: &mut Ui, t: &Tokens, used: u64, limit: u64) {
    if limit == 0 {
        ui.label(RichText::new(tf!("{used} used on the server", used = widgets::size_text(used))).color(t.dim));
        return;
    }
    let frac = (used as f32 / limit as f32).clamp(0.0, 1.0);
    ui.horizontal(|ui| {
        widgets::meter(ui, t, frac, 260.0);
        let free = widgets::size_text(limit.saturating_sub(used));
        ui.label(RichText::new(tf!("{free} free of {total}", free = free, total = widgets::size_text(limit))).color(t.dim));
    });
}

/// This computer's side: what the folders take, and the rest of the data file.
fn local_summary(ui: &mut Ui, t: &Tokens, all: u64, kept: u64) {
    let size = widgets::size_text;
    egui::Grid::new("sizes-local").num_columns(2).spacing(vec2(16.0, 4.0)).show(ui, |ui| {
        ui.label(RichText::new(tl!("Mail folders")).color(t.dim));
        ui.label(RichText::new(size(all)).strong());
        ui.end_row();
        if kept > all {
            ui.label(RichText::new(tl!("Calendars, contacts, the search index and free space")).color(t.dim));
            ui.label(size(kept - all));
            ui.end_row();
            ui.label(RichText::new(tl!("In all on this computer")).color(t.dim));
            ui.label(size(kept));
            ui.end_row();
        }
    });
}

/// The folders as listed: each named by its place (this computer's with the names the folder
/// pane gives them), with its subfolders' sizes added to its own, in the order asked for.
fn lines(s: &Session, account: &str, folders: &[FolderSize], server: bool, sort: SizeSort) -> Vec<Line> {
    let known: HashMap<&str, &quatrelle_engine::model::Folder> =
        s.account(account).map(|a| a.folders.iter().map(|f| (f.path.as_str(), f)).collect()).unwrap_or_default();
    let mut lines: Vec<Line> = folders
        .iter()
        .map(|f| {
            let path = match f.folder.as_deref().and_then(|p| known.get(p)) {
                Some(kf) if !server => shown_path(kf, &known),
                _ => f.path.clone(),
            };
            Line { path, size: f.size, total: 0, items: f.items }
        })
        .collect();
    let mut under: HashMap<Vec<String>, u64> = HashMap::new();
    for l in &lines {
        for n in 1..=l.path.len() {
            let e = under.entry(l.path.get(..n).map(<[String]>::to_vec).unwrap_or_default()).or_default();
            *e = e.saturating_add(l.size);
        }
    }
    for l in &mut lines {
        l.total = under.get(&l.path).copied().unwrap_or(l.size);
    }
    match sort {
        SizeSort::Place => lines.sort_by_cached_key(|l| l.path.iter().map(|p| alphabetical(p)).collect::<Vec<_>>()),
        SizeSort::Size => lines.sort_by_key(|l| std::cmp::Reverse(l.size)),
        SizeSort::Total => lines.sort_by_key(|l| std::cmp::Reverse(l.total)),
    }
    lines
}

/// A name as it sorts: case and accents aside (Éléments envoyés with the E's, not after Z).
fn alphabetical(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            c => c,
        })
        .collect()
}

/// A folder kept here, named by its place as the folder pane names each folder on the way.
fn shown_path(f: &quatrelle_engine::model::Folder, known: &HashMap<&str, &quatrelle_engine::model::Folder>) -> Vec<String> {
    let Some(d) = f.delimiter else { return vec![crate::i18n::folder(f)] };
    let parts: Vec<&str> = f.path.split(d).collect();
    (1..=parts.len())
        .map(|n| {
            let at = parts.get(..n).map(|p| p.join(&d.to_string())).unwrap_or_default();
            match known.get(at.as_str()) {
                Some(k) => crate::i18n::folder(k),
                None => quatrelle_engine::model::utf7::decode(parts.get(n - 1).copied().unwrap_or_default()),
            }
        })
        .collect()
}

/// The list, its headings sorting it (the biggest first by size).
fn table(ui: &mut Ui, t: &Tokens, lines: &[Line], sort: &mut SizeSort, server: bool) {
    let width = ui.available_width();
    let numbers = if server { ITEMS_W + 2.0 * SIZE_W } else { 2.0 * SIZE_W };
    let name_w = (width - numbers - 12.0).max(120.0);
    let font = theme::regular(13.0);
    // The headings.
    let (head, _) = ui.allocate_exact_size(vec2(width, ROW), Sense::hover());
    ui.painter().line_segment([head.left_bottom(), head.right_bottom()], egui::Stroke::new(1.0, t.border));
    let mut x = head.left() + 6.0;
    let mut cols: Vec<(&str, f32, Option<SizeSort>, bool)> = vec![("Folder", name_w, Some(SizeSort::Place), false)];
    if server {
        cols.push(("Items", ITEMS_W, None, true));
    }
    cols.push(("Size", SIZE_W, Some(SizeSort::Size), true));
    cols.push(("Total size", SIZE_W, Some(SizeSort::Total), true));
    for (label, w, by, right) in cols {
        let r = egui::Rect::from_min_size(egui::pos2(x, head.top()), vec2(w, ROW));
        let marked = by == Some(*sort);
        let text = if marked { format!("{} ▾", tl!(label)) } else { tl!(label).to_string() };
        let resp = ui.interact(r, egui::Id::unique(("size-head", label)), if by.is_some() { Sense::click() } else { Sense::hover() });
        let color = if marked || resp.hovered() { t.text } else { t.dim };
        let (pos, align) =
            if right { (r.right_center() - vec2(6.0, 0.0), egui::Align2::RIGHT_CENTER) } else { (r.left_center(), egui::Align2::LEFT_CENTER) };
        ui.painter().text(pos, align, text, theme::bold(12.5), color);
        if let Some(by) = by
            && resp.clicked()
        {
            *sort = by;
        }
        x += w;
    }
    egui::ScrollArea::vertical().id_salt("folder-sizes").max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
        for (i, l) in lines.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(width, ROW), Sense::hover());
            if resp.hovered() {
                ui.painter().rect_filled(r, 4.0, t.hover_row);
            } else if i % 2 == 1 {
                ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.5));
            }
            let name = l.path.join(" › ");
            let g = widgets::one_line(ui, &name, font.clone(), t.text, name_w - 8.0);
            ui.painter().galley(egui::pos2(r.left() + 6.0, r.center().y - g.size().y / 2.0), g, t.text);
            let mut right = r.left() + 6.0 + name_w;
            let mut number = |text: String, w: f32, color: egui::Color32| {
                right += w;
                ui.painter().text(egui::pos2(right - 6.0, r.center().y), egui::Align2::RIGHT_CENTER, text, font.clone(), color);
            };
            if server {
                number(widgets::count_text(u64::from(l.items)), ITEMS_W, t.dim);
            }
            number(widgets::size_text(l.size), SIZE_W, t.text);
            number(widgets::size_text(l.total), SIZE_W, if l.total > l.size { t.text } else { t.dim });
        }
        if lines.is_empty() {
            ui.label(RichText::new(tl!("No folders.")).color(t.dim));
        }
    });
}
