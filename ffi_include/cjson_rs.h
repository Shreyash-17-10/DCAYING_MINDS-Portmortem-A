/* cjson_rs.h — hand-written C ABI header for cjson-rs.
 * A real cbindgen.toml is included alongside this file; run
 *   cbindgen --config cbindgen.toml --output cjson_rs_generated.h
 * to regenerate this automatically from src/ffi.rs. This hand-written
 * version exists so the differential-test harness works without requiring
 * cbindgen to be installed.
 */
#ifndef CJSON_RS_H
#define CJSON_RS_H

typedef struct CJsonRsValue CJsonRsValue; /* opaque */

CJsonRsValue *cjson_rs_parse(const char *json);
char *cjson_rs_print(const CJsonRsValue *handle);
char *cjson_rs_print_unformatted(const CJsonRsValue *handle);
void cjson_rs_free(CJsonRsValue *handle);
void cjson_rs_free_string(char *s);

#endif
