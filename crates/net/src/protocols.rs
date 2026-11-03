// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
//! Mail and calendar protocols, each over a stream: IMAP, POP, SMTP, ManageSieve, JMAP,
//! CalDAV/CardDAV, and a calendar published at a link.

pub mod dav;
pub mod imap;
pub mod jmap;
pub mod pop;
pub mod sieve;
pub mod smtp;
pub mod web_calendar;
