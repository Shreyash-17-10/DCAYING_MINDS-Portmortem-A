//! C ABI shim.
//!
//! This is the *only* file in the crate that uses `unsafe` or crosses the
//! FFI boundary. Its sole purpose is differential testing / fuzzing: it lets
//! a small C harness (or the original cJSON.c itself, compiled side-by-side)
//! call into this Rust port and compare output byte-for-byte, satisfying the
//! hackathon's "behavioral equivalence" and "minimal unsafe code" bonus
//! criteria simultaneously — all safe logic lives in parse.rs/print.rs/
//! value.rs/utils.rs; this module is a thin, carefully-bounded translation
//! layer around it.
//!
//! Memory ownership contract (mirrors cJSON's own alloc/free pairing):
//! - `cjson_rs_parse` returns an opaque `*mut Value` owned by the caller.
//!   It must be released with `cjson_rs_free`, exactly once.
//! - `cjson_rs_print` / `cjson_rs_print_unformatted` return a `*mut c_char`
//!   owned by the caller. It must be released with `cjson_rs_free_string`,
//!   exactly once — never with libc `free()`, since the allocation was made
//!   by Rust's allocator (mirrors cJSON's own convention that
//!   `cJSON_Print`'s result must be freed with `cJSON_free`, not raw `free`).
//! - All functions return NULL on any failure (parse error, invalid UTF-8,
//!   null argument); callers must NULL-check before use, same as cJSON.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

use crate::parse::parse as parse_json;
use crate::print::{print, print_unformatted};
use crate::value::Value;

/// Parses a NUL-terminated UTF-8 JSON string. Returns an opaque handle owned
/// by the caller, or NULL on any parse/encoding failure.
/// Mirrors `cJSON_Parse` (cJSON.c:1227).
///
/// # Safety
/// `json` must be a valid pointer to a NUL-terminated C string, or NULL.
#[no_mangle]
pub unsafe extern "C" fn cjson_rs_parse(json: *const c_char) -> *mut Value {
    if json.is_null() {
        return ptr::null_mut();
    }
    let bytes = CStr::from_ptr(json);
    let text = match bytes.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    match parse_json(text) {
        Ok(value) => Box::into_raw(Box::new(value)),
        Err(_) => ptr::null_mut(),
    }
}

/// Pretty-prints a parsed value. Returns a caller-owned NUL-terminated
/// string (free with `cjson_rs_free_string`), or NULL on failure.
/// Mirrors `cJSON_Print` (cJSON.c:1307).
///
/// # Safety
/// `handle` must be a live pointer previously returned by `cjson_rs_parse`
/// and not yet passed to `cjson_rs_free`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn cjson_rs_print(handle: *const Value) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let value = &*handle;
    match print(value).ok().and_then(|s| CString::new(s).ok()) {
        Some(cstr) => cstr.into_raw(),
        None => ptr::null_mut(),
    }
}

/// Compact-prints a parsed value (no whitespace). Same ownership contract as
/// `cjson_rs_print`. Mirrors `cJSON_PrintUnformatted` (cJSON.c:1312).
///
/// # Safety
/// Same as `cjson_rs_print`.
#[no_mangle]
pub unsafe extern "C" fn cjson_rs_print_unformatted(handle: *const Value) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let value = &*handle;
    match print_unformatted(value).ok().and_then(|s| CString::new(s).ok()) {
        Some(cstr) => cstr.into_raw(),
        None => ptr::null_mut(),
    }
}

/// Releases a handle returned by `cjson_rs_parse`. Mirrors `cJSON_Delete`.
///
/// # Safety
/// `handle` must be either NULL or a pointer previously returned by
/// `cjson_rs_parse` that has not already been freed. Double-free is UB,
/// same as the original C API.
#[no_mangle]
pub unsafe extern "C" fn cjson_rs_free(handle: *mut Value) {
    if !handle.is_null() {
        drop(Box::from_raw(handle));
    }
}

/// Releases a string returned by `cjson_rs_print`/`cjson_rs_print_unformatted`.
/// Mirrors `cJSON_free` being used to release `cJSON_Print`'s result.
///
/// # Safety
/// `s` must be either NULL or a pointer previously returned by one of this
/// module's print functions that has not already been freed.
#[no_mangle]
pub unsafe extern "C" fn cjson_rs_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn round_trip_through_the_c_abi() {
        unsafe {
            let input = CString::new(r#"{"a":1,"b":[true,null]}"#).unwrap();
            let handle = cjson_rs_parse(input.as_ptr());
            assert!(!handle.is_null());

            let printed = cjson_rs_print_unformatted(handle);
            assert!(!printed.is_null());
            let out = CStr::from_ptr(printed).to_str().unwrap();
            assert_eq!(out, r#"{"a":1,"b":[true,null]}"#);

            cjson_rs_free_string(printed);
            cjson_rs_free(handle);
        }
    }

    #[test]
    fn null_input_returns_null_not_ub() {
        unsafe {
            assert!(cjson_rs_parse(ptr::null()).is_null());
            assert!(cjson_rs_print(ptr::null()).is_null());
            assert!(cjson_rs_print_unformatted(ptr::null()).is_null());
            // Must not crash.
            cjson_rs_free(ptr::null_mut());
            cjson_rs_free_string(ptr::null_mut());
        }
    }

    #[test]
    fn invalid_json_returns_null() {
        unsafe {
            let input = CString::new("{not valid json").unwrap();
            let handle = cjson_rs_parse(input.as_ptr());
            assert!(handle.is_null());
        }
    }
}
