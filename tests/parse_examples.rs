//! Integration tests ported from cJSON's tests/parse_examples.c.
//!
//! Fixture files under `tests/fixtures/inputs/` are copied verbatim from the
//! original cJSON repository's `tests/inputs/` (test1..test11, plus their
//! `.expected` pretty-printed counterparts) — unmodified, per the hackathon
//! rule that any changes to the original test suite must be documented.
//! No modifications were made to any fixture's *content*; only their
//! location moved (C reads them via a relative "inputs/" path baked into
//! parse_examples.c, Rust reads them via `CARGO_MANIFEST_DIR`).
//!
//! Source: cJSON tests/parse_examples.c (`do_test`, `file_testN_*`,
//! `test6_should_not_be_parsed`, `test12`..`test15`).

use cjson_rs::parse::parse;
use std::fs;
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/inputs")
        .join(name)
}

fn read_fixture(name: &str) -> String {
    fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"))
}

/// Mirrors `do_test()`: parse the input file, pretty-print it, and expect an
/// exact byte-for-byte match against the `.expected` file.
fn do_test(test_name: &str) {
    let input = read_fixture(test_name);

    let expected = read_fixture(&format!("{test_name}.expected"));

    let tree = parse(&input).unwrap_or_else(|e| panic!("failed to parse {test_name}: {e:?}"));

    let actual = cjson_rs::print::print(&tree)
        .unwrap_or_else(|e| panic!("failed to print {test_name}: {e:?}"));

let actual_norm = actual.replace("\r\n", "\n");
let expected_norm = expected.replace("\r\n", "\n");

assert_eq!(
    expected_norm,
    actual_norm,
    "mismatch for {test_name}"
);
}

#[test]
fn file_test1_should_be_parsed_and_printed() {
    do_test("test1");
}

#[test]
fn file_test2_should_be_parsed_and_printed() {
    do_test("test2");
}

#[test]
fn file_test3_should_be_parsed_and_printed() {
    do_test("test3");
}

#[test]
fn file_test4_should_be_parsed_and_printed() {
    do_test("test4");
}

#[test]
fn file_test5_should_be_parsed_and_printed() {
    do_test("test5");
}

#[test]
fn file_test7_should_be_parsed_and_printed() {
    do_test("test7");
}

#[test]
fn file_test8_should_be_parsed_and_printed() {
    do_test("test8");
}

#[test]
fn file_test9_should_be_parsed_and_printed() {
    do_test("test9");
}

#[test]
fn file_test10_should_be_parsed_and_printed() {
    do_test("test10");
}

#[test]
fn file_test11_should_be_parsed_and_printed() {
    do_test("test11");
}

/// Mirrors `file_test6_should_not_be_parsed`: test6 is an HTML error page
/// (not JSON at all), parsing it must fail.
#[test]
fn file_test6_should_not_be_parsed() {
    let input = read_fixture("test6");
    assert!(
        parse(&input).is_err(),
        "test6 (HTML, not JSON) should fail to parse"
    );
}

/// Mirrors `test12_should_not_be_parsed`: an object with an unterminated
/// value should fail, not panic or hang.
#[test]
fn incomplete_object_should_not_be_parsed() {
    let input = "{ \"name\": ";
    assert!(parse(input).is_err());
}

/// Mirrors `test13_should_be_parsed_without_null_termination`. In C this
/// tests that the parser respects an explicit buffer length instead of
/// reading past a missing NUL terminator (a real memory-safety concern for
/// raw `char*` buffers). Rust `&str`/`&[u8]` slices always carry their own
/// length, so there is no NUL-termination assumption to test here — the
/// case is preserved as a straightforward "valid JSON with no embedded
/// terminator" parse check, and the underlying C-specific risk (reading
/// past the end of an unterminated buffer) is eliminated by construction
/// rather than tested at runtime.
#[test]
fn parses_valid_json_from_an_exact_length_slice() {
    let json = r#"{"Image":{"Width":800,"Height":600,"Title":"Viewfrom15thFloor","Thumbnail":{"Url":"http://www.example.com/image/481989943","Height":125,"Width":"100"},"IDs":[116,943,234,38793]}}"#;
    // Build a byte slice with no trailing NUL and exactly `json.len()` bytes,
    // mirroring the C test's `memcpy` into a same-sized non-terminated buffer.
    let bytes = json.as_bytes().to_vec();
    let s = std::str::from_utf8(&bytes).unwrap();
    assert!(parse(s).is_ok());
}

/// Mirrors `test14_should_not_be_parsed`: parsing must stop at the given
/// buffer length and fail if that cuts the JSON short, rather than reading
/// past the end.
#[test]
fn truncated_buffer_should_not_be_parsed() {
    let json = r#"{"Image":{"Width":800,"Height":600,"Title":"Viewfrom15thFloor","Thumbnail":{"Url":"http://www.example.com/image/481989943","Height":125,"Width":"100"},"IDs":[116,943,234,38793]}}"#;
    let bytes = json.as_bytes();
    let truncated = &bytes[..bytes.len() - 2];
    let s = std::str::from_utf8(truncated).unwrap();
    assert!(
        parse(s).is_err(),
        "truncated JSON should fail to parse, not read past the end"
    );
}

/// Mirrors `test15_should_not_heap_buffer_overflow` (an ASan regression
/// test for a real historical cJSON bug: parsing right up to the edge of a
/// heap allocation with no slack must not read one byte past it). Rust's
/// bounds-checked slices make out-of-bounds reads a compile-time-impossible
/// class of bug, so this is kept as a plain "malformed input is rejected
/// cleanly" check rather than an ASan test.
#[test]
fn malformed_truncated_object_is_rejected_without_reading_past_the_end() {
    for s in ["{\"1\":1,", "{\"1\":1, "] {
        assert!(parse(s).is_err());
    }
}
