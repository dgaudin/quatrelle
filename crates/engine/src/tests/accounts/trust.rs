//! The servers an account trusts: the three commands that record the user's answer, and the
//! questions a server's proposal raises (one per account and site while Quatrelle runs).

use super::*;

/// A session with one server account, and the account's id; its start-up job taken.
fn with_account(name: &str) -> (Session, String) {
    let mut s = Session::open(Some(temp(name)), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    (s, id)
}

fn trusted(s: &Session, id: &str) -> Vec<String> {
    s.account(id).unwrap().config.trusted_domains.clone()
}

fn restarts(s: &mut Session) -> usize {
    s.take_jobs().iter().filter(|j| matches!(j, Job::Start { .. })).count()
}

#[test]
fn trusting_a_domain_saves_it_and_restarts_the_account() {
    let (mut s, id) = with_account("trust-saves");
    // Its site, whatever was typed: a host, a URL, capitals, a trailing dot.
    let r = s.run("account.trustDomain", &json!({"account": id, "domain": "https://Api.Fastmail.com/jmap/session"})).unwrap();
    assert_eq!(r["trusted"], "fastmail.com");
    assert_eq!(trusted(&s, &id), ["fastmail.com"]);
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { account, .. }] if account.trusted_domains == ["fastmail.com"]));
    // Twice: kept once, and the account starts again all the same (something may be waiting).
    s.run("account.trustDomain", &json!({"account": id, "domain": "FASTMAIL.com."})).unwrap();
    assert_eq!(trusted(&s, &id), ["fastmail.com"]);
    assert_eq!(restarts(&mut s), 1);
    // Saved and read back.
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    let s2 = Session::open(Some(Store::open(&root).unwrap()), quatrelle_model::dates::Clock::new(NOW, 0));
    assert_eq!(trusted(&s2, &id), ["fastmail.com"]);
    // Removed: it starts again without it.
    let r = s.run("account.distrustDomain", &json!({"account": id, "domain": "fastmail.com"})).unwrap();
    assert_eq!(r["wasTrusted"], true);
    assert!(trusted(&s, &id).is_empty());
    assert_eq!(restarts(&mut s), 1);
    // Not there: nothing changes, nothing restarts, and it says so.
    let r = s.run("account.distrustDomain", &json!({"account": id, "domain": "fastmail.com"})).unwrap();
    assert_eq!(r["wasTrusted"], false);
    assert_eq!(restarts(&mut s), 0);
    // An account that doesn't exist.
    let e = s.run("account.trustDomain", &json!({"account": "nope", "domain": "fastmail.com"})).unwrap_err();
    assert!(e.to_string().contains("no account `nope`"), "{e}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn changing_the_allowed_domains_in_the_settings_restarts_the_account() {
    let (mut s, id) = with_account("trust-settings");
    let mut config = s.account(&id).unwrap().config.clone();
    config.trusted_domains.push("icloud.com".into());
    s.update_account(config, None).unwrap();
    assert_eq!(restarts(&mut s), 1);
}

#[test]
fn twenty_other_servers_at_most() {
    let (mut s, id) = with_account("trust-twenty");
    for n in 0..20 {
        s.run("account.trustDomain", &json!({"account": id, "domain": format!("server{n}.example")})).unwrap();
    }
    assert_eq!(trusted(&s, &id).len(), 20);
    s.take_jobs();
    let e = s.run("account.trustDomain", &json!({"account": id, "domain": "server20.example"})).unwrap_err();
    assert!(e.to_string().contains("at most 20 other servers"), "{e}");
    assert_eq!((trusted(&s, &id).len(), restarts(&mut s)), (20, 0), "refused: nothing changed");
    // One already there is no 21st.
    s.run("account.trustDomain", &json!({"account": id, "domain": "server3.example"})).unwrap();
    assert_eq!(trusted(&s, &id).len(), 20);
    // A hand-edited file with more is read as it is, and one can still be removed.
    s.accounts.iter_mut().find(|a| a.config.id == id).unwrap().config.trusted_domains.extend((20..25).map(|n| format!("server{n}.example")));
    s.run("account.distrustDomain", &json!({"account": id, "domain": "server24.example"})).unwrap();
    assert_eq!(trusted(&s, &id).len(), 24);
}

