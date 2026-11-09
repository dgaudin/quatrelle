// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
use super::*;

fn text(b: &Block) -> String {
    match b {
        Block::Text(r) => r.iter().map(|l| l.iter().map(|s| s.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("\n"),
        Block::Table(_) => String::new(),
    }
}

#[test]
fn a_newsletter_keeps_its_columns() {
    let html = r##"<html><head><style>.hide{display:none}</style></head><body>
            <div style="display:none">Preheader text</div>
            <center><table width="600" align="center" bgcolor="#f4f4f4"><tr><td align="center"><h1>Studio news</h1></td></tr>
            <tr><td><table width="100%"><tr>
                <td width="50%" style="background-color: #ffeedd"><p>Left <b>column</b></p><img src="cid:a@x" alt="poster"></td>
                <td width="50%"><p>Right column</p><a href="https://example.com/x">Read more</a></td>
            </tr></table></td></tr></table></center>
            <p class="hide">Hidden by a rule</p><p>Footer</p></body></html>"##;
    let b = html_layout(html);
    assert!(!b.is_empty());
    // Centre wrapper → table → second row's cell → the two columns.
    let Block::Table(center) = &b[0] else { panic!("{b:?}") };
    let Block::Table(outer) = &center.rows[0][0].blocks[0] else { panic!() };
    assert_eq!(outer.width, Some(Width::Px(600)));
    assert_eq!(outer.align, Align::Center);
    assert_eq!(outer.background, Some([0xf4, 0xf4, 0xf4]));
    assert_eq!(outer.rows[0][0].align, Align::Center);
    assert!(text(&outer.rows[0][0].blocks[0]).contains("Studio news"));
    let Block::Table(cols) = &outer.rows[1][0].blocks[0] else { panic!() };
    let row = &cols.rows[0];
    assert_eq!(row.len(), 2);
    assert_eq!(row[0].width, Some(Width::Percent(50)));
    assert_eq!(row[0].background, Some([0xff, 0xee, 0xdd]));
    let Block::Text(left) = &row[0].blocks[0] else { panic!() };
    assert!(left.iter().flatten().any(|s| s.bold && s.text.contains("column")));
    assert!(left.iter().flatten().any(|s| s.image.as_deref() == Some("cid:a@x")));
    let Block::Text(right) = &row[1].blocks[0] else { panic!() };
    assert!(right.iter().flatten().any(|s| s.link.as_deref() == Some("https://example.com/x")));
    // Hidden parts stay hidden; text after the table follows it.
    let all = format!("{b:?}");
    assert!(!all.contains("Preheader") && !all.contains("Hidden by a rule"), "{all}");
    assert!(text(b.last().unwrap()).contains("Footer"));
}

#[test]
fn css_columns_are_columns_too() {
    // Flexbox, a grid and inline-block boxes, no tables.
    let flex = html_layout(
        r#"<div style="display:flex"><div style="width:30%"><p>Left</p></div><div style="background-color:#eee"><p>Right</p></div></div>"#,
    );
    let Block::Table(t) = &flex[0] else { panic!("{flex:?}") };
    assert_eq!(t.rows[0].len(), 2);
    assert_eq!(t.rows[0][0].width, Some(Width::Percent(30)));
    assert_eq!(t.rows[0][1].background, Some([0xee, 0xee, 0xee]));
    let grid = html_layout(r#"<div style="display: grid; grid-template-columns: repeat(2, 1fr)"><div>a</div><div>b</div><div>c</div></div>"#);
    let Block::Table(t) = &grid[0] else { panic!("{grid:?}") };
    assert_eq!(t.rows.iter().map(Vec::len).collect::<Vec<_>>(), [2, 1]);
    let inline =
        html_layout(r#"<div><div style="display:inline-block;width:280px">One</div> <div style="display:inline-block;width:280px">Two</div></div>"#);
    assert!(matches!(&inline[0], Block::Table(t) if t.rows[0].len() == 2), "{inline:?}");
    // A signature as Outlook writes it: a logo table and a name table floated side by side
    // (with CSS, or `align="left"`), after the message's text.
    let sig = html_layout(
        r#"<p>Thanks, see you Monday.</p>
        <table align="left" border="0" style="float:left;"><tr><td><a href="https://www.example.com"><img src="cid:logo@x" width="194"></a></td></tr></table>
        <table border="0" style="float:left;border-left:1px solid #ccc;"><tr><td><p><span style="font-weight:bold">Didier GAUDIN</span><br>Tél. 02 51 70 93 93</p></td></tr></table>"#,
    );
    let row = sig
        .iter()
        .find_map(|b| match b {
            Block::Table(t) if t.rows.first().is_some_and(|r| r.len() == 2) => t.rows.first(),
            _ => None,
        })
        .expect("logo and name side by side");
    assert_eq!(row.len(), 2);
    // The same signature as Outlook sends it, rewritten by Word: only the logo's table floats
    // (`align="left"`), an empty paragraph (its content hidden) comes before the name's table,
    // which draws a rule down its left edge; the logo's width is in pixels as an attribute and
    // in inches in its style. The name's table sits beside the logo, which is as wide as its
    // picture (it was laid out as text, one under the other).
    let word = html_layout(
        r#"<div class="WordSection1"><p class="MsoNormal">Merci,<o:p></o:p></p>
        <p class="MsoNormal"><o:p>&nbsp;</o:p></p>
        <table class="MsoNormalTable" border="0" cellspacing="0" cellpadding="0" align="left"><tbody><tr><td style="padding:0cm 0cm 0cm 0cm">
        <p class="MsoNormal"><a href="https://www.example.com/"><span style="color:blue;text-decoration:none"><img border="0" width="194" height="75" style="width:2.0208in;height:.7812in" src="cid:image001.gif@01DD6260"></span></a><span><o:p></o:p></span></p>
        </td></tr></tbody></table>
        <p class="MsoNormal"><span style="font-size:11.0pt;display:none"><o:p>&nbsp;</o:p></span></p>
        <table class="MsoNormalTable" border="1" cellspacing="0" cellpadding="0" style="border:none;border-left:solid #CCCCCC 1.0pt"><tbody><tr style="height:47.25pt">
        <td nowrap="" style="border:none;padding:0cm 0cm 0cm 7.5pt;height:47.25pt"><p class="MsoNormal"><b><span style="font-size:10.5pt;font-family:&quot;Arial&quot;,sans-serif;color:#4B5763">Riley MORENO</span></b><br>
        <span style="font-size:10.0pt;color:#4B5763">T&eacute;l.:&nbsp;+&nbsp;33(0)2&nbsp;00&nbsp;00&nbsp;00&nbsp;00</span><br>
        <a href="mailto:riley@example.com"><span style="color:#333333">riley@example.com</span></a></p></td></tr></tbody></table>
        <p class="MsoNormal"><o:p>&nbsp;</o:p></p></div>"#,
    );
    let row = word
        .iter()
        .find_map(|b| match b {
            Block::Table(t) if t.rows.first().is_some_and(|r| r.len() == 2) => t.rows.first(),
            _ => None,
        })
        .unwrap_or_else(|| panic!("logo and name side by side: {word:?}"));
    assert_eq!(row[0].fit, Some(194), "the logo's box as wide as the logo");
    assert_eq!(row[1].fit, None, "the name's takes the rest");
    let rule = Some(Border { width: 1, color: [0xcc, 0xcc, 0xcc] });
    let ruled =
        row[1].blocks.iter().any(|b| matches!(b, Block::Table(t) if t.borders.left == rule && t.borders.right.is_none() && t.borders.top.is_none()));
    assert!(ruled, "the rule between them: {:?}", row[1].blocks);
    // The name's cell: no border of its own (Word's `border:none` over the table's
    // `border="1"`), and its padding in points and centimetres, 10 pixels on the left.
    let Some(Block::Table(name)) = row[1].blocks.first() else { panic!("{:?}", row[1].blocks) };
    assert!(name.rows[0][0].borders.is_none());
    assert_eq!(name.rows[0][0].padding, Some([0, 0, 0, 10]));
    let floats = html_layout(r#"<div><div style="float:left">One</div><div style="float:right">Two</div></div>"#);
    assert!(!floats.is_empty());
    // Stacked flexbox, inline text with spans: no columns.
    assert!(html_layout(r#"<div style="display:flex;flex-direction:column"><div>a</div><div>b</div></div>"#).is_empty());
    assert!(html_layout(r#"<p>Hello <span style="display:inline-block">a</span> and <span style="display:inline-block">b</span></p>"#).is_empty());
}

#[test]
fn pictures_behind_boxes_are_kept() {
    // A banner: a cell with a picture behind its heading (the address keeps its case).
    let banner = html_layout(
        r##"<table width="600"><tr><td bgcolor="#2e7d6b" background="https://img.example/Hero.JPG" style="color:#fff"><h1>Open call</h1></td></tr></table>"##,
    );
    let Block::Table(t) = &banner[0] else { panic!("{banner:?}") };
    assert_eq!(t.rows[0][0].background_image.as_deref(), Some("https://img.example/Hero.JPG"));
    assert_eq!(t.rows[0][0].background, Some([0x2e, 0x7d, 0x6b]), "the colour stays, for when the picture isn't shown");
    // CSS forms, on a `<div>`: `background-image` and `background` with `url()`.
    for css in ["background-image: url('cid:Hero@x')", "background: #000 url(cid:Hero@x) no-repeat center / cover"] {
        let b = html_layout(&format!(r#"<div style="{css}"><p>Over the picture</p></div>"#));
        let Block::Table(t) = &b[0] else { panic!("{css}: {b:?}") };
        assert_eq!(t.rows[0][0].background_image.as_deref(), Some("cid:Hero@x"), "{css}");
        assert!(text(&t.rows[0][0].blocks[0]).contains("Over the picture"));
        assert_eq!(background_pictures(&b), ["cid:Hero@x"]);
    }
    // Columns inside a box with a picture behind it stay columns.
    let cols = html_layout(r#"<div style="display:flex;background-image:url(cid:b@x)"><div>One</div><div>Two</div></div>"#);
    let Block::Table(t) = &cols[0] else { panic!("{cols:?}") };
    assert!(matches!(&t.rows[0][0].blocks[0], Block::Table(inner) if inner.rows[0].len() == 2), "{cols:?}");
    // Nothing that runs, and nothing local.
    for bad in ["javascript:alert(1)", "file:///etc/passwd", "data:text/html,x"] {
        assert!(html_layout(&format!(r#"<div style="background-image:url({bad})"><p>x</p></div>"#)).is_empty(), "{bad}");
    }
}

#[test]
fn text_takes_its_cells_colour() {
    // White on a dark band, set on the cell (or the table), as CSS inherits it.
    let colour_of = |b: &[Block], word: &str| {
        format!("{b:?}").contains(word).then(|| {
            fn spans(b: &[Block], out: &mut Vec<(String, Option<[u8; 3]>)>) {
                for x in b {
                    match x {
                        Block::Text(r) => out.extend(r.iter().flatten().map(|s| (s.text.clone(), s.color))),
                        Block::Table(t) => t.rows.iter().flatten().for_each(|c| spans(&c.blocks, out)),
                    }
                }
            }
            let mut v = Vec::new();
            spans(b, &mut v);
            v.into_iter().find(|(t, _)| t.contains(word)).and_then(|(_, c)| c)
        })
    };
    let cell = html_layout(r##"<table><tr><td style="color:#ffffff" bgcolor="#123456">Band</td><td>Other</td></tr></table>"##);
    assert_eq!(colour_of(&cell, "Band").flatten(), Some([255, 255, 255]), "{cell:?}");
    assert_eq!(colour_of(&cell, "Other").flatten(), None, "a neighbour keeps its own");
    let table = html_layout(r##"<table style="color:#00ff00"><tr><td>A</td><td>B</td></tr></table>"##);
    assert_eq!(colour_of(&table, "A").flatten(), Some([0, 255, 0]));
    // A colour can't smuggle markup in.
    let evil = html_layout(r##"<table><tr><td style="color:red&quot;><script>x</script>">A</td><td>B</td></tr></table>"##);
    assert!(!format!("{evil:?}").contains("script"), "{evil:?}");
}

#[test]
fn boxes_placed_over_others_are_kept() {
    // A label over a picture, and a note in the bottom right corner.
    let b = html_layout(
        r##"<div style="position:relative; width:300px"><img src="cid:poster@x" alt="Poster">
               <span style="position:absolute; top:8px; left:10px; background:#c62828; color:#fff">NEW</span>
               <div style="position:absolute; right:4px; bottom:6px; width:120px">Ends Friday</div></div>"##,
    );
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    assert_eq!(t.width, Some(Width::Px(300)));
    let cell = &t.rows[0][0];
    assert!(matches!(&cell.blocks[0], Block::Text(r) if r.iter().flatten().any(|s| s.image.as_deref() == Some("cid:poster@x"))));
    assert_eq!(cell.overlays.len(), 2);
    let (new, ends) = (&cell.overlays[0], &cell.overlays[1]);
    assert_eq!((new.x, new.y, new.from_right, new.from_bottom), (10, 8, false, false));
    assert_eq!(new.background, Some([0xc6, 0x28, 0x28]));
    assert!(text(&new.blocks[0]).contains("NEW"));
    assert_eq!((ends.x, ends.y, ends.from_right, ends.from_bottom), (4, 6, true, true));
    assert_eq!(ends.width, Some(Width::Px(120)));
    // Placed with nothing to sit over: it flows; absurd offsets are capped.
    assert!(html_layout(r#"<p>Hi <span style="position:absolute;top:3px">there</span></p>"#).is_empty());
    let far = html_layout(r#"<div style="position:relative"><p>a</p><b style="position:absolute;top:99999999px;left:-50px">b</b></div>"#);
    let Block::Table(t) = &far[0] else { panic!("{far:?}") };
    assert_eq!((t.rows[0][0].overlays[0].x, t.rows[0][0].overlays[0].y), (0, MAX_OFFSET));
}

/// Borders (CSS and `border="1"`), round corners, padding (CSS and `cellpadding`) and where a
/// cell's content sits in its row come with the cells, as a newsletter, a receipt and Word's
/// signature set them.
#[test]
fn borders_padding_and_alignment_are_kept() {
    let grey = Border { width: 1, color: [0x99, 0x99, 0x99] };
    // A newsletter: a framed, rounded card with a rounded button (its link not underlined).
    let news = html_layout(
        r##"<table width="600" align="center" style="border:1px solid #dddddd; border-radius:8px; border-collapse:separate">
        <tr><td style="padding:20px"><h1>Studio news</h1><p>What's on this month.</p></td></tr>
        <tr><td align="center" style="padding:0 20px 20px">
            <table cellpadding="0" cellspacing="0"><tr>
            <td bgcolor="#1a73e8" style="border-radius:4px; padding:12px 24px"><a href="https://example.com/shop" style="color:#ffffff;text-decoration:none">Shop now</a></td>
            </tr></table>
        </td></tr></table>"##,
    );
    let Block::Table(card) = &news[0] else { panic!("{news:?}") };
    assert_eq!(card.borders.uniform(), Some(Border { width: 1, color: [0xdd, 0xdd, 0xdd] }));
    assert_eq!(card.radius, 8);
    assert_eq!(card.rows[0][0].padding, Some([20; 4]));
    assert!(card.rows[0][0].borders.is_none(), "the cells of a bordered table have none of their own");
    assert_eq!(card.rows[1][0].padding, Some([0, 20, 20, 20]));
    let Block::Table(inner) = &card.rows[1][0].blocks[0] else { panic!("{card:?}") };
    let button = &inner.rows[0][0];
    assert_eq!((button.background, button.radius, button.padding), (Some([0x1a, 0x73, 0xe8]), 4, Some([12, 24, 12, 24])));
    assert_eq!(button.valign, VAlign::Middle, "a cell's content sits in the middle unless it says otherwise");
    assert!(!format!("{button:?}").contains("underline: true"), "the button's link isn't underlined: {button:?}");
    // A receipt: `border="1"` draws the table's edge and every cell's; `cellpadding` pads them.
    let receipt = html_layout(
        r#"<table border="1" cellpadding="4" cellspacing="0"><tr><th>Item</th><th>Price</th></tr>
        <tr valign="bottom"><td valign="top" style="padding:0">Poster</td><td align="right" style="border-color:#ff0000">40</td></tr></table>"#,
    );
    let Block::Table(t) = &receipt[0] else { panic!("{receipt:?}") };
    assert_eq!(t.borders.uniform(), Some(grey));
    assert_eq!(t.rows[0][0].borders.uniform(), Some(grey));
    assert_eq!(t.rows[0][0].padding, Some([4; 4]));
    assert_eq!((t.rows[1][0].padding, t.rows[1][0].valign), (Some([0; 4]), VAlign::Top), "the cell's own padding and alignment");
    assert_eq!(t.rows[1][1].valign, VAlign::Bottom, "the row's alignment");
    assert_eq!(t.rows[1][1].borders.uniform(), Some(Border { width: 1, color: [255, 0, 0] }), "the attribute's border, recoloured");
    assert_eq!(t.rows[1][1].radius, 0);
    // A table of one column with a border is a layout worth keeping (the border shows);
    // its style's `border:none` takes the attribute's away from the table, not from its cells.
    let boxed = html_layout(r#"<table border="1" style="border:none"><tr><td>Note</td></tr></table>"#);
    let Block::Table(t) = &boxed[0] else { panic!("{boxed:?}") };
    assert!(t.borders.is_none());
    assert_eq!(t.rows[0][0].borders.uniform(), Some(grey));
    assert_eq!(html_layout(r#"<table border="2"><tr><td>Note</td></tr></table>"#).len(), 1);
    // Boxes laid out with CSS keep their edges and padding too, at the top of their row.
    let flex = html_layout(r#"<div style="display:flex"><div style="border-bottom:2px dashed #000;padding:4px 8px">a</div><div>b</div></div>"#);
    let Block::Table(t) = &flex[0] else { panic!("{flex:?}") };
    assert_eq!(t.rows[0][0].borders, Edges { bottom: Some(Border { width: 2, color: [0, 0, 0] }), ..Default::default() });
    assert_eq!((t.rows[0][0].padding, t.rows[0][0].valign), (Some([4, 8, 4, 8]), VAlign::Top));
    assert!(t.rows[0][1].borders.is_none() && t.rows[0][1].padding.is_none());
    // Hostile values: widths and padding capped, nothing panics.
    let huge =
        html_layout(r#"<table style="border:99999px solid #000"><tr><td style="padding:99999px;border-radius:-5px">a</td><td>b</td></tr></table>"#);
    let Block::Table(t) = &huge[0] else { panic!("{huge:?}") };
    assert_eq!(t.borders.top.map(|b| b.width), Some(MAX_BORDER));
    assert_eq!(t.rows[0][0].padding, Some([MAX_PADDING; 4]));
    assert_eq!(t.rows[0][0].radius, 0);
}

#[test]
fn plain_mail_and_one_column_tables_have_no_layout() {
    assert!(html_layout("<p>Hello <b>there</b></p>").is_empty());
    assert!(html_layout("<table><tr><td>One</td></tr><tr><td>Two</td></tr></table>").is_empty());
    // A receipt (a data table) has columns.
    assert!(!html_layout("<table><tr><th>Item</th><th>Price</th></tr><tr><td>Poster</td><td>40</td></tr></table>").is_empty());
}

/// An empty cell in a report holds its place: left out, the cells after it slid under the wrong
/// headings, in the white rows only (a striped row's colour kept its empty cells).
#[test]
fn an_empty_cell_holds_its_place_in_its_column() {
    let b = html_layout(
        "<table><tr><th>Who</th><th>Ticket</th><th>Server</th></tr>\
         <tr><td>Ann</td><td></td><td>S1</td></tr>\
         <tr bgcolor=\"#dddddd\"><td>Bob</td><td></td><td>S2</td></tr>\
         <tr><td>Cid</td><td>&nbsp;</td><td>S3</td></tr>\
         <tr><td>Dee</td><td>T4</td></tr></table>",
    );
    let Some(Block::Table(t)) = b.first() else { panic!("a table: {b:?}") };
    let shape: Vec<usize> = t.rows.iter().map(Vec::len).collect();
    assert_eq!(shape, [3, 3, 3, 3, 2], "a short row stays short");
    for (row, server) in [(1, "S1"), (2, "S2"), (3, "S3")] {
        let cell = t.rows.get(row).and_then(|r| r.get(2)).expect("a third cell");
        assert_eq!(cell.blocks.first().map(text).as_deref(), Some(server));
    }
}

/// What's empty all the way down is a layout's spacer, and is left out as it always was: a
/// gutter between two columns (under a banner across them too), a row of nothing.
#[test]
fn a_column_or_a_row_of_nothing_is_left_out() {
    let shape = |html: &str| {
        let b = html_layout(html);
        let Some(Block::Table(t)) = b.first() else { panic!("a table: {b:?}") };
        t.rows.iter().map(Vec::len).collect::<Vec<usize>>()
    };
    assert_eq!(shape("<table><tr><td>Left</td><td></td><td>Right</td></tr></table>"), [2]);
    assert_eq!(
        shape(
            "<table><tr><td colspan=\"3\">Banner</td></tr><tr><td>A</td><td width=\"20\">&nbsp;</td><td>B</td></tr><tr><td>C</td><td></td><td>D</td></tr></table>"
        ),
        [1, 2, 2]
    );
    assert_eq!(shape("<table><tr><td>A</td><td>B</td></tr><tr><td></td><td></td></tr><tr><td>C</td><td>D</td></tr></table>"), [2, 2]);
    // An empty cell across columns that show something holds their place.
    assert_eq!(shape("<table><tr><td>A</td><td>B</td><td>C</td><td>D</td></tr><tr><td>E</td><td colspan=\"2\"></td><td>F</td></tr></table>"), [4, 3]);
}

#[test]
fn hostile_markup_is_survived() {
    // Unclosed, misnested, deep, huge.
    let deep = "<table><tr><td>".repeat(500) + "x";
    let _ = html_layout(&deep);
    let _ = html_layout("<table><td>a<td>b</tr></tr></table></td><tr><td><td>");
    let _ = html_layout("<table><tr><td><script>if (a < b) { document.write('<td>') }</script>z</td><td>y</td></tr></table>");
    let many = "<table><tr>".to_string() + &"<td>c</td>".repeat(5000) + "</tr></table>";
    assert!(html_layout(&many).is_empty(), "too many cells: flows as text");
    assert_eq!(color("#abc"), Some([0xaa, 0xbb, 0xcc]));
    assert_eq!(color("rgb(1, 2, 300)"), Some([1, 2, 255]));
    assert_eq!(color("#12"), None);
}

/// A message in a centred column of a set width keeps it, one column or not: it was stretched
/// across the window (upstream issue #10).
#[test]
fn a_centred_column_of_a_set_width_keeps_its_width() {
    // The usual table: a full-width one, a 600-pixel one centred in it.
    let b = html_layout(
        r#"<body><table width="100%"><tr><td align="center"><table width="600"><tr><td><h1>Spring sale</h1><p>All week.</p></td></tr>
        <tr><td><img src="cid:banner@x" width="600"></td></tr></table></td></tr></table></body>"#,
    );
    let Some(Block::Table(outer)) = b.first() else { panic!("{b:?}") };
    assert_eq!(outer.rows[0][0].align, Align::Center);
    let Block::Table(inner) = &outer.rows[0][0].blocks[0] else { panic!("{b:?}") };
    assert_eq!(inner.width, Some(Width::Px(600)));
    // A box: `max-width` and auto margins, no table at all.
    let b = html_layout(r#"<div style="max-width:600px;margin:0 auto;background-color:#ffffff"><p>Hello</p><p>Thanks for your order.</p></div>"#);
    let Some(Block::Table(t)) = b.first() else { panic!("{b:?}") };
    assert_eq!((t.width, t.align), (Some(Width::Px(600)), Align::Center));
    assert_eq!(t.rows[0][0].background, Some([0xff, 0xff, 0xff]));
    assert!(text(&t.rows[0][0].blocks[0]).contains("Thanks for your order"));
    // Narrower than the window: `width: 100%` held to its `max-width`; `<center>` around a box.
    let b = html_layout(r#"<center><div style="width:100%;max-width:480px;text-align:left">Hi there</div></center>"#);
    let Some(Block::Table(t)) = b.first() else { panic!("{b:?}") };
    let Block::Table(t) = &t.rows[0][0].blocks[0] else { panic!("{b:?}") };
    assert_eq!(t.width, Some(Width::Px(480)));
    assert_eq!(t.rows[0][0].align, Align::Left);
    // No set width: the text flows, as before.
    assert!(html_layout(r#"<div style="width:100%"><p>Just text</p></div>"#).is_empty());
    assert!(html_layout("<p>Plain <b>mail</b></p><table><tr><td>one cell</td></tr></table>").is_empty());
}

/// A business card (upstream issue #11): the photo's column as wide as the photo, the details
/// beside it.
#[test]
fn a_photo_column_is_as_wide_as_its_photo() {
    let b = html_layout(r#"<table><tr><td><img src="cid:p@x" width="80" height="80"></td><td><b>Dean Moss</b><br>Sales</td></tr></table>"#);
    let Some(Block::Table(t)) = b.first() else { panic!("{b:?}") };
    assert_eq!(t.rows[0].iter().map(|c| (c.width, c.fit)).collect::<Vec<_>>(), [(None, Some(80)), (None, None)]);
    // A picture of no set width, or with text beside it, leaves the column as it was.
    let b = html_layout(r#"<table><tr><td><img src="cid:p@x"> Hi</td><td>There</td></tr></table>"#);
    let Some(Block::Table(t)) = b.first() else { panic!("{b:?}") };
    assert_eq!((t.rows[0][0].width, t.rows[0][0].fit), (None, None));
    let b = html_layout(r#"<table><tr><td><img src="cid:p@x"></td><td>There</td></tr></table>"#);
    let Some(Block::Table(t)) = b.first() else { panic!("{b:?}") };
    assert_eq!(t.rows[0][0].fit, None);
}

/// Upstream issue #41: a centred column whose width is in points (as Outlook and Word write it),
/// in `em` or `rem` (as email frameworks write it), set on the body, or given by an attribute
/// beside a style width a browser can't read, keeps its width instead of filling the window.
#[test]
fn a_centred_column_in_points_or_ems_keeps_its_width() {
    let column = |html: &str| -> Option<(Option<Width>, Align)> {
        fn first(b: &[Block]) -> Option<(Option<Width>, Align)> {
            let Some(Block::Table(t)) = b.first() else { return None };
            if matches!(t.width, Some(Width::Px(_))) {
                return Some((t.width, t.align));
            }
            first(&t.rows.first()?.first()?.blocks)
        }
        first(&html_layout(&inline_css(html)))
    };
    let word = r#"<div class=WordSection1><div align=center><table class=MsoNormalTable border=0 cellspacing=0 cellpadding=0 width=600
        style='width:450.0pt;mso-cellspacing:0cm' align=center><tr><td style='padding:0cm'><p class=MsoNormal>Hello from Outlook</p></td></tr></table></div></div>"#;
    assert_eq!(column(word), Some((Some(Width::Px(600)), Align::Center)));
    let framework = r#"<body style="background-color:#ffffff"><table align="center" width="100%" role="presentation"
        style="max-width:37.5em;margin:0 auto"><tbody><tr style="width:100%"><td><p>Your code is ready.</p></td></tr></tbody></table></body>"#;
    assert_eq!(column(framework), Some((Some(Width::Px(600)), Align::Center)));
    assert_eq!(column(r#"<div style="width:40rem;margin:0 auto"><p>rem</p></div>"#), Some((Some(Width::Px(640)), Align::Center)));
    assert_eq!(column(r#"<div style="width:6.25in;margin:0 auto"><p>inches</p></div>"#), Some((Some(Width::Px(600)), Align::Center)));
    assert_eq!(column(r#"<body style="width:600px;margin:0 auto"><p>In the body itself</p></body>"#), Some((Some(Width::Px(600)), Align::Center)));
    assert_eq!(
        column(r#"<table width="560" align="center" style="width:35vw"><tr><td>Unread unit</td></tr></table>"#).map(|c| c.0),
        Some(Some(Width::Px(560)))
    );
    // A style a browser reads still wins over the attribute, and `auto` sets no width.
    assert!(html_layout(r#"<table width="600" align="center" style="width:100%"><tr><td>Full width</td></tr></table>"#).is_empty());
    assert!(html_layout(r#"<table width="600" style="width:auto"><tr><td>Auto</td></tr></table>"#).is_empty());
    assert_eq!(length_px("12pt"), Some(16.0));
    assert_eq!(length_px("2.54cm").map(f32::round), Some(96.0));
}

/// Upstream issue #41, as Outlook shows it: a title centred in the column (`text-align: center`)
/// stays centred, and its size, set on a span inside `<b>` as Outlook writes it, makes it a title.
#[test]
fn a_centred_title_in_a_column_stays_centred_and_big() {
    let html = r#"<table width=600 style='width:450.0pt' align=center><tr><td>
        <p class=MsoNormal style='text-align:center'><b><span style='font-size:20.0pt;color:#1F3864'>Quarterly update</span></b></p>
        <p class=MsoNormal>Hello everyone,</p></td></tr></table>"#;
    let b = html_layout(&inline_css(html));
    let Some(Block::Table(column)) = b.first() else { panic!("{b:?}") };
    let cell = &column.rows[0][0].blocks;
    let Some(Block::Table(title)) = cell.first() else { panic!("{cell:?}") };
    assert_eq!((title.align, title.width, title.rows[0][0].align), (Align::Center, None, Align::Center));
    let Some(Block::Text(lines)) = title.rows[0][0].blocks.first() else { panic!("{title:?}") };
    let Some(span) = lines.iter().flatten().find(|s| s.text.contains("Quarterly")) else { panic!("{lines:?}") };
    assert!(span.heading > 0 && span.bold, "{span:?}");
    assert_eq!(span.color, Some([0x1F, 0x38, 0x64]));
    assert!(matches!(cell.get(1), Some(Block::Text(t)) if t.iter().flatten().any(|s| s.text.contains("Hello") && s.heading == 0)), "{cell:?}");
    // Text that flows (no column) keeps flowing: no layout for a centred paragraph alone.
    assert!(html_layout(r#"<p style="text-align:center">Just centred</p>"#).is_empty());
}

/// Upstream issue #41, the tester's newsletter: blocks of one width stacked one on another (each
/// its own table, or a table's rows) keep their own room, as in Outlook: the tables'
/// `cellspacing` (HTML's 2 when not said, none with `border-collapse`), `cellpadding` and each
/// cell's padding; a line break between two of them is the empty line a browser leaves; a
/// gradient Outlook doesn't draw shows its `bgcolor`; a box of CSS keeps its own padding.
#[test]
fn stacked_blocks_keep_their_own_room() {
    let html = r##"<table width="100%" cellpadding="0" cellspacing="0"><tr><td align="center">
        <table width="600" cellpadding="0" cellspacing="0" bgcolor="#fff8cc"><tr><td style="padding:24px">
          <table width="100%" cellpadding="0" cellspacing="0" style="background:linear-gradient(90deg,#ffcc00,#c08000)" bgcolor="#f0b800">
            <tr><td style="padding:20px 10px">BANNER</td></tr></table></td></tr></table>
        <table width="600" cellpadding="0" cellspacing="0" bgcolor="#fff8cc"><tr><td style="padding:24px 20px 12pt">First block</td></tr></table>
        <br>
        <table width="600" cellpadding="4" cellspacing="0"><tr><td bgcolor="#cce8ff">Row one</td></tr><tr><td bgcolor="#b8dcff">Row two</td></tr></table>
        <table width="600"><tr><td>HTML's own room</td></tr></table>
        <table width="600" cellspacing="8" style="border-collapse:collapse"><tr><td>Collapsed</td></tr></table>
        <table width="600" style="border-spacing:6px"><tr><td>Spaced by CSS</td></tr></table>
        </td></tr></table>"##;
    let b = html_layout(&inline_css(html));
    let Block::Table(outer) = &b[0] else { panic!("{b:?}") };
    assert_eq!(outer.spacing, Some(0));
    let parts = &outer.rows[0][0].blocks;
    let tables: Vec<&Table> = parts.iter().filter_map(|p| if let Block::Table(t) = p { Some(t) } else { None }).collect();
    let room: Vec<Option<u8>> = tables.iter().map(|t| t.spacing).collect();
    assert_eq!(room, [Some(0), Some(0), Some(0), Some(2), Some(0), Some(6)]);
    assert_eq!(tables[0].rows[0][0].padding, Some([24, 24, 24, 24]));
    assert_eq!(tables[1].rows[0][0].padding, Some([24, 20, 16, 20]), "a missing side is the second's; points as pixels");
    assert_eq!(tables[2].rows[1][0].padding, Some([4; 4]), "its table's cellpadding");
    assert_eq!(tables[3].rows[0][0].padding, None, "the reader's own");
    let Block::Table(banner) = &tables[0].rows[0][0].blocks[0] else { panic!() };
    assert_eq!(banner.background, Some([0xf0, 0xb8, 0x00]), "the bgcolor under a gradient");
    assert_eq!(banner.rows[0][0].padding, Some([20, 10, 20, 10]));
    // The line break between the second block and the rows: one empty line; nothing between
    // the first two.
    let at = parts.iter().position(|p| matches!(p, Block::Table(t) if t.rows.len() == 2)).unwrap();
    assert!(matches!(&parts[at - 1], Block::Text(lines) if lines.len() == 1 && lines[0].is_empty()), "{:?}", parts[at - 1]);
    assert!(matches!(&parts[1], Block::Table(_)), "nothing between the first two: {:?}", parts[1]);
    // A gradient with no bgcolor: its first colour; one written after a colour: that colour.
    let b = html_layout(r#"<table width="600" style="background-image:linear-gradient(#123456, #fff)"><tr><td>x</td></tr></table>"#);
    let Block::Table(t) = &b[0] else { panic!() };
    assert_eq!(t.background, Some([0x12, 0x34, 0x56]));
    let b = html_layout(r#"<table width="600" style="background:#abcdef url(x.png) no-repeat"><tr><td>x</td></tr></table>"#);
    let Block::Table(t) = &b[0] else { panic!() };
    assert_eq!(t.background, Some([0xab, 0xcd, 0xef]));
    // A column of CSS (no table) keeps its padding and has no spacing.
    let b = html_layout(r#"<div style="max-width:600px;margin:0 auto;padding:18px 12px;background:#fff8cc">A column</div>"#);
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    assert_eq!((t.spacing, t.rows[0][0].padding), (None, Some([18, 12, 18, 12])));
}

/// A line break alone is room only between two boxes: a spacer cell of one stays a spacer (left
/// out), and one after a cell's last box adds nothing.
#[test]
fn a_line_break_alone_is_room_only_between_two_boxes() {
    let html = r#"<table width="600"><tr><td>Text</td><td><br></td></tr><tr><td><br></td><td><br></td></tr>
        <tr><td><table width="100"><tr><td>Box</td></tr></table><br></td><td>x</td></tr></table>"#;
    let b = html_layout(html);
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    assert_eq!(t.rows.len(), 2, "the row of line breaks is a spacer: {:?}", t.rows);
    assert_eq!(t.rows[0].len(), 2, "a cell of a line break holds its column's place");
    assert!(t.rows[0][1].blocks.is_empty(), "{:?}", t.rows[0][1]);
    assert_eq!(t.rows[1][0].blocks.len(), 1, "nothing after the box: {:?}", t.rows[1][0].blocks);
}

/// A cell that says its text isn't to wrap (`nowrap`, as Outlook writes a report's cells, or
/// `white-space: nowrap`) keeps that, so its column can be as wide as its line.
#[test]
fn a_cell_that_says_not_to_wrap_keeps_it() {
    let b = html_layout(
        "<table><tr><td nowrap=\"\">Rapport de garde</td><td style=\"white-space: nowrap\">B</td><td style=\"white-space:normal\">C</td><td>D</td></tr></table>",
    );
    let Some(Block::Table(t)) = b.first() else { panic!("a table: {b:?}") };
    assert_eq!(t.rows[0].iter().map(|c| c.nowrap).collect::<Vec<_>>(), [true, true, false, false]);
}
