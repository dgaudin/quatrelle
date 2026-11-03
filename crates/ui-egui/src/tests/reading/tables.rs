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
        "Serveur",
        "Statut",
        "vCPU",
        "RAM (Go)",
        "CPU moyen (%)",
        "CPU p95 (%)",
        "CPU max (%)",
        "Heures CPU > 90%",
        "RAM moyenne (%)",
        "RAM p95 (%)",
        "Heures RAM > 90%",
        "File CPU p95 / cœur",
    ];
    let row = ["D4BATCHVN001", "Saturé", "10", "36", "34,6", "99", "99,7", "50,9", "14,2", "19,2", "0", "2,7"];
    let cells = |tag: &str, texts: &[&str]| {
        texts.iter().map(|t| format!("<{tag} style=\"padding:2px 6px\">{}</{tag}>", t.replace('>', "&gt;"))).collect::<String>()
    };
    let html = format!(
        "<p>Pour les Batch voici une première analyse :</p><table style=\"border-collapse:collapse\"><tr style=\"background:#1f5f8b;color:#ffffff\">{}</tr>{}</table>",
        cells("th", &heads),
        (0..20).map(|_| format!("<tr>{}</tr>", cells("td", &row))).collect::<String>()
    );
    let (mut a, dir) = app_with_html("figures", "CR Astreinte : Semaine 43", &html);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.starts_with("CR Astreinte")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let text_y = |shapes: &[Shape], text: &str| {
        shapes.iter().find_map(|s| match s {
            Shape::Text(t) if t.galley.text() == text => Some(t.pos.y),
            _ => None,
        })
    };
    // Wide, the whole table in the pane (some headings over two lines); narrow, its words can't
    // all fit: it scrolls sideways, its first columns in sight.
    for (size, last, fits) in [(egui::vec2(2400.0, 900.0), "File CPU p95 / cœur", true), (egui::vec2(900.0, 900.0), "Statut", false)] {
        let mut shown = Vec::new();
        for _ in 0..30 {
            shown = frame_shapes(&ctx, &mut a, vec![], size);
        }
        let texts: Vec<String> =
            shown.iter().filter_map(|s| if let Shape::Text(t) = s { Some(t.galley.text().chars().take(20).collect()) } else { None }).collect();
        // On one row (a heading over two lines sits in the middle of it); stacked, they were a
        // row apart each.
        let (serveur, other) = (text_y(&shown, "Serveur"), text_y(&shown, last));
        let side_by_side = serveur.zip(other).is_some_and(|(a, b)| (a - b).abs() < 30.0);
        assert!(side_by_side, "the headings side by side at {size:?}: {serveur:?}, {other:?} in {texts:?}");
        assert_eq!(text_y(&shown, "D4BATCHVN001"), text_y(&shown, "Saturé"), "a row's cells side by side at {size:?}");
        assert_eq!(text_y(&shown, "File CPU p95 / cœur").is_some(), fits, "the last column in sight at {size:?}: {texts:?}");
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
