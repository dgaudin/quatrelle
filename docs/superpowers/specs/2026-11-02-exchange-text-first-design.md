# Exchange: a big message's text first, its attachments after

Status: the owner approved this design and its one departure from Cached Exchange Mode on 2026-11-02
("oui"), after reading it in French. It ports upstream's issue #24 (`59515345`, `42c45848`), which was only
tried against grommunio. Nothing here has been tried on a real Exchange server yet: the code was read, and
Microsoft's `FieldURI` list was checked. The test build at the end is the first real trial.

## Why

Opening a message that isn't on this computer waits for its whole source, attachments included: `GetItem`
with `IncludeMimeContent` (`Client::download`, `ews/mailbox.rs`). A 20 MB message at 2 MB/s shows nothing
for 20 seconds.

On the owner's mailbox this happens with what isn't downloaded ahead: public folders (never kept by
default), a big message that has just arrived, a shared mailbox that isn't kept.

Exchange and Graph can give a message's body and the list of its attachments apart from its source. So the
text can show at once and the whole message can follow.

## Cached Exchange Mode, and the departure the owner accepted

As far as we know (not checked by trial), Outlook in Cached Exchange Mode fetches a whole item when it is
opened and shows it when it has it. Here the text shows first. The end state is the same: the whole message
in the cache, indexed, as if it had been downloaded normally. Only the order of arrival differs. The owner
accepted this on 2026-11-02, as their rule on Cached Exchange Mode requires of any departure.

## What the owner sees

While the message is on its way, the reading pane shows:

- the header and the text;
- each attachment with its name and size, and a spinner with "Downloading the attachments…";
- a spinner where each embedded picture will go.

"Save all…" is hidden until the whole message is here.

When the whole message arrives, it replaces this in place. The scroll position stays. Only then is it kept
in the cache and indexed for search.

If another message is opened meanwhile, the download goes on. One whole message downloads at a time, and
the last one opened goes first.

### What can be done meanwhile

- **Reply and Reply all** work at once, unless what they need isn't known yet. Then they say the message is
  still coming, as below. They wait when:
  - the message has embedded pictures, which the quote would lose;
  - someone they would write to has no address yet. A colleague's address can be Exchange-internal until
    the whole message comes. The sender's is read from `0x5D01`, but To and Cc have no such property.
- **Forward, Forward as attachment, open, save, save all, preview an attachment, Save as, Save as PDF,
  print, message headers** say so too: "its attachments are still downloading: a moment", translated. Buttons
  stay clickable, so that the reason is seen. A greyed button wouldn't say why.

### When something fails

- **The text can't be had** (any error, no body, a server that refuses the request): the message comes the
  way it does today, with one line in the log. The user only sees it take longer.
- **The whole message can't be had:** the text stays on screen, with
  "The attachments couldn't be downloaded: {error}" and the existing "Try again" button.

## When it applies

`Session::fetch_job` decides, so "Download all" and the CLI, which push `Job::FetchBody` directly, stay
whole. All of these must hold:

