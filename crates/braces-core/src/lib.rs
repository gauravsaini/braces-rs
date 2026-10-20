//! braces-core — a faithful, dependency-free Rust port of micromatch/braces
//! v3.0.3 (plus its fill-range / to-regex-range dependencies), written against
//! the 2008-fixture conformance corpus generated from the JS reference.
//!
//! All string processing happens on UTF-16 code units (`Vec<u16>`), matching
//! JavaScript string semantics exactly (indexing, `.length`, `charCodeAt`,
//! surrogate pairs) — see `parse` and `fill`.

pub mod api;
pub mod ast;
pub mod compile;
pub mod error;
pub mod expand;
pub mod fill;
pub mod jsnum;
pub mod options;
pub mod parse;
pub mod regex_range;
pub mod stringify;
pub mod utils;

pub use api::{
    braces, compile_str, create, expand_ast, expand_str, parse_str, stringify_ast, stringify_str,
    CreateResult, Input, Value,
};
pub use ast::Ast;
pub use error::BracesError;
pub use options::{Options, RangeLimit, Transform};
