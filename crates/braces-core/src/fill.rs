//! Faithful port of micromatch/fill-range `index.js`.
//!
//! Only the subset reachable from braces is exercised, but the control flow
//! (invalidRange/invalidStep, `step || opts.step || 1` fallback, the
//! `toRegex`/`toSequence`/`toRange` compile paths, padding, transforms) is
//! ported exactly, including the `TypeError: input.padStart is not a
//! function` failure when a transform returns a number for a padded range.

use crate::error::BracesError;
use crate::jsnum;
use crate::options::{StepVal, Transform, TransformOut};
use crate::regex_range::{to_regex_range, RrOptions};

#[derive(Clone, Debug, Default)]
pub struct FillOptions {
    pub transform: Option<Transform>,
    pub step: Option<StepVal>,
    pub to_regex: bool,
    pub capture: Option<bool>,
    pub wrap: Option<bool>,
    pub strict_zeros: Option<bool>,
    pub relax_zeros: Option<bool>,
    pub shorthand: Option<bool>,
}

/// fill() output: an expansion list, or a single regex string (toRegex).
pub enum FillOut {
    List(Vec<Vec<u16>>),
    Regex(Vec<u16>),
}

impl FillOptions {
    /// Options for the expand path: compile forces
    /// `{...options, wrap: false, toRegex: true, strictZeros: true}`.
    pub fn from_options(o: &crate::options::Options, to_regex: bool) -> Self {
        FillOptions {
            transform: o.transform.clone(),
            step: o.step.clone(),
            to_regex,
            capture: o.capture,
            wrap: if to_regex { Some(false) } else { o.wrap },
            strict_zeros: if to_regex { Some(true) } else { o.strict_zeros },
            relax_zeros: o.relax_zeros,
            shorthand: o.shorthand,
        }
    }

    pub fn from_compile_options(o: &crate::options::Options) -> Self {
        Self::from_options(o, true)
    }
}

#[derive(Clone, Debug)]
enum Item {
    Num(f64),
    Str(Vec<u16>),
}

fn is_number_val(s: &[u16]) -> bool {
    // fill-range's isNumber: Number.isInteger(+num)
    jsnum::is_integer_str(s)
}

fn zeros(input: &[u16]) -> bool {
    let mut v = input;
    if !v.is_empty() && v[0] == b'-' as u16 {
        v = &v[1..];
    }
    if v == [b'0' as u16] {
        return false;
    }
    let mut index = 0;
    while index < v.len() && v[index] == b'0' as u16 {
        index += 1;
    }
    index > 0
}

fn pad(input: Item, max_length: usize, to_number: bool) -> Result<Item, BracesError> {
    let mut s: Vec<u16> = match input {
        Item::Num(_) if max_length > 0 => {
            // `input.padStart` does not exist on numbers.
            return Err(BracesError::PadStartOnNumber);
        }
        Item::Num(n) => {
            if to_number {
                return Ok(Item::Num(n));
            }
            jsnum::num_to_string(n)
        }
        Item::Str(v) => v,
    };
    if max_length > 0 {
        let dash = !s.is_empty() && s[0] == b'-' as u16;
        let body = if dash { &s[1..] } else { &s[..] };
        let target = if dash { max_length - 1 } else { max_length };
        let mut padded = Vec::with_capacity(target.max(body.len()) + 1);
        if dash {
            padded.push(b'-' as u16);
        }
        for _ in body.len()..target {
            padded.push(b'0' as u16);
        }
        padded.extend_from_slice(body);
        s = padded;
    }
    if to_number {
        // unreachable in braces usage (toNumber ⟺ !padded ⟺ max_length == 0),
        // kept for port fidelity
        Ok(Item::Str(s))
    } else {
        Ok(Item::Str(s))
    }
}

fn to_max_len(input: &[u16], max_length: usize) -> Vec<u16> {
    let negative = !input.is_empty() && input[0] == b'-' as u16;
    let body = if negative { &input[1..] } else { input };
    let target = if negative {
        max_length.saturating_sub(1)
    } else {
        max_length
    };
    let mut out = Vec::with_capacity(target.max(body.len()) + 1);
    if negative {
        out.push(b'-' as u16);
    }
    for _ in body.len()..target {
        out.push(b'0' as u16);
    }
    out.extend_from_slice(body);
    out
}

fn default_format_numbers(to_number: bool) -> impl Fn(f64, usize) -> TransformOut {
    move |v, _| {
        if to_number {
            TransformOut::Num(v)
        } else {
            TransformOut::Str(jsnum::num_to_string(v))
        }
    }
}

fn apply_transform(t: &Option<Transform>, v: f64, i: usize, default: &dyn Fn(f64, usize) -> TransformOut) -> TransformOut {
    match t {
        None => default(v, i),
        Some(Transform::Double) => TransformOut::Num(v * 2.0),
        Some(Transform::AlphaFoo) => {
            let mut s: Vec<u16> = "foo/".encode_utf16().collect();
            s.push(jsnum::from_char_code(v));
            s.push(b'-' as u16);
            s.extend(format!("{i}").encode_utf16());
            TransformOut::Str(s)
        }
        Some(Transform::PrefixN) => {
            let mut s: Vec<u16> = vec![b'N' as u16];
            s.extend(jsnum::num_to_string(v));
            TransformOut::Str(s)
        }
        Some(Transform::Custom(f)) => f(v, i),
    }
}

