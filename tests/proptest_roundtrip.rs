//! Property-based testing: generates thousands of structurally random
//! `Value` trees and JSON texts, and checks invariants that should hold for
//! *any* input, not just the hand-picked examples elsewhere in the suite.
//!
//! This exists specifically to strengthen this port's Behavioral
//! Equivalence evidence beyond fixed test cases: `cargo test` alone proves
//! correctness on the inputs someone thought to write down; property tests
//! sample the input space itself, which is a meaningfully different (and
//! complementary) kind of evidence. No `cargo-fuzz`/nightly toolchain is
//! required - `proptest` runs on stable Rust, so this executes in any
//! environment `cargo test` does.
//!
//! ## Two real findings from writing this file, and how they were resolved
//!
//! Early versions of these properties asserted invariants that turned out
//! to be *false even for the original C library*, not just for this port.
//! Both were tracked down and verified against the actual, unmodified
//! `original_c_reference/cJSON.c` before concluding anything - see
//! DECISIONS.md for the full writeup. In short:
//!
//! 1. **Exact print/parse round-tripping is not guaranteed by cJSON's own
//!    number-printing algorithm.** `print_number` tries 15 significant
//!    digits and only falls back to 17 if a *tolerant* (epsilon-based)
//!    round-trip check fails - so a small set of doubles print in a form
//!    that reads back as a value merely "close enough" by that same
//!    epsilon, not bit-identical. Confirmed by compiling and running the
//!    real `cJSON.c` on the exact failing input
//!    (`d = -631908566981097.9`): it also prints `-631908566981098` and
//!    also fails to round-trip exactly. This port's `compare_double` is a
//!    faithful port of C's `compare_double`, so it inherits the same
//!    characteristic - correctly. The fix was to the *test*, not the
//!    code: round-trip properties below use `value::compare()` (cJSON's
//!    own epsilon-tolerant equality) rather than exact `PartialEq`.
//! 2. **Case-insensitive object comparison isn't well-defined for objects
//!    containing case-insensitive-duplicate keys** (e.g. both `"c"` and
//!    `"C"` in the same object) - `compare(_, _, case_sensitive=false)`'s
//!    lookup always resolves to the *first* case-insensitive match, so the
//!    second occurrence's value can get compared against the wrong entry.
//!    Confirmed by compiling and running the real `cJSON_Compare` on the
//!    exact failing structure: it returns `0` (not equal) for the object
//!    against an identical copy of itself, matching this port exactly.
//!    The fix was to the *test's generator* - `arb_value()` below
//!    deduplicates object keys case-insensitively, since this degenerate
//!    input class is a shared, verified characteristic of the algorithm
//!    both implementations use, not something either implementation gets
//!    to unilaterally "fix" without becoming a different algorithm.

use cjson_rs::parse::parse;
use cjson_rs::print::{print, print_unformatted};
use cjson_rs::value::{compare, Value};
use proptest::prelude::*;
use std::collections::HashSet;

/// Generates an arbitrary `Value` tree. Recursion depth and container size
/// are both bounded so generated cases stay well under `NESTING_LIMIT`
/// (1000) - this strategy exercises general correctness, not the
/// nesting-limit boundary itself, which already has a dedicated test in
/// `src/parse.rs`.
fn arb_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        // Finite only: NaN/Infinity can't round-trip through JSON text at
        // all (see print::tests::number_nan_and_infinity_print_null - this
        // is documented, intentional print-side behavior, not something a
        // round-trip property should be asserting against).
        (-1e15f64..1e15f64).prop_filter("finite", |n| n.is_finite()).prop_map(Value::Number),
        "[a-zA-Z0-9 _\\-]{0,12}".prop_map(Value::String),
    ];

    leaf.prop_recursive(4, 64, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::vec(
                ("[a-zA-Z][a-zA-Z0-9_]{0,8}".prop_map(String::from), inner),
                0..6
            )
            .prop_map(dedupe_keys_case_insensitively),
        ]
    })
}

