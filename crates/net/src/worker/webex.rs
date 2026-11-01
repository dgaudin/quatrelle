//! The pool: Webex's jobs. Its calls run one after the other on a thread of their own, started
//! with the first of them: a change never overtakes the one before it, and two refreshes of the
//! token never race. Signing in waits on the browser, so it has a thread to itself.

use quatrelle_model::{WEBEX_REDIRECT_PORT, WebexError};

use super::*;
use crate::oauth;
use crate::webex::{REFRESH_KEY, SECRET_KEY, Webex, keyring, provider};

/// How long the sign-in in the browser is waited for.
const SIGN_IN_WAIT: Duration = Duration::from_secs(5 * 60);

impl Pool {
    pub(super) fn webex(&mut self, job: Job) {
        match job {
            Job::WebexSignIn { client_id } => self.webex_sign_in(client_id),
            Job::SetWebexSecret { secret } => {
                if secret.0.trim().is_empty() {
                    self.creds.delete_secret(&keyring(), SECRET_KEY);
                } else if let Err(e) = self.creds.set_secret(&keyring(), SECRET_KEY, secret.0.trim()) {
                    self.sink.emit(Event::Notice { account: String::new(), message: format!("the Webex client secret couldn't be kept: {e}") });
                }
            }
            job => self.webex_in_turn(job),
        }
    }

    /// Browser sign-in, on a thread of its own: the page opens through the UI (`Event::OpenUrl`)
    /// and the refresh token goes to the keychain.
    fn webex_sign_in(&mut self, client_id: String) {
        let (sink, creds) = (self.sink.clone(), self.creds.clone());
        let spawned = std::thread::Builder::new().name("webex-sign-in".into()).spawn(move || {
            let mut open = |url: &str| -> std::result::Result<(), String> {
                sink.emit(Event::OpenUrl { url: url.to_string() });
                Ok(())
            };
            let (user, error) = match sign_in(&client_id, creds, &mut open) {
                Ok(user) => (user, None),
                Err(e) => (None, Some(e)),
            };
            sink.emit(Event::WebexSignedIn { user, error });
        });
        if let Err(e) = spawned {
            self.sink.emit(Event::WebexSignedIn { user: None, error: Some(WebexError::Failed(e.to_string())) });
        }
    }

    /// Hand a job to the thread of Webex's calls, started the first time.
    fn webex_in_turn(&mut self, job: Job) {
        if self.webex_jobs.is_none() {
            match start(self.sink.clone(), self.creds.clone(), Webex::new) {
                Ok(tx) => self.webex_jobs = Some(tx),
                Err(e) => return self.webex_not_started(job, WebexError::Failed(e.to_string())),
            }
        }
        if let Some(tx) = &self.webex_jobs {
            let _ = tx.send(job);
        }
    }

    /// A job no thread can run: answered with why, and a sign-out is done here, as nothing can
    /// race it.
    fn webex_not_started(&mut self, job: Job, why: WebexError) {
        let error = Some(why);
        match job {
            Job::WebexCreate { ask, .. } => self.sink.emit(Event::WebexCreated { ask, meeting: None, error }),
            Job::WebexUpdate { id, .. } => self.sink.emit(Event::WebexUpdated { id, meeting: None, error }),
            Job::WebexDelete { id, .. } => self.sink.emit(Event::WebexDeleted { id, error }),
            Job::WebexSignOut => self.creds.delete_secret(&keyring(), REFRESH_KEY),
            _ => {}
        }
    }
}

/// Sign in through the browser, on the caller's thread: who signed in, when Webex says.
fn sign_in(
    client_id: &str,
    creds: Arc<dyn Credentials>,
    open: &mut dyn FnMut(&str) -> std::result::Result<(), String>,
) -> std::result::Result<Option<String>, WebexError> {
    let failed = |text: &str| WebexError::Failed(text.to_string());
    let keyring = keyring();
    let secret = creds.get_secret(&keyring, SECRET_KEY).ok_or_else(|| failed("the client secret isn't set"))?;
    let p = provider(client_id, &secret).ok_or_else(|| failed("the client id or the client secret can't be used"))?;
    let t = oauth::authorize_on(&p, WEBEX_REDIRECT_PORT, "", open, SIGN_IN_WAIT).map_err(|e| failed(&e.to_string()))?;
    let refresh = t.refresh.filter(|r| !r.is_empty()).ok_or_else(|| failed("Webex gave no lasting sign-in; try again"))?;
    creds.set_secret(&keyring, REFRESH_KEY, &refresh).map_err(|e| failed(&format!("the sign-in couldn't be kept in the keychain: {e}")))?;
    Ok(Webex::new(client_id, creds).me())
}

