//! JSON parsing primitives: numbers and strings.
//! This module contains number, string, array, object, and value parsing.
//!
//! Source mapping:
//! - `parse_number` (cJSON.c:307)      -> `parse_number` below
//! - `parse_hex4` (cJSON.c:666)        -> `parse_hex4` below
//! - `utf16_literal_to_utf8` (cJSON.c:703) -> `parse_unicode_escape` below
//! - `parse_string` (cJSON.c:824)      -> `parse_string_literal` below
//!
//! Design notes (see DECISIONS.md):
//! - C's `strtod` is locale-dependent (it swaps '.' for the current locale's
//!   decimal point, cJSON.c:312). Rust's `f64::from_str` is always '.'-based
//!   regardless of locale. This is treated as an intentional improvement, not
//!   a bug to replicate — cJSON's locale-swap behavior is a known footgun
//!   upstream. Flagged in DECISIONS.md.
//! - No `unsafe`: scanning is done over `&[u8]` with bounds-checked indexing,
//!   unicode escapes decoded via `char::from_u32` instead of manual UTF-8
//!   byte-packing (cJSON.c:766-817).

use crate::error::CJsonError;

/// Scans a JSON number at the start of `input` and returns `(value, bytes_consumed)`.
/// Returns `None` if no valid number could be scanned (mirrors parse_number
/// returning `false` when strtod consumes zero characters).
///
/// Only scans the character classes cJSON scans (digits, '+', '-', 'e', 'E', '.')
/// before handing off to the float parser, matching cJSON.c:325-354.
pub fn parse_number(input: &[u8]) -> Option<(f64, usize)> {
    let mut scan_len = 0usize;
    for &b in input {
        match b {
            b'0'..=b'9' | b'+' | b'-' | b'e' | b'E' | b'.' => scan_len += 1,
            _ => break,
        }
    }

    if scan_len == 0 {
        return None;
    }

    // cJSON hands the *entire* scanned run to strtod, but only requires
    // strtod to consume a non-empty *prefix* of it (cJSON.c:376-382:
    // `if (number_c_string == after_end) return false;` - note this does
    // NOT check that after_end reached the end of the buffer). Anything
    // strtod stops at is left unconsumed for the caller. E.g. "1.2.3" scans
    // as a 5-byte run (both '.'s are in the character class), but strtod
    // only parses "1.2" and stops at the second '.', so cJSON yields the
    // number 1.2 and advances the input position by 3, not 5.
    //
    // Rust's f64::from_str requires the *whole* string to be a valid
    // number, so to match, we search for the longest valid prefix of the
    // scanned run ourselves rather than parsing it all-or-nothing.
    let s = std::str::from_utf8(&input[..scan_len]).ok()?;
    for len in (1..=scan_len).rev() {
        // s is pure ASCII (digits/+/-/e/E/.), so byte index == char boundary.
        if let Ok(value) = s[..len].parse::<f64>() {
            return Some((value, len));
        }
    }
    None
}

/// Parses 4 hex digits into a u16, mirrors parse_hex4 (cJSON.c:666).
/// Returns `None` on any non-hex-digit character (C returns 0 for both a
/// genuinely invalid sequence and a valid `\u0000`; we disambiguate with
/// `Option` since Rust doesn't need the overloaded sentinel).
fn parse_hex4(bytes: &[u8]) -> Option<u16> {
    if bytes.len() < 4 {
        return None;
    }
    let mut h: u16 = 0;
    for &b in &bytes[..4] {
        let nibble = match b {
            b'0'..=b'9' => b - b'0',
            b'A'..=b'F' => 10 + (b - b'A'),
            b'a'..=b'f' => 10 + (b - b'a'),
            _ => return None,
        };
        h = (h << 4) | nibble as u16;
    }
    Some(h)
}

