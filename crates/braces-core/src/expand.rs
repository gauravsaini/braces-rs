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
use std::cell::RefCell;
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
    Concat { l: u32, r: u32 },
}

/// Bump-allocated rope-node arena, one per expand call (the V8
/// young-generation equivalent: node allocation is a `Vec` push, a few
/// nanoseconds, and ropes are plain `Copy` indices — no `Rc` bumps at
/// all in the walk).
type RopeArena = Rc<RefCell<Vec<RopeNode>>>;

fn new_arena() -> RopeArena {
    // Index 0 is reserved for the empty rope; real nodes start at 1.
    Rc::new(RefCell::new(vec![RopeNode::Leaf(Rc::from(&[][..]))]))
}

/// An output string under construction: a copyable handle into a shared
/// rope-node arena. Concatenation is a single arena push; the segment
/// bytes are copied exactly once, at the final join.
#[derive(Clone, Debug, Default)]
pub struct Rope {
    arena: Option<RopeArena>,
    idx: u32, // 0 = empty
    len: usize,
}

impl Rope {
    fn with_arena(arena: &RopeArena) -> Self {
        Rope {
            arena: Some(Rc::clone(arena)),
            idx: 0,
            len: 0,
        }
    }

    pub fn empty() -> Self {
        Rope {
            arena: None,
            idx: 0,
            len: 0,
        }
    }

    fn arena(&self) -> RopeArena {
        self.arena.clone().unwrap_or_else(new_arena)
    }

    fn alloc_node(&self, node: RopeNode) -> u32 {
        let arena = self.arena();
        let mut a = arena.borrow_mut();
        a.push(node);
        (a.len() - 1) as u32
    }

    pub fn from_units(v: Vec<u16>) -> Self {
        Self::from_units_in(&new_arena(), v)
    }

    fn from_units_in(arena: &RopeArena, v: Vec<u16>) -> Self {
        if v.is_empty() {
            return Rope::with_arena(arena);
        }
        let len = v.len();
        let mut r = Rope::with_arena(arena);
        r.idx = r.alloc_node(RopeNode::Leaf(Rc::from(v)));
        r.len = len;
        r
    }

    /// Total length in UTF-16 code units.
    pub fn unit_len(&self) -> usize {
        self.len
    }

    /// Adopt `other` into this rope's arena (deep transplant), used only
    /// when arenas differ — inside one expand call all ropes share the
    /// arena, so this stays O(1).
    fn other_idx(&self, other: &Rope) -> u32 {
        match (&self.arena, &other.arena) {
            (Some(a), Some(b)) if Rc::ptr_eq(a, b) => other.idx,
            _ => {
                // Foreign arena: materialise once into a leaf. Rare by
                // construction (only tests mix arenas).
                let units = other.to_units();
                self.alloc_node(RopeNode::Leaf(Rc::from(units)))
            }
        }
    }

    /// `self + other` as a new rope (one arena push, no byte copies).
    fn concat(&self, other: &Rope) -> Rope {
        if self.idx == 0 {
            return other.clone();
        }
        if other.idx == 0 {
            return self.clone();
        }
        let l = self.idx;
        let r = self.other_idx(other);
        let mut out = Rope {
            arena: Some(self.arena()),
            idx: 0,
            len: self.len + other.len,
        };
        out.idx = out.alloc_node(RopeNode::Concat { l, r });
        out
    }

    /// `self + "{" + inner + "}"` — the enclose path for invalid braces.
    fn concat_enclosed(&self, inner: &Rope) -> Rope {
        let arena = self.arena();
        let l = Rope::from_units_in(&arena, vec![b'{' as u16]);
        let rr = Rope::from_units_in(&arena, vec![b'}' as u16]);
        self.concat(&l).concat(inner).concat(&rr)
    }

    /// `"{" + self + "}"` as a single rope (q-empty enclose branch).
    fn enclosed(&self) -> Rope {
        let arena = self.arena();
        let l = Rope::from_units_in(&arena, vec![b'{' as u16]);
        let r = Rope::from_units_in(&arena, vec![b'}' as u16]);
        l.concat(self).concat(&r)
    }