fn fill_numbers(
    start: &[u16],
    end: &[u16],
    step: f64,
    opts: &FillOptions,
) -> Result<FillOut, BracesError> {
    let mut a = jsnum::to_number(start);
    let mut b = jsnum::to_number(end);
    if !jsnum::is_integer_num(a) || !jsnum::is_integer_num(b) {
        return Ok(FillOut::List(vec![]));
    }
    if a == 0.0 {
        a = 0.0;
    }
    if b == 0.0 {
        b = 0.0;
    }
    let descending = a > b;
    let step = step.abs().max(1.0);
    let padded = zeros(start) || zeros(end) || zeros(&jsnum::num_to_string(step));
    let max_len = if padded {
        start.len().max(end.len()).max(jsnum::num_to_string(step).len())
    } else {
        0
    };
    // stringify(start, end, options): braces always passes strings → true,
    // so toNumber is always false here. Kept explicit for fidelity.
    let stringify_flag = true;
    let to_number = !padded && !stringify_flag;

    if opts.to_regex && step == 1.0 {
        // to-regex-range derives relaxZeros from strictZeros:
        // `if (typeof opts.strictZeros === 'boolean') opts.relaxZeros = opts.strictZeros === false`
        let rr = RrOptions {
            relax_zeros: opts
                .strict_zeros
                .map(|s| !s)
                .unwrap_or_else(|| opts.relax_zeros.unwrap_or(true)),
            shorthand: opts.shorthand.unwrap_or(false),
            capture: opts.capture.unwrap_or(false),
            wrap: false,
        };
        let r = to_regex_range(
            &to_max_len(start, max_len),
            &to_max_len(end, max_len),
            &rr,
        );
        return Ok(FillOut::Regex(r));
    }

    let mut parts_neg: Vec<f64> = Vec::new();
    let mut parts_pos: Vec<f64> = Vec::new();
    let mut range: Vec<Item> = Vec::new();
    let mut index = 0usize;
    let def = default_format_numbers(to_number);
    while if descending { a >= b } else { a <= b } {
        if opts.to_regex && step > 1.0 {
            if a < 0.0 {
                parts_neg.push(a.abs());
            } else {
                parts_pos.push(a);
            }
        } else {
            let formatted = apply_transform(&opts.transform, a, index, &def);
            let item = match formatted {
                TransformOut::Num(n) => Item::Num(n),
                TransformOut::Str(s) => Item::Str(s),
            };
            range.push(pad(item, max_len, to_number)?);
        }
        a = if descending { a - step } else { a + step };
        index += 1;
    }

    if opts.to_regex {
        if step > 1.0 {
            return Ok(FillOut::Regex(to_sequence(
                &parts_neg,
                &parts_pos,
                opts,
                max_len,
            )));
        }
        // Dead in practice (step == 1 returns earlier); port for fidelity:
        // toRegex(range, null, { wrap: false, ...options }) → join('|').
        let joined = join_items(&range, b'|' as u16);
        return Ok(FillOut::Regex(joined));
    }

    Ok(FillOut::List(
        range.into_iter().map(item_to_units).collect(),
    ))
}

fn item_to_units(it: Item) -> Vec<u16> {
    match it {
        Item::Num(n) => jsnum::num_to_string(n),
        Item::Str(s) => s,
    }
}

fn join_items(items: &[Item], sep: u16) -> Vec<u16> {
    let mut out = Vec::new();
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            out.push(sep);
        }
        out.extend(item_to_units(it.clone()));
    }
    out
}

/// `toRange` for the alpha path: `String.fromCharCode` endpoints.
fn to_range_letters(a: f64, b: f64) -> Vec<u16> {
    let start = jsnum::from_char_code(a);
    if a == b {
        return vec![start];
    }
    let stop = jsnum::from_char_code(b);
    vec![b'[' as u16, start, b'-' as u16, stop, b']' as u16]
}