/// Decodes a `\uXXXX` or `\uXXXX\uXXXX` (surrogate pair) escape starting at
/// `bytes[0] == '\\'`, `bytes[1] == 'u'`. Returns `(char, bytes_consumed)`
/// where `bytes_consumed` is 6 for a single unit or 12 for a surrogate pair.
/// Mirrors utf16_literal_to_utf8 (cJSON.c:703-821), including its rejection
/// of unpaired low surrogates and malformed second halves.
fn parse_unicode_escape(bytes: &[u8], pos: usize) -> Result<(char, usize), CJsonError> {
    if bytes.len() < 6 {
        return Err(CJsonError::UnterminatedString { pos });
    }
    let first = parse_hex4(&bytes[2..6]).ok_or(CJsonError::InvalidUnicodeEscape { pos })?;

    // Unpaired low surrogate is always invalid (cJSON.c:722-726).
    if (0xDC00..=0xDFFF).contains(&first) {
        return Err(CJsonError::InvalidUnicodeEscape { pos });
    }

    // High surrogate: require a second \uXXXX forming a valid pair.
    if (0xD800..=0xDBFF).contains(&first) {
        if bytes.len() < 12 || bytes[6] != b'\\' || bytes[7] != b'u' {
            return Err(CJsonError::InvalidUnicodeEscape { pos });
        }
        let second = parse_hex4(&bytes[8..12]).ok_or(CJsonError::InvalidUnicodeEscape { pos })?;
        if !(0xDC00..=0xDFFF).contains(&second) {
            return Err(CJsonError::InvalidUnicodeEscape { pos });
        }
        let codepoint = 0x10000u32 + (((first as u32 & 0x3FF) << 10) | (second as u32 & 0x3FF));
        let ch = char::from_u32(codepoint).ok_or(CJsonError::InvalidUnicodeEscape { pos })?;
        return Ok((ch, 12));
    }

    let ch = char::from_u32(first as u32).ok_or(CJsonError::InvalidUnicodeEscape { pos })?;
    Ok((ch, 6))
}

