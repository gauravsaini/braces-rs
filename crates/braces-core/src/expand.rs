//! Faithful port of micromatch/braces `lib/expand.js`.
//!
//! The reference's queue/`append` cartesian-product dance is reproduced
//! exactly (including `undefined`-pop defaulting to `''` and deep
//! flattening). Includes the no-brace fast path (LEARNINGS.md #4): a
//! brace-free AST stringifies to a single result without any queue
//! machinery.
//!
//! Segment-join walk (perf restructure): queue entries are `Rope`s —
//! binary trees of shared segments (`Rc<[u16]>`) — instead of fully
//! materialised strings. `append` is then a single `Rc` allocation per
//! concatenation (the V8 cons-string equivalent) and each output string
//! is joined exactly once, at final materialisation. The old walk
//! rebuilt every output string at every segment append, which is
//! quadratic on multi-group patterns (e.g.
//! `user-{200..300}/project-{a,b,c}-{1..10}`) where V8's rope
//! concatenation made the JS reference cheaper.

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

#[derive(Debug)]
enum RopeNode {
    Leaf(Seg),
    Concat(Rope, Rope),
}

/// An output string under construction: a binary tree of shared
/// segments. Cloning a `Rope` is one `Rc` bump; concatenating is one
/// `Rc` allocation — segment bytes are never copied until the single
/// final join (`to_units` / `write_into`).
#[derive(Clone, Debug)]
pub struct Rope {
    node: Option<Rc<RopeNode>>,
    len: usize, // total UTF-16 units
}

impl Default for Rope {
    fn default() -> Self {
        Self::empty()
    }
}

impl Rope {
    pub fn empty() -> Self {
        Rope { node: None, len: 0 }
    }

    pub fn from_units(v: Vec<u16>) -> Self {
        if v.is_empty() {
            return Rope::empty();
        }
        let len = v.len();
        Rope {
            node: Some(Rc::new(RopeNode::Leaf(Rc::from(v)))),
            len,
        }
    }

    fn leaf(seg: Seg) -> Self {
        let len = seg.len();
        Rope {
            node: Some(Rc::new(RopeNode::Leaf(seg))),
            len,
        }
    }

    /// Total length in UTF-16 code units.
    pub fn unit_len(&self) -> usize {
        self.len
    }

    /// `self + other` as a new rope (one allocation, no byte copies).
    fn concat(&self, other: &Rope) -> Rope {
        match (&self.node, &other.node) {
            (None, _) => other.clone(),
            (_, None) => self.clone(),
            (Some(_), Some(_)) => Rope {
                node: Some(Rc::new(RopeNode::Concat(self.clone(), other.clone()))),
                len: self.len + other.len,
            },
        }
    }

    /// `self + "{" + inner + "}"` — the enclose path for invalid braces.
    fn concat_enclosed(&self, inner: &Rope) -> Rope {
        self.concat(&Rope::leaf(seg_of(&[b'{' as u16])))
            .concat(inner)
            .concat(&Rope::leaf(seg_of(&[b'}' as u16])))
    }

    /// `"{" + self + "}"` as a single rope (q-empty enclose branch).
    fn enclosed(&self) -> Rope {
        Rope::leaf(seg_of(&[b'{' as u16]))
            .concat(self)
            .concat(&Rope::leaf(seg_of(&[b'}' as u16])))
    }

    /// Append this rope's bytes to `out` — the one and only full copy of
    /// the output bytes. Traversal is recursive; depth is bounded by the
    /// pattern's segment count (concat chains lean right by construction
    /// and realistic patterns stay well under a hundred deep).
    pub fn write_into(&self, out: &mut Vec<u16>) {
        fn rec(node: &RopeNode, out: &mut Vec<u16>) {
            match node {
                RopeNode::Leaf(s) => out.extend_from_slice(s),
                RopeNode::Concat(l, r) => {
                    if let Some(n) = &l.node {
                        rec(n, out);
                    }
                    if let Some(n) = &r.node {
                        rec(n, out);
                    }
                }
            }
        }
        if let Some(n) = &self.node {
            rec(n, out);
        }
    }

    /// Join the segments into the final string.
    pub fn to_units(&self) -> Vec<u16> {
        let mut out = Vec::with_capacity(self.len);
        self.write_into(&mut out);
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

/// Expand an AST. Returns the flat list of expanded output ropes;
/// callers join them (`Rope::to_units` / `write_into`) at the boundary
/// where real strings are needed.
pub fn expand(ast: &mut Ast, opts: &Options) -> Result<Vec<Rope>, BracesError> {
    let n = ast.nodes.len();
    let mut expander = Expander {
        ast,
        queues: vec![Vec::new(); n],
        opts,
    };
    let root = expander.ast.root;
    expander.walk(root, None)?;
    let q = std::mem::take(&mut expander.queues[root]);
    Ok(flatten::<Rope>(q))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rope_of(parts: &[&str]) -> Rope {
        let mut r = Rope::empty();
        for p in parts {
            r = r.concat(&Rope::from_units(units(p)));
        }
        r
    }

    fn s(v: &[u16]) -> String {
        String::from_utf16(v).unwrap()
    }

    #[test]
    fn concat_joins_segments_once() {
        let a = rope_of(&["user-", "200"]);
        let b = rope_of(&["/project-"]);
        let c = a.concat(&b);
        assert_eq!(s(&c.to_units()), "user-200/project-");
        // sources are untouched (shared segments, no rebuild)
        assert_eq!(s(&a.to_units()), "user-200");
        assert_eq!(s(&b.to_units()), "/project-");
    }

    #[test]
    fn concat_with_empty_is_identity() {
        let a = rope_of(&["x"]);
        assert_eq!(s(&a.concat(&Rope::empty()).to_units()), "x");
        assert_eq!(s(&Rope::empty().concat(&a).to_units()), "x");
        assert_eq!(s(&Rope::empty().to_units()), "");
    }

    #[test]
    fn enclosed_wraps_inner_in_literal_braces() {
        let pre = rope_of(&["pre-"]);
        let inner = rope_of(&["a,b"]);
        assert_eq!(s(&pre.concat_enclosed(&inner).to_units()), "pre-{a,b}");
        assert_eq!(s(&inner.enclosed().to_units()), "{a,b}");
        assert_eq!(s(&Rope::empty().enclosed().to_units()), "{}");
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
            .map(|r| s(&r.to_units()))
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
            .map(|r| s(&r.to_units()))
            .collect();
        assert_eq!(flat, ["{a,b}"]);
    }
}
