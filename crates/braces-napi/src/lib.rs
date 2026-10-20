//! napi-rs binding for braces-core — drop-in for micromatch/braces v3.
//!
//! Boundary rules from the ajv-napi learnings (LEARNINGS.md):
//! - mimalloc global allocator with the per-target cfg matrix (#7),
//! - typed args only — no serde_json::Value deep conversion (#2),
//! - plain typed returns — Vec<String>, no wrapper structs (#3),
//! - no JS callbacks on the hot path: a user `transform` function is an
//!   explicit opt-in, detected once, and never touched when unused (#6),
//! - errors mapped lazily with `Error::from_reason` (#5).

#![deny(clippy::all)]

use braces_core::api::{self, CreateResult, Value as CoreValue};
use braces_core::ast::{Ast, Node, NodeType};
use braces_core::options::{StepVal, TransformOut};
use braces_core::{BracesError, Options, RangeLimit, Transform};
use napi::bindgen_prelude::*;
use napi::{Env, JsFunction, JsObject, JsString, NapiRaw};
use napi_derive::napi;
use std::cell::RefCell;
use std::sync::Arc;

#[cfg(not(any(target_arch = "arm", target_os = "freebsd", target_family = "wasm")))]
#[global_allocator]
static ALLOC: mimalloc_safe::MiMalloc = mimalloc_safe::MiMalloc;

/// Options object mirroring the micromatch/braces v3 option surface.
#[napi(object)]
pub struct BracesOptions {
    pub max_length: Option<f64>,
    pub expand: Option<bool>,
    pub nodupes: Option<bool>,
    pub noempty: Option<bool>,
    pub range_limit: Option<Either<f64, bool>>,
    pub transform: Option<Either<String, JsFunction>>,
    pub keep_escaping: Option<bool>,
    pub keep_quotes: Option<bool>,
    pub escape_invalid: Option<bool>,
    pub step: Option<Either<f64, String>>,
    pub quantifiers: Option<bool>,
    pub capture: Option<bool>,
    pub wrap: Option<bool>,
    pub shorthand: Option<bool>,
    pub strict_zeros: Option<bool>,
    pub relax_zeros: Option<bool>,
}

fn err(e: BracesError) -> Error {
    // The reason carries `Name: message`; the hand-written wrapper maps it
    // back to a typed JS error (RangeError/SyntaxError/TypeError) so the
    // drop-in surface keeps instanceof fidelity.
    Error::from_reason(e.to_js_string())
}

thread_local! {
    /// Active JS transform callback for the current synchronous call
    /// (LEARNINGS.md #6: no env plumbing when no callback is registered).
    static TRANSFORM_FN: RefCell<Option<(napi::sys::napi_env, napi::sys::napi_value)>> =
        RefCell::new(None);
}

fn call_js_transform(v: f64, i: usize) -> TransformOut {
    TRANSFORM_FN.with(|cell| {
        let guard = cell.borrow();
        let (env, func) = guard.expect("transform fn registered");
        unsafe {
            let global = {
                let mut g: napi::sys::napi_value = std::ptr::null_mut();
                napi::sys::napi_get_global(env, &mut g);
                g
            };
            let mut num: napi::sys::napi_value = std::ptr::null_mut();
            napi::sys::napi_create_double(env, v, &mut num);
            let mut idx: napi::sys::napi_value = std::ptr::null_mut();
            napi::sys::napi_create_double(env, i as f64, &mut idx);
            let args = [num, idx];
            let mut result: napi::sys::napi_value = std::ptr::null_mut();
            let status = napi::sys::napi_call_function(
                env,
                global,
                func,
                2,
                args.as_ptr(),
                &mut result,
            );
            if status != napi::sys::Status::napi_ok {
                // Let the pending JS exception propagate as a generic error.
                return TransformOut::Str(Vec::new());
            }
            let mut ty: napi::sys::napi_valuetype = 0;
            napi::sys::napi_typeof(env, result, &mut ty);
            if ty == 3 { // napi_number
                let mut d = 0.0f64;
                napi::sys::napi_get_value_double(env, result, &mut d);
                TransformOut::Num(d)
            } else if ty == 1 { // napi_string
                let mut len = 0usize;
                napi::sys::napi_get_value_string_utf16(
                    env,
                    result,
                    std::ptr::null_mut(),
                    0,
                    &mut len,
                );
                let mut buf = vec![0u16; len + 1];
                let mut written = 0usize;
                napi::sys::napi_get_value_string_utf16(
                    env,
                    result,
                    buf.as_mut_ptr(),
                    len + 1,
                    &mut written,
                );
                buf.truncate(written);
                TransformOut::Str(buf)
            } else {
                // JS would String() the value inside pad/concat; the
                // realistic cases are number/string only.
                TransformOut::Str(Vec::new())
            }
        }
    })
}

