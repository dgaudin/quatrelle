//! Joining an online meeting: a Teams one in a browser's app window (File › Options ›
//! Applications says which browser).

use quatrelle_engine::MEETINGS_PROFILE;

use crate::*;

impl MailApp {
    /// Join a meeting from its link: a Teams meeting in a browser's app window (a window of its
    /// own, which Teams needs for calls on Linux), any other link as [`Self::open_link`] does. The
    /// browser is the one chosen in Options while it's installed, else the first found; the
    /// default browser when that's the choice, when none is found, or when the one tried fails.
    /// Only a program the system lists is launched: the setting alone never names what runs. The
    /// app window opens in the profile chosen ([`meeting_profile`]); one the browser no longer
    /// has gives way to the one it used last, and the user is told.
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
        // The browser's profiles as they are now: one may have been deleted since Options showed it.
        let profiles = self.services.browser_profiles(program);
        let chosen = self.session.prefs.meeting_profile.clone();
        let profile = meeting_profile(chosen.as_deref(), &profiles);
        if let Some(gone) = chosen.as_deref()
            && profile.is_none()
        {
            let text =
                tf!("The profile “{profile}” is no longer in {browser}; the meeting opens in the last profile used", profile = gone, browser = name);
            self.session.notice(text, false);
        }
        if let Err(e) = self.services.open_app_window(program, profile, u) {
            self.session.notice(tf!("{browser} couldn't open the meeting: {error}", browser = name, error = e), true);
            self.open_link(u);
        }
    }

    /// Teams in the meeting browser's app window, in Quatrelle's own profile
    /// ([`MEETINGS_PROFILE`]): the browser makes the profile as it opens, under a name of its
    /// own, and the user signs in to Microsoft 365 there once. Only then is it the one meetings
    /// open in: a browser that didn't open has made nothing, and the next meeting would open in a
    /// profile signed in to nothing. Nothing when meetings open in the default browser, which has
    /// no profile to name.
    pub(crate) fn make_meeting_profile(&mut self) {
        let browsers = self.services.browsers();
        let Some((name, program)) = meeting_browser(self.session.prefs.meeting_browser.as_deref(), &browsers) else { return };
        match self.services.open_app_window(program, Some(MEETINGS_PROFILE), TEAMS_HOME) {
            Ok(()) => self.run("view.meetingProfile", json!({ "dir": MEETINGS_PROFILE })),
            Err(e) => self.session.notice(tf!("{browser} couldn't be opened: {error}", browser = name, error = e), true),
        }
    }
}

/// Where a new meetings profile opens, for the user to sign in to Microsoft 365.
const TEAMS_HOME: &str = "https://teams.microsoft.com/";

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

/// The browser profile folder a Teams meeting opens in, from the setting and the chosen
/// browser's `profiles`: `None`, the one the browser used last. Quatrelle's own
/// ([`MEETINGS_PROFILE`]) is always named, the browser making it when it isn't there yet; any
/// other only while the browser lists it, since naming a folder gone would make the browser
/// create an empty profile in its place. "Join meeting" and File › Options both ask here.
pub(crate) fn meeting_profile<'a>(chosen: Option<&'a str>, profiles: &[BrowserProfile]) -> Option<&'a str> {
    chosen.filter(|dir| *dir == MEETINGS_PROFILE || profiles.iter().any(|p| p.dir == *dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sign-in page passes the check every link handed to a browser passes (the desktop's
    /// launcher refuses any other).
    #[test]
    fn the_sign_in_page_can_be_launched() {
        assert!(quatrelle_engine::model::ical::launchable_link(TEAMS_HOME));
    }
}
