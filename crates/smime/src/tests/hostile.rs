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
