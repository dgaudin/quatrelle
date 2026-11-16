// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Joining an online meeting: a Teams one in a browser's app window (File › Options ›
//! Applications says which browser). Teams' own pages open there too: Microsoft's page to sign in
//! to Teams on, and a chat in Teams ("Open in Teams").
//!
//! What is hostile here: the pages come from the Teams worker (Microsoft's sign-in service) and
//! from Graph (a chat's link). Each is checked again before it's handed to a browser, as one
//! argument after the app window's option: only Microsoft's sign-in page
//! ([`is_microsoft_sign_in`]) and a Teams chat ([`is_teams_chat_link`]) ever open. Each page
//! leaves one line in the log ([`page_line`]): what it is, where it opened, and its link's shape
//! ([`link_shape`]), never an id, a value or a profile's folder.

use quatrelle_engine::model::ical::{is_microsoft_sign_in, is_teams_chat_link, link_shape};
use quatrelle_engine::{MEETINGS_PROFILE, TeamsPage, is_profile_dir};

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
        // Where the link goes in the end: a Teams address inside the organisation's link check
        // is Teams' own, and opens in the app window. Any other link opens as links do.
        let cleaned = if self.session.prefs.clean_links { quatrelle_engine::mime::link_destination(url) } else { url.to_string() };
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
        match self.in_app_window(u) {
            Ok(Some(_)) => {}
            Ok(None) => self.open_link(u),
            Err((browser, e)) => {
                self.session.notice(tf!("{browser} couldn't open the meeting: {error}", browser = browser, error = e), true);
                self.open_link(u);
            }
        }
    }

    /// `url` in the meetings browser's app window, in the profile chosen ([`meeting_profile`]):
    /// opened (`Ok(Some(…))`: the browser's shown name and which profile); no browser found, or
    /// the default browser chosen (`Ok(None)`: the caller opens it there); or the browser's name
    /// and why it couldn't (`Err`). The browser is the one chosen in Options while it's installed,
    /// else the first found; only a program the system lists is launched, the setting alone never
    /// names what runs. A profile the browser no longer has gives way to the one it used last, and
    /// the user is told.
    fn in_app_window(&mut self, url: &str) -> Result<Option<(String, ProfileKind)>, (String, String)> {
        let browsers = self.services.browsers();
        let Some((name, program)) = meeting_browser(self.session.prefs.meeting_browser.as_deref(), &browsers) else { return Ok(None) };
        // The browser's profiles as they are now: one may have been deleted since Options showed it.
        let profiles = self.services.browser_profiles(program);
        let chosen = self.session.prefs.meeting_profile.clone();
        let profile = meeting_profile(chosen.as_deref(), &profiles);
        if let Some(gone) = chosen.as_deref()
            && profile.is_none()
        {
            // The setting comes from a file: said cut short, without control characters.
            let gone: String = gone.chars().filter(|c| !c.is_control()).collect();
            let gone = widgets::shorten(&gone, 40);
            let text =
                tf!("The profile “{profile}” is no longer in {browser}; the meeting opens in the last profile used", profile = gone, browser = name);
            self.session.notice(text, false);
        }
        let kind = ProfileKind::of(profile);
        self.services.open_app_window(program, profile, url).map(|()| Some((name.clone(), kind))).map_err(|e| (name.clone(), e))
    }

    /// The Teams pages the engine asks for (Microsoft's page to sign in on, a chat in Teams), in
    /// the meetings browser's app window, else the default browser (none found, the default chosen,
    /// or the launch failed, which is said). Each is checked again first: a page that isn't
    /// Microsoft's sign-in or a Teams chat opens nowhere. Each leaves its line in the log
    /// ([`page_line`]).
    pub(crate) fn teams_pages(&mut self) {
        while let Some(page) = self.session.take_teams_page() {
            let (url, ours) = match &page {
                TeamsPage::SignIn(url) => (url.as_str(), is_microsoft_sign_in(url)),
                TeamsPage::Chat(url) => (url.as_str(), is_teams_chat_link(url)),
            };
            if !ours {
                log::warn!("{}", page_line(&page, &PageOutcome::Refused));
                continue;
            }
            let failed = match self.in_app_window(url) {
                Ok(Some((browser, profile))) => {
                    log::info!("{}", page_line(&page, &PageOutcome::AppWindow { browser: &browser, profile }));
                    continue;
                }
                Ok(None) => None,
                Err((browser, e)) => {
                    self.session.notice(tf!("{browser} couldn't open the page: {error}", browser = browser, error = e), true);
                    Some((browser, e))
                }
            };
            match (self.services.open_url(url), &failed) {
                (Ok(()), None) => log::info!("{}", page_line(&page, &PageOutcome::DefaultBrowser)),
                (Ok(()), Some((browser, e))) => log::warn!("{}", page_line(&page, &PageOutcome::InsteadOf { browser, error: e })),
                (Err(e), _) => {
                    self.session.notice(tf!("The link couldn't be opened: {error}", error = e), true);
                    // Both tries failed: both said.
                    let error = failed.map_or_else(|| e.clone(), |(browser, first)| format!("{browser}: {first}; the default browser: {e}"));
                    log::warn!("{}", page_line(&page, &PageOutcome::Failed { error: &error }));
                }
            }
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

/// Which browser profile a Teams page opened in, for the log: never a folder's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProfileKind {
    /// Quatrelle's own ([`MEETINGS_PROFILE`]).
    Quatrelles,
    /// Another one chosen in Options.
    Chosen,
    /// The one the browser used last (none named).
    LastUsed,
}

impl ProfileKind {
    /// The kind of the profile folder named to the browser (`None`: none named).
    pub(crate) fn of(profile: Option<&str>) -> Self {
        match profile {
            Some(MEETINGS_PROFILE) => Self::Quatrelles,
            Some(_) => Self::Chosen,
            None => Self::LastUsed,
        }
    }
}

/// Where a Teams page went, for its line in the log ([`page_line`]).
pub(crate) enum PageOutcome<'a> {
    /// The meetings browser's app window (its shown name), in that profile.
    AppWindow { browser: &'a str, profile: ProfileKind },
    /// The default browser: none found, or the default chosen.
    DefaultBrowser,
    /// The app window couldn't open (the browser, why); the default browser did.
    InsteadOf { browser: &'a str, error: &'a str },
    /// Nowhere: why.
    Failed { error: &'a str },
    /// Refused: not Teams' own page.
    Refused,
}

/// The line a Teams page leaves in the log: what it is, where it went, and its link's shape
/// ([`link_shape`]): never the chat's id, the organisation's, a query's value or a profile's
/// folder. A browser's name and a launcher's error are kept to one line of at most 200
/// characters. A launcher's error is free text from the system and may name the page (macOS'
/// `open` prints its whole command line, the address with it): one that holds `://`, the
/// page's address or a `"` is withheld.
pub(crate) fn page_line(page: &TeamsPage, outcome: &PageOutcome) -> String {
    let (what, url) = match page {
        TeamsPage::SignIn(url) => ("the sign-in page", url),
        TeamsPage::Chat(url) => ("a chat", url),
    };
    let shape = link_shape(url);
    let plain = |s: &str| s.chars().filter(|c| !c.is_control()).take(200).collect::<String>();
    let told = |e: &str| {
        let names_it = e.contains("://") || e.contains('"') || (!url.is_empty() && e.contains(url.as_str()));
        if names_it { "(withheld: it names the address)".to_string() } else { plain(e) }
    };
    match outcome {
        PageOutcome::AppWindow { browser, profile } => {
            let profile = match profile {
                ProfileKind::Quatrelles => "Quatrelle's own profile",
                ProfileKind::Chosen => "another profile chosen",
                ProfileKind::LastUsed => "the profile it used last",
            };
            format!("teams: {what} opened in {}, in {profile}: {shape}", plain(browser))
        }
        PageOutcome::DefaultBrowser => format!("teams: {what} opened in the default browser: {shape}"),
        PageOutcome::InsteadOf { browser, error } => {
            format!("teams: {what} couldn't be opened in {} ({}); the default browser instead: {shape}", plain(browser), told(error))
        }
        PageOutcome::Failed { error } => format!("teams: {what} couldn't be opened: {}: {shape}", told(error)),
        PageOutcome::Refused => format!("teams: a page that isn't Teams' own was not opened ({what}): {shape}"),
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
/// create an empty profile in its place. A folder that isn't one name is never named, whoever
/// lists it. "Join meeting" and File › Options both ask here.
pub(crate) fn meeting_profile<'a>(chosen: Option<&'a str>, profiles: &[BrowserProfile]) -> Option<&'a str> {
    chosen.filter(|dir| is_profile_dir(dir) && (*dir == MEETINGS_PROFILE || profiles.iter().any(|p| p.dir == *dir)))
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