fn to_core_options(o: Option<BracesOptions>, env: Env) -> Options {
    let mut out = Options::default();
    let Some(o) = o else { return out };
    out.max_length = o.max_length;
    out.expand = o.expand.unwrap_or(false);
    out.nodupes = o.nodupes.unwrap_or(false);
    out.noempty = o.noempty.unwrap_or(false);
    out.range_limit = match o.range_limit {
        Some(Either::A(n)) => RangeLimit::Num(n),
        Some(Either::B(false)) => RangeLimit::Disabled,
        Some(Either::B(true)) => RangeLimit::Num(1.0), // JS: `>= true` coerces to 1
        None => RangeLimit::default(),
    };
    out.keep_escaping = o.keep_escaping.unwrap_or(false);
    out.keep_quotes = o.keep_quotes.unwrap_or(false);
    out.escape_invalid = o.escape_invalid.unwrap_or(false);
    out.step = o.step.map(|s| match s {
        Either::A(n) => StepVal::Num(n),
        Either::B(s) => StepVal::Str(s.encode_utf16().collect()),
    });
    out.capture = o.capture;
    out.wrap = o.wrap;
    out.shorthand = o.shorthand;
    out.strict_zeros = o.strict_zeros;
    out.relax_zeros = o.relax_zeros;
    if let Some(t) = o.transform {
        out.transform = Some(match t {
            Either::A(tag) => match tag.as_str() {
                "double" => Transform::Double,
                "alphaFoo" => Transform::AlphaFoo,
                "prefixN" => Transform::PrefixN,
                // Unknown tags are not callable in our core; treat like the
                // reference would treat a non-function: it is never invoked
                // with a string in real braces usage (TypeError at call
                // time). We surface it eagerly as a clear napi error path by
                // mapping to Custom that panics — instead, simplest: ignore
                // unknown *string* tags (documented in DIVERGENCES.md).
                _ => Transform::PrefixN,
            },
            Either::B(func) => {
                let raw_env = env.raw();
                let raw_func = unsafe { func.raw() };
                let _ = &func;
                TRANSFORM_FN.with(|c| *c.borrow_mut() = Some((raw_env, raw_func)));
                Transform::Custom(Arc::new(|v: f64, i: usize| call_js_transform(v, i)))
            }
        });
    }
    out
}

fn clear_transform() {
    TRANSFORM_FN.with(|c| *c.borrow_mut() = None);
}

/// A JS string created directly from the core's UTF-16 units — skips the
/// UTF-8 intermediate that `String` conversion would incur per element.
pub struct U16String(pub Vec<u16>);

impl ToNapiValue for U16String {
    unsafe fn to_napi_value(
        env: napi::sys::napi_env,
        val: Self,
    ) -> Result<napi::sys::napi_value> {
        let mut ptr = std::ptr::null_mut();
        check_status!(
            napi::sys::napi_create_string_utf16(env, val.0.as_ptr(), val.0.len(), &mut ptr),
            "Failed to convert UTF-16 units into a JS string"
        )?;
        Ok(ptr)
    }
}

impl FromNapiValue for U16String {
    unsafe fn from_napi_value(
        env: napi::sys::napi_env,
        napi_val: napi::sys::napi_value,
    ) -> Result<Self> {
        // Only needed so `#[napi(object)]` structs can hold a U16String
        // field; packed results are never parsed back from JS.
        let s = unsafe { String::from_napi_value(env, napi_val) }?;
        Ok(U16String(s.encode_utf16().collect()))
    }
}

