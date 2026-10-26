//! Engine tests: who a message is from (Outlook's From): a shared mailbox, another address.

use super::*;

/// The From button offers the mailboxes an account opens; a reply in a shared mailbox's folder
/// goes from that mailbox, and its draft keeps it. An address typed in is checked, the
/// account's own is no other address, and one sent from is offered next time.
#[test]
fn a_reply_in_a_shared_mailbox_goes_from_it() {
    let mut s = sample_session("send-from");
    assert_eq!(s.senders().iter().filter(|x| x.account == "sample").count(), 1, "the account's own address alone");
    // The Inbox as a shared mailbox's (Exchange marks such folders).
    for f in s.accounts.iter_mut().flat_map(|a| a.folders.iter_mut()).filter(|f| f.path == "INBOX") {
        f.mailbox = Some("support@studio.example".into());
    }
    assert!(s.senders().iter().any(|x| x.account == "sample" && x.from == "support@studio.example"));
    let poster = key_of(&s, "Your proof for the festival poster is ready");
    s.run("list.open", &poster).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().from, "support@studio.example");
    s.run("compose.update", &json!({"body": "On it.\n"})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.compose = None;
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    let draft = key_of(&s, "Re: Your proof for the festival poster is ready");
    s.run("draft.edit", &draft).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().from, "support@studio.example", "the draft keeps it");
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let sent = s.list.iter().find(|m| m.subject == "Re: Your proof for the festival poster is ready").unwrap();
    assert_eq!(sent.from.email, "support@studio.example", "sent from it");
    assert!(s.account("sample").unwrap().config.send_from.is_empty(), "a mailbox the account opens is offered anyway");

    s.run("mail.new", &json!({})).unwrap();
    assert!(s.run("compose.update", &json!({"from": "not an address"})).is_err());
    s.run("compose.update", &json!({"from": "Riley <Riley@studio.example>"})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().from, "", "the account's own address");
    s.run("compose.update", &json!({"from": "Bookings <bookings@studio.example>", "to": "zoe@example.org", "subject": "Dates"})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    assert_eq!(s.account("sample").unwrap().config.send_from, ["Bookings <bookings@studio.example>"]);
    assert!(s.senders().iter().any(|x| x.from == "Bookings <bookings@studio.example>"), "offered next time");
    done(s);
}
