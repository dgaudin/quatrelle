//! Tests of the Webex client, over the fake server.

use serde_json::json;

use super::*;
use crate::fake_webex::{self, Data, REVOKED, SECRET, Server};
use crate::worker::MemoryCredentials;

const SITE: &str = "acme.webex.com";

fn utc_time(text: &str) -> i64 {
    quatrelle_jmap::utc_date(text).unwrap()
}

fn fields() -> WebexFields {
    WebexFields {
        title: "Budget".into(),
        start: utc_time("2026-11-02T09:00:00Z"),
        end: utc_time("2026-11-02T10:00:00Z"),
        timezone: "Europe/Paris".into(),
        recurrence: None,
    }
}

/// A keychain holding the client secret and this refresh token.
fn keychain(refresh: Option<&str>) -> MemoryCredentials {
    let creds = MemoryCredentials::default();
    creds.set_secret(&keyring(), SECRET_KEY, SECRET).unwrap();
    if let Some(r) = refresh {
        creds.set_secret(&keyring(), REFRESH_KEY, r).unwrap();
    }
    creds
}

/// A client of the fake.
fn pointed_at(server: &Server, creds: Arc<dyn Credentials>) -> Webex {
    let mut w = Webex::new("C1", creds);
    w.api = server.base.clone();
    w.token_url = format!("{}/access_token", server.base);
    w
}

/// A client of the fake, whose keychain holds the client secret and this refresh token.
fn client(server: &Server, refresh: Option<&str>) -> (Webex, Arc<MemoryCredentials>) {
    let creds = Arc::new(keychain(refresh));
    (pointed_at(server, creds.clone()), creds)
}

/// A client signed in, its first token taken already: the fake has seen nothing yet.
fn signed_in(data: Data) -> (Webex, Server, Arc<MemoryCredentials>) {
    let server = fake_webex::serve(data);
    let (mut w, creds) = client(&server, Some("r1"));
    w.token().unwrap();
    server.data.lock().unwrap().calls.clear();
    (w, server, creds)
}

/// Signed in, with the meeting `m1` on the site.
fn with_meeting() -> (Webex, Server, Arc<MemoryCredentials>) {
    let m1 = json!({"id": "m1", "webLink": "https://acme.webex.com/acme/j.php?MTID=m1", "title": "Budget"});
    signed_in(Data { meetings: vec![m1], ..Default::default() })
}

fn calls(server: &Server) -> Vec<String> {
    server.data.lock().unwrap().calls.clone()
}

fn bodies(server: &Server) -> Vec<Value> {
    server.data.lock().unwrap().bodies.clone()
}

fn refuse(server: &Server, status: u16, body: Value) {
    server.data.lock().unwrap().refuse.push_back((status, body));
}

/// The names of a body's fields, sorted.
fn names(body: &Value) -> Vec<String> {
    let mut names: Vec<String> = body.as_object().unwrap().keys().cloned().collect();
    names.sort();
    names
}

#[test]
fn creating_posts_the_appointment_to_the_site() {
    let (mut w, server, _) = signed_in(Data::default());
    let m = w.create(SITE, &fields(), true).unwrap();
    assert_eq!(calls(&server), ["POST /v1/meetings"]);
    let body = &bodies(&server)[0];
    assert_eq!(body["title"], "Budget");
    assert_eq!(body["start"], "2026-11-02T09:00:00Z");
    assert_eq!(body["end"], "2026-11-02T10:00:00Z");
    assert_eq!(body["timezone"], "Europe/Paris");
    assert_eq!(body["siteUrl"], "acme.webex.com");
    assert_eq!(body["sendEmail"], false);
    assert!(body.get("recurrence").is_none(), "{body}");
    assert_eq!(m.id, "m1");
    assert_eq!(m.web_link, "https://acme.webex.com/acme/j.php?MTID=m1");
    assert_eq!(m.meeting_number, "2623456701");
    assert_eq!(m.password, Secret("Xy7qT2".into()));
    assert_eq!(m.sip_address, "2623456701@acme.webex.com");

    let rule = WebexFields { recurrence: Some("FREQ=WEEKLY;COUNT=4".into()), ..fields() };
    w.create(SITE, &rule, false).unwrap();
    let body = &bodies(&server)[1];
    assert_eq!(body["sendEmail"], true);
    assert_eq!(body["recurrence"], "FREQ=WEEKLY;COUNT=4");
}