/// Start the thread of Webex's calls; `client` makes the client of an integration.
fn start(
    sink: Sink,
    creds: Arc<dyn Credentials>,
    client: impl Fn(&str, Arc<dyn Credentials>) -> Webex + Send + 'static,
) -> std::io::Result<Sender<Job>> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new().name("webex".into()).spawn(move || run(&rx, &sink, &creds, client))?;
    Ok(tx)
}

/// Run jobs in the order they came, until the pool goes. The client is kept between them, for its
/// token; another client id is another integration, and takes a new one.
fn run(rx: &Receiver<Job>, sink: &Sink, creds: &Arc<dyn Credentials>, client: impl Fn(&str, Arc<dyn Credentials>) -> Webex) {
    let mut held: Option<(String, Webex)> = None;
    for job in rx {
        let client_id = match &job {
            Job::WebexCreate { client_id, .. } | Job::WebexUpdate { client_id, .. } | Job::WebexDelete { client_id, .. } => client_id,
            // Here, not at once: a refresh going on would put the token back after it.
            Job::WebexSignOut => {
                creds.delete_secret(&keyring(), REFRESH_KEY);
                continue;
            }
            _ => continue,
        };
        if held.as_ref().is_none_or(|(id, _)| id != client_id) {
            held = Some((client_id.clone(), client(client_id, creds.clone())));
        }
        if let Some((_, webex)) = &mut held
            && let Some(answer) = call(webex, job)
        {
            sink.emit(answer);
        }
    }
}

