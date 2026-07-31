# cjson-rs fuzz targets

Rust port of `fuzzing/cjson_read_fuzzer.c` using `cargo-fuzz`/`libFuzzer`.

## Requirements

`cargo-fuzz` requires a **nightly** Rust toolchain (uses `-Z` sanitizer
flags under the hood). This was not available in the sandbox this port was
built in (apt-installed stable rustc only), so this target is **structurally
complete but has not been run there**. It was verified independently via
`differential/diff_test.c`, which actually compiled and ran, comparing this
port's output against the real cJSON.c on 22 inputs including edge cases
(0 mismatches — see `../BENCHMARK_REPORT.md`'s sibling, or run
`differential/diff_test.c` yourself).

## Setup (on a machine with rustup)

```bash
rustup install nightly
cargo install cargo-fuzz
cargo +nightly fuzz run cjson_read_fuzzer
```

## Corpus

`corpus/cjson_read_fuzzer/` contains the original `fuzzing/inputs/test1..test11`
files, copied verbatim (unmodified) from upstream cJSON. They follow the
same 4-byte flag-prefix format the original `cjson_read_fuzzer.c` expects
(byte 0 = minify, byte 1 = require_termination, byte 2 = formatted, byte 3 =
buffered, each `'0'`/`'1'`, followed by the JSON body). Note: as committed
upstream, none of these seed files end in a NUL byte, so they're immediate
no-ops under the harness's own precondition check until libFuzzer mutates
them — this is upstream's original behavior, not a bug introduced here.

## Dictionary

Upstream's `fuzzing/json.dict` (token dictionary for smarter mutation) can
be reused as-is: `cargo +nightly fuzz run cjson_read_fuzzer -- -dict=../../original_c_reference/../fuzzing/json.dict`
(adjust the path to wherever you keep the original repo's `fuzzing/` directory).
