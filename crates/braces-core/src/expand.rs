//! Faithful port of micromatch/braces `lib/expand.js`.
//!
//! The reference's queue/`append` cartesian-product dance is reproduced
//! exactly (including `undefined`-pop defaulting to `''` and deep
//! flattening). Includes the no-brace fast path (LEARNINGS.md #4): a
//! brace-free AST stringifies to a single result without any queue
//! machinery.

use crate::ast::{Ast, NodeId, NodeType};
use crate::error::BracesError;
use crate::fill::{fill, FillOptions, FillOut};
use crate::options::Options;
use crate::stringify::stringify_subtree;
use crate::utils::{enclose_brace, exceeds_limit, flatten, reduce, QueueVal};

type Q = QueueVal<Vec<u16>>;

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Port of `append(queue = '', stash = '', enclose = false)`.
fn append(queue: Option<Q>, stash: Q, enclose: bool) -> Vec<Q> {
    // [].concat(queue): None (JS undefined) → ['']; One → [it]; Many → elements
    let q: Vec<Q> = match queue {
        None => vec![QueueVal::One(Vec::new())],
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
            return flatten::<Vec<u16>>(st)
                .into_iter()
                .map(|e| {
                    let mut w = units("{");
                    w.extend(e);
                    w.push(b'}' as u16);
                    QueueVal::One(w)
                })
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
                            let e2 = if enclose {
                                let mut w = units("{");
                                w.extend_from_slice(e);
                                w.push(b'}' as u16);
                                w
                            } else {
                                e.clone()
                            };
                            let mut c = s.clone();
                            c.extend_from_slice(&e2);
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
    flatten::<Vec<u16>>(result)
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
            let r = append(popped, QueueVal::One(s), false);
            self.queues[qid].push(QueueVal::Many(r));
            return Ok(());
        }

        if ty == NodeType::Brace && invalid != Some(true) && nchildren == 2 {
            let popped = self.queues[qid].pop();
            let r = append(popped, QueueVal::One(units("{}")), false);
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
                FillOut::List(items) if !items.is_empty() => {
                    QueueVal::Many(items.into_iter().map(QueueVal::One).collect())
                }
                _ => QueueVal::One(stringify_subtree(
                    &self.ast.nodes,
                    node,
                    self.opts.escape_invalid,
                )),
            };
            let popped = self.queues[qid].pop();
            let r = append(popped, stash, false);
            self.queues[qid].push(QueueVal::Many(r));
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
                    self.queues[queue_id].push(QueueVal::One(Vec::new()));
                }
                self.queues[queue_id].push(QueueVal::One(Vec::new()));
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
                        let r = append(popped, QueueVal::One(v), false);
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
    Ok(flatten::<Vec<u16>>(q))
}
