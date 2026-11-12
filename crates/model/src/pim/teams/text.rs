// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A Teams message's text as Quatrelle keeps and draws it: blocks (paragraphs, quotes, code, list
//! items) of runs of text with their styles, links, mentions and pictures. `mime` makes it from
//! Teams' HTML and writes it back for sending.

use serde::{Deserialize, Serialize};

/// A message's text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatText {
    pub blocks: Vec<Block>,
}

impl ChatText {
    /// Its words, each block on a line of its own.
    pub fn plain(&self) -> String {
        let mut out = String::new();
        for (i, block) in self.blocks.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            for run in &block.runs {
                out.push_str(&run.text);
            }
        }
        out
    }
}

/// A paragraph, a quote, a block of code or a list item.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub kind: BlockKind,
    pub runs: Vec<Run>,
}

/// What a block is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "block", rename_all = "camelCase")]
pub enum BlockKind {
    #[default]
    Paragraph,
    Quote,
    Code,
    /// A list item, numbered or not, `depth` lists deep (1 for a list's own items).
    Item {
        ordered: bool,
        depth: u8,
    },
}

/// A piece of text with one style. Written without the fields left as they are by default (a
/// message of many short runs is kept in a fraction of the room), and read back with them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Run {
    pub text: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub underline: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub strike: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub code: bool,
    /// Where it links to (`http` or `https`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// The mention it is ([`super::Mention::id`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mention: Option<u32>,
    /// The id of this message's hosted image it shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hosted: Option<String>,
    /// The address of an image from elsewhere it shows, fetched only when the user asks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outside: Option<String>,
    /// The id of the attachment whose place it marks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<String>,
}
