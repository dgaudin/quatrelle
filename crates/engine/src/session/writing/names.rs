//! Check Names (Ctrl+K), as Outlook's, for any contacts field. Each entry typed is looked at:
//! an address is checked here alone (one the book, the directory's answers or the room lists
//! know takes its name), a contact group's reference is kept, and a name is looked up among the
//! contacts and the people written to or heard from, then in the organisation's directory
//! (Exchange's, searched with the account's Contacts folder). One person found is written out
//! with their address, several are offered to choose from, and none is said.
//!
//! A field is known by its window and its name ([`FieldId`]), so several windows don't mix. The
//! message being written owns the text of its To, Cc, Bcc and Reply-To: what's resolved is
//! written back to it, now and as the directory answers. Any other field (a dialog's) hands its
//! text in and gets the rewrite back. Either way each name's outcome is kept by its field until
//! the field takes it ([`Session::take_people_outcomes`]).

use quatrelle_model::addr::{Accepts, group_ref, parse_group_ref, parse_one, split_entries, valid_item_id};
use quatrelle_model::{Addressee, AddresseeKind};

use super::*;

/// What Check Names says when a name is left as it was typed.
pub const NAMES_NOT_FOUND: &str = "Some names weren't found in your contacts or among the people you've written to";

/// The most names one field's check looks up (those past them are kept as typed).
const MAX_NAMES: usize = 500;

/// The most names kept waiting on the directory, and choices waiting (the oldest go first).
const MAX_PENDING: usize = 2_000;
const MAX_CHOICES: usize = 200;

/// The fields whose outcomes are kept; past them, the one recorded to longest ago is forgotten
/// (a window closed says so: [`Session::forget_people_window`]).
const OUTCOME_FIELDS: usize = 64;

/// Why a name given up while it waited is unreached: past [`MAX_PENDING`], or its account gone.
const TOO_MANY_WAITING: &str = "too many names waiting";
const ACCOUNT_GONE: &str = "the account was removed";

/// The message's own fields, in the order Check Names goes through them.
const COMPOSE_FIELDS: [&str; 4] = ["to", "cc", "bcc", "replyTo"];

/// A name a field waits on the directory for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NameCheck {
    field: FieldId,
    account: String,
    token: String,
    /// The kinds the field takes.
    kinds: Vec<AddresseeKind>,
}

/// Several people fit a name typed in a field: the one meant is chosen
/// ([`Session::people_choose`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NameChoice {
    pub field: FieldId,
    pub token: String,
    pub people: Vec<Addressee>,
}

/// What a name typed came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// One person (or list, room, contact group…) written out, or an address kept as typed
    /// (of an unknown kind when nothing here knows it).
    Found(Addressee),
    /// No one: left as typed.
    NotFound,
    /// A piece with `@` that isn't an address.
    NotAnAddress,
    /// The directory couldn't be asked (why): left as typed.
    Unreached(String),
    /// Several fit: a choice waits ([`Session::name_choice_for`]).
    Ambiguous,
}

/// A field checked: its text with what was found written out, how many names were, those left
/// as typed, and those waiting on the directory.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Resolved {
    pub text: String,
    pub resolved: usize,
    pub unresolved: Vec<String>,
    pub searching: Vec<String>,
}

/// What a name comes to with what is known here (and what the directory answered).
enum Settled {
    One(Addressee),
    Several(Vec<Addressee>),
    /// No one; why the directory couldn't be asked, if it couldn't.
    No(Option<String>),
}

impl Settled {
    fn of(mut people: Vec<Addressee>) -> Settled {
        match people.len() {
            0 => Settled::No(None),
            1 => people.pop().map_or(Settled::No(None), Settled::One),
            _ => Settled::Several(people),
        }
    }
}

/// What one entry of a field came to, and how it's written and counted.
#[derive(Clone)]
enum Step {
    /// Found, and written out.
    Written(Addressee),
    /// Kept as typed (a contact group's reference; an address, known or not), counted as
    /// resolved when something here knows it.
    Kept(Addressee, bool),
    /// Several fit: left as typed while a choice waits.
    Choice,
    /// Left as typed, and said: not found, not an address, the directory unreached.
    Left(Outcome),
    /// Left as typed while the directory is asked.
    Waiting,
    /// Past the names one check looks at: left as typed, not looked at.
    Past,
}

