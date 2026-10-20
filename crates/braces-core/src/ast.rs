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
