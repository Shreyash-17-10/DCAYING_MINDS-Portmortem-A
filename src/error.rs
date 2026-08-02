//! Error types for cjson-rs.
//! Contains the CJsonError enum used for parsing errors.

use std::fmt;

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

impl CJsonError {
    /// Returns the byte position where the error was detected.
    pub fn position(&self) -> usize {
        match self {
            CJsonError::InvalidString { pos }
            | CJsonError::UnterminatedString { pos }
            | CJsonError::InvalidEscape { pos }
            | CJsonError::InvalidUnicodeEscape { pos }
            | CJsonError::InvalidUtf8 { pos }
            | CJsonError::InvalidNumber { pos }
            | CJsonError::UnexpectedEnd { pos }
            | CJsonError::UnexpectedToken { pos }
            | CJsonError::NestingTooDeep { pos }
            | CJsonError::ExpectedObjectKey { pos }
            | CJsonError::ExpectedColon { pos }
            | CJsonError::ExpectedCommaOrClose { pos }
            | CJsonError::TrailingCharacters { pos } => *pos,
        }
    }
}

impl fmt::Display for CJsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CJsonError::InvalidString { pos } => {
                write!(f, "invalid string at position {pos}")
            }
            CJsonError::UnterminatedString { pos } => {
                write!(f, "unterminated string at position {pos}")
            }
            CJsonError::InvalidEscape { pos } => {
                write!(f, "invalid escape sequence at position {pos}")
            }
            CJsonError::InvalidUnicodeEscape { pos } => {
                write!(f, "invalid unicode escape at position {pos}")
            }
            CJsonError::InvalidUtf8 { pos } => {
                write!(f, "invalid UTF-8 at position {pos}")
            }
            CJsonError::InvalidNumber { pos } => {
                write!(f, "invalid number at position {pos}")
            }
            CJsonError::UnexpectedEnd { pos } => {
                write!(f, "unexpected end of input at position {pos}")
            }
            CJsonError::UnexpectedToken { pos } => {
                write!(f, "unexpected token at position {pos}")
            }
            CJsonError::NestingTooDeep { pos } => {
                write!(f, "nesting too deep at position {pos}")
            }
            CJsonError::ExpectedObjectKey { pos } => {
                write!(f, "expected object key at position {pos}")
            }
            CJsonError::ExpectedColon { pos } => {
                write!(f, "expected ':' at position {pos}")
            }
            CJsonError::ExpectedCommaOrClose { pos } => {
                write!(f, "expected ',' or closing bracket at position {pos}")
            }
            CJsonError::TrailingCharacters { pos } => {
                write!(f, "trailing characters at position {pos}")
            }
        }
    }
}

impl std::error::Error for CJsonError {}

