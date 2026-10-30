//! UI tests: joining an online meeting (a Teams one in a browser's app window).

use super::*;

use std::cell::RefCell;
use std::rc::Rc;

const TEAMS: &str = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_x%40thread.v2/0?context=%7b%7d";

/// Services that offer `browsers` for meetings and remember what was opened: `url <link>` for the
/// default browser, `app <program> <link>` for an app window (which fails when `fails`).
struct Launching {
    browsers: Vec<(String, String)>,
    fails: bool,
    opened: Rc<RefCell<Vec<String>>>,
}

impl crate::Services for Launching {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
    fn save_path(&mut self, _: &str) -> Option<std::path::PathBuf> {
        None
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Err("refused".into())
    }
    fn open_url(&mut self, url: &str) -> Result<(), String> {
        self.opened.borrow_mut().push(format!("url {url}"));
        Ok(())
    }
    fn browsers(&mut self) -> Vec<(String, String)> {
        self.browsers.clone()
    }
    fn open_app_window(&mut self, program: &str, url: &str) -> Result<(), String> {
        self.opened.borrow_mut().push(format!("app {program} {url}"));
        if self.fails { Err("boom".into()) } else { Ok(()) }
    }
}

/// The app over the sample mailbox with `browsers` installed, and what it opens.
fn launching(name: &str, browsers: &[(&str, &str)], fails: bool) -> (MailApp, std::path::PathBuf, Rc<RefCell<Vec<String>>>) {
    let dir = std::env::temp_dir().join(format!("quatrelle-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    sample::seed_if_empty(&store, NOW).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    let opened = Rc::new(RefCell::new(Vec::new()));
    let browsers = browsers.iter().map(|(n, p)| (n.to_string(), p.to_string())).collect();
    let services = Launching { browsers, fails, opened: opened.clone() };
    (MailApp::new(s, Box::new(NullBackend), Box::new(services)), dir, opened)
}

fn opened(o: &Rc<RefCell<Vec<String>>>) -> Vec<String> {
    o.borrow().clone()
}

#[test]
fn a_teams_meeting_joins_in_the_app_window() {
    let (mut a, dir, o) = launching("meet-app", &[("Google Chrome", "/opt/chrome")], false);
    a.act(crate::Action::OpenMeeting(TEAMS.into()));
    assert_eq!(opened(&o), [format!("app /opt/chrome {TEAMS}")]);
    // The one chosen, when it's there.
    let (mut a2, dir2, o2) = launching("meet-app-chosen", &[("Google Chrome", "/opt/chrome"), ("Chromium", "/usr/bin/chromium")], false);
    a2.session.prefs.meeting_browser = Some("/usr/bin/chromium".into());
    a2.act(crate::Action::OpenMeeting(TEAMS.into()));
    assert_eq!(opened(&o2), [format!("app /usr/bin/chromium {TEAMS}")]);
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
}

#[test]
fn other_meetings_open_as_before() {
    let (mut a, dir, o) = launching("meet-zoom", &[("Google Chrome", "/opt/chrome")], false);
    a.act(crate::Action::OpenMeeting("https://zoom.us/j/123456".into()));
    assert_eq!(opened(&o), ["url https://zoom.us/j/123456"]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_default_browser_when_chosen_or_none_found() {
    let (mut a, dir, o) = launching("meet-default", &[("Google Chrome", "/opt/chrome")], false);
    a.session.prefs.meeting_browser = Some(String::new());
    a.act(crate::Action::OpenMeeting(TEAMS.into()));
    assert_eq!(opened(&o), [format!("url {TEAMS}")]);
    let (mut a2, dir2, o2) = launching("meet-none", &[], false);
    a2.act(crate::Action::OpenMeeting(TEAMS.into()));
    assert_eq!(opened(&o2), [format!("url {TEAMS}")]);
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
}

/// The browser chosen in Options was uninstalled since: the first one found. Only a program the
/// system offers is ever launched, whatever the setting says.
#[test]
fn a_browser_gone_gives_way_to_the_first_found() {
    let (mut a, dir, o) = launching("meet-gone", &[("Chromium", "/usr/bin/chromium")], false);
    a.session.prefs.meeting_browser = Some("/gone".into());
    a.act(crate::Action::OpenMeeting(TEAMS.into()));
    assert_eq!(opened(&o), [format!("app /usr/bin/chromium {TEAMS}")]);
    let _ = std::fs::remove_dir_all(dir);
}

/// An organisation's invitations carry their links wrapped by Safe Links: unwrapped, it's Teams.
#[test]
fn a_safe_links_wrapped_teams_link_still_joins() {
    let (mut a, dir, o) = launching("meet-safelinks", &[("Google Chrome", "/opt/chrome")], false);
    a.session.prefs.clean_links = true;
    let wrapped = "https://eur01.safelinks.protection.outlook.com/?url=https%3A%2F%2Fteams.microsoft.com%2Fl%2Fmeetup-join%2F19%253ameeting_x%2540thread.v2%2F0%3Fcontext%3D%257b%257d&data=05%7C01&reserved=0";
    a.act(crate::Action::OpenMeeting(wrapped.into()));
    assert_eq!(opened(&o), [format!("app /opt/chrome {TEAMS}")]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_browser_that_fails_still_opens_the_meeting() {
    let (mut a, dir, o) = launching("meet-fails", &[("Google Chrome", "/opt/chrome")], true);
    a.act(crate::Action::OpenMeeting(TEAMS.into()));
    assert_eq!(opened(&o), [format!("app /opt/chrome {TEAMS}"), format!("url {TEAMS}")]);
    let last = a.session.notices.last();
    assert!(last.is_some_and(|n| n.error && n.text == "Google Chrome couldn't open the meeting: boom"), "{last:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_doubtful_teams_link_is_not_opened() {
    let (mut a, dir, o) = launching("meet-doubtful", &[("Google Chrome", "/opt/chrome")], false);
    a.act(crate::Action::OpenMeeting("https://teams.microsoft.com/meet/1 x".into()));
    assert!(opened(&o).is_empty(), "{:?}", opened(&o));
    let last = a.session.notices.last();
    assert!(last.is_some_and(|n| n.error && n.text == "This meeting link can't be opened"), "{last:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The event's "Join meeting" button, and a Teams link in its notes, both join in the app window.
#[test]
fn join_meeting_asks_for_the_meeting() {
    let (mut a, dir, o) = launching("meet-join", &[("Google Chrome", "/opt/chrome")], false);
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    let day = quatrelle_engine::model::dates::day_string(NOW.div_euclid(86_400) + 1);
    let notes = "Or join here: https://teams.microsoft.com/meet/42";
    a.run(
        "event.create",
        json!({"summary": "Weekly sync", "start": format!("{day}T10:00"), "end": format!("{day}T10:30"), "location": TEAMS, "description": notes}),
    );
    let it = a.session.agenda(NOW, NOW + 3 * 86_400).into_iter().find(|i| i.summary == "Weekly sync").unwrap();
    a.ui.dialog = Dialog::open_event(&a.session, &it.uid, it.occurrence);
    let join = settled_text_at(&ctx, &mut a, "Join meeting", size).unwrap();
    click_at(&ctx, &mut a, join, 1, size);
    assert_eq!(opened(&o), [format!("app /opt/chrome {TEAMS}")]);
    let link = settled_text_at(&ctx, &mut a, "teams.microsoft.com/meet/42", size).unwrap();
    click_at(&ctx, &mut a, link, 1, size);
    assert_eq!(opened(&o).last().map(String::as_str), Some("app /opt/chrome https://teams.microsoft.com/meet/42"));
    // The notes being edited: the same link, under them in the "Links:" row.
    let edit = settled_text_at(&ctx, &mut a, "Edit notes", size).unwrap();
    click_at(&ctx, &mut a, edit, 1, size);
    // It follows the first link on the row, and its galley starts where the row does: its end.
    let link = (0..40)
        .filter_map(|_| {
            frame_shapes(&ctx, &mut a, vec![], size).iter().rev().find_map(|s| match s {
                egui::Shape::Text(t) if t.galley.text() == "teams.microsoft.com/meet/42" => {
                    Some(t.pos + t.galley.rect.right_center().to_vec2() - egui::vec2(8.0, 0.0))
                }
                _ => None,
            })
        })
        .last()
        .unwrap();
    click_at(&ctx, &mut a, link, 1, size);
    assert_eq!(opened(&o).len(), 3, "{:?}", opened(&o));
    assert_eq!(opened(&o).last().map(String::as_str), Some("app /opt/chrome https://teams.microsoft.com/meet/42"));
    let _ = std::fs::remove_dir_all(dir);
}

/// The appointment's form drawn on its own (the app shows it in a window of its own, as above):
/// its "Join meeting" button and its notes' links ask for the meeting too.
#[test]
fn the_appointment_form_joins_too() {
    let (mut a, dir, _) = launching("meet-form", &[], false);
    let day = quatrelle_engine::model::dates::day_string(NOW.div_euclid(86_400) + 1);
    let notes = "Or join here: https://teams.microsoft.com/meet/42";
    a.run(
        "event.create",
        json!({"summary": "Weekly sync", "start": format!("{day}T10:00"), "end": format!("{day}T10:30"), "location": TEAMS, "description": notes}),
    );
    let it = a.session.agenda(NOW, NOW + 3 * 86_400).into_iter().find(|i| i.summary == "Weekly sync").unwrap();
    let Some(Dialog::Event(mut f)) = Dialog::open_event(&a.session, &it.uid, it.occurrence) else { panic!("no form") };
    let ctx = egui::Context::default();
    let tokens = a.ui.tokens;
    let mut asked = Vec::new();
    // Frames of the form alone; what it asks for, and where `text` ends (a link on a row with
    // others starts where the row does).
    let mut draw = |events: Vec<Event>, text: &str, asked: &mut Vec<String>| {
        let input =
            RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 800.0))), events, ..Default::default() };
        let mut acts = Vec::new();
        let mut out = ctx.run_ui(input, |ui| {
            crate::pim_dialogs::event(ui, &mut f, &a.session, &tokens, &mut acts);
        });
        out.textures_delta.clear();
        asked.extend(acts.into_iter().filter_map(|act| match act {
            crate::Action::OpenMeeting(u) => Some(format!("meeting {u}")),
            crate::Action::OpenUrl(u) => Some(format!("url {u}")),
            _ => None,
        }));
        out.shapes.iter().rev().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.pos + t.galley.rect.right_center().to_vec2() - egui::vec2(8.0, 0.0)),
            _ => None,
        })
    };
    let mut click = |text: &str, asked: &mut Vec<String>| {
        let at = (0..10).filter_map(|_| draw(vec![], text, asked)).last().unwrap();
        let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        for events in [vec![Event::PointerMoved(at)], vec![press(true)], vec![press(false)], vec![]] {
            draw(events, text, asked);
        }
    };
    click("Join meeting", &mut asked);
    click("teams.microsoft.com/meet/42", &mut asked);
    // The notes being edited: the "Links:" row under them.
    click("Edit notes", &mut asked);
    click("teams.microsoft.com/meet/42", &mut asked);
    let meet = "meeting https://teams.microsoft.com/meet/42".to_string();
    assert_eq!(asked, [format!("meeting {TEAMS}"), meet.clone(), meet]);
    let _ = std::fs::remove_dir_all(dir);
}