/// Parses a quoted JSON string literal starting at `input[0] == '"'`.
/// Returns `(unescaped_value, bytes_consumed)` including both quote characters.
/// Mirrors parse_string (cJSON.c:824-951).
pub fn parse_string_literal(input: &[u8]) -> Result<(String, usize), CJsonError> {
    if input.is_empty() || input[0] != b'"' {
        return Err(CJsonError::InvalidString { pos: 0 });
    }

    let mut out = String::new();
    let mut i = 1usize;

    loop {
        if i >= input.len() {
            return Err(CJsonError::UnterminatedString { pos: i });
        }

        match input[i] {
            b'"' => {
                i += 1;
                return Ok((out, i));
            }
            b'\\' => {
                if i + 1 >= input.len() {
                    return Err(CJsonError::UnterminatedString { pos: i });
                }
                match input[i + 1] {
                    b'b' => {
                        out.push('\u{8}');
                        i += 2;
                    }
                    b'f' => {
                        out.push('\u{c}');
                        i += 2;
                    }
                    b'n' => {
                        out.push('\n');
                        i += 2;
                    }
                    b'r' => {
                        out.push('\r');
                        i += 2;
                    }
                    b't' => {
                        out.push('\t');
                        i += 2;
                    }
                    b'"' => {
                        out.push('"');
                        i += 2;
                    }
                    b'\\' => {
                        out.push('\\');
                        i += 2;
                    }
                    b'/' => {
                        out.push('/');
                        i += 2;
                    }
                    b'u' => {
                        let (ch, consumed) = parse_unicode_escape(&input[i..], i)?;
                        out.push(ch);
                        i += consumed;
                    }
                    _ => return Err(CJsonError::InvalidEscape { pos: i }),
                }
            }
            _ => {
                // Batch a run of plain (non-quote, non-backslash) bytes and
                // decode as UTF-8 in one go, rather than byte-by-byte.
                let start = i;
                while i < input.len() && input[i] != b'"' && input[i] != b'\\' {
                    i += 1;
                }
                let chunk = std::str::from_utf8(&input[start..i])
                    .map_err(|_| CJsonError::InvalidUtf8 { pos: start })?;
                out.push_str(chunk);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;

    // --- parse_number ---

    #[test]
    fn number_integers() {
        assert_eq!(parse_number(b"0"), Some((0.0, 1)));
        assert_eq!(parse_number(b"123"), Some((123.0, 3)));
        assert_eq!(parse_number(b"-123"), Some((-123.0, 4)));
    }

    #[test]
    fn number_decimals_and_exponents() {
        assert_eq!(parse_number(b"1.5"), Some((1.5, 3)));
        assert_eq!(parse_number(b"3.1416"), Some((3.1416, 6)));
        assert_eq!(parse_number(b"1E10"), Some((1e10, 4)));
        assert_eq!(parse_number(b"1e-10"), Some((1e-10, 5)));
        assert_eq!(parse_number(b"1.234E+10"), Some((1.234e10, 9)));
    }

    #[test]
    fn number_stops_at_non_number_chars() {
        // Only "123" should be consumed; trailing text is left for the caller.
        assert_eq!(parse_number(b"123abc"), Some((123.0, 3)));
    }

    #[test]
    fn number_rejects_non_numeric_input() {
        assert_eq!(parse_number(b"abc"), None);
        assert_eq!(parse_number(b""), None);
    }

    // --- partial-consumption regressions (matches cJSON's strtod-based
    // "longest prefix wins" behavior, not all-or-nothing) ---

    #[test]
    fn number_second_decimal_point_stops_the_number() {
        // C: strtod("1.2.3") consumes "1.2", leaves ".3" for the caller.
        assert_eq!(parse_number(b"1.2.3"), Some((1.2, 3)));
    }

    #[test]
    fn number_dangling_exponent_falls_back_to_mantissa() {
        // "1e" has no exponent digits, so strtod (and our prefix search)
        // back off to just "1".
        assert_eq!(parse_number(b"1e"), Some((1.0, 1)));
    }

    #[test]
    fn number_mid_string_minus_stops_the_number() {
        // '-' is in the scan class even mid-string; strtod only consumes "1".
        assert_eq!(parse_number(b"1-2"), Some((1.0, 1)));
    }

    #[test]
    fn number_trailing_decimal_point_is_valid() {
        assert_eq!(parse_number(b"5."), Some((5.0, 2)));
    }

    // --- parse_string_literal ---

    #[test]
    fn string_basic() {
        assert_eq!(
            parse_string_literal(b"\"hello\"").unwrap(),
            ("hello".to_string(), 7)
        );
    }

    #[test]
    fn string_escapes() {
        assert_eq!(parse_string_literal(b"\"a\\nb\"").unwrap().0, "a\nb");
        assert_eq!(
            parse_string_literal(b"\"\\t\\r\\b\\f\"").unwrap().0,
            "\t\r\u{8}\u{c}"
        );
        assert_eq!(parse_string_literal(b"\"\\\"\\\\\\/\"").unwrap().0, "\"\\/");
    }

    #[test]
    fn string_unicode_escape() {
        // \u00e9 = 'é'
        assert_eq!(parse_string_literal(b"\"\\u00e9\"").unwrap().0, "é");
    }

    #[test]
    fn string_surrogate_pair() {
        // \ud83d\ude00 = U+1F600 (grinning face emoji)
        let (s, consumed) = parse_string_literal(b"\"\\ud83d\\ude00\"").unwrap();
        assert_eq!(s, "\u{1F600}");
        assert_eq!(consumed, 14); // opening quote(1) + \ud83d(6) + \ude00(6) + closing quote(1)
    }

    #[test]
    fn string_unpaired_low_surrogate_is_error() {
        let err = parse_string_literal(b"\"\\udc00\"").unwrap_err();
        assert!(matches!(err, CJsonError::InvalidUnicodeEscape { .. }));
    }

    #[test]
    fn string_unterminated_is_error() {
        let err = parse_string_literal(b"\"abc").unwrap_err();
        assert!(matches!(err, CJsonError::UnterminatedString { .. }));
    }

    #[test]
    fn string_trailing_backslash_is_error() {
        let err = parse_string_literal(b"\"abc\\").unwrap_err();
        assert!(matches!(err, CJsonError::UnterminatedString { .. }));
    }

    #[test]
    fn string_invalid_escape_is_error() {
        let err = parse_string_literal(b"\"\\q\"").unwrap_err();
        assert!(matches!(err, CJsonError::InvalidEscape { .. }));
    }

    #[test]
    fn string_not_starting_with_quote_is_error() {
        let err = parse_string_literal(b"abc\"").unwrap_err();
        assert!(matches!(err, CJsonError::InvalidString { .. }));
    }
}

// ============================================================================
// Phase 3: value / array / object parsing (cJSON.c: parse_value, parse_array,
// parse_object, cJSON_Parse, cJSON_ParseWithOpts, buffer_skip_whitespace,
// skip_utf8_bom).
// ============================================================================

use crate::value::Value;

/// Maximum array/object nesting depth, mirrors CJSON_NESTING_LIMIT (cJSON.h:137).
pub const NESTING_LIMIT: usize = 1000;

/// Recursive-descent parser over a byte slice. Mirrors cJSON's `parse_buffer`
/// (content + offset + length + depth), but as owned local state rather than
/// a struct threaded through free functions and mutated via pointers.
struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a [u8]) -> Self {
        Parser {
            input,
            pos: 0,
            depth: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn starts_with(&self, needle: &[u8]) -> bool {
        self.input[self.pos..].starts_with(needle)
    }

    /// Skips bytes <= 32 (space and all C0 control characters), matching
    /// cJSON's `buffer_at_offset(buffer)[0] <= 32` check verbatim
    /// (buffer_skip_whitespace, cJSON.c:1090-1113) rather than Rust's
    /// narrower `char::is_whitespace`.
    fn skip_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            if b <= 32 {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// Skips a leading UTF-8 byte-order-mark, mirrors skip_utf8_bom (cJSON.c:1116-1129).
    fn skip_bom(&mut self) {
        if self.pos == 0 && self.starts_with(b"\xEF\xBB\xBF") {
            self.pos += 3;
        }
    }

    /// Re-bases an error position produced by a sub-slice parse (e.g.
    /// `parse_string_literal(&self.input[self.pos..])`) back onto the full
    /// input's coordinate space.
    fn offset_error(&self, err: CJsonError) -> CJsonError {
        let base = self.pos;
        match err {
            CJsonError::InvalidString { pos } => CJsonError::InvalidString { pos: base + pos },
            CJsonError::UnterminatedString { pos } => {
                CJsonError::UnterminatedString { pos: base + pos }
            }
            CJsonError::InvalidEscape { pos } => CJsonError::InvalidEscape { pos: base + pos },
            CJsonError::InvalidUnicodeEscape { pos } => {
                CJsonError::InvalidUnicodeEscape { pos: base + pos }
            }
            CJsonError::InvalidUtf8 { pos } => CJsonError::InvalidUtf8 { pos: base + pos },
            CJsonError::InvalidNumber { pos } => CJsonError::InvalidNumber { pos: base + pos },
            other => other,
        }
    }

    /// Mirrors parse_value (cJSON.c:1368-1420): tries null/false/true
    /// literals, then string, then number, then array, then object, in the
    /// same order as upstream.
    fn parse_value(&mut self) -> Result<Value, CJsonError> {
        match self.peek() {
            None => Err(CJsonError::UnexpectedEnd { pos: self.pos }),
            Some(b'n') if self.starts_with(b"null") => {
                self.pos += 4;
                Ok(Value::Null)
            }
            Some(b'f') if self.starts_with(b"false") => {
                self.pos += 5;
                Ok(Value::Bool(false))
            }
            Some(b't') if self.starts_with(b"true") => {
                self.pos += 4;
                Ok(Value::Bool(true))
            }
            Some(b'"') => {
                let (s, len) = parse_string_literal(&self.input[self.pos..])
                    .map_err(|e| self.offset_error(e))?;
                self.pos += len;
                Ok(Value::String(s))
            }
            Some(b'-') | Some(b'0'..=b'9') => {
                let (n, len) = parse_number(&self.input[self.pos..])
                    .ok_or(CJsonError::InvalidNumber { pos: self.pos })?;
                self.pos += len;
                Ok(Value::Number(n))
            }
            Some(b'[') => self.parse_array(),
            Some(b'{') => self.parse_object(),
            Some(_) => Err(CJsonError::UnexpectedToken { pos: self.pos }),
        }
    }

    /// Mirrors parse_array (cJSON.c:1497-1592).
    fn parse_array(&mut self) -> Result<Value, CJsonError> {
        if self.depth >= NESTING_LIMIT {
            return Err(CJsonError::NestingTooDeep { pos: self.pos });
        }
        self.depth += 1;

        self.pos += 1; // consume '['
        self.skip_whitespace();

        let mut items = Vec::new();

        if self.peek() == Some(b']') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(Value::Array(items));
        }

        loop {
            self.skip_whitespace();
            items.push(self.parse_value()?);
            self.skip_whitespace();

            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => {
                    self.depth -= 1;
                    return Err(CJsonError::ExpectedCommaOrClose { pos: self.pos });
                }
            }
        }

        self.depth -= 1;
        Ok(Value::Array(items))
    }

    /// Mirrors parse_object (cJSON.c:1662-1765), including the
    /// name-then-colon-then-value structure. Note: unlike the C version,
    /// which parses the key via `parse_string` into a temporary item and
    /// then swaps `valuestring`/`string`, we parse the key directly into a
    /// `String` since there's no shared item struct to repurpose.
    fn parse_object(&mut self) -> Result<Value, CJsonError> {
        if self.depth >= NESTING_LIMIT {
            return Err(CJsonError::NestingTooDeep { pos: self.pos });
        }
        self.depth += 1;

        self.pos += 1; // consume '{'
        self.skip_whitespace();

        let mut pairs = Vec::new();

        if self.peek() == Some(b'}') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(Value::Object(pairs));
        }

        loop {
            self.skip_whitespace();

            if self.peek() != Some(b'"') {
                self.depth -= 1;
                return Err(CJsonError::ExpectedObjectKey { pos: self.pos });
            }
            let (key, len) =
                parse_string_literal(&self.input[self.pos..]).map_err(|e| self.offset_error(e))?;
            self.pos += len;
            self.skip_whitespace();

            if self.peek() != Some(b':') {
                self.depth -= 1;
                return Err(CJsonError::ExpectedColon { pos: self.pos });
            }
            self.pos += 1;
            self.skip_whitespace();

            let value = self.parse_value()?;
            pairs.push((key, value));
            self.skip_whitespace();

            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => {
                    self.depth -= 1;
                    return Err(CJsonError::ExpectedCommaOrClose { pos: self.pos });
                }
            }
        }

        self.depth -= 1;
        Ok(Value::Object(pairs))
    }
}

