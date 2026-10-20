//! Faithful port of micromatch/braces `lib/parse.js`.
//!
//! Single pass over UTF-16 code units with the reference's exact node
//! merging, dot/range state machine, quote and bracket consumption, and the
//! unbalanced-block splice pass at the end. Every observable quirk is kept:
//! `]` is always escaped to `\]`, NBSP/ZWNBSP are stripped, a trailing
//! backslash appends the literal string "undefined" (JS `'' + undefined`),
//! and paren blocks are transparent containers.

use crate::ast::{Ast, Node, NodeId, NodeType};
use crate::error::BracesError;
use crate::jsnum;
use crate::options::ParseOpts;
use crate::stringify::stringify_subtree;

pub const MAX_LENGTH: f64 = 10000.0;

const CH_BACKSLASH: u16 = b'\\' as u16;
const CH_BACKTICK: u16 = b'`' as u16;
const CH_COMMA: u16 = b',' as u16;
const CH_DOT: u16 = b'.' as u16;
const CH_LPAREN: u16 = b'(' as u16;
const CH_RPAREN: u16 = b')' as u16;
const CH_LBRACE: u16 = b'{' as u16;
const CH_RBRACE: u16 = b'}' as u16;
const CH_LBRACKET: u16 = b'[' as u16;
const CH_RBRACKET: u16 = b']' as u16;
const CH_DQUOTE: u16 = b'"' as u16;
const CH_SQUOTE: u16 = b'\'' as u16;
const CH_DOLLAR: u16 = b'$' as u16;
const CH_NBSP: u16 = 0x00A0;
const CH_ZWNBSP: u16 = 0xFEFF;

fn undefined_units() -> Vec<u16> {
    "undefined".encode_utf16().collect()
}

struct Parser<'a> {
    input: &'a [u16],
    ast: Ast,
    stack: Vec<NodeId>,
    block: NodeId,
    prev: NodeId,
    brackets: i32,
    index: usize,
    depth: u32,
    keep_escaping: bool,
    keep_quotes: bool,
}

impl<'a> Parser<'a> {
    fn advance(&mut self) -> Option<u16> {
        let v = self.input.get(self.index).copied();
        self.index += 1;
        v
    }

    /// Port of parse.js `push`. Returns the pushed node id, or `None` when
    /// the node was merged into `prev` (text+text merge).
    fn push(&mut self, node: Node) -> Option<NodeId> {
        let mut node = node;
        if node.ty == NodeType::Text && self.ast.node(self.prev).ty == NodeType::Dot {
            self.ast.node_mut(self.prev).ty = NodeType::Text;
        }
        if self.ast.node(self.prev).ty == NodeType::Text && node.ty == NodeType::Text {
            let v = node.value.take().unwrap_or_default();
            self.ast
                .node_mut(self.prev)
                .value
                .as_mut()
                .expect("text node has value")
                .extend_from_slice(&v);
            return None;
        }
        let block = self.block;
        let prev = self.prev;
        node.parent = Some(block);
        node.prev = Some(prev);
        let id = self.ast.alloc(node);
        self.ast.node_mut(block).children.push(id);
        self.prev = id;
        Some(id)
    }

    fn push_text(&mut self, value: Vec<u16>) -> Option<NodeId> {
        self.push(Node::text(value))
    }

