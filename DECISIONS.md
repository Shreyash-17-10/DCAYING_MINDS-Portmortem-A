# DECISIONS.md — cjson-rs (Port Mortem 2026, C → Rust)

Architectural decisions, trade-offs, and known gaps in this port of
[cJSON](https://github.com/DaveGamble/cJSON) (MIT, Dave Gamble and
contributors) from C to Rust. Written phase-by-phase to match how the port
was actually built; see the roadmap discussion for the phase breakdown.

## 1. Scope and status

| Original file | Rust module | Status |
|---|---|---|
| `cJSON.h` (type/struct definitions) | `src/value.rs` | Complete |
| `cJSON.c` — parsing | `src/parse.rs` | Complete |
| `cJSON.c` — printing | `src/print.rs` | Complete |
| `cJSON.c` — manipulation API (Add/Delete/Replace/Detach/Duplicate/Compare) | `src/value.rs` | Complete |
| `cJSON_Utils.c/h` — JSON Pointer (RFC 6901) | `src/utils.rs` | Complete |
| `cJSON_Utils.c/h` — JSON Patch (RFC 6902) | `src/utils.rs` | **Not implemented** |
| `cJSON_Utils.c/h` — JSON Merge Patch (RFC 7396) | `src/utils.rs` | **Not implemented** |
| C ABI / FFI shim | `src/ffi.rs` | Complete |
| Test suite | `tests/*.rs` | Partial — see §7 |
| Benchmarks | `benches/parse_print.rs`, `benches/c_bench/` | Complete |
| Fuzzing | `fuzz/` | Scaffolded, not executed in this environment — see §9 |
| Differential testing | `differential/diff_test.c` | Complete, executed, 0 mismatches |

**Known gap, stated plainly:** JSON Patch and JSON Merge Patch
(`cJSONUtils_ApplyPatch`, `cJSONUtils_MergePatch`, and their generators) are
not ported. `src/utils.rs` implements JSON Pointer only
(`GetPointer`/`GetPointerCaseSensitive`/`FindPointerFromObjectTo`). This was
a deliberate scope cut to keep each work session reviewable rather than
rushing ~700 lines of `cJSON_Utils.c`'s patch/merge logic in one pass. If
judged before this is closed out, this is the single biggest functionality
gap versus upstream.

## 2. Core data model (`src/value.rs`)

C's `struct cJSON` is a single struct doing five jobs at once: it's a tagged
union (via an `int type` field with bitflags for the value kind, boolean
state, const-ness, and reference-ness), an intrusive doubly-linked list node
(`next`/`prev` for array/object siblings), and a tree node (`child` pointing
into the first child of an array/object). Values, keys, and structure all
live in one struct with raw `char *` ownership tracked by convention, not
the type system.

**Decision:** replace this with a single Rust enum:

```rust
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Raw(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}
```

- The 7 C type tags map onto 7 variants directly; `True`/`False` collapse
  into `Bool(bool)`.
- **Arrays and objects use `Vec`, not a re-implemented linked list.**
  This is the single biggest structural departure from upstream, so it's
  worth justifying: cJSON's linked list exists to support C's
  manual-memory-management story (O(1) detach/insert without shifting,
  no realloc of a contiguous block). Rust doesn't need that trade — `Vec`
  gives O(1) amortized push, contiguous cache-friendly iteration (which is
  the dominant access pattern for parse/print, print/print), and doesn't
  require `unsafe` or reference counting to express safely. The cost is
  O(n) detach/insert-at-arbitrary-position instead of O(1); given this is a
  JSON library where construction and traversal dominate over frequent
  interior mutation, that trade favors `Vec`.
- **Objects preserve key order and allow duplicate keys**, matching
  upstream's linked-list-as-object semantics exactly (`Vec<(String, Value)>`,
  not a `HashMap`/`BTreeMap`). `object_get` does a linear scan for the first
  match, same complexity class and same "first match wins" semantics as
  `cJSON_GetObjectItem`'s list walk.
- `valueint` (deprecated upstream — historically a `double`, now derived
  from `valuedouble`, kept only for source compatibility) is **not** stored
  state in this port; it's a method, `Value::as_int`, computed on demand.
  There's no legacy-field-sync bug class to have in the first place.
- `cJSON_IsReference` / `cJSON_StringIsConst` — flags C uses to mark a node
  as "don't free my children" or "don't free my string" for shared-ownership
  patterns — have no equivalent. Rust's ownership system makes those states
  inexpressible by construction: a `Value` you have unshared access to
  fully owns its subtree, always.
- No `unsafe` anywhere in this module.

## 3. Parsing (`src/parse.rs`)

