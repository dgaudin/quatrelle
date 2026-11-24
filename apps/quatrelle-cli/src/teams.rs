// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! `quatrelle-cli teams check`: the owner's step 0. It signs in to Microsoft in the browser
//! with the organisation's own app registration (`docs/teams.md`), runs the check of
//! [`quatrelle_net::teams::run_check`] on the account, and prints what it finds: counts, yes or
//! no, HTTP status numbers and durations only, to be pasted back. With `--channels`, alone, it
//! checks Teams' teams and channels instead ([`quatrelle_net::teams::run_channel_check`]), reading
//! only: the sign-in asks for the channels' permissions too, to learn which are granted.
//!
//! ```text
//! quatrelle-cli teams check [--client ID] [--tenant ID-OR-DOMAIN] [--write CHAT] [--files CHAT] [--hour] [--call CHAT]
//! quatrelle-cli teams check --channels [--client ID] [--tenant ID-OR-DOMAIN]
//! ```
//!
//! The tokens live only in this process: nothing is written to disk or to the keychain, and no
//! settings are read.

use std::cell::Cell;
use std::time::Duration;

use quatrelle_engine::model::ical::teams_call_link;
use quatrelle_engine::model::mailto::percent_decode;
use quatrelle_net::NetError;
use quatrelle_net::oauth;
use quatrelle_net::teams::{ChannelCheck, CheckOptions, TeamsClient, run_channel_check, run_check};
use serde_json::Value;

/// Graph's address.
const GRAPH: &str = "https://graph.microsoft.com/v1.0";
/// How long the browser sign-in may take.
const SIGN_IN: Duration = Duration::from_secs(5 * 60);
/// How long `--hour` runs the polling cadences.
const HOUR: Duration = Duration::from_secs(3600);
/// Longest chat id taken.
const MAX_CHAT_ID: usize = 200;
/// Where a link to a chat, or to a message in it (Teams' "Copy link" on a message), points, on
/// either of Teams' web addresses: the chat id is the first segment after these.
const CHAT_LINKS: [&str; 4] = [
    "https://teams.microsoft.com/l/chat/",
    "https://teams.cloud.microsoft/l/chat/",
    "https://teams.microsoft.com/l/message/",
    "https://teams.cloud.microsoft/l/message/",
];
const CLIENT_VAR: &str = "QUATRELLE_TEAMS_CLIENT_ID";
const TENANT_VAR: &str = "QUATRELLE_TEAMS_TENANT";
const USAGE: &str = "usage: quatrelle-cli teams check [--client ID] [--tenant ID-OR-DOMAIN] [--write CHAT] [--files CHAT] [--hour] [--call CHAT], \
                     or quatrelle-cli teams check --channels [--client ID] [--tenant ID-OR-DOMAIN]";
const CONSENT: &str = "Microsoft asks for consent to these permissions: see docs/teams.md, step 4";
/// What means consent is needed: OAuth's codes, and Microsoft's (AADSTS65001 not consented,
/// 65004 consent declined, 70011 a scope refused, 90094 admin consent required).
const CONSENT_CODES: [&str; 6] = ["consent_required", "invalid_scope", "AADSTS65001", "AADSTS65004", "AADSTS70011", "AADSTS90094"];
/// OAuth's error codes (RFC 6749, OpenID Connect), said as they come.
const OAUTH_ERRORS: [&str; 13] = [
    "access_denied",
    "invalid_request",
    "invalid_client",
    "invalid_grant",
    "unauthorized_client",
    "unsupported_grant_type",
    "unsupported_response_type",
    "server_error",
    "temporarily_unavailable",
    "interaction_required",
    "login_required",
    "account_selection_required",
    "invalid_resource",
];

/// What the command line asks for.
#[derive(Default)]
struct Asked {
    client: Option<String>,
    tenant: Option<String>,
    write: Option<String>,
    files: Option<String>,
    hour: bool,
    call: Option<String>,
    /// Teams' teams and channels, read only, instead of the chats.
    channels: bool,
}

/// `teams …`: `args` are what follows `teams`.
pub fn run(args: &[String]) -> Result<(), String> {
    run_with(args, &|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty()).or_else(|| built(name)))
}