    /// Append this rope's bytes to `out` — the one and only full copy of
    /// the output bytes. Traversal is recursive; depth is bounded by the
    /// pattern's segment count (concat chains lean right by construction
    /// and realistic patterns stay well under a hundred deep).
    pub fn write_into(&self, out: &mut Vec<u16>) {
        fn rec(arena: &[RopeNode], idx: u32, out: &mut Vec<u16>) {
            match &arena[idx as usize] {
                RopeNode::Leaf(s) => out.extend_from_slice(s),
                RopeNode::Concat { l, r } => {
                    rec(arena, *l, out);
                    rec(arena, *r, out);
                }
            }
        }
        if self.idx == 0 {
            return;
        }
        let arena = self.arena();
        let a = arena.borrow();
        rec(&a, self.idx, out);
    }

    /// Join the segments into the final string.
    pub fn to_units(&self) -> Vec<u16> {
        let mut out = Vec::with_capacity(self.len);
        self.write_into(&mut out);
        out
    }

    /// Append many ropes into `out`. Ropes produced by one expand() call
    /// share a single arena — it is borrowed exactly once, and traversal
    /// is iterative over a reusable index stack (recursive traversal of
    /// ~40k rope nodes per expansion measurably dominated the pack cost).
    pub fn write_many(ropes: &[Rope], out: &mut Vec<u16>) {
        let shared = ropes.first().and_then(|r| r.arena.clone());
        let all_shared = match &shared {
            Some(a) => ropes
                .iter()
                .all(|r| r.arena.as_ref().is_some_and(|o| Rc::ptr_eq(o, a))),
            None => false,
        };
        if let (Some(a), true) = (shared, all_shared) {
            let arena = a.borrow();
            let mut stack: Vec<u32> = Vec::with_capacity(64);
            for r in ropes {
                if r.idx == 0 {
                    continue;
                }
                stack.push(r.idx);
                while let Some(idx) = stack.pop() {
                    match &arena[idx as usize] {
                        RopeNode::Leaf(s) => out.extend_from_slice(s),
                        RopeNode::Concat { l, r } => {
                            stack.push(*r);
                            stack.push(*l);
                        }
                    }
                }
            }
        } else {
            for r in ropes {
                r.write_into(out);
            }
        }
    }
}

/// Walk-internal rope handle: a `Copy` (index, length) pair into the
/// Expander's plain `Vec` node arena — no `Rc`/`RefCell` traffic in the
/// hot loop (bump allocation only, the V8 young-gen equivalent).
#[derive(Clone, Copy, Debug, Default)]
struct W {
    idx: u32, // 0 = empty
    len: usize,
}

impl W {
    const EMPTY: W = W { idx: 0, len: 0 };
}

type Q = QueueVal<W>;

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn concat_in(arena: &mut Vec<RopeNode>, l: W, r: W) -> W {
    if l.idx == 0 {
        return r;
    }
    if r.idx == 0 {
        return l;
    }
    let len = l.len + r.len;
    arena.push(RopeNode::Concat { l: l.idx, r: r.idx });
    W {
        idx: (arena.len() - 1) as u32,
        len,
    }
}

fn leaf_in(arena: &mut Vec<RopeNode>, v: Vec<u16>) -> W {
    if v.is_empty() {
        return W::EMPTY;
    }
    let len = v.len();
    arena.push(RopeNode::Leaf(Rc::from(v)));
    W {
        idx: (arena.len() - 1) as u32,
        len,
    }
}

/// `s + "{" + inner + "}"` (enclose path for invalid braces).
fn concat_enclosed_in(arena: &mut Vec<RopeNode>, s: W, inner: W) -> W {
    let l = leaf_in(arena, vec![b'{' as u16]);
    let r = leaf_in(arena, vec![b'}' as u16]);
    let a = concat_in(arena, s, l);
    let b = concat_in(arena, a, inner);
    concat_in(arena, b, r)
}

/// `"{" + w + "}"` (q-empty enclose branch).
fn enclosed_in(arena: &mut Vec<RopeNode>, w: W) -> W {
    let l = leaf_in(arena, vec![b'{' as u16]);
    let r = leaf_in(arena, vec![b'}' as u16]);
    let a = concat_in(arena, l, w);
    concat_in(arena, a, r)
}

