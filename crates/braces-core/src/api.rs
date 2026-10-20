//! Top-level API mirroring `micromatch/braces` `index.js`:
//! `braces` (default fn), `expand`, `compile`, `create`, `stringify`, `parse`.
//!
//! All text enters as Rust `&str` (or, for AST round-trips, an already-parsed
//! `Ast`) and is processed as UTF-16 code units internally, so JS string
//! semantics (length, charCodeAt, surrogate pairs) hold exactly.

use crate::ast::{Ast, NodeId, NodeType};
use crate::compile;
use crate::error::BracesError;
use crate::expand;
pub use crate::expand::Rope;
use crate::fill::{fill, FillOptions, FillOut};
use crate::jsnum;
use crate::options::Options;
use crate::parse;
use crate::parse::MAX_LENGTH;
use crate::stringify::{stringify_ast as stringify_nodes, stringify_subtree};
use crate::utils::{exceeds_limit, reduce, zeroish};
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

/// A bare range pattern `{X..Y}` / `{X..Y..S}` (integer endpoints with raw
/// sign/padding preserved, or single-letter endpoints). For such patterns
/// parse+walk reduces exactly to `exceedsLimit` + `fill-range` on the raw
/// endpoint strings, so the fast path calls those directly.
fn simple_range(units: &[u16]) -> Option<(Vec<u16>, Vec<u16>, Option<Vec<u16>>)> {
    let n = units.len();
    if n < 6 || units[0] != b'{' as u16 || units[n - 1] != b'}' as u16 {
        return None;
    }
    let inner = &units[1..n - 1];
    let find_dd = |s: &[u16]| -> Option<usize> {
        if s.len() < 2 {
            return None;
        }
        (0..s.len() - 1).find(|&i| s[i] == b'.' as u16 && s[i + 1] == b'.' as u16)
    };
    let i = find_dd(inner)?;
    let a = &inner[..i];
    let rest = &inner[i + 2..];
    let (b, c) = match find_dd(rest) {
        Some(j) => (&rest[..j], Some(rest[j + 2..].to_vec())),
        None => (rest, None),
    };
    let is_int = |s: &[u16]| -> bool {
        let s = if !s.is_empty() && (s[0] == b'+' as u16 || s[0] == b'-' as u16) {
            &s[1..]
        } else {
            s
        };
        !s.is_empty() && s.iter().all(|&c| (b'0' as u16..=b'9' as u16).contains(&c))
    };
    let is_alpha = |s: &[u16]| -> bool {
        s.len() == 1 && (s[0] as u8).is_ascii_alphabetic()
    };
    if !((is_int(a) && is_int(b)) || (is_alpha(a) && is_alpha(b))) {
        return None;
    }
    if let Some(ref step) = c {
        if !is_int(step) {
            return None;
        }
    }
    Some((a.to_vec(), b.to_vec(), c))
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
    Ok(expand_str_ropes(input, opts)?
        .iter()
        .map(|r| r.to_units())
        .collect())
}

/// Expand to output **ropes** (segment trees) without materialising —
/// the napi layer packs results straight from the ropes, so multi-output
/// expansions never pay a per-string allocation + copy.
/// Internal expand result: the two zero-waste fast paths (plain
/// literal, simple-range fill) produce plain item vectors; only the
/// general walk produces ropes.
enum ExpandOut {
    Items(Vec<Vec<u16>>),
    Ropes(Vec<Rope>),
}