#[test]
fn no_personal_room_link_is_ever_built() {
    let (mut w, server, _) = signed_in(Data::default());
    let m = w.create(SITE, &fields(), true).unwrap();
    w.update(&m.id, &fields(), &WebexFields { title: "Other".into(), ..fields() }, true).unwrap();
    w.delete(&m.id, true).unwrap();
    assert_eq!(calls(&server).len(), 3);
    for call in calls(&server) {
        assert!(!call.contains("/meet/"), "{call}");
    }
    for body in bodies(&server) {
        assert!(!body.to_string().contains("/meet/"), "{body}");
    }
}

#[test]
fn changing_the_time_sends_only_what_changed() {
    let (mut w, server, _) = with_meeting();
    let hour_later = WebexFields { start: fields().start + 3600, end: fields().end + 3600, ..fields() };
    let m = w.update("m1", &fields(), &hour_later, true).unwrap();
    assert_eq!(m.id, "m1");
    assert_eq!(calls(&server), ["PATCH /v1/meetings/m1"]);
    let body = &bodies(&server)[0];
    assert_eq!(names(body), ["end", "sendEmail", "start", "timezone"]);
    assert_eq!(body["start"], "2026-11-02T10:00:00Z");
    assert_eq!(body["end"], "2026-11-02T11:00:00Z");
    assert_eq!(body["timezone"], "Europe/Paris");
    assert_eq!(body["sendEmail"], false);

    w.update("m1", &fields(), &WebexFields { title: "Other".into(), ..fields() }, false).unwrap();
    let body = &bodies(&server)[1];
    assert_eq!(names(body), ["sendEmail", "title"]);
    assert_eq!((&body["title"], &body["sendEmail"]), (&json!("Other"), &json!(true)));

    // A new zone alone still sends the three together.
    w.update("m1", &fields(), &WebexFields { timezone: "Europe/Berlin".into(), ..fields() }, true).unwrap();
    assert_eq!(names(&bodies(&server)[2]), ["end", "sendEmail", "start", "timezone"]);

    let weekly = WebexFields { recurrence: Some("FREQ=WEEKLY".into()), ..fields() };
    w.update("m1", &fields(), &weekly, true).unwrap();
    let body = &bodies(&server)[3];
    assert_eq!(names(body), ["recurrence", "sendEmail"]);
    assert_eq!(body["recurrence"], "FREQ=WEEKLY");

    // No longer repeating is a change too.
    w.update("m1", &weekly, &fields(), true).unwrap();
    let body = &bodies(&server)[4];
    assert_eq!(names(body), ["recurrence", "sendEmail"]);
    assert!(body["recurrence"].is_null(), "{body}");
}

#[test]
fn deleting_asks_webex_to_mail_no_one() {
    let (mut w, server, _) = with_meeting();
    w.delete("m1", true).unwrap();
    assert_eq!(calls(&server), ["DELETE /v1/meetings/m1?sendEmail=false"]);
    assert!(server.data.lock().unwrap().meetings.is_empty(), "the meeting is gone from the site");

    let (mut w, server, _) = with_meeting();
    w.delete("m1", false).unwrap();
    assert_eq!(calls(&server), ["DELETE /v1/meetings/m1"]);
}

#[test]
fn a_meeting_already_gone_is_deleted() {
    let (mut w, server, _) = signed_in(Data::default());
    assert_eq!(w.delete("m9", true), Ok(()));
    assert_eq!(calls(&server), ["DELETE /v1/meetings/m9?sendEmail=false"]);
}