/// Port of `append(queue = '', stash = '', enclose = false)`, returning
/// the flattened output ropes directly (the reference's deep-flatten of
/// the nested result is fused into the build).
fn append(arena: &mut Vec<RopeNode>, queue: Option<Q>, stash: Q, enclose: bool) -> Vec<W> {
    // [].concat(queue): None (JS undefined) → ['']; One → [it]; Many → elements
    let q: Vec<Q> = match queue {
        None => vec![QueueVal::One(W::EMPTY)],
        Some(QueueVal::One(s)) => vec![QueueVal::One(s)],
        Some(QueueVal::Many(v)) => v,
    };
    let st: Vec<Q> = match stash {
        QueueVal::One(s) => vec![QueueVal::One(s)],
        QueueVal::Many(v) => v,
    };
    if st.is_empty() {
        return flatten::<W>(q);
    }
    if q.is_empty() {
        let flat = flatten::<W>(st);
        if enclose {
            return flat.into_iter().map(|e| enclosed_in(arena, e)).collect();
        }
        return flat;
    }
    let mut result: Vec<W> = Vec::new();
    for item in q {
        match item {
            QueueVal::Many(arr) => {
                for value in arr {
                    result.extend(append(arena, Some(value), QueueVal::Many(st.clone()), enclose));
                }
            }
            QueueVal::One(s) => {
                for ele in &st {
                    match ele {
                        QueueVal::One(e) => {
                            result.push(if enclose {
                                concat_enclosed_in(arena, s, *e)
                            } else {
                                concat_in(arena, s, *e)
                            });
                        }
                        QueueVal::Many(a) => {
                            result.extend(append(
                                arena,
                                Some(QueueVal::One(s)),
                                QueueVal::Many(a.clone()),
                                enclose,
                            ));
                        }
                    }
                }
            }
        }
    }
    result
}

/// Wrap flat append output as a single queue element (as the reference's
/// walk pushes the whole expansion list as one queue entry).
fn as_queue(flat: Vec<W>) -> Q {
    QueueVal::Many(flat.into_iter().map(QueueVal::One).collect())
}

