// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
use super::*;

fn pair(a: &str, b: &str) -> Vec<(String, String)> {
    vec![(a.to_string(), b.to_string())]
}

#[test]
fn a_shown_domain_that_isnt_the_targets_is_misleading() {
    let found = misleading_links(&pair("www.ma-banque.fr", "https://evil.example/login"));
    assert_eq!(found, vec![Sign::MisleadingLink { shown: "www.ma-banque.fr".into(), target: "https://evil.example/login".into() }]);
    assert!(misleading_links(&pair("Cliquez ici", "https://tracker.example/abc")).is_empty());
    assert!(misleading_links(&pair("ma-banque.fr", "https://www.ma-banque.fr/x")).is_empty());
    assert!(misleading_links(&pair("https://bank.example/a", "https://login.bank.example/b")).is_empty());
    assert!(misleading_links(&pair("bank.example.evil.example", "https://bank.example/")).len() == 1);
    assert!(misleading_links(&pair("e.g.", "https://x.example/")).is_empty());
    let many: Vec<(String, String)> = (0..20).map(|i| (format!("a{i}.example"), "https://z.example/".to_string())).collect();
    assert_eq!(misleading_links(&many).len(), 5);
}

#[test]
fn risky_attachments_are_named() {
    let names: Vec<String> = ["facture.pdf.exe", "photo.JPG", "a.docm", "page.htm", "notes.txt"].map(String::from).to_vec();
    let found = risky_attachments(&names);
    assert_eq!(
        found,
        vec![Sign::RiskyAttachment("facture.pdf.exe".into()), Sign::RiskyAttachment("a.docm".into()), Sign::RiskyAttachment("page.htm".into())]
    );
    assert!(risky_attachments(&["noext".to_string(), String::new(), ".".to_string()]).is_empty());
}

#[test]
fn failed_authentication_is_read() {
    let h = "Authentication-Results: mx.example; spf=fail smtp.mailfrom=x; dkim=pass; dmarc=fail\r\n";
    assert_eq!(auth_failures(h), vec![Sign::AuthFailed("spf"), Sign::AuthFailed("dmarc")]);
    assert!(auth_failures("Received: from x (spf=fail)\r\n").is_empty());
    let folded = "Authentication-Results: mx.example;\r\n\tdkim=fail header.d=x.example\r\nSubject: s\r\n";
    assert_eq!(auth_failures(folded), vec![Sign::AuthFailed("dkim")]);
    assert!(auth_failures(&"é".repeat(9000)).is_empty());
}
