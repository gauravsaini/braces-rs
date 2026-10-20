//! Faithful port of micromatch/braces `lib/expand.js`.
//!
//! The reference's queue/`append` cartesian-product dance is reproduced
//! exactly (including `undefined`-pop defaulting to `''` and deep
//! flattening). Includes the no-brace fast path (LEARNINGS.md #4): a
//! brace-free AST stringifies to a single result without any queue
//! machinery.
//!
//! Segment-join walk (perf restructure): queue entries are `Rope`s —
//! ordered lists of shared segments (`Rc<[u16]>`) — instead of fully
//! materialised strings. `append` then only clones segment *lists*
//! (pointer-sized `Rc` clones) and each output string is joined exactly
//! once, at final materialisation. The old walk rebuilt every output
//! string at every segment append, which is quadratic on multi-group
//! patterns (e.g. `user-{200..300}/project-{a,b,c}-{1..10}`) where V8's
//! rope (cons-string) concatenation made the JS reference cheaper.

use crate::ast::{Ast, NodeId, NodeType};
use crate::error::BracesError;
use crate::fill::{fill, FillOptions, FillOut};
use crate::options::Options;
use crate::stringify::stringify_subtree;
use crate::utils::{enclose_brace, exceeds_limit, flatten, reduce, QueueVal};
use std::rc::Rc;

/// One immutable piece of an output string (text node value, fill item,
/// stringified subtree, or a literal brace for the enclose path).
type Seg = Rc<[u16]>;

fn seg_of(s: &[u16]) -> Seg {
    Rc::from(s)
}

/// An output string under construction: a list of shared segments.
/// Cloning a `Rope` copies the segment list (one `Rc` bump per segment),
/// never the segment bytes.
#[derive(Clone, Default, Debug)]
pub struct Rope {
    segs: Vec<Seg>,
}

impl Rope {
    fn empty() -> Self {
        Rope { segs: Vec::new() }
    }

    fn from_units(v: Vec<u16>) -> Self {
        if v.is_empty() {
            return Rope::empty();
        }
        Rope { segs: vec![Rc::from(v)] }
    }

    /// `self + other` as a new rope (segment lists concatenated).
    fn concat(&self, other: &Rope) -> Rope {
        let mut segs = Vec::with_capacity(self.segs.len() + other.segs.len());
        segs.extend_from_slice(&self.segs);
        segs.extend_from_slice(&other.segs);
        Rope { segs }
    }

    /// `self + "{" + inner + "}"` — the enclose path for invalid braces.
    fn concat_enclosed(&self, inner: &Rope) -> Rope {
        let mut segs = Vec::with_capacity(self.segs.len() + inner.segs.len() + 2);
        segs.extend_from_slice(&self.segs);
        segs.push(seg_of(&[b'{' as u16]));
        segs.extend_from_slice(&inner.segs);
        segs.push(seg_of(&[b'}' as u16]));
        Rope { segs }
    }

    /// `"{" + self + "}"` as a single rope (q-empty enclose branch).
    fn enclosed(&self) -> Rope {
        let mut segs = Vec::with_capacity(self.segs.len() + 2);
        segs.push(seg_of(&[b'{' as u16]));
        segs.extend_from_slice(&self.segs);
        segs.push(seg_of(&[b'}' as u16]));
        Rope { segs }
    }

    /// Join the segments into the final string — the one and only full
    /// copy of the output bytes.
    fn materialize(&self) -> Vec<u16> {
        let total: usize = self.segs.iter().map(|s| s.len()).sum();
        let mut out = Vec::with_capacity(total);
        for s in &self.segs {
            out.extend_from_slice(s);
        }
        out
    }
}

