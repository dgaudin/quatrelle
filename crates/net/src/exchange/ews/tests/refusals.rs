// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! EWS: a refusal says which fields Exchange named in its `MessageXml`, as far as they look
//! like names (an answer refused at every sync on 04/11, "Set action is invalid for
//! property.", the field it was about dropped).

use super::*;

const TEXT: &str = "Set action is invalid for property.";

/// What `check_message` says of an `ErrorInvalidPropertySet` refusal carrying this
/// `MessageXml` (none when empty).
fn said(xml: &str) -> String {
    let more = if xml.is_empty() { String::new() } else { format!("<m:MessageXml>{xml}</m:MessageXml>") };
    let doc = parse(&format!(
        "<s:Envelope xmlns:s=\"x\"><s:Body><m:CreateItemResponse><m:ResponseMessages>\
         <m:CreateItemResponseMessage ResponseClass=\"Error\"><m:MessageText>{TEXT}</m:MessageText>\
         <m:ResponseCode>ErrorInvalidPropertySet</m:ResponseCode>{more}<m:DescriptiveLinkKey>0</m:DescriptiveLinkKey><m:Items/>\
         </m:CreateItemResponseMessage></m:ResponseMessages></m:CreateItemResponse></s:Body></s:Envelope>"
    ));
    match messages(&doc).first().map(|m| check_message(m)) {
        Some(Err(NetError::No(text))) => text,
        other => panic!("{other:?}"),
    }
}

fn naming(names: &str) -> String {
    format!("{TEXT} (ErrorInvalidPropertySet: {names})")
}

#[test]
fn a_refusal_names_the_fields() {
    assert_eq!(said("<t:FieldURI FieldURI=\"calendar:Organizer\"/>"), naming("calendar:Organizer"));
    // Extended fields by their tag or name, and indexed ones, the same way, in Exchange's order.
    let kinds = "<t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/>\
                 <t:IndexedFieldURI FieldURI=\"contacts:EmailAddress\" FieldIndex=\"EmailAddress1\"/>\
                 <t:ExtendedFieldURI DistinguishedPropertySetId=\"PublicStrings\" PropertyName=\"Keywords\" PropertyType=\"String\"/>";
    assert_eq!(said(kinds), naming("0x1090, contacts:EmailAddress, Keywords"));
    // Ten fields: the first three.
    let ten: String = (0..10).map(|i| format!("<t:FieldURI FieldURI=\"item:F{i}\"/>")).collect();
    assert_eq!(said(&ten), naming("item:F0, item:F1, item:F2"));
    // The server's words: only what looks like a name is said (80 bytes at most, nothing but
    // ASCII letters, digits and `:_-.`).
    let (long, longest) = ("a".repeat(81), "b".repeat(80));
    let odd = format!(
        "<t:FieldURI FieldURI=\"{long}\"/><t:FieldURI FieldURI=\"a b\"/><t:FieldURI FieldURI=\"x&lt;y\"/>\
         <t:FieldURI FieldURI=\"caf\u{e9}:x\"/><t:FieldURI FieldURI=\"\"/><t:FieldURI/><t:FieldURI FieldURI=\"item:Subject\"/>"
    );
    assert_eq!(said(&odd), naming("item:Subject"));
    assert_eq!(said(&format!("<t:FieldURI FieldURI=\"{longest}\"/>")), naming(&longest), "80 bytes is a name still");
    // None named: the text as it always was.
    assert_eq!(said(""), TEXT);
    assert_eq!(said("<t:Value Name=\"BackOffMilliseconds\">10</t:Value>"), TEXT);
    assert_eq!(said(&format!("<t:FieldURI FieldURI=\"{long}\"/>")), TEXT, "nothing fit to name");
    // Hostile: deep, and many.
    let deep = format!("{}<t:FieldURI FieldURI=\"item:Deep\"/>{}", "<t:Path>".repeat(5_000), "</t:Path>".repeat(5_000));
    assert!(said(&deep).starts_with(TEXT));
    let many = "<t:FieldURI FieldURI=\"bad name\"/>".repeat(20_000);
    assert_eq!(said(&many), TEXT);
}