#[test]
fn an_id_is_encoded_in_the_address() {
    let (mut w, server, _) = signed_in(Data::default());
    let _ = w.delete("a/b?c", true);
    let _ = w.update("a/b?c", &fields(), &fields(), true);
    assert_eq!(calls(&server), ["DELETE /v1/meetings/a%2Fb%3Fc?sendEmail=false", "PATCH /v1/meetings/a%2Fb%3Fc"]);
}

#[test]
fn an_expired_token_is_refreshed_once_and_the_request_tried_again() {
    let (mut w, server, creds) = signed_in(Data::default());
    refuse(&server, 401, json!({"message": "The request requires a valid access token."}));
    assert_eq!(w.create(SITE, &fields(), true).unwrap().id, "m1");
    assert_eq!(calls(&server), ["POST /v1/meetings", "POST /v1/access_token", "POST /v1/meetings"]);
    assert_eq!(creds.get_secret(&keyring(), REFRESH_KEY).as_deref(), Some("r1"), "the same refresh token is kept");

    let (mut w, server, creds) = signed_in(Data { rotate: true, ..Default::default() });
    assert_eq!(creds.get_secret(&keyring(), REFRESH_KEY).as_deref(), Some("rotated-1"));
    refuse(&server, 401, json!({}));
    w.create(SITE, &fields(), true).unwrap();
    assert_eq!(creds.get_secret(&keyring(), REFRESH_KEY).as_deref(), Some("rotated-2"), "the keychain holds the new refresh token");
}

/// A keychain that reads, and refuses to keep anything new (the trait's `set_secret`).
struct Locked(MemoryCredentials);

impl Credentials for Locked {
    fn get(&self, _: &AccountConfig) -> Option<String> {
        None
    }
    fn set(&self, _: &AccountConfig, _: &str) -> Result<(), String> {
        Err("locked".into())
    }
    fn delete(&self, _: &AccountConfig) {}
    fn get_smtp(&self, _: &AccountConfig) -> Option<String> {
        None
    }
    fn set_smtp(&self, _: &AccountConfig, _: &str) -> Result<(), String> {
        Err("locked".into())
    }
    fn delete_smtp(&self, _: &AccountConfig) {}
    fn get_secret(&self, account: &AccountConfig, what: &str) -> Option<String> {
        self.0.get_secret(account, what)
    }
}

/// The keychain still holds the old refresh token: the access token in hand is the one it
/// stays good for, and the meeting is made all the same.
#[test]
fn a_new_refresh_token_the_keychain_refuses_loses_nothing_now() {
    let server = fake_webex::serve(Data { rotate: true, ..Default::default() });
    let creds = Arc::new(Locked(keychain(Some("r1"))));
    let mut w = pointed_at(&server, creds.clone());
    w.create(SITE, &fields(), true).unwrap();
    w.create(SITE, &fields(), true).unwrap();
    assert_eq!(server.data.lock().unwrap().refreshes, 1);
    assert_eq!(creds.get_secret(&keyring(), REFRESH_KEY).as_deref(), Some("r1"));
}

#[test]
fn a_second_401_asks_to_sign_in_again() {
    let (mut w, server, _) = signed_in(Data::default());
    refuse(&server, 401, json!({}));
    refuse(&server, 401, json!({}));
    assert_eq!(w.create(SITE, &fields(), true), Err(WebexError::SignIn));
    assert_eq!(calls(&server), ["POST /v1/meetings", "POST /v1/access_token", "POST /v1/meetings"]);
}

#[test]
fn no_refresh_token_asks_to_sign_in() {
    let server = fake_webex::serve(Data::default());
    let (mut w, _) = client(&server, None);
    assert_eq!(w.create(SITE, &fields(), true), Err(WebexError::SignIn));
    assert_eq!(calls(&server), Vec::<String>::new());

    // No secret, or a client id that couldn't be one: nothing is sent either.
    let (mut w, creds) = client(&server, Some("r1"));
    creds.delete_secret(&keyring(), SECRET_KEY);
    assert_eq!(w.create(SITE, &fields(), true), Err(WebexError::SignIn));
    let (mut w, _) = client(&server, Some("r1"));
    w.client_id = "not an id!".into();
    assert_eq!(w.create(SITE, &fields(), true), Err(WebexError::SignIn));
    assert_eq!(calls(&server), Vec::<String>::new());
}