#[test]
fn not_a_domain_is_refused() {
    let (mut s, id) = with_account("trust-not-a-domain");
    for bad in [
        "",
        "  ",
        "nodot",
        "a b.example",
        "-a.example",
        "a-.example",
        "a..example",
        ".example",
        "ex_ample.com",
        "bücher.example",
        "[::1]",
        "https://",
        "a@",
        "http://x y.example/",
    ] {
        for cmd in ["account.trustDomain", "account.distrustDomain", "account.refuseDomain"] {
            let e = s.run(cmd, &json!({"account": id, "domain": bad})).unwrap_err();
            assert!(e.to_string().contains(&format!("not a domain: {}", bad)), "{cmd} {bad:?}: {e}");
        }
    }
    let long = format!("{}.example", "a".repeat(250));
    assert!(s.run("account.trustDomain", &json!({"account": id, "domain": long})).is_err(), "over 253 characters");
    let label = format!("{}.example", "a".repeat(64));
    assert!(s.run("account.trustDomain", &json!({"account": id, "domain": label})).is_err(), "a label over 63");
    // What is allowed: localhost, an address, a URL's host.
    for ok in ["localhost", "127.0.0.1", "https://exch01.corp.local:444/EWS/", "xn--bcher-kva.example"] {
        s.run("account.trustDomain", &json!({"account": id, "domain": ok})).unwrap();
    }
    assert_eq!(trusted(&s, &id), ["localhost", "127.0.0.1", "corp.local", "xn--bcher-kva.example"]);
    assert!(s.run("account.trustDomain", &json!({"account": id})).is_err(), "`domain` is required");
}

#[test]
fn a_refused_domain_is_not_asked_again_this_run() {
    let (mut s, id) = with_account("trust-refused");
    s.ask_trust_once(&id, "contoso.example");
    s.ask_trust_once(&id, "contoso.example");
    assert_eq!(s.ask_trust, [(id.clone(), "contoso.example".to_string())], "once per account and site");
    s.ask_trust_once(&id, "other.example");
    s.ask_trust_once("a2", "contoso.example");
    assert_eq!(s.ask_trust.len(), 3, "another site, another account: asked");
    // The answer, no: the notice carries the account's address, and the question is gone.
    s.ask_trust.clear();
    let r = s.run("account.refuseDomain", &json!({"account": id, "domain": "https://mail.contoso.example/x"})).unwrap();
    assert_eq!(r["refused"], "contoso.example");
    assert!(
        s.notices.iter().any(|n| n.text == "contoso.example wasn't allowed: zoe@example.org won't use it" && !n.error),
        "{:?}",
        s.notices.iter().map(|n| &n.text).collect::<Vec<_>>()
    );
    assert!(trusted(&s, &id).is_empty() && restarts(&mut s) == 0, "the account is left as it is");
    // Not asked again, even after the pair was forgotten as asked.
    s.ask_trust_once(&id, "contoso.example");
    assert!(s.ask_trust.is_empty());
    // Allowed after all (Add in the settings): the refusal is lifted.
    s.run("account.trustDomain", &json!({"account": id, "domain": "contoso.example"})).unwrap();
    s.run("account.distrustDomain", &json!({"account": id, "domain": "contoso.example"})).unwrap();
    s.ask_trust_once(&id, "contoso.example");
    assert_eq!(s.ask_trust.len(), 1);
}

