//! cjson-rs: Rust port of cJSON (https://github.com/DaveGamble/cJSON), MIT licensed.
//! Progress: value model (`value`), parsing (`parse`), and printing (`print`)
//! are implemented, including the full manipulation API (`value`: detach/
//! delete/insert/replace/duplicate/compare) and `cJSON_Minify` (`parse`).
//! `utils` currently has JSON Pointer (RFC 6901); JSON Patch and JSON Merge
//! Patch are still pending.

pub mod error;
pub mod ffi;
pub mod parse;
pub mod print;
pub mod utils;
pub mod value;