fn expand_str_out(input: &str, opts: &Options) -> Result<ExpandOut, BracesError> {
    let units = to_units(input);
    if is_plain_literal(&units) && opts.transform.is_none() {
        check_max_length(&units, opts)?;
        if units.is_empty() {
            return Ok(ExpandOut::Items(apply_post_units(Vec::new(), opts)));
        }
        return Ok(ExpandOut::Items(apply_post_units(vec![units], opts)));
    }
    if let Some((s, e, st)) = simple_range(&units) {
        check_max_length(&units, opts)?;
        let mut args: Vec<Vec<u16>> = vec![s.clone(), e.clone()];
        if let Some(step) = &st {
            args.push(step.clone());
        }
        if crate::utils::exceeds_limit(&args, opts.step_num(), opts.range_limit) {
            return Err(BracesError::RangeLimit);
        }
        let fopts = crate::fill::FillOptions::from_options(opts, false);
        return match crate::fill::fill(&s, Some(&e), st.as_deref(), &fopts)? {
            crate::fill::FillOut::List(items) if !items.is_empty() => {
                Ok(ExpandOut::Items(apply_post_units(items, opts)))
            }
            // Reference fallback: an empty fill stringifies the brace
            // verbatim (simple-range chars need no re-escaping).
            crate::fill::FillOut::List(_) => {
                Ok(ExpandOut::Items(apply_post_units(vec![units], opts)))
            }
            crate::fill::FillOut::Regex(_) => unreachable!(),
        };
    }
    let mut ast = parse_units(&units, opts)?;
    crate::ast::check_depth(&ast.nodes, ast.root)?;
    if let Some(flat) = flat_segments(&mut ast, opts) {
        let segments = flat?;
        return Ok(ExpandOut::Items(apply_post_units(
            cartesian_items(&segments),
            opts,
        )));
    }
    Ok(ExpandOut::Ropes(expand_ast_ropes(&mut ast, opts)?))
}

/// Pack plain items into the napi wire format: `[count: u32][cumulative
/// offsets: u32 * count][data]`, all as UTF-16 units in one buffer.
fn pack_items(items: &[Vec<u16>]) -> Vec<u16> {
    let total: usize = items.iter().map(|v| v.len()).sum();
    let mut buf: Vec<u16> = Vec::with_capacity(2 + items.len() * 2 + total);
    let c = items.len() as u32;
    buf.push((c & 0xFFFF) as u16);
    buf.push((c >> 16) as u16);
    let mut pos: u32 = 0;
    for it in items {
        pos += it.len() as u32;
        buf.push((pos & 0xFFFF) as u16);
        buf.push((pos >> 16) as u16);
    }
    for it in items {
        buf.extend_from_slice(it);
    }
    buf
}

// ---------------------------------------------------------------------------
// Flat-cartesian fast path.
//
// A pattern qualifies as flat when its root children are only value leaves
// (text / dots / bos / eos) and simple braces — braces that the general
// walk would expand in isolation to a plain item list, i.e. valid
// range braces (handled by the identical `reduce` + `exceeds_limit` + `fill`
// calls the walk uses) and valid list braces (comma-split groups of plain
// values, computed with the walk's exact comma/empty-item semantics).
// Anything else (nesting, parens, invalid/dollar-with-content quirks not
// covered below) falls back to the general walk.
//
// The general walk is exactly a left-to-right cartesian product of the
// per-segment expansions (each root-level stage pops the whole accumulated
// queue and appends the next segment), so computing the cartesian product
// directly yields byte-identical results — with zero rope-node allocation,
// zero RefCell traffic, and a single sequential write into the wire buffer.
//
// `None` = not a flat pattern (caller uses the general walk). Errors are
// `Some(Err)` and must propagate exactly like the walk's.

