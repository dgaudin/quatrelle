// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The banners above a message: who sent it doesn't add up (`sender_check`), and the language
//! model thinks it is a scam (`scam`).

use super::*;

mod scam;
mod sender_check;

pub(crate) use scam::{scam_banner, scam_headline};
pub(crate) use sender_check::{external_tag, sender_warning};