/// The client id or tenant built in (set while building, like `QUATRELLE_MICROSOFT_CLIENT_ID`).
fn built(name: &str) -> Option<String> {
    let v = match name {
        CLIENT_VAR => option_env!("QUATRELLE_TEAMS_CLIENT_ID"),
        TENANT_VAR => option_env!("QUATRELLE_TEAMS_TENANT"),
        _ => None,
    };
    v.filter(|v| !v.trim().is_empty()).map(str::to_string)
}

/// [`run`], with the settings read through `env` (the environment, then what was built in).
fn run_with(args: &[String], env: &dyn Fn(&str) -> Option<String>) -> Result<(), String> {
    let asked = parse(args)?;
    let client = asked.client.clone().or_else(|| env(CLIENT_VAR));
    let tenant = asked.tenant.clone().or_else(|| env(TENANT_VAR));
    let (Some(client), Some(tenant)) = (client, tenant) else {
        return Err(format!(
            "the app registration is needed: pass --client <Application (client) ID> and --tenant <Directory (tenant) ID \
             or domain>, or set {CLIENT_VAR} and {TENANT_VAR} (see docs/teams.md)"
        ));
    };
    let mut provider = oauth::teams_provider(&client, &tenant, false)
        .ok_or("--client or --tenant isn't one: the client id is a GUID, the tenant a GUID or a domain (see docs/teams.md)")?;
    if asked.channels {
        provider.scope = channels_scope(&provider.scope);
    }
    let tokens = oauth::authorize(&provider, "", &mut sign_in_page, SIGN_IN).map_err(|e| sign_in_error(&e))?;
    if asked.channels {
        errln("signed in; the channel check starts: a few minutes, reading only (Ctrl+C stops it, nothing is left behind)");
        let check = ChannelCheck {
            tenant: tokens.tenant.clone().filter(|t| quatrelle_engine::model::valid_oauth_tenant(t)),
            granted: tokens.granted.clone(),
            sleep: std::thread::sleep,
        };
        return check_channels_with(GRAPH, &tokens.access, &check, &mut |line| outln(line));
    }
    errln("signed in; the check starts");
    // The refresh token, kept here only, and replaced when Microsoft rotates it.
    let refresh = Cell::new(tokens.refresh.clone());
    let renew = move || {
        let r = refresh.take()?;
        match oauth::refresh(&provider, &r) {
            Ok(t) => {
                refresh.set(t.refresh.or(Some(r)));
                Some(t.access)
            }
            Err(_) => {
                refresh.set(Some(r));
                None
            }
        }
    };
    let opts = CheckOptions {
        tenant: tokens.tenant.clone().filter(|t| quatrelle_engine::model::valid_oauth_tenant(t)),
        write_chat: asked.write,
        files_chat: asked.files,
        hour: asked.hour.then_some(HOUR),
        sleep: std::thread::sleep,
        renew: Some(Box::new(renew)),
    };
    check_with(GRAPH, &tokens.access, &opts, asked.call.as_deref(), &mut launch, &mut |line| outln(line))
}

/// The command line, its chats checked: one that isn't a chat's id or link is refused here,
/// before anything is asked of anyone (and not repeated: it could be anything).
fn parse(args: &[String]) -> Result<Asked, String> {
    let mut it = args.iter();
    if it.next().map(String::as_str) != Some("check") {
        return Err(USAGE.into());
    }
    let mut asked = Asked::default();
    while let Some(a) = it.next() {
        let mut value = || it.next().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).ok_or(format!("{a} needs a value; {USAGE}"));
        match a.as_str() {
            "--client" => asked.client = Some(value()?),
            "--tenant" => asked.tenant = Some(value()?),
            "--write" => asked.write = Some(chat_id(&value()?).ok_or("--write: not a Teams chat's link or id (Teams: a message's … › Copy link)")?),
            "--files" => asked.files = Some(chat_id(&value()?).ok_or("--files: not a Teams chat's link or id (Teams: a message's … › Copy link)")?),
            "--call" => asked.call = Some(chat_id(&value()?).ok_or("--call: not a Teams chat's link or id (Teams: a message's … › Copy link)")?),
            "--hour" => asked.hour = true,
            "--channels" => asked.channels = true,
            _ => return Err(USAGE.into()),
        }
    }
    if asked.channels && (asked.write.is_some() || asked.files.is_some() || asked.hour || asked.call.is_some()) {
        return Err(format!("--channels goes without --write, --files, --hour and --call; {USAGE}"));
    }
    Ok(asked)
}