/// Entries as a field writes them: `a; b; `, or nothing.
fn joined(parts: &[String]) -> String {
    if parts.is_empty() { String::new() } else { format!("{}; ", parts.join("; ")) }
}

/// `field` with the name `token` written out as `with`; `None` when it isn't there any more.
fn replaced(field: &str, token: &str, with: &str) -> Option<String> {
    let mut done = false;
    let parts: Vec<String> = split_entries(field)
        .into_iter()
        .map(|t| {
            if !done && t == token {
                done = true;
                with.to_string()
            } else {
                t.to_string()
            }
        })
        .collect();
    done.then(|| joined(&parts))
}

/// A contact group's reference typed as an entry (`Team <group:ID>`, `<group:ID>`): its name
/// (empty when it has none) and its item id.
fn group_entry(token: &str) -> Option<(String, &str)> {
    if let (Some(lt), Some(gt)) = (token.rfind('<'), token.rfind('>'))
        && lt < gt
    {
        let id = parse_group_ref(token.get(lt + 1..gt)?)?;
        let name = token.get(..lt)?.trim().trim_matches('"').trim().to_string();
        return Some((name, id));
    }
    parse_group_ref(token).map(|id| (String::new(), id))
}

/// Of several that fit, those of the kinds the field takes, when any is left. One that fits
/// alone stays whatever it is: the field says why it doesn't take it.
fn prefer(mut people: Vec<Addressee>, kinds: &[AddresseeKind]) -> Vec<Addressee> {
    if people.len() > 1 && people.iter().any(|p| Accepts::taken_by(kinds, p.kind)) {
        people.retain(|p| Accepts::taken_by(kinds, p.kind));
    }
    people
}

impl Session {
    /// The people here a name fits: by name, or the start of an address, among the contacts,
    /// the people written to or heard from, and the user's own addresses.
    fn local_names(&mut self, token: &str) -> Vec<EmailAddress> {
        self.people.lookups = self.people.lookups.saturating_add(1);
        let lower = token.to_lowercase();
        let fits = |a: &EmailAddress| a.name.to_lowercase().contains(&lower) || a.email.to_lowercase().starts_with(&lower);
        let mut out: Vec<EmailAddress> = self.suggest_contacts(token, 10).into_iter().map(|c| c.address).filter(|a| fits(a)).collect();
        for a in self.own_addresses() {
            if fits(&a) && !out.iter().any(|x| x.email.eq_ignore_ascii_case(&a.email)) {
                out.push(a);
            }
        }
        out
    }

    /// [`Session::local_names`], each with what it is (`account`: whose rooms).
    fn local_people(&mut self, account: Option<&str>, token: &str) -> Vec<Addressee> {
        self.local_names(token)
            .into_iter()
            .map(|a| {
                let kind = self.kind_lower(account, &a.email.trim().to_ascii_lowercase());
                Addressee::new(a.name, a.email, kind)
            })
            .collect()
    }

    /// An address typed, with what it is and, when it has no name of its own, the one the room
    /// lists, the book (a name of one's own first) or the directory know it by, as
    /// [`fill_name`] fills one; and whether anything here knows it.
    fn known_address(&self, account: Option<&str>, typed: EmailAddress) -> (Addressee, bool) {
        let lower = typed.email.trim().to_ascii_lowercase();
        let kind = self.kind_lower(account, &lower);
        let room = match account {
            Some(a) => self.people.rooms.get(a).and_then(|r| r.get(&lower)),
            None => self.people.rooms.values().find_map(|r| r.get(&lower)),
        };
        let book = self.known_contact(&lower).map(|c| &c.address.name);
        let directory = self.directory_known(&lower);
        let known = kind != AddresseeKind::Unknown || directory.is_some();
        let name =
            [room, book, directory.map(|d| &d.name)].into_iter().flatten().find(|n| !bare_name(&EmailAddress::new(n.as_str(), lower.as_str())));
        let mut a = typed;
        if let Some(name) = name {
            fill_name(&mut a, &EmailAddress::new(name.as_str(), ""));
        }
        (Addressee::new(a.name, a.email, kind), known)
    }