/// Parses a single JSON value, returning `(value, bytes_consumed)`.
/// Trailing content after the value is left unconsumed for the caller to
/// inspect (see `parse` vs `parse_strict`).
fn parse_with_end(input: &[u8]) -> Result<(Value, usize), CJsonError> {
    let mut parser = Parser::new(input);
    parser.skip_bom();
    parser.skip_whitespace();
    let value = parser.parse_value()?;
    Ok((value, parser.pos))
}

/// Permissive top-level parse, mirrors `cJSON_Parse` (cJSON.c:1227-1230),
/// which calls `cJSON_ParseWithOpts(value, 0, /* require_null_terminated */ 0)`.
/// Trailing bytes after a valid value — including trailing garbage — are
/// silently ignored, matching upstream's default behavior exactly.
pub fn parse(input: &str) -> Result<Value, CJsonError> {
    parse_with_end(input.as_bytes()).map(|(value, _consumed)| value)
}

/// Strict top-level parse, mirrors `cJSON_ParseWithOpts(value, NULL, 1)`
/// (require_null_terminated=true, cJSON.c:1179-1186): after the value,
/// only whitespace may remain. Anything else is a `TrailingCharacters` error.
pub fn parse_strict(input: &str) -> Result<Value, CJsonError> {
    let bytes = input.as_bytes();
    let (value, mut end) = parse_with_end(bytes)?;
    while end < bytes.len() && bytes[end] <= 32 {
        end += 1;
    }
    if end != bytes.len() {
        return Err(CJsonError::TrailingCharacters { pos: end });
    }
    Ok(value)
}