/// The sign-in's permissions for the channel check: the chats' own, and the channels' to read
/// and to write (those are asked for only to learn whether they are granted: nothing is written).
fn channels_scope(scope: &str) -> String {
    format!("{scope} {} {}", oauth::TEAMS_CHANNELS_SCOPE, oauth::TEAMS_CHANNELS_WRITE_SCOPE)
}

/// The channel check against Graph at `base` with `token`: every line said through `say`.
fn check_channels_with(base: &str, token: &str, check: &ChannelCheck, say: &mut dyn FnMut(&str)) -> Result<(), String> {
    let (client, me) = TeamsClient::connect(base, token).map_err(|e| format!("Graph refused the check: {e}"))?;
    run_channel_check(&client, &me, check, say).map_err(|e| format!("Graph refused the check: {e}"))
}

/// A chat's id, from the id itself (`19:…`), a link to the chat
/// (`https://teams.microsoft.com/l/chat/<id, percent-encoded>/…`) or a link to a message in it
/// (`…/l/message/<chat id>/<message id>?context=…`), the query and fragment ignored. `None` for
/// anything else.
fn chat_id(arg: &str) -> Option<String> {
    let arg = arg.trim();
    let id = match CHAT_LINKS.iter().find_map(|l| arg.strip_prefix(l)) {
        Some(rest) => {
            let rest = rest.split(['?', '#']).next().unwrap_or_default();
            percent_decode(rest.split('/').next().unwrap_or_default())
        }
        None => arg.to_string(),
    };
    let ok = id.len() <= MAX_CHAT_ID
        && id.len() > 3
        && id.starts_with("19:")
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'@' | b'-'));
    ok.then_some(id)
}

/// Teams' link to call the people given (their sign-in names), but `me`: the model's
/// ([`teams_call_link`]); `None` when nobody is left or the link couldn't be made.
fn call_link(me: &str, upns: &[String]) -> Option<String> {
    let others: Vec<&str> = upns.iter().map(|u| u.trim()).filter(|u| !u.is_empty() && !u.eq_ignore_ascii_case(me.trim())).collect();
    teams_call_link(&others, false)
}

/// Why [`call_link`] gave none.
fn why_no_call(me: &str, upns: &[String]) -> &'static str {
    if upns.iter().any(|u| !u.trim().is_empty() && !u.trim().eq_ignore_ascii_case(me.trim())) {
        "the chat has too many members for a call link"
    } else {
        "nobody else in the chat has an address"
    }
}

/// A failed sign-in, said without Microsoft's words (which can name the account): consent to
/// ask for, or the error's codes.
fn sign_in_error(e: &NetError) -> String {
    let NetError::Auth(text) = e else { return format!("sign-in: {e}") };
    if CONSENT_CODES.iter().any(|c| text.contains(c)) {
        return CONSENT.into();
    }
    if text.contains("wasn't finished in time") {
        return "the sign-in wasn't finished within 5 minutes".into();
    }
    // `AADSTS50011`-like codes, and OAuth's error codes (`access_denied`).
    let codes: Vec<&str> = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| {
            (w.len() > 6 && w.len() <= 12 && w.strip_prefix("AADSTS").is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit())))
                || OAUTH_ERRORS.contains(w)
        })
        .collect();
    if codes.is_empty() {
        "the sign-in was refused (see docs/teams.md)".into()
    } else {
        format!("the sign-in was refused: {} (see docs/teams.md)", codes.join(", "))
    }
}