    /// The text of a message's own field, when `field` is one: To, Cc, Bcc or Reply-To of a
    /// message being written (the current one or one set aside) whose id is its window.
    fn compose_field(&mut self, field: FieldId) -> Option<&mut String> {
        let c = self.compose.iter_mut().chain(self.composes.iter_mut()).find(|c| c.id == field.window)?;
        match field.name {
            "to" => Some(&mut c.to),
            "cc" => Some(&mut c.cc),
            "bcc" => Some(&mut c.bcc),
            "replyTo" => Some(&mut c.reply_to),
            _ => None,
        }
    }

    /// Check the names of a field. A message's own field (To, Cc, Bcc, Reply-To of the message
    /// being written whose id is the window) is checked in the message, and what's found is
    /// written back to it; any other field hands its `text` in, and only the answer carries the
    /// rewrite. Names nothing here settles wait on the directory of `account` (an Exchange one,
    /// else the first); the answers come as outcomes ([`Session::take_people_outcomes`]) and, in
    /// a message, into its text. `accepts`: of several that fit a name, the field's kinds first.
    pub fn people_resolve(&mut self, field: FieldId, account: Option<&str>, accepts: &Accepts, text: Option<&str>) -> Result<Resolved, CmdError> {
        self.people_resolve_kinds(field, account, accepts.kinds, text)
    }

    /// [`Session::people_resolve`] for the kinds a field takes, as a command gives them.
    pub(crate) fn people_resolve_kinds(
        &mut self,
        field: FieldId,
        account: Option<&str>,
        kinds: &[AddresseeKind],
        text: Option<&str>,
    ) -> Result<Resolved, CmdError> {
        let (r, not_found) = self.resolve_field(field, account, kinds, text)?;
        // Each name left is pointed out under its field once the cursor leaves it.
        if not_found {
            self.notice(NAMES_NOT_FOUND, true);
        }
        Ok(r)
    }

    /// A field checked, and whether a name was found nowhere (said once, by the caller).
    fn resolve_field(
        &mut self,
        field: FieldId,
        account: Option<&str>,
        kinds: &[AddresseeKind],
        text: Option<&str>,
    ) -> Result<(Resolved, bool), CmdError> {
        let typed = match self.compose_field(field) {
            Some(own) => own.clone(),
            None => text.map(str::to_string).ok_or_else(|| CmdError::Params("`text` is needed for a field that isn't a message's".into()))?,
        };
        let directory = self.directory_account(account);
        // Rooms are an Exchange account's: those of the directory asked.
        let rooms_of = directory.clone().or_else(|| account.map(str::to_string));
        let (mut r, mut not_found) = (Resolved::default(), false);
        let mut parts: Vec<String> = Vec::new();
        // A name typed again in the field is looked up once.
        let mut seen: HashMap<&str, Step> = HashMap::new();
        for (i, token) in split_entries(&typed).into_iter().enumerate() {
            let step = match seen.get(token) {
                _ if i >= MAX_NAMES => Step::Past,
                Some(step) => step.clone(),
                None => {
                    let step = self.resolve_entry(field, token, directory.as_deref(), rooms_of.as_deref(), kinds);
                    seen.insert(token, step.clone());
                    step
                }
            };
            let outcome = match step {
                Step::Written(who) => {
                    parts.push(who.text());
                    r.resolved += 1;
                    Outcome::Found(who)
                }
                Step::Kept(who, counted) => {
                    parts.push(token.to_string());
                    r.resolved += usize::from(counted);
                    Outcome::Found(who)
                }
                Step::Choice => {
                    parts.push(token.to_string());
                    Outcome::Ambiguous
                }
                Step::Left(why) => {
                    parts.push(token.to_string());
                    r.unresolved.push(token.to_string());
                    not_found |= why == Outcome::NotFound;
                    why
                }
                Step::Waiting => {
                    parts.push(token.to_string());
                    r.searching.push(token.to_string());
                    continue;
                }
                Step::Past => {
                    parts.push(token.to_string());
                    continue;
                }
            };
            self.record_outcome(field, token, outcome);
        }
        r.text = joined(&parts);
        if let Some(own) = self.compose_field(field) {
            own.clone_from(&r.text);
        }
        Ok((r, not_found))
    }