fn to_sequence(neg: &[f64], pos: &[f64], opts: &FillOptions, max_len: usize) -> Vec<u16> {
    let mut negatives = neg.to_vec();
    let mut positives = pos.to_vec();
    negatives.sort_by(|x, y| x.partial_cmp(y).unwrap());
    positives.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let prefix: Vec<u16> = if opts.capture == Some(true) {
        Vec::new()
    } else {
        vec![b'?' as u16, b':' as u16]
    };
    let mut result: Vec<u16> = Vec::new();
    let mut positives_s = Vec::new();
    for v in &positives {
        positives_s.extend(to_max_len(&jsnum::num_to_string(*v), max_len));
        positives_s.push(b'|' as u16);
    }
    if !positives_s.is_empty() {
        positives_s.pop();
    }
    let mut negatives_s = Vec::new();
    if !negatives.is_empty() {
        negatives_s.push(b'-' as u16);
        negatives_s.push(b'(' as u16);
        negatives_s.extend_from_slice(&prefix);
        for (i, v) in negatives.iter().enumerate() {
            if i > 0 {
                negatives_s.push(b'|' as u16);
            }
            negatives_s.extend(to_max_len(&jsnum::num_to_string(*v), max_len));
        }
        negatives_s.push(b')' as u16);
    }
    if !positives_s.is_empty() && !negatives_s.is_empty() {
        result.extend(positives_s);
        result.push(b'|' as u16);
        result.extend(negatives_s);
    } else if !positives_s.is_empty() {
        result.extend(positives_s);
    } else {
        result.extend(negatives_s);
    }
    if opts.wrap == Some(true) {
        let mut w = vec![b'(' as u16];
        w.extend_from_slice(&prefix);
        w.extend(result);
        w.push(b')' as u16);
        w
    } else {
        result
    }
}

fn fill_letters(
    start: &[u16],
    end: &[u16],
    step: f64,
    opts: &FillOptions,
) -> Result<FillOut, BracesError> {
    if (!is_number_val(start) && start.len() > 1) || (!is_number_val(end) && end.len() > 1) {
        return Ok(FillOut::List(vec![]));
    }
    let mut a = jsnum::char_code_at(start) as f64;
    let b = jsnum::char_code_at(end) as f64;
    let descending = a > b;
    let (min, max) = if descending { (b, a) } else { (a, b) };

    if opts.to_regex && step == 1.0 {
        return Ok(FillOut::Regex(to_range_letters(min, max)));
    }

    let mut range: Vec<Item> = Vec::new();
    let mut index = 0usize;
    while if descending { a >= b } else { a <= b } {
        let formatted = match &opts.transform {
            None => TransformOut::Str(vec![jsnum::from_char_code(a)]),
            t => apply_transform(t, a, index, &|v, _| {
                TransformOut::Str(vec![jsnum::from_char_code(v)])
            }),
        };
        range.push(match formatted {
            TransformOut::Num(n) => Item::Num(n),
            TransformOut::Str(s) => Item::Str(s),
        });
        a = if descending { a - step } else { a + step };
        index += 1;
    }

    if opts.to_regex {
        // Port of the reference's quirk: toRegex(range, null, { wrap: false, options })
        return Ok(FillOut::Regex(join_items(&range, b'|' as u16)));
    }
    Ok(FillOut::List(
        range.into_iter().map(item_to_units).collect(),
    ))
}

/// Port of fill-range `fill(start, end, step, options)`.
/// `step_arg`: the optional third range element from `reduce` (a string),
/// or `None` when the pattern has no explicit step.
pub fn fill(
    start: &[u16],
    end: Option<&[u16]>,
    step_arg: Option<&[u16]>,
    opts: &FillOptions,
) -> Result<FillOut, BracesError> {
    let end = match end {
        // In the JS reference, `fill(...args, options)` with a single arg
        // passes the options *object* as `end` — `isValidValue({})` is
        // false, so `invalidRange` → `[]`. (strictRanges is never set by
        // braces.)
        Some(e) => e,
        None => return Ok(FillOut::List(vec![])),
    };
    if start.is_empty() || end.is_empty() {
        return Ok(FillOut::List(vec![]));
    }

    let mut fopts = opts.clone();
    if fopts.capture == Some(true) {
        fopts.wrap = Some(true);
    }

    // step = step || opts.step || 1 — JS falsy chain. step_arg is a string;
    // empty string is falsy (cannot happen from reduce, but keep semantics).
    enum StepNum {
        Num(f64),
        Invalid,
    }
    let mut step_num: Option<f64> = None;
    let mut step_invalid = false;
    if let Some(s) = step_arg {
        if !s.is_empty() {
            let n = jsnum::to_number(s);
            if jsnum::is_integer_num(n) {
                step_num = Some(n);
            } else {
                step_invalid = true;
            }
        }
    }
    if step_num.is_none() && !step_invalid {
        if let Some(ref sv) = fopts.step {
            match sv {
                // JS falsy chain: 0 and NaN fall through to 1; non-integers
                // fail isNumber → invalidStep.
                StepVal::Num(n) if jsnum::is_integer_num(*n) && *n != 0.0 => {
                    step_num = Some(*n)
                }
                StepVal::Num(_) => { /* 0/NaN/Infinity → fall through */ }
                StepVal::Str(s) if !s.is_empty() => {
                    let n = jsnum::to_number(s);
                    if jsnum::is_integer_num(n) {
                        step_num = Some(n);
                    } else {
                        step_invalid = true;
                    }
                }
                _ => {}
            }
        }
    }
    if step_invalid {
        // invalidStep: strictRanges is never set by braces → []
        return Ok(FillOut::List(vec![]));
    }
    let step = step_num.unwrap_or(1.0);

    if is_number_val(start) && is_number_val(end) {
        fill_numbers(start, end, step, &fopts)
    } else {
        fill_letters(start, end, step.abs().max(1.0), &fopts)
    }
}
