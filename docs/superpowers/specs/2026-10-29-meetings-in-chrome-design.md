# Teams meetings and calls in a Chrome app window (1.1, part A3)

Status: approved in conversation on 2026-10-29; this document is the written spec to review.

## Why

The owner works on Linux, where Microsoft no longer ships a Teams desktop client. Teams meetings
and calls run only in Chromium-family browsers there (Edge, Chrome; Firefox with caveats; never
WebKit). Today "Join meeting" opens the link in the default browser, which may not be one of them.

This is the first of three parts of "Teams and WhatsApp in Quatrelle" (approach A, chosen by the
owner on 2026-10-29):

- **A1** WhatsApp Web inside Quatrelle's window (a module in the left rail).
- **A2** Teams drawn by Quatrelle itself through Microsoft Graph: chats, channels, presence, files.
- **A3** (this spec) a shared launcher that opens Teams meetings and calls, and later WhatsApp
  calls, in a Chromium-family browser's app window. The owner prefers calls in a window of their
  own.

## What the owner sees

- "Join meeting" on a Teams meeting (the event dialog, the meeting window, wherever Quatrelle
  offers it) opens a separate window of Google Chrome, Chromium or Microsoft Edge in app mode:
  no tabs, no address bar. It uses the browser's usual profile, so a Microsoft 365 sign-in
  already there carries over, and a browser already running adds the window to itself.
- Other meeting links (Zoom, Google Meet, Webex, GoTo, Jitsi, Whereby, Skype) keep opening as
  they do now, in the default browser (which hands them to their own apps).
- File › Options › Applications has "Open Teams meetings and calls in": the Chromium-family
  browsers found on the computer, and "The default browser". At first: the first one found
  (Chrome, then Chromium, then Edge), else the default browser.
- With no Chromium-family browser, or the default browser chosen, Teams links open as today.

## What counts as a Teams meeting link

`https://` (or `http://`) to `teams.microsoft.com` (or a subdomain) under `l/meetup-join` or
`meet/`, or to `teams.live.com` under `meet`: exactly what `quatrelle_model::ical::notes::
is_meeting_link` already says is a Teams link. One definition: a new `is_teams_link(url)` sits
beside it and `is_meeting_link` uses it.

## How the browser is found

A pure function over what the platform offers, tested with fake inputs:

- **Linux and the BSDs:** the first of `google-chrome-stable`, `google-chrome`, `chromium`,
  `chromium-browser`, `microsoft-edge-stable`, `microsoft-edge` found as an executable file in a
  `PATH` directory.
- **Windows:** `%ProgramFiles%` and `%ProgramFiles(x86)%` under `Google\Chrome\Application\chrome.exe`
  and `Microsoft\Edge\Application\msedge.exe`, and `%LocalAppData%\Google\Chrome\Application\chrome.exe`.
- **macOS:** `/Applications/Google Chrome.app`, `/Applications/Chromium.app`,
  `/Applications/Microsoft Edge.app` (and the same under `~/Applications`), launched through the
  executable inside `Contents/MacOS/`.

Each found browser is (name shown, program path). The setting keeps the program path; a path no
longer there falls back to the first found, then the default browser.

## How it's opened

`Services::open_app_window(program, url) -> Result<(), String>` (the UI picks the program from
`browsers()`): the desktop app runs `<program> --app=<url>` directly (`std::process::Command`, no shell), detached, without waiting.
The browser's windows and processes are its own: Quatrelle doesn't watch or close them.

Before running anything the URL is checked: `https://` or `http://` only, no whitespace or control
characters, at most 2,048 bytes, and it must parse as a URL with a host. A link that fails is
not opened and the user is told (a notice: "This meeting link can't be opened").

Links clicked in the event and meeting dialogs (the Join buttons, "Links:", the notes' links) all take
this path; links in a mail body don't.

The Options › Applications card is desktop-only.

The browser version (`quatrelle-web`) has no such service: Teams links open as other links do
there.

## Where it lives

| Layer | What |
|---|---|
| `model` | `is_teams_link` beside `is_meeting_link` (pure, wasm) |
| `engine` | `Prefs.meeting_browser: Option<String>` (the program chosen; `None`: first found; the empty string: the default browser); command `view.meetingBrowser {"path": string \| ""}` |
| `ui-egui` | `Services::open_app_window(program, url)` (default: an error, so the caller opens the default browser), `Services::browsers() -> Vec<(String, String)>` (default: none); the "Join meeting" buttons push `Action::OpenMeeting(url)`, which picks app window or default browser; the Options › Applications card |
| `apps/quatrelle` | the desktop `Services`: finding the browsers (the pure function over `PATH` and the known paths) and launching |

## Never crash, privacy

- A browser that won't start, a path gone, a link refused: a notice, never a panic.
- Nothing is sent anywhere by Quatrelle; the browser opens the meeting link the user clicked.
- No new dependency.

## Tests

- `is_teams_link`: real Teams meeting links (with query strings, upper case, subdomains) and
  look-alikes (`teams.microsoft.com.evil.example`, `https://evil.example/teams.microsoft.com/meet/`,
  other paths of teams.microsoft.com).
- Finding browsers: a temporary `PATH` with some names present (and a non-executable one), the
  order kept; the known Windows and macOS paths through an injected "exists" function.
- The link check: refused with a space, a newline, `javascript:`, `file:`, too long.
- The UI: "Join meeting" on a Teams event asks `Services` for an app window with the link; on a
  Zoom event, `open_url`; with "The default browser" chosen, `open_url` for Teams too.
- Each test is checked to fail without its change.

## Out of scope (for now)

- A browser profile of Quatrelle's own (the usual profile is used).
- Watching the call window, or telling Teams anything.
- A1 and A2, each with its own spec.