#[test]
fn a_refresh_token_refused_asks_to_sign_in() {
    let server = fake_webex::serve(Data::default());
    let (mut w, _) = client(&server, Some(REVOKED));
    assert_eq!(w.create(SITE, &fields(), true), Err(WebexError::SignIn));
    assert_eq!(calls(&server), ["POST /v1/access_token"]);
}

#[test]
fn a_service_out_of_reach_is_a_failure_not_a_sign_in() {
    let server = fake_webex::serve(Data::default());
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://127.0.0.1:{}/v1", l.local_addr().unwrap().port())
    };
    let (mut w, _) = client(&server, Some("r1"));
    w.token_url = format!("{closed}/access_token");
    assert!(matches!(w.create(SITE, &fields(), true), Err(WebexError::Failed(_))));

    let (mut w, _, _) = signed_in(Data::default());
    w.api = closed;
    assert!(matches!(w.create(SITE, &fields(), true), Err(WebexError::Failed(_))));
    assert!(matches!(w.delete("m1", true), Err(WebexError::Failed(_))));
}

/// A token is kept while it's good, and not a request longer: not past its time, and not once
/// the refresh token it came from is no longer the keychain's (signed out, or in as someone else).
#[test]
fn a_token_lasts_until_it_runs_out_or_the_sign_in_changes() {
    let (mut w, server, creds) = signed_in(Data::default());
    let refreshes = || server.data.lock().unwrap().refreshes;
    w.create(SITE, &fields(), true).unwrap();
    w.create(SITE, &fields(), true).unwrap();
    assert_eq!(refreshes(), 1, "taken once, by signed_in");

    creds.set_secret(&keyring(), REFRESH_KEY, "r2").unwrap();
    w.create(SITE, &fields(), true).unwrap();
    assert_eq!(refreshes(), 2, "another sign-in");

    if let Some((t, _)) = &mut w.access {
        t.expires_at = 0;
    }
    w.create(SITE, &fields(), true).unwrap();
    assert_eq!(refreshes(), 3, "run out");

    creds.delete_secret(&keyring(), REFRESH_KEY);
    assert_eq!(w.create(SITE, &fields(), true), Err(WebexError::SignIn), "signed out, a good token is no use");
}

#[test]
fn a_403_says_the_account_cant_schedule() {
    let (mut w, server, _) = signed_in(Data::default());
    refuse(&server, 403, json!({"message": "Forbidden"}));
    assert_eq!(w.create(SITE, &fields(), true), Err(WebexError::Forbidden));
}

#[test]
fn other_refusals_carry_the_status_and_webexs_words() {
    let (mut w, server, _) = signed_in(Data::default());
    let mut refused = |status: u16, body: Value| {
        refuse(&server, status, body);
        w.create(SITE, &fields(), true).unwrap_err()
    };
    assert_eq!(refused(400, json!({"message": "x", "errors": [{"description": "y"}]})), WebexError::Refused { status: 400, message: "y".into() });
    assert_eq!(refused(409, json!({"message": "x", "errors": []})), WebexError::Refused { status: 409, message: "x".into() });
    assert_eq!(refused(400, json!({"message": "x", "errors": [{"description": " "}]})), WebexError::Refused { status: 400, message: "x".into() });
    assert_eq!(refused(502, json!("<html>Bad Gateway</html>")), WebexError::Refused { status: 502, message: "502".into() });
    assert_eq!(refused(500, json!({"message": 3})), WebexError::Refused { status: 500, message: "500".into() });
    assert_eq!(refused(400, json!({"message": "two\r\nlines\there"})), WebexError::Refused { status: 400, message: "two lines here".into() });
    match refused(400, json!({"message": "é".repeat(1000)})) {
        WebexError::Refused { message, .. } => assert_eq!(message.chars().count(), 300),
        other => panic!("{other:?}"),
    }
}

