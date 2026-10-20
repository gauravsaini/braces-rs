//! Top-level API mirroring `micromatch/braces` `index.js`:
//! `braces` (default fn), `expand`, `compile`, `create`, `stringify`, `parse`.
//!
//! All text enters as Rust `&str` (or, for AST round-trips, an already-parsed
//! `Ast`) and is processed as UTF-16 code units internally, so JS string
//! semantics (length, charCodeAt, surrogate pairs) hold exactly.

use crate::ast::Ast;
use crate::compile;
use crate::error::BracesError;
use crate::expand;
use crate::jsnum;
use crate::options::Options;
use crate::parse;
use crate::parse::MAX_LENGTH;
use crate::stringify::stringify_ast as stringify_nodes;
use std::collections::HashSet;

pub use crate::options::{RangeLimit, Transform};

fn to_units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// A pattern with no `{`, `}`, `\`, `]`, NBSP or ZWNBSP parses to verbatim
/// text (those are the only characters `parse` transforms in text): the
/// walk reduces to the input itself, so we skip parse+walk entirely.
/// `transform` is gated off for expand (it only fires inside range fills);
/// `quantifiers` for compile (it only affects brace compilation).
fn is_plain_literal(units: &[u16]) -> bool {
    units.iter().all(|&c| {
        c != b'{' as u16
            && c != b'}' as u16
            && c != b'\\' as u16
            && c != b']' as u16
            && c != 0x00A0
            && c != 0xFEFF
    })
}

/// The same maxLength check `parse` performs, for the fast paths.
fn check_max_length(units: &[u16], opts: &Options) -> Result<(), BracesError> {
    let max = match opts.max_length {
        Some(m) => MAX_LENGTH.min(m),
        None => MAX_LENGTH,
    };
    if (units.len() as f64) > max {
        return Err(BracesError::MaxLength {
            len: units.len(),
            max: jsnum::num_to_string(max)
                .iter()
                .map(|&c| c as u8 as char)
                .collect::<String>()
                .parse::<usize>()
                .unwrap_or(0),
        });
    }
    Ok(())
}

pub fn from_units(v: &[u16]) -> String {
    String::from_utf16(v).expect("parser only produces valid UTF-16")
}

/// The `braces()` input: a single pattern or an array of patterns.
#[derive(Clone, Debug)]
pub enum Input {
    Str(String),
    Arr(Vec<String>),
}

/// Non-string inputs, for modelling the reference's V8-typed error paths
/// (expand/compile on null/undefined/number/boolean/object/array throw
/// engine-specific TypeErrors — documented divergences, see DIVERGENCES.md).
#[derive(Clone, Debug)]
pub enum Value {
    Str(String),
    Arr(Vec<String>),
    Null,
    Undefined,
    Num(f64),
    Bool(bool),
    Obj,
}

/// `create()` result: an expanded array, or a single compiled string.
#[derive(Clone, Debug)]
pub enum CreateResult {
    Expanded(Vec<String>),
    Compiled(String),
}

fn parse_units(input: &[u16], opts: &Options) -> Result<Ast, BracesError> {
    parse::parse(input, &opts.for_parse())
}

/// `braces.parse(input, options)` — throws `TypeError: Expected a string`
/// for non-string input (byte-identical to the reference).
pub fn parse_str(input: &str, opts: &Options) -> Result<Ast, BracesError> {
    parse_units(&to_units(input), opts)
}

pub fn parse_value(input: &Value, opts: &Options) -> Result<Ast, BracesError> {
    match input {
        Value::Str(s) => parse_str(s, opts),
        _ => Err(BracesError::ExpectedString),
    }
}

fn apply_post(mut items: Vec<String>, opts: &Options) -> Vec<String> {
    if opts.noempty {
        items.retain(|s| !s.is_empty());
    }
    if opts.nodupes {
        let mut seen = HashSet::with_capacity(items.len());
        items.retain(|s| seen.insert(s.clone()));
    }
    items
}

/// `braces.expand(input, options)` — `input` is a pattern string or an AST.
pub fn expand_str(input: &str, opts: &Options) -> Result<Vec<String>, BracesError> {
    Ok(expand_units(input, opts)?
        .iter()
        .map(|v| from_units(v))
        .collect())
}

