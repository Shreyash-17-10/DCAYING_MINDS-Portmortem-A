//! Integration tests executing the official RFC 6902 json-patch-tests suite.
//!
//! Replicates upstream's `test_apply_patch` semantics from `json_patch_tests.c`
//! exactly: apply each case's `patch` to a duplicate of `doc`; if the case has
//! an `"error"` key, expect failure; otherwise expect success and, if `"expected"`
//! is present, expect a case-sensitive `compare()` match; skip cases marked
//! `"disabled": true`.

use cjson_rs::parse::parse;
use cjson_rs::utils::apply_patches_case_sensitive;
use cjson_rs::value::{compare, Value};
use std::fs;
use std::path::PathBuf;

fn run_conformance_file(filename: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/json-patch-tests")
        .join(filename);
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {filename} at {:?}: {e}", path));
    let tests = parse(&content).unwrap_or_else(|e| panic!("failed to parse {filename}: {e:?}"));

    let tests_arr = tests.as_array().expect("tests file must be a JSON array");

    let mut total_run = 0;
    let mut total_skipped = 0;

    for (idx, test) in tests_arr.iter().enumerate() {
        // Skip cases marked "disabled": true
        if test.object_get("disabled") == Some(&Value::Bool(true)) {
            total_skipped += 1;
            continue;
        }

        let doc = test
            .object_get("doc")
            .expect("test must have a 'doc' field");
        let patch = test
            .object_get("patch")
            .expect("test must have a 'patch' field");

        let mut target = doc.duplicate(true);
        let res = apply_patches_case_sensitive(&mut target, patch);

        if test.object_has("error") {
            assert!(
                res.is_err(),
                "Test case #{idx} ('{}') in {filename} was expected to fail, but succeeded.",
                test.object_get("comment")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
            );
        } else {
            assert!(
                res.is_ok(),
                "Test case #{idx} ('{}') in {filename} failed: {:?}",
                test.object_get("comment")
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
                res
            );

            if let Some(expected) = test.object_get("expected") {
                assert!(
                    compare(&target, expected, true),
                    "Test case #{idx} ('{}') in {filename} mismatch.\nActual: {:?}\nExpected: {:?}",
                    test.object_get("comment")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    target,
                    expected
                );
            }
        }
        total_run += 1;
    }
    println!("File {filename}: run {total_run}, skipped {total_skipped}");
}

#[test]
fn test_json_patch_conformance_tests() {
    run_conformance_file("tests.json");
}

#[test]
fn test_json_patch_conformance_spec_tests() {
    run_conformance_file("spec_tests.json");
}

#[test]
fn test_json_patch_conformance_cjson_utils_tests() {
    run_conformance_file("cjson-utils-tests.json");
}
