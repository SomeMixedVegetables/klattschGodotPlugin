// (Public only so the parity tests can reach it; not part of the API.)
// Helpers that reproduce JavaScript semantics where Rust's built-ins differ.
// The port aims for bit-exact output against the JS engine, so anything that
// touches floating point or string classification goes through here.

// JS `/\s/`: WhiteSpace + LineTerminator. Unlike `char::is_whitespace`, this
// includes U+FEFF and excludes U+0085 and U+180E.
pub(crate) fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

// Math.max: NaN-propagating, and +0 beats -0.
pub(crate) fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == b {
        return if a == 0.0 && a.is_sign_negative() { b } else { a };
    }
    if a > b { a } else { b }
}

// Math.min: NaN-propagating, and -0 beats +0.
pub(crate) fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == b {
        return if a == 0.0 && a.is_sign_positive() { b } else { a };
    }
    if a < b { a } else { b }
}

// Math.round: ties round toward +infinity.
pub(crate) fn round(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    let f = x.floor();
    if x - f >= 0.5 { f + 1.0 } else { f }
}

// Math.sin / Math.cos: V8's own fdlibm port.
pub fn sin(x: f64) -> f64 {
    crate::fdlibm::sin(x)
}

pub fn cos(x: f64) -> f64 {
    crate::fdlibm::cos(x)
}

// Math.pow as V8 implements it (v8::internal::math::pow with the default
// --use-std-math-pow): the C library's pow plus a few special cases. Like
// Node, the result follows the platform's libm (powf calls it here).
pub fn pow(x: f64, y: f64) -> f64 {
    if y.is_nan() {
        return f64::NAN;
    }
    if y.is_infinite() && (x == 1.0 || x == -1.0) {
        return f64::NAN;
    }
    if y == 2.0 {
        return x * x;
    }
    if y == 0.5 {
        return if x.is_infinite() { f64::INFINITY } else { (x + 0.0).sqrt() };
    }
    x.powf(y)
}

// Number.prototype.toString() for radix 10.
pub(crate) fn number_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    let sign = if x < 0.0 { "-" } else { "" };
    // `{:e}` yields the shortest round-tripping digits, e.g. "1.25e-7".
    let sci = format!("{:e}", x.abs());
    let (mant, exp) = sci.split_once('e').expect("exponent form");
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exp.parse::<i32>().expect("exponent") + 1;
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let esign = if e < 0 { '-' } else { '+' };
        if k == 1 {
            format!("{digits}e{esign}{}", e.abs())
        } else {
            format!("{}.{}e{esign}{}", &digits[..1], &digits[1..], e.abs())
        }
    };
    format!("{sign}{body}")
}

// StringToNumber, as used by Number(str) and arithmetic coercion.
pub(crate) fn string_to_number(s: &str) -> f64 {
    let t = s.trim_matches(is_space);
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let radix = match t.get(..2) {
        Some("0x") | Some("0X") => 16,
        Some("0o") | Some("0O") => 8,
        Some("0b") | Some("0B") => 2,
        _ => 0,
    };
    if radix != 0 {
        let body = &t[2..];
        if body.is_empty() {
            return f64::NAN;
        }
        let mut acc = 0.0f64;
        for c in body.chars() {
            match c.to_digit(radix) {
                Some(d) => acc = acc * radix as f64 + d as f64,
                None => return f64::NAN,
            }
        }
        return acc;
    }
    if !is_decimal_literal(t) {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

// [+-]? ( digits [. digits?] | . digits ) ( [eE] [+-]? digits )?
fn is_decimal_literal(t: &str) -> bool {
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - int_start;
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let fs = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        frac_digits = i - fs;
    }
    if int_digits == 0 && frac_digits == 0 {
        return false;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let es = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == es {
            return false;
        }
    }
    i == b.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_to_string_matches_js() {
        let cases = [
            (1.0, "1"), (-1.5, "-1.5"), (0.1, "0.1"), (1e21, "1e+21"), (1e20, "100000000000000000000"),
            (1.5e-7, "1.5e-7"), (0.000001, "0.000001"), (123.456, "123.456"), (2.0f64.powi(60), "1152921504606847000"),
            (-0.0, "0"), (5e-324, "5e-324"),
            (f64::NAN, "NaN"), (f64::INFINITY, "Infinity"), (f64::NEG_INFINITY, "-Infinity"),
        ];
        for (x, s) in cases {
            assert_eq!(number_to_string(x), s, "{x}");
        }
    }

    #[test]
    fn string_to_number_matches_js() {
        assert_eq!(string_to_number(" 42 "), 42.0);
        assert_eq!(string_to_number(""), 0.0);
        assert_eq!(string_to_number("0x1F"), 31.0);
        assert_eq!(string_to_number(".5"), 0.5);
        assert_eq!(string_to_number("5."), 5.0);
        assert_eq!(string_to_number("-1e3"), -1000.0);
        assert!(string_to_number("inf").is_nan());
        assert!(string_to_number("NaN").is_nan());
        assert!(string_to_number("1_0").is_nan());
        assert!(string_to_number("-0x1").is_nan());
    }

    #[test]
    fn min_max_round() {
        assert!(max(1.0, f64::NAN).is_nan());
        assert!(max(-0.0, 0.0).is_sign_positive());
        assert!(min(0.0, -0.0).is_sign_negative());
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(-2.5), -2.0);
        assert_eq!(round(0.49999999999999994), 0.0);
    }
}
