// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: tables in a message, their columns and their rows.

use super::*;

/// A report's table of figures (twelve columns, a dark heading row) stays a table, as Outlook
/// shows it: its cells side by side, its columns sized to their content, scrolling sideways in a
/// pane too narrow for its words. Each cell went on a line of its own (twelve columns can't each
/// be 140 wide).
#[test]
fn a_table_of_figures_stays_a_table() {
    use egui::Shape;
    let heads = [
        "Machine",
        "État",
        "Cœurs",
        "Mémoire (Go)",
        "Charge moy. (%)",
        "Charge p95 (%)",
        "Charge max (%)",
        "Heures > 90 %",
        "Mémoire moy. (%)",
        "Mémoire p95 (%)",
        "Heures mém. > 90 %",
        "Attente p95 / cœur",
    ];
    let row = ["CALCUL-EXEMPLE-07", "Chargé", "8", "32", "41,2", "97", "98,3", "47,5", "12,8", "17,6", "0", "2,4"];
    let cells = |tag: &str, texts: &[&str]| {
        texts.iter().map(|t| format!("<{tag} style=\"padding:2px 6px\">{}</{tag}>", t.replace('>', "&gt;"))).collect::<String>()
    };
    let html = format!(
        "<p>Voici une première analyse :</p><table style=\"border-collapse:collapse\"><tr style=\"background:#1f5f8b;color:#ffffff\">{}</tr>{}</table>",
        cells("th", &heads),
        (0..20).map(|_| format!("<tr>{}</tr>", cells("td", &row))).collect::<String>()
    );
    let (mut a, dir) = app_with_html("figures", "Rapport : semaine 12", &html);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.starts_with("Rapport : semaine")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let text_y = |shapes: &[Shape], text: &str| {
        shapes.iter().find_map(|s| match s {
            Shape::Text(t) if t.galley.text() == text => Some(t.pos.y),
            _ => None,
        })
    };
    // Wide, the whole table in the pane (some headings over two lines); narrow, its words can't
    // all fit: it scrolls sideways, its first columns in sight.
    for (size, last, fits) in [(egui::vec2(2400.0, 900.0), "Attente p95 / cœur", true), (egui::vec2(900.0, 900.0), "État", false)] {
        let mut shown = Vec::new();
        for _ in 0..30 {
            shown = frame_shapes(&ctx, &mut a, vec![], size);
        }
        let texts: Vec<String> =
            shown.iter().filter_map(|s| if let Shape::Text(t) = s { Some(t.galley.text().chars().take(20).collect()) } else { None }).collect();
        // On one row (a heading over two lines sits in the middle of it); stacked, they were a
        // row apart each.
        let (serveur, other) = (text_y(&shown, "Machine"), text_y(&shown, last));
        let side_by_side = serveur.zip(other).is_some_and(|(a, b)| (a - b).abs() < 30.0);
        assert!(side_by_side, "the headings side by side at {size:?}: {serveur:?}, {other:?} in {texts:?}");
        assert_eq!(text_y(&shown, "CALCUL-EXEMPLE-07"), text_y(&shown, "Chargé"), "a row's cells side by side at {size:?}");
        assert_eq!(text_y(&shown, "Attente p95 / cœur").is_some(), fits, "the last column in sight at {size:?}: {texts:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// A striped report, nine columns, one of them empty in most rows: the x each text is drawn at.
fn striped_report(a: &mut MailApp, ctx: &egui::Context, size: egui::Vec2) -> std::collections::HashMap<String, f32> {
    let mut shown = Vec::new();
    for _ in 0..30 {
        shown = frame_shapes(ctx, a, vec![], size);
    }
    shown
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.pos.x)),
            _ => None,
        })
        .collect()
}

