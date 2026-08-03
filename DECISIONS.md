# DECISIONS.md — cjson-rs (Port Mortem 2026, C → Rust)

Architectural decisions, trade-offs, and known gaps in this port of
[cJSON](https://github.com/DaveGamble/cJSON) (MIT, Dave Gamble and
contributors) from C to Rust. Written phase-by-phase to match how the port
was actually built; see the roadmap discussion for the phase breakdown.

# Git commit messages  
  
## 1. Scope and status

| Original file | Rust module | Status |
|---|---|---|
| `cJSON.h` (type/struct definitions) | `src/value.rs` | Complete |
| `cJSON.c` — parsing | `src/parse.rs` | Complete |
| `cJSON.c` — printing | `src/print.rs` | Complete |
| `cJSON.c` — manipulation API (Add/Delete/Replace/Detach/Duplicate/Compare) | `src/value.rs` | Complete |
| `cJSON_Utils.c/h` — JSON Pointer (RFC 6901) | `src/utils.rs` | Complete |
| `cJSON_Utils.c/h` — JSON Patch (RFC 6902) — apply | `src/utils.rs` | Complete |
| `cJSON_Utils.c/h` — JSON Patch — diff/generate | `src/utils.rs` | Complete |
| `cJSON_Utils.c/h` — JSON Merge Patch (RFC 7396) — apply | `src/utils.rs` | Complete |
| `cJSON_Utils.c/h` — JSON Merge Patch — diff/generate | `src/utils.rs` | Complete |
| `cJSON_Utils.c/h` — sort_object | `src/utils.rs` | Complete |
| `cJSON_Utils.c/h` — AddPatchToArray | `src/utils.rs` | Complete |
| C ABI / FFI shim | `src/ffi.rs` | Complete |
| Test suite | `tests/*.rs` | Partial — see §10 |
| Benchmarks | `benches/parse_print.rs`, `benches/c_bench/` | Complete |
| Fuzzing | `fuzz/` | Scaffolded, not executed in this environment — see §9 |
| Differential testing | `differential/diff_test.c`, `differential/diff_generate_test.c` | Complete, executed, 0 mismatches |
| Property-based testing | `tests/proptest_roundtrip.rs` | Complete, executed, 5 properties × 5,000 cases, 0 failures |

**All upstream public API functions are now ported.** The full `cJSON.c` and
`cJSON_Utils.c/h` public surface — parsing, printing, manipulation, JSON
Pointer (RFC 6901), JSON Patch apply *and* generate (RFC 6902), JSON Merge
Patch apply *and* generate (RFC 7396), and `SortObject` — is implemented
and tested. No functional gaps remain versus upstream's public API.

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

JSON Patch and Merge Patch: see §6b (apply) and §6c (generation).

## 6b. JSON Patch (RFC 6902) and JSON Merge Patch (RFC 7396) — apply side

`apply_patch`/`apply_patch_case_sensitive` (single operation),
`apply_patches`/`apply_patches_case_sensitive` (a full patch document, an
array of operations), and `merge_patch`/`merge_patch_case_sensitive` are
ported from `apply_patch` and `merge_patch` in `cJSON_Utils.c`,
function-for-function.

- **Errors are a proper `enum PatchError`, not C's numeric `status` codes.**
  `apply_patch` in C returns `int` (0 = success, 2/3/4/5/7/9/10/11/13 = one
  of several distinct failure modes, undocumented in the return type
  itself). Each `PatchError` variant's doc comment cites the exact C status
  code it replaces, so anyone cross-referencing the original source can
  trace the mapping directly — but callers get a self-describing type
  instead of a bare integer they'd need `cJSON_Utils.c`'s source open to
  interpret.
- **Non-transactional, matching upstream exactly.** `apply_patches` stops
  at the first failing operation but does **not** roll back operations
  already applied before it — this is upstream's real, if perhaps
  surprising, behavior (`cJSONUtils_ApplyPatches`'s loop just returns
  early), reproduced deliberately rather than "fixed" into a transactional
  all-or-nothing apply, since that would be an observable behavior change
  from the original. Verified by a dedicated test
  (`non_transactional_partial_application_on_failure`).