/// The check against Graph at `base` with `token`, then the call, if asked: every line said
/// through `say`; `open` opens the call's link.
fn check_with(
    base: &str,
    token: &str,
    opts: &CheckOptions,
    call: Option<&str>,
    open: &mut dyn FnMut(&str) -> Result<(), String>,
    say: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let (client, me) = TeamsClient::connect(base, token).map_err(|e| format!("Graph refused the check: {e}"))?;
    run_check(&client, &me, opts, say).map_err(|e| format!("Graph refused the check: {e}"))?;
    if let Some(chat) = call {
        let said = match client.members(chat) {
            Err(e) => format!("call: no (the chat's members: {})", status(&e)),
            Ok(members) => {
                let upns: Vec<String> = members
                    .iter()
                    .filter(|m| m.get("userId").and_then(Value::as_str) != Some(me.id.as_str()))
                    .filter_map(|m| m.get("email").and_then(Value::as_str).map(str::to_string))
                    .collect();
                let mine = if me.upn.is_empty() { me.mail.as_str() } else { me.upn.as_str() };
                match call_link(mine, &upns) {
                    None => format!("call: no ({})", why_no_call(mine, &upns)),
                    Some(link) => match open(&link) {
                        Ok(()) => "call: opened".into(),
                        Err(e) => format!("call: no ({e})"),
                    },
                }
            }
        };
        say(&said);
    }
    Ok(())
}

/// An error by its HTTP status alone (its text can carry the address asked).
fn status(e: &NetError) -> String {
    let text = e.to_string();
    text.split("HTTP ")
        .skip(1)
        .map(|r| r.chars().take_while(char::is_ascii_digit).collect::<String>())
        .find(|s| s.len() == 3)
        .unwrap_or_else(|| "failed".into())
}

/// Show Microsoft's sign-in page: its address on the terminal, and the browser if it starts.
fn sign_in_page(url: &str) -> Result<(), String> {
    errln(&format!("Open this address to sign in: {url}"));
    let _ = launch(url);
    Ok(())
}

/// Open an address in the system's browser, without a shell.
fn launch(url: &str) -> Result<(), String> {
    // Windows: the URL handler itself (`explorer` can split an address at its commas, and a
    // call link has them); the address goes as one argument, with no shell between. rundll32 is
    // Windows' own, by its full path: a `rundll32.exe` beside a portable copy would run.
    #[cfg(windows)]
    let (opener, first): (std::path::PathBuf, &[&str]) = (crate::windows_program::path("rundll32.exe"), &["url.dll,FileProtocolHandler"]);
    #[cfg(not(windows))]
    let (opener, first): (&str, &[&str]) = if cfg!(target_os = "macos") { ("open", &[]) } else { ("xdg-open", &[]) };
    let mut child = std::process::Command::new(opener)
        .args(first)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "the browser couldn't be started".to_string())?;
    // Reaped in the background: the hour may run long after.
    std::thread::spawn(move || child.wait());
    Ok(())
}

fn outln(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stdout(), "{}", crate::printable(line));
}

