# Sender check: a known person's name, or signature, on someone else's address

Status: lot 5 of the upstream ZIDAN44 v0.31 round (`~/mailcraft/merge-tools/survey-v0.31.md`), issue
#29, in the 1.1 scope the owner set on 2026-11-04. Upstream's `14c0bee4`, `3a87dfa4`, `22796fda`,
`f421f17e`, `80fb57d9` and `06a623db` are the reference. Designed with the owner on 2026-11-04 as an
adaptation, not a port: upstream's engine part is built on its own address book, which the fork
replaced (the contacts field). Owner's choices: the whole known-name warning, the address after the
name and the External tag as options **off at first**; Exchange's directory **asked when a message
is opened**; and, the owner's own addition, **the signature compared with the sender**, by a fixed
rule. A language model is not used for any of it (see "Later").

## Why

The cheapest impersonation needs no look-alike domain. "Jane Director" <jane.director1984@gmail.com>
is a real address at a real mail service: the look-alike check (`model/src/lookalike.rs`, the same
file as upstream's before lot 5) lets it through, and the message list shows only the name, as
Outlook does. CEO fraud adds the real signature, copied from a real message, real address and phone
included, so the mail looks right to anyone who reads the name and the signature but not the address
it came from.

The fork warns today only about a look-alike domain (`paypa1.com`) and an address written into the
name, in the reading pane only. Exchange on premises puts no safety tip in the message, as Microsoft
365's Defender does.

## What the owner sees

**Reading pane and message window.** The red banner the look-alike warnings already use
(`reading.rs`, `sender_warning`), with two more texts, the second line unchanged ("N'ouvrez ni ses
liens ni ses pièces jointes…"):

- known name (upstream's text): "Careful: you know {name} as {known}, but this comes from {real}." /
  « Attention : vous connaissez {name} à l'adresse {known}, mais ceci vient de l'adresse {real}. »
- signature (new): "Careful: the signature gives {shown}, but this comes from {real}." /
  « Attention : la signature donne {shown}, mais ceci vient de l'adresse {real}. » `{shown}` is the
  address, or the domain for a website.

One banner at a time, the strongest: an address in the name, then a look-alike domain, then a known
name, then the signature. It follows what changes: it appears when the directory answers (a second
after the message, at most a few), and goes when the sender is saved as a contact.

**Message list.** A red ⚠ before the sender's name when the header alone tells (address in the name,
look-alike domain, known name); its hover and accessible text "Check who sent it" / « Vérifiez qui
l'a envoyé ». The signature warning is in the reading pane only: the list doesn't have the text. Two
options, both off at first so the list keeps Outlook's look:

- the address after the name, dimmer: "Jane Director <jane.director1984@gmail.com>"; where the line is
  short, cut to its domain ("<…@gmail.com>"), the name keeping at least nine twentieths of the line
  (`06a623db`); never for the user's own mail; the conversation count comes after the address, as
  upstream has it;
- the External tag: a small framed « Externe » before the time in the list, and beside the sender's
  name above the message, never wrapped (`80fb57d9`), never on the user's own mail; hover: « Cet
  expéditeur est extérieur à votre organisation. »

The table layout's From column shows the same marks (External left out where the column can't hold
the name and the cut address; the row is still read as External). The marks don't wait for a message
to be opened: when nothing has been gathered, the address book starts being read in slices, one per
frame, and the check is gathered once it's settled.

**File › Options › Safety**, after the three settings there:

- « Adresses des expéditeurs dans la liste des messages »: Jamais (default), Personnes que vous ne
  connaissez pas, Toujours;
- « Signaler le courrier venant de l'extérieur de votre organisation »: a switch, off;
- « Domaines de votre organisation », always shown (the signature rule's case a reads it with the
  switch off too), its note « Sert à l'étiquette Externe et à la vérification de la signature » :
  comma-separated; empty means the domains of the user's accounts with mail services left out, and
  the field says which those are (« Aucun : rien n'est signalé » when there are none).

**Commands** (upstream's): `view.senderAddresses` (`show`: `never` | `strangers` | `always`),
`view.externalTag` (`on`: bool; `domains`: a list or a text, at most 100). `list.messages` gives each row's marks
(`address`, `warning`, `external`); `message.read` gives the warning and `external`. The CLI prints
them.

## Who is known

The people the check knows, by name and by address:

- the user's accounts (address and user name, when it's an address), under the account's name;
- every card of every account's Contacts (`Pim::cards`), under the card's display name;
- everyone the user wrote to (`ContactBook::written`, the book's `sent > 0`), under the newest name
  the book has;
- everyone the directory has answered for this run (the people index, `mailboxes/people.rs`, at most
  `INDEX_MAX` = 30 000, in memory only).

They are added in that order but with the directory before the people written to, as upstream: a
warning calls a person by their card's name, then the directory's, then the name seen in mail. A
change of the listed organisation domains alone doesn't gather the people again.

Someone known **only** through the directory warns by name only when the sender writes from a mail
service (`is_free_mail`) or a look-alike of a known domain (theirs, or the look-alike check's): its
thousands of names make namesakes common (a customer « Jean Martin » beside a colleague). Known by a
card, written to, or the user's own account, the whole rule applies, a person known both ways
included.

The known domains (the look-alike check's) are the accounts' and the written-to people's, as today.
The organisation, for the External tag and the signature rule's case a: the domains listed in
Options, or else `own_domains` of the accounts (mail services left out: someone at gmail.com doesn't
work with everyone there).

## The directory question

When a message is opened (reading pane or window) and all of these hold, its sender's name is asked
of the directory, once, whether or not a warning was found (a colleague at a sister domain warned
for the organisation's site in her signature is known by address once it answers):

- the sender is neither the user nor known by address;
- the message isn't from a list or sent in bulk, unless it comes from a mail service (bulk mail is
  excused from the warning anyway: asking would be for nothing);
- the name is a person's (`name_key` is `Some`: not one word, not digits, not a team's word such as
  "IT Support");
- the user has an Exchange account (`directory_account`; Microsoft Graph and IMAP accounts have no
  directory here, and stay offline).

The name asked is the name as a person would type it (`lookup_name`, new in `people.rs`): titles,
suffixes and initials left out, invisible characters dropped, full-width and mathematical letters made
plain, letters of other scripts drawn like Latin ones made Latin, a digit inside a word of letters
made the letter it stands for; accents and case kept, at most `MAX_QUERY` (100) characters. It goes
through the queue Check Names uses (`search_directory(account, name, typed = false)`: one question at
a time per account, answers kept, a failed one asked again after 60 s at the earliest). A sender's
question still waiting gives its place to the next message's (arrowing through the list asks for the
last one), never takes the place of a Check Names question, and waits behind them. Nothing is
asked for the list, nor as mail arrives. The answer fills the people index; the check is gathered
again, and the open message's warning follows. An error is logged and shows nothing.

## The signature rule

**The author's own text** is the message's text (`Parsed::text`, an HTML message's converted text
included) up to the first of:

- a line `-----Original Message-----`, `-----Message d'origine-----` or any line of three or more
  dashes around words;
- Outlook's line of underscores (at least ten);
- a header block: a line starting `From:`, `De :`, `Von:`, `De:`, `Da:` or `Van:` followed within four
  lines by one starting with a short label: one to three words of letters, then a colon after a
  space, a no-break space or nothing (`Sent:`, `Envoyé :`, `Enviado el:`, `Para:`, `Enviada em:`,
  `A:`, `Aan:`, `Betreff:`…; case left aside; not a link's `https://`);
- a line ending `wrote:`, `a écrit :`, `schrieb:`, `escribió:`, `ha scritto:` or `schreef:`, or
  ending `:` with ` schrieb ` or ` schreef ` in it (Gmail's German and Dutch lines), or such a line
  starting `Am `/`Op ` cut in two, its next line ending `:`;
- Apple Mail's `Begin forwarded message:` or `Début du message réexpédié :`;
- a line starting `>`.

The marks are read through emphasis: the converted HTML writes Outlook's labels bold (`**De :**`), so
a leading `**`, a leading `*` stuck to a word and trailing `*` are left aside, a list's `* ` bullet
isn't.

**The signature** is what follows its last `-- ` line, or else its last 15 lines that aren't empty.
Only its short lines are read: an address or site with at most four other words on the line (a
token without a letter, such as a separator or a phone number's digits, isn't a word), so a sentence
naming a site isn't a signature. A line in parts between spaced `|`, `–`, `—`, `•` or `·` (a
signature on one line) counts the words of the part holding the address or site only. The converted HTML's link notes at the end of the text (`[1]: …`)
count only when their link is on a signature line. Read in it: addresses (`addr::is_valid_email`), `mailto:` links and websites (`www.…`, `http(s)://…`),
ten at most, each reduced to its domain where it's a site. A mail service's domain doesn't count, nor
a domain related to the sender's (`related`: the same, or one under the other), nor the user's own
addresses (their own message quoted where no quotation mark was found).

**It warns** (`SenderWarning::SignatureShowsOther { shown, real }`) for the first of:

- **a.** a domain of the organisation (as above), the message coming from outside it;
- **b.** an address the check knows exactly (a contact, someone written to, a person of the directory),
  the message coming from another;
- **c.** an address carrying the sender's name (its user part read as a name, `name_key` equal to the
  sender's), the sender writing from a mail service.

**It never warns** when the sender's address is known, for mail from a list or sent in bulk
(`Parsed::bulk` or the row's `bulk`) unless it comes from a mail service, nor for the user's own mail.

## Taken, rewritten, left out

- **Taken through `pick.sh`** (the model; same base here): `lookalike/people.rs` (`KnownPeople`,
  `name_key`, `SenderCheck`, `NameOfKnownPerson`), `lookalike/external.rs` (`is_free_mail`,
  `own_domains`, `clean_domain`, `is_external`), `22796fda`'s disguises (styled letters, a digit for a
  letter, titles), the property test and the `search` fuzz target's part, `ContactBook::written`,
  `Parsed::bulk` (`f421f17e`'s `mime` part), with their tests.
- **Rewritten** (the engine): upstream's `Senders` counts its address book again after each calendar
  or contacts sync and keeps a ten-minute cache (`FRESH_SECS`) to hide the hitch. Here the book is kept
  step by step and on disk, with a revision: the check is gathered again only when its stamp changes —
  the accounts (address, user name, name), `pim_revision`, `contacts_revision` (which the people index
  bumps too, `apply_directory` → `contacts.changed()`) and the listed organisation domains — never
  each frame, never by age. `pim_revision` moves for an appointment too: when only it moved, a print
  of the cards (their number, each one's name and addresses) decides whether to gather. Each gathering sets the open message's and every window's warning again.
  `known_domains` and `sender_warning` become views of it.
- **Adapted** (the interface): upstream's list row (⚠, address, External, the name's share), the
  reading pane's tag and the Options rows, in the fork's look; the 13 catalogues.
- **New here**: the directory question, `lookup_name`, the signature rule and its warning.
- **Left**: upstream's other pieces of the commits (Arabic interface parts, its own `opening.rs`
  rework around its book).

## Robustness

Names, addresses and text come from the sender: hostile.

- Everything is capped: names as upstream (`LONGEST_NAME`, `MOST_WORDS`, `MOST_PEOPLE` = 50 000), the
  text read for the signature 64 KiB, its lines, ten addresses or sites, the name asked 100 characters.
- No panics (the crate roots' `deny`), checked arithmetic, char-boundary slicing.
- A new fuzz target, `signature` (the reader of the author's own text and of the signature, then the
  rule), beside upstream's part of `search`; `fuzz/README.md` lists it.
- A property test: nothing after the first quotation mark changes the signature found.
- `every_command_survives_junk_params` runs the three new commands.
- Gathering the check is timed: at most 50 ms in a release build for 20 000 people written to and
  30 000 of the directory (the debug test's bound is 500 ms, best of three runs, as the calendar search
  test). If it can't, it's gathered in slices, as the book itself is read. The per-frame gathering
  waits while the address book is being read, and never loads it.

## Tests

- **Model**: upstream's name tests (disguises, titles, digits, lists, team names); each quotation mark
  in each language; the signature after `-- ` and without one; the three cases that warn and those
  that don't (list, known sender, a Gmail address in the signature, a third party's signature under
  `De :`, a related domain).
- **Engine**: who is known (account, card, written to, directory); the directory asked once, only for
  an unknown person's name with an Exchange account, never for the list or at arrival; the answer
  bringing the banner; a sender saved as a contact taking it away; the list's marks for each option;
  the user's own mail never marked; the timing bound.
- **Interface, headless** (`ui-egui/src/tests/reading.rs`): each banner; ⚠ in a row; the address cut
  to its domain with the name keeping its share; External on one line in a narrow pane; the Options
  rows; every catalogue has the new strings.

## Files

- `model/src/lookalike.rs` declares `people`, `external` and the new `signature`; `SenderWarning`
  gains `NameOfKnownPerson` (upstream) and `SignatureShowsOther`.
- `engine/src/session/opening/senders.rs` (new): the stamp, the gathering, the directory question, the
  list's marks; `opening.rs` keeps calling it.
- `engine/src/cmd/view.rs` and `session/prefs.rs`: the three settings (`view.rs` 340 lines, `prefs.rs`
  417: room enough).
- `ui-egui/src/reading.rs` is at 781 lines: the banner and the External tag move first into
  `reading/sender.rs`, then change there. `panes/list.rs` (651) takes the row's part.
- `FORK.md`: what was taken, rewritten and left; the survey's lot 5 marked done.

## Later (1.2)

The owner asked whether a language model could do this. Not as the check: the attacker writes the
message the model would read and can talk to it ("this message is genuine"), the identity question
needs the address book and the directory the model doesn't have, and every opened message would go
to the model's server. Planned for 1.2: an « Est-ce une arnaque ? » button beside Summarize, on
demand like the rest of the assistant, given the text without its hidden parts and what the rules
found; it can only add a caution (a company named in words, a job title asking for an urgent
transfer, a phone number not the contact's), never take a warning away or call a message safe.
