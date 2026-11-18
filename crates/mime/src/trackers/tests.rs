// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
use super::*;

#[test]
fn links_lose_their_tags_and_wrappers() {
    for (link, clean) in [
        ("https://shop.example/sale?utm_source=news&utm_medium=email&id=7#top", "https://shop.example/sale?id=7#top"),
        ("https://shop.example/?fbclid=abc", "https://shop.example/"),
        ("https://Shop.Example:8443/a?MC_EID=1&q=x+y", "https://Shop.Example:8443/a?q=x+y"),
        ("https://www.google.com/url?q=https://site.example/page&sa=D&ust=1", "https://site.example/page"),
        ("https://abcd.r.us-east-1.awstrack.me/L0/https:%2F%2Fapp.example%2Fwelcome/1/0100-x/abc=", "https://app.example/welcome"),
        ("https://l.facebook.com/l.php?u=https%3A%2F%2Fevent.example%2F&h=AT0", "https://event.example/"),
        // Nothing to take out, or not a web address: as it was.
        ("https://site.example/a?b=c", "https://site.example/a?b=c"),
        ("mailto:ana@x.example?subject=Hi", "mailto:ana@x.example?subject=Hi"),
        // A wrapper whose parameter isn't a web address stays wrapped.
        ("https://www.google.com/url?q=javascript:alert(1)", "https://www.google.com/url?q=javascript:alert(1)"),
    ] {
        assert_eq!(clean_link(link), clean, "{link}");
    }
}

/// A wrapper that checks the address when it's clicked (Microsoft's Safe Links, Proofpoint's
/// URL Defense) is the organisation's own protection: the link opens through it. Where it
/// goes is still told, for showing.
#[test]
fn links_checked_when_clicked_open_through_their_check() {
    let safe =
        "https://eur01.safelinks.protection.outlook.com/?url=https%3A%2F%2Fdocs.example%2Fplan%3Fv%3D2%26utm_campaign%3Dq3&data=05%7C01&reserved=0";
    let defended = "https://urldefense.com/v3/__https://portal.example/login__;!!abc$";
    assert_eq!(clean_link(safe), safe);
    assert_eq!(clean_link(defended), defended);
    assert_eq!(link_destination(safe), "https://docs.example/plan?v=2");
    assert_eq!(link_destination(defended), "https://portal.example/login");
    // A redirect that only counts the click, around one that checks: taken off down to the check.
    let counted = format!("https://www.google.com/url?q={}", safe.replace('%', "%25").replace('&', "%26"));
    assert_eq!(clean_link(&counted), safe);
    assert_eq!(link_destination(&counted), "https://docs.example/plan?v=2");
    // Both still tell their sender the link was followed.
    assert!(tracks_click(safe) && tracks_click(defended));
    // Elsewhere the two say the same.
    assert_eq!(link_destination("https://shop.example/?fbclid=abc"), "https://shop.example/");
}

#[test]
fn trackers_are_counted() {
    let html = r#"<p><a href="https://news.us1.list-manage.com/track/click?u=1&id=2">Read</a>
        <a href="https://shop.example/?utm_source=x">Shop</a>
        <a href="https://shop.example/about">About</a>
        <img src="https://shop.example/logo.png" width="120" alt="Shop">
        <img src="https://open.shop.example/o.gif?r=9" width="1" height="1">
        <img src="https://news.us1.list-manage.com/track/open.php?u=1" alt="">
        <img src="https://t.example/p.png" style="display: none">
        <img src="https://img.shop.example/phone.png" alt="phone" width="13" style="border-width: 0px; display: block">
        <img src="https://t.example/q.png" style="height: 1px !important; width: 1px !important">
        <img src="https://t.example/r.png" style="WIDTH:0"></p>"#;
    let t = trackers(html);
    assert_eq!(t.links, 2);
    // A signature's icon without a border isn't one (`border-width: 0px` was read as `width:0`):
    // shown when the message's pictures are.
    assert_eq!(
        t.pictures,
        [
            "https://open.shop.example/o.gif?r=9",
            "https://news.us1.list-manage.com/track/open.php?u=1",
            "https://t.example/p.png",
            "https://t.example/q.png",
            "https://t.example/r.png"
        ]
    );
    assert!(trackers("<p>plain</p>").is_empty());
}