#[test]
fn a_domain_removed_is_asked_again_in_the_same_run() {
    let (mut s, id) = with_account("trust-asked-again");
    s.ask_trust_once(&id, "icloud.com");
    assert_eq!(s.ask_trust.len(), 1);
    s.ask_trust.clear();
    s.run("account.trustDomain", &json!({"account": id, "domain": "icloud.com"})).unwrap();
    // A request already on its way when the answer came is asked about once more...
    s.ask_trust_once(&id, "icloud.com");
    s.ask_trust.clear();
    s.run("account.distrustDomain", &json!({"account": id, "domain": "icloud.com"})).unwrap();
    // ...and the pair is free again once the domain is removed.
    s.ask_trust_once(&id, "icloud.com");
    assert_eq!(s.ask_trust, [(id.clone(), "icloud.com".to_string())], "removed: asked again, or the account would stay silent");
}

/// The two homes saved in the account's calendars document, as a sync would have left them.
fn with_homes(s: &mut Session, id: &str, caldav: &str, carddav: &str) {
    let p = s.pim_mut(id);
    p.caldav_home = Some(caldav.into());
    p.carddav_home = Some(carddav.into());
    p.dav_checked = Some(NOW);
    s.write_pim(id);
}

/// The calendars document as the worker reads it: from the disk, not from the window's memory.
fn saved_pim(s: &Session, id: &str) -> quatrelle_model::pim::Pim {
    s.store.as_ref().unwrap().load_account_doc(id, quatrelle_model::pim::PIM_DOC).unwrap()
}

fn saved_ews(s: &Session, id: &str) -> String {
    s.store.as_ref().unwrap().load_account_doc(id, quatrelle_model::EWS_FOUND_DOC).unwrap()
}

fn save_ews(s: &Session, id: &str, url: &str) {
    s.store.as_ref().unwrap().save_account_doc(id, quatrelle_model::EWS_FOUND_DOC, url).unwrap();
}

/// Removed means removed: what was found on that site goes with it, or the saved URL would put
/// the site back among those the account trusts, and the password would go there unasked.
#[test]
fn removing_a_domain_forgets_what_was_found_there() {
    let (mut s, id) = with_account("trust-forgets");
    s.run("account.trustDomain", &json!({"account": id, "domain": "icloud.com"})).unwrap();
    with_homes(&mut s, &id, "https://p01-caldav.icloud.com/123/calendars/", "https://contacts.example.org/card/");
    save_ews(&s, &id, "https://ex.icloud.com/EWS/Exchange.asmx");
    s.take_jobs();

    s.run("account.distrustDomain", &json!({"account": id, "domain": "icloud.com"})).unwrap();
    assert_eq!(saved_ews(&s, &id), "", "ews.json on that site");
    let pim = saved_pim(&s, &id);
    assert_eq!(pim.caldav_home.as_deref(), Some(""), "the home on that site");
    assert_eq!(pim.carddav_home.as_deref(), Some("https://contacts.example.org/card/"), "another site's home stays");
    assert_eq!(pim.dav_checked, None, "looked for again at the next sync, so the question comes back");
    assert_eq!(s.account(&id).unwrap().config.withdrawn_domains, ["icloud.com"]);
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { account, .. }] if account.withdrawn_domains == ["icloud.com"]));
    // Withdrawn, the site is asked about again.
    s.ask_trust_once(&id, "icloud.com");
    assert_eq!(s.ask_trust, [(id.clone(), "icloud.com".to_string())]);
    // And the file says so across a restart.
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    let s2 = Session::open(Some(Store::open(&root).unwrap()), quatrelle_model::dates::Clock::new(NOW, 0));
    assert_eq!(s2.account(&id).unwrap().config.withdrawn_domains, ["icloud.com"]);
}

