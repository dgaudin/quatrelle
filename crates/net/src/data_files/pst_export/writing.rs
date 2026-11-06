// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Handing a batch of messages to the writer, each with its recipients, properties and files.

use super::*;

/// Write `batch` into `folder` of the file at `path`, made with the first message unless it's
/// `created`.
pub(super) fn write(path: &Path, folder: &[&str], batch: &[Owned], created: &mut bool) -> Result<(), String> {
    let recipients: Vec<Vec<UnicodePstRecipient<'_>>> = batch
        .iter()
        .map(|m| {
            m.recipients.iter().map(|(name, email, kind)| UnicodePstRecipient { name, email, recipient_type: *kind, ..Default::default() }).collect()
        })
        .collect();
    let files: Vec<Vec<UnicodePstAttachment<'_>>> = batch
        .iter()
        .map(|m| {
            m.files
                .iter()
                .map(|(filename, mime_type, cid, data)| UnicodePstAttachment {
                    filename,
                    mime_type,
                    content_id: cid.as_deref(),
                    data,
                    ..Default::default()
                })
                .collect()
        })
        .collect();
    let categories: Vec<Vec<&str>> = batch.iter().map(|m| m.state.categories.iter().map(String::as_str).collect()).collect();
    let properties: Vec<Vec<UnicodePstProperty<'_>>> = batch.iter().zip(&categories).map(|(m, c)| item::properties(m, c)).collect();
    let messages: Vec<UnicodePstBatchMessage<'_>> = batch
        .iter()
        .zip(&recipients)
        .zip(&files)
        .zip(&properties)
        .map(|(((m, r), f), p)| UnicodePstBatchMessage {
            message: UnicodePstMessage {
                subject: &m.subject,
                sender_name: &m.from.0,
                sender_email: &m.from.1,
                recipients: r,
                body: &m.body,
                html_body: m.html.as_deref(),
                message_id: &m.message_id,
                delivery_time: m.received,
                message_class: "IPM.Note",
                properties: p,
            },
            attachments: f,
        })
        .collect();
    let (first, rest) = match (*created, messages.split_first()) {
        (_, None) => return Ok(()),
        (true, _) => (None, messages.as_slice()),
        (false, Some((first, rest))) => (Some(first), rest),
    };
    if let Some(first) = first {
        UnicodePstFile::create_in_folder_with_attachments(path, folder, &first.message, first.attachments).map_err(|e| e.to_string())?;
        *created = true;
    }
    if !rest.is_empty() {
        append(path, folder, rest)?;
    }
    Ok(())
}

/// Add `batch` to the file at `path`. A file whose name-to-ID map can't take names (damaged, or
/// made by a program that keeps it some other way) gets the batch without its named properties:
/// the mail goes in, without its follow-up text, due days and categories, and the log says why.
pub(super) fn append(path: &Path, folder: &[&str], batch: &[UnicodePstBatchMessage<'_>]) -> Result<(), String> {
    let Err(e) = UnicodePstFile::append_many_in_folder_with_attachments(path, folder, batch) else { return Ok(()) };
    let named = |p: &UnicodePstProperty<'_>| matches!(p.key, UnicodePstPropertyKey::Named(..));
    // The writer says so before it writes any of the batch's messages (their folder may be made
    // by then, and stays).
    if !e.to_string().starts_with("name-to-ID map") || !batch.iter().any(|m| m.message.properties.iter().any(named)) {
        return Err(e.to_string());
    }
    log::info!("the data file's name-to-ID map can't be added to ({e}): mail written without its flags' text, due days and categories");
    let tagged: Vec<Vec<UnicodePstProperty<'_>>> =
        batch.iter().map(|m| m.message.properties.iter().filter(|p| !named(p)).copied().collect()).collect();
    let batch: Vec<UnicodePstBatchMessage<'_>> = batch
        .iter()
        .zip(&tagged)
        .map(|(m, p)| UnicodePstBatchMessage { message: UnicodePstMessage { properties: p, ..m.message }, attachments: m.attachments })
        .collect();
    UnicodePstFile::append_many_in_folder_with_attachments(path, folder, &batch).map(drop).map_err(|e| e.to_string())
}
