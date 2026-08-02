# cjson-rs

An idiomatic Rust port of [cJSON](https://github.com/DaveGamble/cJSON), the
ultralightweight ANSI C JSON parser/serializer by Dave Gamble and
contributors (MIT licensed). Built for **Port Mortem 2026**, part of the
[Code Resurrection Hackathon](https://coderesurrection.com/2026) — C → Rust
migration track.

This is a genuine port, not a wrapper: there is no `unsafe extern "C"` call
into the original `cJSON.c` anywhere in the library itself. The original C
source is kept in `original_c_reference/` purely for license attribution
and as the reference implementation for differential testing/benchmarking —
it is never compiled into `libcjson_rs`.

See **[DECISIONS.md](./DECISIONS.md)** for the full architectural writeup
(data model, error handling, what's ported vs. not, and every intentional
behavioral difference from upstream) and
**[BENCHMARK_REPORT.md](./BENCHMARK_REPORT.md)** for measured performance
vs. the original C.

## Build

Single documented command:

```bash
cargo build --release
```

Requires a Rust toolchain (`rustc`/`cargo`), 1.75+. Install via
[rustup.rs](https://rustup.rs) if you don't have one.

Produces `target/release/libcjson_rs.a` (static), `libcjson_rs.so`
(dynamic), and `libcjson_rs.rlib` (for other Rust crates) — see
[FFI / calling from C](#ffi--calling-from-c) below.

## Test

```bash
cargo test
```

Runs 112 unit tests (co-located with each module) plus three integration
test files: `tests/parse_examples.rs` (15 tests, using the original cJSON
`tests/inputs/test1..test11` fixtures copied verbatim, parse → print →
byte-exact match against upstream's own `.expected` outputs),
`tests/json_pointer_examples.rs` (the RFC 6901 conformance case ported from
upstream's `old_utils_tests.c`), and `tests/json_patch_conformance.rs`
(the official [json-patch-tests](https://github.com/json-patch/json-patch-tests)
RFC 6902 conformance suite — `tests.json`, `spec_tests.json`,
`cjson-utils-tests.json`, copied verbatim from upstream — **121 cases total,
4 disabled matching upstream's own flags, 117/117 active cases pass**).
131 tests total, all passing.

## Benchmark

```bash
cargo bench --bench parse_print
```

Criterion-based benchmarks over the same fixtures used in the test suite,
plus synthetic large documents (100/1,000/10,000-element arrays). A
companion C benchmark against the unmodified original `cJSON.c` lives in
`benches/c_bench/` (build/run instructions in
[BENCHMARK_REPORT.md](./BENCHMARK_REPORT.md)) — full head-to-head numbers
and analysis are there.

## Differential testing

```bash
cargo build --release   # produces target/release/libcjson_rs.a
cd differential
gcc -O2 diff_test.c ../original_c_reference/cJSON.c \
    ../target/release/libcjson_rs.a \
    -I../original_c_reference -I../ffi_include \
    -lpthread -ldl -lm -o diff_test
./diff_test corpus
```

Links the real `cJSON.c` and this port into one binary, feeds both the same
inputs (11 original fixtures + 11 handwritten edge cases: unicode surrogate
pairs, extreme numbers, invalid input, deep nesting, duplicate keys), and
checks byte-identical output. Last run: **22/22 matched, 0 mismatches.**

## Fuzzing

```bash
rustup install nightly
cargo install cargo-fuzz
cargo +nightly fuzz run cjson_read_fuzzer
```

Requires `rustup`/nightly (not available in the sandbox this port was
originally built in — see `fuzz/README.md` for the honest status: the
target is structurally complete and ports the original
`fuzzing/cjson_read_fuzzer.c` exactly, but has not itself been executed as
part of this submission; the differential test above is the executed
evidence of correctness in the meantime).

## FFI / calling from C

`src/ffi.rs` exposes a small C ABI (`cjson_rs_parse`, `cjson_rs_print`,
`cjson_rs_print_unformatted`, `cjson_rs_free`, `cjson_rs_free_string`) —
see `ffi_include/cjson_rs.h` for the header (hand-written; regenerate
automatically anytime with `cbindgen --config cbindgen.toml --output
ffi_include/cjson_rs_generated.h` if you have
[cbindgen](https://github.com/mozilla/cbindgen) installed). This is the
*only* file in the crate using `unsafe`.

```bash
cargo build --release
cd ffi_include
gcc smoke_test.c -I. ../target/release/libcjson_rs.a -lpthread -ldl -lm -o smoke_test
./smoke_test
```

## Project layout

```
src/
  value.rs   — core Value enum (replaces struct cJSON), constructors,
               predicates, array/object mutation, compare, duplicate
  parse.rs   — number/string primitives + recursive-descent parser
  print.rs   — pretty and compact serialization
  utils.rs   — JSON Pointer (RFC 6901), JSON Patch apply (RFC 6902),
               JSON Merge Patch apply (RFC 7396); diff/generate for
               Patch and Merge Patch not yet ported
  error.rs   — CJsonError, with source position on every variant
  ffi.rs     — C ABI shim (the only unsafe in the crate)
tests/
  parse_examples.rs           — ported from cJSON's parse_examples.c
  json_pointer_examples.rs    — ported from cJSON's old_utils_tests.c
  json_patch_conformance.rs   — official RFC 6902 json-patch-tests suite
  fixtures/inputs/            — original cJSON test fixtures, unmodified
  fixtures/json-patch-tests/  — official JSON Patch conformance suite, unmodified
benches/
  parse_print.rs   — criterion benchmarks
  c_bench/         — companion C benchmark against original cJSON.c
differential/
  diff_test.c   — C↔Rust differential testing harness
  corpus/        — test inputs (originals + adversarial edge cases)
fuzz/
  fuzz_targets/cjson_read_fuzzer.rs — cargo-fuzz target (ported from C)
original_c_reference/
  cJSON.c/h, cJSON_Utils.c/h, LICENSE — unmodified upstream source,
  kept for attribution and as the differential-testing/benchmark baseline
```

## License

MIT — see [`original_c_reference/LICENSE`](./original_c_reference/LICENSE).
This port retains the original cJSON copyright and license terms as
required; new Rust source is contributed under the same terms.

## Known gaps

JSON Patch (RFC 6902) and JSON Merge Patch (RFC 7396) **application** are
fully implemented and pass the official conformance suite (117/117 active
cases). What's *not* ported is patch **generation** — diffing two documents
to produce a patch (`cJSONUtils_GeneratePatches`,
`cJSONUtils_GenerateMergePatch`). See
[DECISIONS.md §6b](./DECISIONS.md#6b-json-patch-rfc-6902-and-json-merge-patch-rfc-7396--apply-side)
for the full writeup.