/// A sync already on its way when the domain was removed answers afterwards, with the home it
/// took from the document or found there: it isn't kept, and the look starts over.
#[test]
fn a_sync_on_its_way_brings_no_removed_home_back() {
    let (mut s, id) = with_account("trust-race");
    s.run("account.trustDomain", &json!({"account": id, "domain": "icloud.com"})).unwrap();
    with_homes(&mut s, &id, "https://p01-caldav.icloud.com/123/calendars/", "");
    s.run("account.distrustDomain", &json!({"account": id, "domain": "icloud.com"})).unwrap();

    let late = quatrelle_model::pim::DavSync {
        caldav_home: Some("https://p01-caldav.icloud.com/123/calendars/".into()),
        carddav_home: Some("https://contacts.example.org/card/".into()),
        checked: NOW + 5,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(late.clone()) });
    let pim = saved_pim(&s, &id);
    assert_eq!(pim.caldav_home.as_deref(), Some(""), "the removed site's home isn't kept");
    assert_eq!(s.pim_mut(&id).caldav_home.as_deref(), Some(""), "in memory neither");
    assert_eq!(s.pim_mut(&id).carddav_home.as_deref(), Some("https://contacts.example.org/card/"), "the other one is");
    assert_eq!(s.pim_mut(&id).dav_checked, None, "still to look for");

    // Allowed again, the same answer is kept: the user said yes.
    s.run("account.trustDomain", &json!({"account": id, "domain": "icloud.com"})).unwrap();
    assert!(s.account(&id).unwrap().config.withdrawn_domains.is_empty());
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(late) });
    assert_eq!(saved_pim(&s, &id).caldav_home.as_deref(), Some("https://p01-caldav.icloud.com/123/calendars/"));
}

/// The owner's on-premises Exchange: its address, on another domain than his mail's, is in
/// `ews.json` and nowhere else. Whatever else is removed, it isn't touched, and nothing is asked.
#[test]
fn removing_another_domain_leaves_the_exchange_address_alone() {
    let (mut s, id) = with_account("trust-owner");
    s.accounts.iter_mut().find(|a| a.config.id == id).unwrap().config.ews = Some(quatrelle_model::EWS_AUTODISCOVER.into());
    let url = "https://exch01.corp.local/EWS/Exchange.asmx";
    save_ews(&s, &id, url);
    s.run("account.trustDomain", &json!({"account": id, "domain": "icloud.com"})).unwrap();
    s.run("account.distrustDomain", &json!({"account": id, "domain": "icloud.com"})).unwrap();
    assert_eq!(saved_ews(&s, &id), url);
    assert_eq!(s.account(&id).unwrap().config.withdrawn_domains, ["icloud.com"]);
    let r = s.run("account.distrustDomain", &json!({"account": id, "domain": "other.example"})).unwrap();
    assert_eq!(r["wasTrusted"], false);
    assert_eq!(saved_ews(&s, &id), url);
    assert_eq!(s.account(&id).unwrap().config.withdrawn_domains, ["icloud.com"], "nothing to withdraw");
    assert!(s.ask_trust.is_empty());
}

/// A site that stays trusted (the account's own server is on it): nothing to forget or withdraw.
#[test]
fn a_site_the_account_still_trusts_keeps_what_was_found() {
    let (mut s, id) = with_account("trust-own-site");
    let own = "https://dav.example.org/cal/";
    s.run("account.trustDomain", &json!({"account": id, "domain": "example.org"})).unwrap();
    with_homes(&mut s, &id, own, "");
    let r = s.run("account.distrustDomain", &json!({"account": id, "domain": "example.org"})).unwrap();
    assert_eq!(r["wasTrusted"], true);
    assert_eq!(saved_pim(&s, &id).caldav_home.as_deref(), Some(own));
    assert_eq!(saved_pim(&s, &id).dav_checked, Some(NOW));
    assert!(s.account(&id).unwrap().config.withdrawn_domains.is_empty());
}

/// A saved URL is all that makes this site trusted: named, it goes too, and the settings list
/// nothing for it to be removed from.
#[test]
fn a_site_trusted_only_through_a_saved_url_can_be_removed() {
    let (mut s, id) = with_account("trust-saved-only");
    with_homes(&mut s, &id, "https://caldav.other.example/cal/", "");
    let r = s.run("account.distrustDomain", &json!({"account": id, "domain": "other.example"})).unwrap();
    assert_eq!(r["wasTrusted"], true);
    assert_eq!(saved_pim(&s, &id).caldav_home.as_deref(), Some(""));
    assert_eq!(s.account(&id).unwrap().config.withdrawn_domains, ["other.example"]);
    assert_eq!(restarts(&mut s), 1);
}