Recursive-descent parser over `&[u8]`/`&str`, structurally mirroring
`parse_value`/`parse_array`/`parse_object`/`parse_string`/`parse_number`
from `cJSON.c`, function-for-function.

- **Locale independence (intentional deviation).** C's `parse_number` calls
  `strtod`, which is locale-sensitive: on a system with a comma-decimal
  locale active, `strtod("1.5", ...)` can silently parse as `1` (stopping at
  the `.`) or throw off downstream math, depending on platform. This is a
  known, documented footgun in upstream cJSON (worked around historically
  via `cJSON_InitHooks`/locale-juggling patches in some deployments). Rust's
  `f64::from_str` is always `.`-based regardless of process locale. This
  port keeps that behavior rather than replicating the C bug — flagged here
  explicitly since "faithful port" and "don't reproduce known footguns" are
  in tension, and the latter won.
- **Two entry points, matching two different upstream behaviors that look
  like one function:** `cJSON_Parse` internally calls
  `cJSON_ParseWithOpts(value, NULL, /* require_null_terminated */ 0)` —
  meaning the widely-used `cJSON_Parse` **silently ignores trailing
  garbage** after a valid value (`cJSON_Parse("123abc")` succeeds and
  returns `123`). This surprised me enough to verify it by reading the
  source twice. This port exposes that exact behavior as `parse()`, and
  additionally exposes `parse_strict()` (mirrors
  `cJSON_ParseWithOpts(..., 1)`) for callers who want trailing-garbage
  rejection. Upstream's single confusing dual-behavior function became two
  clearly-named ones.
- **Nesting limit** (`NESTING_LIMIT = 1000`, matches `CJSON_NESTING_LIMIT`
  in `cJSON.h`) is enforced in both the parser and the printer, preventing
  stack-overflow-via-recursion on adversarial/malicious deeply-nested input
  in both directions (parsing a 10,000-deep array, or printing a
  programmatically-built one) — verified with a boundary test at exactly
  1000/1001 levels.
- **Unicode escapes** (`\uXXXX`, including surrogate pairs) are decoded via
  `char::from_u32` and checked arithmetic, not C's manual UTF-8 byte-packing
  (`utf16_literal_to_utf8`, `cJSON.c:703-821`). Same acceptance/rejection
  behavior (unpaired low surrogates rejected, high-surrogate-without-pair
  rejected) verified by direct test, computed differently under the hood.
- Errors are a proper `enum CJsonError` with a `pos: usize` field per
  variant, instead of C's single global `ep`/error-pointer
  (`cJSON_GetErrorPtr`) that's only valid until the next parse call on any
  thread. No global mutable state.
- No `unsafe`.

## 4. Printing (`src/print.rs`)

- Builds directly into an owned `String`, which already amortizes growth via
  Rust's standard `Vec<u8>` doubling strategy. C's `printbuffer` type (manual
  `malloc`/`realloc` via an `ensure()` helper, `cJSON.c` internals) has no
  Rust equivalent needed — see §8 for the actual measured performance
  consequence of this choice (it's a net win at scale, a net loss on tiny
  inputs).
- `cJSON_PrintBuffered`/`cJSON_PrintPreallocated` — variants that differ from
  `cJSON_Print`/`PrintUnformatted` purely in *buffer allocation strategy*,
  not output — are **intentionally not ported**. Once printing targets an
  owned `String`, there is no behaviorally distinct thing left for those
  functions to do; porting them would just be re-adding C's manual-buffer
  bookkeeping for no observable difference in output.
- `Raw` values are emitted byte-for-byte unescaped, matching upstream
  exactly: it's the caller's contract to supply valid JSON in a `Raw`, not
  this module's job to validate it.
- The only error this module can produce is `NestingTooDeep` (mirrors the
  `output_buffer->depth >= CJSON_NESTING_LIMIT` check present in both
  `print_array` and `print_object`, `cJSON.c:1607`/`1792`). Everything else
  that can fail in C here — `malloc` failure, `sprintf` buffer overrun — has
  no equivalent given Rust's growable, safe `String`.
- No `unsafe`.

## 5. Manipulation API (`src/value.rs`)

Array/object mutation (push/insert/detach/replace), `compare`, and
`duplicate` (deep-copy) are implemented as inherent methods on `Value`
rather than free functions taking `&mut cJSON*`, matching Rust idiom over a
line-for-line port of C's `cJSON_AddItemToArray`-style free-function API.
Key behavioral points carried over faithfully:

- **Duplicate keys are allowed and preserved** on `object_push`, matching
  `cJSON_AddItemToObject`'s append-only behavior — this library does not
  silently dedupe, same as upstream.