type Q = QueueVal<Rope>;

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Port of `append(queue = '', stash = '', enclose = false)`.
fn append(queue: Option<Q>, stash: Q, enclose: bool) -> Vec<Q> {
    // [].concat(queue): None (JS undefined) → ['']; One → [it]; Many → elements
    let q: Vec<Q> = match queue {
        None => vec![QueueVal::One(Rope::empty())],
        Some(QueueVal::One(s)) => vec![QueueVal::One(s)],
        Some(QueueVal::Many(v)) => v,
    };
    let st: Vec<Q> = match stash {
        QueueVal::One(s) => vec![QueueVal::One(s)],
        QueueVal::Many(v) => v,
    };
    if st.is_empty() {
        return q;
    }
    if q.is_empty() {
        if enclose {
            return flatten::<Rope>(st)
                .into_iter()
                .map(|e| QueueVal::One(e.enclosed()))
                .collect();
        }
        return st;
    }
    let mut result: Vec<Q> = Vec::new();
    for item in q {
        match item {
            QueueVal::Many(arr) => {
                for value in arr {
                    let r = append(Some(value), QueueVal::Many(st.clone()), enclose);
                    result.push(QueueVal::Many(r));
                }
            }
            QueueVal::One(s) => {
                for ele in &st {
                    match ele {
                        QueueVal::One(e) => {
                            let c = if enclose {
                                s.concat_enclosed(e)
                            } else {
                                s.concat(e)
                            };
                            result.push(QueueVal::One(c));
                        }
                        QueueVal::Many(a) => {
                            let r = append(
                                Some(QueueVal::One(s.clone())),
                                QueueVal::Many(a.clone()),
                                enclose,
                            );
                            result.push(QueueVal::Many(r));
                        }
                    }
                }
            }
        }
    }
    flatten::<Rope>(result)
        .into_iter()
        .map(QueueVal::One)
        .collect()
}

pub struct Expander<'a> {
    ast: &'a mut Ast,
    queues: Vec<Vec<Q>>,
    opts: &'a Options,
}

impl<'a> Expander<'a> {
    fn nearest_queue(&self, mut id: NodeId) -> NodeId {
        loop {
            let ty = self.ast.node(id).ty;
            if ty == NodeType::Brace || ty == NodeType::Root {
                return id;
            }
            id = self
                .ast
                .node(id)
                .parent
                .expect("walk only visits nodes with brace/root ancestors");
        }
    }

    fn walk(&mut self, node: NodeId, parent: Option<NodeId>) -> Result<(), BracesError> {
        self.queues[node] = Vec::new();
        let qid = match parent {
            Some(p) => self.nearest_queue(p),
            None => node,
        };

        let invalid = self.ast.node(node).invalid;
        let dollar = self.ast.node(node).dollar;
        let ty = self.ast.node(node).ty;
        let ranges = self.ast.node(node).ranges;
        let has_nodes = self.ast.node(node).has_nodes();
        let nchildren = self.ast.node(node).children.len();

        if invalid == Some(true) || dollar {
            let s = stringify_subtree(&self.ast.nodes, node, self.opts.escape_invalid);
            let popped = self.queues[qid].pop();
            let r = append(popped, QueueVal::One(Rope::from_units(s)), false);
            self.queues[qid].push(QueueVal::Many(r));
            return Ok(());
        }

        if ty == NodeType::Brace && invalid != Some(true) && nchildren == 2 {
            let popped = self.queues[qid].pop();
            let r = append(popped, QueueVal::One(Rope::from_units(units("{}"))), false);
            self.queues[qid].push(QueueVal::Many(r));
            return Ok(());
        }

        if has_nodes && ranges > 0 {
            let args = reduce(self.ast, node);
            if exceeds_limit(&args, self.opts.step_num(), self.opts.range_limit) {
                return Err(BracesError::RangeLimit);
            }
            let fopts = FillOptions::from_options(self.opts, false);
            let out = fill(
                args.first().map(|v| v.as_slice()).unwrap_or(&[]),
                args.get(1).map(|v| v.as_slice()),
                args.get(2).map(|v| v.as_slice()),
                &fopts,
            )?;
            let stash = match out {
                FillOut::List(items) if !items.is_empty() => QueueVal::Many(
                    items
                        .into_iter()
                        .map(|i| QueueVal::One(Rope::from_units(i)))
                        .collect(),
                ),
                _ => QueueVal::One(Rope::from_units(stringify_subtree(
                    &self.ast.nodes,
                    node,
                    self.opts.escape_invalid,
                ))),
            };
            let popped = self.queues[qid].pop();
            match (popped, stash) {
                (None, QueueVal::Many(v)) => {
                    // append([], stash) with enclose=false is the identity
                    // on stash's flattened contents.
                    self.queues[qid].push(QueueVal::Many(v));
                }
                (None, one) => {
                    self.queues[qid].push(QueueVal::Many(vec![one]));
                }
                (Some(p), stash) => {
                    let r = append(Some(p), stash, false);
                    self.queues[qid].push(QueueVal::Many(r));
                }
            }
            self.ast.node_mut(node).children.clear();
            return Ok(());
        }

        let enclose = enclose_brace(self.ast, node);
        let queue_id = self.nearest_queue(node);
        let children = self.ast.node(node).children.clone();
        let node_ty = ty;

        for (i, child) in children.iter().enumerate() {
            let cty = self.ast.node(*child).ty;
            if cty == NodeType::Comma && node_ty == NodeType::Brace {
                if i == 1 {
                    self.queues[queue_id].push(QueueVal::One(Rope::empty()));
                }
                self.queues[queue_id].push(QueueVal::One(Rope::empty()));
                continue;
            }
            if cty == NodeType::Close {
                let stash = std::mem::take(&mut self.queues[queue_id]);
                let popped = self.queues[qid].pop();
                let r = append(popped, QueueVal::Many(stash), enclose);
                self.queues[qid].push(QueueVal::Many(r));
                continue;
            }
            let cval = self.ast.node(*child).value.clone();
            if cty != NodeType::Open {
                if let Some(v) = cval {
                    if !v.is_empty() {
                        let popped = self.queues[queue_id].pop();
                        let r = append(popped, QueueVal::One(Rope::from_units(v)), false);
                        self.queues[queue_id].push(QueueVal::Many(r));
                        continue;
                    }
                }
            }
            if self.ast.node(*child).has_nodes() {
                self.walk(*child, Some(node))?;
            }
        }
        Ok(())
    }
}

