//! The interface icons: Microsoft's Fluent UI System Icons (MIT, `assets/icons/fluent`, the
//! owner's choice so the window looks like Outlook's), rendered from their SVG once per size and
//! tinted when drawn. Each also has a line drawing in code on a 16-unit grid, used should its
//! picture fail to render.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use egui::{Color32, ColorImage, Context, Id, Painter, Pos2, Rect, Stroke, TextureHandle, TextureOptions, Vec2, pos2};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    NewMail,
    Delete,
    Archive,
    Junk,
    Reply,
    ReplyAll,
    Forward,
    Move,
    Read,
    Unread,
    Flag,
    Sync,
    Inbox,
    Sent,
    Drafts,
    Folder,
    Search,
    Attach,
    Gear,
    Sun,
    Moon,
    Close,
    ChevronDown,
    ChevronUp,
    ChevronRight,
    ChevronLeft,
    Star,
    Image,
    File,
    Send,
    Plus,
    Layout,
    Person,
    Warning,
    Outbox,
    Pin,
    Clock,
    Tag,
    Print,
    Back,
    /// Undo the last change (the quick access bar).
    Undo,
    Info,
    SpellCheck,
    Reminder,
    Away,
    Storage,
    Funnel,
    Calendar,
    /// The calendar's views and today.
    CalendarDay,
    CalendarWeek,
    CalendarWorkWeek,
    CalendarMonth,
    CalendarToday,
    CalendarAdd,
    /// Voting buttons.
    Vote,
    People,
    Phone,
    Location,
    /// A window with an arrow leaving it (pop out), and the reverse (pop in).
    PopOut,
    PopIn,
    /// The message editor's ribbon.
    ListBullet,
    ListNumber,
    AlignLeft,
    AlignCenter,
    AlignRight,
    IndentMore,
    IndentLess,
    Link,
    ClearFormat,
    AddressBook,
    /// A sticky note (Notes).
    Note,
    /// Two chevrons running ahead (Quick Steps: several things in one go).
    Steps,
    /// The language the spelling is checked in (Review › Language).
    Language,
    /// The message window's ribbon: the clipboard, the signature, importance, a rule, the
    /// font's size, the word count, delay delivery, Bcc, and names checked.
    Paste,
    Cut,
    Copy,
    Signature,
    Important,
    LowImportance,
    Rule,
    FontGrow,
    FontShrink,
    WordCount,
    DelayDelivery,
    Bcc,
    CheckNames,
    /// A padlock (encrypted mail).
    Lock,
    /// A rosette with a tick (signed mail).
    Seal,
}

struct Pen<'a> {
    p: &'a Painter,
    r: Rect,
    s: Stroke,
}

impl Pen<'_> {
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.r.min.x + x / 16.0 * self.r.width(), self.r.min.y + y / 16.0 * self.r.height())
    }
    fn line(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.p.add(egui::Shape::line(v, self.s));
    }
    fn closed(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.p.add(egui::Shape::closed_line(v, self.s));
    }
    fn filled(&self, pts: &[(f32, f32)], c: Color32) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.p.add(egui::Shape::convex_polygon(v, c, Stroke::NONE));
    }
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        self.closed(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
    }
    fn circle(&self, x: f32, y: f32, rad: f32) {
        self.p.circle_stroke(self.pt(x, y), rad / 16.0 * self.r.width(), self.s);
    }
    fn dot(&self, x: f32, y: f32, rad: f32) {
        self.p.circle_filled(self.pt(x, y), rad / 16.0 * self.r.width(), self.s.color);
    }
    fn arc(&self, cx: f32, cy: f32, rad: f32, a0: f32, a1: f32) {
        let n = 14;
        let pts: Vec<(f32, f32)> = (0..=n)
            .map(|i| {
                let a = a0 + (a1 - a0) * i as f32 / n as f32;
                (cx + rad * a.cos(), cy + rad * a.sin())
            })
            .collect();
        self.line(&pts);
    }
}

