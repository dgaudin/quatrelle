//! HTML laid out as blocks: paragraphs, tables and their columns, colours, background pictures.

use super::*;

pub(super) struct Ctx {
    /// The message's `<style>` blocks, given to each piece of text (for colours and hidden parts).
    pub(super) css: String,
    pub(super) cells: usize,
    /// The text colour the boxes around inherit to the text inside (a cell's `color`).
    pub(super) color: Option<String>,
}

/// `f` with the text colour `n` sets, if it sets one, as CSS inherits it.
pub(super) fn colored<R>(ctx: &mut Ctx, n: &Node, f: impl FnOnce(&mut Ctx) -> R) -> R {
    // Only what a colour is written with: it goes back into markup.
    let own = n
        .style_as_written("color")
        .or_else(|| (n.tag == "font").then(|| n.attr("color").map(str::to_string)).flatten())
        .filter(|c| c.len() < 40 && c.chars().all(|ch| ch.is_ascii_alphanumeric() || "#(),. %".contains(ch)));
    match own {
        Some(c) => {
            let saved = ctx.color.replace(c);
            let r = f(ctx);
            ctx.color = saved;
            r
        }
        None => f(ctx),
    }
}

pub(super) fn blocks(kids: &[Kid], ctx: &mut Ctx, depth: usize) -> Vec<Block> {
    // Runs of boxes side by side among other content, as boxes of columns (only when there are
    // others: a parent of nothing but such boxes lays them out itself).
    let mixed = kids.iter().any(|k| matches!(k, Kid::El(n) if !n.beside() && !n.hidden()) || matches!(k, Kid::Text(t) if !t.trim().is_empty()));
    if mixed && let Some(grouped) = group_beside(kids) {
        return blocks_of(&grouped, ctx, depth);
    }
    blocks_of(kids, ctx, depth)
}

/// Whether any table puts content side by side, or a picture behind it, or boxes over it: a
/// layout worth keeping (otherwise the text flows).
pub(super) fn has_columns(b: &[Block]) -> bool {
    b.iter().any(|b| match b {
        Block::Text(_) => false,
        Block::Table(t) => {
            t.background_image.is_some()
                || t.rows.iter().any(|r| r.iter().filter(|c| !c.blocks.is_empty()).count() > 1)
                || t.rows.iter().flatten().any(|c| c.background_image.is_some() || !c.overlays.is_empty() || has_columns(&c.blocks))
        }
    })
}

/// The pictures behind boxes in a layout (to be shown, or held back, like the message's others).
pub fn background_pictures(b: &[Block]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |src: &Option<String>, out: &mut Vec<String>| {
        if let Some(s) = src
            && !out.contains(s)
            && out.len() < 200
        {
            out.push(s.clone());
        }
    };
    pub(super) fn walk(b: &[Block], out: &mut Vec<String>, add: &mut dyn FnMut(&Option<String>, &mut Vec<String>), depth: usize) {
        if depth > MAX_DEPTH * 2 {
            return;
        }
        for x in b {
            if let Block::Table(t) = x {
                add(&t.background_image, out);
                for c in t.rows.iter().flatten() {
                    add(&c.background_image, out);
                    walk(&c.blocks, out, add, depth + 1);
                    for o in &c.overlays {
                        walk(&o.blocks, out, add, depth + 1);
                    }
                }
            }
        }
    }
    walk(b, &mut out, &mut add, 0);
    out
}
