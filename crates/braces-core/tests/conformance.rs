//! Conformance gate: run the 2008-fixture corpus generated from the JS
//! reference (micromatch/braces v3.0.3) through braces-core and require
//! 100% pass. Error fixtures compare name + message; the V8-specific
//! non-string-input TypeErrors compare the error *name* only (documented
//! divergences in DIVERGENCES.md).
//!
//! Corpus path: BRACE_RS_CONFORMANCE env var, else the vendored copy at
//! tests/conformance.json (byte-identical to the lane-a output).

use braces_core::api::{self, CreateResult, Input, Value};
use braces_core::ast::{Ast, NodeType};
use braces_core::{BracesError, Options, RangeLimit, Transform};
use serde_json::Value as Json;
use std::collections::HashMap;

fn corpus() -> Vec<Json> {
    let path = std::env::var("BRACE_RS_CONFORMANCE").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/conformance.json").to_string()
    });
    let raw = std::fs::read_to_string(&path).expect("read conformance corpus");
    serde_json::from_str(&raw).expect("parse conformance corpus")
}

fn opts_from(j: &Json) -> Options {
    let mut o = Options::default();
    let get = |k: &str| j.get(k);
    if let Some(b) = get("expand").and_then(|v| v.as_bool()) {
        o.expand = b;
    }
    if let Some(b) = get("nodupes").and_then(|v| v.as_bool()) {
        o.nodupes = b;
    }
    if let Some(b) = get("noempty").and_then(|v| v.as_bool()) {
        o.noempty = b;
    }
    if let Some(v) = get("rangeLimit") {
        match v {
            Json::Bool(false) => o.range_limit = RangeLimit::Disabled,
            Json::Number(n) => o.range_limit = RangeLimit::Num(n.as_f64().unwrap()),
            _ => {}
        }
    }
    if let Some(n) = get("maxLength").and_then(|v| v.as_f64()) {
        o.max_length = Some(n);
    }
    if let Some(b) = get("keepEscaping").and_then(|v| v.as_bool()) {
        o.keep_escaping = b;
    }
    if let Some(b) = get("keepQuotes").and_then(|v| v.as_bool()) {
        o.keep_quotes = b;
    }
    if let Some(b) = get("escapeInvalid").and_then(|v| v.as_bool()) {
        o.escape_invalid = b;
    }
    if let Some(n) = get("step").and_then(|v| v.as_f64()) {
        o.step = Some(braces_core::options::StepVal::Num(n));
    } else if let Some(s) = get("step").and_then(|v| v.as_str()) {
        o.step = Some(braces_core::options::StepVal::Str(
            s.encode_utf16().collect(),
        ));
    }
    if let Some(b) = get("capture").and_then(|v| v.as_bool()) {
        o.capture = Some(b);
    }
    if let Some(b) = get("wrap").and_then(|v| v.as_bool()) {
        o.wrap = Some(b);
    }
    if let Some(b) = get("shorthand").and_then(|v| v.as_bool()) {
        o.shorthand = Some(b);
    }
    if let Some(b) = get("strictZeros").and_then(|v| v.as_bool()) {
        o.strict_zeros = Some(b);
    }
    if let Some(b) = get("relaxZeros").and_then(|v| v.as_bool()) {
        o.relax_zeros = Some(b);
    }
    if let Some(t) = get("transform").and_then(|v| v.as_str()) {
        o.transform = Some(match t {
            "double" => Transform::Double,
            "alphaFoo" => Transform::AlphaFoo,
            "prefixN" => Transform::PrefixN,
            other => panic!("unknown transform tag: {other}"),
        });
    }
    o
}

/// The sanitized AST shape the corpus generator records.
fn ast_json(ast: &Ast) -> Json {
    fn node_json(ast: &Ast, id: usize) -> Json {
        let n = ast.node(id);
        let mut o = serde_json::Map::new();
        o.insert("type".into(), Json::String(n.ty.as_str().to_string()));
        if let Some(v) = &n.value {
            o.insert(
                "value".into(),
                Json::String(String::from_utf16(v).unwrap()),
            );
        }
        if n.ty == NodeType::Brace {
            o.insert("dollar".into(), Json::Bool(n.dollar));
            o.insert("depth".into(), Json::Number(n.depth.into()));
            o.insert("commas".into(), Json::Number(n.commas.into()));
            o.insert("ranges".into(), Json::Number(n.ranges.into()));
            if let Some(inv) = n.invalid {
                o.insert("invalid".into(), Json::Bool(inv));
            }
            o.insert("open".into(), Json::Bool(n.open));
            o.insert("close".into(), Json::Bool(n.close));
        } else {
            if let Some(inv) = n.invalid {
                o.insert("invalid".into(), Json::Bool(inv));
            }
            if n.is_open {
                o.insert("isOpen".into(), Json::Bool(true));
            }
            if n.is_close {
                o.insert("isClose".into(), Json::Bool(true));
            }
        }
        if n.has_nodes() {
            let kids: Vec<Json> = n.children.iter().map(|&c| node_json(ast, c)).collect();
            o.insert("nodes".into(), Json::Array(kids));
        }
        Json::Object(o)
    }
    node_json(ast, ast.root)
}