    /// One entry of a field: a contact group's reference is kept; an address is checked here
    /// alone; a name is looked up here, then (when nothing here settles it) in the directory,
    /// whose answer may already be in.
    fn resolve_entry(&mut self, field: FieldId, token: &str, directory: Option<&str>, rooms_of: Option<&str>, kinds: &[AddresseeKind]) -> Step {
        if let Some((name, id)) = group_entry(token) {
            let name = if name.is_empty() { self.directory_known(&group_ref(id)).map(|g| g.name.clone()).unwrap_or_default() } else { name };
            return Step::Kept(Addressee { item_id: id.to_string(), ..Addressee::new(name, "", AddresseeKind::ContactGroup) }, true);
        }
        // An address never needs the directory.
        if token.contains('@') {
            let Some(typed) = parse_one(token) else { return Step::Left(Outcome::NotAnAddress) };
            // The book names the people known here: read first (in one go when there's none
            // yet), as `addressee_kind` only reads it. A name reads it too (`local_names`).
            self.contacts_whole();
            let (who, known) = self.known_address(rooms_of, typed);
            return if known { Step::Written(who) } else { Step::Kept(who, false) };
        }
        let here = prefer(self.local_people(rooms_of, token), kinds);
        let ask = match (here.as_slice(), directory) {
            // Known here by its address alone, with a directory to ask: its full name from there
            // (issue #9), as Outlook checks the address list first.
            ([one], Some(_)) => bare_name(&one.email()),
            (_, Some(_)) => true,
            (_, None) => false,
        };
        let settled = match directory {
            Some(dir) if ask => {
                self.search_directory(dir, token, false);
                if self.directory_found(dir, token).is_none() || self.directory_searching(dir, token) {
                    self.wait_for(NameCheck { field, account: dir.to_string(), token: token.to_string(), kinds: kinds.to_vec() });
                    return Step::Waiting;
                }
                // Answered before: settled now.
                self.settle(here, dir, token, kinds)
            }
            _ => Settled::of(here),
        };
        match settled {
            Settled::One(who) => Step::Written(who),
            Settled::Several(people) => {
                self.offer_name(NameChoice { field, token: token.to_string(), people });
                Step::Choice
            }
            Settled::No(why) => Step::Left(why.map_or(Outcome::NotFound, Outcome::Unreached)),
        }
    }

    /// What a name comes to with the people here it fits (`people`) and what `account`'s
    /// directory answered: the people here first, a person the directory has too named by it
    /// when known here by an address alone, each address once and each contact group (no
    /// address) once by its item id.
    fn settle(&mut self, mut people: Vec<Addressee>, account: &str, token: &str, kinds: &[AddresseeKind]) -> Settled {
        let (answer, error) = self.directory_found(account, token).map(|f| (f.people.clone(), f.error.clone())).unwrap_or_default();
        for p in answer {
            if p.address.trim().is_empty() {
                // A contact group: one without a usable item id has nothing to write.
                if p.kind == AddresseeKind::ContactGroup
                    && valid_item_id(&p.item_id)
                    && !people.iter().any(|x| x.address.is_empty() && x.item_id == p.item_id)
                {
                    people.push(p);
                }
                continue;
            }
            match people.iter_mut().find(|x| x.address.eq_ignore_ascii_case(&p.address)) {
                // Known here by its address alone: the directory's full name for it.
                Some(known) => {
                    if bare_name(&known.email()) && !p.name.trim().is_empty() {
                        known.name = p.name;
                    }
                }
                // The room lists, and the mailboxes opened, know better than the directory.
                None => {
                    let kind = self.kind_lower(Some(account), &p.address.trim().to_ascii_lowercase());
                    people.push(Addressee { kind, ..p });
                }
            }
        }
        match Settled::of(prefer(people, kinds)) {
            Settled::No(_) => Settled::No(error),
            s => s,
        }
    }