pub struct Expander<'a> {
    ast: &'a mut Ast,
    queues: Vec<Vec<Q>>,
    opts: &'a Options,
    arena: Vec<RopeNode>,
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
            let leaf = leaf_in(&mut self.arena, s);
            let popped = self.queues[qid].pop();
            let r = append(&mut self.arena, popped, QueueVal::One(leaf), false);
            self.queues[qid].push(as_queue(r));
            return Ok(());
        }

        if ty == NodeType::Brace && invalid != Some(true) && nchildren == 2 {
            let leaf = leaf_in(&mut self.arena, units("{}"));
            let popped = self.queues[qid].pop();
            let r = append(&mut self.arena, popped, QueueVal::One(leaf), false);
            self.queues[qid].push(as_queue(r));
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
                        .map(|i| QueueVal::One(leaf_in(&mut self.arena, i)))
                        .collect(),
                ),
                _ => QueueVal::One(leaf_in(
                    &mut self.arena,
                    stringify_subtree(&self.ast.nodes, node, self.opts.escape_invalid),
                )),
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
                    let r = append(&mut self.arena, Some(p), stash, false);
                    self.queues[qid].push(as_queue(r));
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
                    self.queues[queue_id].push(QueueVal::One(W::EMPTY));
                }
                self.queues[queue_id].push(QueueVal::One(W::EMPTY));
                continue;
            }
            if cty == NodeType::Close {
                let stash = std::mem::take(&mut self.queues[queue_id]);
                let popped = self.queues[qid].pop();
                let r = append(&mut self.arena, popped, QueueVal::Many(stash), enclose);
                self.queues[qid].push(as_queue(r));
                continue;
            }
            let cval = self.ast.node(*child).value.clone();
            if cty != NodeType::Open {
                if let Some(v) = cval {
                    if !v.is_empty() {
                        let leaf = leaf_in(&mut self.arena, v);
                        let popped = self.queues[queue_id].pop();
                        let r = append(&mut self.arena, popped, QueueVal::One(leaf), false);
                        self.queues[queue_id].push(as_queue(r));
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
        // Index 0 is the reserved empty node, matching W::EMPTY.
        arena: vec![RopeNode::Leaf(Rc::from(&[][..]))],
    };
    let root = expander.ast.root;
    expander.walk(root, None)?;
    let q = std::mem::take(&mut expander.queues[root]);
    let ws = flatten::<W>(q);
    let shared: RopeArena = Rc::new(RefCell::new(std::mem::take(&mut expander.arena)));
    Ok(ws
        .into_iter()
        .map(|w| Rope {
            arena: Some(Rc::clone(&shared)),
            idx: w.idx,
            len: w.len,
        })
        .collect())
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

    // ---- walk-handle (W) append, over a local arena ----

    fn test_arena() -> Vec<RopeNode> {
        vec![RopeNode::Leaf(Rc::from(&[][..]))]
    }

    fn w_of(arena: &mut Vec<RopeNode>, parts: &[&str]) -> W {
        let mut w = W::EMPTY;
        for p in parts {
            let leaf = leaf_in(arena, units(p));
            w = concat_in(arena, w, leaf);
        }
        w
    }

    fn w_str(arena: &[RopeNode], w: W) -> String {
        fn rec(arena: &[RopeNode], idx: u32, out: &mut Vec<u16>) {
            match &arena[idx as usize] {
                RopeNode::Leaf(seg) => out.extend_from_slice(seg),
                RopeNode::Concat { l, r } => {
                    rec(arena, *l, out);
                    rec(arena, *r, out);
                }
            }
        }
        let mut v = Vec::new();
        if w.idx != 0 {
            rec(arena, w.idx, &mut v);
        }
        s(&v)
    }

    #[test]
    fn append_cartesian_over_walk_ropes() {
        let mut arena = test_arena();
        let a = w_of(&mut arena, &["a"]);
        let b = w_of(&mut arena, &["b"]);
        let one = w_of(&mut arena, &["1"]);
        let two = w_of(&mut arena, &["2"]);
        let q = Some(QueueVal::Many(vec![QueueVal::One(a), QueueVal::One(b)]));
        let stash = QueueVal::Many(vec![QueueVal::One(one), QueueVal::One(two)]);
        let out = append(&mut arena, q, stash, false);
        let flat: Vec<String> = out.iter().map(|w| w_str(&arena, *w)).collect();
        assert_eq!(flat, ["a1", "a2", "b1", "b2"]);
    }

    #[test]
    fn append_enclose_branch() {
        // q empty + enclose: every stash element is brace-wrapped.
        let mut arena = test_arena();
        let ab = w_of(&mut arena, &["a,b"]);
        let stash = QueueVal::Many(vec![QueueVal::One(ab)]);
        let out = append(&mut arena, Some(QueueVal::Many(vec![])), stash, true);
        let flat: Vec<String> = out.iter().map(|w| w_str(&arena, *w)).collect();
        assert_eq!(flat, ["{a,b}"]);
    }

    #[test]
    fn append_segment_join_multigroup_shape() {
        // user-{200..202} x {a,b} x {1..2} through raw appends: outputs
        // are segment trees, joined once here.
        let mut arena = test_arena();
        let pre = w_of(&mut arena, &["user-"]);
        let mut cur: Vec<W> = vec![pre];
        for seg in ["200", "201", "202"] {
            let _ = seg;
        }
        // stage 1: prefix x range options
        let opts1: Vec<W> = ["200", "201", "202"].iter().map(|t| w_of(&mut arena, &[t])).collect();
        let mut next = Vec::new();
        for c in &cur {
            for o in &opts1 {
                next.push(concat_in(&mut arena, *c, *o));
            }
        }
        cur = next;
        // stage 2: x list options
        let mid = w_of(&mut arena, &["/p-"]);
        cur = cur.iter().map(|c| concat_in(&mut arena, *c, mid)).collect();
        let opts2: Vec<W> = ["a", "b"].iter().map(|t| w_of(&mut arena, &[t])).collect();
        let mut next = Vec::new();
        for c in &cur {
            for o in &opts2 {
                next.push(concat_in(&mut arena, *c, *o));
            }
        }
        cur = next;
        let got: Vec<String> = cur.iter().map(|w| w_str(&arena, *w)).collect();
        assert_eq!(
            got,
            [
                "user-200/p-a", "user-200/p-b", "user-201/p-a", "user-201/p-b", "user-202/p-a",
                "user-202/p-b"
            ]
        );
    }
}