/// Drops later entries whose key case-insensitively collides with an
/// earlier one. See this file's top-level doc comment, finding #2: objects
/// with case-insensitive-duplicate keys are a genuine, shared degenerate
/// case for `compare(_, _, case_sensitive=false)` in *both* this port and
/// the original C library - not something these properties should be
/// generating and then asserting well-defined behavior for.
fn dedupe_keys_case_insensitively(pairs: Vec<(String, Value)>) -> Value {
    let mut seen: HashSet<String> = HashSet::new();
    let deduped: Vec<(String, Value)> = pairs
        .into_iter()
        .filter(|(k, _)| seen.insert(k.to_lowercase()))
        .collect();
    Value::Object(deduped)
}

proptest! {
    /// print(v) must always produce valid JSON that parses back to a value
    /// `compare()`-equal to the original - the fundamental round-trip
    /// invariant for any serializer/parser pair, stated using cJSON's own
    /// epsilon-tolerant notion of equality (see finding #1 above for why
    /// exact `PartialEq` is the wrong invariant to assert here). Runs both
    /// formatted and unformatted output.
    #[test]
    fn print_then_parse_round_trips(v in arb_value()) {
        let unformatted = print_unformatted(&v).unwrap();
        let reparsed = parse(&unformatted).unwrap();
        prop_assert!(compare(&v, &reparsed, true), "unformatted round-trip mismatch");

        let formatted = print(&v).unwrap();
        let reparsed_formatted = parse(&formatted).unwrap();
        prop_assert!(compare(&v, &reparsed_formatted, true), "formatted round-trip mismatch");
    }

    /// Formatted and unformatted output must parse back to the *same*
    /// tree - whitespace/indentation choices must never be
    /// semantically significant.
    #[test]
    fn formatted_and_unformatted_agree_on_meaning(v in arb_value()) {
        let a = parse(&print(&v).unwrap()).unwrap();
        let b = parse(&print_unformatted(&v).unwrap()).unwrap();
        prop_assert!(compare(&a, &b, true));
    }

    /// duplicate(recurse = true) must produce a tree that prints identically
    /// to the original - print() output is a stronger, more direct check
    /// here than value comparison, since duplicate() copies the exact
    /// underlying f64 bits (no text round-trip involved), so this one
    /// property *can* assert byte-exact equality safely.
    #[test]
    fn duplicate_preserves_print_output(v in arb_value()) {
        let dup = v.duplicate(true);
        prop_assert_eq!(print_unformatted(&v).unwrap(), print_unformatted(&dup).unwrap());
    }

    /// compare(v, v.duplicate(true), _) must always be true - a deep copy
    /// is definitionally equal to its source under both case-sensitive and
    /// case-insensitive comparison.
    #[test]
    fn value_always_compares_equal_to_its_own_duplicate(v in arb_value()) {
        let dup = v.duplicate(true);
        prop_assert!(compare(&v, &dup, true));
        prop_assert!(compare(&v, &dup, false));
    }

    /// generate_patch(from, to) applied to `from` must always reach `to`,
    /// for arbitrary structurally-different trees - not just the
    /// hand-picked cases in utils::generate_tests. Uses case-sensitive
    /// comparison and generation throughout: case-insensitive equality
    /// for objects with case-insensitive-duplicate keys is the genuine,
    /// shared degenerate case documented in finding #2 above, and
    /// `arb_value()` already prevents that key shape from being generated
    /// at all - this test uses the case-sensitive API pair regardless, to
    /// keep the property meaningful independent of that generator detail.
    #[test]
    fn generated_patch_always_reaches_the_target(from in arb_value(), to in arb_value()) {
        let patch = cjson_rs::utils::generate_patch_case_sensitive(&from, &to);
        let mut applied = from.clone();
        let result = cjson_rs::utils::apply_patches_case_sensitive(&mut applied, &patch);
        prop_assert!(result.is_ok(), "generated patch failed to apply: {:?}", result);
        prop_assert!(
            compare(&applied, &to, true),
            "applying the generated patch did not reach the target"
        );
    }
}