#[cfg(test)]
#[allow(clippy::approx_constant)]
mod value_parser_tests {
    use super::*;

    #[test]
    fn parses_literals() {
        assert_eq!(parse("null").unwrap(), Value::Null);
        assert_eq!(parse("true").unwrap(), Value::Bool(true));
        assert_eq!(parse("false").unwrap(), Value::Bool(false));
    }

    #[test]
    fn parses_scalar_string_and_number() {
        assert_eq!(parse("\"hi\"").unwrap(), Value::String("hi".to_string()));
        assert_eq!(parse("42").unwrap(), Value::Number(42.0));

        let expected = "-3.14".parse::<f64>().unwrap();
        assert_eq!(parse("-3.14").unwrap(), Value::Number(expected));
    }

    #[test]
    fn parses_empty_array_and_object() {
        assert_eq!(parse("[]").unwrap(), Value::Array(vec![]));
        assert_eq!(parse("{}").unwrap(), Value::Object(vec![]));
    }

    #[test]
    fn parses_nested_array() {
        let v = parse("[1, 2, [3, 4], null]").unwrap();
        assert_eq!(
            v,
            Value::Array(vec![
                Value::Number(1.0),
                Value::Number(2.0),
                Value::Array(vec![Value::Number(3.0), Value::Number(4.0)]),
                Value::Null,
            ])
        );
    }

