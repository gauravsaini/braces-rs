//! Regression tests for CVE-2026-93687 (uncontrolled recursion on
//! deeply nested brace patterns; the JS reference dies with
//! `RangeError: Maximum call stack size exceeded` at a chain depth of
//! ~4,999 on stock V8).
//!
//! Guard contract (see `ast::MAX_AST_DEPTH`): ASTs deeper than 10,000 get
//! a typed `RangeError` ("Maximum call stack size exceeded") from every
//! recursive walker — the process can never die with a native stack
//! overflow. Because the (reference-faithful) 10,000-char input cap means
//! no string input can express a depth above ~5,000, the guard never
//! rejects an input the JS reference handles.

use braces_core::ast::{Ast, Node, NodeType};
use braces_core::{api, Options};

/// A brace chain with exactly `n` braces: "{{{...a...}}}" (2n+1 chars).
fn chain_exact(n: usize) -> String {
    format!("{}a{}", "{".repeat(n), "}".repeat(n))
}

/// A structurally faithful synthetic deep AST: root -> [bos, brace_1, eos];
/// brace_i children = [open, inner, close] with inner = brace_{i+1} or a
/// text leaf — mirrors what the parser produces for a brace chain.
fn deep_ast(n_braces: usize) -> Ast {
    fn leaf(nodes: &mut Vec<Node>, ty: NodeType, v: u8) -> usize {
        let mut n = Node::text(vec![v as u16]);
        n.ty = ty;
        nodes.push(n);
        nodes.len() - 1
    }
    let mut nodes: Vec<Node> = vec![Node::new(NodeType::Root)];
    let bos = {
        nodes.push(Node::new(NodeType::Bos));
        nodes.len() - 1
    };
    let eos = {
        nodes.push(Node::new(NodeType::Eos));
        nodes.len() - 1
    };
    let braces: Vec<usize> = (0..n_braces)
        .map(|_| {
            nodes.push(Node::new(NodeType::Brace));
            nodes.len() - 1
        })
        .collect();
    let text = leaf(&mut nodes, NodeType::Text, b'a');
    for i in (0..n_braces).rev() {
        let b = braces[i];
        let open = leaf(&mut nodes, NodeType::Open, b'{');
        let close = leaf(&mut nodes, NodeType::Close, b'}');
        let inner = if i + 1 < n_braces { braces[i + 1] } else { text };
        nodes[b].children = vec![open, inner, close];
        nodes[open].parent = Some(b);
        nodes[inner].parent = Some(b);
        nodes[close].parent = Some(b);
    }
    nodes[0].children = vec![bos, braces[0], eos];
    nodes[bos].parent = Some(0);
    nodes[braces[0]].parent = Some(0);
    nodes[eos].parent = Some(0);
    Ast {
        nodes,
        root: 0,
        input: Vec::new(),
    }
}

/// Deep-recursion cases run on a 64MB stack thread: they emulate the
/// production (Node main thread, ~8MB) budget instead of the smaller
/// cargo-test worker stacks.
fn on_big_stack<F: FnOnce() + Send + 'static>(f: F) {
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join");
}

#[test]
fn cve_chain_4999_does_not_crash() {
    // The CVE reproducer shape at the reviewer's verified depth: the JS
    // reference throws RangeError here; we must complete without crashing.
    let p = chain_exact(4999);
    assert_eq!(p.len(), 9999);
    on_big_stack(move || {
        let opts = Options::default();
        let out = api::expand_str(&p, &opts).expect("expand chain 4999 ok");
        // Reference behavior (byte-identical at every depth): nested
        // single-child braces are left unexpanded.
        assert_eq!(out, vec![p.as_str()]);
        assert!(api::compile_str(&p, &opts).is_ok(), "compile chain ok");
        assert!(api::stringify_str(&p, &opts).is_ok(), "stringify chain ok");
        assert!(api::parse_str(&p, &opts).is_ok(), "parse chain ok");
    });
}

