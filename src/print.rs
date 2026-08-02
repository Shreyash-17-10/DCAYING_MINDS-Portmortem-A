//! JSON serialization/printing.
//! Source: cJSON.c (print_number, print_string_ptr, print_value, print_array,
//! print_object, cJSON_Print, cJSON_PrintUnformatted).
//!
//! Design notes (see DECISIONS.md):
//! - C's growable `printbuffer` (manual malloc/realloc via `ensure()`) has no
//!   Rust equivalent needed: we build directly into an owned `String`, which
//!   already amortizes growth. `cJSON_PrintBuffered`/`PrintPreallocated`
//!   (pure buffer-strategy variants with identical *output*) are intentionally
//!   not ported - there is nothing behaviorally different left to port.
//! - `cJSON_Raw` values are emitted byte-for-byte, unescaped, matching
//!   upstream: it's the caller's responsibility to hand in valid JSON.
//! - Printing can fail exactly one way: nesting >= CJSON_NESTING_LIMIT
//!   (mirrors print_array/print_object's `output_buffer->depth >=
//!   CJSON_NESTING_LIMIT` check, cJSON.c:1607, 1792). Everything else that
//!   can fail in C (allocation failure, sprintf overrun) has no Rust
//!   equivalent given our design, so `CJsonError::NestingTooDeep` is the only
//!   error this module can produce.

use crate::error::CJsonError;
use crate::parse::NESTING_LIMIT;
use crate::value::Value;

/// Pretty-printed output with tab indentation, mirrors `cJSON_Print`.
pub fn print(value: &Value) -> Result<String, CJsonError> {
    let mut out = String::new();
    print_value(value, &mut out, true, 0)?;
    Ok(out)
}

/// Compact output, no whitespace, mirrors `cJSON_PrintUnformatted`.
pub fn print_unformatted(value: &Value) -> Result<String, CJsonError> {
    let mut out = String::new();
    print_value(value, &mut out, false, 0)?;
    Ok(out)
}

/// Mirrors print_value (cJSON.c:1423-1483).
fn print_value(
    value: &Value,
    out: &mut String,
    format: bool,
    depth: usize,
) -> Result<(), CJsonError> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => out.push_str(&print_number(*n)),
        // Raw JSON fragment, emitted verbatim - mirrors the cJSON_Raw case
        // (memcpy of item->valuestring with no escaping, cJSON.c:1463-1476).
        Value::Raw(s) => out.push_str(s),
        Value::String(s) => print_string_ptr(s, out),
        Value::Array(items) => print_array(items, out, format, depth)?,
        Value::Object(pairs) => print_object(pairs, out, format, depth)?,
    }
    Ok(())
}

/// Mirrors print_array (cJSON.c:1595-1655): elements joined by `,` (plus a
/// space if `format`), with **no** newlines or indentation at the array
/// level itself - only nested objects get multi-line treatment. This looks
/// odd for e.g. an array of objects, but it's upstream's actual behavior.
fn print_array(
    items: &[Value],
    out: &mut String,
    format: bool,
    depth: usize,
) -> Result<(), CJsonError> {
    if depth >= NESTING_LIMIT {
        return Err(CJsonError::NestingTooDeep { pos: depth });
    }
    out.push('[');
    let depth = depth + 1;
    for (i, item) in items.iter().enumerate() {
        print_value(item, out, format, depth)?;
        if i + 1 < items.len() {
            out.push(',');
            if format {
                out.push(' ');
            }
        }
    }
    out.push(']');
    Ok(())
}

/// Mirrors print_object (cJSON.c:1780-1880): `{`, newline (if format), then
/// per entry `\t`*depth + `"key"` + `:` + (`\t` if format) + value + (`,` if
/// not last) + (`\n` if format), then `\t`*(depth-1) + `}`.
///
/// Quirk preserved from upstream: an *empty* formatted object still prints
/// as `{\n}` (the `\n` after `{` is unconditional), not `{}`.
fn print_object(
    pairs: &[(String, Value)],
    out: &mut String,
    format: bool,
    depth: usize,
) -> Result<(), CJsonError> {
    if depth >= NESTING_LIMIT {
        return Err(CJsonError::NestingTooDeep { pos: depth });
    }
    out.push('{');
    if format {
        out.push('\n');
    }
    let depth = depth + 1;
    for (i, (key, val)) in pairs.iter().enumerate() {
        if format {
out.extend(std::iter::repeat_n('\t', depth));
        }
        print_string_ptr(key, out);
        out.push(':');
        if format {
            out.push('\t');
        }
        print_value(val, out, format, depth)?;
        if i + 1 < pairs.len() {
            out.push(',');
        }
        if format {
            out.push('\n');
        }
    }
    if format {
out.extend(std::iter::repeat_n('\t', depth - 1));
    }
    out.push('}');
    Ok(())
}

