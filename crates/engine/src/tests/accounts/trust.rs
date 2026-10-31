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

#[test]
fn a_hostile_server_cannot_queue_endless_questions() {
    let (mut s, id) = with_account("trust-flood");
    for n in 0..1000 {
        s.ask_trust_once(&id, &format!("host{n}.example"));
    }
    assert!(s.ask_trust.len() <= 200, "{}", s.ask_trust.len());
    assert!(!s.ask_trust.is_empty());
}