/// Items of one top-level brace node in isolation (mirrors the walk's
/// per-node branches: invalid/dollar, `{}`, ranges, then the list dance).
fn flat_brace_items(
    ast: &mut Ast,
    node: NodeId,
    opts: &Options,
) -> Option<Result<Vec<Vec<u16>>, BracesError>> {
    let (invalid, dollar, ty, nchildren, ranges, commas) = {
        let n = ast.node(node);
        (n.invalid, n.dollar, n.ty, n.children.len(), n.ranges, n.commas)
    };
    // invalid / dollar: the walk appends the stringified subtree as one item.
    if invalid == Some(true) || dollar {
        // `stringify_subtree` recurses — keep the CVE depth guard for
        // this branch only (bail to the general path, which raises the
        // typed error). Every other flat branch is structurally shallow:
        // the list dance bails on any child with nodes, and the range
        // branch recurses nowhere except its stringify fallback, which
        // is guarded the same way below.
        if crate::ast::tree_depth(&ast.nodes, node) > crate::ast::MAX_AST_DEPTH {
            return None;
        }
        return Some(Ok(vec![stringify_subtree(
            &ast.nodes,
            node,
            opts.escape_invalid,
        )]));
    }
    // `{}`: the walk's literal-brace special case.
    if ty == NodeType::Brace && invalid != Some(true) && nchildren == 2 {
        return Some(Ok(vec![vec![b'{' as u16, b'}' as u16]]));
    }
    // Ranges: identical calls to the walk's branch (`reduce` mutates the
    // range nodes exactly like the walk relies on).
    if ranges > 0 {
        let args = reduce(ast, node);
        if exceeds_limit(&args, opts.step_num(), opts.range_limit) {
            return Some(Err(BracesError::RangeLimit));
        }
        let fopts = FillOptions::from_options(opts, false);
        let out = match fill(
            args.first().map(|v| v.as_slice()).unwrap_or(&[]),
            args.get(1).map(|v| v.as_slice()),
            args.get(2).map(|v| v.as_slice()),
            &fopts,
        ) {
            Ok(o) => o,
            Err(e) => return Some(Err(e)),
        };
        return Some(Ok(match out {
            FillOut::List(items) if !items.is_empty() => items,
            _ => {
                // Stringify fallback recurses over the subtree — same
                // depth guard as the invalid branch.
                if crate::ast::tree_depth(&ast.nodes, node) > crate::ast::MAX_AST_DEPTH {
                    return None;
                }
                vec![stringify_subtree(&ast.nodes, node, opts.escape_invalid)]
            }
        }));
    }
    // List dance, at string level: the walk's queue dance with an empty
    // incoming queue reduces to comma-split groups whose values are the
    // plain concatenation of the group's text values, preserving the
    // exact empty-item semantics (`{,a}` -> ["", "a"], `{a,,b}` ->
    // ["a", "", "b"], `{,}` -> ["", ""]).
    // `enclose` mirrors the walk's `enclose_brace` on this node
    // (`zeroish` — without the mutation, which no consumer of a flat AST
    // can observe since the walk is not run afterwards).
    let enclose = zeroish(commas, ranges);
    let children = ast.node(node).children.clone();
    let mut groups: Vec<Vec<u16>> = vec![Vec::new()];
    let mut saw_close = false;
    for child in children {
        let cn = ast.node(child);
        match cn.ty {
            NodeType::Open => {}
            NodeType::Comma => groups.push(Vec::new()),
            NodeType::Close => {
                saw_close = true;
                break;
            }
            _ => {
                // A child the walk would recurse into is not flat.
                if cn.has_nodes() {
                    return None;
                }
                if let Some(v) = &cn.value {
                    groups.last_mut().unwrap().extend_from_slice(v);
                }
            }
        }
    }
    if !saw_close {
        return None;
    }
    if enclose {
        for it in &mut groups {
            let mut w = Vec::with_capacity(it.len() + 2);
            w.push(b'{' as u16);
            w.extend_from_slice(it);
            w.push(b'}' as u16);
            *it = w;
        }
    }
    Some(Ok(groups))
}