- the account is EWS or Graph;
- the message isn't older mail read online (`is_older`), which stays on `Job::ReadOnline`;
- its list entry is known (`Session::summary`) and has `has_attachments`;
- EWS only: `size >= TEXT_FIRST_OVER` (1 MiB, upstream's value). Graph gives no size, so any message with
  attachments qualifies.

The worker then leaves these to the whole-message path, because their real content isn't in the body:

- S/MIME: item class `IPM.Note.SMIME*`, or an attachment that is `application/pkcs7-mime`,
  `application/x-pkcs7-mime`, `multipart/signed`, or named `smime.p7m`. The class settles it on EWS; Graph
  has only the attachment test;
- invitations: item class `IPM.Schedule.Meeting.*` on EWS, `#microsoft.graph.eventMessage` on Graph;
- an empty body, or no attachment listed.

## What differs from upstream

1. **The field list.** Upstream asks for `message:InReplyTo`, which doesn't exist: Microsoft's name is
   `item:InReplyTo`. Exchange validates `FieldURI` against its schema, so we expect the whole request to be
   refused, and the feature would silently never work. We ask for `HEADER_FIELDS` (`ews.rs`), already proven
   on the owner's server by every sync, plus `item:Body`, `item:Attachments`, `item:ItemClass` and
   `item:Importance`, all in Microsoft's list. No `BodyType`, as upstream: grommunio sends the whole source
   with one.

   We don't ask for `In-Reply-To` or `References`. They would only serve one case, and each new field risks
   the whole request. So a reply written during the text phase has `In-Reply-To` built from the message's
   `Message-ID` and a short `References`.
2. **The sender.** Upstream's `mailbox()` drops any address without `@`, so a colleague's mail, whose
   address is Exchange-internal, shows no sender. We read addresses as the list does: `address()` for the
   name and `0x5D01` for the sender's SMTP address, as in `header_block`.
3. **Where the whole message downloads.** Upstream uses the account's second connection. Here that doesn't
   work:
   - it has no numbering on Exchange (`with_numbering(false)`), and reloading 550,000 ids per message is out;
   - it doesn't know the routes to public folders and shared mailboxes;
   - a long download would hold up sending, rooms and free/busy, which is what that connection is for.

   Instead the main connection hands the work to a thread of its own, with a `Client::sibling()`. That is
   how `read_ticked` already reads calendars, on the owner's server.
4. **No marker header.** Upstream marks the built message with `X-MailCraft-Text-First`. A received message
   could carry that header and would then stay "still coming" forever, and never be indexed. We use an event
   of its own, `Event::TextFirst`, so nothing in a message can claim to be one.

## Design

### Model

- `Job::FetchTextFirst { key }`, answered by `Event::TextFirst { key, raw }`, then `Event::Body` or
  `Event::BodyFailed`. A fallback answers with `Event::Body` directly.
- Add it to `Job::account()`, to the pool's offline arm next to `Job::FetchBody` (same answer), and to
  `between_folders` and `jumps_queue`. In `push_job`, a request already waiting for the same key is dropped.
- The web app's backend treats it as `Job::FetchBody`.

### `mime`

Ported from upstream without the marker: `PendingFile { name, mime, size, content_id, inline }`,
`mime::text_first(&Draft, from, date, message_id, &[PendingFile])`, and `shown_size`, which reads an RFC 2183
`size` parameter on an empty part so that a pending file has a size.

Everything in it comes from the server, so it is hostile. CR and LF in a subject, an address, a file name, a
content id or a media type must not add a header. A media type that isn't one becomes
`application/octet-stream`. At most 500 attachments are listed, on EWS and Graph alike. The built message
goes through `mime::parse` as any other, so remote content stays blocked.

### EWS (`exchange/ews/text_first.rs`)

`Client::download_text(id)` sends the `GetItem` above and builds the message from the answer.
`ews::text_first(c, st, store, key)` takes the id through `server_id`, so it finds a message the numbering
lost, as `fetch_body` does. It returns the built message and the item id, or `None`.

### Graph (`exchange/graph/text_first.rs`)

Ported, with these changes:

- `who(st.owner(&key.folder))` instead of `/me`, so another person's mailbox works;
- `isInline` is respected. `contentId` isn't asked for, since a derived type's field needs its own
  `$select`, and it isn't needed: a picture that can't be found yet shows the spinner;
- `@odata.type`, which Graph writes unasked on a derived type, tells an invitation (it contains
  `eventMessage`) and an attached message (`itemAttachment`). Should a server leave it out, an invitation
  shows its text first and its card when the whole message comes: nothing is lost;
- `Client` gets a `sibling()`: its base address and its current token.

In this lot, Graph is tried against the fake server only.

### The worker: the text (`worker/rest.rs`)

`Worker::fetch_text_first(key)` on the main connection:

1. A body already in the store: `Event::Body`, as `fetch` does.
2. Connect if needed. Anything but EWS or Graph: `fetch(key)`.
3. Ask for the text. On `None` or an error, log it and call `fetch(key)`.
4. Save the numbering if it changed, as `fetch` does.
5. Emit `Event::TextFirst`, **then** hand the rest over. In that order, the text can never arrive after the
   whole message.

A request the server refuses for its shape, not a network failure, stops the worker asking for the rest of
the session, with one warning in the log. A server that can't answer would otherwise cost a wasted request
each time a big message is opened.

### The worker: the rest (`worker/rest.rs`)

A small shared queue: the keys waiting, each with its item id and its own connection copy taken at that
moment (`Client::sibling()`, which carries the routes, and on Graph the current token), the key being
downloaded, and whether the thread is up.

- **New work:** a key that's running needs nothing. One already waiting moves to the front. A new one goes
  to the front. The thread starts if it isn't up. If it can't start, the main connection downloads the
  message itself.
- **The thread:** takes the first key; skips it if the store has the body now (its `Event::Body` has gone
  out already, from "Download all" for example); downloads it; writes it with `write_body`; emits `Event::Body`,
  or `Event::BodyFailed` on an error. It ends when the queue is empty.
- **It never touches the folder's list.** It writes the source only, as `download_ahead` does, so it can't
  race the sync over `save_folder`. For the same reason no `Event::FolderChanged`.
- **Stopping:** when the worker ends (closing, working offline, a removed account), a shared flag says so.
  The thread then writes nothing and starts no other message. A download in progress isn't waited for, and
  the worker doesn't join the thread: closing Quatrelle must never wait on a 200 MB message, and a removed
  account must not get files written back.
- **Nothing waits for ever:** every key left at the stop gets `Event::BodyFailed`, the ones waiting from the
  worker as it ends, the one running from the thread when its call returns. Otherwise a message would keep
  its spinner after the account went offline.

An authentication failure here isn't the main connection's failure: it reports `Event::BodyFailed` only, and
the main connection meets the same problem on its own.

### The engine

- `Reading.text_first: bool`, for the reading pane and for windows.
- `Event::TextFirst` parses `raw` the way `show_body` does (`parse_for_reading`, sender warning,
  `request_pictures`) and fills **only a reading or window whose `parsed` is empty**, setting `text_first`.
  So it can never overwrite a whole message. It is not stored, not indexed, and `downloaded()` isn't called.
- `show_body` also replaces a window with `text_first`, not only an empty one, and clears the flag.
- `Event::BodyFailed` also reaches a window with `text_first`.
- A helper refuses the commands above with `CmdError::Failed(STILL_COMING)`.

### The reading pane

- `text_first` shows the spinner and "Downloading the attachments…" where "Save all…" would be.
- `r.error` with `parsed` set shows the failure line and "Try again" (`Action::Open`), which today appear
  only when `parsed` is empty.
- Nothing fetched during the text phase is kept as an absence: `st.previews`, the attachment preview and the
  picture cache stay empty for that message. The attachments' indexes can differ in the whole message, so
  nothing keyed by `(key, index)` may survive it.
- A chip clicked during the text phase opens no preview: it gives the message above.
- The scroll area's id doesn't depend on `parsed`, so the position stays with no extra work.

### The log

One line per message: its size, how long the text took, how long the rest took. That line is how the test
build proves the feature works on the owner's server.

## A small fix in the same lot

`Job::ReadOnline` joins `between_folders` and `jumps_queue`: old mail opened during a full sync waits for the
end of that sync today.

## Known limits

- **The text goes through the main connection**, the only one with the numbering. If it is in the middle of
  a long call, a big message of its own for instance, the text waits for that call. Upstream has the same
  limit.
- **One download at a time:** a very big message holds up the next one's attachments, not its text.
- **The list line:** a message that came this way doesn't get its list preview from its source, like any
  message downloaded ahead.
- **Where Exchange says a message has no attachments,** which it may for embedded pictures alone, the message
  comes whole, as today.
- **Mail older than what's kept** stays out of this lot. By the code's reading, its attachments can't be
  opened and its embedded pictures don't show, since nothing keeps its source. That is checked for real at the
  end of this lot, and fixed before the calendar features if confirmed, reliability coming first.

## Tests

- **`mime`:** build and parse back: subject, addresses, sizes, pending files, embedded pictures. A proptest
  for CR and LF, and for hostile names.
- **EWS reading:** a realistic `GetItem` answer with a colleague (`RoutingType` EX, `0x5D01`), a file, an
  embedded picture and an attached message (`ItemAttachment`). S/MIME and invitation classes give no text
  first.
- **The fake EWS server refuses any `FieldURI` outside Microsoft's list**, as Exchange does. This is the test
  upstream's field would have failed.
- **Worker, with slow sources on the fake servers:**
  - the text arrives before the whole message;
  - another message opened meanwhile shows at once;
  - the last opened goes first, and the same key isn't downloaded twice;
  - sending isn't held up;
  - a public folder and a shared mailbox work, their routes carried by `sibling()`;
  - a refused request falls back, and isn't asked again;
  - a failed download gives `Event::BodyFailed`;
  - nothing is written after the stop, and every key left gets `Event::BodyFailed`;
  - the folder's list isn't rewritten.
- **Engine:** text then whole, in the pane and in a window; `Event::TextFirst` doesn't overwrite a whole
  message, doesn't store and doesn't index; every command listed above; Reply's two waiting cases; "Download
  all" stays whole.
- **Interface:** spinner and label, pictures waiting then shown, failure with "Try again", every catalog
  translated.
- **A hostile-input check:** a fuzz target for the two new builders, as `CLAUDE.md` asks of a new parser of
  server input.
- **`Job::ReadOnline`** jumps the queue.

Then a test build. On the owner's server: open a big message and a public folder's message, and read the log
line. That is the only real proof of this lot.

## Out of scope

- Mail older than what's kept (see Known limits).
- A size or attachment count on Graph, which it doesn't give.
- Downloading a single attachment on demand. The whole message still comes, as Cached Exchange Mode keeps it.

## Settled during the work

What changed from the text above:

- A name alone is read only through `parse_text_first`; `parse` keeps addresses only, as before.
- The thread finding a body already stored emits `Event::Body`, it doesn't skip silently.
- Only `NetError::Auth` on the text skips the whole-message fallback, and a failed connect reports as `fetch` does.
- The stop flag is set by the pool as the account stops.
- Reply's guard checks the draft it built, not the message's `from` and `reply_to`.
- After a failure, commands say the failure instead of "still downloading".