    #[test]
    fn parses_object_with_nested_value() {
        let v = parse(r#"{"a": 1, "b": {"c": true}}"#).unwrap();
        assert!(v.is_object());
        assert_eq!(v.object_get("a").unwrap().as_f64(), Some(1.0));
        let b = v.object_get("b").unwrap();
        assert_eq!(b.object_get("c").unwrap(), &Value::Bool(true));
    }

    #[test]
    fn skips_bom_and_whitespace() {
        let input = "\u{FEFF}  \n\t {\"x\": 1}  ";
        assert!(parse(input).unwrap().is_object());
    }

    #[test]
    fn readme_style_example() {
        // Mirrors the shape of cJSON's README.md monitor example.
        let json = r#"
        {
            "quiet": true,
            "sensors": ["motion", "light"],
            "threshold": 42.5
        }
        "#;
        let v = parse(json).unwrap();
        assert_eq!(v.object_get("quiet").unwrap(), &Value::Bool(true));
        assert_eq!(v.object_get("threshold").unwrap().as_f64(), Some(42.5));
        assert_eq!(v.object_get("sensors").unwrap().array_len(), Some(2));
    }

    #[test]
    fn permissive_parse_ignores_trailing_garbage() {
        // Matches cJSON_Parse's actual (permissive) behavior: only "123" is
        // consumed, "abc" is silently ignored.
        assert_eq!(parse("123abc").unwrap(), Value::Number(123.0));
    }

    #[test]
    fn strict_parse_rejects_trailing_garbage() {
        let err = parse_strict("123abc").unwrap_err();
        assert!(matches!(err, CJsonError::TrailingCharacters { .. }));
    }

    #[test]
    fn strict_parse_allows_trailing_whitespace() {
        assert_eq!(parse_strict("123   \n").unwrap(), Value::Number(123.0));
    }

    #[test]
    fn rejects_unterminated_array() {
        let err = parse("[1, 2").unwrap_err();
        assert!(
            matches!(err, CJsonError::UnexpectedEnd { .. })
                || matches!(err, CJsonError::ExpectedCommaOrClose { .. })
        );
    }

    #[test]
    fn rejects_object_missing_colon() {
        let err = parse(r#"{"a" 1}"#).unwrap_err();
        assert!(matches!(err, CJsonError::ExpectedColon { .. }));
    }

    #[test]
    fn rejects_object_non_string_key() {
        let err = parse("{1: 2}").unwrap_err();
        assert!(matches!(err, CJsonError::ExpectedObjectKey { .. }));
    }

    #[test]
    fn rejects_garbage_value() {
        let err = parse("nul").unwrap_err();
        assert!(matches!(err, CJsonError::UnexpectedToken { .. }));
    }

    #[test]
    fn rejects_deeply_nested_arrays() {
        let deep = "[".repeat(NESTING_LIMIT + 1) + &"]".repeat(NESTING_LIMIT + 1);
        let err = parse(&deep).unwrap_err();
        assert!(matches!(err, CJsonError::NestingTooDeep { .. }));
    }

    #[test]
    fn accepts_exactly_at_nesting_limit() {
        let deep = "[".repeat(NESTING_LIMIT) + &"]".repeat(NESTING_LIMIT);
        assert!(parse(&deep).is_ok());
    }
}

// ============================================================================
// Phase 5: cJSON_Minify. Strips insignificant whitespace and `//`/`/* */`
// comments from raw JSON *text*, leaving string literals untouched. This is
// a standalone text transform, independent of the `Value` parser above -
// upstream runs it as a byte-level pre-pass, not a parse-then-reprint.
// ============================================================================

/// Mirrors `cJSON_Minify` (cJSON.c:2924-2957) plus its `skip_oneline_comment`
/// / `skip_multiline_comment` / `minify_string` helpers.
///
/// Known upstream quirk, faithfully preserved rather than fixed: inside a
/// string, the closing-quote scan only looks one character ahead for `\"`.
/// A literal backslash immediately followed by the *real* closing quote
/// (source text `"\\"`, i.e. a string containing one backslash) is
/// misread as an escaped quote, so the scanner overruns into what should be
/// text *after* the string. This is upstream's actual behavior, not
/// something introduced in this port - see DECISIONS.md.
pub fn minify(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;

    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1; // consume the newline itself
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < bytes.len() && !(bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 2; // consume the closing "*/"
                }
                // If unterminated, upstream's loop runs to the input's
                // '\0' and returns with nothing appended for the rest -
                // mirrored here by simply falling off the end.
            }
            b'/' => {
                // Lone '/', not a comment opener: upstream's `else { json++; }`
                // branch advances past it *without* writing to `into` - so a
                // standalone '/' is silently dropped, not preserved. Quirk
                // preserved here for behavioral equivalence.
                i += 1;
            }
            b'"' => {
                // minify_string: copy the string literal verbatim,
                // including the naive one-lookahead-char `\"` handling
                // described above.
                out.push(b'"');
                i += 1;
                while i < bytes.len() {
                    let b = bytes[i];
                    out.push(b);
                    if b == b'"' {
                        i += 1;
                        break;
                    } else if b == b'\\' && bytes.get(i + 1) == Some(&b'"') {
                        out.push(b'"');
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }

    // Input is guaranteed valid UTF-8 (it came from a `&str`); we only ever
    // copy whole bytes through unchanged or drop ASCII whitespace/comment
    // bytes, so the result is still valid UTF-8.
    String::from_utf8(out).expect("minify only removes ASCII bytes from valid UTF-8 input")
}

#[cfg(test)]
mod minify_tests {
    use super::*;

    #[test]
    fn strips_insignificant_whitespace() {
        assert_eq!(minify(" { \"a\" : 1 ,\n\t\"b\": 2 } "), r#"{"a":1,"b":2}"#);
    }

    #[test]
    fn strips_oneline_comments() {
        assert_eq!(minify("{\"a\":1} // trailing comment\n"), r#"{"a":1}"#);
    }

    #[test]
    fn strips_multiline_comments() {
        assert_eq!(minify("{/* a comment */\"a\":1}"), r#"{"a":1}"#);
    }

    #[test]
    fn preserves_whitespace_inside_strings() {
        assert_eq!(
            minify(r#"{"a": "hello   world"}"#),
            r#"{"a":"hello   world"}"#
        );
    }

    #[test]
    fn preserves_escaped_quotes_inside_strings() {
        assert_eq!(minify(r#"{"a": "say \"hi\""}"#), r#"{"a":"say \"hi\""}"#);
    }

    #[test]
    fn slash_inside_string_is_not_a_comment() {
        assert_eq!(
            minify(r#"{"url": "http://example.com"}"#),
            r#"{"url":"http://example.com"}"#
        );
    }

    #[test]
    fn lone_slash_outside_string_is_silently_dropped() {
        // Upstream quirk: a '/' that isn't part of `//` or `/*` is deleted,
        // not preserved.
        assert_eq!(minify("1/2"), "12");
    }
}