/// Per-segment item lists for the whole pattern: one `Vec<Vec<u16>>`
/// (items) per top-level segment.
fn flat_segments(
    ast: &mut Ast,
    opts: &Options,
) -> Option<Result<Vec<Vec<Vec<u16>>>, BracesError>> {
    let root = ast.root;
    let children = ast.node(root).children.clone();
    if children.is_empty() {
        return None;
    }
    let mut segments: Vec<Vec<Vec<u16>>> = Vec::with_capacity(children.len());
    for child in children {
        let (ty, has_nodes, value) = {
            let n = ast.node(child);
            (n.ty, n.has_nodes(), n.value.clone())
        };
        match ty {
            NodeType::Brace => match flat_brace_items(ast, child, opts) {
                Some(Ok(items)) => segments.push(items),
                Some(Err(e)) => return Some(Err(e)),
                None => return None,
            },
            NodeType::Open | NodeType::Close => return None,
            _ if has_nodes => return None,
            // A leaf the walk would append as a plain value (text, dot,
            // bos, eos, ...); an absent value is an empty factor.
            _ => segments.push(vec![value.unwrap_or_default()]),
        }
    }
    if segments.is_empty() {
        return None;
    }
    // Coalesce single-item segments into their neighbours: a one-item
    // segment contributes the same bytes to every output, so folding it
    // into the adjacent multi-item segment's items (prepend to the next
    // segment, append to the previous at the tail) is byte-identical and
    // cuts the per-output segment visits in the cartesian writers.
    let mut merged: Vec<Vec<Vec<u16>>> = Vec::with_capacity(segments.len());
    let mut pending: Vec<u16> = Vec::new();
    for seg in segments {
        if seg.len() == 1 {
            pending.extend_from_slice(&seg[0]);
            continue;
        }
        let mut seg = seg;
        if !pending.is_empty() {
            for item in &mut seg {
                let mut w = Vec::with_capacity(pending.len() + item.len());
                w.extend_from_slice(&pending);
                w.extend_from_slice(item);
                *item = w;
            }
            pending = Vec::new();
        }
        merged.push(seg);
    }
    if !pending.is_empty() {
        match merged.last_mut() {
            Some(last) => {
                for item in last.iter_mut() {
                    item.extend_from_slice(&pending);
                }
            }
            None => merged.push(vec![pending]),
        }
    }
    Some(Ok(merged))
}

/// Materialised cartesian product (string level), for the `Vec<String>`
/// APIs. The general walk is a left-to-right cartesian product, so the
/// last segment varies fastest — identical order.
fn cartesian_items(segments: &[Vec<Vec<u16>>]) -> Vec<Vec<u16>> {
    let mut result: Vec<Vec<u16>> = vec![Vec::new()];
    for seg in segments {
        let mut next = Vec::with_capacity(result.len().saturating_mul(seg.len()));
        for prefix in &result {
            for item in seg {
                let mut v = Vec::with_capacity(prefix.len() + item.len());
                v.extend_from_slice(prefix);
                v.extend_from_slice(item);
                next.push(v);
            }
        }
        result = next;
    }
    result
}

