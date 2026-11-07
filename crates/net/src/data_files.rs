// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
//! Outlook data files (`.pst`): read into an account on this computer, and written out.

pub mod msg;
pub mod pst;
#[cfg(test)]
mod pst_bench;
pub mod pst_export;
pub(crate) mod pst_pim;