/// Same, but returns UTF-16 units (lets the napi layer skip a transcode).
pub fn expand_units(input: &str, opts: &Options) -> Result<Vec<Vec<u16>>, BracesError> {
    let units = to_units(input);
    if is_plain_literal(&units) && opts.transform.is_none() {
        check_max_length(&units, opts)?;
        if units.is_empty() {
            return Ok(apply_post_units(Vec::new(), opts));
        }
        return Ok(apply_post_units(vec![units], opts));
    }
    let mut ast = parse_units(&units, opts)?;
    expand_ast_units(&mut ast, opts)
}

fn apply_post_units(mut items: Vec<Vec<u16>>, opts: &Options) -> Vec<Vec<u16>> {
    if opts.noempty {
        items.retain(|s| !s.is_empty());
    }
    if opts.nodupes {
        let mut seen = HashSet::with_capacity(items.len());
        items.retain(|s| seen.insert(s.clone()));
    }
    items
}

pub fn expand_ast(ast: &mut Ast, opts: &Options) -> Result<Vec<String>, BracesError> {
    let items = expand_ast_units(ast, opts)?;
    Ok(items.iter().map(|v| from_units(v)).collect())
}

pub fn expand_ast_units(ast: &mut Ast, opts: &Options) -> Result<Vec<Vec<u16>>, BracesError> {
    let items = expand::expand(ast, opts)?;
    Ok(apply_post_units(items, opts))
}

/// `braces.compile(input, options)` — returns the regex-source *string*
/// (the JSDoc claiming an array is wrong; FINDINGS.md §4).
pub fn compile_str(input: &str, opts: &Options) -> Result<String, BracesError> {
    Ok(from_units(&compile_units(input, opts)?))
}

/// Same, but returns UTF-16 units (lets the napi layer skip a transcode).
pub fn compile_units(input: &str, opts: &Options) -> Result<Vec<u16>, BracesError> {
    let units = to_units(input);
    if is_plain_literal(&units) {
        check_max_length(&units, opts)?;
        return Ok(units);
    }
    let mut ast = parse_units(&units, opts)?;
    compile::compile(&mut ast, opts)
}

pub fn compile_ast(ast: &mut Ast, opts: &Options) -> Result<String, BracesError> {
    let out = compile::compile(ast, opts)?;
    Ok(from_units(&out))
}

/// `braces.stringify(input, options)` — `input` is a pattern string or AST.
pub fn stringify_str(input: &str, opts: &Options) -> Result<String, BracesError> {
    let ast = parse_str(input, opts)?;
    Ok(from_units(&stringify_nodes(
        &ast.nodes,
        ast.root,
        opts.escape_invalid,
    )))
}

pub fn stringify_ast(ast: &Ast, opts: &Options) -> String {
    from_units(&stringify_nodes(
        &ast.nodes,
        ast.root,
        opts.escape_invalid,
    ))
}

/// `create()` result in UTF-16 units (napi path).
#[derive(Clone, Debug)]
pub enum CreateUnits {
    Expanded(Vec<Vec<u16>>),
    Compiled(Vec<u16>),
}

/// `braces.create(input, options)` in UTF-16 units.
pub fn create_units(input: &Value, opts: &Options) -> Result<CreateUnits, BracesError> {
    match input {
        Value::Str(s) => {
            let units = to_units(s);
            if s.is_empty() || units.len() < 3 {
                return Ok(CreateUnits::Expanded(vec![units]));
            }
            if opts.expand {
                Ok(CreateUnits::Expanded(expand_units(s, opts)?))
            } else {
                Ok(CreateUnits::Compiled(compile_units(s, opts)?))
            }
        }
        // JS: `input.length` on null/undefined throws a V8 TypeError —
        // documented divergence.
        Value::Null => Err(BracesError::NonStringInput(
            "create: cannot read 'length' of null",
        )),
        Value::Undefined => Err(BracesError::NonStringInput(
            "create: cannot read 'length' of undefined",
        )),
        // JS: `.length` is undefined for numbers/booleans/objects/arrays, so
        // the short-circuit is skipped; `compile()` walks the value and
        // returns '' (verified against the reference); `expand()` throws
        // setting `.queue` on primitives.
        Value::Num(_) | Value::Bool(_) | Value::Obj | Value::Arr(_) => {
            if opts.expand {
                expand_value(input, opts)
                    .map(|v| CreateUnits::Expanded(v.iter().map(|s| to_units(s)).collect()))
            } else {
                Ok(CreateUnits::Compiled(Vec::new()))
            }
        }
    }
}