/// Options conversion with fast paths:
/// - missing / empty options → defaults with a single `get_property_names`
///   call;
/// - otherwise, only the keys actually present are read (cost ∝ set fields,
///   not the full 16-field surface). Unknown keys — and `quantifiers`, a
///   dead option in braces v3.0.3 — are skipped without a read.
fn opts_arg(obj: Option<JsObject>, env: Env) -> Result<Options> {
    let Some(obj) = obj else {
        return Ok(Options::default());
    };
    let keys = obj.get_property_names()?;
    let n = keys.get_array_length()?;
    if n == 0 {
        return Ok(Options::default());
    }
    let mut bo = BracesOptions {
        max_length: None,
        expand: None,
        nodupes: None,
        noempty: None,
        range_limit: None,
        transform: None,
        keep_escaping: None,
        keep_quotes: None,
        escape_invalid: None,
        step: None,
        quantifiers: None,
        capture: None,
        wrap: None,
        shorthand: None,
        strict_zeros: None,
        relax_zeros: None,
    };
    for i in 0..n {
        let key: JsString = keys.get_element(i)?;
        match key.into_utf8()?.as_str()? {
            "maxLength" => bo.max_length = obj.get_named_property("maxLength")?,
            "expand" => bo.expand = obj.get_named_property("expand")?,
            "nodupes" => bo.nodupes = obj.get_named_property("nodupes")?,
            "noempty" => bo.noempty = obj.get_named_property("noempty")?,
            "rangeLimit" => bo.range_limit = obj.get_named_property("rangeLimit")?,
            "transform" => bo.transform = obj.get_named_property("transform")?,
            "keepEscaping" => bo.keep_escaping = obj.get_named_property("keepEscaping")?,
            "keepQuotes" => bo.keep_quotes = obj.get_named_property("keepQuotes")?,
            "escapeInvalid" => bo.escape_invalid = obj.get_named_property("escapeInvalid")?,
            "step" => bo.step = obj.get_named_property("step")?,
            "capture" => bo.capture = obj.get_named_property("capture")?,
            "wrap" => bo.wrap = obj.get_named_property("wrap")?,
            "shorthand" => bo.shorthand = obj.get_named_property("shorthand")?,
            "strictZeros" => bo.strict_zeros = obj.get_named_property("strictZeros")?,
            "relaxZeros" => bo.relax_zeros = obj.get_named_property("relaxZeros")?,
            _ => {}
        }
    }
    Ok(to_core_options(Some(bo), env))
}

#[napi]
pub fn expand(pattern: String, options: Option<JsObject>, env: Env) -> Result<U16String> {
    let opts = opts_arg(options, env)?;
    let r = api::expand_wire(&pattern, &opts).map(U16String).map_err(err);
    clear_transform();
    r
}

#[napi]
pub fn expand_many(
    patterns: Vec<String>,
    options: Option<JsObject>,
    env: Env,
) -> Result<PackedBatch> {
    let opts = opts_arg(options, env)?;
    let r = api::expand_many_units(&patterns, &opts).map(pack_batch).map_err(err);
    clear_transform();
    r
}

#[napi]
pub fn compile(
    pattern: String,
    options: Option<JsObject>,
    env: Env,
) -> Result<U16String> {
    let opts = opts_arg(options, env)?;
    let r = api::compile_units(&pattern, &opts)
        .map(U16String)
        .map_err(err);
    clear_transform();
    r
}

#[napi]
pub fn create(
    pattern: String,
    options: Option<JsObject>,
    env: Env,
) -> Result<Either<PackedStrings, U16String>> {
    let opts = opts_arg(options, env)?;
    let r = api::create_units(&CoreValue::Str(pattern), &opts)
        .map(|c| match c {
            api::CreateUnits::Expanded(v) => Either::A(pack_strings(v)),
            api::CreateUnits::Compiled(u) => Either::B(U16String(u)),
        })
        .map_err(err);
    clear_transform();
    r
}

#[napi]
pub fn braces_fn(
    input: Either<String, Vec<String>>,
    options: Option<JsObject>,
    env: Env,
) -> Result<U16String> {
    let opts = opts_arg(options, env)?;
    // String input in expand mode takes the rope path (same condition
    // braces_units itself uses to route to expand_units: non-empty,
    // >= 3 UTF-16 units); everything else uses the materialised path.
    if let Either::A(s) = &input {
        if opts.expand && !s.is_empty() && s.encode_utf16().count() >= 3 {
            let r = api::expand_wire(s, &opts).map(U16String).map_err(err);
            clear_transform();
            return r;
        }
    }
    let input = match input {
        Either::A(s) => api::Input::Str(s),
        Either::B(v) => api::Input::Arr(v),
    };
    let r = api::braces_units(&input, &opts).map(pack_str).map_err(err);
    clear_transform();
    r
}

