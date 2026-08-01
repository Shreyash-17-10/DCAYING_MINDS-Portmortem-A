//! Integration test running the official JSON Patch (RFC 6902) conformance
//! suite from https://github.com/json-patch/json-patch-tests, exactly as
//! upstream cJSON's own tests/json_patch_tests.c does (`test_apply_patch`):
//! for each case, apply `patch` to a duplicate of `doc` using
//! `apply_patches_case_sensitive`; if the case has an `"error"` key, expect
//! `apply_patches` to fail; otherwise expect it to succeed and, if
//! `"expected"` is present, expect the result to `compare()`-equal it
//! (case-sensitive, matching upstream's `cJSON_Compare(object, expected, true)`).
//! Cases marked `"disabled": true` are skipped, exactly as upstream skips them.
//!
//! Fixture files (`tests.json`, `spec_tests.json`, `cjson-utils-tests.json`)
//! are copied **verbatim, unmodified** from upstream's
//! `tests/json-patch-tests/`.

use cjson_rs::parse::parse;
use cjson_rs::utils::apply_patches_case_sensitive;
use cjson_rs::value::compare;
use cjson_rs::value::Value;
use std::fs;
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/json-patch-tests")
        .join(name)
}

struct CaseResult {
    comment: String,
    passed: bool,
    reason: String,
}

fn run_suite(file: &str) -> Vec<CaseResult> {
    let text = fs::read_to_string(fixture_path(file))
        .unwrap_or_else(|e| panic!("failed to read {file}: {e}"));
    let suite = parse(&text).unwrap_or_else(|e| panic!("failed to parse {file}: {e:?}"));
    let cases = suite.as_array().unwrap_or_else(|| panic!("{file} is not a JSON array"));

    let mut results = Vec::new();

    for case in cases {
        let comment = case
            .object_get("comment")
            .and_then(Value::as_str)
            .unwrap_or("(no comment)")
            .to_string();

        if case.object_get("disabled").map(Value::is_true) == Some(true) {
            continue; // matches upstream: disabled cases are skipped, not failed
        }

        let doc = match case.object_get("doc") {
            Some(d) => d,
            None => {
                results.push(CaseResult { comment, passed: false, reason: "missing \"doc\"".into() });
                continue;
            }
        };
        let patch = match case.object_get("patch") {
            Some(p) => p,
            None => {
                results.push(CaseResult { comment, passed: false, reason: "missing \"patch\"".into() });
                continue;
            }
        };

        let mut object = doc.duplicate(true);
        let apply_result = apply_patches_case_sensitive(&mut object, patch);

        let expects_error = case.object_get("error").is_some();

        let (passed, reason) = if expects_error {
            match apply_result {
                Err(_) => (true, String::new()),
                Ok(()) => (false, "expected patch application to fail, but it succeeded".into()),
            }
        } else {
            match apply_result {
                Err(e) => (false, format!("patch application failed unexpectedly: {e:?}")),
                Ok(()) => match case.object_get("expected") {
                    Some(expected) if !compare(&object, expected, true) => {
                        (false, "result did not match \"expected\"".into())
                    }
                    _ => (true, String::new()),
                },
            }
        };

        results.push(CaseResult { comment, passed, reason });
    }

    results
}

fn assert_suite_passes(file: &str) {
    let results = run_suite(file);
    let total = results.len();
    let failures: Vec<&CaseResult> = results.iter().filter(|r| !r.passed).collect();

    if !failures.is_empty() {
        let mut msg = format!(
            "{file}: {}/{} cases failed\n",
            failures.len(),
            total
        );
        for f in &failures {
            msg.push_str(&format!("  - \"{}\": {}\n", f.comment, f.reason));
        }
        panic!("{msg}");
    }

    println!("{file}: {total}/{total} cases passed");
}

#[test]
fn spec_tests_json() {
    assert_suite_passes("spec_tests.json");
}

#[test]
fn tests_json() {
    assert_suite_passes("tests.json");
}

#[test]
fn cjson_utils_tests_json() {
    assert_suite_passes("cjson-utils-tests.json");
}