/// `braces.create(input, options)`.
pub fn create(input: &Value, opts: &Options) -> Result<CreateResult, BracesError> {
    Ok(match create_units(input, opts)? {
        CreateUnits::Expanded(v) => {
            CreateResult::Expanded(v.iter().map(|u| from_units(u)).collect())
        }
        CreateUnits::Compiled(u) => CreateResult::Compiled(from_units(&u)),
    })
}

/// The default `braces(input, options)` function, in UTF-16 units.
pub fn braces_units(input: &Input, opts: &Options) -> Result<Vec<Vec<u16>>, BracesError> {
    let mut output: Vec<Vec<u16>> = Vec::new();
    match input {
        Input::Arr(patterns) => {
            for p in patterns {
                match create_units(&Value::Str(p.clone()), opts)? {
                    CreateUnits::Expanded(mut v) => output.append(&mut v),
                    CreateUnits::Compiled(u) => output.push(u),
                }
            }
        }
        Input::Str(s) => match create_units(&Value::Str(s.clone()), opts)? {
            CreateUnits::Expanded(v) => output.extend(v),
            CreateUnits::Compiled(u) => output.push(u),
        },
    }
    if opts.expand && opts.nodupes {
        let mut seen = HashSet::with_capacity(output.len());
        output.retain(|s| seen.insert(s.clone()));
    }
    Ok(output)
}

/// The default `braces(input, options)` function.
pub fn braces(input: &Input, opts: &Options) -> Result<Vec<String>, BracesError> {
    Ok(braces_units(input, opts)?
        .iter()
        .map(|u| from_units(u))
        .collect())
}

/// `braces.expand` on a non-string value: the JS reference skips `parse`
/// and throws an engine TypeError while walking the value as an AST.
/// We keep the TypeError name with a descriptive message (documented
/// divergence).
pub fn expand_value(input: &Value, opts: &Options) -> Result<Vec<String>, BracesError> {
    match input {
        Value::Str(s) => expand_str(s, opts),
        Value::Null => Err(BracesError::NonStringInput("walk: cannot set 'queue' on null")),
        Value::Undefined => Err(BracesError::NonStringInput(
            "walk: cannot set 'queue' on undefined",
        )),
        Value::Num(_) => Err(BracesError::NonStringInput(
            "walk: cannot create property 'queue' on a number",
        )),
        Value::Bool(_) => Err(BracesError::NonStringInput(
            "walk: cannot create property 'queue' on a boolean",
        )),
        Value::Obj | Value::Arr(_) => Err(BracesError::NonStringInput(
            "walk: cannot read 'length' of undefined",
        )),
    }
}

/// `braces.compile` on a non-string value: the JS reference walks the value
/// directly — null/undefined throw reading `.invalid` (documented
/// divergence), every other value compiles to the empty string.
pub fn compile_value(input: &Value, opts: &Options) -> Result<String, BracesError> {
    match input {
        Value::Str(s) => compile_str(s, opts),
        Value::Null => Err(BracesError::NonStringInput(
            "walk: cannot read 'invalid' of null",
        )),
        Value::Undefined => Err(BracesError::NonStringInput(
            "walk: cannot read 'invalid' of undefined",
        )),
        _ => Ok(String::new()),
    }
}

/// Batch expansion (LEARNINGS.md #1): amortises the napi crossing over many
/// small patterns. Parses + expands each pattern with a single options set.
pub fn expand_many(patterns: &[String], opts: &Options) -> Result<Vec<Vec<String>>, BracesError> {
    patterns.iter().map(|p| expand_str(p, opts)).collect()
}

/// Same, in UTF-16 units.
pub fn expand_many_units(
    patterns: &[String],
    opts: &Options,
) -> Result<Vec<Vec<Vec<u16>>>, BracesError> {
    patterns.iter().map(|p| expand_units(p, opts)).collect()
}
