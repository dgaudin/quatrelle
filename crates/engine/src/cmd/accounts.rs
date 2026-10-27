//! Commands: accounts, data files, mailbox settings.

use super::*;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("folder.empty", "Empty folder", "Folder › Empty", |s, v| {
            let account = account_param(s, v)?;
            let folder = match (p::str(v, "folder"), p::str(v, "role")) {
                (Some(f), _) => f.to_string(),
                (None, Some(r)) => {
                    let role = FolderRole::from_code(r).ok_or_else(|| CmdError::Params(format!("unknown folder role `{r}`")))?;
                    s.account(&account).and_then(|a| a.folder(role)).map(|f| f.path.clone()).ok_or_else(|| CmdError::Params(format!("the account has no {r} folder")))?
                }
                (None, None) => return Err(CmdError::Params("`folder` or `role` is required".into())),
            };
            s.empty_folder(&account, &folder)
        })
        .params(r#"{"account"?: string, "folder"?: string (Deleted Items and Junk are emptied for good; any other folder's mail goes to Deleted Items), "role"?: "trash"|"junk"}"#),
        CommandSpec::new("account.add", "Add account", "File › Accounts", add_account)
            .params(r#"{"email": string, "password"?: string, "oauth"?: "google"|"microsoft", "deferSignIn"?: bool, "local"?: bool, "name"?: string, "username"?: string, "imapHost"?: string, "imapPort"?: number, "imapSecurity"?: "tls"|"starttls"|"none", "smtpHost"?: string, "smtpPort"?: number, "smtpSecurity"?: …, "smtpUsername"?: string and "smtpPassword"?: string (the outgoing server's own sign-in, when it isn't the incoming server's), "jmap"?: string (a JMAP session URL, instead of IMAP and SMTP), "pop"?: true (POP3 instead of IMAP: popHost, popPort, popSecurity, "popDelete"?: bool to remove mail from the server once downloaded, else a copy stays: "popKeepDays"?: number (removed that many days after downloading; 14 when not given, 0 for never), "popDeleteWithTrash"?: bool (removed once deleted here for good)), "pst"?: string (an existing Outlook data file whose mail comes in first), "ews"?: string|true (on-premises Exchange: its EWS address or server name, or true to find it with Autodiscover; username DOMAIN\\user or an address), "graph"?: true|string (Exchange Online through Microsoft Graph; signs in with Microsoft), "userAgent"?: string (with ews: the User-Agent its requests name instead of Quatrelle's, for a server that lets in only clients it knows), "oauthClientId"?: string and "oauthTenant"?: string (Microsoft sign-in through your organisation's own app registration: its Application (client) ID, and its tenant in place of common), "onlineOnly"?: bool (keep none of its mail on this computer; not for POP)}"#),
        CommandSpec::new("account.importPst", "Open Outlook data file", "File › Open & Export", |s, v| {
            let path = p::req_str(v, "path")?.to_string();
            s.import_pst(&path)
        })
        .params(r#"{"path": string (a .pst or .quatrelle data file; its mail goes into a new account on this computer)}"#),
        CommandSpec::new("account.exportPst", "Export to Outlook data file", "File › Open & Export", |s, v| {
            let path = p::req_str(v, "path")?.to_string();
            let account = match p::str(v, "account") {
                Some(a) => a.to_string(),
                None => s.current_account().map(|a| a.config.id.clone()).ok_or_else(|| CmdError::Params("there is no account to export".into()))?,
            };
            s.export_pst(&account, &path)
        })
        .params(r#"{"path": string (the .pst file to write; one already there is replaced), "account"?: string (default: the current one)}"#),
        CommandSpec::new("account.discover", "Find server settings", "File › Accounts", |s, v| {
            let email = p::req_str(v, "email")?.trim().to_ascii_lowercase();
            if !quatrelle_model::addr::is_valid_email(&email) {
                return Err(CmdError::Params(format!("`{email}` is not an email address")));
            }
            match s.discovered.get(&email) {
                Some(Some((Some(c), source))) => Ok(json!({
                    "found": true,
                    "source": source,
                    "imap": c.imap,
                    "smtp": c.smtp,
                    "pop": c.pop,
                    "username": c.username,
                    // Microsoft 365 hosts the domain's mail: Exchange Online through Graph.
                    "graph": c.graph,
                })),
                Some(Some((None, _))) => Ok(json!({"found": false})),
                Some(None) => Ok(json!({"pending": true})),
                None => {
                    s.discovered.insert(email.clone(), None);
                    s.push_job(quatrelle_model::Job::Discover { email });
                    Ok(json!({"pending": true}))
                }
            }
        })
        .params(r#"{"email": string} (asks once; call again for the answer)"#),
        CommandSpec::new("account.move", "Move account", "Folder pane › account menu", |s, v| {
            let id = p::req_str(v, "account")?.to_string();
            let by = p::i64(v, "by").ok_or_else(|| CmdError::Params("`by` is required: -1 (up) or 1 (down)".into()))?;
            s.move_account(&id, by.clamp(-1000, 1000))
        })
        .params(r#"{"account": string, "by": number (-1 moves it up, 1 down)}"#),
        CommandSpec::new("folderPane.move", "Move mailbox", "Folder pane › mailbox menu", |s, v| {
            let key = p::req_str(v, "key")?.to_string();
            let by = p::i64(v, "by").ok_or_else(|| CmdError::Params("`by` is required: -1 (up) or 1 (down)".into()))?;
            let to = s.move_pane_section(&key, by.clamp(-1000, 1000))?;
            Ok(json!({"key": key, "position": to}))
        })
        .params(r#"{"key": string (an account id, or <account>|<address> for a shared mailbox), "by": number (-1 moves it up, 1 down)}"#),
        CommandSpec::new("account.remove", "Remove account", "File › Accounts", |s, v| {
            let id = p::req_str(v, "account")?.to_string();
            s.remove_account(&id)
        })
        .params(r#"{"account": string}"#),
        CommandSpec::new("account.signIn", "Sign in with the browser", "File › Accounts", |s, v| {
            let id = p::req_str(v, "account")?;
            let config = s.account(id).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
            if config.oauth.is_none() {
                // A Microsoft account signing in with a password (refused there) switches over.
                if quatrelle_model::microsoft_password_account(&config) {
                    if !quatrelle_model::microsoft_sign_in_possible(&config, &s.oauth_available) {
                        return Err(CmdError::Params("signing in with Microsoft isn't set up in this copy of Quatrelle".into()));
                    }
                    return s.use_microsoft_sign_in(id);
                }
                return Err(CmdError::Params("this account signs in with a password".into()));
            }
            s.push_job(quatrelle_model::Job::SignIn { account: config });
            Ok(json!({"signingIn": id}))
        })
        .params(r#"{"account": string} (a Microsoft account that signs in with a password switches to Sign in with Microsoft, keeping its mail)"#),
        CommandSpec::new("account.useGraph", "Connect through Microsoft Graph", "File › Accounts", |s, v| {
            let id = p::req_str(v, "account")?.to_string();
            s.use_graph(&id)
        })
        .params(r#"{"account": string} (a Microsoft account over IMAP starts again through Microsoft Graph, for a mailbox Microsoft won't open over IMAP)"#),
        CommandSpec::new("account.setPassword", "Sign in again", "File › Accounts", set_password).params(r#"{"account": string, "password": string}"#),
        CommandSpec::new("account.setSignature", "Signature", "File › Options › Mail", set_signature)
            .params(r#"{"account": string, "signature": string, "styles"?: object (its look as the editor keeps it: a style per character and paragraph), "html"?: string (its look as HTML: bold, colours, sizes, links; the text then comes from it), "newMessages"?: bool (added to new messages; default: as it was), "replies"?: bool (added to replies and forwards), "rawHtml"?: string (the signature written in HTML, sent as it is, cleaned)}"#),
        CommandSpec::new("account.list", "Accounts", "File › Accounts", list_accounts),
        // Asks where Quatrelle is published; File › About shows the answer (`Session::update`).
        CommandSpec::new("app.checkUpdates", "Check for updates", "File › About", |s, _| {
            if s.jmap_only {
                return Err(CmdError::Failed("the browser version is always the newest".into()));
            }
            s.check_updates();
            Ok(json!({"checking": true}))
        }),
        CommandSpec::new("account.update", "Account settings", "File › Info › Account settings", update_account).params(
            r#"{"account": string, "name"?: string, "username"?: string, "imapHost"?: string, "imapPort"?: number, "imapSecurity"?: …, "smtpHost"?: …, "smtpUsername"?: string|null (the outgoing server's own sign-in; null: the incoming server's), "smtpPassword"?: string, "jmap"?: string|null, "popHost"?: …, "popDelete"?: bool, "popKeepDays"?: number, "popDeleteWithTrash"?: bool, "onlineOnly"?: bool, "ews"?: string, "userAgent"?: string (Exchange: the User-Agent its requests name; "" for Quatrelle's), "oauthClientId"?: string|null and "oauthTenant"?: string|null (Microsoft sign-in through your organisation's own app registration; null or empty: the built-in one, common)}"#,
        ),
        CommandSpec::new("account.mailbox", "Mailbox settings", "File › Info › Mailbox settings", |s, v| {
            let id = account_param(s, v)?;
            let a = s.account(&id).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
            let quota = a.quota.map(|(used, limit)| json!({"used": used, "limit": limit}));
            Ok(json!({"account": id, "quota": quota, "cached": s.cache_size(&id)}))
        })
        .params(r#"{"account"?: string}"#),
        CommandSpec::new("account.vacation", "Automatic replies", "File › Info › Automatic replies", |s, v| {
            let id = account_param(s, v)?;
            if p::bool(v, "refresh").unwrap_or(true) {
                s.load_vacation(&id)
            } else {
                Ok(s.vacation_json(&id))
            }
        })
        .params(r#"{"account"?: string, "refresh"?: bool} (asks the server; call with refresh=false for the answer)"#),
        CommandSpec::new("account.setVacation", "Turn automatic replies on or off", "File › Info › Automatic replies", |s, v| {
            let id = account_param(s, v)?;
            let day = |k: &str| p::str(v, k).map(str::trim).filter(|d| !d.is_empty()).map(str::to_string);
            let vacation = quatrelle_model::job::Vacation {
                enabled: p::bool(v, "enabled").unwrap_or(true),
                subject: p::str(v, "subject").unwrap_or_default().chars().take(200).collect(),
                text: p::str(v, "text").unwrap_or_default().chars().take(20_000).collect(),
                from: day("from"),
                until: day("until"),
                external: p::str(v, "external").map(|t| t.chars().take(20_000).collect::<String>()).filter(|t| !t.trim().is_empty()),
                outside: match p::str(v, "outside").map(str::trim) {
                    None | Some("" | "all") => String::new(),
                    Some(o @ ("known" | "none")) => o.to_string(),
                    Some(other) => return Err(CmdError::Params(format!("`outside` is all, known or none, not `{other}`"))),
                },
            };
            s.set_vacation(&id, vacation)
        })
        .params(r#"{"account"?: string, "enabled": bool, "subject"?: string, "text": string, "from"?: "YYYY-MM-DD", "until"?: "YYYY-MM-DD", "external"?: string (Exchange: another reply for people outside the organisation), "outside"?: "all"|"known"|"none" (Exchange: who outside gets one)}"#),
    ]
}

pub(super) fn server(v: &Value, prefix: &str, guess: &ServerConfig) -> Result<ServerConfig, CmdError> {
    let host = p::str(v, &format!("{prefix}Host")).map(|h| h.trim().to_string()).unwrap_or_else(|| guess.host.clone());
    let port = match p::u64(v, &format!("{prefix}Port")) {
        Some(n) => u16::try_from(n).map_err(|_| CmdError::Params(format!("{prefix}Port: {n} is not a port")))?,
        None => guess.port,
    };
    let security = match p::str(v, &format!("{prefix}Security")) {
        Some(x) => Security::from_code(x).ok_or_else(|| CmdError::Params(format!("{prefix}Security: unknown `{x}`")))?,
        None => guess.security,
    };
    if host.contains(char::is_whitespace) || host.contains('/') {
        return Err(CmdError::Params(format!("{prefix}Host: `{host}` is not a server name")));
    }
    Ok(ServerConfig { host, port, security })
}

pub(super) fn add_account(s: &mut Session, v: &Value) -> CmdResult {
    let email = p::req_str(v, "email")?;
    let guess = AccountConfig::guess(email);
    let local = p::bool(v, "local").unwrap_or(false);
    let jmap = p::str(v, "jmap").map(str::trim).filter(|u| !u.is_empty()).map(str::to_string);
    // `graph`: true for Exchange Online through Microsoft Graph (or the API's address).
    let graph = match v.get("graph") {
        Some(Value::Bool(true)) => Some(quatrelle_model::GRAPH_URL.to_string()),
        Some(Value::String(u)) if !u.trim().is_empty() => Some(u.trim().to_string()),
        _ => None,
    };
    // `ews`: on-premises Exchange through Exchange Web Services, at this address (a server name
    // alone means its usual `/EWS/Exchange.asmx`).
    // `true`: found with Autodiscover when the account connects.
    let ews = match v.get("ews") {
        Some(Value::Bool(true)) => Some(quatrelle_model::EWS_AUTODISCOVER.to_string()),
        Some(Value::String(u)) if !u.trim().is_empty() => Some(ews_url(u)),
        _ => None,
    };
    if s.jmap_only && jmap.is_none() {
        return Err(CmdError::Params("in the browser, Quatrelle works with JMAP accounts only".into()));
    }
    // `pop`: true to download mail over POP3 (popHost, popPort, popSecurity) and send over SMTP.
    let pop = if p::bool(v, "pop").unwrap_or(false) {
        let host = match guess.imap.host.strip_prefix("imap.") {
            Some(rest) => format!("pop.{rest}"),
            None => guess.imap.host.clone(),
        };
        Some(server(v, "pop", &ServerConfig { host, port: 995, security: Security::Tls })?)
    } else {
        None
    };
    let plain = local || jmap.is_some() || graph.is_some() || ews.is_some();
    let user_agent = if ews.is_some() { user_agent_param(v)?.flatten() } else { None };
    let password = p::str(v, "password").map(str::to_string);
    // Graph signs in with Microsoft, unless given a token to use as it is.
    let oauth = p::str(v, "oauth")
        .map(str::to_string)
        .or_else(|| (graph.is_some() && password.as_deref().is_none_or(str::is_empty)).then(|| "microsoft".to_string()));
    // An organisation's own Microsoft app registration, in place of the built-in one.
    let oauth_client_id = oauth_setting(v, "oauthClientId", quatrelle_model::valid_oauth_client_id)?.flatten();
    let oauth_tenant = oauth_setting(v, "oauthTenant", quatrelle_model::valid_oauth_tenant)?.flatten();
    let config = AccountConfig {
        id: String::new(),
        name: p::str(v, "name").map(str::to_string).unwrap_or(guess.name.clone()),
        email: email.to_string(),
        username: p::str(v, "username").map(str::to_string).unwrap_or(guess.username.clone()),
        imap: if plain || pop.is_some() { ServerConfig::default() } else { server(v, "imap", &guess.imap)? },
        smtp: if plain { ServerConfig::default() } else { server(v, "smtp", &guess.smtp)? },
        smtp_username: None,
        signature: p::str(v, "signature").unwrap_or_default().to_string(),
        signature_html: String::new(),
        signature_styles: String::new(),
        signature_raw_html: String::new(),
        signature_pictures: Vec::new(),
        signature_not_new: false,
        signature_not_replies: false,
        color: 0,
        oauth,
        jmap,
        graph,
        ews,
        pop,
        pop_delete: p::bool(v, "popDelete").unwrap_or(false),
        // A copy stays on the server for 14 days unless asked otherwise (0: for good).
        pop_keep_days: match v.get("popKeepDays").and_then(serde_json::Value::as_u64) {
            None => Some(14),
            Some(0) => None,
            Some(n) => Some(u32::try_from(n).unwrap_or(u32::MAX)),
        },
        pop_delete_with_trash: p::bool(v, "popDeleteWithTrash").unwrap_or(false),
        oauth_user: None,
        shared_mailboxes: Vec::new(),
        send_from: Vec::new(),
        user_agent,
        online_only: p::bool(v, "onlineOnly").unwrap_or(false),
        shared_online: p::bool(v, "sharedOnline").unwrap_or(false),
        public_favorites_kept: p::bool(v, "publicFavoritesKept").unwrap_or(false),
        oauth_client_id,
        oauth_tenant,
    };
    // A registration built in, or the account's own (Microsoft).
    // A line break or tab pasted in with the user name: no server would ever take it (in an
    // Exchange `DOMAIN\user`, a `\r` read from JSON became part of it, and sign-in failed).
    if config.username.chars().any(char::is_control) {
        return Err(CmdError::Params("the user name has a line break or another invisible character in it".into()));
    }
    if let Some(o) = &config.oauth
        && !s.oauth_available.contains(o)
        && !(o == "microsoft" && config.own_oauth_app())
    {
        return Err(CmdError::Params(format!("signing in with {o} isn't set up in this copy of Quatrelle")));
    }
    // `deferSignIn`: add now, ask for the password when the account first connects.
    let defer = p::bool(v, "deferSignIn").unwrap_or(false);
    if !local && !defer && config.oauth.is_none() && password.as_deref().is_none_or(str::is_empty) {
        return Err(CmdError::Params("`password` is required".into()));
    }
    // The outgoing server's own sign-in (IMAP or POP with SMTP).
    let mut config = config;
    let smtp_password = smtp_sign_in(v, &mut config, true)?;
    let id = s.add_account(config, password)?;
    if let Some(pw) = smtp_password {
        s.keep_smtp_password(&id, pw);
    }
    // `pst`: an existing data file whose mail comes into the account first.
    if let Some(path) = p::str(v, "pst").map(str::trim).filter(|p| !p.is_empty()) {
        s.attach_data_file(&id, path)?;
    }
    Ok(json!({"account": id}))
}

/// `oauthClientId` / `oauthTenant`: an organisation's own Microsoft app registration. `None`
/// when not given; `Some(None)` to use the built-in one (null, or empty); checked for what it
/// could be otherwise.
fn oauth_setting(v: &Value, key: &str, valid: fn(&str) -> bool) -> Result<Option<Option<String>>, CmdError> {
    match v.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => {
            let s = s.trim();
            if s.is_empty() {
                Ok(Some(None))
            } else if valid(s) {
                Ok(Some(Some(s.to_string())))
            } else {
                Err(CmdError::Params(format!("{key}: `{s}` isn't one")))
            }
        }
        Some(_) => Err(CmdError::Params(format!("{key}: a string is expected"))),
    }
}

/// An Exchange Web Services address from what the user typed: a server name gets `https://` and
/// `/EWS/Exchange.asmx`.
pub(super) fn ews_url(typed: &str) -> String {
    let t = typed.trim().trim_end_matches('/');
    let with_scheme = if t.contains("://") { t.to_string() } else { format!("https://{t}") };
    let after_host = with_scheme.split_once("://").map_or("", |(_, rest)| rest);
    if after_host.contains('/') { with_scheme } else { format!("{with_scheme}/EWS/Exchange.asmx") }
}

/// `userAgent`: `None` when it isn't given, `Some(None)` when it's empty (Quatrelle's own), else
/// the one to name; refused when it can't go in a header.
fn user_agent_param(v: &Value) -> Result<Option<Option<String>>, CmdError> {
    match v.get("userAgent") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(Some(None)),
        Some(Value::String(s)) => quatrelle_model::user_agent(s).map(|a| Some(Some(a))).ok_or_else(|| {
            CmdError::Params(format!(
                "a User-Agent is up to {} letters, digits, spaces and punctuation (no other characters)",
                quatrelle_model::MAX_USER_AGENT
            ))
        }),
        Some(_) => Err(CmdError::Params("`userAgent` is text".into())),
    }
}

pub(super) fn set_password(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "account")?;
    let pw = p::req_str(v, "password")?;
    if pw.is_empty() {
        return Err(CmdError::Params("`password` is empty".into()));
    }
    let config = s.account(id).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
    s.update_account(config, Some(pw.to_string()))
}

pub(super) fn update_account(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "account")?;
    let mut c = s.account(id).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
    if c.is_local() {
        return Err(CmdError::Params("the sample mailbox has no server settings".into()));
    }
    if let Some(n) = p::str(v, "name") {
        c.name = n.trim().chars().take(200).collect();
    }
    if let Some(u) = p::str(v, "username").map(str::trim).filter(|u| !u.is_empty()) {
        if u.chars().any(char::is_control) {
            return Err(CmdError::Params("the user name has a line break or another invisible character in it".into()));
        }
        c.username = u.to_string();
    }
    match v.get("jmap") {
        Some(Value::String(u)) if !u.trim().is_empty() => c.jmap = Some(u.trim().to_string()),
        Some(Value::String(_) | Value::Null) => c.jmap = None,
        _ => {}
    }
    if let Some(u) = p::str(v, "ews").map(str::trim).filter(|u| !u.is_empty())
        && c.ews.is_some()
    {
        c.ews = Some(ews_url(u));
    }
    if let Some(agent) = user_agent_param(v)?
        && c.ews.is_some()
    {
        c.user_agent = agent;
    }
    if let Some(pop) = c.pop.clone() {
        c.pop = Some(server(v, "pop", &pop)?);
        c.smtp = server(v, "smtp", &c.smtp)?;
        if let Some(d) = p::bool(v, "popDelete") {
            c.pop_delete = d;
        }
        if let Some(n) = v.get("popKeepDays").and_then(serde_json::Value::as_u64) {
            c.pop_keep_days = (n > 0).then(|| u32::try_from(n).unwrap_or(u32::MAX));
        }
        if let Some(d) = p::bool(v, "popDeleteWithTrash") {
            c.pop_delete_with_trash = d;
        }
    } else if !c.is_api() {
        c.imap = server(v, "imap", &c.imap)?;
        c.smtp = server(v, "smtp", &c.smtp)?;
        if c.imap.host.is_empty() || c.imap.port == 0 || c.smtp.host.is_empty() || c.smtp.port == 0 {
            return Err(CmdError::Params("incoming and outgoing server names and ports are required".into()));
        }
    }
    if let Some(on) = p::bool(v, "onlineOnly") {
        c.online_only = on;
    }
    if let Some(on) = p::bool(v, "sharedOnline") {
        c.shared_online = on;
    }
    if let Some(on) = p::bool(v, "publicFavoritesKept") {
        c.public_favorites_kept = on;
    }
    // Its own Microsoft registration, changed or let go of (the built-in one again). The
    // sign-in kept for the old registration is no good for a new one: asked again when it fails.
    if let Some(id) = oauth_setting(v, "oauthClientId", quatrelle_model::valid_oauth_client_id)? {
        c.oauth_client_id = id;
    }
    if let Some(t) = oauth_setting(v, "oauthTenant", quatrelle_model::valid_oauth_tenant)? {
        c.oauth_tenant = t;
    }
    let smtp_password = smtp_sign_in(v, &mut c, false)?;
    let done = s.update_account(c, None)?;
    if let Some(pw) = smtp_password {
        s.keep_smtp_password(id, pw);
    }
    Ok(done)
}

/// The outgoing server's own sign-in, as Outlook's "Log on using" for SMTP (issue #3: its user
/// name and password aren't the incoming server's): `smtpUsername` (null or empty: signs in as
/// the incoming server does) and `smtpPassword`, which is returned to be kept. A new one needs
/// its password; one already kept keeps it when none is given.
pub(super) fn smtp_sign_in(v: &Value, c: &mut AccountConfig, adding: bool) -> Result<Option<String>, CmdError> {
    let password = p::str(v, "smtpPassword").filter(|p| !p.is_empty()).map(str::to_string);
    let user = match v.get("smtpUsername") {
        None => return Ok(password.filter(|_| c.smtp_username.is_some())),
        Some(Value::Null) => None,
        Some(u) => u.as_str().map(str::trim).filter(|u| !u.is_empty()).map(|u| u.chars().take(320).collect::<String>()),
    };
    let Some(user) = user else {
        c.smtp_username = None;
        return Ok(None);
    };
    if c.is_local() || c.is_api() {
        return Err(CmdError::Params("only accounts that send over SMTP have an outgoing server's sign-in".into()));
    }
    if password.is_none() && (adding || c.smtp_username.is_none()) && !p::bool(v, "deferSignIn").unwrap_or(false) {
        return Err(CmdError::Params("the outgoing server's password is required".into()));
    }
    c.smtp_username = Some(user);
    Ok(password)
}

/// Most palette entries a signature's styles may bring (a hostile caller aside, a few dozen).
const SIGNATURE_STYLES_MAX: usize = 1000;

/// The signature's look from `styles` (the editor's own, as JSON) or `html`: the text and styles
/// a message shows, when it has a look at all.
fn signature_look(v: &Value, text: &str) -> Result<Option<(String, quatrelle_mime::rich::Styles)>, CmdError> {
    use quatrelle_mime::rich::Styles;
    if let Some(given) = v.get("styles").filter(|st| !st.is_null()) {
        let mut styles: Styles = serde_json::from_value(given.clone()).map_err(|e| CmdError::Params(format!("styles: {e}")))?;
        styles.palette.truncate(SIGNATURE_STYLES_MAX);
        styles.fit(text);
        return Ok((!text.trim().is_empty() && !styles.is_plain()).then(|| (text.to_string(), styles)));
    }
    let Some(html) = p::str(v, "html").filter(|h| !h.trim().is_empty()) else { return Ok(None) };
    let (text, styles) = quatrelle_mime::signature_html(&html.chars().take(200_000).collect::<String>());
    Ok((!text.trim().is_empty() && !styles.is_plain()).then(|| (text.chars().take(10_000).collect(), styles)))
}

pub(super) fn set_signature(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "account")?;
    let mut config = s.account(id).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
    config.signature = p::req_str(v, "signature")?.chars().take(10_000).collect();
    // Written in HTML: its style sheets written into its elements first (Outlook's class-based
    // styles would go with the sheet otherwise), then kept cleaned, sent as it is; its text
    // (and look, pictures on the web aside) is what a message shows while it's written.
    // Otherwise there's none.
    config.signature_raw_html = p::str(v, "rawHtml")
        .map(|h| quatrelle_mime::layout::clean_html(&quatrelle_mime::layout::inline_css(&h.chars().take(200_000).collect::<String>())))
        .unwrap_or_default();
    if !config.signature_raw_html.trim().is_empty() {
        let (text, styles) = quatrelle_mime::signature_html(&config.signature_raw_html);
        if text.trim().is_empty() {
            return Err(CmdError::Params("the signature's HTML has no text".into()));
        }
        config.signature = text.chars().take(10_000).collect();
        config.signature_html = if styles.is_plain() { String::new() } else { quatrelle_mime::rich::to_html(&text, &styles) };
        config.signature_styles.clear();
        config.signature_pictures.clear();
        let out = s.update_account(config, None)?;
        // Its pictures on the web: fetched and kept with it.
        s.fetch_signature_pictures();
        return Ok(out);
    }
    // When it's added, as Outlook's New messages and Replies/forwards.
    if let Some(on) = p::bool(v, "newMessages") {
        config.signature_not_new = !on;
    }
    if let Some(on) = p::bool(v, "replies") {
        config.signature_not_replies = !on;
    }
    // Its look: the editor's own styles (every size, typeface, underline and alignment as set,
    // kept as they are so nothing is lost on the way back), or HTML pasted or given from the
    // command line, read as the editor would show it; its HTML twin is what a message sends.
    config.signature_html = String::new();
    config.signature_styles = String::new();
    // Its pictures (a logo), as the editor sends them: those its text names are kept, a few
    // and small enough to go with every message.
    let given: Vec<quatrelle_mime::InlinePicture> = v.get("pictures").and_then(|p| serde_json::from_value(p.clone()).ok()).unwrap_or_default();
    let mut pictures: Vec<quatrelle_model::SignaturePicture> = Vec::new();
    let mut used: Vec<String> = Vec::new();
    if let Some((text, styles)) = signature_look(v, &config.signature)? {
        config.signature_html = quatrelle_mime::rich::to_html(&text, &styles);
        config.signature_styles = serde_json::to_string(&styles).unwrap_or_default();
        config.signature = text.chars().take(10_000).collect();
        used = quatrelle_mime::rich::pictures(&text, &styles);
        for p in given.into_iter().filter(|p| used.contains(&p.cid)) {
            if pictures.len() >= SIGNATURE_PICTURES || p.data.len() > SIGNATURE_PICTURE_MAX || !p.mime.starts_with("image/") {
                return Err(CmdError::Params(format!("{}: a signature takes {SIGNATURE_PICTURES} pictures of 500 KB at most", p.name)));
            }
            pictures.push(quatrelle_model::SignaturePicture { cid: p.cid, name: p.name, mime: p.mime, data: p.data });
        }
    }
    // Pictures given or not, its own stay while its text names them.
    if pictures.is_empty() {
        pictures = config.signature_pictures.iter().filter(|p| used.contains(&p.cid)).cloned().collect();
    }
    config.signature_pictures = pictures;
    s.update_account(config, None)
}

pub(super) fn list_accounts(s: &mut Session, _: &Value) -> CmdResult {
    Ok(json!(s
        .accounts
        .iter()
        .map(|a| json!({
            "account": a.config.id,
            "email": a.config.email,
            "name": a.config.name,
            "graph": a.config.graph,
            "pop": a.config.pop.as_ref().map(|p| format!("{}:{} ({})", p.host, p.port, p.security.code())),
            "ews": a.config.ews,
            "userAgent": a.config.user_agent,
            "imap": format!("{}:{} ({})", a.config.imap.host, a.config.imap.port, a.config.imap.security.code()),
            "smtp": format!("{}:{} ({})", a.config.smtp.host, a.config.smtp.port, a.config.smtp.security.code()),
            "status": a.status,
            "folders": a.folders.iter().map(|f| json!({"path": f.path, "name": f.display_name(), "role": f.role.code(), "unread": f.unread, "total": f.total})).collect::<Vec<_>>(),
        }))
        .collect::<Vec<_>>()))
}
