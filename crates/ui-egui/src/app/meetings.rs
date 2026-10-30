//! Joining an online meeting: a Teams one in a browser's app window (File › Options ›
//! Applications says which browser).

use crate::*;

impl MailApp {
    /// Join a meeting from its link: a Teams meeting in a browser's app window (a window of its
    /// own, which Teams needs for calls on Linux), any other link as [`Self::open_link`] does. The
    /// browser is the one chosen in Options while it's installed, else the first found; the
    /// default browser when that's the choice, when none is found, or when the one tried fails.
    /// Only a program the system lists is launched: the setting alone never names what runs.
    pub(crate) fn open_meeting(&mut self, url: &str) {
        let cleaned = if self.session.prefs.clean_links { quatrelle_engine::mime::clean_link(url) } else { url.to_string() };
        let u = cleaned.trim();
        if !quatrelle_engine::model::ical::is_teams_link(u) {
            self.open_link(url);
            return;
        }
        // A Teams address with a space or a control character in it, or far too long: not handed
        // to a browser.
        if !quatrelle_engine::model::ical::launchable_link(u) {
            self.session.notice(tl!("This meeting link can't be opened"), true);
            return;
        }
        let browsers = self.services.browsers();
        let Some((name, program)) = meeting_browser(self.session.prefs.meeting_browser.as_deref(), &browsers) else {
            self.open_link(u);
            return;
        };
        if let Err(e) = self.services.open_app_window(program, u) {
            self.session.notice(tf!("{browser} couldn't open the meeting: {error}", browser = name, error = e), true);
            self.open_link(u);
        }
    }
}

/// The browser (shown name, program) Teams meetings open in, from the setting and the browsers
/// found: `None` is the default browser. Nothing chosen, or a choice no longer installed: the
/// first one found. "Join meeting" and File › Options both ask here, so they never disagree.
pub(crate) fn meeting_browser<'a>(chosen: Option<&str>, browsers: &'a [(String, String)]) -> Option<&'a (String, String)> {
    match chosen {
        Some("") => None,
        Some(chosen) => browsers.iter().find(|(_, program)| program == chosen).or_else(|| browsers.first()),
        None => browsers.first(),
    }
}