/// Styles that hide a picture or make it a pixel, as a browser reads them: what the first rule
/// (looking for `width:1px`, `width:0`… in the text) caught stays caught — when in doubt, a
/// tracker.
const PIXEL_STYLES: &[&str] = &[
    "width:1px",
    "height:1px",
    "max-width:1px",
    "max-height:1px",
    "max-width:0px",
    "width:1px !important",
    "WIDTH:1PX",
    "width : 1px",
    "width:0",
    "width:0.1em",
    "height:0",
    "width:1pt",
    "width:1",
    "width:2px",
    "width:\n1px",
    "display:none",
    "display:none!important",
    "DISPLAY: NONE",
    "height:1px !important;width:1px !important;border-width:0 !important;margin:0 !important",
    // `!` and `important` apart, an `!important` before a plain one, a zero in any unit,
    // a comment: all valid CSS, taken for pictures once the text was read as declarations.
    "width: 1px ! important; height: 1px ! important",
    "display: none ! important",
    "width:0%",
    "width:0vw",
    "width:1px !important; width:300px",
    "width:1px/**/",
    "width:1px; width:auto",
    "width:1px; width:300px",
    "display:none; display:block",
    // A `/*` in an address isn't a comment (the rest of the style went with it); one after
    // it is.
    "background:url(https://x.example/a/*.png); width:1px",
    "background:url('https://x.example/a/*.png'); width:1px",
    "background:url(https://x.example/a.png) /* plain */; width:1px",
    // Hidden, not sized.
    "visibility:hidden",
    "visibility: hidden !important",
    "opacity:0",
    "opacity: 0.0",
];

/// Styles that make no picture small (an icon's `border-width: 0`).
const PICTURE_STYLES: &[&str] = &[
    "min-width:1px",
    "min-height:1px",
    "line-height:1px",
    "border-width:0",
    "border-width: 0px; display: block",
    "width:13px",
    "width:3px;height:3px",
    "opacity:0.6",
    "visibility:visible",
    "width:50%",
    "background:url(https://x.example/a/*.png); width:13px",
];

/// The styles of [`PIXEL_STYLES`] and [`PICTURE_STYLES`] that `tracker` (whether a picture of
/// that style is taken for a tracker) reads wrong, each with what it should have said.
fn read_wrong(tracker: impl Fn(&str) -> bool) -> Vec<(&'static str, bool)> {
    PIXEL_STYLES
        .iter()
        .map(|s| (*s, true))
        .chain(PICTURE_STYLES.iter().map(|s| (*s, false)))
        .filter(|(s, expected)| tracker(s) != *expected)
        .collect()
}

#[test]
fn pixels_by_their_style() {
    let wrong = read_wrong(|style| !trackers(&format!("<p><img src=\"https://img.example/p.png\" style=\"{style}\"></p>")).pictures.is_empty());
    assert!(wrong.is_empty(), "read wrong (style, a tracker): {wrong:?}");
}

/// The same styles in a whole message whose style sheet touches its pictures (`img{border:0}`,
/// as most newsletters' does): written on the picture, the sheet keeps one value of each
/// property and drops the tag's `!important`, which loaded `width:1px !important;
/// width:300px` again.
#[test]
fn pixels_by_their_style_under_a_style_sheet() {
    let wrong = read_wrong(|style| {
        let raw = format!(
            "From: a@shop.example\r\nTo: b@studio.example\r\nSubject: s\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
             <html><head><style>img{{border:0}}</style></head><body><p>Hello</p>\
             <img src=\"https://img.example/p.png\" style=\"{style}\"></body></html>"
        );
        crate::parse(raw.as_bytes()).is_some_and(|p| !p.trackers.pictures.is_empty())
    });
    assert!(wrong.is_empty(), "read wrong in a message (style, a tracker): {wrong:?}");
}