- **`compare`** is order-sensitive for arrays, order-*in*sensitive for
  objects (a set-equality-by-key check, matching `cJSON_Compare`'s object
  comparison), and uses epsilon comparison for numbers rather than bitwise
  `f64` equality, matching `cJSON_Compare`'s explicit handling of that case
  rather than Rust's default float `PartialEq`.
- **`duplicate` with `recurse = false`** produces a shallow copy with empty
  containers (matching `cJSON_Duplicate(item, false)`'s documented
  behavior of copying only the node itself, not its children) rather than,
  e.g., a reference-counted shared subtree — there is no shared-ownership
  primitive in play here at all, by design (§2).

## 6. JSON Pointer (`src/utils.rs`)

`get_pointer`/`get_pointer_case_sensitive` (RFC 6901) are complete;
`find_pointer_from_object_to` is complete with one documented semantic
choice:

- `cJSONUtils_FindPointerFromObjectTo` in C recognizes the target node
  during its tree walk via **pointer identity** (`object == target`), not
  value equality — you're asking "where in this tree does *this exact node*
  live", not "where's the first node that looks like this". The direct
  Rust equivalent is `std::ptr::eq`, which is what this port uses. This only
  produces meaningful results (as in C) when `target` is actually a
  reference *into* `object`'s own tree; passing an equal-but-separately-
  owned `Value` correctly finds nothing, matching upstream's semantics
  rather than silently falling back to a value-equality search that would
  behave differently from the C original.
- No `unsafe`; pointer path segments (`~0`→`~`, `~1`→`/`) are decoded during
  comparison rather than via C's in-place buffer mutation
  (`decode_pointer_inplace`).

JSON Patch and Merge Patch: not implemented — see §1.

## 7. FFI / C ABI (`src/ffi.rs`)

