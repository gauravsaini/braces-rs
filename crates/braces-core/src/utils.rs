//! Port of micromatch/braces `lib/utils.js`.
//!
//! Preserves the reference's exact quirks, including the operator-precedence
//! quirk in `encloseBrace`/`isInvalidBrace`: `commas >> 0 + ranges >> 0`
//! parses as `(commas >> (0 + ranges)) >> 0`, so the emptiness test is
//! `commas >> ranges == 0` (32-bit shift, count masked to 5 bits).

use crate::ast::{Ast, NodeId, NodeType};
use crate::jsnum;
use crate::options::RangeLimit;

/// `(commas >> (0 + ranges)) >> 0 === 0`, JS shift semantics.
#[inline]
pub fn zeroish(commas: u32, ranges: u32) -> bool {
    ((commas as i32) >> ((ranges as u32) & 31)) == 0
}

/// Port of `encloseBrace`: marks the brace invalid and reports whether its
/// contents must be re-enclosed in literal braces on expansion.
pub fn enclose_brace(ast: &mut Ast, id: NodeId) -> bool {
    if ast.node(id).ty != NodeType::Brace {
        return false;
    }
    let (commas, ranges) = {
        let n = ast.node(id);
        (n.commas, n.ranges)
    };
    if zeroish(commas, ranges) {
        ast.node_mut(id).invalid = Some(true);
        return true;
    }
    false
}

/// Port of `isInvalidBrace`. Mutates: sets `invalid` when the
/// commas/ranges test or the open/close test fails.
pub fn is_invalid_brace(ast: &mut Ast, id: NodeId) -> bool {
    if ast.node(id).ty != NodeType::Brace {
        return false;
    }
    let n = ast.node(id);
    if n.invalid == Some(true) || n.dollar {
        return true;
    }
    let (commas, ranges, open, close) = (n.commas, n.ranges, n.open, n.close);
    if zeroish(commas, ranges) {
        ast.node_mut(id).invalid = Some(true);
        return true;
    }
    if !open || !close {
        ast.node_mut(id).invalid = Some(true);
        return true;
    }
    false
}

/// Port of `reduce`: collects text-node values; range nodes are flipped to
/// text (the mutation the reference performs).
pub fn reduce(ast: &mut Ast, id: NodeId) -> Vec<Vec<u16>> {
    let children = ast.node(id).children.clone();
    let mut acc = Vec::with_capacity(children.len());
    for child in children {
        let ty = ast.node(child).ty;
        if ty == NodeType::Text {
            acc.push(ast.node(child).value.clone().unwrap_or_default());
        } else if ty == NodeType::Range {
            ast.node_mut(child).ty = NodeType::Text;
        }
    }
    acc
}

/// Port of `exceedsLimit(min, max, step = 1, limit)` as called by expand:
/// `exceedsLimit(...args, options.step, rangeLimit)`. Positional quirk
/// preserved: with an explicit pattern step, `options.step` lands in the
/// `limit` slot and `rangeLimit` is ignored; the `step = 1` default applies
/// only when the argument is `undefined`.
pub fn exceeds_limit(args: &[Vec<u16>], opt_step: Option<f64>, range_limit: RangeLimit) -> bool {
    // limit === false disables (only reachable when args.len() == 2).
    let limit_js: Option<f64> = if args.len() >= 3 {
        opt_step
    } else {
        match range_limit {
            RangeLimit::Disabled => return false,
            RangeLimit::Num(n) => Some(n),
        }
    };
    let step: f64 = if args.len() >= 3 {
        jsnum::to_number(&args[2])
    } else {
        opt_step.unwrap_or(1.0)
    };
    let min = args.first().map(|v| v.as_slice()).unwrap_or(&[]);
    let max = args.get(1).map(|v| v.as_slice()).unwrap_or(&[]);
    if !jsnum::is_integer_str(min) || !jsnum::is_integer_str(max) {
        return false;
    }
    match limit_js {
        None => false, // `>= undefined` is always false
        Some(limit) => (jsnum::to_number(max) - jsnum::to_number(min)) / step >= limit,
    }
}

/// Deep flatten of queue values (port of `utils.flatten`).
pub fn flatten<T: Clone>(items: Vec<QueueVal<T>>) -> Vec<T> {
    let mut out = Vec::new();
    fn rec<T: Clone>(v: &QueueVal<T>, out: &mut Vec<T>) {
        match v {
            QueueVal::One(x) => out.push(x.clone()),
            QueueVal::Many(xs) => {
                for x in xs {
                    rec(x, out);
                }
            }
        }
    }
    for v in &items {
        rec(v, &mut out);
    }
    out
}

/// A queue slot: a single string or a nested array of slots.
#[derive(Clone, Debug)]
pub enum QueueVal<T> {
    One(T),
    Many(Vec<QueueVal<T>>),
}