    fn run(&mut self) {
        let length = self.input.len();
        self.push(Node::new(NodeType::Bos));

        while self.index < length {
            self.block = *self.stack.last().expect("stack nonempty");
            let value = self.advance().expect("index < length");

            // Invalid chars: NBSP / ZWNBSP are stripped.
            if value == CH_ZWNBSP || value == CH_NBSP {
                continue;
            }

            // Escaped chars.
            if value == CH_BACKSLASH {
                let mut v: Vec<u16> = Vec::with_capacity(2);
                if self.keep_escaping {
                    v.push(CH_BACKSLASH);
                }
                match self.advance() {
                    Some(c) => v.push(c),
                    None => v.extend(undefined_units()),
                }
                self.push_text(v);
                continue;
            }

            // ']' is always escaped.
            if value == CH_RBRACKET {
                self.push_text(vec![CH_BACKSLASH, CH_RBRACKET]);
                continue;
            }

            // '[' ... ']' consumed as one text token.
            if value == CH_LBRACKET {
                self.brackets += 1;
                let mut text: Vec<u16> = vec![value];
                while self.index < length {
                    let next = self.advance().expect("index < length");
                    text.push(next);
                    if next == CH_LBRACKET {
                        self.brackets += 1;
                        continue;
                    }
                    if next == CH_BACKSLASH {
                        match self.advance() {
                            Some(c) => text.push(c),
                            None => text.extend(undefined_units()),
                        }
                        continue;
                    }
                    if next == CH_RBRACKET {
                        self.brackets -= 1;
                        if self.brackets == 0 {
                            break;
                        }
                    }
                }
                self.push_text(text);
                continue;
            }

            // Parentheses.
            if value == CH_LPAREN {
                let mut paren = Node::new(NodeType::Paren);
                paren.children = Vec::new();
                let id = self.push(paren).expect("paren never merges");
                self.block = id;
                self.stack.push(id);
                self.push_text(vec![value]);
                continue;
            }
            if value == CH_RPAREN {
                if self.ast.node(self.block).ty != NodeType::Paren {
                    self.push_text(vec![value]);
                    continue;
                }
                self.block = self.stack.pop().expect("paren on stack");
                self.push_text(vec![value]);
                self.block = *self.stack.last().expect("stack nonempty");
                continue;
            }

            // Quotes.
            if value == CH_DQUOTE || value == CH_SQUOTE || value == CH_BACKTICK {
                let left = value;
                let mut text: Vec<u16> = Vec::new();
                if has_right_quote(self.input, self.index, left) {
                    if self.keep_quotes {
                        text.push(left);
                    }
                    loop {
                        if self.index > length {
                            break;
                        }
                        let next = match self.advance() {
                            Some(c) => c,
                            None => break,
                        };
                        if next == CH_BACKSLASH {
                            text.push(next);
                            match self.advance() {
                                Some(c) => text.push(c),
                                None => text.extend(undefined_units()),
                            }
                            continue;
                        }
                        if next == left {
                            if self.keep_quotes {
                                text.push(next);
                            }
                            break;
                        }
                        text.push(next);
                    }
                } else {
                    // Unpaired quote: consumed as a regular character.
                    text.push(left);
                }
                self.push_text(text);
                continue;
            }

            // '{'
            if value == CH_LBRACE {
                self.depth += 1;
                let prev_dollar = {
                    let p = self.ast.node(self.prev);
                    matches!(&p.value, Some(v) if v.last() == Some(&CH_DOLLAR))
                };
                let dollar = prev_dollar || self.ast.node(self.block).dollar;
                let mut brace = Node::new(NodeType::Brace);
                brace.open = true;
                brace.close = false;
                brace.dollar = dollar;
                brace.depth = self.depth;
                brace.commas = 0;
                brace.ranges = 0;
                let id = self.push(brace).expect("brace never merges");
                self.block = id;
                self.stack.push(id);
                let mut open = Node::new(NodeType::Open);
                open.value = Some(vec![value]);
                self.push(open);
                continue;
            }

            // '}'
            if value == CH_RBRACE {
                if self.ast.node(self.block).ty != NodeType::Brace {
                    self.push_text(vec![value]);
                    continue;
                }
                self.block = self.stack.pop().expect("brace on stack");
                self.ast.node_mut(self.block).close = true;
                let mut close = Node::new(NodeType::Close);
                close.value = Some(vec![value]);
                self.push(close);
                self.depth -= 1;
                self.block = *self.stack.last().expect("stack nonempty");
                continue;
            }

            // ','
            if value == CH_COMMA && self.depth > 0 {
                if self.ast.node(self.block).ranges > 0 {
                    self.ast.node_mut(self.block).ranges = 0;
                    let open_id = self.ast.node_mut(self.block).children.remove(0);
                    let s = stringify_subtree(&self.ast.nodes, self.block, false);
                    let text_id = self.ast.alloc(Node::text(s));
                    self.ast.node_mut(self.block).children = vec![open_id, text_id];
                }
                self.ast.node_mut(self.block).invalid = Some(false);
                let mut comma = Node::new(NodeType::Comma);
                comma.value = Some(vec![value]);
                self.push(comma);
                self.ast.node_mut(self.block).commas += 1;
                continue;
            }

            // '.'
            if value == CH_DOT && self.depth > 0 && self.ast.node(self.block).commas == 0 {
                let siblings_len = self.ast.node(self.block).children.len();
                if self.depth == 0 || siblings_len == 0 {
                    self.push_text(vec![value]);
                    continue;
                }
                if self.ast.node(self.prev).ty == NodeType::Dot {
                    self.ast.node_mut(self.prev).value.as_mut().expect("dot value").push(value);
                    self.ast.node_mut(self.prev).ty = NodeType::Range;
                    if siblings_len != 3 && siblings_len != 5 {
                        self.ast.node_mut(self.block).invalid = Some(true);
                        self.ast.node_mut(self.block).ranges = 0;
                        self.ast.node_mut(self.prev).ty = NodeType::Text;
                        continue;
                    }
                    self.ast.node_mut(self.block).ranges += 1;
                    continue;
                }
                if self.ast.node(self.prev).ty == NodeType::Range {
                    let popped = self
                        .ast
                        .node_mut(self.block)
                        .children
                        .pop()
                        .expect("range node is last child");
                    let before = *self
                        .ast
                        .node(self.block)
                        .children
                        .last()
                        .expect("sibling before range");
                    let pv = self
                        .ast
                        .node(popped)
                        .value
                        .clone()
                        .unwrap_or_default();
                    let bv = self.ast.node_mut(before).value.as_mut().expect("text value");
                    bv.extend_from_slice(&pv);
                    bv.push(value);
                    self.prev = before;
                    self.ast.node_mut(self.block).ranges -= 1;
                    continue;
                }
                let mut dot = Node::new(NodeType::Dot);
                dot.value = Some(vec![value]);
                self.push(dot);
                continue;
            }

            // Plain text.
            self.push_text(vec![value]);
        }

        // Mark imbalanced braces and brackets as invalid: splice unclosed
        // blocks into their parents, leaf children become invalid text.
        loop {
            let b = self.stack.pop().expect("stack nonempty");
            if self.ast.node(b).ty != NodeType::Root {
                let children = self.ast.node(b).children.clone();
                for child in children {
                    let cn = self.ast.node_mut(child);
                    if !cn.has_nodes() {
                        if cn.ty == NodeType::Open {
                            cn.is_open = true;
                        }
                        if cn.ty == NodeType::Close {
                            cn.is_close = true;
                        }
                        cn.ty = NodeType::Text;
                        cn.invalid = Some(true);
                    }
                }
                let parent = *self.stack.last().expect("parent on stack");
                let idx = self
                    .ast
                    .node(parent)
                    .children
                    .iter()
                    .position(|&x| x == b)
                    .expect("block is a child of parent");
                let bchildren = std::mem::take(&mut self.ast.node_mut(b).children);
                self.ast
                    .node_mut(parent)
                    .children
                    .splice(idx..idx + 1, bchildren);
            }
            self.block = b;
            if self.stack.is_empty() {
                break;
            }
        }

        self.push(Node::new(NodeType::Eos));
    }
}