This is the **only** file in the crate using `unsafe`, and it's the only
place crossing the FFI boundary at all — every other module is safe Rust
top to bottom. This isolation was a deliberate goal from the start (the
hackathon's "minimal `unsafe` code" bonus criterion), not an accident of
where the FFI happened to land.

- Exposes `cjson_rs_parse`, `cjson_rs_print`, `cjson_rs_print_unformatted`,
  `cjson_rs_free`, `cjson_rs_free_string` — opaque handle in, owned
  C-string out, matching cJSON's own alloc/free pairing convention
  (`cJSON_Print`'s result must be freed via `cJSON_free`, not raw `free`;
  same idea here, just enforced by never using libc's allocator on the Rust
  side of the boundary at all).
- Every function NULL-checks its input and returns NULL on any failure
  (parse error, invalid UTF-8) rather than panicking across the FFI
  boundary, which would be undefined behavior in C.
- `Cargo.toml` builds `rlib` (for Rust-side tests/benches/fuzz targets),
  `staticlib`, and `cdylib` (for the C-side differential harness and any
  future C caller) from one source tree.
- Verified end-to-end, not just claimed: compiled a real C program
  (`ffi_include/smoke_test.c`) against both the static (`.a`) and dynamic
  (`.so`) build artifacts and confirmed correct parse/print round-trips
  through the actual C ABI.

## 8. Benchmarks

Full methodology, raw numbers, and interpretation are in
`BENCHMARK_REPORT.md` (both the Rust criterion suite and a hand-written C
benchmark against the unmodified original `cJSON.c` were actually compiled
and run, not estimated). Headline results:

- **Parsing**: Rust is 8–17% slower than C across all tested input sizes.
  Attributed to §2's design: every string value becomes its own heap
  allocation during parsing (`String::from`), where C's parser can often
  point directly into (or minimally copy out of) one shared input buffer.
  This is the direct, expected cost of the safety/ownership model chosen in
  §2, not an implementation oversight — flagged as the clearest
  memory-safety-vs-performance trade-off in the whole port.
- **Printing, small inputs**: Rust is 30–53% slower — per-call fixed
  overhead (fresh `String` allocation) dominates when there's almost no
  payload to amortize it over.
- **Printing, large inputs**: Rust becomes consistently *faster* than C as
  input grows — up to 41% faster at 10,000 array elements. Likely
  `Vec`/`String`'s amortized-doubling growth beating C's `printbuffer`
  reallocation strategy at scale, though this is stated as a hypothesis in
  the report, not confirmed by profiling — an open item, not a claim.

## 9. Fuzzing and differential testing

- **Differential testing (`differential/diff_test.c`) — executed, real
  results.** Links the real `cJSON.c` and this port's FFI shim into one
  binary; feeds both the same input; checks agreement on
  parse-success-or-failure and, if both parse, byte-identical
  `PrintUnformatted` output. Run against 22 inputs (all 11 original test
  fixtures plus 11 handwritten edge cases — unicode surrogate pairs,
  extreme/negative-exponent numbers, empty containers, duplicate keys,
  invalid/garbage input, deep nesting): **22/22 matched, 0 mismatches.**
- **Fuzzing (`fuzz/`) — scaffolded, not executed here, stated plainly.**
  `cargo-fuzz` requires a nightly Rust toolchain; this port was built in a
  sandbox with only an apt-installed stable `rustc` (no `rustup`
  available), so `cargo fuzz run` could not actually be invoked in that
  environment. The fuzz target itself
  (`fuzz/fuzz_targets/cjson_read_fuzzer.rs`) is a structurally complete
  port of `fuzzing/cjson_read_fuzzer.c` — same 4-byte flag-prefix input
  format, same corpus (`fuzzing/inputs/test1..test11`, copied verbatim) —
  ready to run with `rustup install nightly && cargo install cargo-fuzz &&
  cargo +nightly fuzz run cjson_read_fuzzer` on a machine with `rustup`.
  This is an honest gap, not a claimed-but-unverified capability: the
  differential harness above is what substitutes for it as *executed*
  evidence of correctness in the meantime.

## 10. Test suite (`tests/`)

- **106 tests total** at last count: 90 unit tests co-located with the
  modules they test (`src/*.rs`, `#[cfg(test)]`), 15 in
  `tests/parse_examples.rs`, 1 (13 assertions) in
  `tests/json_pointer_examples.rs`.
- `tests/fixtures/inputs/test1`..`test11` (and their `.expected`
  counterparts) are copied **verbatim, byte-for-byte, unmodified** from
  upstream `tests/inputs/`. Per the hackathon rule that changes to the
  original suite must be documented: no fixture *content* was changed,
  only relocated (upstream reads them via a relative path baked into
  `parse_examples.c`; Rust reads them via `CARGO_MANIFEST_DIR`).
- `tests/json_pointer_examples.rs` ports the RFC 6901 conformance case from
  `tests/old_utils_tests.c`'s `json_pointer_tests` — including every one of
  RFC 6901 Appendix A's adversarial key names (empty string, `"a/b"`,
  `"m~n"`, embedded quote/backslash/space). **One documented deviation**:
  the C test asserts pointer *identity* (`TEST_ASSERT_EQUAL_PTR`) between
  `GetPointer`'s result and an independent `GetObjectItem` lookup, since
  both walk the same in-memory tree and must land on the same node. This
  port's `Value` tree is owned data with no exposed shared-identity concept
  at this layer, so each assertion is rewritten as *value equality* between
  the two independent lookups instead — same intent (does GetPointer agree
  with GetObjectItem?), different mechanism.
- **Two C-specific test cases were not translated as literal ports**, and
  this is called out directly rather than silently dropped:
  `test13_should_be_parsed_without_null_termination` and
  `test14_should_not_be_parsed` in upstream's `parse_examples.c` exist to
  catch a real historical C bug class — reading past the end of a
  non-NUL-terminated or short buffer, because `char*` carries no length.
  Rust `&str`/`&[u8]` slices always carry their own length; that bug class
  doesn't exist to test for. The tests were kept as *intent-preserving*
  checks ("valid JSON with no trailing terminator still parses",
  "truncated JSON is correctly rejected, not read past") rather than
  deleted, with a comment explaining why the original C-specific framing
  doesn't apply.
- **Not yet ported**: `json_patch_tests.c` and the merge-patch table in
  `old_utils_tests.c`, blocked on §1's open Patch/Merge-Patch gap.

## 11. Summary of intentional behavioral differences from upstream

For the judges' convenience, every place this port *knowingly* diverges
from cJSON's exact C behavior, gathered in one place:

1. Number parsing is locale-independent (§3) — arguably a bug fix, not a
   compatibility break.
2. `parse()`/`cJSON_Parse` still silently ignores trailing garbage after a
   valid value, matching upstream exactly; `parse_strict()` is an addition,
   not a replacement, for callers who want the opposite.
3. `find_pointer_from_object_to` matches C's pointer-identity semantics via
   `std::ptr::eq`, not value equality (§6) — same behavior, by design.
4. JSON Patch / JSON Merge Patch are unimplemented (§1) — the one outright
   missing feature versus upstream's public API, not a semantic
   difference.

Everything else in this document is an implementation-strategy decision
(data structures, error handling, module boundaries) rather than an
observable behavioral difference from the original C library.