/// The Fluent UI icon drawn for each icon (its 20-pixel regular outline).
fn svg(icon: Icon) -> &'static str {
    macro_rules! f {
        ($name:literal) => {
            include_str!(concat!("../../../../assets/icons/fluent/", $name, "_20_regular.svg"))
        };
    }
    match icon {
        Icon::NewMail => f!("mail_add"),
        Icon::Delete => f!("delete"),
        Icon::Archive => f!("archive"),
        Icon::Junk => f!("mail_prohibited"),
        Icon::Steps => f!("flash"),
        Icon::Reply => f!("arrow_reply"),
        Icon::ReplyAll => f!("arrow_reply_all"),
        Icon::Forward => f!("arrow_forward"),
        Icon::Move => f!("folder_arrow_right"),
        Icon::Read => f!("mail_read"),
        Icon::Unread => f!("mail"),
        Icon::Flag => f!("flag"),
        Icon::Sync => f!("arrow_sync"),
        Icon::Inbox => f!("mail_inbox"),
        Icon::Sent => f!("send"),
        Icon::Drafts => f!("drafts"),
        Icon::Folder => f!("folder"),
        Icon::Search => f!("search"),
        Icon::Attach => f!("attach"),
        Icon::Gear => f!("settings"),
        Icon::Sun => f!("weather_sunny"),
        Icon::Moon => f!("weather_moon"),
        Icon::Close => f!("dismiss"),
        Icon::ChevronDown => f!("chevron_down"),
        Icon::ChevronUp => f!("chevron_up"),
        Icon::ChevronRight => f!("chevron_right"),
        Icon::ChevronLeft => f!("chevron_left"),
        Icon::Star => f!("star"),
        Icon::Image => f!("image"),
        Icon::File => f!("document"),
        Icon::Send => f!("send"),
        Icon::Plus => f!("add"),
        Icon::Layout => f!("layout_column_two"),
        Icon::Person => f!("person"),
        Icon::Warning => f!("warning"),
        Icon::Outbox => f!("mail_inbox_arrow_up"),
        Icon::Pin => f!("pin"),
        Icon::Clock => f!("clock"),
        Icon::Tag => f!("tag"),
        Icon::Print => f!("print"),
        Icon::Back => f!("arrow_left"),
        Icon::Undo => f!("arrow_undo"),
        Icon::Info => f!("info"),
        Icon::SpellCheck => f!("text_grammar_checkmark"),
        Icon::Reminder => f!("alert"),
        Icon::Away => f!("person_clock"),
        Icon::Storage => f!("storage"),
        Icon::Funnel => f!("filter"),
        Icon::Calendar => f!("calendar_ltr"),
        Icon::CalendarDay => f!("calendar_day"),
        Icon::CalendarWeek => f!("calendar_week_start"),
        Icon::CalendarWorkWeek => f!("calendar_work_week"),
        Icon::CalendarMonth => f!("calendar_month"),
        Icon::CalendarToday => f!("calendar_today"),
        Icon::CalendarAdd => f!("calendar_add"),
        Icon::Vote => f!("vote"),
        Icon::People => f!("people"),
        Icon::Phone => f!("call"),
        Icon::Location => f!("location"),
        Icon::PopOut => f!("open"),
        Icon::PopIn => f!("arrow_enter"),
        Icon::ListBullet => f!("text_bullet_list_ltr"),
        Icon::ListNumber => f!("text_number_list_ltr"),
        Icon::AlignLeft => f!("text_align_left"),
        Icon::AlignCenter => f!("text_align_center"),
        Icon::AlignRight => f!("text_align_right"),
        Icon::IndentMore => f!("text_indent_increase_ltr"),
        Icon::IndentLess => f!("text_indent_decrease_ltr"),
        Icon::Link => f!("link"),
        Icon::ClearFormat => f!("text_clear_formatting"),
        Icon::AddressBook => f!("book_contacts"),
        Icon::Note => f!("note"),
        Icon::Language => f!("local_language"),
        Icon::Paste => f!("clipboard_paste"),
        Icon::Cut => f!("cut"),
        Icon::Copy => f!("copy"),
        Icon::Signature => f!("signature"),
        Icon::Important => f!("important"),
        Icon::LowImportance => f!("arrow_down"),
        Icon::Rule => f!("line_horizontal_1"),
        Icon::FontGrow => f!("font_increase"),
        Icon::FontShrink => f!("font_decrease"),
        Icon::WordCount => f!("text_word_count"),
        Icon::DelayDelivery => f!("calendar_clock"),
        Icon::Bcc => f!("eye"),
        Icon::CheckNames => f!("person_available"),
        Icon::Lock => f!("lock_closed"),
        Icon::Seal => f!("ribbon"),
    }
}

/// Rendered icons by (icon, size in pixels): white on transparent, tinted when drawn; `None` for
/// one that wouldn't render.
#[derive(Clone, Default)]
struct Rendered(Arc<Mutex<HashMap<(Icon, u32), Option<TextureHandle>>>>);

fn texture(ctx: &Context, icon: Icon, px: u32) -> Option<TextureHandle> {
    let cache = ctx.data_mut(|d| d.get_temp_mut_or_default::<Rendered>(Id::unique("quatrelle-icons")).clone());
    let mut map = cache.0.lock().ok()?;
    if let Some(t) = map.get(&(icon, px)) {
        return t.clone();
    }
    // Every size used stays bounded (zooming through every size starts again).
    if map.len() > 600 {
        map.clear();
    }
    let t = render(icon, px).map(|img| ctx.load_texture(format!("icon-{icon:?}-{px}"), img, TextureOptions::LINEAR));
    map.insert((icon, px), t.clone());
    t
}

/// An icon's SVG at `px` pixels square, in white.
fn render(icon: Icon, px: u32) -> Option<ColorImage> {
    let source = svg(icon).replacen("<svg ", "<svg fill=\"#ffffff\" ", 1);
    let tree = resvg::usvg::Tree::from_str(&source, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px)?;
    let scale = px as f32 / tree.size().width().max(1.0);
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let side = usize::try_from(px).ok()?;
    Some(ColorImage::from_rgba_premultiplied([side, side], pixmap.data()))
}

