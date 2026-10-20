//! Port of micromatch/braces `lib/stringify.js`.
//!
//! Reconstructs the raw pattern text from an AST (or subtree). Note the
//! reference's inner walker always re-defaults `parent` to `{}`, so
//! `invalidBlock` is constantly false — only `invalidNode` (node.invalid &&
//! escapeInvalid on open/close leaves) matters.

use crate::ast::{Node, NodeId, NodeType};

#[inline]
fn is_open_or_close(n: &Node) -> bool {
    matches!(n.ty, NodeType::Open | NodeType::Close) || n.open || n.close
}

fn walk(nodes: &[Node], id: NodeId, escape_invalid: bool, out: &mut Vec<u16>) {
    let n = &nodes[id];
    if n.has_value() {
        let invalid_node = n.invalid == Some(true) && escape_invalid;
        if invalid_node && is_open_or_close(n) {
            out.push(b'\\' as u16);
        }
        out.extend_from_slice(n.value.as_ref().unwrap());
        return;
    }
    for i in 0..n.children.len() {
        let child = n.children[i];
        walk(nodes, child, escape_invalid, out);
    }
}

/// Stringify a whole AST (the `braces.stringify` path).
pub fn stringify_ast(nodes: &[Node], root: NodeId, escape_invalid: bool) -> Vec<u16> {
    let mut out = Vec::new();
    walk(nodes, root, escape_invalid, &mut out);
    out
}

/// Stringify a subtree (used by the parser's comma surgery and expand's
/// invalid/dollar fallback). Port of the same inner walker.
pub fn stringify_subtree(nodes: &[Node], id: NodeId, escape_invalid: bool) -> Vec<u16> {
    let mut out = Vec::new();
    walk(nodes, id, escape_invalid, &mut out);
    out
}
