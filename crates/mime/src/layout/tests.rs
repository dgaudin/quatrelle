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
    let ruled = row[1].blocks.iter().any(|b| matches!(b, Block::Table(t) if t.border_left == Some([0xcc, 0xcc, 0xcc])));
    assert!(ruled, "the rule between them: {:?}", row[1].blocks);
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

#[test]
fn plain_mail_and_one_column_tables_have_no_layout() {
    assert!(html_layout("<p>Hello <b>there</b></p>").is_empty());
    assert!(html_layout("<table><tr><td>One</td></tr><tr><td>Two</td></tr></table>").is_empty());
    // A receipt (a data table) has columns.
    assert!(!html_layout("<table><tr><th>Item</th><th>Price</th></tr><tr><td>Poster</td><td>40</td></tr></table>").is_empty());
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