/// Paint `icon` into `rect` (square works best) in `color`.
pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let px = (rect.width().max(rect.height()) * p.ctx().pixels_per_point()).round().clamp(4.0, 256.0) as u32;
    match texture(p.ctx(), icon, px) {
        Some(t) => {
            p.image(t.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), color);
        }
        None => drawn(p, rect, icon, color),
    }
}

/// The icon's line drawing.
fn drawn(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let w = (rect.width() / 16.0 * 1.35).clamp(1.0, 2.6);
    let pen = Pen { p, r: rect, s: Stroke::new(w, color) };
    use std::f32::consts::PI;
    match icon {
        Icon::NewMail => {
            pen.line(&[(9.0, 4.0), (2.0, 4.0), (2.0, 13.0), (14.0, 13.0), (14.0, 9.0)]);
            pen.line(&[(2.0, 4.5), (8.0, 9.0), (10.0, 7.5)]);
            pen.line(&[(13.0, 1.5), (13.0, 6.5)]);
            pen.line(&[(10.5, 4.0), (15.5, 4.0)]);
        }
        Icon::Unread => {
            pen.rect(2.0, 4.0, 14.0, 13.0);
            pen.line(&[(2.0, 4.5), (8.0, 9.0), (14.0, 4.5)]);
        }
        Icon::Read => {
            pen.closed(&[(2.0, 7.0), (8.0, 3.0), (14.0, 7.0), (14.0, 14.0), (2.0, 14.0)]);
            pen.line(&[(2.0, 7.0), (8.0, 11.0), (14.0, 7.0)]);
        }
        Icon::Delete => {
            pen.line(&[(2.5, 4.0), (13.5, 4.0)]);
            pen.line(&[(6.0, 4.0), (6.5, 2.0), (9.5, 2.0), (10.0, 4.0)]);
            pen.closed(&[(3.8, 4.0), (4.8, 14.0), (11.2, 14.0), (12.2, 4.0)]);
            pen.line(&[(6.8, 6.5), (7.0, 11.5)]);
            pen.line(&[(9.2, 6.5), (9.0, 11.5)]);
        }
        Icon::Archive => {
            pen.rect(1.5, 2.5, 14.5, 5.5);
            pen.line(&[(2.5, 5.5), (2.5, 13.5), (13.5, 13.5), (13.5, 5.5)]);
            pen.line(&[(6.0, 8.0), (10.0, 8.0)]);
        }
        Icon::Junk => {
            pen.closed(&[(8.0, 1.5), (13.5, 3.5), (13.0, 9.0), (8.0, 14.5), (3.0, 9.0), (2.5, 3.5)]);
            pen.line(&[(5.5, 5.5), (10.5, 10.5)]);
            pen.line(&[(10.5, 5.5), (5.5, 10.5)]);
        }
        Icon::Reply => {
            pen.line(&[(6.0, 3.0), (2.0, 7.0), (6.0, 11.0)]);
            pen.line(&[(2.0, 7.0), (9.0, 7.0)]);
            pen.arc(9.0, 11.0, 4.5, -PI / 2.0, 0.0);
            pen.line(&[(13.5, 11.0), (13.5, 13.5)]);
        }
        Icon::ReplyAll => {
            pen.line(&[(5.0, 3.5), (1.5, 7.0), (5.0, 10.5)]);
            pen.line(&[(8.5, 3.0), (4.5, 7.0), (8.5, 11.0)]);
            pen.line(&[(4.5, 7.0), (10.0, 7.0)]);
            pen.arc(10.0, 11.0, 4.0, -PI / 2.0, 0.0);
            pen.line(&[(14.0, 11.0), (14.0, 13.5)]);
        }
        Icon::Forward => {
            pen.line(&[(10.0, 3.0), (14.0, 7.0), (10.0, 11.0)]);
            pen.line(&[(14.0, 7.0), (7.0, 7.0)]);
            pen.arc(7.0, 11.0, 4.5, -PI / 2.0, -PI);
            pen.line(&[(2.5, 11.0), (2.5, 13.5)]);
        }
        Icon::Move | Icon::Folder => {
            pen.closed(&[(1.5, 3.0), (6.0, 3.0), (7.5, 4.8), (14.5, 4.8), (14.5, 13.5), (1.5, 13.5)]);
            if icon == Icon::Move {
                pen.line(&[(5.0, 9.2), (11.0, 9.2)]);
                pen.line(&[(8.8, 7.0), (11.0, 9.2), (8.8, 11.4)]);
            }
        }
        Icon::Flag => {
            pen.line(&[(3.5, 1.5), (3.5, 15.0)]);
            pen.closed(&[(3.5, 2.5), (13.0, 2.5), (10.5, 5.8), (13.0, 9.0), (3.5, 9.0)]);
        }
        Icon::Sync => {
            pen.arc(8.0, 8.0, 5.5, -PI * 0.9, -PI * 0.05);
            pen.line(&[(13.5, 3.0), (13.5, 7.7), (9.0, 7.7)]);
            pen.arc(8.0, 8.0, 5.5, PI * 0.1, PI * 0.95);
            pen.line(&[(2.5, 13.0), (2.5, 8.3), (7.0, 8.3)]);
        }
        Icon::Inbox => {
            pen.closed(&[(1.5, 9.0), (4.0, 2.5), (12.0, 2.5), (14.5, 9.0), (14.5, 13.5), (1.5, 13.5)]);
            pen.line(&[(1.5, 9.0), (5.0, 9.0), (6.0, 11.0), (10.0, 11.0), (11.0, 9.0), (14.5, 9.0)]);
        }
        Icon::Sent | Icon::Send => {
            pen.closed(&[(1.5, 7.5), (14.5, 2.0), (10.5, 14.5), (7.5, 9.0)]);
            pen.line(&[(7.5, 9.0), (14.5, 2.0)]);
        }
        Icon::Drafts => {
            pen.closed(&[(10.5, 2.0), (14.0, 5.5), (6.0, 13.5), (2.0, 14.0), (2.5, 10.0)]);
            pen.line(&[(9.0, 3.5), (12.5, 7.0)]);
        }
        Icon::Search => {
            pen.circle(6.8, 6.8, 4.6);
            pen.line(&[(10.2, 10.2), (14.5, 14.5)]);
        }
        Icon::Attach => {
            pen.line(&[(11.5, 5.0), (11.5, 11.0)]);
            pen.arc(8.75, 11.0, 2.75, 0.0, PI);
            pen.line(&[(6.0, 11.0), (6.0, 4.0)]);
            pen.arc(7.75, 4.0, 1.75, PI, 2.0 * PI);
            pen.line(&[(9.5, 4.0), (9.5, 10.5)]);
        }
        Icon::Gear => {
            pen.circle(8.0, 8.0, 2.4);
            for i in 0..8 {
                let a = i as f32 * PI / 4.0;
                pen.line(&[(8.0 + 4.3 * a.cos(), 8.0 + 4.3 * a.sin()), (8.0 + 6.2 * a.cos(), 8.0 + 6.2 * a.sin())]);
            }
            pen.circle(8.0, 8.0, 4.4);
        }
        Icon::Sun => {
            pen.circle(8.0, 8.0, 3.2);
            for i in 0..8 {
                let a = i as f32 * PI / 4.0;
                pen.line(&[(8.0 + 5.0 * a.cos(), 8.0 + 5.0 * a.sin()), (8.0 + 6.8 * a.cos(), 8.0 + 6.8 * a.sin())]);
            }
        }
        Icon::Moon => {
            pen.arc(8.0, 8.0, 6.0, PI * 0.35, PI * 1.85);
            pen.arc(10.6, 5.6, 4.6, PI * 0.62, PI * 1.55);
        }
        Icon::Close => {
            pen.line(&[(3.5, 3.5), (12.5, 12.5)]);
            pen.line(&[(12.5, 3.5), (3.5, 12.5)]);
        }
        Icon::ChevronDown => pen.line(&[(4.0, 6.0), (8.0, 10.0), (12.0, 6.0)]),
        Icon::ChevronUp => pen.line(&[(4.0, 10.0), (8.0, 6.0), (12.0, 10.0)]),
        Icon::ChevronRight => pen.line(&[(6.0, 4.0), (10.0, 8.0), (6.0, 12.0)]),
        Icon::ChevronLeft => pen.line(&[(10.0, 4.0), (6.0, 8.0), (10.0, 12.0)]),
        Icon::Star => {
            let pts: Vec<(f32, f32)> = (0..10)
                .map(|i| {
                    let a = -PI / 2.0 + i as f32 * PI / 5.0;
                    let rad = if i % 2 == 0 { 6.6 } else { 2.9 };
                    (8.0 + rad * a.cos(), 8.4 + rad * a.sin())
                })
                .collect();
            pen.closed(&pts);
        }
        Icon::Image => {
            pen.rect(1.5, 2.5, 14.5, 13.5);
            pen.circle(5.5, 6.0, 1.3);
            pen.line(&[(1.5, 12.0), (6.0, 8.0), (9.0, 10.5), (11.0, 9.0), (14.5, 12.0)]);
        }
        Icon::File => {
            pen.closed(&[(3.0, 1.5), (10.0, 1.5), (13.0, 4.5), (13.0, 14.5), (3.0, 14.5)]);
            pen.line(&[(10.0, 1.5), (10.0, 4.5), (13.0, 4.5)]);
        }
        Icon::Plus => {
            pen.line(&[(8.0, 2.5), (8.0, 13.5)]);
            pen.line(&[(2.5, 8.0), (13.5, 8.0)]);
        }
        Icon::Layout => {
            pen.rect(1.5, 2.5, 14.5, 13.5);
            pen.line(&[(6.0, 2.5), (6.0, 13.5)]);
            pen.line(&[(6.0, 8.0), (14.5, 8.0)]);
        }
        Icon::Person => {
            pen.circle(8.0, 5.0, 3.0);
            pen.arc(8.0, 15.0, 6.0, PI, 2.0 * PI);
        }
        Icon::Pin => {
            pen.closed(&[(5.5, 2.0), (10.5, 2.0), (10.0, 6.5), (12.5, 9.0), (3.5, 9.0), (6.0, 6.5)]);
            pen.line(&[(8.0, 9.0), (8.0, 14.5)]);
        }
        Icon::Clock => {
            pen.circle(8.0, 8.5, 6.0);
            pen.line(&[(8.0, 5.0), (8.0, 8.5), (10.5, 10.0)]);
        }
        Icon::ListBullet => {
            for y in [4.0, 8.0, 12.0] {
                pen.dot(2.8, y, 1.1);
                pen.line(&[(6.0, y), (14.5, y)]);
            }
        }
        Icon::ListNumber => {
            pen.line(&[(2.0, 2.8), (3.2, 2.0), (3.2, 5.6)]);
            pen.line(&[(1.8, 7.8), (3.0, 7.0), (4.2, 7.8), (1.8, 10.2), (4.4, 10.2)]);
            pen.line(&[(1.8, 12.2), (4.2, 12.2), (3.0, 13.4), (4.2, 14.6), (1.8, 14.8)]);
            for y in [4.0, 9.0, 13.5] {
                pen.line(&[(6.5, y), (14.5, y)]);
            }
        }
        Icon::AlignLeft | Icon::AlignCenter | Icon::AlignRight => {
            for (i, y) in [3.0f32, 6.3, 9.6, 12.9].iter().enumerate() {
                let w = if i % 2 == 0 { 12.5 } else { 8.0 };
                let x0 = match icon {
                    Icon::AlignLeft => 1.8,
                    Icon::AlignCenter => 8.0 - w / 2.0,
                    _ => 14.2 - w,
                };
                pen.line(&[(x0, *y), (x0 + w, *y)]);
            }
        }
        Icon::IndentMore | Icon::IndentLess => {
            for y in [2.5, 13.5] {
                pen.line(&[(1.8, y), (14.5, y)]);
            }
            for y in [6.2, 9.8] {
                pen.line(&[(7.0, y), (14.5, y)]);
            }
            if icon == Icon::IndentMore {
                pen.line(&[(1.8, 5.5), (4.6, 8.0), (1.8, 10.5)]);
            } else {
                pen.line(&[(4.6, 5.5), (1.8, 8.0), (4.6, 10.5)]);
            }
        }
        Icon::Link => {
            pen.line(&[(7.0, 9.0), (9.0, 7.0)]);
            pen.closed(&[(5.6, 7.6), (3.0, 10.2), (3.0, 12.4), (3.6, 13.0), (5.8, 13.0), (8.4, 10.4)]);
            pen.closed(&[(7.6, 5.6), (10.2, 3.0), (12.4, 3.0), (13.0, 3.6), (13.0, 5.8), (10.4, 8.4)]);
        }
        Icon::ClearFormat => {
            pen.line(&[(3.0, 3.0), (11.5, 3.0)]);
            pen.line(&[(7.2, 3.0), (5.0, 13.0)]);
            pen.line(&[(9.5, 9.0), (14.0, 13.5)]);
            pen.line(&[(14.0, 9.0), (9.5, 13.5)]);
        }
        Icon::Note => {
            pen.closed(&[(2.5, 2.5), (13.5, 2.5), (13.5, 10.0), (10.0, 13.5), (2.5, 13.5)]);
            pen.line(&[(13.5, 10.0), (10.0, 10.0), (10.0, 13.5)]);
            pen.line(&[(5.0, 6.0), (11.0, 6.0)]);
            pen.line(&[(5.0, 9.0), (8.5, 9.0)]);
        }
        Icon::AddressBook => {
            pen.rect(3.0, 1.5, 13.5, 14.5);
            pen.line(&[(1.5, 4.5), (4.5, 4.5)]);
            pen.line(&[(1.5, 8.0), (4.5, 8.0)]);
            pen.line(&[(1.5, 11.5), (4.5, 11.5)]);
            pen.circle(8.5, 6.5, 1.8);
            pen.line(&[(5.8, 11.8), (6.6, 10.0), (10.4, 10.0), (11.2, 11.8)]);
        }
        Icon::PopOut => {
            pen.line(&[(7.0, 2.5), (2.5, 2.5), (2.5, 13.5), (13.5, 13.5), (13.5, 9.0)]);
            pen.line(&[(7.5, 8.5), (13.5, 2.5)]);
            pen.line(&[(9.5, 2.5), (13.5, 2.5), (13.5, 6.5)]);
        }
        Icon::Lock => {
            // The shackle, then the body.
            pen.line(&[(5.0, 7.0), (5.0, 4.8), (5.6, 3.0), (8.0, 1.8), (10.4, 3.0), (11.0, 4.8), (11.0, 7.0)]);
            pen.closed(&[(3.0, 7.0), (13.0, 7.0), (13.0, 14.5), (3.0, 14.5)]);
            pen.line(&[(8.0, 9.8), (8.0, 11.8)]);
        }
        Icon::Seal => {
            // A rosette (a circle with notches round it) and a tick inside.
            let pts: Vec<(f32, f32)> = (0..24)
                .map(|i| {
                    let a = i as f32 * PI / 12.0;
                    let r = if i % 2 == 0 { 6.8 } else { 5.8 };
                    (8.0 + r * a.cos(), 8.0 + r * a.sin())
                })
                .collect();
            pen.closed(&pts);
            pen.line(&[(5.4, 8.2), (7.2, 10.0), (10.8, 6.2)]);
        }
        Icon::Steps => {
            pen.line(&[(3.0, 4.0), (7.0, 8.0), (3.0, 12.0)]);
            pen.line(&[(8.5, 4.0), (12.5, 8.0), (8.5, 12.0)]);
        }
        Icon::PopIn => {
            pen.line(&[(7.0, 2.5), (2.5, 2.5), (2.5, 13.5), (13.5, 13.5), (13.5, 9.0)]);
            pen.line(&[(7.5, 8.5), (13.5, 2.5)]);
            pen.line(&[(7.5, 4.5), (7.5, 8.5), (11.5, 8.5)]);
        }
        Icon::Undo => {
            pen.arc(8.0, 9.0, 5.0, PI, 2.0 * PI + 0.6);
            pen.line(&[(1.0, 6.5), (3.0, 9.0), (5.5, 7.0)]);
        }
        Icon::Back => {
            pen.circle(8.0, 8.0, 6.8);
            pen.line(&[(8.0, 4.5), (4.5, 8.0), (8.0, 11.5)]);
            pen.line(&[(4.5, 8.0), (11.5, 8.0)]);
        }
        Icon::Info => {
            pen.circle(8.0, 8.0, 6.5);
            pen.line(&[(8.0, 7.0), (8.0, 11.5)]);
            pen.line(&[(8.0, 4.4), (8.0, 4.9)]);
        }
        Icon::Away => {
            // A door, and an arrow leaving through it.
            pen.line(&[(8.5, 5.0), (8.5, 1.5), (1.5, 1.5), (1.5, 14.5), (8.5, 14.5), (8.5, 11.0)]);
            pen.line(&[(5.0, 8.0), (14.5, 8.0)]);
            pen.line(&[(11.5, 5.0), (14.5, 8.0), (11.5, 11.0)]);
        }
        Icon::Storage => {
            // A stack of discs: flat ellipses joined at the sides.
            let ellipse = |cy: f32, a0: f32, a1: f32| -> Vec<(f32, f32)> {
                (0..=16).map(|i| a0 + (a1 - a0) * i as f32 / 16.0).map(|a| (8.0 + 5.5 * a.cos(), cy + 2.0 * a.sin())).collect()
            };
            pen.closed(&ellipse(3.5, 0.0, 2.0 * PI));
            pen.line(&[(2.5, 3.5), (2.5, 12.5)]);
            pen.line(&[(13.5, 3.5), (13.5, 12.5)]);
            pen.line(&ellipse(8.0, 0.0, PI));
            pen.line(&ellipse(12.5, 0.0, PI));
        }
        Icon::CalendarDay | Icon::CalendarWeek | Icon::CalendarWorkWeek | Icon::Vote => drawn(p, rect, Icon::ListBullet, color),
        Icon::CalendarMonth | Icon::CalendarToday | Icon::CalendarAdd => drawn(p, rect, Icon::Calendar, color),
        Icon::Calendar => {
            pen.rect(1.5, 3.0, 14.5, 14.5);
            pen.line(&[(1.5, 6.5), (14.5, 6.5)]);
            pen.line(&[(5.0, 1.5), (5.0, 4.5)]);
            pen.line(&[(11.0, 1.5), (11.0, 4.5)]);
            for (x, y) in [(5.0, 9.5), (8.0, 9.5), (11.0, 9.5), (5.0, 12.0), (8.0, 12.0)] {
                pen.line(&[(x - 0.6, y), (x + 0.6, y)]);
            }
        }
        Icon::People => {
            pen.circle(6.0, 5.5, 2.6);
            pen.arc(6.0, 14.5, 5.0, PI, 2.0 * PI);
            pen.arc(11.5, 6.0, 2.2, -PI * 0.6, PI * 0.6);
            pen.arc(11.5, 14.5, 4.0, -PI * 0.5, 0.0);
        }
        Icon::Phone => {
            pen.closed(&[
                (3.0, 2.0),
                (6.0, 2.0),
                (7.0, 5.5),
                (5.0, 7.0),
                (9.0, 11.0),
                (10.5, 9.0),
                (14.0, 10.0),
                (14.0, 13.0),
                (12.0, 14.5),
                (2.0, 4.0),
            ]);
        }
        Icon::Location => {
            pen.circle(8.0, 6.0, 2.0);
            pen.line(&[(3.6, 8.0), (8.0, 15.0), (12.4, 8.0)]);
            pen.arc(8.0, 6.0, 4.8, PI * 0.82, PI * 2.18);
        }
        Icon::Funnel => {
            pen.closed(&[(1.5, 2.5), (14.5, 2.5), (9.5, 8.5), (9.5, 14.0), (6.5, 12.5), (6.5, 8.5)]);
        }
        Icon::Print => {
            pen.rect(4.0, 1.5, 12.0, 5.5);
            pen.closed(&[(1.5, 5.5), (14.5, 5.5), (14.5, 11.5), (12.0, 11.5), (12.0, 9.5), (4.0, 9.5), (4.0, 11.5), (1.5, 11.5)]);
            pen.rect(4.0, 9.5, 12.0, 14.5);
        }
        Icon::Tag => {
            pen.closed(&[(1.5, 2.5), (8.0, 2.5), (14.5, 9.0), (9.0, 14.5), (1.5, 8.0)]);
            pen.circle(5.0, 6.0, 1.2);
        }
        Icon::Outbox => {
            pen.line(&[(1.5, 9.0), (4.0, 4.5), (5.5, 4.5)]);
            pen.line(&[(10.5, 4.5), (12.0, 4.5), (14.5, 9.0)]);
            pen.closed(&[(1.5, 9.0), (5.0, 9.0), (6.0, 11.0), (10.0, 11.0), (11.0, 9.0), (14.5, 9.0), (14.5, 13.5), (1.5, 13.5)]);
            pen.line(&[(8.0, 8.5), (8.0, 1.5)]);
            pen.line(&[(5.5, 4.0), (8.0, 1.5), (10.5, 4.0)]);
        }
        Icon::Warning => {
            pen.closed(&[(8.0, 1.5), (15.0, 14.0), (1.0, 14.0)]);
            pen.line(&[(8.0, 6.0), (8.0, 9.8)]);
            pen.filled(&[(7.3, 11.4), (8.7, 11.4), (8.7, 12.6), (7.3, 12.6)], color);
        }
        Icon::Reminder => {
            // A bell, its clapper under it.
            pen.line(&[(3.0, 11.5), (4.0, 10.5), (4.0, 6.5), (4.6, 4.6), (6.0, 3.4), (8.0, 3.0)]);
            pen.line(&[(13.0, 11.5), (12.0, 10.5), (12.0, 6.5), (11.4, 4.6), (10.0, 3.4), (8.0, 3.0)]);
            pen.line(&[(3.0, 11.5), (13.0, 11.5)]);
            pen.line(&[(8.0, 2.0), (8.0, 3.0)]);
            pen.line(&[(6.6, 13.5), (7.4, 14.4), (8.6, 14.4), (9.4, 13.5)]);
        }
        Icon::SpellCheck => {
            // Lines of text, and a tick in a circle beside the last one.
            pen.line(&[(1.5, 4.0), (14.5, 4.0)]);
            pen.line(&[(1.5, 7.5), (14.5, 7.5)]);
            pen.line(&[(1.5, 11.0), (7.5, 11.0)]);
            pen.circle(11.5, 11.5, 4.0);
            pen.line(&[(9.5, 11.5), (11.0, 13.0), (13.5, 10.0)]);
        }
        Icon::Language => {
            // A capital A, with a smaller letter's hook and a bar beside it.
            pen.line(&[(1.5, 14.0), (6.0, 4.0), (10.5, 14.0)]);
            pen.line(&[(3.3, 10.0), (8.7, 10.0)]);
            pen.line(&[(12.0, 3.0), (12.0, 9.0), (14.5, 9.0)]);
            pen.line(&[(11.0, 5.5), (14.5, 5.5)]);
        }
        Icon::Paste => {
            // A clipboard with a sheet on it.
            pen.rect(3.0, 3.0, 13.0, 15.0);
            pen.rect(6.0, 1.5, 10.0, 4.5);
            pen.line(&[(5.5, 8.0), (10.5, 8.0)]);
            pen.line(&[(5.5, 11.0), (10.5, 11.0)]);
        }
        Icon::Cut => {
            // Scissors: two rings and the blades crossing.
            pen.circle(4.5, 12.5, 2.3);
            pen.circle(11.5, 12.5, 2.3);
            pen.line(&[(6.0, 10.8), (12.0, 2.0)]);
            pen.line(&[(10.0, 10.8), (4.0, 2.0)]);
        }
        Icon::Copy => {
            // Two sheets, one over the other.
            pen.rect(5.5, 5.0, 14.0, 15.0);
            pen.line(&[(5.5, 2.0), (2.0, 2.0), (2.0, 11.5)]);
            pen.line(&[(2.0, 2.0), (10.5, 2.0), (10.5, 5.0)]);
        }
        Icon::Signature => {
            // A pen stroke, and the line it's signed on.
            pen.line(&[(2.0, 11.0), (4.0, 7.0), (6.0, 11.0), (8.0, 5.0), (10.0, 11.0), (12.0, 8.0), (14.0, 10.0)]);
            pen.line(&[(2.0, 14.0), (14.0, 14.0)]);
        }
        Icon::Important => {
            // An exclamation mark.
            pen.line(&[(8.0, 2.5), (8.0, 10.5)]);
            pen.dot(8.0, 13.5, 1.1);
        }
        Icon::LowImportance => {
            // An arrow pointing down.
            pen.line(&[(8.0, 2.5), (8.0, 13.5)]);
            pen.line(&[(3.5, 9.0), (8.0, 13.5), (12.5, 9.0)]);
        }
        Icon::Rule => {
            // A line across.
            pen.line(&[(2.0, 8.0), (14.0, 8.0)]);
        }
        Icon::FontGrow => {
            // A capital A and a small arrow up.
            pen.line(&[(1.5, 13.5), (5.5, 3.5), (9.5, 13.5)]);
            pen.line(&[(3.1, 9.5), (7.9, 9.5)]);
            pen.line(&[(13.0, 3.0), (13.0, 9.0)]);
            pen.line(&[(11.0, 5.0), (13.0, 3.0), (15.0, 5.0)]);
        }
        Icon::FontShrink => {
            // A capital A and a small arrow down.
            pen.line(&[(1.5, 13.5), (5.5, 3.5), (9.5, 13.5)]);
            pen.line(&[(3.1, 9.5), (7.9, 9.5)]);
            pen.line(&[(13.0, 3.0), (13.0, 9.0)]);
            pen.line(&[(11.0, 7.0), (13.0, 9.0), (15.0, 7.0)]);
        }
        Icon::WordCount => {
            // Lines of text, and "123" as marks under them.
            pen.line(&[(2.0, 3.0), (14.0, 3.0)]);
            pen.line(&[(2.0, 6.5), (14.0, 6.5)]);
            pen.line(&[(2.0, 10.0), (8.0, 10.0)]);
            pen.dot(9.5, 13.5, 1.0);
            pen.dot(12.0, 13.5, 1.0);
            pen.dot(14.5, 13.5, 1.0);
        }
        Icon::DelayDelivery => {
            // A calendar with a clock over its corner.
            pen.rect(2.0, 3.5, 11.0, 13.5);
            pen.line(&[(2.0, 6.5), (11.0, 6.5)]);
            pen.circle(12.0, 11.5, 3.2);
            pen.line(&[(12.0, 9.8), (12.0, 11.5), (13.4, 12.4)]);
        }
        Icon::Bcc => {
            // An eye.
            pen.line(&[(1.5, 8.0), (4.0, 5.0), (8.0, 3.8), (12.0, 5.0), (14.5, 8.0), (12.0, 11.0), (8.0, 12.2), (4.0, 11.0), (1.5, 8.0)]);
            pen.circle(8.0, 8.0, 2.2);
        }
        Icon::CheckNames => {
            // A person, and a tick beside them.
            pen.circle(6.5, 5.0, 2.6);
            pen.line(&[(1.5, 14.0), (2.0, 11.0), (4.5, 9.0), (8.5, 9.0), (10.0, 10.0)]);
            pen.line(&[(10.5, 12.5), (12.0, 14.0), (15.0, 10.5)]);
        }
    }
}