    /// Wait on the directory for a name (once per field). Past [`MAX_PENDING`] the oldest is
    /// given up, and unreached: a name never settles silently.
    fn wait_for(&mut self, check: NameCheck) {
        let pending = &mut self.people.pending;
        if let Some(c) = pending.iter_mut().find(|c| c.field == check.field && c.account == check.account && c.token == check.token) {
            c.kinds = check.kinds;
            return;
        }
        pending.push(check);
        if pending.len() > MAX_PENDING {
            let old = pending.remove(0);
            self.record_outcome(old.field, &old.token, Outcome::Unreached(TOO_MANY_WAITING.into()));
        }
    }

    /// The directory answered `query` (lower case): the names waiting on it are settled with
    /// what it and the address book have, by field (a message's written into it).
    pub(crate) fn directory_checked(&mut self, account: &str, query: &str) {
        let (now, later): (Vec<NameCheck>, Vec<NameCheck>) =
            std::mem::take(&mut self.people.pending).into_iter().partition(|c| c.account == account && directory_key(&c.token) == query);
        self.people.pending = later;
        let mut missing = false;
        for check in now {
            let here = self.local_people(Some(account), &check.token);
            match self.settle(here, account, &check.token, &check.kinds) {
                Settled::One(who) => {
                    self.write_name(check.field, &check.token, &who.text());
                    self.record_outcome(check.field, &check.token, Outcome::Found(who));
                }
                Settled::Several(people) => {
                    self.record_outcome(check.field, &check.token, Outcome::Ambiguous);
                    self.offer_name(NameChoice { field: check.field, token: check.token, people });
                }
                Settled::No(None) => {
                    missing = true;
                    self.record_outcome(check.field, &check.token, Outcome::NotFound);
                }
                Settled::No(Some(why)) => self.record_outcome(check.field, &check.token, Outcome::Unreached(why)),
            }
        }
        if missing {
            self.notice(NAMES_NOT_FOUND, true);
        }
    }

    /// A window closed: its fields' waits, choices, outcomes and last suggestions go. A question
    /// already out stays (another field may wait on it too), but settles nothing there.
    pub fn forget_people_window(&mut self, window: u64) {
        let p = &mut self.people;
        p.pending.retain(|c| c.field.window != window);
        p.choices.retain(|c| c.field.window != window);
        p.outcomes.retain(|(f, _)| f.window != window);
        self.forget_window_suggestions(window);
    }

    /// An account is gone: the names waiting on its directory are given up, unreached.
    pub(crate) fn forget_name_checks(&mut self, account: &str) {
        let (gone, kept): (Vec<NameCheck>, Vec<NameCheck>) = std::mem::take(&mut self.people.pending).into_iter().partition(|c| c.account == account);
        self.people.pending = kept;
        for c in gone {
            self.record_outcome(c.field, &c.token, Outcome::Unreached(ACCOUNT_GONE.into()));
        }
    }

    /// Write the name `token` out as `with` in a message's own field (nothing for another
    /// field: its outcome carries it).
    fn write_name(&mut self, field: FieldId, token: &str, with: &str) -> bool {
        let Some(text) = self.compose_field(field) else { return false };
        match replaced(text, token, with) {
            Some(new) => {
                *text = new;
                true
            }
            None => false,
        }
    }

    /// Keep what a name came to until its field takes it (the newest for a name). The field
    /// goes last; past [`OUTCOME_FIELDS`], the one recorded to longest ago is forgotten.
    fn record_outcome(&mut self, field: FieldId, token: &str, outcome: Outcome) {
        let all = &mut self.people.outcomes;
        let mut list = all.iter().position(|(f, _)| *f == field).map(|at| all.remove(at).1).unwrap_or_default();
        list.retain(|(t, _)| t != token);
        list.push((token.to_string(), outcome));
        if list.len() > MAX_NAMES {
            list.remove(0);
        }
        all.push((field, list));
        if all.len() > OUTCOME_FIELDS {
            all.remove(0);
        }
    }

    /// What a field's names came to since it last asked, in the order they were settled.
    pub fn take_people_outcomes(&mut self, field: FieldId) -> Vec<(String, Outcome)> {
        let all = &mut self.people.outcomes;
        all.iter().position(|(f, _)| *f == field).map(|at| all.remove(at).1).unwrap_or_default()
    }

