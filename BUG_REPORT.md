# Bug Report — cJSON's `cJSONUtils_GeneratePatches` silently ignores content changes in `cJSON_Raw` values

**Component:** `cJSON_Utils.c`, `create_patches` (static, called by
`cJSONUtils_GeneratePatches` / `cJSONUtils_GeneratePatchesCaseSensitive`)
**Version affected:** 1.7.19 (current `master` at time of writing; the bug
has been present since `cJSON_Raw` and JSON-Patch generation were both
added and is not version-specific to any recent change)
**Severity:** Low-to-moderate — silent incorrect output, not a crash or
memory-safety issue. Affects only applications that (a) use `cJSON_Raw`
nodes and (b) diff documents via `cJSONUtils_GeneratePatches`.

## Summary

`create_patches` diffs two `cJSON` trees and generates the RFC 6902 JSON
Patch operations needed to turn one into the other. Its type-dispatch
`switch` statement has cases for `cJSON_Number`, `cJSON_String`,
`cJSON_Array`, and `cJSON_Object` — but **no case for `cJSON_Raw`**. Two
`cJSON_Raw` nodes with the same type tag but different `valuestring`
content therefore produce **no patch operation at all**, even though the
two documents are genuinely different.

## Root cause

`cJSON_Utils.c`, `create_patches`, lines 1015–1134:

```c
switch (from->type & 0xFF) {
case cJSON_Number:
  /* ... compares from->valueint / from->valuedouble, emits "replace" if different ... */
  return;

case cJSON_String:
  if (strcmp(from->valuestring, to->valuestring) != 0) {
    compose_patch(patches, (const unsigned char *)"replace", path, NULL, to);
  }
  return;

case cJSON_Array: { /* ... */ }

case cJSON_Object: { /* ... */ }

default:
  break;
}
```

`cJSON_Raw` (like `cJSON_String`) stores its payload in `->valuestring`
(see `cJSON_CreateRaw`, `cJSON.c`), so a content comparison identical to
the `cJSON_String` case would be the natural, correct handling. Instead,
because there's no `case cJSON_Raw:`, execution falls to `default: break;`
— a silent no-op. (Note: `cJSON_NULL`, `cJSON_True`, and `cJSON_False`
also lack explicit cases and share this `default:` fallthrough, but this
is *correct* for them: those types carry no payload beyond their type tag,
which the earlier `(from->type & 0xFF) != (to->type & 0xFF)` check at line
1010 already catches when they differ. `cJSON_Raw` is the only type in
this fallthrough that has content the switch fails to inspect.)

## Reproduction

```c
#include "cJSON.h"
#include "cJSON_Utils.h"

int main(void) {
    cJSON *a = cJSON_CreateObject();
    cJSON_AddItemToObject(a, "r", cJSON_CreateRaw("[1,2]"));
    cJSON *b = cJSON_CreateObject();
    cJSON_AddItemToObject(b, "r", cJSON_CreateRaw("[1,2,3]"));

    cJSON *patches = cJSONUtils_GeneratePatches(a, b);
    printf("%s\n", cJSON_PrintUnformatted(patches)); /* prints "[]" */
    printf("%d\n", cJSON_GetArraySize(patches));      /* prints "0" */
    return 0;
}
```

**Expected:** a single `replace` operation for `/r`, since the two
documents' `r` field genuinely differs (`[1,2]` vs `[1,2,3]`).

**Actual:** `cJSONUtils_GeneratePatches` returns an empty patch array —
i.e. it reports the two documents as identical, when they are not.

Verified by compiling and running the above against an unmodified,
freshly-cloned `cJSON.c`/`cJSON_Utils.c` (not a hypothetical — this was
built and executed, output captured verbatim above).

## Suggested fix

Add an explicit case in the `switch` at `cJSON_Utils.c:1015`:

```c
case cJSON_Raw:
  if (strcmp(from->valuestring, to->valuestring) != 0) {
    compose_patch(patches, (const unsigned char *)"replace", path, NULL, to);
  }
  return;
```

(Identical to the existing `cJSON_String` case, since `cJSON_Raw` stores
its content the same way.)

## How this was found

Discovered while porting cJSON to Rust for Port Mortem 2026 (a C→Rust
migration hackathon). The port's own `create_patches` equivalent initially
used a single unified comparison helper for all scalar leaf types
(`Null`/`Bool`/`Number`/`String`/`Raw`), which — as a side effect —
correctly diffed `Raw` content where this bug does not. Per the
hackathon's Behavioral Equivalence requirement, the port was subsequently
changed to faithfully reproduce this exact upstream behavior (silent
no-patch for differing `Raw` content) rather than "fixing" it
unilaterally, and this bug is reported here separately instead, per the
project rules. See the Rust port's `DECISIONS.md` §6c and its test
`generate_patches_raw_content_change_produces_no_patch_matching_upstream_bug`
for where this is documented and verified in that codebase.

## Where to file this upstream

https://github.com/DaveGamble/cJSON/issues (not yet filed as of this
report; filing is left to the hackathon participant/submitter).