/// An icon-sized rect centred in `rect`.
pub fn centered(rect: Rect, size: f32) -> Rect {
    Rect::from_center_size(rect.center(), Vec2::splat(size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_renders_from_its_picture() {
        use Icon::*;
        let all = [
            NewMail,
            Delete,
            Archive,
            Junk,
            Reply,
            ReplyAll,
            Forward,
            Move,
            Read,
            Unread,
            Flag,
            Sync,
            Inbox,
            Sent,
            Drafts,
            Folder,
            Search,
            Attach,
            Gear,
            Sun,
            Moon,
            Close,
            ChevronDown,
            ChevronUp,
            ChevronRight,
            ChevronLeft,
            Star,
            Image,
            File,
            Send,
            Plus,
            Layout,
            Person,
            Warning,
            Outbox,
            Pin,
            Clock,
            Tag,
            Print,
            Back,
            Undo,
            Info,
            Away,
            Storage,
            Funnel,
            Calendar,
            CalendarDay,
            CalendarWeek,
            CalendarWorkWeek,
            CalendarMonth,
            CalendarToday,
            CalendarAdd,
            Vote,
            People,
            Phone,
            Location,
            PopOut,
            PopIn,
            ListBullet,
            ListNumber,
            AlignLeft,
            AlignCenter,
            AlignRight,
            IndentMore,
            IndentLess,
            Link,
            ClearFormat,
            AddressBook,
            Note,
            Steps,
            Language,
            Paste,
            Cut,
            Copy,
            Signature,
            Important,
            LowImportance,
            Rule,
            FontGrow,
            FontShrink,
            WordCount,
            DelayDelivery,
            Bcc,
            CheckNames,
        ];
        for icon in all {
            let img = render(icon, 32).unwrap_or_else(|| panic!("{icon:?} didn't render"));
            let inked = img.pixels.iter().filter(|p| p.a() > 128).count();
            // A single thin line (the horizontal rule) inks about 25 pixels at this size.
            assert!((20..32 * 32 / 2).contains(&inked), "{icon:?}: {inked} pixels drawn");
            assert!(img.pixels.iter().all(|p| p.r() == p.a()), "{icon:?}: white, to be tinted");
        }
        // A context draws them, and keeps each size once.
        let ctx = Context::default();
        let a = texture(&ctx, Icon::Reply, 20).unwrap();
        assert_eq!(texture(&ctx, Icon::Reply, 20).unwrap().id(), a.id());
    }
}