/// Expand an AST. Returns the flat list of expansion strings.
pub fn expand(ast: &mut Ast, opts: &Options) -> Result<Vec<Vec<u16>>, BracesError> {
    let n = ast.nodes.len();
    let mut expander = Expander {
        ast,
        queues: vec![Vec::new(); n],
        opts,
    };
    let root = expander.ast.root;
    expander.walk(root, None)?;
    let q = std::mem::take(&mut expander.queues[root]);
    Ok(flatten::<Rope>(q)
        .into_iter()
        .map(|r| r.materialize())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rope_of(parts: &[&str]) -> Rope {
        Rope {
            segs: parts.iter().map(|p| seg_of(&units(p))).collect(),
        }
    }

    fn s(v: &[u16]) -> String {
        String::from_utf16(v).unwrap()
    }

    #[test]
    fn concat_joins_segments_once() {
        let a = rope_of(&["user-", "200"]);
        let b = rope_of(&["/project-"]);
        let c = a.concat(&b);
        assert_eq!(s(&c.materialize()), "user-200/project-");
        // sources are untouched (shared segments, no rebuild)
        assert_eq!(s(&a.materialize()), "user-200");
        assert_eq!(s(&b.materialize()), "/project-");
    }

    #[test]
    fn concat_with_empty_is_identity() {
        let a = rope_of(&["x"]);
        assert_eq!(s(&a.concat(&Rope::empty()).materialize()), "x");
        assert_eq!(s(&Rope::empty().concat(&a).materialize()), "x");
        assert_eq!(s(&Rope::empty().materialize()), "");
    }

    #[test]
    fn enclosed_wraps_inner_in_literal_braces() {
        let pre = rope_of(&["pre-"]);
        let inner = rope_of(&["a,b"]);
        assert_eq!(s(&pre.concat_enclosed(&inner).materialize()), "pre-{a,b}");
        assert_eq!(s(&inner.enclosed().materialize()), "{a,b}");
        assert_eq!(s(&Rope::empty().enclosed().materialize()), "{}");
    }

    #[test]
    fn append_cartesian_over_ropes() {
        let q = Some(QueueVal::Many(vec![
            QueueVal::One(rope_of(&["a"])),
            QueueVal::One(rope_of(&["b"])),
        ]));
        let stash = QueueVal::Many(vec![
            QueueVal::One(rope_of(&["1"])),
            QueueVal::One(rope_of(&["2"])),
        ]);
        let out = append(q, stash, false);
        let flat: Vec<String> = flatten::<Rope>(out)
            .iter()
            .map(|r| s(&r.materialize()))
            .collect();
        assert_eq!(flat, ["a1", "a2", "b1", "b2"]);
    }

    #[test]
    fn append_enclose_branch() {
        // q empty + enclose: every stash element is brace-wrapped.
        let stash = QueueVal::Many(vec![QueueVal::One(rope_of(&["a,b"]))]);
        let out = append(Some(QueueVal::Many(vec![])), stash, true);
        let flat: Vec<String> = flatten::<Rope>(out)
            .iter()
            .map(|r| s(&r.materialize()))
            .collect();
        assert_eq!(flat, ["{a,b}"]);
    }
}