/// Write the cartesian product into the wire format by level-by-level
/// expansion: outputs of the first k segments are held as one staging
/// buffer plus cumulative end offsets, then extended by the next
/// segment's items. Prefix bytes are copied once per level in bulk
/// instead of once per output per segment, and the final level's end
/// offsets are exactly the wire header's cumulative offsets — no
/// separate counting pass. `nodupes` falls back to the materialised
/// path (rare, opt-in); `noempty` filters zero-length outputs while
/// assembling the final buffer.
fn flat_wire(segments: &[Vec<Vec<u16>>], opts: &Options) -> Option<Vec<u16>> {
    let mut count: usize = 1;
    for seg in segments {
        if seg.is_empty() {
            return Some(pack_items(&[]));
        }
        count = count.checked_mul(seg.len())?;
    }
    let _ = count;
    if opts.nodupes {
        return Some(pack_items(&apply_post_units(cartesian_items(segments), opts)));
    }
    // Level 0: the first segment's items, concatenated.
    let mut data: Vec<u16> = Vec::new();
    let mut ends: Vec<u32> = Vec::with_capacity(segments[0].len());
    for item in &segments[0] {
        data.extend_from_slice(item);
        ends.push(data.len() as u32);
    }
    // Levels 1..: extend every current output by every next item.
    for seg in &segments[1..] {
        let seg_total: usize = seg.iter().map(|i| i.len()).sum();
        let cur_count = ends.len();
        let mut next: Vec<u16> =
            Vec::with_capacity(data.len() * seg.len() + seg_total * cur_count);
        let mut next_ends: Vec<u32> = Vec::with_capacity(cur_count * seg.len());
        let mut start = 0usize;
        for &end in &ends {
            let out = &data[start..end as usize];
            start = end as usize;
            for item in seg {
                next.extend_from_slice(out);
                next.extend_from_slice(item);
                next_ends.push(next.len() as u32);
            }
        }
        data = next;
        ends = next_ends;
    }
    // Assemble: header (count + cumulative ends) + data. `noempty`
    // drops zero-length outputs (their bytes are nothing, so the data
    // section is copied output-by-output only in that case).
    let total = data.len();
    if opts.noempty {
        let kept: Vec<u32> = {
            let mut k = Vec::with_capacity(ends.len());
            let mut prev = 0u32;
            for &e in &ends {
                if e > prev {
                    k.push(e - prev);
                }
                prev = e;
            }
            k
        };
        let kept_total: usize = kept.iter().map(|&l| l as usize).sum();
        let mut buf: Vec<u16> = Vec::with_capacity(2 + kept.len() * 2 + kept_total);
        let n = kept.len() as u32;
        buf.push((n & 0xFFFF) as u16);
        buf.push((n >> 16) as u16);
        let mut pos: u32 = 0;
        for &l in &kept {
            pos += l;
            buf.push((pos & 0xFFFF) as u16);
            buf.push((pos >> 16) as u16);
        }
        let mut prev = 0usize;
        let mut src = 0usize;
        for &e in &ends {
            let end = e as usize;
            if end > prev {
                buf.extend_from_slice(&data[src..end]);
            }
            src = end;
            prev = end;
        }
        let _ = total;
        return Some(buf);
    }
    let mut buf: Vec<u16> = Vec::with_capacity(2 + ends.len() * 2 + total);
    let n = ends.len() as u32;
    buf.push((n & 0xFFFF) as u16);
    buf.push((n >> 16) as u16);
    for &e in &ends {
        buf.push((e & 0xFFFF) as u16);
        buf.push((e >> 16) as u16);
    }
    buf.extend_from_slice(&data);
    Some(buf)
}

/// Try the flat-cartesian path on already-decoded input units. Parses once;
/// `None` means "not flat" (caller falls back). No separate depth pass:
/// flat shapes are structurally shallow (see `flat_brace_items`), and the
/// two recursive stringify branches carry their own CVE depth guard.
fn try_flat_wire(units: &[u16], opts: &Options) -> Option<Result<Vec<u16>, BracesError>> {
    let mut ast = match parse::parse(units, &opts.for_parse()) {
        Ok(a) => a,
        Err(e) => return Some(Err(e)),
    };
    let segments = match flat_segments(&mut ast, opts) {
        Some(Ok(s)) => s,
        Some(Err(e)) => return Some(Err(e)),
        None => return None,
    };
    Some(Ok(flat_wire(&segments, opts)?))
}

pub fn expand_str_ropes(input: &str, opts: &Options) -> Result<Vec<Rope>, BracesError> {
    Ok(match expand_str_out(input, opts)? {
        ExpandOut::Items(items) => items.into_iter().map(Rope::from_units).collect(),
        ExpandOut::Ropes(ropes) => ropes,
    })
}

