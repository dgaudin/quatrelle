// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Items as the writer takes them: their properties and files, written a batch at a time.

use outlook_pst::UnicodePstName;
use outlook_pst::messaging::named_prop::PS_PUBLIC_STRINGS;

use super::*;

/// The multi-valued properties' lists of an item, in the order of its properties.
pub(in super::super) struct Lists<'a> {
    texts: Vec<Vec<&'a str>>,
    bins: Vec<Vec<&'a [u8]>>,
}

/// The lists of `props`' multi-valued properties.
pub(in super::super) fn lists(props: &[(Key, Value)]) -> Lists<'_> {
    let mut l = Lists { texts: Vec::new(), bins: Vec::new() };
    for (_, v) in props {
        match v {
            Value::Texts(t) => l.texts.push(t.iter().map(String::as_str).collect()),
            Value::Bins(b) => l.bins.push(b.iter().map(Vec::as_slice).collect()),
            _ => {}
        }
    }
    l
}

/// Properties (`props`, whose lists are `lists`) as the writer takes them.
pub(in super::super) fn properties<'a>(props: &'a [(Key, Value)], lists: &'a Lists<'a>) -> Vec<UnicodePstProperty<'a>> {
    let (mut texts, mut bins) = (lists.texts.iter(), lists.bins.iter());
    let mut out = Vec::new();
    for (key, v) in props {
        let value = match (v, single(v)) {
            (_, Some(one)) => one,
            (Value::Texts(_), None) => match texts.next() {
                Some(t) => UnicodePstValue::MultipleUnicode(t),
                None => continue,
            },
            (_, None) => match bins.next() {
                Some(b) => UnicodePstValue::MultipleBinary(b),
                None => continue,
            },
        };
        out.push(match key {
            Key::Tag(id) => UnicodePstProperty::tagged(*id, value),
            Key::Lid(set, id) => UnicodePstProperty::named(*set, UnicodePstName::Id(*id), value),
            Key::Keywords => UnicodePstProperty::named(PS_PUBLIC_STRINGS, UnicodePstName::String("Keywords"), value),
        });
    }
    out
}

/// An item's files as attachments; `picture`: the properties of a contact's picture.
fn files<'a>(item: &'a Item, picture: &'a [UnicodePstProperty<'a>]) -> Vec<UnicodePstAttachment<'a>> {
    item.files
        .iter()
        .map(|f| UnicodePstAttachment {
            filename: &f.name,
            mime_type: &f.mime,
            data: &f.data,
            properties: if f.picture { picture } else { &[] },
            ..Default::default()
        })
        .collect()
}

/// The message the writer writes for `item`, of its properties `p`.
fn message<'a>(item: &'a Item, p: &'a [UnicodePstProperty<'a>]) -> UnicodePstMessage<'a> {
    UnicodePstMessage {
        subject: &item.subject,
        sender_name: "",
        sender_email: "",
        recipients: &[],
        body: &item.body,
        html_body: None,
        message_id: "",
        delivery_time: item.modified,
        message_class: item.class,
        properties: p,
    }
}

/// Write `batch` into `folder` of the file at `path` (in `format`).
pub(super) fn write_batch(path: &Path, format: Format, folder: &[&str], batch: &[Item]) -> Result<(), String> {
    // PidTagAttachmentContactPhoto, and hidden, as Outlook marks a contact's picture.
    let picture =
        [UnicodePstProperty::tagged(0x7FFF, UnicodePstValue::Boolean(true)), UnicodePstProperty::tagged(0x7FFE, UnicodePstValue::Boolean(true))];
    let lists: Vec<Lists<'_>> = batch.iter().map(|item| lists(&item.props)).collect();
    let own: Vec<Vec<UnicodePstProperty<'_>>> = batch.iter().zip(&lists).map(|(item, l)| properties(&item.props, l)).collect();
    let attachments: Vec<Vec<UnicodePstAttachment<'_>>> = batch.iter().map(|item| files(item, &picture)).collect();
    let messages: Vec<UnicodePstBatchMessage<'_>> =
        batch.iter().zip(&own).zip(&attachments).map(|((item, p), f)| UnicodePstBatchMessage { message: message(item, p), attachments: f }).collect();
    writing::append(path, format, folder, &messages)
}
