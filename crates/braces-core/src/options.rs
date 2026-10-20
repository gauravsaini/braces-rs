//! Option plumbing. Mirrors the micromatch/braces v3 option surface:
//! maxLength, expand, nodupes, noempty, rangeLimit, transform, keepEscaping,
//! keepQuotes, escapeInvalid, step, quantifiers (dead in v3.0.3 — accepted and
//! ignored), capture, wrap, shorthand, strictZeros, relaxZeros.

use std::sync::Arc;

/// The `transform` option. The corpus exercises the three built-in tags;
/// a real JS function can be supplied through the napi layer (`Custom`),
/// which is kept off the hot path (LEARNINGS.md #6 — detected once,
/// branched once).
#[derive(Clone)]
pub enum Transform {
    /// `v => (typeof v === 'number' ? v * 2 : v)`
    Double,
    /// `(v, i) => 'foo/' + String.fromCharCode(v) + '-' + i`
    AlphaFoo,
    /// `v => 'N' + v`
    PrefixN,
    /// A caller-supplied function. Receives the numeric value (number ranges)
    /// or the UTF-16 code (alpha ranges) plus the item index — exactly like
    /// the JS reference.
    Custom(Arc<dyn Fn(f64, usize) -> TransformOut + Send + Sync>),
}

impl std::fmt::Debug for Transform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Transform::Double => write!(f, "Double"),
            Transform::AlphaFoo => write!(f, "AlphaFoo"),
            Transform::PrefixN => write!(f, "PrefixN"),
            Transform::Custom(_) => write!(f, "Custom(..)"),
        }
    }
}

/// What a transform produced. Mirrors the JS reference, where `pad()`
/// distinguishes numbers from strings (`input.padStart` exists only on
/// strings — a number there raises `TypeError: input.padStart is not a
/// function`).
#[derive(Clone, Debug)]
pub enum TransformOut {
    Num(f64),
    Str(Vec<u16>),
}

/// `rangeLimit`: a number, or `false` to disable (default 1000).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RangeLimit {
    Num(f64),
    Disabled,
}

impl Default for RangeLimit {
    fn default() -> Self {
        RangeLimit::Num(1000.0)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub max_length: Option<f64>,
    pub expand: bool,
    pub nodupes: bool,
    pub noempty: bool,
    pub range_limit: RangeLimit,
    pub transform: Option<Transform>,
    /// `step` option: fill-range fallback step for ranges without an
    /// explicit `..step`. `None` = unset. Note the JS falsy semantics are
    /// preserved by keeping the raw value (`Some(0.0)` is meaningful —
    /// it makes `exceedsLimit` divide by zero, matching the reference).
    pub step: Option<StepVal>,
    pub keep_escaping: bool,
    pub keep_quotes: bool,
    pub escape_invalid: bool,
    pub capture: Option<bool>,
    pub wrap: Option<bool>,
    pub shorthand: Option<bool>,
    pub strict_zeros: Option<bool>,
    pub relax_zeros: Option<bool>,
}

/// The `step` option value, kept raw so JS falsy semantics survive.
#[derive(Clone, Debug)]
pub enum StepVal {
    Num(f64),
    Str(Vec<u16>),
}

impl Options {
    /// Parse-level options shared by parse/stringify.
    pub fn for_parse(&self) -> ParseOpts {
        ParseOpts {
            keep_escaping: self.keep_escaping,
            keep_quotes: self.keep_quotes,
            max_length: self.max_length,
        }
    }

    /// The raw `options.step` value as f64 for `exceedsLimit`: `None` means
    /// JS `undefined` (the `step = 1` default applies); `Some(NaN)` comes
    /// from non-numeric step strings (`(max-min)/NaN >= limit` is false,
    /// exactly like the reference).
    pub fn step_num(&self) -> Option<f64> {
        match &self.step {
            None => None,
            Some(StepVal::Num(n)) => Some(*n),
            Some(StepVal::Str(s)) => Some(crate::jsnum::to_number(s)),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ParseOpts {
    pub keep_escaping: bool,
    pub keep_quotes: bool,
    pub max_length: Option<f64>,
}