/// Mirrors print_string_ptr (cJSON.c:954-1074): quote the string, escaping
/// the seven single-char escapes and any other control byte (< 0x20) as
/// `\u00XX`. Everything >= 0x20, including multi-byte UTF-8 sequences, is
/// copied through unchanged (upstream operates byte-wise and never
/// re-encodes valid UTF-8; iterating `char`s and re-pushing them is
/// equivalent since every byte < 0x20 is its own complete UTF-8 codepoint).
/// (The C `input == NULL` branch has no equivalent here: our `String` is
/// always present, never null.)
fn print_string_ptr(input: &str, out: &mut String) {
    out.push('"');
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Mirrors print_number (cJSON.c:596-651).
fn print_number(d: f64) -> String {
    if d.is_nan() || d.is_infinite() {
        return "null".to_string();
    }

    // Rust's float->int cast is saturating (post-1.45), which is exactly
    // the manual saturation cJSON_SetNumberValue does to populate the
    // deprecated `valueint` field (cJSON.c: `if (number >= INT_MAX) ...`).
    // So `d as i32` here reproduces `item->valueint` without needing to
    // store it separately.
    let as_int = d as i32;
    if d == f64::from(as_int) {
        return as_int.to_string();
    }

    // Try 15 significant digits; if that doesn't round-trip back to the
    // exact same f64, fall back to 17 (cJSON.c:619-630).
    let s15 = format_g(d, 15);
    if let Ok(parsed) = s15.parse::<f64>() {
        if compare_double(parsed, d) {
            return s15;
        }
    }
    format_g(d, 17)
}

/// Mirrors compare_double (cJSON.c:589-593): relative-epsilon comparison,
/// not bit-exact equality.
fn compare_double(a: f64, b: f64) -> bool {
    let max_val = a.abs().max(b.abs());
    (a - b).abs() <= max_val * f64::EPSILON
}

/// Formats `d` (assumed finite, nonzero) the way C's `%1.<sig_digits>g`
/// would: `sig_digits` significant digits, fixed notation when the decimal
/// exponent is in `[-4, sig_digits)`, scientific notation otherwise, with
/// trailing zeros (and a bare trailing '.') stripped in both cases - the
/// standard `%g` behavior (no `#` flag).
fn format_g(d: f64, sig_digits: usize) -> String {
    if d == 0.0 {
        return "0".to_string();
    }
    let neg = d.is_sign_negative();
    let abs_d = d.abs();

    // Correctly-rounded scientific form with exactly `sig_digits` significant
    // digits, e.g. "3.14159265358979e0". Rust's `{:e}` mantissa is always a
    // single nonzero leading digit (matching %e/%g convention).
    let sci = format!("{:.*e}", sig_digits - 1, abs_d);
    let (mantissa_part, exp_part) = sci
        .split_once('e')
        .expect("float Display always includes 'e'");
    let exponent: i32 = exp_part
        .parse()
        .expect("exponent is always a plain integer");
    let digits: String = mantissa_part.chars().filter(|c| *c != '.').collect();

    let use_scientific = exponent < -4 || exponent >= sig_digits as i32;

    let body = if use_scientific {
        let (first, rest) = digits.split_at(1);
        let rest_trimmed = rest.trim_end_matches('0');
        let mantissa = if rest_trimmed.is_empty() {
            first.to_string()
        } else {
            format!("{}.{}", first, rest_trimmed)
        };
        let exp_sign = if exponent < 0 { '-' } else { '+' };
        format!("{}e{}{:02}", mantissa, exp_sign, exponent.abs())
    } else if exponent >= 0 {
        let int_len = (exponent + 1) as usize;
        let (int_part, frac_part) = digits.split_at(int_len);
        let frac_trimmed = frac_part.trim_end_matches('0');
        if frac_trimmed.is_empty() {
            int_part.to_string()
        } else {
            format!("{}.{}", int_part, frac_trimmed)
        }
    } else {
        let leading_zeros = "0".repeat((-exponent - 1) as usize);
        let frac = format!("{}{}", leading_zeros, digits);
        let frac_trimmed = frac.trim_end_matches('0');
        format!(
            "0.{}",
            if frac_trimmed.is_empty() {
                "0"
            } else {
                frac_trimmed
            }
        )
    };

    if neg {
        format!("-{}", body)
    } else {
        body
    }
}
#[cfg(test)]
#[allow(clippy::approx_constant, clippy::excessive_precision)]
mod tests {

    use super::*;

    // --- print_number: vectors lifted directly from upstream's
    // tests/print_number.c (assert_print_number cases) ---

    #[test]
    fn number_zero() {
        assert_eq!(print_number(0.0), "0");
    }

    #[test]
    fn number_negative_integers() {
        assert_eq!(print_number(-1.0), "-1");
        assert_eq!(print_number(-32768.0), "-32768");
        assert_eq!(print_number(-2147483648.0), "-2147483648");
    }

    #[test]
    fn number_positive_integers() {
        assert_eq!(print_number(1.0), "1");
        assert_eq!(print_number(32767.0), "32767");
        assert_eq!(print_number(2147483647.0), "2147483647");
    }

    #[test]
    // This 17-significant-digit literal is deliberate: it's the value
    // used to exercise print_number's 17-digit fallback path (the case
    // where 15 digits doesn't round-trip exactly, see cJSON.c:622-630 and
    // this port's own doc comment on print_number). Truncating it, as
    // clippy's excessive_precision lint suggests, would test a different
    // code path than the one this test is named for.
    #[allow(clippy::approx_constant, clippy::excessive_precision)]
    fn number_positive_reals() {
        assert_eq!(print_number(0.123), "0.123");
        assert_eq!(print_number(10e-10), "1e-09");
        assert_eq!(print_number(10e11), "1000000000000");
        assert_eq!(print_number(123e+127), "1.23e+129");
        assert_eq!(print_number(123e-128), "1.23e-126");
        let value = "3.1415926535897931".parse::<f64>().unwrap();
        assert_eq!(print_number(value), "3.1415926535897931");
    }
    #[test]
    fn number_negative_reals() {
        assert_eq!(print_number(-0.0123), "-0.0123");
        assert_eq!(print_number(-10e-10), "-1e-09");
        assert_eq!(print_number(-10e20), "-1e+21");
        assert_eq!(print_number(-123e+127), "-1.23e+129");
        assert_eq!(print_number(-123e-128), "-1.23e-126");
    }

    #[test]
    fn number_nan_and_infinity_print_null() {
        assert_eq!(print_number(f64::NAN), "null");
        assert_eq!(print_number(f64::INFINITY), "null");
        assert_eq!(print_number(f64::NEG_INFINITY), "null");
    }

    // --- print_string_ptr: vectors from upstream's tests/print_string.c ---

    #[test]
    fn string_empty() {
        let mut out = String::new();
        print_string_ptr("", &mut out);
        assert_eq!(out, "\"\"");
    }

    #[test]
    fn string_ascii_control_and_printable() {
        let ascii: String = (1u8..0x7F).map(|b| b as char).collect();
        let mut out = String::new();
        print_string_ptr(&ascii, &mut out);
        let expected = "\"\\u0001\\u0002\\u0003\\u0004\\u0005\\u0006\\u0007\\b\\t\\n\\u000b\\f\\r\\u000e\\u000f\\u0010\\u0011\\u0012\\u0013\\u0014\\u0015\\u0016\\u0017\\u0018\\u0019\\u001a\\u001b\\u001c\\u001d\\u001e\\u001f !\\\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\\\]^_`abcdefghijklmnopqrstuvwxyz{|}~\"";
        assert_eq!(out, expected);
    }

    #[test]
    fn string_utf8_passthrough() {
        let mut out = String::new();
        print_string_ptr("ü猫慕", &mut out);
        assert_eq!(out, "\"ü猫慕\"");
    }

    // --- structural: array/object formatting ---

    #[test]
    fn unformatted_object_and_array() {
        let v = Value::Object(vec![
            ("a".to_string(), Value::Number(1.0)),
            (
                "b".to_string(),
                Value::Array(vec![Value::Number(2.0), Value::Number(3.0)]),
            ),
        ]);
        assert_eq!(print_unformatted(&v).unwrap(), r#"{"a":1,"b":[2,3]}"#);
    }

    #[test]
    fn formatted_object_uses_tabs_and_newlines() {
        let v = Value::Object(vec![
            ("a".to_string(), Value::Number(1.0)),
            ("b".to_string(), Value::Bool(true)),
        ]);
        assert_eq!(print(&v).unwrap(), "{\n\t\"a\":\t1,\n\t\"b\":\ttrue\n}");
    }

    #[test]
    fn formatted_empty_object_is_brace_newline_brace() {
        // Preserves the upstream quirk: the '\n' after '{' is unconditional.
        assert_eq!(print(&Value::Object(vec![])).unwrap(), "{\n}");
    }

    #[test]
    fn formatted_array_has_no_internal_newlines() {
        let v = Value::Array(vec![Value::Number(1.0), Value::Number(2.0)]);
        assert_eq!(print(&v).unwrap(), "[1, 2]");
    }

    #[test]
    fn nested_formatted_object_increases_indentation() {
        let v = Value::Object(vec![(
            "outer".to_string(),
            Value::Object(vec![("inner".to_string(), Value::Number(1.0))]),
        )]);
        assert_eq!(
            print(&v).unwrap(),
            "{\n\t\"outer\":\t{\n\t\t\"inner\":\t1\n\t}\n}"
        );
    }

    #[test]
    fn raw_is_emitted_verbatim_unescaped() {
        let v = Value::Array(vec![Value::Raw("{\"already\":\"json\"}".to_string())]);
        assert_eq!(print_unformatted(&v).unwrap(), r#"[{"already":"json"}]"#);
    }

    #[test]
    fn rejects_printing_past_nesting_limit() {
        // Build an array nested NESTING_LIMIT+1 deep.
        let mut v = Value::Array(vec![]);
        for _ in 0..NESTING_LIMIT {
            v = Value::Array(vec![v]);
        }
        assert!(matches!(print(&v), Err(CJsonError::NestingTooDeep { .. })));
    }
}