/// A pixel sized or hidden by a class of the message's style sheet is one too: the trackers are
/// read once the page's styles are written on its elements.
#[test]
fn pixels_by_a_class() {
    let raw = "From: a@shop.example\r\nTo: b@studio.example\r\nSubject: s\r\nMIME-Version: 1.0\r\n\
               Content-Type: text/html; charset=utf-8\r\n\r\n\
               <html><head><style>.px{width:1px;height:1px} .ghost{visibility:hidden} .clear{opacity:0}</style></head>\
               <body><p>Hello</p><img class=\"px\" src=\"https://t.example/a.png\"><img class=\"ghost\" src=\"https://t.example/b.png\">\
               <img class=\"clear\" src=\"https://t.example/c.png\"><img src=\"https://img.example/logo.png\" width=\"120\" alt=\"Logo\"></body></html>";
    let p = crate::parse(raw.as_bytes()).unwrap();
    assert_eq!(p.trackers.pictures, ["https://t.example/a.png", "https://t.example/b.png", "https://t.example/c.png"]);
}

/// A whole message with `body` under `sheet`, as parsed.
fn message(sheet: &str, body: &str) -> crate::Parsed {
    let raw = format!(
        "From: a@shop.example\r\nTo: b@studio.example\r\nSubject: s\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
         <html><head>{sheet}</head><body><p>Hello</p>{body}</body></html>"
    );
    crate::parse(raw.as_bytes()).unwrap()
}

/// The web pictures of `p` asked for once its pictures are shown (as the session asks them).
fn loaded(p: &crate::Parsed) -> Vec<String> {
    p.images.iter().filter(|u| u.starts_with("https://") && !p.trackers.pictures.contains(u)).cloned().collect()
}

/// An address written with a character reference (`&#38;`, `&#x26;`, `&quest;` and `&amp;`) is
/// one address, as a browser reads it, in the pictures and in the trackers alike: a pixel was
/// listed twice (as written, and as decoded once the style sheet was written on it), which said
/// two pixels for one.
#[test]
fn a_pixel_written_with_character_references_is_listed_once() {
    let written = ["?a=1&#38;b=2", "?a=1&#x26;b=3", "?a=1&amp;b=4", "&quest;a=1&#38;b=5"];
    let decoded = ["?a=1&b=2", "?a=1&b=3", "?a=1&b=4", "?a=1&b=5"];
    let pixels: String = written.iter().map(|q| format!("<img src=\"https://t.example/o.gif{q}\" width=\"1\" height=\"1\">")).collect();
    let listed: Vec<String> = decoded.iter().map(|q| format!("https://t.example/o.gif{q}")).collect();
    for sheet in ["", "<style>img{border:0}</style>"] {
        let p = message(sheet, &pixels);
        assert_eq!((&p.trackers.pictures, loaded(&p)), (&listed, Vec::new()), "each once, as decoded, none asked for, under {sheet:?}");
    }
}

/// … which used up the 50 kept: some of 49 such pixels were asked for.
#[test]
fn forty_nine_pixels_written_with_references_are_all_held_back() {
    let many: String = (0..49).map(|n| format!("<img src=\"https://t.example/o.gif?n={n}&#38;r=1\" width=\"1\" height=\"1\">")).collect();
    let p = message("<style>img{border:0}</style>", &many);
    assert_eq!((p.trackers.pictures.len(), loaded(&p).len()), (49, 0), "listed, asked for");
}

/// … and asked for the one sized `width:1px !important; width:300px` under a style sheet.
#[test]
fn a_pixel_sized_twice_written_with_a_reference_is_held_back() {
    let n1 = "<img src=\"https://t.example/o.gif?a=1&#38;b=2\" style=\"width:1px !important; width:300px\">";
    let p = message("<style>img{border:0}</style>", n1);
    assert_eq!((p.trackers.pictures.clone(), loaded(&p)), (vec!["https://t.example/o.gif?a=1&b=2".to_string()], Vec::new()));
}

/// The tokenizer's reads of addresses while `f` runs on this thread.
fn tokenized(f: impl FnOnce()) -> usize {
    let before = crate::layout::TOKENIZED.with(std::cell::Cell::get);
    f();
    crate::layout::TOKENIZED.with(std::cell::Cell::get) - before
}

