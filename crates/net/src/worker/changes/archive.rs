// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The local archive in an account's worker: IMAP mail copied into the Archives account on this
//! computer, checked, and only then deleted on the server.

mod copy;

pub(crate) use copy::copy_checked;
