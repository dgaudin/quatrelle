# Webex: a scheduled meeting for an appointment

Quatrelle's Webex Meeting button makes a real scheduled meeting on the user's Webex site, through
Webex's public REST API (`https://webexapis.com/v1/meetings`), signed in as the person using it.
The invitation carries the title, the times and the time zone of the appointment, and the link,
meeting number, password and video address Webex answers with; Quatrelle never builds a link
itself. The personal room stays, as an explicit choice in the button's menu.

Nothing in Quatrelle ships with a Webex registration: Webex's token request needs a client secret
even with PKCE, and a secret must not be built into an application. Each person or organisation
registers an integration of their own (step 1), and types its client id and secret into Options.

## 1. Registering the integration (once)

On [developer.webex.com](https://developer.webex.com), signed in with the Webex account that will
schedule the meetings.

1. **My Webex Apps › Create a New App › Create an Integration**.
   - *Name* and *Icon*: anything; people see them on the consent page.
   - *Redirect URI(s)*: `http://127.0.0.1:53683/`. Webex matches the address exactly, port
     included, so Quatrelle's sign-in listens on this port and no other (File › Options › Webex
     shows the address, with a Copy button).
   - *Scopes*: `meeting:schedules_read`, `meeting:schedules_write` and `spark:people_read`
     (the last one only names the signed-in account in Options). Quatrelle asks for exactly these.
2. **Add Integration**. Copy the **Client ID** and the **Client Secret**. The secret is shown once.
3. In Quatrelle, **File › Options › Webex** (Calendar):
   - *Site*: the Webex site, for example `example.webex.com`. Never built in.
   - *Client ID* and *Client secret*. The secret goes to the system keychain when the box is left
     and is never shown again; it is not written to any file or log.
   - **Sign in to Webex** opens the browser. Approve the permissions; Options then shows the
     account, with Sign out. The refresh token is kept in the keychain too, and the access token
     in memory only.

Quatrelle uses the expiries Webex returns (typically 14 days for the access token and 90 for
the refresh token). A refresh may return a new refresh token, which replaces the old one. If the
sign-in can't be renewed, the window says "Sign in to Webex again".

The Options page is not drawn in the browser build: there is no loopback sign-in there.

## 2. In the appointment window

The **Webex Meeting** button is a menu:

- **New Webex meeting** (a plain click does it): needs a title and a start, else it says so and
  sends nothing. Not signed in, or without a site, it opens Options at the Webex section.
  Otherwise the button shows it is working and Webex makes the meeting at once, as Outlook's
  scheduler does, so that the invitation can be read and edited before it is sent. On the
  answer:
  - the place is set to the meeting link, unless one was typed;
  - the notes gain, in the interface's language, the join link, the meeting number, the password
    and the video address, each as Webex gave it;
  - Join works, as for any meeting link.
- **Use my personal room**: what the button used to do. It writes the room typed in Options
  (*Webex personal room*) into the appointment and sends nothing to Webex.

No error ever falls back to the personal room silently. Errors show in the window and change
nothing in the appointment: 401 is retried once with a refreshed token, then "Sign in to Webex
again"; 403 is "Your Webex account can't schedule meetings on this site"; other 4xx and 5xx show
Webex's own message with the status.

## 3. What follows the appointment

The meeting's id is kept on the event, and Quatrelle alone follows it: a change made in another
calendar program or on a phone doesn't reach Webex.

- **Saving a changed appointment** (title, start, end, time zone, repeat) sends a `PATCH` with
  only what changed since the last send. If Webex answers with another link, the old one is
  replaced in the notes and the place, and the guests hear of it. The meeting number, password
  and video address never change this way: they are not sent.
- **Deleting or cancelling the appointment** sends a `DELETE`. A meeting already gone counts as
  deleted.
- **A repeat** is sent as the rule's `RRULE` in `recurrence`; when the repeat is removed,
  `"recurrence": null`. Webex refuses several days or dates in a monthly or yearly rule; its
  error is shown, and nothing is converted.
- **One occurrence of a series** is not the series: deleting or moving one sends nothing to
  Webex, because the id is the whole series'.
- **Whose events are followed.** Only an event the user organizes, or a lone appointment with no
  organizer, no guests and no answer of the user's. An event someone else organizes never
  sends anything to Webex, whatever id it carries, so a stranger's invitation can't make the
  user's token delete a meeting. The id is the organizer's and isn't written into an invitation.
- **Guests.** `sendEmail` is false on create, update and delete when the appointment has guests:
  the calendar server's invitation is the one they get, and Webex mails no second one.

## 4. Where the id lives

| Kept in | As |
|---|---|
| iCalendar (CalDAV, the local cache, `.ics`) | `X-QUATRELLE-WEBEX-ID` |
| Exchange, through EWS | an extended property named `WebexId` in a property set of Quatrelle's own, read with the appointment and written on create and update |
| Microsoft Graph | the same property as a `singleValueExtendedProperties` entry, asked for with each event |

A value that is not a plausible Webex id is dropped when read.

## 5. A meeting made and not kept

- **A window closed without saving** deletes the meeting it made. If the answer to the creation
  is still on its way, the meeting is deleted when it arrives.
- **Quatrelle quitting** with such a window open deletes its meeting too. Closing waits for the
  delete for at most 3 seconds.
- **A crash**, or a delete Webex refuses (signed out, network down), can leave a meeting on the
  site. It is harmless and visible there; delete it by hand.

## 6. What a real account must still confirm

The tests run against a fake Webex only; no real server was called. With an integration
registered, confirm once:

1. The browser sign-in works on port 53683, and Options shows the account.
2. **Create:** New Webex meeting returns a link, number, password and video address, and the
   meeting is on the site with the title, times and time zone.
3. **Update:** moving the appointment moves the meeting.
4. **Delete:** deleting the appointment removes the meeting.
5. Removing a repeat sends `"recurrence": null` and Webex accepts it.
6. An appointment with guests sends no second invitation from Webex.
7. On a real on-premises Exchange, the extended property `WebexId` is stored with the
   appointment and read back after a sync.