/// Run a job with its client; what it came to.
fn call(webex: &mut Webex, job: Job) -> Option<Event> {
    let split = |answer: std::result::Result<_, WebexError>| match answer {
        Ok(meeting) => (Some(meeting), None),
        Err(e) => (None, Some(e)),
    };
    match job {
        Job::WebexCreate { ask, site, fields, quiet, .. } => {
            let (meeting, error) = split(webex.create(&site, &fields, quiet));
            Some(Event::WebexCreated { ask, meeting, error })
        }
        Job::WebexUpdate { id, before, after, quiet, .. } => {
            let (meeting, error) = split(webex.update(&id, &before, &after, quiet));
            Some(Event::WebexUpdated { id, meeting, error })
        }
        Job::WebexDelete { id, quiet, .. } => {
            let error = webex.delete(&id, quiet).err();
            Some(Event::WebexDeleted { id, error })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use quatrelle_model::WebexFields;

    use super::*;
    use crate::fake_webex::{self, Data, SECRET, Server};

    fn fields() -> WebexFields {
        WebexFields { title: "Budget".into(), start: 1_793_610_000, end: 1_793_613_600, timezone: "Europe/Paris".into(), recurrence: None }
    }

    /// A pool whose Webex thread calls the fake, with a keychain holding the client secret and a
    /// sign-in.
    fn pool_on(server: &Server, name: &str) -> (Pool, Arc<MemoryCredentials>) {
        let creds = Arc::new(MemoryCredentials::default());
        creds.set_secret(&keyring(), SECRET_KEY, SECRET).unwrap();
        creds.set_secret(&keyring(), REFRESH_KEY, "r1").unwrap();
        let mut pool = Pool::new(crate::fake::tests_store(name), creds.clone());
        let (api, token_url) = (server.base.clone(), format!("{}/access_token", server.base));
        let client = move |client_id: &str, creds: Arc<dyn Credentials>| {
            let mut w = Webex::new(client_id, creds);
            w.api.clone_from(&api);
            w.token_url.clone_from(&token_url);
            w
        };
        pool.webex_jobs = Some(start(pool.sink.clone(), pool.creds.clone(), client).unwrap());
        (pool, creds)
    }

    fn answers(pool: &Pool, how_many: usize) -> Vec<Event> {
        (0..how_many).filter_map(|_| pool.wait(Duration::from_secs(10))).collect()
    }

    /// The update and the delete go before the creation has answered: only their order on the
    /// one thread makes `m1` there for them.
    #[test]
    fn jobs_run_in_order_on_one_thread() {
        let server = fake_webex::serve(Data::default());
        let (mut pool, _) = pool_on(&server, "webex-order");
        let after = WebexFields { title: "Budget, moved".into(), ..fields() };
        pool.submit(Job::WebexCreate { ask: 7, client_id: "C1".into(), site: "acme.webex.com".into(), fields: fields(), quiet: false });
        pool.submit(Job::WebexUpdate { client_id: "C1".into(), id: "m1".into(), before: fields(), after, quiet: false });
        pool.submit(Job::WebexDelete { client_id: "C1".into(), id: "m1".into(), quiet: true });
        match &answers(&pool, 3)[..] {
            [
                Event::WebexCreated { ask: 7, meeting: Some(made), error: None },
                Event::WebexUpdated { id: updated, meeting: Some(changed), error: None },
                Event::WebexDeleted { id: deleted, error: None },
            ] => assert_eq!((made.id.as_str(), changed.id.as_str(), updated.as_str(), deleted.as_str()), ("m1", "m1", "m1", "m1")),
            other => panic!("{other:?}"),
        }
        let data = server.data.lock().unwrap();
        assert_eq!(data.calls, ["POST /v1/access_token", "POST /v1/meetings", "PATCH /v1/meetings/m1", "DELETE /v1/meetings/m1?sendEmail=false"]);
    }

    /// A token belongs to an integration: another client id starts again, the same one doesn't.
    #[test]
    fn another_client_id_takes_a_new_client() {
        let server = fake_webex::serve(Data::default());
        let (mut pool, _) = pool_on(&server, "webex-clients");
        for client_id in ["C1", "C1", "C2"] {
            pool.submit(Job::WebexDelete { client_id: client_id.into(), id: "gone".into(), quiet: false });
        }
        assert_eq!(answers(&pool, 3).len(), 3);
        assert_eq!(server.data.lock().unwrap().refreshes, 2);
    }

    /// Signing out takes its turn: a refresh going on would put the token back after it.
    #[test]
    fn signing_out_takes_its_turn() {
        let server = fake_webex::serve(Data::default());
        let (mut pool, creds) = pool_on(&server, "webex-sign-out");
        pool.submit(Job::WebexCreate { ask: 7, client_id: "C1".into(), site: "acme.webex.com".into(), fields: fields(), quiet: false });
        pool.submit(Job::WebexSignOut);
        pool.submit(Job::WebexDelete { client_id: "C1".into(), id: "m1".into(), quiet: false });
        match &answers(&pool, 2)[..] {
            [Event::WebexCreated { meeting: Some(_), error: None, .. }, Event::WebexDeleted { error: Some(WebexError::SignIn), .. }] => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(creds.get_secret(&keyring(), REFRESH_KEY), None);
        assert_eq!(creds.get_secret(&keyring(), SECRET_KEY).as_deref(), Some(SECRET), "the integration's secret stays");
    }

    #[test]
    fn the_client_secret_is_kept_and_forgotten() {
        let creds = Arc::new(MemoryCredentials::default());
        let mut pool = Pool::new(crate::fake::tests_store("webex-secret"), creds.clone());
        pool.submit(Job::SetWebexSecret { secret: Secret(" s3cret ".into()) });
        assert_eq!(creds.get_secret(&keyring(), SECRET_KEY).as_deref(), Some("s3cret"));
        pool.submit(Job::SetWebexSecret { secret: Secret("  ".into()) });
        assert_eq!(creds.get_secret(&keyring(), SECRET_KEY), None);
    }

    #[test]
    fn signing_in_needs_the_client_secret() {
        let mut pool = Pool::new(crate::fake::tests_store("webex-no-secret"), Arc::new(MemoryCredentials::default()));
        pool.submit(Job::WebexSignIn { client_id: "C1".into() });
        let refused = WebexError::Failed("the client secret isn't set".into());
        assert_eq!(answers(&pool, 1), [Event::WebexSignedIn { user: None, error: Some(refused) }]);
    }
}
