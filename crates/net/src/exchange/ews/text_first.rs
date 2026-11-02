//! A message's text first: its body, headers and the list of its files, without their bytes, from
//! one `GetItem`, so it can be read while its whole source comes.

use quatrelle_model::EmailAddress;

use super::*;

/// What's asked beyond the list's headers, every name from Microsoft's list: Exchange checks them
/// all against its schema, and one it doesn't know refuses the whole request. No `BodyType`:
/// grommunio sends the whole source with one.
const TEXT_FIELDS: &str = "<t:FieldURI FieldURI=\"item:Body\"/><t:FieldURI FieldURI=\"item:Attachments\"/>\
<t:FieldURI FieldURI=\"item:ItemClass\"/><t:FieldURI FieldURI=\"item:Importance\"/>";

/// Item classes whose real content isn't in their body: signed or encrypted mail, invitations.
const LEFT_WHOLE: [&str; 2] = ["ipm.note.smime", "ipm.schedule.meeting"];

const ANY_FILE: &str = "application/octet-stream";

/// What [`address`] wrote, taken apart: `"Name" <a@b>`, `"Name" <>` for a name alone, or `a@b`.
fn split(a: &str) -> EmailAddress {
    match a.strip_suffix('>').and_then(|a| a.rsplit_once(" <")) {
        Some((name, email)) => EmailAddress::new(name.trim().trim_matches('"'), email),
        None => EmailAddress::new("", a),
    }
}

fn recipients(it: &El, name: &str) -> Vec<EmailAddress> {
    it.child(name).map(|l| l.children("Mailbox").filter_map(address).map(|a| split(&a)).collect()).unwrap_or_default()
}

/// A file in an item's list. An attached message has no `ContentType` to go by, and a link to a
/// file (`ReferenceAttachment`) is no file of that type here.
fn pending_file(a: &El) -> mime::PendingFile {
    let kind = match a.name.as_str() {
        "FileAttachment" => a.text_of("ContentType").filter(|t| !t.trim().is_empty()).unwrap_or(ANY_FILE),
        "ItemAttachment" => "message/rfc822",
        _ => ANY_FILE,
    };
    let content_id = a.text_of("ContentId").map(str::trim).filter(|c| !c.is_empty()).map(str::to_string);
    mime::PendingFile {
        name: a.text_of("Name").unwrap_or_default().to_string(),
        mime: kind.to_string(),
        size: a.text_of("Size").and_then(|s| s.trim().parse().ok()).unwrap_or(0),
        inline: content_id.is_some() && a.text_of("IsInline").is_some_and(|v| v.trim().eq_ignore_ascii_case("true")),
        content_id,
    }
}

/// The message in a `GetItem` answer (`m`, its response message) as MIME, its files listed empty.
/// `None` when it's left to come whole: no text or no file to show, or its real content isn't in
/// its text.
pub fn text_first(m: &El) -> Option<Vec<u8>> {
    let it = m.find("Items")?.kids.first()?;
    let class = it.text_of("ItemClass").unwrap_or_default().trim().to_ascii_lowercase();
    if LEFT_WHOLE.iter().any(|c| class.starts_with(c)) {
        return None;
    }
    let body = it.child("Body").filter(|b| !b.text.trim().is_empty())?;
    let list = it.child("Attachments").filter(|l| !l.kids.is_empty())?;
    if list.kids.iter().any(|a| mime::is_protected_file(a.text_of("ContentType").unwrap_or_default(), a.text_of("Name").unwrap_or_default())) {
        return None;
    }
    let files: Vec<mime::PendingFile> = list.kids.iter().map(pending_file).collect();
    let html = !body.attr("BodyType").is_some_and(|t| t.trim().eq_ignore_ascii_case("Text"));
    let draft = mime::Draft {
        to: recipients(it, "ToRecipients"),
        cc: recipients(it, "CcRecipients"),
        reply_to: recipients(it, "ReplyTo"),
        subject: it.text_of("Subject").unwrap_or_default().to_string(),
        body: if html { String::new() } else { body.text.clone() },
        html: html.then(|| body.text.clone()),
        importance: match it.text_of("Importance").map(str::trim) {
            Some("High") => 1,
            Some("Low") => -1,
            _ => 0,
        },
        ..Default::default()
    };
    let from = sender(it).map(|f| split(&f)).unwrap_or_default();
    let date = ["DateTimeSent", "DateTimeReceived"].iter().find_map(|f| it.text_of(f).and_then(jmap::utc_date)).unwrap_or(0);
    mime::text_first(&draft, &from, date, it.text_of("InternetMessageId").unwrap_or_default(), &files).ok()
}