/// A stranger's thousands of pictures cost no more than a few reads of their addresses, in the
/// three readings of a message (its pictures, its trackers as it came and once its style sheet
/// is on its elements): an address with `&amp;` alone is read as it is, the same address once,
/// a message's no more than [`MAX_DECODED`](crate::html::pictures::MAX_DECODED). Each `&` cost a
/// tokenizer of its own, thrice: a message of 80,000 pictures took two and a half times as
/// long to read.
#[test]
fn many_pictures_cost_a_few_reads_of_their_addresses() {
    let tags = |src: &dyn Fn(usize) -> String| (0..5_000).map(|n| format!("<img src=\"{}\" width=\"120\">", src(n))).collect::<String>();
    // (what, the reads, the most allowed)
    let mut reads: Vec<(String, usize, usize)> = Vec::new();
    for sheet in ["", "<style>img{border:0}</style>"] {
        let amp = tags(&|n| format!("https://img.example/{n}.png?x=1&amp;y=2"));
        reads.push((format!("`&amp;` alone, under {sheet:?}"), tokenized(|| drop(message(sheet, &amp))), 0));
        let same = tags(&|_| "https://img.example/a.png?x=1&#38;y=2".to_string());
        reads.push((format!("the same address, under {sheet:?}"), tokenized(|| drop(message(sheet, &same))), 1));
        let distinct = tags(&|n| format!("https://img.example/{n}.png?x=1&#38;y=2"));
        reads.push((format!("5,000 addresses, under {sheet:?}"), tokenized(|| drop(message(sheet, &distinct))), crate::html::pictures::MAX_DECODED));
    }
    // The trackers are the pictures listed that report back (each tag of a picture listed is
    // asked, so every address is read): no more reads than a message's.
    let pixels: String = (0..5_000).map(|n| format!("<img src=\"https://t.example/o.gif?n={n}&#38;r=1\" width=\"1\" height=\"1\">")).collect();
    let mut t = Trackers::default();
    reads.push(("5,000 pixels".into(), tokenized(|| t = trackers(&pixels)), crate::html::pictures::MAX_DECODED));
    assert_eq!(t.pictures.len(), crate::html::pictures::MAX_PICTURES);
    let over: Vec<_> = reads.iter().filter(|(_, n, most)| n > most).collect();
    assert!(over.is_empty(), "read too often (what, reads, most): {over:?}");
}

