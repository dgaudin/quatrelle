// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What a message is, as the list shows it: what protects it (S/MIME or OpenPGP, as its header
//! says).

use serde::{Deserialize, Serialize};

/// What a message's header says protects it: S/MIME (RFC 8551 §3.2) or OpenPGP (RFC 3156).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Smime {
    #[default]
    None,
    Signed,
    Encrypted,
}

impl Smime {
    pub fn is_none(&self) -> bool {
        *self == Smime::None
    }

    /// From a message's top-level `Content-Type`: its type and subtype, and its `protocol`,
    /// `smime-type` and `name` parameters.
    pub fn from_content_type(kind: &str, subtype: &str, protocol: Option<&str>, smime_type: Option<&str>, name: Option<&str>) -> Smime {
        let (kind, subtype) = (kind.to_ascii_lowercase(), subtype.to_ascii_lowercase());
        let lower = |v: Option<&str>| v.map(|s| s.trim().to_ascii_lowercase()).unwrap_or_default();
        if kind == "multipart" && subtype == "signed" {
            return match lower(protocol).as_str() {
                "application/pkcs7-signature" | "application/x-pkcs7-signature" | "application/pgp-signature" => Smime::Signed,
                _ => Smime::None,
            };
        }
        // PGP/MIME (RFC 3156) shows the same way.
        if kind == "multipart" && subtype == "encrypted" && lower(protocol) == "application/pgp-encrypted" {
            return Smime::Encrypted;
        }
        let p7m = lower(name).ends_with(".p7m");
        if kind == "application" && (subtype == "pkcs7-mime" || subtype == "x-pkcs7-mime" || (subtype == "octet-stream" && p7m)) {
            return if lower(smime_type) == "signed-data" { Smime::Signed } else { Smime::Encrypted };
        }
        Smime::None
    }
}
