//! AST arena. Mirrors the JS parse tree of micromatch/braces: nodes carry
//! `parent`/`prev` links (by arena index here), and the parser mutates the
//! tree (text merging, range-node type flips, unbalanced-block splicing)
//! exactly like the reference.

pub type NodeId = usize;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeType {
    Root,
    Bos,
    Eos,
    Text,
    Dot,
    Range,
    Open,
    Close,
    Comma,
    Brace,
    Paren,
}

impl NodeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeType::Root => "root",
            NodeType::Bos => "bos",
            NodeType::Eos => "eos",
            NodeType::Text => "text",
            NodeType::Dot => "dot",
            NodeType::Range => "range",
            NodeType::Open => "open",
            NodeType::Close => "close",
            NodeType::Comma => "comma",
            NodeType::Brace => "brace",
            NodeType::Paren => "paren",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub ty: NodeType,
    /// `None` when the JS node has no `value` property (bos/eos/brace/paren).
    pub value: Option<Vec<u16>>,
    /// brace only: parameter-expansion lookalike flag (`${...}`).
    pub dollar: bool,
    /// brace only: nesting depth at creation.
    pub depth: u32,
    /// brace only: comma count.
    pub commas: u32,
    /// brace only: range-segment count.
    pub ranges: u32,
    /// `None` = JS `undefined`; `Some(b)` = explicitly set.
    pub invalid: Option<bool>,
    /// set by the unbalanced-block splice pass on former open/close leaves.
    pub is_open: bool,
    pub is_close: bool,
    /// brace only.
    pub open: bool,
    /// brace only.
    pub close: bool,
    pub children: Vec<NodeId>,
    pub parent: Option<NodeId>,
    pub prev: Option<NodeId>,
}

impl Node {
    pub fn new(ty: NodeType) -> Self {
        Node {
            ty,
            value: None,
            dollar: false,
            depth: 0,
            commas: 0,
            ranges: 0,
            invalid: None,
            is_open: false,
            is_close: false,
            open: false,
            close: false,
            children: Vec::new(),
            parent: None,
            prev: None,
        }
    }

    pub fn text(value: Vec<u16>) -> Self {
        let mut n = Node::new(NodeType::Text);
        n.value = Some(value);
        n
    }

    /// JS truthiness of `node.nodes`: only root/brace/paren carry a nodes
    /// array (an empty array is still truthy in JS).
    #[inline]
    pub fn has_nodes(&self) -> bool {
        matches!(self.ty, NodeType::Root | NodeType::Brace | NodeType::Paren)
    }

    /// JS truthiness of `node.value`: present and non-empty.
    #[inline]
    pub fn has_value(&self) -> bool {
        matches!(&self.value, Some(v) if !v.is_empty())
    }
}

#[derive(Debug)]
pub struct Ast {
    pub nodes: Vec<Node>,
    pub root: NodeId,
    /// Original input (UTF-16), kept for parity with `ast.input`.
    pub input: Vec<u16>,
}

impl Ast {
    pub fn alloc(&mut self, node: Node) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(node);
        id
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id]
    }
}

/// Maximum AST nesting depth the recursive walkers (expand / compile /
/// stringify) will traverse — CVE-2026-93687 hardening.
///
/// Justification: the JS reference dies with `RangeError: Maximum call
/// stack size exceeded` at a brace-chain depth of ~4,999 on a stock V8
/// (empirically verified), and the (reference-faithful) 10,000-char
/// input cap means no string input can express a depth above ~5,000
/// anyway — so a 10,000 limit can never reject an input the reference
/// handles. It exists so the walkers stay safe if the length cap is
/// ever raised or an AST is built programmatically.
pub const MAX_AST_DEPTH: usize = 10_000;

/// Iterative (explicit-stack) depth of the tree rooted at `root`;
/// the root itself counts as depth 1.
pub fn tree_depth(nodes: &[Node], root: NodeId) -> usize {
    let mut max = 0usize;
    let mut stack: Vec<(NodeId, usize)> = vec![(root, 1)];
    while let Some((id, d)) = stack.pop() {
        if d > max {
            max = d;
        }
        for &c in &nodes[id].children {
            stack.push((c, d + 1));
        }
    }
    max
}

/// The CVE-2026-93687 guard: typed `RangeError` instead of a native
/// stack overflow when an AST is too deep for the recursive walkers.
pub fn check_depth(nodes: &[Node], root: NodeId) -> Result<(), crate::error::BracesError> {
    if tree_depth(nodes, root) > MAX_AST_DEPTH {
        return Err(crate::error::BracesError::DepthLimit);
    }
    Ok(())
}
