# Quatrelle and MailCraft

Quatrelle started as a fork of [ZIDAN44/mailcraft](https://github.com/ZIDAN44/mailcraft), the
MailCraft email client of the ArtCraft Team and the MailCraft contributors, from its commit
`7d16bb1` ("Docs: v0.15"), and took the name Quatrelle. It keeps MailCraft's engine, its
conventions (see `CLAUDE.md` and `AGENTS.md`) and its licence, MIT OR Apache-2.0, and goes its own
way in a few places.

## Where it differs

- **It looks like Outlook and fits the desktop.** The interface icons are Microsoft's
  [Fluent UI System Icons](https://github.com/microsoft/fluentui-system-icons), published under the
  MIT licence: the one exception to the rule of no Microsoft artwork (`CLAUDE.md`, "Assets"; each
  file has its row in `ATTRIBUTION.md`). The window follows the desktop: its title bar, light or
  dark, the accent colour and the system's fonts. The app icon and colour stay Quatrelle's own.
- **Exchange as Outlook uses it.** An on-premises Exchange (EWS) is kept on this computer the way
  Outlook's Cached Exchange Mode keeps it; shared mailboxes and calendars, public folders, the
  server's rules, the online archive. Exchange Online goes through Microsoft Graph.
- **Calendar, People, Tasks and Notes** with Exchange, CalDAV and CardDAV, meeting invitations,
  Teams and Webex meetings.
- **Outlook data files**: `.pst` in and out, `.msg` files.

## Code taken from MailCraft

MailCraft's later changes are still looked at and taken when they fit. A commit taken as it is
keeps its author and says `(from upstream <commit>)`; one that had to be adapted is committed by
Didier Gaudin and names the commit it comes from. Each source file names its authors, as `NOTICE`
and the "Authors" part of `CLAUDE.md` say.

## Releases

What each version brought is in its notes, on the
[releases page](https://github.com/dgaudin/quatrelle/releases):

- **1.0.0**: the first stable release: IMAP, SMTP, POP, JMAP, Exchange (EWS) and Microsoft 365;
  mail, calendar, people, tasks and notes; S/MIME; the optional assistant; 13 languages.
- **1.1.0**: the sender check, passwords that never follow a redirect, Exchange's big messages
  opened text first, the contacts field, a calendar closer to Outlook's, Teams and Webex meetings.
- **1.2.0**: a reading pane closer to Outlook's, `.pst` import and export, OpenPGP, delete for good
  and recover deleted items, Exchange at the scale of a very big mailbox.
- **1.3.0**: archiving IMAP mail on this computer, Quick Steps, the scam check, IMAP that rides out
  a server's passing refusals.
- **1.4.0**: Exchange's online archive, a data-file import you can follow and stop, notes in a
  window of their own written in Markdown, a selection through a reply's original.

What comes next is in `ROADMAP.md`; what isn't done yet, in `docs/gaps.md`.