/// The vertical lines of a table go straight down (a report seen on 2026-11-02): an empty cell
/// in a white row used to be left out, which slid the cells after it under the wrong headings,
/// and left the row a column short, so it wasn't narrowed as the full rows were and every line
/// in it stood a few pixels off theirs. At any zoom, and with a cell that sets its own width.
#[test]
fn a_tables_columns_line_up_from_row_to_row() {
    let line = "padding:4px 8px;text-align:left;border:1px solid #333";
    let row = |tag: &str, style: &str, cells: [&str; 9]| {
        let cells: String = cells.iter().map(|c| format!("<{tag} style=\"{line}\">{c}</{tag}>")).collect();
        format!("<tr style=\"{style}\">{cells}</tr>")
    };
    let long = "Une ligne assez longue pour que le tableau n'y tienne pas et que ses mots aillent à la ligne ".repeat(2);
    let mut html = String::from("<table style=\"border-collapse:collapse\">");
    html += &row("th", "background:#1f4e79;color:#fff", ["Quand", "Client", "Utilisateur", "Note", "Message", "Rappel", "Ticket", "Serveur", "Mode"]);
    html += &row("td", "", ["Mardi", "Alpha", "Marie", "1", &format!("{long} A"), "Non-a", "", "Srv-a", "Mode-a"]);
    html += &row("td", "background:#dddddd", ["Lundi", "Beta", "Claire", "2", &format!("{long} B"), "Non-b", "", "Srv-b", "Mode-b"]);
    html += &row("td", "", ["Jeudi", "Gamma", "Erika", "3", &format!("{long} C"), "Non-c", "T-37", "Srv-c", "Mode-c"]);
    // This row's own width for the column stays out of the way of the table's.
    html += &row("td", "", ["Jour", "Delta", "Paul", "4", "Court", "Non-d", "", "Srv-d", "Mode-d"]).replace(">Delta<", " width=\"120\">Delta<");
    html += "</table>";
    let (mut a, dir) = app_with_html("columns-line-up", "Rapport", &html);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject == "Rapport").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    for (zoom, width) in [(100, 2400.0), (100, 1900.0), (80, 1900.0), (150, 2800.0)] {
        a.run("view.zoom", json!({"percent": zoom}));
        // Tall enough to draw every row.
        let size = egui::vec2(width, 3000.0);
        let x = striped_report(&mut a, &ctx, size);
        let at = |t: &str| *x.get(t).unwrap_or_else(|| panic!("{t} not drawn at {zoom} {size:?}: {:?}", x.keys().collect::<Vec<_>>()));
        for column in [["Non-a", "Non-b", "Non-c", "Non-d"], ["Srv-a", "Srv-b", "Srv-c", "Srv-d"], ["Mode-a", "Mode-b", "Mode-c", "Mode-d"]] {
            let first = at(column[0]);
            for other in column {
                assert!((at(other) - first).abs() < 0.5, "{other} at {} under {} at {first}, zoom {zoom}, {size:?}", at(other), column[0]);
            }
        }
        assert!(at("Serveur") < at("Srv-a") + 0.5 && at("Srv-a") < at("Mode"), "Srv-a stays under Serveur at zoom {zoom}");
        assert!((at("Srv-a") - at("Serveur")).abs() < 12.0, "under its heading at zoom {zoom}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// A newsletter laid out as most are (issue #41 upstream): blocks of one width stacked on a pale
/// grey page, each `cellspacing="0" cellpadding="0"` with its own padding, a pale yellow tint,
/// a banner whose gradient isn't drawn (its `bgcolor` is), two coloured rows, a layout table
/// without borders, a bordered table of figures. The blocks touch, as in Outlook, and the pale
/// yellow shows (its contrast with the grey is 1.055). Painted light and dark into
/// `QUATRELLE_FRAME_PNG` when set.
#[test]
fn a_newsletters_stacked_blocks_touch_and_keep_their_tint() {
    use super::super::list::follow_up::{Textures, frame_with_size};
    let block = |text: &str, pad: &str| {
        format!(
            "<table width=\"600\" cellpadding=\"0\" cellspacing=\"0\" bgcolor=\"#fff8cc\"><tr><td style=\"padding:{pad}\">{text}</td></tr></table>"
        )
    };
    let banner = "<table width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" bgcolor=\"#f0b800\" \
                  style=\"background:linear-gradient(90deg,#ffcc00,#c08000)\"><tr><td style=\"padding:20px 10px\"><b>STUDIO NEWS</b></td></tr></table>";
    let html = format!(
        "<html><body style=\"margin:0\"><table width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" bgcolor=\"#f2f2f2\"><tr><td align=\"center\">\
         {}{}{}<br>\
         <table width=\"600\" cellpadding=\"4\" cellspacing=\"0\"><tr><td bgcolor=\"#cce8ff\">Row one</td></tr><tr><td bgcolor=\"#b8dcff\">Row two</td></tr></table>\
         <table width=\"600\"><tr><td width=\"200\"><b>Layout column</b></td><td>Beside it, a column of text without borders, as a newsletter puts a picture and its words.</td></tr></table>\
         <table width=\"600\" border=\"1\" cellpadding=\"6\"><tr><td>Week</td><td>Sent</td><td>Read</td></tr><tr><td>44</td><td>1200</td><td>640</td></tr></table>\
         </td></tr></table></body></html>",
        block(banner, "24px"),
        block("First block, line one.", "24px 20px"),
        block("Centred block", "24px 20px; text-align:center"),
    );
    let (mut a, dir) = app_with_html("newsletter", "Studio News", &html);
    let ctx = egui::Context::default();
    let mut tex = Textures::default();
    let pictures = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let size = (1600, 1000);
    frame_with_size(&ctx, &mut a, &mut tex, vec![], None, size);
    let k = a.session.list.iter().find(|m| m.subject == "Studio News").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let fills = |shapes: &[egui::Shape], c: [u8; 3]| -> Vec<egui::Rect> {
        let c = egui::Color32::from_rgb(c[0], c[1], c[2]);
        shapes.iter().filter_map(|s| if let egui::Shape::Rect(r) = s { (r.fill == c).then_some(r.rect) } else { None }).collect()
    };
    for (dark, theme) in [(true, "dark"), (false, "light")] {
        a.ui.dark = Some(dark);
        let mut shown = Vec::new();
        for _ in 0..12 {
            shown = frame_with_size(&ctx, &mut a, &mut tex, vec![], None, size).1;
        }
        let png = pictures.as_ref().map(|d| d.join(format!("newsletter-{theme}.png")));
        if png.is_some() {
            frame_with_size(&ctx, &mut a, &mut tex, vec![], png, size);
        }
        if dark {
            continue;
        }
        let mut yellow = fills(&shown, [0xff, 0xf8, 0xcc]);
        yellow.sort_by(|a, b| a.top().total_cmp(&b.top()));
        assert_eq!(yellow.len(), 3, "the three pale yellow blocks: {yellow:?}");
        for pair in yellow.windows(2) {
            assert!((pair[1].top() - pair[0].bottom()).abs() < 0.5, "stacked blocks touch: {pair:?}");
        }
        let (one, two) = (fills(&shown, [0xcc, 0xe8, 0xff]), fills(&shown, [0xb8, 0xdc, 0xff]));
        assert!(
            one.first().zip(two.first()).is_some_and(|(a, b)| (b.top() - a.bottom()).abs() < 0.5),
            "rows of no cellspacing touch: {one:?} {two:?}"
        );
        assert_eq!(fills(&shown, [0xf0, 0xb8, 0x00]).len(), 1, "the banner's bgcolor under its gradient");
    }
    let _ = std::fs::remove_dir_all(dir);
}