#[test]
fn hostile_answers_are_refused_or_cleaned() {
    const LINK: &str = "https://acme.webex.com/j.php";
    let (mut w, server, _) = signed_in(Data::default());
    let mut answered = |body: Value| {
        refuse(&server, 200, body);
        w.create(SITE, &fields(), true)
    };
    let m = answered(json!({
        "id": "m1", "webLink": LINK, "password": " ab\ncd\r\n", "meetingNumber": "26\u{7f}23", "sipAddress": "\tz@x.example ",
    }))
    .unwrap();
    assert_eq!(m.password, Secret("abcd".into()));
    assert_eq!((m.meeting_number.as_str(), m.sip_address.as_str()), ("2623", "z@x.example"));

    // What Webex leaves out is empty, and what it overdoes is cut.
    let m = answered(json!({"id": "m1", "webLink": LINK, "password": "p".repeat(1000)})).unwrap();
    assert_eq!((m.password.0.chars().count(), m.meeting_number.as_str()), (300, ""));

    let longest = format!("{LINK}?{}", "a".repeat(2048 - LINK.len() - 1));
    assert_eq!(answered(json!({"id": "m1", "webLink": longest})).unwrap().web_link.chars().count(), 2048);

    for link in
        [json!("javascript:x"), json!("http://x"), json!(format!("{longest}a")), json!("https://x\n.example"), json!(""), json!(3), Value::Null]
    {
        assert!(matches!(answered(json!({"id": "m1", "webLink": link})), Err(WebexError::Failed(_))), "{link}");
    }
    assert!(matches!(answered(json!({"webLink": LINK})), Err(WebexError::Failed(_))), "no id");
    for id in [json!("../m1"), json!("a b"), json!(""), json!("a".repeat(201)), json!(3)] {
        assert!(matches!(answered(json!({"id": id, "webLink": LINK})), Err(WebexError::Failed(_))), "{id}");
    }
    assert!(matches!(answered(json!("not json")), Err(WebexError::Failed(_))));
    assert!(matches!(answered(json!([1, 2])), Err(WebexError::Failed(_))));
    assert!(matches!(answered(json!("x".repeat(2 << 20))), Err(WebexError::Failed(_))), "a 2 MB answer");
}

#[test]
fn me_is_the_first_address() {
    let (mut w, server, _) = signed_in(Data::default());
    assert_eq!(w.me().as_deref(), Some("zoe@acme.example"));
    assert_eq!(calls(&server), ["GET /v1/people/me"]);
    for (status, body) in [
        (200, json!({"emails": []})),
        (200, json!({"emails": [" \n"]})),
        (200, json!({"emails": [3]})),
        (200, json!("not json")),
        (403, json!({"emails": ["zoe@acme.example"]})),
    ] {
        refuse(&server, status, body.clone());
        assert_eq!(w.me(), None, "{status} {body}");
    }
    let server = fake_webex::serve(Data::default());
    assert_eq!(client(&server, None).0.me(), None, "not signed in");
}

#[test]
fn the_provider_is_webexs_own() {
    let p = provider(" C1f2 ", " s3 ").unwrap();
    assert_eq!((p.id.as_str(), p.name.as_str()), ("webex", "Webex"));
    assert_eq!((p.auth_url.as_str(), p.token_url.as_str(), p.scope.as_str()), (AUTHORIZE_URL, TOKEN_URL, SCOPE));
    assert_eq!((p.client_id.as_str(), p.client_secret.as_deref()), ("C1f2", Some("s3")));
    assert_eq!(provider("", "s3"), None);
    assert_eq!(provider("not an id!", "s3"), None);
    assert_eq!(provider("C1f2", " "), None);
    assert_eq!((keyring().id.as_str(), keyring().email.as_str()), ("webex", "webex"));
}
