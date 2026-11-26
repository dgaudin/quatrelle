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

/// A report as Outlook writes one: a table 2100 pixels wide, each column's width set in pixels,
/// each cell `nowrap` with its text in a centred paragraph. Its narrow columns went one letter
/// per line: the centred paragraphs
/// (one-cell boxes) made it a layout's table, squeezed into the pane, and a column set narrower
/// than its text kept that width. Now each column is at least as wide as its line, as
/// Thunderbird and Outlook show it, and the table scrolls sideways.
#[test]
fn a_report_of_set_widths_keeps_its_words_whole() {
    let widths = [90u32, 58, 52, 80, 400, 860, 560];
    let cell = |w: u32, bg: &str, t: &str| {
        format!(
            "<td width=\"{w}\" nowrap=\"\" style=\"width:{}.0pt;{bg}padding:0cm 3.5pt 0cm 3.5pt;height:14.4pt\">\
             <p class=\"MsoNormal\" align=\"center\" style=\"text-align:center\"><span style=\"font-size:11.0pt;\
             font-family:&quot;Aptos Narrow&quot;,sans-serif;color:black\">{t}<o:p></o:p></span></p></td>",
            w * 3 / 4
        )
    };
    let row = |bg: &str, texts: [&str; 7]| format!("<tr>{}</tr>", widths.iter().zip(texts).map(|(w, t)| cell(*w, bg, t)).collect::<String>());
    let mut html = String::from(
        "<table class=\"MsoNormalTable\" border=\"0\" cellspacing=\"0\" cellpadding=\"0\" width=\"2100\" style=\"width:1575.0pt;border-collapse:collapse\">",
    );
    html +=
        &row("background:#AEABAB;", ["<b>Jour</b>", "<b>Début</b>", "<b>Durée</b>", "<b>Nature</b>", "<b>Site</b>", "<b>Détail</b>", "<b>Suite</b>"]);
    for _ in 0..6 {
        html += &row(
            "",
            ["12/03/2026", "2h15", "45min", "Sauvegarde", "Studio Alpha", "Une copie de nuit arrêtée à mi-chemin", "Copie relancée, finie à 6 h"],
        );
    }
    html += "</table>";
    let (mut a, dir) = app_with_html("set-widths", "Rapport de garde", &html);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject == "Rapport de garde").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    for width in [1400.0, 3600.0] {
        let mut shown = Vec::new();
        for _ in 0..30 {
            shown = frame_shapes(&ctx, &mut a, vec![], egui::vec2(width, 1200.0));
        }
        let lines = |text: &str| {
            shown.iter().find_map(|s| match s {
                egui::Shape::Text(t) if t.galley.text() == text => Some(t.galley.rows.len()),
                _ => None,
            })
        };
        for text in ["Jour", "Début", "Durée", "Nature", "12/03/2026", "2h15", "45min", "Sauvegarde"] {
            assert_eq!(lines(text), Some(1), "{text} on one line at {width}");
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// A signature in a frame, as a mailing's template draws one (made up here): a row of a red
/// edge 2 wide, the content 604, the other edge 2, and a last cell set no width that holds only
/// a spacer 0 wide. That cell holds nothing and takes no room: the frame's right edge stands 604
/// from its left, and the content's columns keep their widths, its words whole. It was given a
/// column's least (140), the content shrank by a fifth, its right edge stood short of the lines
/// above and under it, and `See you` broke inside its word.
#[test]
fn a_frames_empty_last_cell_takes_no_room() {
    let html = "<table width=\"608\" cellspacing=\"0\" cellpadding=\"0\" border=\"0\"><tr>\
                <td width=\"2\" bgcolor=\"#e2001a\"></td>\
                <td width=\"604\" bgcolor=\"#ffffff\"><table width=\"604\" cellspacing=\"0\" cellpadding=\"0\" border=\"0\"><tr>\
                <td width=\"135\">Any question?</td><td width=\"2\" bgcolor=\"#000001\"></td><td width=\"409\">Shops near you</td>\
                <td width=\"58\"><b>See you</b></td></tr></table></td>\
                <td width=\"2\" bgcolor=\"#e2001a\"></td>\
                <td bgcolor=\"#ffffff\"><img src=\"https://cdn.example/s.png\" width=\"0\" height=\"32\"></td>\
                </tr></table>";
    let (mut a, dir) = app_with_html("frame-last-cell", "Framed signature", html);
    let ctx = egui::Context::default();
    let size = egui::vec2(2000.0, 900.0);
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject == "Framed signature").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let mut shown = Vec::new();
    for _ in 0..12 {
        shown = frame_shapes(&ctx, &mut a, vec![], size);
    }
    let red = egui::Color32::from_rgb(0xe2, 0x00, 0x1a);
    let mut edges: Vec<egui::Rect> =
        shown.iter().filter_map(|s| if let egui::Shape::Rect(r) = s { (r.fill == red).then_some(r.rect) } else { None }).collect();
    edges.sort_by(|a, b| a.left().total_cmp(&b.left()));
    let [left, right] = edges.as_slice() else { panic!("the frame's two edges: {edges:?}") };
    assert!((right.left() - left.right() - 604.0).abs() < 1.5, "the content keeps its 604: {left:?} {right:?}");
    let lines = shown.iter().find_map(|s| match s {
        egui::Shape::Text(t) if t.galley.text() == "See you" => Some(t.galley.rows.len()),
        _ => None,
    });
    assert_eq!(lines, Some(1), "its last column's words whole");
    let _ = std::fs::remove_dir_all(dir);
}

/// A signature laid out as some mail programs write one (made up here): the name, then a table
/// whose first column, 30 pixels wide, holds a tiny icon on the web (13 pixels wide, in a box
/// of colour) beside each contact line, and a picture 1 pixel square by its style after it.
fn icon_signature() -> String {
    let row = |icon: &str, alt: &str, text: &str| {
        format!(
            "<tr style=\"vertical-align: middle; height: 25px;\"><td style=\"vertical-align: middle; width: 30px; height: 25px;\">\
             <table border=\"0\" cellspacing=\"0\" cellpadding=\"0\"><tbody><tr><td style=\"vertical-align: bottom;\">\
             <span style=\"display: inline-block; background-color: #2266cc;\"><img style=\"border-width: 0px; vertical-align: middle; \
             display: block;\" src=\"https://img.contoso.example/sig/{icon}.png\" alt=\"{alt}\" width=\"13\"></span></td></tr></tbody></table></td>\
             <td style=\"padding: 0px; width: 424px; height: 25px;\"><span style=\"font-size: 14px;\">{text}</span></td></tr>"
        )
    };
    format!(
        "<div>Hello Riley,</div><div>The new build is ready.</div>\
         <div class=\"mail_signature\"><table border=\"0\" cellspacing=\"0\" cellpadding=\"0\" style=\"font-family: Georgia;\"><tbody><tr>\
         <td style=\"padding: 0px; vertical-align: middle;\"><h2 style=\"margin: 0px; font-size: 18px;\">Ana Exemple</h2>\
         <div><span style=\"font-size: 14px;\">Sales, Contoso</span></div>\
         <table border=\"0\" cellspacing=\"0\" cellpadding=\"0\" style=\"width: 454px; height: 100px;\"><tbody>{}{}{}</tbody></table></td>\
         <td colspan=\"3\" style=\"max-width: 300px; font-size: 12px;\"><div><p style=\"margin: 0px;\">Thanks</p></div></td><td width=\"46\"></td>\
         </tr></tbody></table></div>\
         <img style=\"height: 1px !important; max-height: 1px !important; max-width: 1px !important; width: 1px !important\" \
         src=\"https://t.contoso.example/open.png\">",
        row("phone", "mobilePhone", "+1 555 0100"),
        row("mail", "emailAddress", "ana.lima@contoso.example"),
        row("web", "website", "<a href=\"https://www.contoso.example/\">www.contoso.example</a>"),
    )
}

/// A signature's column of tiny icons, not loaded (they're on the web), reads as Outlook shows it:
/// each icon a small box of its colour, its contact line beside it, the lines one under the
/// other and lined up. Each icon's description went down its 30-pixel column a few letters a
/// line (`[`, `mob`, `ileP`…), the contact lines pushed out of line, and the picture of 1 pixel
/// sized by its style showed as `[picture]`. Each icon's description is still said to a screen
/// reader. Painted into `QUATRELLE_FRAME_PNG` when set.
#[test]
fn a_signatures_tiny_icons_not_loaded_keep_its_lines_in_line() {
    use super::super::list::follow_up::{Textures, frame_with_size};
    let (mut a, dir) = app_with_html("icon-signature", "Build ready", &icon_signature());
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut tex = Textures::default();
    // A reading pane wide enough for the signature's table: its rows stay side by side.
    let size = (2400, 1000);
    frame_with_size(&ctx, &mut a, &mut tex, vec![], None, size);
    let k = a.session.list.iter().find(|m| m.subject == "Build ready").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let (mut said, mut shown) = (Vec::new(), Vec::new());
    for _ in 0..12 {
        (said, shown) = frame_with_size(&ctx, &mut a, &mut tex, vec![], None, size);
    }
    for alt in ["mobilePhone", "emailAddress", "website"] {
        assert!(said.iter().any(|l| l == alt), "{alt} said: {said:?}");
    }
    let texts: Vec<(String, egui::Rect, usize)> = shown
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.galley.rect.translate(t.pos.to_vec2()), t.galley.rows.len())),
            _ => None,
        })
        .collect();
    // No word broken across lines.
    for s in &shown {
        let egui::Shape::Text(t) = s else { continue };
        for pair in t.galley.rows.windows(2) {
            let end = pair[0].row.glyphs.last().map(|g| g.chr);
            let start = pair[1].row.glyphs.first().map(|g| g.chr);
            let at_space = pair[0].ends_with_newline || end.is_none_or(char::is_whitespace) || start.is_none_or(char::is_whitespace);
            assert!(at_space, "{:?} broken inside a word: {texts:?}", t.galley.text());
        }
    }
    assert!(!texts.iter().any(|(t, ..)| t.contains("[picture]")), "the tracking pixel shows nothing: {texts:?}");
    // The contact lines: each on one line, lined up, one under the other.
    let lines: Vec<egui::Rect> = ["+1 555 0100", "ana.lima@contoso.example", "www.contoso.example"]
        .iter()
        .map(|l| {
            let (_, r, rows) = texts.iter().find(|(t, ..)| t == l).unwrap_or_else(|| panic!("{l} drawn: {texts:?}"));
            assert_eq!(*rows, 1, "{l} on one line");
            *r
        })
        .collect();
    for pair in lines.windows(2) {
        assert!((pair[0].left() - pair[1].left()).abs() < 0.5, "lined up: {lines:?}");
        assert!(pair[1].top() >= pair[0].bottom() - 0.5 && pair[1].top() - pair[0].top() < 40.0, "one under the other: {lines:?}");
    }
    // Each icon: a small box of its colour beside its line, as tall as it is wide (its height
    // isn't set), apart from the next one.
    let blue = egui::Color32::from_rgb(0x22, 0x66, 0xcc);
    let mut boxes: Vec<egui::Rect> =
        shown.iter().filter_map(|s| if let egui::Shape::Rect(r) = s { (r.fill == blue).then_some(r.rect) } else { None }).collect();
    boxes.sort_by(|a, b| a.top().total_cmp(&b.top()));
    for line in &lines {
        let beside =
            boxes.iter().any(|b| (b.width() - 13.0).abs() < 1.5 && b.right() <= line.left() && b.bottom() > line.top() && b.top() < line.bottom());
        assert!(beside, "a 13-point box of the icon's colour beside {line:?}: {boxes:?}");
    }
    assert!(boxes.iter().all(|b| (b.height() - b.width()).abs() < 0.5), "square: {boxes:?}");
    assert!(boxes.windows(2).all(|p| p[1].top() - p[0].bottom() >= 2.0), "apart, not one bar: {boxes:?}");
    // Too small to write their descriptions in.
    let over: Vec<&String> = texts.iter().filter(|(_, r, _)| boxes.iter().any(|b| b.shrink(1.0).intersects(*r))).map(|(t, ..)| t).collect();
    assert!(over.is_empty(), "nothing written in the icons: {over:?}");
    // Light and dark, and in a pane too narrow for the table (its rows one cell under the other).
    if let Some(d) = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from) {
        for (dark, size, name) in [(false, size, "light"), (true, size, "dark"), (false, (1400, 900), "narrow")] {
            a.ui.dark = Some(dark);
            for _ in 0..12 {
                frame_with_size(&ctx, &mut a, &mut tex, vec![], None, size);
            }
            frame_with_size(&ctx, &mut a, &mut tex, vec![], Some(d.join(format!("icon-signature-{name}.png"))), size);
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// A newsletter's centred cell (`align="center"`) with a short paragraph, then one too long for a
/// line with a line break in it: each row of the long one is centred, as a browser centres it,
/// not only a line that fits (the wrapped ones kept to the left). Painted into
/// `QUATRELLE_FRAME_PNG` when set.
#[test]
fn a_centred_paragraph_that_wraps_is_centred_row_by_row() {
    use super::super::list::follow_up::{Textures, frame_with_size};
    let long_one =
        "You are one of the members picked for our yearly survey of the studio's rooms, classes and opening hours: it takes about a minute.";
    let long_two = "Your answers shape the next season of the studio, and we thank you in advance for the time you give us today.";
    let html = format!(
        "<table width=\"600\" align=\"center\" cellpadding=\"0\" cellspacing=\"0\" bgcolor=\"#ffffff\"><tr><td style=\"padding:30px 20px\">\
         <table width=\"100%\" cellpadding=\"0\" cellspacing=\"0\"><tr><td align=\"center\">\
         <p style=\"margin:0;font-size:14px\">Tell us what you think of the studio!<br></p>\
         <p style=\"margin:0;font-size:14px\"><br></p>\
         <p style=\"margin:0;font-size:14px\">{long_one}<br><br> {long_two}</p>\
         </td></tr></table></td></tr></table>"
    );
    let (mut a, dir) = app_with_html("centred-wrap", "Survey", &html);
    let ctx = egui::Context::default();
    let mut tex = Textures::default();
    let size = (1600, 1000);
    frame_with_size(&ctx, &mut a, &mut tex, vec![], None, size);
    let k = a.session.list.iter().find(|m| m.subject == "Survey").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let mut shown = Vec::new();
    for _ in 0..12 {
        shown = frame_with_size(&ctx, &mut a, &mut tex, vec![], None, size).1;
    }
    if let Some(d) = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from) {
        frame_with_size(&ctx, &mut a, &mut tex, vec![], Some(d.join("centred-wrap.png")), size);
    }
    // A text's rows where they're drawn (the text alone: not the list's preview of the message).
    let rows = |text: &str| -> Vec<egui::Rect> {
        shown
            .iter()
            .find_map(|s| match s {
                egui::Shape::Text(t) if t.galley.text().trim() == text => {
                    Some(t.galley.rows.iter().map(|r| r.rect_without_leading_space().translate(t.pos.to_vec2())).collect())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{text:?} drawn"))
    };
    let short = rows("Tell us what you think of the studio!");
    assert_eq!(short.len(), 1, "the short paragraph on one line");
    let middle = short[0].center().x;
    for text in [long_one, long_two] {
        let wrapped = rows(text);
        assert!(wrapped.len() >= 2, "{text:?} too long for a line of the column: {wrapped:?}");
        for r in &wrapped {
            assert!((r.center().x - middle).abs() < 6.0, "each row of {text:?} centred on {middle}, as the short one: {wrapped:?}");
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}