#[test]
fn cve_depth_sweep_no_false_positives() {
    // The guard must not fire anywhere the JS reference survives.
    for d in [10usize, 100, 500, 1000, 2000, 4999] {
        let p = chain_exact(d);
        if d * 2 + 1 > 10_000 {
            continue;
        }
        on_big_stack(move || {
            let opts = Options::default();
            assert!(api::expand_str(&p, &opts).is_ok(), "expand depth {d}");
            assert!(api::compile_str(&p, &opts).is_ok(), "compile depth {d}");
            assert!(api::stringify_str(&p, &opts).is_ok(), "stringify depth {d}");
        });
    }
}

#[test]
fn cve_length_cap_parity() {
    // 6,000 braces = 12,001 chars > the 10,000 cap: both implementations
    // throw SyntaxError from the max-length guard (the reference min()s
    // maxLength against its own cap too).
    let p = chain_exact(6000);
    let opts = Options::default();
    for (name, r) in [
        ("expand", api::expand_str(&p, &opts).map(|_| ())),
        ("compile", api::compile_str(&p, &opts).map(|_| ())),
        ("stringify", api::stringify_str(&p, &opts).map(|_| ())),
        ("parse", api::parse_str(&p, &opts).map(|_| ())),
    ] {
        let e = r.expect_err("over cap errors");
        assert_eq!(e.name(), "SyntaxError", "{name} class");
    }
}

#[test]
fn cve_guard_fires_past_limit() {
    // Programmatic ASTs (the only way past the length cap): depth
    // 20,002 must surface RangeError, never crash the process.
    let opts = Options::default();
    let mut ast = deep_ast(20_000);
    let r = api::expand_ast(&mut ast, &opts).map(|_| ());
    let e = r.expect_err("expand over depth limit errors");
    assert_eq!(e.name(), "RangeError");
    assert_eq!(e.message(), "Maximum call stack size exceeded");

    let mut ast = deep_ast(20_000);
    let r = api::compile_ast(&mut ast, &opts).map(|_| ());
    let e = r.expect_err("compile over depth limit errors");
    assert_eq!(e.name(), "RangeError");
    assert_eq!(e.message(), "Maximum call stack size exceeded");

    let ast = deep_ast(20_000);
    let r = api::stringify_ast(&ast, &opts).map(|_| ());
    let e = r.expect_err("stringify over depth limit errors");
    assert_eq!(e.name(), "RangeError");
    assert_eq!(e.message(), "Maximum call stack size exceeded");
}

#[test]
fn cve_guard_boundary() {
    // depth = n_braces + 2; 10,000 is allowed, 10,001 is not. (Also
    // proves the walkers themselves survive ~10k-deep recursion.)
    let opts = Options::default();
    on_big_stack(move || {
        let opts = Options::default();
        let mut ast_ok = deep_ast(9_998);
        assert!(api::expand_ast(&mut ast_ok, &opts).is_ok(), "depth 10000 ok");
        let mut ast_ok = deep_ast(9_998);
        assert!(
            api::compile_ast(&mut ast_ok, &opts).is_ok(),
            "compile depth 10000 ok"
        );
        let ast_ok = deep_ast(9_998);
        assert!(
            api::stringify_ast(&ast_ok, &opts).is_ok(),
            "stringify depth 10000 ok"
        );
    });

    let mut ast_bad = deep_ast(9_999);
    assert_eq!(
        api::expand_ast(&mut ast_bad, &opts).unwrap_err().name(),
        "RangeError",
        "depth 10001 errors"
    );
    let mut ast_bad = deep_ast(9_999);
    assert_eq!(
        api::compile_ast(&mut ast_bad, &opts).unwrap_err().name(),
        "RangeError",
        "compile depth 10001 errors"
    );
    let ast_bad = deep_ast(9_999);
    assert_eq!(
        api::stringify_ast(&ast_bad, &opts).unwrap_err().name(),
        "RangeError",
        "stringify depth 10001 errors"
    );
}