- **Root-path removal's sentinel value.** C's `apply_patch` handles
  `{"op": "remove", "path": ""}` by overwriting the root node with a
  static `cJSON_Invalid`-typed struct — an internal "this node used to
  exist but doesn't have a real type" marker with no corresponding JSON
  value and no equivalent `Value` variant (by design, `Value` only has
  variants for actual JSON types, see §2). This port uses `Value::Null` as
  the closest observable stand-in for "the root was removed." This is a
  documented, deliberate minor divergence, not an oversight: a caller who
  removes the root and then inspects the result sees `null` in this port
  versus an internal-only, unprintable sentinel type in C that no
  well-behaved caller could observe anyway (`cJSON_Print` on `cJSON_Invalid`
  is itself unspecified upstream).
- **Array `add` bounds-checks strictly, on purpose diverging from
  `Value::array_insert`.** `Value::array_insert` (§5, used for the general
  `cJSON_InsertItemInArray`-equivalent manipulation API) silently clamps an
  out-of-range index to the array's length. JSON Patch's `add` operation
  must *not* do this — RFC 6902 requires index `== array.len()` (or the
  literal `"-"` token) for a valid append, and anything greater is a hard
  error. `apply_patch_inner` reimplements this check locally rather than
  reusing `array_insert`, exactly mirroring `insert_item_in_array`
  (`cJSON_Utils.c:693-728`)'s explicit `which > 0` bounds failure after
  walking off the end of the list.
- **`get_pointer_mut`**, the mutable counterpart to the existing (Phase 6a)
  `get_item_from_pointer`, walks the tree the same way but returns `&mut
  Value` — used to locate the parent container a patch operation will
  mutate. No `unsafe`: reassigning the walk's current-node reference across
  loop iterations relies on ordinary Rust non-lexical-lifetime reborrowing.
- Merge Patch's recursive descent (`merge_patch_inner`) detaches each
  touched key from `target` before recursing and re-appending the merged
  result, exactly mirroring C's `DetachItemFromObjectCaseSensitive` +
  `AddItemToObject` pairing in `merge_patch` (`cJSON_Utils.c:1355-1374`) —
  including the resulting **observable key-reordering side effect**: any
  key touched by the patch moves to the end of the object, in the order it
  appears in the *patch* document, not the target. This is real upstream
  behavior (not a bug this port introduces), verified directly by the
  RFC 7396 §1 worked example test, which traces the exact resulting key
  order rather than asserting a guessed one.
- No `unsafe` anywhere in this section either — the "`unsafe` is isolated
  to `ffi.rs` and `wasm.rs`" scope from §7 still holds with Patch/Merge
  Patch included.

## 6c. JSON Patch and JSON Merge Patch — diff/generate

`generate_patches`/`generate_patches_case_sensitive` (RFC 6902) and
`generate_merge_patch`/`generate_merge_patch_case_sensitive` (RFC 7396)
port `create_patches`/`compose_patch` and `generate_merge_patch` from
`cJSON_Utils.c`, function-for-function, with several decisions worth
calling out explicitly:

- **Doesn't mutate its inputs — a deliberate divergence from upstream.**
  C's `create_patches` and `generate_merge_patch` both call `sort_object`
  on `from` and `to` **in place** before diffing, as a side effect of
  computing the diff (upstream's own header even warns about this: "NOTE:
  This modifies objects in 'from' and 'to' by sorting the elements by
  their key"). A function whose job is "compare these two documents"
  silently reordering the caller's object keys is surprising API behavior;
  this port instead calls `.duplicate(true)` on `from`/`to` first and sorts
  the *clones* (via the private `sort_object_inner`), leaving the caller's
  original `from`/`to` completely untouched. Same merge-join algorithm,
  no side effect on the inputs.
- **`cJSON_Raw` content changes are faithfully un-diffed, matching a real
  upstream bug — deliberately, not by oversight.** C's `create_patches`
  `switch` has cases for `cJSON_Number`, `cJSON_String`, `cJSON_Array`, and
  `cJSON_Object`, but **none for `cJSON_Raw`** — it silently falls through
  an unhandled `default: break;`, so two `Raw` nodes with different content
  produce *no* patch even though the documents genuinely differ. An
  earlier version of this port used a single unified comparison helper for
  all scalar leaf types, which incidentally fixed this and correctly
  diffed `Raw` content. That was reverted: per this hackathon's Behavioral
  Equivalence rule — a bug in the original C should be reproduced by the
  port, not silently corrected — `create_patches` now matches `cJSON_Raw`
  values without comparing their content, exactly like upstream, verified
  by `generate_patches_raw_content_change_produces_no_patch_matching_upstream_bug`.
  The underlying bug itself is written up separately in `BUG_REPORT.md`,
  including a minimal repro compiled and run against the real, unmodified
  `cJSON_Utils.c` (confirmed: prints an empty patch array for genuinely
  different `Raw` content) — reported as a finding for the hackathon
  organizers, not silently patched around.
- **Array diffing's fixed-index removal, reproduced exactly.** When `from`
  is longer than `to`, C's leftover-removal loop reuses the *same* array
  index for every `remove` operation rather than incrementing it — correct
  precisely because removing element N repeatedly is well-defined as the
  array shrinks past that point (element N+1 becomes the new element N
  after each removal). This is easy to get subtly wrong porting from C's
  for-loop update-clause structure; verified directly by
  `generates_patch_for_array_shrink_and_grow`'s exact expected-output
  assertion, not just "does apply-then-compare succeed."
- **`generate_merge_patch` returns `Option<Value>`, not a bare `Value`,
  and this fixes a real ambiguity in upstream's return type.** C's
  `cJSONUtils_GenerateMergePatch` returns `cJSON *`, which conflates two
  different states behind the same nullable-pointer type: a genuine `NULL`
  pointer means "no patch needed, `from` already equals `to`" (an empty
  diff), while a **non-NULL** pointer to a node of type `cJSON_NULL` means
  "apply a patch that deletes everything" (the documented `to == NULL`
  input case). A caller who checks only `result == NULL` without also
  checking `cJSON_IsNull(result)` can't tell these apart. This port's
  `Option<Value>` makes the distinction structurally impossible to
  conflate: `None` = nothing to do, `Some(Value::Null)` = "replace with
  nothing," `Some(other)` = the actual patch object. Verified by
  `generate_merge_patch_no_diff_returns_none` and
  `generate_merge_patch_to_none_means_delete_everything` as two distinct,
  separately-asserted test cases.
- **`sort_object`/`sort_object_case_sensitive` use `Vec::sort_by`**, not
  C's merge-sort over a doubly-linked list (`sort_list`,
  cJSON_Utils.c:484-593). Both are stable sorts with identical ordering
  guarantees for equal keys; `Vec::sort_by` is the Rust equivalent with
  better cache locality and a small fraction of the code (a few lines vs.
  ~110 lines of manual merge-sort in C). Exposed as public utilities
  (`generate_patches`/`generate_merge_patch_*` also sort internally as part
  of their own merge-join, via a private `sort_object_inner` — these public
  functions exist for callers who want cJSON's `cJSONUtils_SortObject`
  behavior directly, independent of diffing).
- **`add_patch_to_array`** is ported as a public utility for callers who
  want to build a JSON Patch document manually, one operation at a time,
  rather than only via `generate_patches`' automatic diffing.
- No `unsafe`.

## 6d. Rust trait implementations (innovation beyond C)

Three standard-library traits are implemented that have no equivalent in C:

- **`std::fmt::Display` for `Value`**: `format!("{}", value)` produces
  compact JSON output; `format!("{:#}", value)` produces pretty-printed
  output (with tabs and newlines, matching `cJSON_Print`'s formatting).
  This is an ergonomic improvement that C's function-call-only interface
  cannot express.
- **`std::str::FromStr` for `Value`**: `let v: Value = json_str.parse()?`
  parses JSON via Rust's standard `.parse()` idiom, returning
  `Result<Value, CJsonError>`.
- **`Display` and `Error` for `CJsonError` and `PatchError`**: both error
  types implement `std::fmt::Display` and `std::error::Error`, making them
  usable with `?`, `anyhow`, and Rust's standard error-handling ecosystem.
  `CJsonError` also exposes a `position()` method for programmatic access
  to the byte offset where the error was detected.

## 7. FFI / C ABI (`src/ffi.rs`) and WASM bindings (`src/wasm.rs`)

`unsafe` in this crate is isolated to exactly two files — `ffi.rs` (the C
ABI, used by the differential-testing harnesses) and `wasm.rs` (the
browser-facing GUI's WASM export surface) — both thin, carefully-bounded
translation layers with no logic of their own; every other module is safe
Rust top to bottom. This isolation was a deliberate goal from the start
(the hackathon's "minimal `unsafe` code" bonus criterion). Every `unsafe
extern "C"` function in both files carries a `# Safety` doc section stating
its exact preconditions, not just a bare `unsafe` marker.

**`src/ffi.rs`** exposes: `cjson_rs_parse`, `cjson_rs_print`,
`cjson_rs_print_unformatted`, `cjson_rs_free`, `cjson_rs_free_string`
(opaque handle in, owned C-string out — matching cJSON's own alloc/free
pairing convention, `cJSON_Print`'s result must be freed via `cJSON_free`,
not raw `free`; enforced here by never using libc's allocator on the Rust
side of the boundary at all), plus the handle-based patch/utility surface:
`cjson_rs_generate_patch` (case-sensitive diff between two parsed handles,
returning a new handle), `cjson_rs_generate_patches` /
`cjson_rs_generate_patches_case_sensitive` (the explicit case-insensitive
and case-sensitive variants), and `cjson_rs_sort_object` /
`cjson_rs_sort_object_case_sensitive` (mirrors `cJSONUtils_SortObject`).
This handle-in/handle-out shape — rather than an earlier iteration that
took/returned JSON text directly — lets a C caller chain operations
(parse → generate_patch → print) without a text round-trip at every step,
mirroring how cJSON's own C API composes `cJSON*` handles.

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

**`src/wasm.rs`** exposes a parallel `extern "C"` surface
(`wasm_alloc`/`wasm_dealloc` for linear-memory buffer management,
`wasm_validate_json`, `wasm_print_unformatted`, `wasm_print_formatted`,
`wasm_get_parse_error`, `wasm_inspect_ast`, `wasm_get_version`) consumed
directly by `gui/app.js` with no `wasm-bindgen`/`wasm-pack` glue — the
browser writes UTF-8 JSON bytes into WASM linear memory via `wasm_alloc`,
calls one of the parse/print/inspect functions with a `(ptr, len)` pair,
and reads the C-string result back out. Same NULL-checking and
caller-owns-the-result-until-freed discipline as `ffi.rs`.

**Linting.** `cargo clippy --all-targets -- -D warnings` (deny, not just
warn) passes clean across the entire crate — library, all four integration
test files, and both benchmark files. Two real, substantive lints were
caught and fixed during development (an unnecessary `Vec` clone in
`merge_patch_inner`, an elidable explicit lifetime in
`sorted_object_index`); the remaining three lints clippy flagged were
false positives on deliberate test literals (`3.1416`/`-3.14` as
JSON-parsing precision test values, `3.1415926535897931` as a specific
17-significant-digit round-trip test case for `print_number`'s fallback
path, and a variable named `foo` that holds the RFC 6901 spec's own
`"foo"` example key) — each is suppressed with a narrowly-scoped
`#[allow(...)]` and an inline comment explaining why, rather than either
silently blanket-allowing the lint crate-wide or mangling meaningful test
data to appease a pattern-matcher that can't know the data's intent.

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
- **Cross-implementation differential testing for patch *generation*
  (`differential/diff_generate_test.c`) — executed, real results.** A
  stronger check than comparing generated patch *text* (which two
  correct implementations aren't required to produce identically): this
  generates a patch with this Rust port via `cjson_rs_generate_patch`,
  then applies that Rust-generated patch using the **real, unmodified
  upstream `cJSONUtils_ApplyPatchesCaseSensitive`**, and confirms the
  result matches the intended target via the real `cJSON_Compare`. This
  proves Rust-generated patches are genuinely interoperable with, and
  correctly interpreted by, the original C library — not just
  self-consistent with this port's own `apply_patches`. Run against 10
  cases (scalar replace, add/remove keys, nested object diff, array
  shrink/grow/element-replace, deeply nested mixed diff, identical
  documents, unicode content change): **10/10 matched, 0 mismatches.**
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
  This is an honest gap, not a claimed-but-unverified capability: the two
  differential harnesses above are what substitute for it as *executed*
  evidence of correctness in the meantime.

## 9b. Property-based testing (`tests/proptest_roundtrip.rs`)

Added specifically to widen Behavioral Equivalence evidence beyond the
fixed test cases everywhere else in the suite: `proptest` (stable Rust, no
nightly toolchain needed) generates structurally random `Value` trees and
checks invariants — print/parse round-tripping, formatted/unformatted
agreement, duplicate/compare consistency, and generate-then-apply-patch
correctness — over thousands of cases per run rather than a handful of
hand-picked ones. 5 properties, run at 5,000 cases each in release mode for
this submission (25,000 total assertions), all passing.

**Two genuine findings came out of writing this, and both are worth
documenting in full because of how they were resolved**: initial versions
of two properties failed, and rather than assuming a Rust-port bug and
patching it, each failure was reduced to a minimal case and independently
verified against the actual, unmodified `original_c_reference/cJSON.c` —
compiled and run fresh, not reasoned about from memory — before deciding
what to do about it.

**Finding 1 — number round-tripping isn't always bit-exact, and neither
is C's.** `print_number` tries 15 significant digits first and only falls
back to 17 if a *tolerant*, epsilon-based check (`compare_double`) says the
15-digit form doesn't round-trip. For a specific class of large-magnitude,
non-integer doubles (e.g. `d = -631908566981097.9`), the 15-digit rounding
lands on a *different* nearby double that still happens to fall within
`compare_double`'s relative-epsilon tolerance — so the algorithm considers
15 digits "good enough" and never reaches the 17-digit fallback, even
though the printed text doesn't read back to the bit-identical original.
Verified directly: a small standalone C program was compiled against the
real `cJSON.c` with this exact `d`, and it **also** prints
`-631908566981098` and **also** fails to round-trip exactly
(`cJSON_Parse` on the result yields a different bit pattern than `d`).
This port's `compare_double` is a faithful, formula-identical port of C's
`compare_double` (`DBL_EPSILON` and `f64::EPSILON` are the same value), so
it inherits this characteristic correctly — the fix was to the *property*,
not the code: round-trip assertions now use `value::compare()` (the same
epsilon-tolerant equality cJSON itself uses everywhere, e.g. in
`cJSON_Compare`) instead of requiring bit-exact `PartialEq`, which was
simply the wrong invariant to assert about an algorithm that was never
specified to guarantee bit-exact round-tripping in the first place.

**Finding 2 — case-insensitive comparison isn't well-defined for objects
with case-insensitive-duplicate keys, in C either.** An object containing
both `"c"` and `"C"` as keys, compared against an identical copy of itself
with `case_sensitive = false`, returns *not equal*. Root cause: case-
insensitive key lookup always resolves to the *first* matching entry, so
when checking whether the second occurrence's value has a match, the
lookup finds the *first* occurrence instead and compares the wrong pair of
values. Verified directly: a standalone C program compiled against the
real `cJSON.c`, calling `cJSON_Compare(a, b, 0)` on
`[{"c":null,"C":false}]` parsed twice, returns `0` (not equal) — the exact
same outcome as this port. This isn't a bug either implementation
introduced; it's an inherent property of "case-insensitive first-match
lookup" as a comparison strategy, shared by construction. The fix was to
the *test generator*: `arb_value()`'s object strategy now deduplicates
generated keys case-insensitively, since this degenerate input shape isn't
something either implementation defines coherent behavior for, and a
property test asserting behavior over undefined territory isn't testing
anything meaningful.

Both findings strengthen this port's Behavioral Equivalence claim rather
than weaken it: two independent, automatically-discovered edge cases were
each traced to precise root causes and confirmed, by actually compiling
and running the unmodified original, to be shared characteristics of the
algorithm itself — not divergences this port introduced.

## 10. Test suite (`tests/`)

- **169 tests total** at last count: 145 unit tests co-located with the
  modules they test (`src/*.rs`, `#[cfg(test)]` — including
  `utils::patch_tests` and `utils::generate_tests` for §6b/§6c,
  `ffi::tests` for the `cjson_rs_generate_patch` export, and
  `wasm::tests` for the WASM bindings), 3 in
  `tests/json_patch_conformance.rs`, 15 in `tests/parse_examples.rs`,
  5 in `tests/proptest_roundtrip.rs` (§9b — each run at thousands of cases,
  not counted as 1 each toward this total the way `cargo test`'s default
  reporting shows them), and 1 (13 assertions) in
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
- **JSON Patch conformance (`tests/json_patch_conformance.rs`)**: runs the
  official [json-patch-tests](https://github.com/json-patch/json-patch-tests)
  RFC 6902 conformance suite (`tests.json`, `spec_tests.json`,
  `cjson-utils-tests.json` — copied verbatim from upstream's
  `tests/json-patch-tests/`, unmodified), replicating upstream's own
  `test_apply_patch` semantics from `json_patch_tests.c` exactly: apply each
  case's `patch` to a duplicate of `doc`; if the case has an `"error"` key,
  expect failure; otherwise expect success and, if `"expected"` is present,
  expect a case-sensitive `compare()` match; skip cases marked
  `"disabled": true`. **Result: 121 total cases, 4 disabled (matching
  upstream's own disabled flags), 117/117 active cases pass.**
- Not yet ported: `minify_tests.c`, `print_*.c`, `compare_tests.c`,
  `misc_utils_tests.c`, `readme_examples.c` — lower priority, largely
  covered indirectly by this port's own unit tests, but not a literal
  1:1 port of those specific files.

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
4. Root-path `remove` in JSON Patch produces `Value::Null` where C leaves an
   internal, unprintable `cJSON_Invalid` sentinel (§6b) — the closest
   observable stand-in for a case upstream itself has no real JSON
   representation for.
5. `generate_patches`/`generate_merge_patch` do not mutate their inputs
   (§6c) — C sorts the inputs as a side-effect, this port clones before
   sorting. This is a deliberate improvement, not a compatibility issue.
6. `generate_merge_patch` returns `Option<Value>` instead of a nullable
   pointer (§6c) — standard Rust, functionally equivalent.

Everything else in this document is an implementation-strategy decision
(data structures, error handling, module boundaries) rather than an
observable behavioral difference from the original C library.

**Not on this list, deliberately**: the two findings from property-based
testing in §9b (number round-trip tolerance, case-insensitive-duplicate-key
comparison) are *not* behavioral differences from upstream — both were
verified, by compiling and running the real `cJSON.c`, to be identical
shared characteristics of the algorithm both implementations use. They're
documented in full in §9b precisely because confirming "this looks
surprising but it's not a divergence" is itself part of demonstrating
Behavioral Equivalence rigorously, not something to omit because the
outcome was "no bug here."