/// Reconstruct non-string inputs from their `String(pattern)` encoding.
fn value_from_encoded(s: &str) -> Value {
    match s {
        "null" => Value::Null,
        "undefined" => Value::Undefined,
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "[object Object]" => Value::Obj,
        "x" => Value::Arr(vec!["x".to_string()]),
        _ => {
            if let Ok(n) = s.parse::<f64>() {
                Value::Num(n)
            } else {
                Value::Str(s.to_string())
            }
        }
    }
}

fn strs(v: &[String]) -> Json {
    Json::Array(v.iter().map(|s| Json::String(s.clone())).collect())
}

#[derive(Default)]
struct Report {
    pass: usize,
    fail: usize,
    failures: Vec<String>,
}

fn is_v8_error(msg: &str) -> bool {
    msg.contains("Cannot create property")
        || msg.contains("Cannot set properties")
        || msg.contains("Cannot read properties")
        || msg.contains("input.padStart is not a function")
}

#[test]
fn conformance() {
    let recs = corpus();
    assert_eq!(recs.len(), 2008, "corpus fixture count");
    let mut rep = Report::default();
    let mut empty_default_seen = 0usize;
    let _seen_keys: HashMap<String, usize> = HashMap::new();

    let mut check_err = |rep: &mut Report, idx: usize, rec: &Json, got: Result<(), BracesError>| {
        let expected = rec["error"].as_str().unwrap();
        match got {
            Ok(()) => {
                rep.fail += 1;
                rep.failures
                    .push(format!("#{idx} expected error {expected:?}, got success"));
            }
            Err(e) => {
                let mine = e.to_js_string();
                let (exp_name, _) = expected.split_once(": ").unwrap();
                if mine == expected {
                    rep.pass += 1;
                } else if is_v8_error(expected) && e.name() == exp_name {
                    // Documented divergence: same JS error name, V8 message differs.
                    rep.pass += 1;
                } else {
                    rep.fail += 1;
                    rep.failures.push(format!(
                        "#{idx} error mismatch: expected {expected:?}, got {mine:?}"
                    ));
                }
            }
        }
    };

    let mut check_val = |rep: &mut Report, idx: usize, rec: &Json, got: Json| {
        let expected = &rec["result"];
        if got == *expected {
            rep.pass += 1;
        } else {
            // Truncated fixtures: compare length + head.
            if let Some(tr) = expected.as_object() {
                if tr.get("__truncated").and_then(|v| v.as_bool()) == Some(true) {
                    let exp_len = tr["length"].as_u64().unwrap() as usize;
                    let head = tr["head"].clone();
                    let (my_len, my_head) = match &got {
                        Json::Array(a) => (
                            a.len(),
                            Json::Array(a.iter().take(20).cloned().collect()),
                        ),
                        Json::String(s) => (
                            s.encode_utf16().count(),
                            Json::String(s.chars().take(200).collect()),
                        ),
                        _ => (0, Json::Null),
                    };
                    if my_len == exp_len && my_head == head {
                        rep.pass += 1;
                        return;
                    }
                }
            }
            // Numeric-result fixtures (e.g. `.length` recorded): compare length.
            if let Some(n) = expected.as_u64() {
                if let Json::Array(a) = &got {
                    if a.len() as u64 == n {
                        rep.pass += 1;
                        return;
                    }
                }
            }
            rep.fail += 1;
            rep.failures.push(format!(
                "#{idx} value mismatch:\n  fn={} pattern={:?} options={}\n  expected: {}\n  got:      {}",
                rec["fn"].as_str().unwrap(),
                rec["pattern"].as_str().unwrap_or(""),
                rec["options"],
                serde_json::to_string(expected).unwrap(),
                serde_json::to_string(&got).unwrap(),
            ));
        }
    };

    for (idx, rec) in recs.iter().enumerate() {
        let fnname = rec["fn"].as_str().unwrap();
        let pattern = rec["pattern"].as_str().unwrap().to_string();
        let opts = opts_from(&rec["options"]);
        let has_error = rec.get("error").is_some();

        match fnname {
            "expand" => {
                let v = value_from_encoded(&pattern);
                let r = api::expand_value(&v, &opts).map(|out| check_val(&mut rep, idx, rec, strs(&out)));
                if has_error {
                    check_err(&mut rep, idx, rec, r.map(|_| ()));
                } else {
                    match r {
                        Ok(()) => {}
                        Err(e) => {
                            rep.fail += 1;
                            rep.failures.push(format!("#{idx} expand {pattern:?}: unexpected error {e}"));
                        }
                    }
                }
            }
            "compile" => {
                let v = value_from_encoded(&pattern);
                let r = api::compile_value(&v, &opts)
                    .map(|out| check_val(&mut rep, idx, rec, Json::String(out)));
                if has_error {
                    check_err(&mut rep, idx, rec, r.map(|_| ()));
                } else if let Err(e) = r {
                    rep.fail += 1;
                    rep.failures.push(format!("#{idx} compile {pattern:?}: unexpected error {e}"));
                }
            }
            "default" => {
                // Reconstruct the array-input fixtures (gen-conformance.js
                // records arrays via String(input)).
                let input = match pattern.as_str() {
                    "{a,b},{1..3}" => Input::Arr(vec!["{a,b}".into(), "{1..3}".into()]),
                    "{a,a},b{a,a}" => Input::Arr(vec!["{a,a}".into(), "b{a,a}".into()]),
                    "" => {
                        empty_default_seen += 1;
                        if empty_default_seen > 1 {
                            Input::Arr(vec![])
                        } else {
                            Input::Str(String::new())
                        }
                    }
                    _ => Input::Str(pattern.clone()),
                };
                let r = api::braces(&input, &opts)
                    .map(|out| check_val(&mut rep, idx, rec, strs(&out)));
                if has_error {
                    check_err(&mut rep, idx, rec, r.map(|_| ()));
                } else if let Err(e) = r {
                    rep.fail += 1;
                    rep.failures.push(format!("#{idx} default {pattern:?}: unexpected error {e}"));
                }
            }
            "create" => {
                // The generator's create list was
                // ['', 'a', 'ab', 'abc', '{a,b}', 'a{b,c}d', '{1..5}', 0, null]:
                // the "0" entry is the NUMBER 0 and "null" is null.
                let v = match pattern.as_str() {
                    "null" => Value::Null,
                    "0" => Value::Num(0.0),
                    _ => Value::Str(pattern.clone()),
                };
                let r = api::create(&v, &opts).map(|out| {
                    check_val(
                        &mut rep,
                        idx,
                        rec,
                        match out {
                            CreateResult::Expanded(v) => strs(&v),
                            CreateResult::Compiled(s) => Json::String(s),
                        },
                    )
                });
                if has_error {
                    check_err(&mut rep, idx, rec, r.map(|_| ()));
                } else if let Err(e) = r {
                    rep.fail += 1;
                    rep.failures.push(format!("#{idx} create {pattern:?}: unexpected error {e}"));
                }
            }
            "stringify" => {
                let r = api::stringify_str(&pattern, &opts)
                    .map(|out| check_val(&mut rep, idx, rec, Json::String(out)));
                if has_error {
                    check_err(&mut rep, idx, rec, r.map(|_| ()));
                } else if let Err(e) = r {
                    rep.fail += 1;
                    rep.failures.push(format!("#{idx} stringify {pattern:?}: unexpected error {e}"));
                }
            }
            "parse" => {
                let v = value_from_encoded(&pattern);
                let r = api::parse_value(&v, &opts).map(|ast| {
                    check_val(&mut rep, idx, rec, ast_json(&ast));
                });
                if has_error {
                    check_err(&mut rep, idx, rec, r.map(|_| ()));
                } else if let Err(e) = r {
                    rep.fail += 1;
                    rep.failures.push(format!("#{idx} parse {pattern:?}: unexpected error {e}"));
                }
            }
            "expand_ast" | "compile_ast" | "stringify_ast" => {
                let parsed = api::parse_str(&pattern, &Options::default());
                match fnname {
                    "expand_ast" => {
                        let r = parsed.and_then(|mut ast| api::expand_ast(&mut ast, &opts))
                            .map(|out| check_val(&mut rep, idx, rec, strs(&out)));
                        if has_error {
                            check_err(&mut rep, idx, rec, r.map(|_| ()));
                        } else if let Err(e) = r {
                            rep.fail += 1;
                            rep.failures.push(format!("#{idx} expand_ast {pattern:?}: {e}"));
                        }
                    }
                    "compile_ast" => {
                        let r = parsed.and_then(|mut ast| api::compile_ast(&mut ast, &opts))
                            .map(|out| check_val(&mut rep, idx, rec, Json::String(out)));
                        if has_error {
                            check_err(&mut rep, idx, rec, r.map(|_| ()));
                        } else if let Err(e) = r {
                            rep.fail += 1;
                            rep.failures.push(format!("#{idx} compile_ast {pattern:?}: {e}"));
                        }
                    }
                    _ => {
                        let r = parsed
                            .map(|ast| api::stringify_ast(&ast, &opts))
                            .map(|out| check_val(&mut rep, idx, rec, Json::String(out)));
                        if has_error {
                            check_err(&mut rep, idx, rec, r.map(|_| ()));
                        } else if let Err(e) = r {
                            rep.fail += 1;
                            rep.failures.push(format!("#{idx} stringify_ast {pattern:?}: {e}"));
                        }
                    }
                }
            }
            other => {
                rep.fail += 1;
                rep.failures.push(format!("#{idx} unknown fn {other}"));
            }
        }
    }

    eprintln!(
        "conformance: {} pass, {} fail, {} total",
        rep.pass,
        rep.fail,
        rep.pass + rep.fail
    );
    let show = 40;
    for f in rep.failures.iter().take(show) {
        eprintln!("FAIL {f}");
    }
    if rep.failures.len() > show {
        eprintln!("... and {} more failures", rep.failures.len() - show);
    }
    assert_eq!(rep.fail, 0, "conformance failures: {}", rep.fail);
    assert_eq!(rep.pass, 2008, "all 2008 fixtures must pass");
}
