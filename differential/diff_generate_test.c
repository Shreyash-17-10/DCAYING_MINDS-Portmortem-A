/* Cross-implementation differential test for JSON Patch *generation*:
 * generates a patch with this Rust port (cjson_rs_generate_patch, via the
 * C ABI shim), then applies that Rust-generated patch using the REAL,
 * unmodified upstream cJSONUtils_ApplyPatchesCaseSensitive, and checks the
 * result equals the intended target document via the real cJSON_Compare.
 *
 * This is a stronger correctness signal than comparing generated patch
 * *text* directly (which isn't required to be byte-identical between
 * implementations to be equally valid) - it proves the Rust-generated
 * patch is genuinely interoperable with, and correctly interpreted by,
 * the original C library.
 *
 * Build:
 *   cargo build --release
 *   gcc -O2 diff_generate_test.c ../original_c_reference/cJSON.c \
 *       ../original_c_reference/cJSON_Utils.c \
 *       ../target/release/libcjson_rs.a \
 *       -I../original_c_reference -I../ffi_include \
 *       -lpthread -ldl -lm -o diff_generate_test
 *
 * Run:
 *   ./diff_generate_test
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cJSON.h"
#include "cJSON_Utils.h"
#include "cjson_rs.h"

static int run_case(const char *label, const char *from_json, const char *to_json) {
    char *patch_json = cjson_rs_generate_patch(from_json, to_json);
    if (!patch_json) {
        printf("[FAIL] %s -- Rust generate_patch returned NULL\n", label);
        return 1;
    }
    printf("  patch: %s\n", patch_json);

    cJSON *doc = cJSON_Parse(from_json);
    cJSON *patch = cJSON_Parse(patch_json);
    cJSON *expected = cJSON_Parse(to_json);

    int apply_status = cJSONUtils_ApplyPatchesCaseSensitive(doc, patch);
    int ok = 0;
    if (apply_status != 0) {
        printf("[FAIL] %s -- original C failed to apply the Rust-generated patch (status %d)\n",
               label, apply_status);
    } else if (!cJSON_Compare(doc, expected, 1)) {
        char *got = cJSON_PrintUnformatted(doc);
        char *want = cJSON_PrintUnformatted(expected);
        printf("[FAIL] %s -- applied result does not match target\n  got:  %s\n  want: %s\n",
               label, got, want);
        free(got);
        free(want);
    } else {
        printf("[MATCH] %s\n", label);
        ok = 1;
    }

    cJSON_Delete(doc);
    cJSON_Delete(patch);
    cJSON_Delete(expected);
    cjson_rs_free_string(patch_json);
    return ok ? 0 : 1;
}

int main(void) {
    int failures = 0;

    failures += run_case("scalar replace",
        "{\"foo\":\"bar\"}", "{\"foo\":\"baz\"}");

    failures += run_case("add and remove keys",
        "{\"a\":1,\"b\":2}", "{\"a\":1,\"c\":3}");

    failures += run_case("nested object diff",
        "{\"a\":1,\"b\":{\"c\":2,\"d\":3}}", "{\"a\":10,\"b\":{\"c\":2,\"d\":30,\"e\":4}}");

    failures += run_case("array shrink",
        "[\"a\",\"b\",\"c\",\"d\"]", "[\"a\",\"x\"]");

    failures += run_case("array grow",
        "[\"a\"]", "[\"a\",\"b\",\"c\"]");

    failures += run_case("array element replace",
        "[1,2,3]", "[1,99,3]");

    failures += run_case("deeply nested mixed diff",
        "{\"users\":[{\"id\":1,\"name\":\"a\"},{\"id\":2,\"name\":\"b\"}],\"count\":2}",
        "{\"users\":[{\"id\":1,\"name\":\"a\"},{\"id\":2,\"name\":\"bee\"},{\"id\":3,\"name\":\"c\"}],\"count\":3}");

    failures += run_case("identical documents produce empty patch",
        "{\"x\":1}", "{\"x\":1}");

    failures += run_case("unicode string change",
        "{\"emoji\":\"\\ud83d\\ude00\"}", "{\"emoji\":\"\\ud83d\\ude01\"}");

    printf("\n--- Summary ---\n");
    printf("%s\n", failures == 0 ? "ALL PASSED" : "SOME FAILED");
    return failures == 0 ? 0 : 1;
}