/// Found in review: a `>` in a quoted value ended the tag where the pictures and the trackers
/// were read (`alt=">"`), before the pixel's size, and the pixel was asked for with the
/// message's pictures. Read as a browser reads the tag, a quoted value that reads like a size
/// (`alt=' width=500'`) isn't one either, and a size written with a character reference
/// (`width&#58;1px`) is the size it says.
#[test]
fn a_pixel_is_one_whatever_its_values_hold() {
    let o = "https://t.example/o.gif";
    for tag in [
        format!(r#"<img src="{o}" alt=">" width="1" height="1">"#),
        format!(r#"<img src="{o}" title='>' width="1" height="1">"#),
        format!(r#"<img src="{o}" alt=">" style="width:1px">"#),
        format!(r#"<img src="{o}" alt=' width=500 height=500 ' width=1 height=1>"#),
        format!(r#"<img src="{o}" style="width&#58;1px;height&#58;1px">"#),
    ] {
        assert_eq!(trackers(&tag).pictures, [o], "{tag}");
        for sheet in ["", "<style>img{border:0}</style>"] {
            let p = message(sheet, &tag);
            assert_eq!((p.trackers.pictures.clone(), loaded(&p)), (vec![o.to_string()], Vec::<String>::new()), "{tag} under {sheet:?}");
        }
    }
}

/// Found in review: the trackers were listed up to 50 and the pictures up to 200: 50 pixels of
/// any address first, and the next, the real one, was asked for. Each picture listed is asked
/// whether it's a tracking picture.
#[test]
fn fifty_decoys_let_no_pixel_through() {
    let decoys: String = (0..50).map(|n| format!("<img src=\"https://d.example/{n}.gif\" width=\"1\" height=\"1\">")).collect();
    let real = "<img src=\"https://t.example/o.gif\" width=\"1\" height=\"1\">";
    for sheet in ["", "<style>img{border:0}</style>"] {
        let p = message(sheet, &format!("{decoys}{real}"));
        assert!(p.trackers.pictures.iter().any(|s| s == "https://t.example/o.gif"), "listed, under {sheet:?}");
        assert_eq!(loaded(&p), Vec::<String>::new(), "none asked for, under {sheet:?}");
    }
    // As many as the pictures listed: each held back, none past them asked for.
    let many: String = (0..300).map(|n| format!("<img src=\"https://d.example/{n}.gif\" width=\"1\" height=\"1\">")).collect();
    let p = message("", &many);
    assert_eq!((p.images.len(), p.trackers.pictures.len(), loaded(&p).len()), (200, 200, 0), "listed, held back, asked for");
}

/// Found in review: a picture behind a box was never asked whether it reports back: a counting
/// service's (the surest sign) loaded with the message's pictures as a box's background, and
/// so did one behind a box 1 pixel wide.
#[test]
fn a_tracking_picture_behind_a_box_is_one() {
    let counter = "https://x.list-manage.com/track/open.php?u=1";
    let pixel = "https://t.example/o.gif";
    for (body, src) in [
        (format!(r#"<div style="background-image:url({counter});width:1px;height:1px"></div>"#), counter),
        (format!(r#"<table><tr><td background="{counter}">Hello</td><td>there</td></tr></table>"#), counter),
        (format!(r#"<div style="background-image:url({pixel});width:2px;height:300px">x</div>"#), pixel),
        (format!(r#"<table><tr><td style="background:url('{pixel}')" height="1">x</td><td>y</td></tr></table>"#), pixel),
    ] {
        let p = message("", &body);
        assert!(p.images.iter().any(|s| s == src), "a picture of the message: {body}");
        assert_eq!((p.trackers.pictures.clone(), loaded(&p)), (vec![src.to_string()], Vec::<String>::new()), "{body}");
    }
    // A banner behind a heading is shown with the others.
    let p = message("", r#"<div style="background-image:url(https://img.example/banner.png);width:600px"><h1>News</h1></div>"#);
    assert_eq!(loaded(&p), ["https://img.example/banner.png"]);
}

/// Found in review: a picture in a comment (or a script, a `<textarea>`…) was asked for when the
/// message's pictures were shown, which no browser does. The pictures are found as a browser
/// reads the tags.
#[test]
fn pictures_no_browser_loads_are_never_asked_for() {
    let o = r#"<img src="https://t.example/o.gif">"#;
    for html in [
        format!("<!-- {o} --><p>x</p>"),
        format!("<!-- a -- {o} --!><p>x</p>"),
        format!("<script>var s = '{o}';</script>"),
        format!("<textarea>{o}</textarea>"),
        format!("<style>/* {o} */</style>"),
        format!("<title>{o}</title>"),
        format!("<template><p>{o}</p></template>"),
        format!("<p title='{o}'>x</p>"),
        format!("<!doctype {o}"),
        format!("<p class='{o}"),
    ] {
        assert_eq!(crate::html_images(&html), Vec::<String>::new(), "{html}");
        assert_eq!(loaded(&message("", &html)), Vec::<String>::new(), "{html}");
    }
    // As a browser reads them: after an empty comment, past a `>` in a quoted value, after a
    // script's end, `<image>`, `<img/src=…>`.
    assert_eq!(
        crate::html_images(r#"<!--><img src=a.png><p alt='>'><img src=b.png><script>x</script><IMAGE SRC=c.png><img/src=d.png>"#),
        ["a.png", "b.png", "c.png", "d.png"]
    );
}

/// Found in review: past the 512 values the tokenizer reads for a message, an address written
/// with a reference (`&#38;`) wasn't read in the message as it came, matched none of the pictures
/// listed, and its tag wasn't asked: a pixel sized `width:1px !important; width:300px` under a
/// style sheet was asked for. A picture whose address can't be read any more is taken for a
/// tracking picture.
#[test]
fn a_pixel_past_the_values_read_is_held_back() {
    let decoys: String = (0..512).map(|n| format!("<img src=\"https://img.example/d.png\" style=\"color:&#35;{n:06}\">")).collect();
    let pixel = "<img src=\"https://t.example/o.gif?a=1&#38;b=2\" style=\"width:1px !important; width:300px\">";
    for sheet in ["", "<style>img{border:0}</style>"] {
        let p = message(sheet, &format!("{decoys}{pixel}"));
        assert!(!loaded(&p).iter().any(|s| s.starts_with("https://t.example/")), "under {sheet:?}: {:?}", loaded(&p));
    }
}

/// Found in review: a size a browser reads by its leading digits (`1x`, `1em`) wasn't read, and
/// the pixel was asked for. A share of the space (`1%`) isn't a size in pixels.
#[test]
fn a_size_is_read_by_its_leading_digits() {
    let o = "https://t.example/o.gif";
    for size in [r#"width="1x" height="1x""#, r#"width="1em""#, r#"height="1.5""#, r#"width=" 2 px""#, r#"height="0pt""#] {
        let tag = format!(r#"<img src="{o}" {size}>"#);
        assert_eq!(trackers(&tag).pictures, [o], "{tag}");
        assert_eq!(loaded(&message("", &tag)), Vec::<String>::new(), "{tag}");
    }
    for size in [r#"width="1%""#, r#"width="13x""#, r#"width="x1""#] {
        let tag = format!(r#"<img src="https://img.example/a.png" {size}>"#);
        assert!(trackers(&tag).pictures.is_empty(), "{tag}");
    }
}

/// Mail as Outlook and others write it loses nothing to the reading of its tags: its conditional
/// comments, `<o:p>`, a `<noscript>` with a picture (read without scripts, as the text
/// converter reads it), and a signature whose pictures aren't loaded keep their pictures, their
/// sizes and their titles.
#[test]
fn ordinary_mail_keeps_its_pictures_and_titles() {
    let body = r#"<!--[if gte mso 9]><xml><o:OfficeDocumentSettings><o:AllowPNG/></o:OfficeDocumentSettings></xml><![endif]-->
<h1>Quarterly news</h1>
<!--[if !mso]><!--><img src="https://img.example/logo.png" alt="Contoso"><!--<![endif]-->
<![if !vml]><img src="https://img.example/vml.png" width="300" alt="Banner"><![endif]>
<p class=MsoNormal>Hello<o:p>&nbsp;</o:p></p>
<p class=MsoNormal><b><span style='font-size:20.0pt;font-family:"Segoe UI"'>Big title</span></b><o:p></o:p></p>
<noscript><img src="https://img.example/ns.png" alt="Without scripts"></noscript>
<table><tr><td><img src="https://img.example/sig.png" width="120" height="40" alt="Ana Lima"></td><td><b>Ana Lima</b><br>Contoso</td></tr></table>"#;
    let shown: Vec<String> = ["logo", "vml", "ns", "sig"].iter().map(|f| format!("https://img.example/{f}.png")).collect();
    for sheet in ["", "<style>p.MsoNormal{margin:0}</style>"] {
        let p = message(sheet, body);
        assert_eq!((&p.images, &p.trackers.pictures, loaded(&p)), (&shown, &Vec::new(), shown.clone()), "under {sheet:?}");
        let spans: Vec<&crate::Span> = p.rich.iter().flatten().collect();
        for src in &shown {
            assert!(spans.iter().any(|s| s.image.as_ref() == Some(src)), "{src} has its place, under {sheet:?}: {spans:?}");
        }
        let sig = spans.iter().find(|s| s.image.as_deref() == Some("https://img.example/sig.png"));
        assert_eq!(sig.and_then(|s| s.image_size).and_then(|s| s.width), Some(crate::layout::Width::Px(120)), "under {sheet:?}");
        // A heading, and a title Word sized (its size and weight kept).
        assert!(spans.iter().any(|s| s.heading == 1 && s.text.contains("Quarterly news")), "under {sheet:?}: {spans:?}");
        assert!(spans.iter().any(|s| s.bold && s.size == Some(20) && s.text.contains("Big title")), "under {sheet:?}: {spans:?}");
        assert!(!crate::html::too_deep(body));
    }
}

#[test]
fn odd_links_never_panic() {
    for link in [
        "",
        "https://",
        "https://%",
        "https://x.example/?%zz=1&&=&utm_",
        "https://urldefense.com/v3/__",
        "http://[::1]:80/?a",
        "https://é.example/%E2%82",
    ] {
        let _ = clean_link(link);
        let _ = link_destination(link);
        let _ = tracks_click(link);
    }
    assert_eq!(clean_link("http://[::1]:8080/a?utm_x=1&b=2"), "http://[::1]:8080/a?b=2");
    let deep = (0..10).fold("https://site.example/".to_string(), |u, _| format!("https://www.google.com/url?q={}", u.replace('&', "%26")));
    let _ = clean_link(&deep);
}
