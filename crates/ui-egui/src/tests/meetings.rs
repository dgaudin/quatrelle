//! UI tests: joining an online meeting (a Teams one in a browser's app window).

use super::*;

use std::cell::RefCell;
use std::rc::Rc;

use crate::BrowserProfile;
use quatrelle_engine::MEETINGS_PROFILE;

const TEAMS: &str = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_x%40thread.v2/0?context=%7b%7d";

/// Services that offer `browsers` for meetings, each with `profiles`, and remember what was
/// opened: `url <link>` for the default browser, `app <program> <link>` for an app window (which
/// fails when `fails`), with `--profile-directory=<folder>` before the link when one is given.
struct Launching {
    browsers: Vec<(String, String)>,
    profiles: Vec<BrowserProfile>,
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
    fn browser_profiles(&mut self, program: &str) -> Vec<BrowserProfile> {
        if self.browsers.iter().any(|(_, p)| p == program) { self.profiles.clone() } else { Vec::new() }
    }
    fn open_app_window(&mut self, program: &str, profile: Option<&str>, url: &str) -> Result<(), String> {
        let flag = profile.map(|p| format!("--profile-directory={p} ")).unwrap_or_default();
        self.opened.borrow_mut().push(format!("app {program} {flag}{url}"));
        if self.fails { Err("boom".into()) } else { Ok(()) }
    }
}

/// The app over the sample mailbox with `browsers` installed, and what it opens.
fn launching(name: &str, browsers: &[(&str, &str)], fails: bool) -> (MailApp, std::path::PathBuf, Rc<RefCell<Vec<String>>>) {
    launching_with(name, browsers, &[], fails)
}

/// A profile as a browser lists it: (folder, name, Google account).
fn profile(dir: &str, name: &str, account: &str) -> BrowserProfile {
    BrowserProfile { dir: dir.into(), name: name.into(), account: account.into() }
}

