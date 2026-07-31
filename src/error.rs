//! Error types for cjson-rs.
//! Filled in during Phase 2 (string/number primitives); extended in Phase 3 (parser).

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CJsonError {
    /// Input did not start with `"` where a string was expected.
    InvalidString { pos: usize },
    /// String literal ran off the end of input before a closing `"`.
    /// Mirrors cJSON's parse_string `fail` path when input ends unexpectedly.
    UnterminatedString { pos: usize },
    /// Backslash followed by a character that isn't a valid escape
    /// (only b,f,n,r,t,",\,/,u are valid — mirrors parse_string's default: goto fail).
    InvalidEscape { pos: usize },
    /// Malformed `\uXXXX` escape: bad hex digits, unpaired low surrogate,
    /// missing/invalid second half of a surrogate pair, or codepoint out of range.
    /// Mirrors utf16_literal_to_utf8's `fail` path.
    InvalidUnicodeEscape { pos: usize },
    /// Raw (non-escaped) bytes in the literal were not valid UTF-8.
    InvalidUtf8 { pos: usize },
    /// No valid number could be scanned at this position.
    /// Mirrors parse_number's `number_c_string == after_end` check (strtod consumed nothing).
    InvalidNumber { pos: usize },
    /// Input ended where a value (null/true/false/string/number/array/object) was expected.
    UnexpectedEnd { pos: usize },
    /// Byte at this position doesn't start any valid JSON value.
    /// Mirrors parse_value's final `return false` (cJSON.c:1419) after all
    /// literal/string/number/array/object checks fail.
    UnexpectedToken { pos: usize },
    /// Array/object nesting exceeded CJSON_NESTING_LIMIT (cJSON.h:137, default 1000).
    /// Mirrors parse_array/parse_object's depth check (cJSON.c:1502, 1667).
    NestingTooDeep { pos: usize },
    /// Inside an object, found a key that isn't a `"`-quoted string.
    /// Mirrors parse_object calling parse_string() on the key (cJSON.c:1726).
    ExpectedObjectKey { pos: usize },
    /// Object key was not followed by `:`. Mirrors cJSON.c:1736-1739.
    ExpectedColon { pos: usize },
    /// After an array/object element, found neither `,` nor the closing
    /// bracket/brace. Mirrors the `while (... == ',')` loop exit followed by
    /// the closing-character check in parse_array/parse_object
    /// (cJSON.c:1564-1569, 1750-1755).
    ExpectedCommaOrClose { pos: usize },
    /// Extra non-whitespace content found after a complete, valid JSON value.
    /// Only surfaced by `parse_strict` (mirrors cJSON_ParseWithOpts called
    /// with require_null_terminated=true, cJSON.c:1179-1186); the permissive
    /// `parse` entry point (mirrors cJSON_Parse, which passes
    /// require_null_terminated=false) ignores trailing content instead.
    TrailingCharacters { pos: usize },
}
