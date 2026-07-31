//! Rust port of fuzzing/cjson_read_fuzzer.c. Same input format as upstream
//! so the existing corpus (fuzzing/inputs/*, copied into fuzz/corpus/) works
//! unmodified: byte 0 = minify flag, byte 1 = require_termination flag,
//! byte 2 = formatted flag, byte 3 = buffered flag (each '0' or '1'),
//! followed by the JSON document itself.
//!
//! Run with:
//!   cargo install cargo-fuzz   (requires a nightly toolchain)
//!   cargo +nightly fuzz run cjson_read_fuzzer
//!
//! Not runnable in the sandbox this port was built in (no nightly rustc
//! available there) -- verified structurally correct and independently
//! exercised via differential/diff_test.c instead. Run this yourself with
//! nightly Rust for continuous fuzzing.

#![no_main]
use libfuzzer_sys::fuzz_target;

use cjson_rs::parse::{parse, parse_strict};
use cjson_rs::print::{print, print_unformatted};
use cjson_rs::parse::minify;

fuzz_target!(|data: &[u8]| {
    const OFFSET: usize = 4;
    if data.len() <= OFFSET {
        return;
    }
    if data[data.len() - 1] != 0 {
        return;
    }
    for &flag in &data[0..4] {
        if flag != b'0' && flag != b'1' {
            return;
        }
    }

    let minify_flag = data[0] == b'1';
    let require_termination = data[1] == b'1';
    let formatted = data[2] == b'1';

    // Body excludes the trailing NUL the original C harness required
    // (a Rust &str/&[u8] doesn't need one, but we mirror the corpus format).
    let body = &data[OFFSET..data.len() - 1];
    let text = match std::str::from_utf8(body) {
        Ok(t) => t,
        Err(_) => return,
    };

    let parsed = if require_termination {
        parse_strict(text)
    } else {
        parse(text)
    };

    let value = match parsed {
        Ok(v) => v,
        Err(_) => return,
    };

    // Exercise both print paths; the important thing under a fuzzer is that
    // neither panics/aborts/UB's on any input that successfully parsed.
    let _ = if formatted { print(&value) } else { print_unformatted(&value) };

    if minify_flag {
        let _ = minify(text);
    }
});