/// [`launching`], each browser having `profiles`.
fn launching_with(
    name: &str,
    browsers: &[(&str, &str)],
    profiles: &[BrowserProfile],
    fails: bool,
) -> (MailApp, std::path::PathBuf, Rc<RefCell<Vec<String>>>) {
    let dir = std::env::temp_dir().join(format!("quatrelle-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    sample::seed_if_empty(&store, NOW).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    let opened = Rc::new(RefCell::new(Vec::new()));
    let browsers = browsers.iter().map(|(n, p)| (n.to_string(), p.to_string())).collect();
    let services = Launching { browsers, profiles: profiles.to_vec(), fails, opened: opened.clone() };
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

/// File › Options › Applications: the browser Teams meetings open in, the one "Join meeting" uses
/// (the first found until one is chosen), or the default browser. The list is asked for as the
/// File page opens, here by its rail's "Options".
#[test]
fn the_options_choose_the_meeting_browser() {
    let (mut a, dir, _) = launching("meet-options", &[("Google Chrome", "/opt/chrome"), ("Chromium", "/usr/bin/chromium")], false);
    let ctx = egui::Context::default();
    let tall = egui::vec2(1280.0, 5000.0);
    a.act(crate::Action::Backstage(crate::backstage::Page::Info));
    let options = settled_text_at(&ctx, &mut a, "Options", tall).unwrap();
    click_at(&ctx, &mut a, options, 1, tall);
    let combo = settled_text_at(&ctx, &mut a, "Google Chrome", tall).unwrap();
    click_at(&ctx, &mut a, combo, 1, tall);
    let chromium = settled_text_at(&ctx, &mut a, "Chromium", tall).unwrap();
    click_at(&ctx, &mut a, chromium, 1, tall);
    assert_eq!(a.session.prefs.meeting_browser.as_deref(), Some("/usr/bin/chromium"));
    let combo = settled_text_at(&ctx, &mut a, "Chromium", tall).unwrap();
    click_at(&ctx, &mut a, combo, 1, tall);
    let default = settled_text_at(&ctx, &mut a, "The default browser", tall).unwrap();
    click_at(&ctx, &mut a, default, 1, tall);
    assert_eq!(a.session.prefs.meeting_browser.as_deref(), Some(""));
    let _ = std::fs::remove_dir_all(dir);
}

/// A setting that points at a browser since uninstalled shows the first one found; choosing that
/// very entry still writes it, so the dead path is gone and a reinstall of the old browser doesn't
/// take the meetings back unasked.
#[test]
fn choosing_the_entry_already_shown_overwrites_a_stale_path() {
    let (mut a, dir, _) = launching("meet-stale", &[("Google Chrome", "/opt/chrome")], false);
    a.session.prefs.meeting_browser = Some("/gone".into());
    let ctx = egui::Context::default();
    let tall = egui::vec2(1280.0, 5000.0);
    a.act(crate::Action::Backstage(crate::backstage::Page::Info));
    let options = settled_text_at(&ctx, &mut a, "Options", tall).unwrap();
    click_at(&ctx, &mut a, options, 1, tall);
    // The combo shows Chrome, though the setting still says "/gone".
    let combo = settled_text_at(&ctx, &mut a, "Google Chrome", tall).unwrap();
    click_at(&ctx, &mut a, combo, 1, tall);
    // The list is open: the same entry again.
    let entry = settled_text_at(&ctx, &mut a, "Google Chrome", tall).unwrap();
    click_at(&ctx, &mut a, entry, 1, tall);
    assert_eq!(a.session.prefs.meeting_browser.as_deref(), Some("/opt/chrome"));
    let _ = std::fs::remove_dir_all(dir);
}

/// The profile chosen in Options is the one the meeting opens in; none chosen, the browser's
/// last used (no profile named).
#[test]
fn a_meeting_joins_in_the_profile_chosen() {
    let profiles = [profile("Default", "Personne 1", ""), profile("Profile 2", "Work", "me@studio.example")];
    let (mut a, dir, o) = launching_with("meet-profile", &[("Google Chrome", "/opt/chrome")], &profiles, false);
    a.session.prefs.meeting_profile = Some("Profile 2".into());
    a.act(crate::Action::OpenMeeting(TEAMS.into()));
    a.session.prefs.meeting_profile = None;
    a.act(crate::Action::OpenMeeting(TEAMS.into()));
    assert_eq!(opened(&o), [format!("app /opt/chrome --profile-directory=Profile 2 {TEAMS}"), format!("app /opt/chrome {TEAMS}")]);
    assert!(a.session.notices.is_empty(), "{:?}", a.session.notices);
    // The default browser has no profiles to choose from.
    a.session.prefs.meeting_profile = Some("Profile 2".into());
    a.session.prefs.meeting_browser = Some(String::new());
    a.act(crate::Action::OpenMeeting(TEAMS.into()));
    assert_eq!(opened(&o).last(), Some(&format!("url {TEAMS}")));
    let _ = std::fs::remove_dir_all(dir);
}

/// A profile since deleted in the browser isn't named (the browser would make an empty one in
/// its place): the meeting opens in the last profile used, and the user is told.
#[test]
fn a_profile_gone_gives_way_to_the_last_used() {
    let (mut a, dir, o) = launching_with("meet-profile-gone", &[("Google Chrome", "/opt/chrome")], &[profile("Default", "Me", "")], false);
    a.session.prefs.meeting_profile = Some("Profile 9".into());
    a.act(crate::Action::OpenMeeting(TEAMS.into()));
    assert_eq!(opened(&o), [format!("app /opt/chrome {TEAMS}")]);
    let last = a.session.notices.last();
    let said = "The profile “Profile 9” is no longer in Google Chrome; the meeting opens in the last profile used";
    assert!(last.is_some_and(|n| n.text == said), "{last:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Quatrelle's own meetings profile is named even before the browser lists it: the browser makes
/// it when asked for it.
#[test]
fn the_meetings_profile_is_always_named() {
    let (mut a, dir, o) = launching_with("meet-profile-own", &[("Chromium", "/usr/bin/chromium")], &[profile("Default", "Me", "")], false);
    a.session.prefs.meeting_profile = Some(MEETINGS_PROFILE.into());
    a.act(crate::Action::OpenMeeting(TEAMS.into()));
    assert_eq!(opened(&o), [format!("app /usr/bin/chromium --profile-directory=Quatrelle {TEAMS}")]);
    assert!(a.session.notices.is_empty(), "{:?}", a.session.notices);
    let _ = std::fs::remove_dir_all(dir);
}