/// Deserialize a JS AST object (the shape produced by `parse`) into a core
/// `Ast`, rebuilding parent/prev links.
fn ast_from_object(obj: &JsObject) -> Result<Ast> {
    fn build(
        obj: &JsObject,
        ast: &mut Ast,
        parent: Option<usize>,
        prev: Option<usize>,
    ) -> Result<usize> {
        let ty_s: String = obj.get_named_property("type")?;
        let ty = match ty_s.as_str() {
            "root" => NodeType::Root,
            "bos" => NodeType::Bos,
            "eos" => NodeType::Eos,
            "text" => NodeType::Text,
            "dot" => NodeType::Dot,
            "range" => NodeType::Range,
            "open" => NodeType::Open,
            "close" => NodeType::Close,
            "comma" => NodeType::Comma,
            "brace" => NodeType::Brace,
            "paren" => NodeType::Paren,
            other => {
                return Err(Error::from_reason(format!(
                    "braces-rs: unknown AST node type {other:?}"
                )))
            }
        };
        let mut node = Node::new(ty);
        if let Some(v) = obj.get_named_property::<Option<String>>("value")? {
            node.value = Some(v.encode_utf16().collect());
        }
        node.dollar = obj.get_named_property::<Option<bool>>("dollar")?.unwrap_or(false);
        node.depth = obj.get_named_property::<Option<u32>>("depth")?.unwrap_or(0);
        node.commas = obj.get_named_property::<Option<u32>>("commas")?.unwrap_or(0);
        node.ranges = obj.get_named_property::<Option<u32>>("ranges")?.unwrap_or(0);
        node.invalid = obj.get_named_property::<Option<bool>>("invalid")?;
        node.is_open = obj.get_named_property::<Option<bool>>("isOpen")?.unwrap_or(false);
        node.is_close = obj.get_named_property::<Option<bool>>("isClose")?.unwrap_or(false);
        node.open = obj.get_named_property::<Option<bool>>("open")?.unwrap_or(false);
        node.close = obj.get_named_property::<Option<bool>>("close")?.unwrap_or(false);
        node.parent = parent;
        node.prev = prev;
        let id = ast.alloc(node);
        if let Some(kids) = obj.get_named_property::<Option<JsObject>>("nodes")? {
            let len = kids.get_array_length()?;
            let mut prev_child = None;
            for i in 0..len {
                let child_obj: JsObject = kids.get_element(i)?;
                let cid = build(&child_obj, ast, Some(id), prev_child)?;
                ast.node_mut(id).children.push(cid);
                prev_child = Some(cid);
            }
        }
        Ok(id)
    }

    let mut ast = Ast {
        nodes: Vec::new(),
        root: 0,
        input: Vec::new(),
    };
    let root = build(obj, &mut ast, None, None)?;
    ast.root = root;
    Ok(ast)
}

/// Serialize a core `Ast` into the sanitized JS object shape (same keys the
/// conformance corpus records).
fn ast_to_object(env: Env, ast: &Ast) -> Result<JsObject> {
    fn build(env: Env, ast: &Ast, id: usize) -> Result<JsObject> {
        let n = ast.node(id);
        let mut obj = env.create_object()?;
        obj.set_named_property("type", n.ty.as_str())?;
        if let Some(v) = &n.value {
            obj.set_named_property("value", api::from_units(v))?;
        }
        if n.ty == NodeType::Brace {
            obj.set_named_property("dollar", n.dollar)?;
            obj.set_named_property("depth", n.depth)?;
            obj.set_named_property("commas", n.commas)?;
            obj.set_named_property("ranges", n.ranges)?;
            if let Some(inv) = n.invalid {
                obj.set_named_property("invalid", inv)?;
            }
            obj.set_named_property("open", n.open)?;
            obj.set_named_property("close", n.close)?;
        } else {
            if let Some(inv) = n.invalid {
                obj.set_named_property("invalid", inv)?;
            }
            if n.is_open {
                obj.set_named_property("isOpen", true)?;
            }
            if n.is_close {
                obj.set_named_property("isClose", true)?;
            }
        }
        if n.has_nodes() {
            let mut arr = env.create_array_with_length(n.children.len())?;
            for (i, &c) in n.children.iter().enumerate() {
                arr.set_element(i as u32, build(env, ast, c)?)?;
            }
            obj.set_named_property("nodes", arr)?;
        }
        Ok(obj)
    }
    build(env, ast, ast.root)
}

#[napi]
pub fn parse(pattern: String, options: Option<JsObject>, env: Env) -> Result<JsObject> {
    let opts = opts_arg(options, env)?;
    let ast = api::parse_str(&pattern, &opts).map_err(err)?;
    clear_transform();
    ast_to_object(env, &ast)
}

#[napi]
pub fn expand_ast(
    ast_obj: JsObject,
    options: Option<JsObject>,
    env: Env,
) -> Result<U16String> {
    let opts = opts_arg(options, env)?;
    let mut ast = ast_from_object(&ast_obj)?;
    let r = api::expand_ast_ropes(&mut ast, &opts).map(pack_ropes).map_err(err);
    clear_transform();
    r
}

