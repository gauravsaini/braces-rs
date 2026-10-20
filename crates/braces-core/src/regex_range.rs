//! Faithful port of micromatch/to-regex-range `index.js`.
//!
//! All range math runs on f64 exactly like the JS reference (`Math.min`,
//! `%`, string slicing of `String(n)`), including the internal result cache
//! (`toRegexRange.cache`) — ported as a thread-local map (no lock
//! contention; the reference cache is process-global, thread-local is
//! equivalent for single-threaded bench use and safe for multi-threaded).

use crate::jsnum;
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub struct RrOptions {
    pub relax_zeros: bool,
    pub shorthand: bool,
    pub capture: bool,
    pub wrap: bool,
}

impl Default for RrOptions {
    fn default() -> Self {
        RrOptions {
            relax_zeros: true,
            shorthand: false,
            capture: false,
            wrap: true,
        }
    }
}

thread_local! {
    static RR_CACHE: RefCell<HashMap<String, Vec<u16>>> = RefCell::new(HashMap::new());
}

fn u16s(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn is_number_str(s: &[u16]) -> bool {
    if jsnum::is_integer_str(s) || {
        let t = s;
        // Number.isFinite(+v) && v.trim() !== ''
        let trimmed_empty = {
            let mut a = 0;
            let mut b = t.len();
            while a < b && (t[a] as u8 as char).is_whitespace() {
                a += 1;
            }
            while b > a && (t[b - 1] as u8 as char).is_whitespace() {
                b -= 1;
            }
            a == b
        };
        !trimmed_empty && jsnum::to_number(t).is_finite()
    } {
        true
    } else {
        false
    }
}

fn has_padding(s: &[u16]) -> bool {
    // /^-?(0+)\d/
    let t = if !s.is_empty() && s[0] == b'-' as u16 {
        &s[1..]
    } else {
        s
    };
    let mut i = 0;
    while i < t.len() && t[i] == b'0' as u16 {
        i += 1;
    }
    i > 0 && i < t.len() && t[i] >= b'0' as u16 && t[i] <= b'9' as u16
}

fn cache_key(min: &[u16], max: &[u16], o: &RrOptions) -> String {
    let s = |v: &[u16]| String::from_utf16_lossy(v);
    format!(
        "{}:{}={}{}{}{}",
        s(min),
        s(max),
        o.relax_zeros,
        o.shorthand,
        o.capture,
        o.wrap
    )
}

pub fn to_regex_range(min: &[u16], max: &[u16], opts: &RrOptions) -> Vec<u16> {
    if !is_number_str(min) {
        // The reference throws TypeError here; braces never triggers it
        // (endpoints are pre-validated). Return empty (unreachable).
        return Vec::new();
    }
    if min == max {
        return min.to_vec();
    }
    if !is_number_str(max) {
        return Vec::new();
    }

    let key = cache_key(min, max, opts);
    if let Some(hit) = RR_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }

    let a0 = jsnum::to_number(min).min(jsnum::to_number(max));
    let b0 = jsnum::to_number(min).max(jsnum::to_number(max));

    if (a0 - b0).abs() == 1.0 {
        let mut result = min.to_vec();
        result.push(b'|' as u16);
        result.extend_from_slice(max);
        if opts.capture {
            let mut w = u16s("(");
            w.extend(result);
            w.extend(u16s(")"));
            return w;
        }
        if !opts.wrap {
            return result;
        }
        let mut w = u16s("(?:");
        w.extend(result);
        w.extend(u16s(")"));
        return w;
    }

    let is_padded = has_padding(min) || has_padding(max);
    let max_len = String::from_utf16_lossy(max).encode_utf16().count();

    let mut a = a0;
    let mut b = b0;
    let mut negatives: Vec<Token> = Vec::new();
    let mut positives: Vec<Token> = Vec::new();

    let tok = Tok {
        is_padded,
        max_len,
    };

    if a < 0.0 {
        let new_min = if b < 0.0 { b.abs() } else { 1.0 };
        negatives = split_to_patterns(new_min, a.abs(), &tok, opts);
        a = 0.0;
    }
    if b >= 0.0 {
        positives = split_to_patterns(a, b, &tok, opts);
    }

    let mut result = collate_patterns(&negatives, &positives, opts);
    if opts.capture {
        let mut w = u16s("(");
        w.extend(result);
        w.push(b')' as u16);
        result = w;
    } else if opts.wrap && negatives.len() + positives.len() > 1 {
        let mut w = u16s("(?:");
        w.extend(result);
        w.push(b')' as u16);
        result = w;
    }

    RR_CACHE.with(|c| {
        c.borrow_mut().insert(key, result.clone());
    });
    result
}

