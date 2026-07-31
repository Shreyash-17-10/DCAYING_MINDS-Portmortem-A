#include <stdio.h>
#include <string.h>
#include <assert.h>
#include "cjson_rs.h"

int main(void) {
    const char *json = "{\"a\":1,\"b\":[true,null,\"x\"]}";

    CJsonRsValue *v = cjson_rs_parse(json);
    assert(v != NULL);

    char *compact = cjson_rs_print_unformatted(v);
    assert(compact != NULL);
    printf("unformatted: %s\n", compact);
    assert(strcmp(compact, json) == 0);

    char *pretty = cjson_rs_print(v);
    assert(pretty != NULL);
    printf("formatted:\n%s\n", pretty);

    cjson_rs_free_string(compact);
    cjson_rs_free_string(pretty);
    cjson_rs_free(v);

    /* NULL / invalid-input safety */
    assert(cjson_rs_parse(NULL) == NULL);
    assert(cjson_rs_parse("{not valid") == NULL);
    cjson_rs_free(NULL);
    cjson_rs_free_string(NULL);

    printf("C ABI smoke test: PASSED\n");
    return 0;
}
