//! Faithful port of micromatch/braces `lib/compile.js`.
//! (The reference's stray `console.log('node.isClose', ...)` at
//! lib/compile.js:17 is a side effect, deliberately not replicated —
//! FINDINGS.md §2.17.)

use crate::ast::{Ast, NodeId, NodeType};
use crate::error::BracesError;
use crate::fill::{fill, FillOptions, FillOut};
use crate::options::Options;
use crate::utils::{is_invalid_brace, reduce};

fn walk(ast: &mut Ast, node: NodeId, parent: Option<NodeId>, opts: &Options, out: &mut Vec<u16>) -> Result<(), BracesError> {
    let invalid_block = match parent {
        Some(p) => is_invalid_brace(ast, p),
        None => false,
    };
    let n_invalid = ast.node(node).invalid;
    let escape = opts.escape_invalid;
    let invalid_node = n_invalid == Some(true) && escape;
    let invalid = invalid_block || invalid_node;
    let prefix: &[u16] = if escape { &[b'\\' as u16] } else { &[] };

    let ty = ast.node(node).ty;
    let is_open = ast.node(node).is_open;
    let is_close = ast.node(node).is_close;
    let value = ast.node(node).value.clone();

    if is_open {
        out.extend_from_slice(prefix);
        out.extend_from_slice(value.as_deref().unwrap_or(&[]));
        return Ok(());
    }
    if is_close {
        out.extend_from_slice(prefix);
        out.extend_from_slice(value.as_deref().unwrap_or(&[]));
        return Ok(());
    }
    if ty == NodeType::Open {
        if invalid {
            out.extend_from_slice(prefix);
            out.extend_from_slice(value.as_deref().unwrap_or(&[]));
        } else {
            out.push(b'(' as u16);
        }
        return Ok(());
    }
    if ty == NodeType::Close {
        if invalid {
            out.extend_from_slice(prefix);
            out.extend_from_slice(value.as_deref().unwrap_or(&[]));
        } else {
            out.push(b')' as u16);
        }
        return Ok(());
    }
    if ty == NodeType::Comma {
        let prev_ty = ast
            .node(node)
            .prev
            .map(|p| ast.node(p).ty);
        if prev_ty == Some(NodeType::Comma) {
            return Ok(());
        }
        if invalid {
            out.extend_from_slice(value.as_deref().unwrap_or(&[]));
        } else {
            out.push(b'|' as u16);
        }
        return Ok(());
    }
    if let Some(v) = &value {
        if !v.is_empty() {
            out.extend_from_slice(v);
            return Ok(());
        }
    }
    let ranges = ast.node(node).ranges;
    let has_nodes = ast.node(node).has_nodes();
    if has_nodes && ranges > 0 {
        let args = reduce(ast, node);
        let fopts = FillOptions::from_compile_options(opts);
        let filled = fill(
            args.first().map(|v| v.as_slice()).unwrap_or(&[]),
            args.get(1).map(|v| v.as_slice()),
            args.get(2).map(|v| v.as_slice()),
            &fopts,
        )?;
        if let FillOut::Regex(range) = filled {
            if !range.is_empty() {
                if args.len() > 1 && range.len() > 1 {
                    out.push(b'(' as u16);
                    out.extend_from_slice(&range);
                    out.push(b')' as u16);
                } else {
                    out.extend_from_slice(&range);
                }
                return Ok(());
            }
        }
    }
    if has_nodes {
        let children = ast.node(node).children.clone();
        for child in children {
            walk(ast, child, Some(node), opts, out)?;
        }
    }
    Ok(())
}

/// Compile an AST into a regex-source string.
pub fn compile(ast: &mut Ast, opts: &Options) -> Result<Vec<u16>, BracesError> {
    let root = ast.root;
    let mut out = Vec::with_capacity(ast.input.len() * 2);
    walk(ast, root, None, opts, &mut out)?;
    Ok(out)
}