    /// How many of a field's names still wait on the directory.
    pub fn people_pending(&self, field: FieldId) -> usize {
        self.people.pending.iter().filter(|c| c.field == field).count()
    }

    fn offer_name(&mut self, choice: NameChoice) {
        let choices = &mut self.people.choices;
        choices.retain(|c| !(c.field == choice.field && c.token == choice.token));
        choices.push(choice);
        if choices.len() > MAX_CHOICES {
            choices.remove(0);
        }
    }

    /// A name several people fit in any field of a window, for the one meant to be chosen.
    pub fn name_choice(&self, window: u64) -> Option<&NameChoice> {
        self.people.choices.iter().find(|c| c.field.window == window)
    }

    /// A name several people fit in a field, for the one meant to be chosen.
    pub fn name_choice_for(&self, field: FieldId) -> Option<&NameChoice> {
        self.people.choices.iter().find(|c| c.field == field)
    }

    /// The person meant by the name `token` several fit in a field (`None`: left as typed). In
    /// a message's own field it's written into the text (`written`: it was still there); in any
    /// other, its outcome carries it. Its kind and item id are the choice's when not given.
    pub fn people_choose(&mut self, field: FieldId, token: &str, person: Option<Addressee>) -> CmdResult {
        let choices = &mut self.people.choices;
        let at = choices
            .iter()
            .position(|c| c.field == field && c.token == token)
            .ok_or_else(|| CmdError::Params(format!("no one to choose for `{token}`")))?;
        let choice = choices.remove(at);
        let Some(mut person) = person else { return Ok(serde_json::json!({ "token": token, "written": false })) };
        let offered = choice.people.iter().find(|p| {
            if person.address.is_empty() {
                p.address.is_empty() && p.item_id == person.item_id
            } else {
                p.address.eq_ignore_ascii_case(&person.address)
            }
        });
        if let Some(p) = offered {
            if person.kind == AddresseeKind::Unknown {
                person.kind = p.kind;
            }
            if person.name.trim().is_empty() {
                person.name.clone_from(&p.name);
            }
        }
        let own = self.compose_field(field).is_some();
        let written = if own { self.write_name(field, token, &person.text()) } else { true };
        self.record_outcome(field, token, Outcome::Found(person));
        Ok(serde_json::json!({ "token": token, "written": written }))
    }

    /// Outlook's Check Names on the message being written: its To, Cc, Bcc and Reply-To. The
    /// names it can't settle here wait on the directory (answered as `searching`); those left as
    /// typed are said.
    pub fn check_names(&mut self) -> CmdResult {
        let c = self.compose.as_ref().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
        let (id, account) = (c.id, c.account.clone());
        let (mut all, mut not_found) = (Resolved::default(), false);
        for name in COMPOSE_FIELDS {
            let (r, missing) = self.resolve_field(FieldId { window: id, name }, Some(&account), Accepts::RECIPIENTS.kinds, None)?;
            all.resolved += r.resolved;
            all.unresolved.extend(r.unresolved);
            all.searching.extend(r.searching);
            not_found |= missing;
        }
        // Each name left is pointed out under its field once the cursor leaves it.
        if not_found {
            self.notice(NAMES_NOT_FOUND, true);
        }
        Ok(serde_json::json!({ "resolved": all.resolved, "unresolved": all.unresolved, "searching": all.searching }))
    }

    /// The person meant by the name `token` in the message being written (`None`: left as
    /// typed), in whichever of its fields a choice waits for it.
    pub fn choose_name(&mut self, token: &str, person: Option<Addressee>) -> CmdResult {
        let id = self.compose.as_ref().map(|c| c.id).ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
        // The one the window shows ([`Session::name_choice`]): any of its fields.
        let field = self
            .people
            .choices
            .iter()
            .find(|c| c.field.window == id && c.token == token)
            .map(|c| c.field)
            .ok_or_else(|| CmdError::Params(format!("no one to choose for `{token}`")))?;
        self.people_choose(field, token, person)
    }
}