fn errln(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "{}", crate::printable(line));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_id_from_raw_and_links() {
        let id = "19:abc_def@unq.gbl.spaces";
        assert_eq!(chat_id(id).as_deref(), Some(id));
        assert_eq!(chat_id("https://teams.microsoft.com/l/chat/19%3Aabc_def%40unq.gbl.spaces/0?tenantId=x#y").as_deref(), Some(id));
        assert_eq!(chat_id(" https://teams.microsoft.com/l/chat/19:abc_def@unq.gbl.spaces ").as_deref(), Some(id));
        assert_eq!(chat_id("19:meeting_NjQ3ZGE@thread.v2").as_deref(), Some("19:meeting_NjQ3ZGE@thread.v2"));
        // A message's "Copy link": the chat id is the first segment, the message id the second.
        let message =
            "https://teams.microsoft.com/l/message/19%3Aabc_def%40unq.gbl.spaces/1730000000000?context=%7B%22contextType%22%3A%22chat%22%7D";
        assert_eq!(chat_id(message).as_deref(), Some(id));
        assert_eq!(chat_id("https://teams.cloud.microsoft/l/message/19:abc_def@unq.gbl.spaces/1730000000000").as_deref(), Some(id));
        let long = format!("19:{}", "a".repeat(198));
        assert_eq!(long.len(), 201);
        for bad in [
            "https://evil.example/l/chat/19:a@b/0",
            "http://teams.microsoft.com/l/chat/19:a@b/0",
            "https://teams.microsoft.com.evil.example/l/chat/19:a@b/0",
            "https://teams.microsoft.com/l/call/19:a@b/0",
            "https://evil.example/l/message/19:a@b/1730000000000",
            "https://teams.microsoft.com.evil.example/l/message/19:a@b/1730000000000",
            "http://teams.cloud.microsoft/l/message/19:a@b/1730000000000",
            "https://teams.microsoft.com/l/message/48:notes/1730000000000",
            "48:notes",
            "https://teams.microsoft.com/l/chat/19%3Aa%20b/0",
            "19:a b",
            "\"; rm -rf /\"",
            "19:",
            "20:abc@thread.v2",
            "19:a/../b",
            long.as_str(),
            "",
        ] {
            assert_eq!(chat_id(bad), None, "{bad}");
        }
        assert!(chat_id(&long[..200]).is_some());
    }

    #[test]
    fn call_link_excludes_me_and_is_launchable() {
        let upns = vec!["ana@contoso.example".to_string(), "Me@Contoso.example".into(), "bruno+x@contoso.example".into()];
        let link = call_link("me@contoso.example", &upns).unwrap();
        assert_eq!(link, "https://teams.microsoft.com/l/call/0/0?users=ana%40contoso.example,bruno%2Bx%40contoso.example");
        assert!(quatrelle_engine::model::ical::launchable_link(&link));
        assert_eq!(call_link("me@contoso.example", &["me@contoso.example".to_string()]), None);
        assert_eq!(call_link("me@contoso.example", &[]), None);
        assert_eq!(call_link("me@contoso.example", &[String::new()]), None);
        // The model's link: an address that could add an option or someone is refused whole, a
        // person counts once whatever the case.
        assert_eq!(call_link("me@contoso.example", &["ana@contoso.example&withVideo=true".to_string()]), None);
        assert_eq!(
            call_link("me@contoso.example", &["Ana@contoso.example".to_string(), "ana@CONTOSO.example".into()]).as_deref(),
            Some("https://teams.microsoft.com/l/call/0/0?users=Ana%40contoso.example")
        );
        // A big group's link is too long to open: said as such, not as nobody to call.
        let many: Vec<String> = (0..120).map(|i| format!("someone.with.a.long.name{i}@contoso.example")).collect();
        assert_eq!(call_link("me@contoso.example", &many), None);
        assert_eq!(why_no_call("me@contoso.example", &many), "the chat has too many members for a call link");
        assert_eq!(why_no_call("me@contoso.example", &[]), "nobody else in the chat has an address");
    }

    #[test]
    fn missing_client_or_tenant_says_how() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for given in
            [args(&["check"]), args(&["check", "--client", "11111111-2222-3333-4444-555555555555"]), args(&["check", "--tenant", "contoso.example"])]
        {
            let e = run_with(&given, &|_| None).unwrap_err();
            assert!(e.contains("docs/teams.md") && e.contains("--client") && e.contains("QUATRELLE_TEAMS_TENANT"), "{e}");
        }
        // A chat that isn't one is refused first, before anything is asked of anyone.
        let e = run_with(&args(&["check", "--write", "https://evil.example/l/chat/19:a@b/0"]), &|_| None).unwrap_err();
        assert!(e.contains("--write") && !e.contains("evil"), "{e}");
        // Junk for a client id or tenant is refused before the browser opens.
        let e = run_with(&args(&["check", "--client", "x y", "--tenant", "contoso.example"]), &|_| None).unwrap_err();
        assert!(e.contains("docs/teams.md"), "{e}");
        assert!(run_with(&args(&["chek"]), &|_| None).unwrap_err().contains("teams check"));
        assert!(run_with(&args(&["check", "--bogus"]), &|_| None).unwrap_err().contains("teams check"));
    }

    #[test]
    fn files_takes_a_chat_as_write_does() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let id = "19:abc_def@unq.gbl.spaces";
        let link = "https://teams.microsoft.com/l/chat/19%3Aabc_def%40unq.gbl.spaces/0?tenantId=x";
        assert_eq!(parse(&args(&["check", "--files", link])).unwrap().files.as_deref(), Some(id));
        assert_eq!(parse(&args(&["check", "--files", id, "--write", id])).unwrap().files.as_deref(), Some(id));
        assert_eq!(parse(&args(&["check"])).unwrap().files, None);
        // Without a chat, or with something that isn't a Teams chat's id or link: refused before
        // anything is asked of anyone, and what was given isn't repeated.
        let e = parse(&args(&["check", "--files"])).err().unwrap();
        assert!(e.contains("--files needs a value") && e.contains("--files CHAT"), "{e}");
        for bad in ["https://evil.example/l/chat/19:a@b/0", "48:notes", "x y", ""] {
            let e = run_with(&args(&["check", "--files", bad]), &|_| None).unwrap_err();
            assert!(e.contains("--files") && !e.contains("evil") && !e.contains("x y"), "{e}");
        }
        assert!(USAGE.contains("[--files CHAT]"));
    }

    #[test]
    fn consent_errors_point_to_step_4() {
        let consent = "Microsoft asks for consent to these permissions: see docs/teams.md, step 4";
        for text in [
            // As `token_request` says them: the provider's name, then `error_description`.
            "Microsoft Teams: AADSTS70011: The provided value for the input parameter 'scope' is not valid. Trace ID: x",
            "Microsoft Teams: AADSTS90094: An administrator of Contoso has set a policy that prevents you from granting Quatrelle the permissions it is requesting.",
            // As `authorize` says a refused redirect: its `error`, then the AADSTS code of its description.
            "Microsoft Teams sign-in was cancelled (access_denied, AADSTS65004)",
            "Microsoft Teams sign-in was cancelled (access_denied, AADSTS90094)",
            "Microsoft Teams: AADSTS65001: The user or administrator has not consented to use the application with ID 'x'.",
            "Microsoft Teams sign-in was cancelled (consent_required)",
            "Microsoft Teams: AADSTS70011: The provided request must include a 'scope' input parameter. (invalid_scope)",
        ] {
            assert_eq!(sign_in_error(&NetError::Auth(text.into())), consent, "{text}");
        }
        // Other refusals: their codes, never Microsoft's words (which can name the account).
        let e = sign_in_error(&NetError::Auth("Microsoft Teams: AADSTS50020: User account 'ana@contoso.example' from identity provider".into()));
        assert!(e.contains("AADSTS50020") && !e.contains("ana"), "{e}");
        let e = sign_in_error(&NetError::Auth("Microsoft Teams: AADSTS50020: User account 'ana_lima@contoso.example' (invalid_grant)".into()));
        assert_eq!(e, "the sign-in was refused: AADSTS50020, invalid_grant (see docs/teams.md)");
        let e = sign_in_error(&NetError::Auth("Microsoft Teams sign-in was cancelled (access_denied)".into()));
        assert!(e.contains("access_denied"), "{e}");
        assert!(sign_in_error(&NetError::Auth("the sign-in wasn't finished in time".into())).contains("5 minutes"));
    }

    #[test]
    fn check_against_the_fake_says_numbers_only() {
        let fake = quatrelle_net::teams_sample();
        let opts = CheckOptions { tenant: None, write_chat: Some(fake.chat.into()), files_chat: None, hour: None, sleep: |_| {}, renew: None };
        let (mut lines, mut opened) = (Vec::new(), Vec::new());
        check_with(
            &fake.base,
            fake.token,
            &opts,
            Some(fake.chat),
            &mut |u| {
                opened.push(u.to_string());
                Ok(())
            },
            &mut |l| lines.push(l.to_string()),
        )
        .unwrap();
        let has = |start: &str| lines.iter().any(|l| l.starts_with(start));
        assert!(lines.iter().any(|l| l == "chats: 2 (one-to-one 1, group 1, meeting 0, other 0)"), "{lines:#?}");
        assert!(lines.iter().any(|l| l == "messages in the newest chat read: 4"), "{lines:#?}");
        assert!(lines.iter().any(|l| l == "reactions: 2 on 1 messages (by name 1, as emoji 1, other 0)"), "{lines:#?}");
        assert!(has("file attachments: 1, the first downloads: yes, 9 bytes"), "{lines:#?}");
        assert!(lines.iter().any(|l| l == "write, send (1/2): yes"), "{lines:#?}");
        assert!(lines.iter().any(|l| l == "write, test messages left undeleted: 0"), "{lines:#?}");
        assert!(lines.iter().any(|l| l == "call: opened"), "{lines:#?}");
        assert!(has("in all: requests "), "{lines:#?}");
        for l in &lines {
            for p in &fake.private {
                assert!(!l.contains(p), "{p:?} in {l:?}");
            }
        }
        // The call is with the two others, not with me.
        assert_eq!(opened, ["https://teams.microsoft.com/l/call/0/0?users=banana%40contoso.example,pudding%40contoso.example"]);
    }

    #[test]
    fn channels_goes_alone_and_asks_for_the_channels_permissions() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let id = "19:abc_def@unq.gbl.spaces";
        assert!(parse(&args(&["check", "--channels"])).unwrap().channels);
        assert!(
            parse(&args(&["check", "--client", "11111111-2222-3333-4444-555555555555", "--channels", "--tenant", "contoso.example"]))
                .unwrap()
                .channels
        );
        assert!(!parse(&args(&["check"])).unwrap().channels);
        // With anything that writes, sends a file, runs an hour or calls: refused before anything
        // is asked of anyone.
        for other in [&["--write", id][..], &["--files", id], &["--hour"], &["--call", id]] {
            let mut given = args(&["check", "--channels"]);
            given.extend(args(other));
            let e = run_with(&given, &|_| None).unwrap_err();
            assert!(e.starts_with("--channels goes without --write, --files, --hour and --call") && e.contains("teams check --channels"), "{e}");
            let mut given = args(&["check"]);
            given.extend(args(other));
            given.push("--channels".into());
            assert!(parse(&given).is_err(), "{given:?}");
        }
        assert!(USAGE.contains("[--files CHAT]") && USAGE.contains("teams check --channels [--client ID]"));
        // The chats' permissions, then the channels' six.
        let scope = channels_scope(oauth::TEAMS_SCOPE);
        assert!(scope.starts_with(oauth::TEAMS_SCOPE));
        for p in [
            "Team.ReadBasic.All",
            "Channel.ReadBasic.All",
            "ChannelMessage.Read.All",
            "ChannelMember.Read.All",
            "ChannelMessage.Send",
            "ChannelMessage.ReadWrite",
        ] {
            assert!(scope.split(' ').any(|s| s == format!("https://graph.microsoft.com/{p}")), "{p}: {scope}");
        }
        assert!(!scope.contains("OnlineMeetings"), "{scope}");
    }

    #[test]
    fn check_channels_against_the_fake_says_numbers_only() {
        let fake = quatrelle_net::teams_sample();
        let granted = format!("{} {} {}", oauth::TEAMS_SCOPE, oauth::TEAMS_CHANNELS_SCOPE, oauth::TEAMS_CHANNELS_WRITE_SCOPE);
        let check = ChannelCheck { tenant: Some("tenant-quixote".into()), granted, sleep: |_| {} };
        let mut lines = Vec::new();
        check_channels_with(&fake.base, fake.token, &check, &mut |l| lines.push(l.to_string())).unwrap();
        let has = |l: &str| lines.iter().any(|x| x == l);
        assert!(has("teams: 1 (archived 0)"), "{lines:#?}");
        assert!(has("channel messages read: 2 of 2 channels (refused 0; why: -)"), "{lines:#?}");
        assert!(has("hosted images: channel form 1, channel reply form 0, reply as a message 0, chat form 0, other 0"), "{lines:#?}");
        assert!(has("mentions: person 1 (the user 1), channel 1, team 0, tag 0, other 0"), "{lines:#?}");
        assert!(lines.iter().any(|l| l.starts_with("channel members: read 2 of 2 (refused 0; why: -); organisation 4, guests 2,")), "{lines:#?}");
        assert!(lines.last().is_some_and(|l| l.starts_with("in all: requests ")), "{lines:#?}");
        for l in &lines {
            for p in &fake.private {
                assert!(!l.contains(p), "{p:?} in {l:?}");
            }
            assert!(l.is_ascii() && !l.contains('@') && !l.contains("//"), "{l:?}");
        }
        // It read, and only read.
        assert_eq!(fake.writes_asked(), 0);
    }

    #[test]
    fn a_refused_token_is_said_by_its_status() {
        let fake = quatrelle_net::teams_sample();
        let opts = CheckOptions { tenant: None, write_chat: None, files_chat: None, hour: None, sleep: |_| {}, renew: None };
        let mut lines = Vec::new();
        let e = check_with(&fake.base, "not-the-token", &opts, None, &mut |_| Ok(()), &mut |l| lines.push(l.to_string())).unwrap_err();
        assert!(e.contains("401"), "{e}");
        assert!(lines.is_empty());
    }
}
