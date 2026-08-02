//! cjson-rs: Rust port of cJSON (https://github.com/DaveGamble/cJSON), MIT licensed.
//! Progress: value model (`value`), parsing (`parse`), printing (`print`),
//! and utilities (`utils` containing JSON Pointer (RFC 6901), JSON Patch (RFC 6902) apply,
//! and JSON Merge Patch (RFC 7396) apply) are fully complete,
//! including the full manipulation API (`value`: detach/delete/insert/replace/duplicate/compare),
//! `cJSON_Minify` (`parse`), and a C ABI / FFI layer (`ffi`).

pub mod error;
pub mod ffi;
pub mod parse;
pub mod print;
pub mod utils;
pub mod value;
pub mod wasm;