impl Client {
    /// Message `id` from the fields above, without its source: `None` when [`text_first()`] leaves
    /// it whole.
    pub fn download_text(&self, id: &str) -> Result<Option<Vec<u8>>> {
        let m = self.call_one(&format!(
            "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{HEADER_FIELDS}{TEXT_FIELDS}</t:AdditionalProperties></m:ItemShape>\
             <m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:GetItem>",
            esc(id)
        ))?;
        Ok(text_first(&m))
    }
}

/// A cached message's text and its item id, for a message without its source here and with an id
/// on this server (found as [`fetch_body`] finds it): `None` otherwise, or when it's left whole.
/// Nothing is kept: the text isn't the message.
pub fn fetch_text(c: &Client, st: &mut State, store: &Store, key: &MessageKey) -> Result<Option<(Vec<u8>, String)>> {
    if key.validity != VALIDITY || store.has_body(key) {
        return Ok(None);
    }
    let Some(id) = sync::server_id(c, st, store, key)? else { return Ok(None) };
    Ok(c.download_text(&id)?.map(|raw| (raw, id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `fields` as the item of a `GetItem` answer, written the way Exchange writes one.
    fn answer(fields: &str) -> El {
        parse(&format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\">\
             <s:Header><h:ServerVersionInfo MajorVersion=\"15\" MinorVersion=\"1\" MajorBuildNumber=\"2507\" MinorBuildNumber=\"6\" Version=\"V2017_07_11\" \
             xmlns:h=\"http://schemas.microsoft.com/exchange/services/2006/types\"/></s:Header><s:Body>\
             <m:GetItemResponse xmlns:m=\"http://schemas.microsoft.com/exchange/services/2006/messages\" xmlns:t=\"http://schemas.microsoft.com/exchange/services/2006/types\">\
             <m:ResponseMessages><m:GetItemResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode><m:Items>{fields}</m:Items>\
             </m:GetItemResponseMessage></m:ResponseMessages></m:GetItemResponse></s:Body></s:Envelope>"
        ))
    }

    fn read(item: &str) -> Option<Vec<u8>> {
        let doc = answer(item);
        text_first(messages(&doc).first()?)
    }

    fn message(fields: &str) -> Option<Vec<u8>> {
        read(&format!("<t:Message>{fields}</t:Message>"))
    }

    const BODY: &str = "<t:Body BodyType=\"HTML\" IsTruncated=\"false\">&lt;p&gt;Here's reel 2.&lt;/p&gt;</t:Body>";

    /// A file, as Exchange lists one.
    fn file(name: &str, kind: &str, size: &str) -> String {
        format!(
            "<t:FileAttachment><t:AttachmentId Id=\"AAMk=\"/><t:Name>{name}</t:Name><t:ContentType>{kind}</t:ContentType><t:Size>{size}</t:Size>\
             <t:LastModifiedTime>2026-10-30T08:02:19</t:LastModifiedTime><t:IsInline>false</t:IsInline><t:IsContactPhoto>false</t:IsContactPhoto></t:FileAttachment>"
        )
    }

    fn with_files(class: &str, body: &str, files: &str) -> Option<Vec<u8>> {
        message(&format!("<t:ItemClass>{class}</t:ItemClass>{body}<t:Attachments>{files}</t:Attachments>"))
    }

    /// A colleague's mail: From and a recipient under their Exchange address, the sender's SMTP
    /// address beside it; a file, an embedded picture, an attached message, a link to a file and a
    /// file without a name.
    #[test]
    fn an_item_read_as_exchange_gives_it() {
        let raw = message(
            "<t:ItemId Id=\"AAMk1\" ChangeKey=\"CQAA\"/><t:ItemClass>IPM.Note</t:ItemClass><t:Subject>Rushes reel 2</t:Subject>\
             <t:Importance>High</t:Importance><t:DateTimeSent>2026-10-30T08:02:19Z</t:DateTimeSent><t:DateTimeReceived>2026-10-30T08:03:00Z</t:DateTimeReceived>\
             <t:Size>21002000</t:Size><t:HasAttachments>true</t:HasAttachments>\
             <t:Body BodyType=\"HTML\" IsTruncated=\"false\">&lt;html&gt;&lt;body&gt;&lt;p&gt;Here's reel 2.&lt;/p&gt;&lt;img src=\"cid:still@x\"&gt;&lt;/body&gt;&lt;/html&gt;</t:Body>\
             <t:Attachments>\
             <t:FileAttachment><t:AttachmentId Id=\"A1\"/><t:Name>reel2.mov</t:Name><t:ContentType>video/quicktime</t:ContentType><t:Size>20971586</t:Size><t:IsInline>false</t:IsInline></t:FileAttachment>\
             <t:FileAttachment><t:AttachmentId Id=\"A2\"/><t:Name>still.png</t:Name><t:ContentType>image/png</t:ContentType><t:ContentId>still@x</t:ContentId><t:Size>2048</t:Size><t:IsInline>true</t:IsInline></t:FileAttachment>\
             <t:ItemAttachment><t:AttachmentId Id=\"A3\"/><t:Name>Earlier note</t:Name><t:Size>4096</t:Size><t:IsInline>false</t:IsInline></t:ItemAttachment>\
             <t:ReferenceAttachment><t:AttachmentId Id=\"A4\"/><t:Name>Plan on the share</t:Name></t:ReferenceAttachment>\
             <t:FileAttachment><t:AttachmentId Id=\"A5\"/><t:ContentType>application/pdf</t:ContentType><t:Size>10</t:Size></t:FileAttachment>\
             </t:Attachments>\
             <t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x5d01\" PropertyType=\"String\"/><t:Value>sam@contoso.example</t:Value></t:ExtendedProperty>\
             <t:ToRecipients>\
             <t:Mailbox><t:Name>Riley Chen</t:Name><t:EmailAddress>riley@contoso.example</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType><t:MailboxType>Mailbox</t:MailboxType></t:Mailbox>\
             <t:Mailbox><t:Name>Alex Martin</t:Name><t:EmailAddress>/O=CONTOSO/OU=EXCHANGE/CN=RECIPIENTS/CN=ALEX</t:EmailAddress><t:RoutingType>EX</t:RoutingType><t:MailboxType>Mailbox</t:MailboxType></t:Mailbox>\
             </t:ToRecipients>\
             <t:CcRecipients><t:Mailbox><t:Name>Zoe Park</t:Name><t:EmailAddress>/O=CONTOSO/OU=EXCHANGE/CN=RECIPIENTS/CN=ZOE</t:EmailAddress><t:RoutingType>EX</t:RoutingType></t:Mailbox></t:CcRecipients>\
             <t:ReplyTo><t:Mailbox><t:Name>Desk</t:Name><t:EmailAddress>desk@contoso.example</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType></t:Mailbox></t:ReplyTo>\
             <t:From><t:Mailbox><t:Name>Sam Lee</t:Name><t:EmailAddress>/O=CONTOSO/OU=EXCHANGE/CN=RECIPIENTS/CN=SAM</t:EmailAddress><t:RoutingType>EX</t:RoutingType></t:Mailbox></t:From>\
             <t:InternetMessageId>&lt;r2@contoso.example&gt;</t:InternetMessageId>",
        )
        .unwrap();
        let p = mime::parse_text_first(&raw).unwrap();
        assert_eq!((p.from.name.as_str(), p.from.email.as_str()), ("Sam Lee", "sam@contoso.example"));
        assert_eq!(p.to, [EmailAddress::new("Riley Chen", "riley@contoso.example"), EmailAddress::new("Alex Martin", "")]);
        assert_eq!(p.cc, [EmailAddress::new("Zoe Park", "")]);
        assert_eq!(p.reply_to, [EmailAddress::new("Desk", "desk@contoso.example")]);
        assert_eq!((p.subject.as_str(), p.message_id.as_str()), ("Rushes reel 2", "r2@contoso.example"));
        assert_eq!(p.date, jmap::utc_date("2026-10-30T08:02:19Z").unwrap());
        assert!(p.had_html && p.text.contains("Here's reel 2."), "{}", p.text);
        let files: Vec<(&str, usize, bool)> = p.attachments.iter().map(|a| (a.mime.as_str(), a.size, a.inline)).collect();
        assert_eq!(
            files,
            [
                ("video/quicktime", 20_971_586, false),
                ("image/png", 2048, true),
                ("message/rfc822", 4096, false),
                ("application/octet-stream", 0, false),
                ("application/pdf", 10, false),
            ]
        );
        assert_eq!(p.attachments[1].content_id.as_deref(), Some("still@x"));
        assert!(String::from_utf8_lossy(&raw).contains("Importance: high"));
    }

    /// What the text alone wouldn't show: signed and encrypted mail and invitations, in any case;
    /// and a message with nothing to show first.
    #[test]
    fn what_is_left_whole() {
        let a_file = file("cut.mov", "video/quicktime", "2097152");
        for class in [
            "IPM.Note.SMIME.MultipartSigned",
            "IPM.Note.SMIME",
            "ipm.note.smime.multipartsigned",
            "IPM.Schedule.Meeting.Request",
            "IPM.Schedule.Meeting.Canceled",
        ] {
            assert!(with_files(class, BODY, &a_file).is_none(), "{class}");
        }
        for protected in [file("smime.p7m", "application/octet-stream", "9"), file("a.bin", "application/pkcs7-mime", "9")] {
            assert!(with_files("IPM.Note", BODY, &format!("{a_file}{protected}")).is_none(), "{protected}");
        }
        assert!(with_files("IPM.Note", "<t:Body BodyType=\"HTML\"/>", &a_file).is_none(), "an empty body");
        assert!(with_files("IPM.Note", "<t:Body BodyType=\"Text\"> \r\n </t:Body>", &a_file).is_none(), "a blank body");
        assert!(with_files("IPM.Note", "", &a_file).is_none(), "no body");
        assert!(with_files("IPM.Note", BODY, "").is_none(), "no file");
        assert!(message(&format!("<t:ItemClass>IPM.Note</t:ItemClass>{BODY}")).is_none(), "no list of files");
        assert!(with_files("IPM.Note", BODY, &a_file).is_some(), "and the same message with its text and a file does come");
        assert!(with_files("IPM.Note.Microsoft.Voicemail", BODY, &a_file).is_some(), "other classes are mail");
    }

    /// Whatever the item's element is: a report or a meeting's answer comes under another name.
    #[test]
    fn the_item_is_the_first_of_the_items_whatever_its_name() {
        let item = format!("<t:ItemClass>IPM.Note</t:ItemClass>{BODY}<t:Attachments>{}</t:Attachments>", file("a.pdf", "application/pdf", "9"));
        assert!(read(&format!("<t:PostItem>{item}</t:PostItem>")).is_some());
        assert!(read("").is_none(), "no item");
        assert!(text_first(&El::default()).is_none());
    }

    #[test]
    fn text_is_text_in_any_case_and_html_otherwise() {
        let f = file("a.pdf", "application/pdf", "9");
        let plain = with_files("IPM.Note", "<t:Body BodyType=\"text\">Line &lt;one&gt;</t:Body>", &f).unwrap();
        let p = mime::parse_text_first(&plain).unwrap();
        assert!(!p.had_html && p.text.contains("Line <one>"), "{}", p.text);
        let unsaid = with_files("IPM.Note", "<t:Body>&lt;p&gt;Hi&lt;/p&gt;</t:Body>", &f).unwrap();
        assert!(mime::parse_text_first(&unsaid).unwrap().had_html);
    }

    #[test]
    fn a_date_and_importance_read_as_the_server_says() {
        let f = format!("{BODY}<t:Attachments>{}</t:Attachments>", file("a.pdf", "application/pdf", "9"));
        let received = "<t:DateTimeReceived>2026-10-30T08:03:00Z</t:DateTimeReceived>";
        let raw = message(&format!("{f}{received}<t:Importance>Low</t:Importance>")).unwrap();
        assert_eq!(mime::parse_text_first(&raw).unwrap().date, jmap::utc_date("2026-10-30T08:03:00Z").unwrap(), "no date sent: received");
        assert!(String::from_utf8_lossy(&raw).contains("Importance: low"));
        let raw = message(&format!("{f}<t:DateTimeSent>not a date</t:DateTimeSent>{received}<t:Importance>Normal</t:Importance>")).unwrap();
        assert_eq!(mime::parse_text_first(&raw).unwrap().date, jmap::utc_date("2026-10-30T08:03:00Z").unwrap(), "an odd date sent: received");
        assert!(!String::from_utf8_lossy(&raw).contains("Importance:"));
    }

    /// A size that can't be read is unknown, and only a picture with an id can be shown in the text.
    #[test]
    fn a_file_says_what_it_can() {
        let junk = file("a.pdf", "application/pdf", "lots");
        let negative = file("b.pdf", "application/pdf", "-5");
        let no_id = "<t:FileAttachment><t:Name>c.png</t:Name><t:ContentType>image/png</t:ContentType><t:Size>7</t:Size><t:ContentId> </t:ContentId><t:IsInline>true</t:IsInline></t:FileAttachment>";
        let raw = with_files("IPM.Note", BODY, &format!("{junk}{negative}{no_id}")).unwrap();
        let p = mime::parse_text_first(&raw).unwrap();
        let sizes: Vec<usize> = p.attachments.iter().map(|a| a.size).collect();
        assert_eq!(sizes, [0, 0, 7]);
        assert!(!p.attachments[2].inline, "no id, nothing in the text shows it");
    }

    /// Exchange's addresses as [`address`] writes them, each read back; what's odd in them stays a
    /// name or an address, and adds no header or recipient.
    #[test]
    fn an_address_read_back() {
        assert_eq!(split("sam@x.example"), EmailAddress::new("", "sam@x.example"));
        assert_eq!(split("\"Sam Lee\" <sam@x.example>"), EmailAddress::new("Sam Lee", "sam@x.example"));
        assert_eq!(split("\"Sam Lee\" <>"), EmailAddress::new("Sam Lee", ""));
        assert_eq!(split("\"Lee, Sam <ops>\" <sam@x.example>"), EmailAddress::new("Lee, Sam <ops>", "sam@x.example"));
        assert_eq!(split("\"Lee, Sam\" <>"), EmailAddress::new("Lee, Sam", ""));

        let raw = message(&format!(
            "{BODY}<t:Attachments>{}</t:Attachments>\
             <t:ToRecipients><t:Mailbox><t:Name>Lee, Sam\r\nBcc: x@y.example</t:Name><t:EmailAddress>/O=X/CN=SAM</t:EmailAddress><t:RoutingType>EX</t:RoutingType></t:Mailbox></t:ToRecipients>\
             <t:From><t:Mailbox><t:Name>Sam</t:Name><t:EmailAddress>/O=X/CN=SAM</t:EmailAddress><t:RoutingType>EX</t:RoutingType></t:Mailbox></t:From>\
             <t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x5D01\" PropertyType=\"String\"/><t:Value>no address</t:Value></t:ExtendedProperty>",
            file("a.pdf", "application/pdf", "9")
        ))
        .unwrap();
        let p = mime::parse_text_first(&raw).unwrap();
        assert_eq!((p.from.name.as_str(), p.from.email.as_str()), ("Sam", ""), "an SMTP address that isn't one isn't used; the name stays");
        assert_eq!(p.to.len(), 1, "{:?}", p.to);
        assert!(!String::from_utf8_lossy(&raw).contains("\nBcc:"));
    }
}