/// Port of the reference's quote lookahead:
/// `input.slice(index).search(/[^\\]q|^q/) !== -1`.
fn has_right_quote(input: &[u16], from: usize, q: u16) -> bool {
    if from < input.len() && input[from] == q {
        return true;
    }
    let mut i = from + 1;
    while i < input.len() {
        if input[i] == q && input[i - 1] != CH_BACKSLASH {
            return true;
        }
        i += 1;
    }
    false
}

/// Parse a pattern (UTF-16 code units) into an AST, port of `lib/parse.js`.
pub fn parse(input: &[u16], opts: &ParseOpts) -> Result<Ast, BracesError> {
    let max = match opts.max_length {
        Some(m) => MAX_LENGTH.min(m),
        None => MAX_LENGTH,
    };
    if (input.len() as f64) > max {
        return Err(BracesError::MaxLength {
            len: input.len(),
            max: jsnum::num_to_string(max)
                .iter()
                .map(|&c| c as u8 as char)
                .collect::<String>()
                .parse::<usize>()
                .unwrap_or(0),
        });
    }

    let mut ast = Ast {
        nodes: Vec::with_capacity(input.len() / 2 + 4),
        root: 0,
        input: input.to_vec(),
    };
    let root_id = ast.alloc(Node::new(NodeType::Root));
    ast.root = root_id;

    let mut parser = Parser {
        input,
        ast,
        stack: vec![root_id],
        block: root_id,
        prev: root_id,
        brackets: 0,
        index: 0,
        depth: 0,
        keep_escaping: opts.keep_escaping,
        keep_quotes: opts.keep_quotes,
    };
    parser.run();
    Ok(parser.ast)
}
