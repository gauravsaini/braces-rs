//! JavaScript number semantics needed by the port:
//! - `StringToNumber` for strings (unary `+`, `Number()`),
//! - `Number.prototype.toString` for f64 (JS shortest-round-trip with JS
//!   exponent rules — Rust's `Display` differs, e.g. `1e21` vs a 22-digit
//!   string),
//! - `String.fromCharCode` (ToUint16),
//! - `Number.isInteger`.
//!
//! All brace-range math in the reference runs on f64, so these must match
//! the engine exactly for byte-identical output.

/// JS `Number.isInteger`: true for integer-valued finite f64.
#[inline]
pub fn is_integer_num(v: f64) -> bool {
    v.is_finite() && v.fract() == 0.0
}

/// JS `StringToNumber` on UTF-16 code units. Handles the forms the reference
/// can observe: surrounding whitespace, `Infinity`, hex/binary/octal
/// literals, decimal with optional fraction and exponent. Returns NaN for
/// anything else.
pub fn to_number(s: &[u16]) -> f64 {
    let t = trim_js(s);
    if t.is_empty() {
        return 0.0;
    }
    let neg = t[0] == b'-' as u16;
    let mut rest: &[u16] = if t[0] == b'-' as u16 || t[0] == b'+' as u16 {
        &t[1..]
    } else {
        t
    };
    if rest.is_empty() {
        return f64::NAN;
    }
    // Infinity
    if eq_ascii(rest, "Infinity") {
        return if neg { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    // Hex / binary / octal (only without sign... JS actually allows sign
    // before 0x? No: Number('-0x10') is NaN. Keep: sign already stripped
    // means we must reject if neg/plus was present.)
    let had_sign = t[0] == b'-' as u16 || t[0] == b'+' as u16;
    if rest.len() > 2 && rest[0] == b'0' as u16 {
        let radix = match rest[1] {
            0x78 | 0x58 => 16,
            0x62 | 0x42 => 2,
            0x6F | 0x4F => 8,
            _ => 0,
        };
        if radix != 0 {
            if had_sign {
                return f64::NAN;
            }
            let mut v: f64 = 0.0;
            for &c in &rest[2..] {
                let d = hex_val(c);
                if d < 0 || d >= radix as i32 {
                    return f64::NAN;
                }
                v = v * radix as f64 + d as f64;
            }
            if rest.len() == 2 {
                return f64::NAN; // "0x" alone
            }
            return v;
        }
    }
    if had_sign {
        rest = &t[1..];
    }
    parse_decimal(rest, neg)
}

fn hex_val(c: u16) -> i32 {
    match c {
        0x30..=0x39 => (c - 0x30) as i32,
        0x61..=0x66 => (c - 0x61) as i32 + 10,
        0x41..=0x46 => (c - 0x41) as i32 + 10,
        _ => -1,
    }
}

fn eq_ascii(s: &[u16], lit: &str) -> bool {
    s.len() == lit.len() && s.iter().zip(lit.bytes()).all(|(a, b)| *a == b as u16)
}

fn parse_decimal(s: &[u16], neg: bool) -> f64 {
    // [digits][.digits][e[+-]digits] with at least one digit overall.
    let mut i = 0;
    let n = s.len();
    let mut int_digits = 0;
    while i < n && s[i] >= b'0' as u16 && s[i] <= b'9' as u16 {
        i += 1;
        int_digits += 1;
    }
    let mut frac_digits = 0;
    if i < n && s[i] == b'.' as u16 {
        i += 1;
        while i < n && s[i] >= b'0' as u16 && s[i] <= b'9' as u16 {
            i += 1;
            frac_digits += 1;
        }
    }
    if int_digits + frac_digits == 0 {
        return f64::NAN;
    }
    let mut exp: i32 = 0;
    let mut exp_start: usize = n;
    if i < n && (s[i] == b'e' as u16 || s[i] == b'E' as u16) {
        exp_start = i;
        i += 1;
        let mut exp_neg = false;
        if i < n && (s[i] == b'+' as u16 || s[i] == b'-' as u16) {
            exp_neg = s[i] == b'-' as u16;
            i += 1;
        }
        let mut ed = 0;
        let start = i;
        while i < n && s[i] >= b'0' as u16 && s[i] <= b'9' as u16 {
            ed = ed * 10 + (s[i] - b'0' as u16) as i32;
            i += 1;
        }
        if i == start {
            return f64::NAN;
        }
        exp = if exp_neg { -ed } else { ed };
    }
    if i != n {
        return f64::NAN;
    }
    // exponent present: hand the mantissa (without the exponent suffix)
    // to the shifter
    let mant_end = if exp == 0 { i } else { exp_start };
    let mut buf = String::with_capacity(n + 1);
    if neg {
        buf.push('-');
    }
    for &c in &s[..mant_end] {
        buf.push(c as u8 as char);
    }
    if exp == 0 {
        return buf.parse::<f64>().unwrap_or(f64::NAN);
    }
    shift_exp(exp, &buf)
}

/// Apply the decimal exponent shift, re-emit, and parse with f64.
fn shift_exp(exp: i32, buf: &str) -> f64 {
    // Split mantissa into digits and decimal position, shift, re-emit.
    let (intpart, fracpart) = match buf.find('.') {
        Some(p) => (&buf[..p], &buf[p + 1..]),
        None => (buf, ""),
    };
    let neg = intpart.starts_with('-');
    let intd = intpart.trim_start_matches('-');
    let mut digits: Vec<u8> = Vec::new();
    digits.extend(intd.bytes().filter(|b| b.is_ascii_digit()));
    let frac_digits: Vec<u8> = fracpart.bytes().filter(|b| b.is_ascii_digit()).collect();
    let point = digits.len() as i32; // decimal point sits after `point` digits
    digits.extend(frac_digits);
    // strip leading zeros for exponent math (keep at least one digit)
    let new_point = point + exp;
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if new_point <= 0 {
        out.push_str("0.");
        for _ in 0..-new_point {
            out.push('0');
        }
        for d in &digits {
            out.push(*d as char);
        }
    } else if new_point as usize >= digits.len() {
        for d in &digits {
            out.push(*d as char);
        }
        for _ in 0..(new_point as usize - digits.len()) {
            out.push('0');
        }
    } else {
        for d in &digits[..new_point as usize] {
            out.push(*d as char);
        }
        out.push('.');
        for d in &digits[new_point as usize..] {
            out.push(*d as char);
        }
    }
    out.parse::<f64>().unwrap_or(f64::NAN)
}

/// JS whitespace trim (WhiteSpace + LineTerminator set relevant here).
fn trim_js(s: &[u16]) -> &[u16] {
    let is_ws = |c: u16| {
        matches!(
            c,
            0x0009 | 0x000A | 0x000B | 0x000C | 0x000D | 0x0020 | 0x00A0 | 0xFEFF
        )
    };
    let mut a = 0;
    let mut b = s.len();
    while a < b && is_ws(s[a]) {
        a += 1;
    }
    while b > a && is_ws(s[b - 1]) {
        b -= 1;
    }
    &s[a..b]
}

/// `Number.isInteger(Number(s))` for a UTF-16 string — used by
/// `isInteger` / `exceedsLimit` on range endpoint strings.
pub fn is_integer_str(s: &[u16]) -> bool {
    if trim_ascii_empty(s) {
        return false;
    }
    is_integer_num(to_number(s))
}

fn trim_ascii_empty(s: &[u16]) -> bool {
    // mirrors `num.trim() !== ''` — trims the same JS whitespace
    trim_js(s).is_empty()
}

/// JS `String(v)` for f64 — shortest round-trip with JS exponent rules:
/// plain decimal for 1e-6 <= |v| < 1e21, otherwise exponential (`1e+21`,
/// `1.5e-7`). NaN/Infinity/±0 handled.
pub fn num_to_string(v: f64) -> Vec<u16> {
    if v.is_nan() {
        return lit("NaN");
    }
    if v.is_infinite() {
        return lit(if v > 0.0 { "Infinity" } else { "-Infinity" });
    }
    if v == 0.0 {
        return lit("0"); // covers -0: JS String(-0) === "0"
    }
    let neg = v < 0.0;
    let a = v.abs();
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if a.fract() == 0.0 {
        // integer-valued
        if a < 1e21 {
            out.push_str(&format!("{a:.0}"));
            return out.encode_utf16().collect();
        }
        // exponential for >= 1e21
        let digits = format!("{a:.0}");
        let exp10 = (digits.len() - 1) as i32;
        let stripped = digits.trim_end_matches('0');
        let stripped = if stripped.is_empty() { "0" } else { stripped };
        push_exp(&mut out, stripped, exp10);
        return out.encode_utf16().collect();
    }
    // non-integers: use Rust's shortest repr, then apply JS notation rules
    let s = format!("{a}");
    if a >= 1e21 || a < 1e-6 {
        // convert decimal string to exponential
        let (digits, point) = decimal_parts(&s);
        push_exp(&mut out, &digits, point - 1);
    } else {
        out.push_str(&s);
    }
    out.encode_utf16().collect()
}

fn lit(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Split a Rust decimal float string ("123.456") into (digits, point_index)
/// where the value = 0.digits * 10^point_index.
fn decimal_parts(s: &str) -> (String, i32) {
    let mut digits = String::new();
    let mut point: i32 = 0;
    let mut seen_dot = false;
    for ch in s.chars() {
        if ch == '.' {
            seen_dot = true;
            continue;
        }
        if ch.is_ascii_digit() {
            if !seen_dot {
                point += 1;
            }
            digits.push(ch);
        }
    }
    // strip leading zeros
    let stripped = digits.trim_start_matches('0');
    let dropped = (digits.len() - stripped.len()) as i32;
    let mut d = stripped.to_string();
    if d.is_empty() {
        d.push('0');
    }
    // strip trailing zeros
    while d.len() > 1 && d.ends_with('0') {
        d.pop();
    }
    (d, point - dropped)
}

fn push_exp(out: &mut String, digits: &str, exp10: i32) {
    let mut it = digits.chars();
    out.push(it.next().unwrap_or('0'));
    let rest: String = it.collect();
    if !rest.is_empty() {
        out.push('.');
        out.push_str(&rest);
    }
    out.push('e');
    if exp10 < 0 {
        out.push('-');
        out.push_str(&(-exp10).to_string());
    } else {
        out.push('+');
        out.push_str(&exp10.to_string());
    }
}

/// JS `String.fromCharCode(v)`: ToUint16(v) as a single UTF-16 unit.
#[inline]
pub fn from_char_code(v: f64) -> u16 {
    // ToIntegerOrInfinity then modulo 2^16 (as unsigned).
    let t = if v.is_nan() || v.is_infinite() {
        0.0
    } else {
        v.trunc()
    };
    let m = t % 65536.0;
    (if m < 0.0 { m + 65536.0 } else { m }) as u16
}

/// `charCodeAt(0)` of a UTF-16 string.
#[inline]
pub fn char_code_at(s: &[u16]) -> u32 {
    s[0] as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn to_number_cases() {
        assert_eq!(to_number(&u("1")), 1.0);
        assert_eq!(to_number(&u("  42 ")), 42.0);
        assert_eq!(to_number(&u("")), 0.0);
        assert_eq!(to_number(&u("0x10")), 16.0);
        assert!(to_number(&u("abc")).is_nan());
        assert_eq!(to_number(&u("-0")), -0.0);
        assert!(to_number(&u("-0")) == 0.0);
        assert_eq!(to_number(&u("1e3")), 1000.0);
        assert_eq!(to_number(&u("1.5e-2")), 0.015);
        assert!(to_number(&u("0x")).is_nan());
        assert!(to_number(&u("-0x10")).is_nan());
    }

    #[test]
    fn num_string_cases() {
        assert_eq!(num_to_string(1.0), u("1"));
        assert_eq!(num_to_string(-0.0), u("0"));
        assert_eq!(num_to_string(9007199254740991.0), u("9007199254740991"));
        assert_eq!(num_to_string(1e21), u("1e+21"));
        assert_eq!(num_to_string(0.000001), u("0.000001"));
        assert_eq!(num_to_string(0.0000001), u("1e-7"));
        assert_eq!(num_to_string(100.0), u("100"));
    }
}