struct Tok {
    is_padded: bool,
    max_len: usize,
}

#[derive(Clone, Debug)]
struct Token {
    pattern: Vec<u16>,
    count: Vec<usize>,
    string: Vec<u16>,
}

fn count_nines(min: f64, len: i32) -> f64 {
    let s = jsnum::num_to_string(min);
    let keep = s.len().saturating_sub(len as usize);
    let mut t: Vec<u16> = s[..keep].to_vec();
    t.extend(std::iter::repeat(b'9' as u16).take(len.max(0) as usize));
    jsnum::to_number(&t)
}

fn count_zeros(integer: f64, zeros: i32) -> f64 {
    integer - (integer % 10f64.powi(zeros))
}

fn split_to_ranges(min: f64, max: f64) -> Vec<f64> {
    let mut nines = 1i32;
    let mut zeros = 1i32;
    let mut stop = count_nines(min, nines);
    let mut stops: Vec<f64> = vec![max];
    while min <= stop && stop <= max {
        if !stops.contains(&stop) {
            stops.push(stop);
        }
        nines += 1;
        stop = count_nines(min, nines);
    }
    stop = count_zeros(max + 1.0, zeros) - 1.0;
    while min < stop && stop <= max {
        if !stops.contains(&stop) {
            stops.push(stop);
        }
        zeros += 1;
        stop = count_zeros(max + 1.0, zeros) - 1.0;
    }
    stops.sort_by(|x, y| x.partial_cmp(y).unwrap());
    stops
}

fn to_character_class(a: u16, b: u16) -> Vec<u16> {
    let mut p = vec![b'[' as u16, a];
    if (b as i32 - a as i32) != 1 {
        p.push(b'-' as u16);
    }
    p.push(b);
    p.push(b']' as u16);
    p
}

fn range_to_pattern(start: &[u16], stop: &[u16], opts: &RrOptions) -> Token {
    if start == stop {
        return Token {
            pattern: start.to_vec(),
            count: vec![],
            string: Vec::new(),
        };
    }
    let mut pattern: Vec<u16> = Vec::new();
    let mut count = 0usize;
    for i in 0..start.len() {
        let sd = start[i];
        let ed = stop[i];
        if sd == ed {
            pattern.push(sd);
        } else if sd != b'0' as u16 || ed != b'9' as u16 {
            pattern.extend(to_character_class(sd, ed));
        } else {
            count += 1;
        }
    }
    if count > 0 {
        pattern.extend(if opts.shorthand {
            u16s("\\d")
        } else {
            u16s("[0-9]")
        });
    }
    Token {
        pattern,
        count: vec![count],
        string: Vec::new(),
    }
}

fn to_quantifier(digits: &[usize]) -> Vec<u16> {
    let start = digits.first().copied().unwrap_or(0);
    let stop = digits.get(1).copied().filter(|&s| s != 0);
    if stop.is_some() || start > 1 {
        let mut q = u16s("{");
        q.extend(u16s(&start.to_string()));
        if let Some(s) = stop {
            q.push(b',' as u16);
            q.extend(u16s(&s.to_string()));
        }
        q.push(b'}' as u16);
        q
    } else {
        Vec::new()
    }
}

