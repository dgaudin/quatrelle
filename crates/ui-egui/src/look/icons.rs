//! The interface icons: Microsoft's Fluent UI System Icons (MIT, `assets/icons/fluent`, the
//! owner's choice so the window looks like Outlook's), rendered from their SVG once per size and
//! tinted when drawn. Each also has a line drawing in code on a 16-unit grid, used should its
//! picture fail to render.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use egui::{Color32, ColorImage, Context, Id, Painter, Rect, TextureHandle, TextureOptions, Vec2, pos2};

mod drawn;

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
    /// A video camera (an online meeting).
    Video,
    /// A list with ticks (Outlook's Tasks).
    Tasks,
    /// A tick in a circle (Mark Complete).
    Complete,
    /// A smiling face (Insert › Emoji).
    Emoji,
    /// The kinds of a contacts field's entries (a person is `Person`): a distribution list, a
    /// contact group, a room, equipment, a shared mailbox, a public folder, a Microsoft 365
    /// group, and an address alone.
    DistributionList,
    ContactGroup,
    Room,
    Equipment,
    SharedMailbox,
    PublicFolder,
    Group365,
    Address,
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
        Icon::Video => f!("video"),
        Icon::Tasks => f!("task_list_ltr"),
        Icon::Complete => f!("checkmark_circle"),
        Icon::Emoji => f!("emoji"),
        Icon::DistributionList => f!("people_list"),
        Icon::ContactGroup => f!("people_team"),
        Icon::Room => f!("conference_room"),
        Icon::Equipment => f!("projection_screen"),
        Icon::SharedMailbox => f!("mail_multiple"),
        Icon::PublicFolder => f!("folder_mail"),
        Icon::Group365 => f!("people_community"),
        Icon::Address => f!("mail"),
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
        None => drawn::drawn(p, rect, icon, color),
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
            Lock,
            Seal,
            Video,
            Tasks,
            Complete,
            Emoji,
            DistributionList,
            ContactGroup,
            Room,
            Equipment,
            SharedMailbox,
            PublicFolder,
            Group365,
            Address,
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