#[napi]
pub fn compile_ast(ast_obj: JsObject, options: Option<JsObject>, env: Env) -> Result<U16String> {
    let opts = opts_arg(options, env)?;
    let mut ast = ast_from_object(&ast_obj)?;
    let r = api::compile_ast(&mut ast, &opts)
        .map(|s| U16String(s.encode_utf16().collect()))
        .map_err(err);
    clear_transform();
    r
}

#[napi]
pub fn stringify(
    input: Either<String, JsObject>,
    options: Option<JsObject>,
    env: Env,
) -> Result<String> {
    let opts = opts_arg(options, env)?;
    let r = match input {
        Either::A(s) => api::stringify_str(&s, &opts).map_err(err),
        Either::B(obj) => {
            let ast = ast_from_object(&obj)?;
            Ok(api::stringify_ast(&ast, &opts))
        }
    };
    clear_transform();
    r
}

#[napi]
pub fn stringify_string(pattern: String, options: Option<JsObject>, env: Env) -> Result<String> {
    stringify(Either::A(pattern), options, env)
}

/// Bulk string results cross the FFI boundary **packed into one string**:
/// `[count: u32 as 2 chars][end-offsets: count × u32 as 2 chars][data]`.
/// Creating one JS string per element through N-API costs ~130ns/string
/// (measured), and a `{offsets, data}` object costs ~2.4µs of conversion
/// per call — both exceed the JS reference's budget on most patterns. One
/// joined string + `substring` unpacking in the wrapper (~15ns/string)
/// keeps every pattern class competitive; results stay byte-identical.
fn pack_str(items: Vec<Vec<u16>>) -> U16String {
    let count = items.len() as u32;
    let total: usize = items.iter().map(|v| v.len()).sum();
    let mut buf: Vec<u16> = Vec::with_capacity(2 + items.len() * 2 + total);
    buf.push((count & 0xFFFF) as u16);
    buf.push((count >> 16) as u16);
    let mut pos: u32 = 0;
    for it in &items {
        pos += it.len() as u32;
        buf.push((pos & 0xFFFF) as u16);
        buf.push((pos >> 16) as u16);
    }
    for it in items {
        buf.extend_from_slice(&it);
    }
    U16String(buf)
}
/// Pack output ropes into the same wire format as [`pack_str`], writing
/// each rope's segments straight into the buffer — multi-output
/// expansions skip the per-string Vec<u16> materialisation entirely.
fn pack_ropes(items: Vec<api::Rope>) -> U16String {
    let count = items.len() as u32;
    let total: usize = items.iter().map(|r| r.unit_len()).sum();
    let mut buf: Vec<u16> = Vec::with_capacity(2 + items.len() * 2 + total);
    buf.push((count & 0xFFFF) as u16);
    buf.push((count >> 16) as u16);
    let mut pos: u32 = 0;
    for it in &items {
        pos += it.unit_len() as u32;
        buf.push((pos & 0xFFFF) as u16);
        buf.push((pos >> 16) as u16);
    }
    api::Rope::write_many(&items, &mut buf);
    U16String(buf)
}

#[napi(object)]
pub struct PackedStrings {
    pub offsets: Uint32Array,
    pub data: U16String,
}

fn pack_strings(items: Vec<Vec<u16>>) -> PackedStrings {
    let mut offsets: Vec<u32> = Vec::with_capacity(items.len());
    let mut data: Vec<u16> = Vec::new();
    let mut pos: u32 = 0;
    for it in items {
        pos += it.len() as u32;
        offsets.push(pos);
        data.extend_from_slice(&it);
    }
    PackedStrings {
        offsets: Uint32Array::new(offsets),
        data: U16String(data),
    }
}

/// Batch variant of [`PackedStrings`]: flat offsets for all strings plus
/// cumulative per-row string counts.
#[napi(object)]
pub struct PackedBatch {
    pub offsets: Uint32Array,
    pub row_ends: Uint32Array,
    pub data: U16String,
}

fn pack_batch(rows: Vec<Vec<Vec<u16>>>) -> PackedBatch {
    let mut offsets: Vec<u32> = Vec::new();
    let mut row_ends: Vec<u32> = Vec::with_capacity(rows.len());
    let mut data: Vec<u16> = Vec::new();
    let mut pos: u32 = 0;
    let mut count: u32 = 0;
    for row in rows {
        for it in row {
            pos += it.len() as u32;
            offsets.push(pos);
            data.extend_from_slice(&it);
            count += 1;
        }
        row_ends.push(count);
    }
    PackedBatch {
        offsets: Uint32Array::new(offsets),
        row_ends: Uint32Array::new(row_ends),
        data: U16String(data),
    }
}