fn pad_zeros(value: f64, tok: &Tok, opts: &RrOptions) -> Vec<u16> {
    if !tok.is_padded {
        return jsnum::num_to_string(value);
    }
    let vlen = jsnum::num_to_string(value).len();
    let diff = (tok.max_len as i64 - vlen as i64).abs();
    let relax = opts.relax_zeros;
    match diff {
        0 => Vec::new(),
        1 => {
            if relax {
                u16s("0?")
            } else {
                u16s("0")
            }
        }
        2 => {
            if relax {
                u16s("0{0,2}")
            } else {
                u16s("00")
            }
        }
        _ => {
            if relax {
                let mut s = u16s("0{0,");
                s.extend(u16s(&diff.to_string()));
                s.push(b'}' as u16);
                s
            } else {
                let mut s = u16s("0{");
                s.extend(u16s(&diff.to_string()));
                s.push(b'}' as u16);
                s
            }
        }
    }
}

fn split_to_patterns(min: f64, max: f64, tok: &Tok, opts: &RrOptions) -> Vec<Token> {
    let ranges = split_to_ranges(min, max);
    let mut tokens: Vec<Token> = Vec::new();
    let mut start = min;
    // prev is an index into `tokens` (the reference mutates the previous
    // token in place on merge — a clone would silently drop the mutation).
    let mut prev: Option<usize> = None;
    for mx in ranges {
        let obj = range_to_pattern(&jsnum::num_to_string(start), &jsnum::num_to_string(mx), opts);
        let merge = !tok.is_padded
            && prev.map_or(false, |i| tokens[i].pattern == obj.pattern);
        if merge {
            let i = prev.unwrap();
            {
                let p = &mut tokens[i];
                if p.count.len() > 1 {
                    p.count.pop();
                }
                p.count.push(obj.count.first().copied().unwrap_or(0));
                let mut s = p.pattern.clone();
                s.extend(to_quantifier(&p.count));
                p.string = s;
            }
            start = mx + 1.0;
            continue;
        }
        let mut obj = obj;
        let mut zeros = Vec::new();
        if tok.is_padded {
            zeros = pad_zeros(mx, tok, opts);
        }
        let mut s = zeros;
        s.extend_from_slice(&obj.pattern);
        s.extend(to_quantifier(&obj.count));
        obj.string = s;
        start = mx + 1.0;
        tokens.push(obj);
        prev = Some(tokens.len() - 1);
    }
    tokens
}

fn contains_string(arr: &[Token], val: &[u16]) -> bool {
    arr.iter().any(|t| t.string == val)
}

fn filter_patterns(
    arr: &[Token],
    comparison: &[Token],
    prefix: &str,
    intersection: bool,
    out: &mut Vec<u16>,
    first: &mut bool,
) {
    for ele in arr {
        let s = &ele.string;
        let take = if !intersection {
            !contains_string(comparison, s)
        } else {
            contains_string(comparison, s)
        };
        if take {
            if !*first {
                out.push(b'|' as u16);
            }
            *first = false;
            out.extend(u16s(prefix));
            out.extend_from_slice(s);
        }
    }
}

fn collate_patterns(neg: &[Token], pos: &[Token], _opts: &RrOptions) -> Vec<u16> {
    let mut out = Vec::new();
    let mut first = true;
    filter_patterns(neg, pos, "-", false, &mut out, &mut first);
    filter_patterns(neg, pos, "-?", true, &mut out, &mut first);
    filter_patterns(pos, neg, "", false, &mut out, &mut first);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(min: &str, max: &str, o: &RrOptions) -> String {
        String::from_utf16(&to_regex_range(
            &min.encode_utf16().collect::<Vec<_>>(),
            &max.encode_utf16().collect::<Vec<_>>(),
            o,
        ))
        .unwrap()
    }

    #[test]
    fn basic() {
        let o = RrOptions {
            wrap: false,
            ..Default::default()
        };
        assert_eq!(r("1", "3", &o), "[1-3]");
        assert_eq!(r("1", "10", &o), "[1-9]|10");
        assert_eq!(r("1", "100", &o), "[1-9]|[1-9][0-9]|100");
    }
}
