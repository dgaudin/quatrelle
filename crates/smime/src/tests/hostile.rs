// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Trying to break it: signatures made to take long, and chains an authority wasn't allowed to
//! make.

use cms::content_info::ContentInfo;
use cms::signed_data::{SignedData, SignerInfos};
use der::asn1::{OctetString, SetOfVec};
use der::{Any, Decode, Encode};

use super::*;

/// `cms` (a signature of Riley's) with `more` signers added whose signatures don't hold.
fn with_signers(cms: &[u8], more: u32) -> Vec<u8> {
    let ci = ContentInfo::from_der(cms).unwrap();
    let mut sd: SignedData = ci.content.decode_as().unwrap();
    let real = sd.signer_infos.0.iter().next().cloned().unwrap();
    let mut signers = SetOfVec::new();
    signers.insert(real.clone()).unwrap();
    for n in 0..more {
        let mut other = real.clone();
        other.signature = OctetString::new(n.to_be_bytes().to_vec()).unwrap();
        signers.insert(other).unwrap();
    }
    sd.signer_infos = SignerInfos(signers);
    let content = Any::from_der(&sd.to_der().unwrap()).unwrap();
    ContentInfo { content_type: oids::SIGNED_DATA, content }.to_der().unwrap()
}

/// A signature may name signers by the hundred, each one a reason to hash the whole message
/// again: megabytes hashed hundreds of times, where the message is shown. No mail has more than
/// a few; past eight it isn't a signature Quatrelle reads, and the content is hashed once.
#[test]
fn signers_by_the_hundred_arent_each_hashed_for() {
    let content = vec![b'a'; 2 << 20];
    let cms = sign(&content, &riley(), NOW, &mut rng()).unwrap();
    assert!(verify(&with_signers(&cms, 7), Some(&content)).is_ok(), "a few signers, one that holds");
    let many = with_signers(&cms, 300);
    assert_eq!(verify(&many, Some(&content)).err(), Some(Error::Malformed("the signature names too many signers")));
}

/// When the chain fixtures (`chain-*.pem`, a root of their own) are in date.
const CHAIN_NOW: i64 = 1_800_000_000;

/// What's wrong with trusting `leaf` for `from`, under the chain test root, through `between`.
fn chain_problems(leaf: &str, between: &[&str], from: &str) -> Vec<cert::Problem> {
    let others: Vec<Vec<u8>> = between.iter().map(|n| pem(n)).collect();
    cert::check(&pem(leaf), &others, &[pem("chain-root.pem")], CHAIN_NOW, from).unwrap()
}

/// An authority signs only what it was allowed to (RFC 5280 §6): for the addresses its own
/// certificate names, as deep as it says, with a key that may sign certificates, for email. A
/// certificate it had no right to make isn't from an authority this computer trusts.
#[test]
fn an_authority_signs_only_what_it_was_allowed_to() {
    let untrusted = vec![cert::Problem::UnknownIssuer];
    // An ordinary chain, and one whose authority is kept to its own domain, used within it.
    assert_eq!(chain_problems("chain-ana.pem", &["chain-ca-ok.pem"], "ana@studio.example"), []);
    assert_eq!(chain_problems("chain-names-ana.pem", &["chain-ca-names.pem"], "ana@studio.example"), []);
    // Kept to studio.example, it made a certificate for another domain's director; and for an
    // address it was told to leave out.
    assert_eq!(chain_problems("chain-names-director.pem", &["chain-ca-names.pem"], "director@victim.example"), untrusted);
    assert_eq!(chain_problems("chain-names-eve.pem", &["chain-ca-names.pem"], "eve@studio.example"), untrusted);
    // An authority for web servers only.
    assert_eq!(chain_problems("chain-tls-director.pem", &["chain-ca-tls.pem"], "director@victim.example"), untrusted);
    // One whose key may not sign certificates.
    assert_eq!(chain_problems("chain-nosign-director.pem", &["chain-ca-nosign.pem"], "director@victim.example"), untrusted);
    // One that may have no authority under it, and has one.
    assert_eq!(chain_problems("chain-deep-director.pem", &["chain-ca-deep.pem", "chain-ca-short.pem"], "director@victim.example"), untrusted);
    // A certificate that says, critically, something Quatrelle doesn't read.
    assert_eq!(chain_problems("chain-critical.pem", &["chain-ca-ok.pem"], "ana@studio.example"), vec![cert::Problem::NotForEmail]);
}