#[test]
fn a_hostile_server_cannot_queue_endless_questions() {
    let (mut s, id) = with_account("trust-flood");
    for n in 0..1000 {
        s.ask_trust_once(&id, &format!("host{n}.example"));
    }
    assert!(s.ask_trust.len() <= 200, "{}", s.ask_trust.len());
    assert!(!s.ask_trust.is_empty());
}

#[test]
fn a_site_with_junk_in_it_is_not_asked() {
    let (mut s, id) = with_account("trust-junk");
    for junk in ["", "a\nb.example", "a b.example", "\u{7}"] {
        s.ask_trust_once(&id, junk);
    }
    assert!(s.ask_trust.is_empty(), "neither queued nor logged");
}

#[test]
fn a_status_needing_trust_asks_once() {
    let (mut s, id) = with_account("trust-status");
    let needs = |s: &mut Session, site: &str| s.apply(Event::Status { account: id.clone(), status: SyncStatus::NeedsTrust { site: site.into() } });
    needs(&mut s, "office365.com");
    assert_eq!(s.ask_trust, [(id.clone(), "office365.com".to_string())]);
    assert!(matches!(&s.account(&id).unwrap().status, SyncStatus::NeedsTrust { site } if site == "office365.com"), "shown in the account's status");
    // The worker says it once per connection try; the question is asked once while Quatrelle runs.
    s.ask_trust.clear();
    needs(&mut s, "office365.com");
    assert!(s.ask_trust.is_empty(), "once per account and site");
    // The next site of an Exchange hybrid is another question.
    needs(&mut s, "onmicrosoft.com");
    assert_eq!(s.ask_trust, [(id.clone(), "onmicrosoft.com".to_string())]);
    // An account that isn't there asks nothing.
    s.apply(Event::Status { account: "nope".into(), status: SyncStatus::NeedsTrust { site: "x.example".into() } });
    assert_eq!(s.ask_trust.len(), 1);
}

#[test]
fn a_status_needing_trust_resets_the_rules_state_as_a_password_does() {
    let (mut s, id) = with_account("trust-rules-state");
    let reads = |s: &mut Session| s.take_jobs().iter().filter(|j| matches!(j, Job::GetServerRules { .. })).count();
    let status = |s: &mut Session, status: SyncStatus| s.apply(Event::Status { account: id.clone(), status });
    status(&mut s, SyncStatus::Online { at: NOW });
    assert_eq!(reads(&mut s), 1, "read on coming online");
    status(&mut s, SyncStatus::Online { at: NOW + 1 });
    assert_eq!(reads(&mut s), 0, "not at each look for new mail");
    status(&mut s, SyncStatus::NeedsTrust { site: "office365.com".into() });
    status(&mut s, SyncStatus::Online { at: NOW + 2 });
    assert_eq!(reads(&mut s), 1, "the server was lost: its rules are read again once it is back");
}

#[test]
fn trust_asked_during_a_sync_asks_once() {
    let (mut s, id) = with_account("trust-dav");
    s.apply(Event::Status { account: id.clone(), status: SyncStatus::Online { at: NOW } });
    let asked = |s: &mut Session| s.apply(Event::TrustAsked { account: id.clone(), site: "icloud.com".into() });
    asked(&mut s);
    assert_eq!(s.ask_trust, [(id.clone(), "icloud.com".to_string())]);
    assert!(matches!(s.account(&id).unwrap().status, SyncStatus::Online { .. }), "the account stays online: the mail goes on");
    s.ask_trust.clear();
    asked(&mut s);
    assert!(s.ask_trust.is_empty(), "once per account and site");
    // After the user said no, the network may say it again: no more questions.
    s.run("account.refuseDomain", &json!({"account": id, "domain": "icloud.com"})).unwrap();
    asked(&mut s);
    assert!(s.ask_trust.is_empty());
}
