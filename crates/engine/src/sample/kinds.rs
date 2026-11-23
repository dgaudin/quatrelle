// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The sample Inbox's messages that are not ordinary mail: reports, receipts, an automatic reply,
//! meeting mail and a message replied to all, whose rows say what they are in the list.

use super::*;

const FROM: &str = "From: Mail System <postmaster@studio.example>\r\nTo: Riley Moreno <riley@studio.example>\r\n";

/// A delivery report (RFC 3464) or a receipt (RFC 8098), its status block as given.
fn report(subject: &str, kind: &str, status: &str) -> String {
    format!(
        "{FROM}Subject: {subject}\r\nContent-Type: multipart/report; report-type={kind}; boundary=r\r\n\r\n\
         --r\r\nContent-Type: text/plain\r\n\r\n{subject}.\r\n\
         --r\r\nContent-Type: message/{kind}\r\n\r\n{status}\r\n--r--\r\n"
    )
}

fn delivery(subject: &str, action: &str) -> String {
    report(
        subject,
        "delivery-status",
        &format!("Reporting-MTA: dns; mx.studio.example\r\n\r\nFinal-Recipient: rfc822; sam@studio.example\r\nAction: {action}\r\nStatus: 2.0.0\r\n"),
    )
}

fn receipt(subject: &str, disposition: &str) -> String {
    report(
        subject,
        "disposition-notification",
        &format!("Final-Recipient: rfc822; sam@studio.example\r\nDisposition: manual-action/MDN-sent-manually; {disposition}\r\n"),
    )
}

/// A calendar mail about a meeting, from `from` (the guest, for an answer).
fn meeting(subject: &str, from: &str, method: &str, partstat: &str) -> String {
    format!(
        "From: {from}\r\nTo: riley@studio.example\r\nSubject: {subject}\r\nContent-Type: text/calendar; method={method}\r\n\r\n\
         BEGIN:VCALENDAR\r\nMETHOD:{method}\r\nBEGIN:VEVENT\r\nUID:kinds-1@studio.example\r\nDTSTART:20261201T100000Z\r\nDTEND:20261201T110000Z\r\n\
         SUMMARY:Colour review\r\nATTENDEE;PARTSTAT={partstat}:mailto:{from}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

/// The messages, newest first in the order given, each with the flags it carries.
pub(super) fn mails() -> Vec<(String, Flags)> {
    let seen = Flags { seen: true, ..Default::default() };
    let all = Flags { seen: true, answered: true, keywords: vec![quatrelle_model::REPLIED_ALL.into()], ..Default::default() };
    let guest = "sam@studio.example";
    let auto = "From: Sam Okafor <sam@studio.example>\r\nTo: riley@studio.example\r\nSubject: Automatic reply: away until Monday\r\nAuto-Submitted: auto-replied\r\n\r\nI am away until Monday.\r\n";
    [
        (delivery("Undeliverable: Colour key", "failed"), &seen),
        (delivery("Delayed: Festival poster", "delayed"), &seen),
        (delivery("Delivered: Storyboard", "delivered"), &seen),
        (receipt("Read: Storyboard review", "displayed"), &seen),
        (receipt("Deleted unread: Storyboard review", "deleted"), &seen),
        (auto.to_string(), &seen),
        (meeting("Canceled: Colour review", "casey@studio.example", "CANCEL", "NEEDS-ACTION"), &seen),
        (meeting("Accepted: Colour review", guest, "REPLY", "ACCEPTED"), &seen),
        (meeting("Tentative: Colour review", guest, "REPLY", "TENTATIVE"), &seen),
        (meeting("Declined: Colour review", guest, "REPLY", "DECLINED"), &seen),
        ("From: Casey Lin <casey@studio.example>\r\nTo: riley@studio.example, sam@studio.example\r\nSubject: Re: Colour review\r\n\r\nSee you both there.\r\n".to_string(), &all),
    ]
    .into_iter()
    .map(|(raw, flags)| (raw, flags.clone()))
    .collect()
}
