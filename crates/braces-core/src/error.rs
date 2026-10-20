//! Error model mirroring the JS reference's observable error behaviour.
//!
//! Three error classes are byte-identical to the reference (name + message):
//! - `RangeError` from the expand range limit,
//! - `SyntaxError` from the parse max-length guard,
//! - `TypeError: Expected a string` from `parse` on non-string input.
//!
//! Everything else is a V8-implementation-specific message (the JS code
//! throws incidental engine TypeErrors such as "Cannot create property
//! 'queue' on number '123'"); those keep the same JS error *name* but carry
//! a descriptive message — they are listed in DIVERGENCES.md.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum BracesError {
    /// RangeError: expanded array length exceeds range limit. Use
    /// options.rangeLimit to increase or disable the limit.
    RangeLimit,
    /// SyntaxError: Input length (N), exceeds max characters (M)
    MaxLength { len: usize, max: usize },
    /// TypeError: Expected a string
    ExpectedString,
    /// JS `TypeError: input.padStart is not a function` — raised by
    /// fill-range's pad() when a transform returns a number for a padded
    /// (zero-prefixed) numeric range. Message is byte-identical to the
    /// reference (verified by the exact napi conformance checker).
    PadStartOnNumber,
    /// Non-string input reached a code path that dereferences it like an
    /// AST node (expand/compile/create on null/undefined/number/bool/
    /// object/array). The JS reference throws V8-worded TypeErrors here;
    /// we throw TypeError with a clear message (documented divergence).
    NonStringInput(&'static str),
}

impl BracesError {
    /// The JS error class name (`TypeError`, `RangeError`, `SyntaxError`).
    pub fn name(&self) -> &'static str {
        match self {
            BracesError::RangeLimit => "RangeError",
            BracesError::MaxLength { .. } => "SyntaxError",
            BracesError::ExpectedString
            | BracesError::PadStartOnNumber
            | BracesError::NonStringInput(_) => "TypeError",
        }
    }

    /// The JS-style message (without the `Name: ` prefix).
    pub fn message(&self) -> String {
        match self {
            BracesError::RangeLimit => {
                "expanded array length exceeds range limit. Use options.rangeLimit to increase or disable the limit."
                    .to_string()
            }
            BracesError::MaxLength { len, max } => {
                format!("Input length ({len}), exceeds max characters ({max})")
            }
            BracesError::ExpectedString => "Expected a string".to_string(),
            BracesError::PadStartOnNumber => "input.padStart is not a function".to_string(),
            BracesError::NonStringInput(what) => {
                format!("braces-rs: {what} input is not a string or AST")
            }
        }
    }

    /// `Name: message` — the shape recorded in the conformance corpus.
    pub fn to_js_string(&self) -> String {
        format!("{}: {}", self.name(), self.message())
    }
}

impl fmt::Display for BracesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_js_string())
    }
}

impl std::error::Error for BracesError {}