/// Expand straight into the packed wire format used by the napi layer:
/// `[count: u32][cumulative offsets: u32 * count][data]`, all as UTF-16
/// units in one buffer. The fast paths pack their items directly — no
/// rope objects are built for fill results at all.
pub fn expand_wire(input: &str, opts: &Options) -> Result<Vec<u16>, BracesError> {
    // Flat-cartesian interception: only when the two zero-parse fast
    // paths do not apply (they stay strictly faster for their shapes).
    let units = to_units(input);
    if !is_plain_literal(&units) && simple_range(&units).is_none() {
        if let Some(w) = try_flat_wire(&units, opts) {
            return w;
        }
    }
    Ok(match expand_str_out(input, opts)? {
        ExpandOut::Items(items) => pack_items(&items),
        ExpandOut::Ropes(ropes) => {
            let total: usize = ropes.iter().map(|r| r.unit_len()).sum();
            let c = ropes.len() as u32;
            let mut buf: Vec<u16> = Vec::with_capacity(2 + ropes.len() * 2 + total);
            buf.push((c & 0xFFFF) as u16);
            buf.push((c >> 16) as u16);
            let mut pos: u32 = 0;
            for r in &ropes {
                pos += r.unit_len() as u32;
                buf.push((pos & 0xFFFF) as u16);
                buf.push((pos >> 16) as u16);
            }
            Rope::write_many(&ropes, &mut buf);
            buf
        }
    })
}

fn apply_post_ropes(mut items: Vec<Rope>, opts: &Options) -> Vec<Rope> {
    if opts.noempty {
        items.retain(|r| r.unit_len() > 0);
    }
    if opts.nodupes {
        let mut seen = HashSet::with_capacity(items.len());
        items.retain(|r| seen.insert(r.to_units()));
    }
    items
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
    Ok(expand_ast_ropes(ast, opts)?
        .iter()
        .map(|r| r.to_units())
        .collect())
}

/// Expand an AST to ropes (post-processing applied, no materialisation).
pub fn expand_ast_ropes(ast: &mut Ast, opts: &Options) -> Result<Vec<Rope>, BracesError> {
    let items = expand::expand(ast, opts)?;
    Ok(apply_post_ropes(items, opts))
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
    if let Some((s, e, st)) = simple_range(&units) {
        // Mirror compile's walk range branch: fill with toRegex options,
        // wrap in parens when the regex is longer than one unit.
        check_max_length(&units, opts)?;
        let fopts = crate::fill::FillOptions::from_compile_options(opts);
        if let crate::fill::FillOut::Regex(range) =
            crate::fill::fill(&s, Some(&e), st.as_deref(), &fopts)?
        {
            if !range.is_empty() {
                if range.len() > 1 {
                    let mut out = Vec::with_capacity(range.len() + 2);
                    out.push(b'(' as u16);
                    out.extend_from_slice(&range);
                    out.push(b')' as u16);
                    return Ok(out);
                }
                return Ok(range);
            }
        }
        // Empty regex: the walk falls through to the children, which for a
        // bare range brace stringify back to the pattern itself.
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
    crate::ast::check_depth(&ast.nodes, ast.root)?;
    Ok(from_units(&stringify_nodes(
        &ast.nodes,
        ast.root,
        opts.escape_invalid,
    )))
}

pub fn stringify_ast(ast: &Ast, opts: &Options) -> Result<String, BracesError> {
    crate::ast::check_depth(&ast.nodes, ast.root)?;
    Ok(from_units(&stringify_nodes(
        &ast.nodes,
        ast.root,
        opts.escape_invalid,
    )))
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
        Input::Str(s) => {
            // create_units() would route expand:true straight to
            // expand_units (past its `< 3` short-circuit) — skip the clone
            // and the duplicate scans on this hot path.
            if opts.expand && !s.is_empty() && s.encode_utf16().count() >= 3 {
                output = expand_units(s, opts)?;
            } else {
                match create_units(&Value::Str(s.clone()), opts)? {
                    CreateUnits::Expanded(v) => output.extend(v),
                    CreateUnits::Compiled(u) => output.push(u),
                }
            }
        }
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
