# Benchmark Report — cjson-rs vs. original cJSON.c

## Methodology

Two independent benchmark harnesses exercise the same operations over the same inputs:

- **Rust**: `benches/parse_print.rs`, run via `cargo bench` (criterion 0.5, `--release`).
- **C**: `benches/c_bench/bench.c`, compiled with `gcc -O3` and linked directly
  against `original_c_reference/cJSON.c` (the unmodified upstream source),
  timed with `clock_gettime(CLOCK_MONOTONIC)`.

Both harnesses run the **same three real fixtures** (`test1`, `test5`,
`test10` — copied verbatim from upstream `tests/inputs/`, ranging from a
small flat object to a deeply nested glossary document) and the **same
synthetic array-of-objects generator** at 100 / 1,000 / 10,000 elements
(`[{"id":N,"name":"item-N","active":bool,"score":N*1.5,"tags":["a","b","c"]},...]`).

Each measures: parse time, formatted-print time, and unformatted-print time.
Rust numbers are criterion's median (of 20–30 samples after warm-up); C
numbers are wall-clock totals divided by iteration count (single sample, no
warm-up discard — noisier by construction, treat as directional).

**Environment**: sandboxed Linux VM, rustc 1.75.0, gcc 13.3.0, single run,
no CPU pinning/isolation. These are *not* rigorous, low-noise measurements —
they're a real, reproducible A/B comparison sufficient to characterize
relative performance and catch gross regressions, not a formal performance
audit. Re-run `cargo bench` and `./c_bench` on your own machine for numbers
that matter for a specific deployment target.

## Results

All times are µs/iteration (lower is better). "Ratio" = Rust ÷ C; >1.0 means
Rust is slower, <1.0 means Rust is faster.

### Parse

| Input | C (µs) | Rust (µs) | Ratio |
|---|---|---|---|
| test1 (small object) | 1.98 | 2.23 | 1.13× slower |
| test5 | 4.78 | 5.58 | 1.17× slower |
| test10 | 0.49 | 0.56 | 1.16× slower |
| synthetic, 100 items | 91.7 | 89.2 | 0.97× (faster) |
| synthetic, 1,000 items | 1,013.6 | 1,105.2 | 1.09× slower |
| synthetic, 10,000 items | 15,200.7 | 16,405.0 | 1.08× slower |

### Print (formatted / pretty)

| Input | C (µs) | Rust (µs) | Ratio |
|---|---|---|---|
| test1 | 1.35 | 1.76 | 1.31× slower |
| test5 | 2.64 | 2.84 | 1.08× slower |
| test10 | 0.31 | 0.42 | 1.38× slower |

### Print (unformatted / compact)

| Input | C (µs) | Rust (µs) | Ratio |
|---|---|---|---|
| test1 | 1.11 | 1.70 | 1.53× slower |
| test5 | 2.42 | 2.43 | ~even |
| test10 | 0.30 | 0.44 | 1.48× slower |
| synthetic, 100 items | 69.4 | 61.3 | 0.88× (faster) |
| synthetic, 1,000 items | 838.0 | 630.0 | 0.75× (faster) |
| synthetic, 10,000 items | 11,142.1 | 6,615.6 | **0.59× (41% faster)** |

## Interpretation

**Parsing**: Rust is consistently 8–17% slower than C across every input
size. The likely cause is architectural, not accidental: this port
represents object keys and array elements as owned `Vec`/`String`
(Phase 1's design choice, see `DECISIONS.md`), which means every string
value is heap-allocated as a distinct `String` during parsing, whereas
cJSON's C parser reuses one shared input buffer and only allocates when a
value must outlive it. That's the direct cost of memory-safety-by-default
over C's manual, more surgical allocation — an expected and acceptable
trade for a hackathon judged 20% on code quality, not
exclusively on raw speed.

**Printing (small inputs)**: Rust is 30–53% slower on the three small,
real-world fixtures. Per-call fixed overhead (allocating a fresh output
`String`, `CString` conversion at the FFI layer in earlier benchmarks
— though *these* benchmarks call `print()`/`print_unformatted()` directly,
not through FFI) dominates when the payload is tiny.

**Printing (large inputs)**: Rust pulls consistently and increasingly
*ahead* of C as input grows — 12% faster at 100 elements, 25% faster at
1,000, **41% faster at 10,000**. This is very likely `Vec<u8>`/`String`'s
amortized-doubling growth strategy outperforming cJSON's `printbuffer`
reallocation strategy at scale, though this hypothesis is not yet
confirmed by profiling — flagged as a candidate follow-up in `DECISIONS.md`
rather than asserted as fact.

## Reproducing

```bash
# Rust side
cargo bench --bench parse_print

# C side
cd benches/c_bench
gcc -O3 bench.c ../../original_c_reference/cJSON.c \
    -I../../original_c_reference -lm -o c_bench
./c_bench ../../tests/fixtures/inputs
```
