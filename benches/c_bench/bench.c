/* Simple wall-clock benchmark for the original cJSON.c, run over the same
 * fixtures and synthetic documents as benches/parse_print.rs, so the two
 * can be compared directly. See BENCHMARK_REPORT.md for results.
 *
 * Build: gcc -O3 bench.c ../../original_c_reference/cJSON.c -I../../original_c_reference -lm -o c_bench
 * Run:   ./c_bench <fixtures_dir>
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "cJSON.h"

static char *read_file(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    rewind(f);
    char *buf = (char *)malloc((size_t)size + 1);
    fread(buf, 1, (size_t)size, f);
    buf[size] = '\0';
    fclose(f);
    return buf;
}

static char *synthetic_large(int n) {
    /* generous upper bound: ~90 bytes per element */
    size_t cap = (size_t)n * 90 + 16;
    char *buf = (char *)malloc(cap);
    size_t pos = 0;
    pos += sprintf(buf + pos, "[");
    for (int i = 0; i < n; i++) {
        if (i > 0) buf[pos++] = ',';
        pos += sprintf(buf + pos,
            "{\"id\":%d,\"name\":\"item-%d\",\"active\":%s,\"score\":%.3f,\"tags\":[\"a\",\"b\",\"c\"]}",
            i, i, (i % 2 == 0) ? "true" : "false", i * 1.5);
    }
    pos += sprintf(buf + pos, "]");
    return buf;
}

static double now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec * 1000.0 + (double)ts.tv_nsec / 1e6;
}

static void bench_parse(const char *label, const char *json, int iters) {
    double start = now_ms();
    for (int i = 0; i < iters; i++) {
        cJSON *tree = cJSON_Parse(json);
        cJSON_Delete(tree);
    }
    double elapsed = now_ms() - start;
    printf("parse,%s,%d,%.6f,%.6f\n", label, iters, elapsed, elapsed / iters * 1000.0);
}

static void bench_print(const char *label, const char *json, int iters, int formatted) {
    cJSON *tree = cJSON_Parse(json);
    double start = now_ms();
    for (int i = 0; i < iters; i++) {
        char *out = formatted ? cJSON_Print(tree) : cJSON_PrintUnformatted(tree);
        free(out);
    }
    double elapsed = now_ms() - start;
    cJSON_Delete(tree);
    printf("print_%s,%s,%d,%.6f,%.6f\n", formatted ? "formatted" : "unformatted",
           label, iters, elapsed, elapsed / iters * 1000.0);
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s <fixtures_dir>\n", argv[0]);
        return 1;
    }
    const char *dir = argv[1];
    char path[1024];

    printf("op,label,iters,total_ms,us_per_iter\n");

    const char *fixtures[] = {"test1", "test5", "test10"};
    for (int i = 0; i < 3; i++) {
        snprintf(path, sizeof(path), "%s/%s", dir, fixtures[i]);
        char *json = read_file(path);
        if (!json) { fprintf(stderr, "missing fixture %s\n", path); continue; }
        int iters = 20000;
        bench_parse(fixtures[i], json, iters);
        bench_print(fixtures[i], json, iters, 1);
        bench_print(fixtures[i], json, iters, 0);
        free(json);
    }

    int sizes[] = {100, 1000, 10000};
    for (int i = 0; i < 3; i++) {
        char *json = synthetic_large(sizes[i]);
        char label[32];
        snprintf(label, sizeof(label), "synthetic_%d", sizes[i]);
        int iters = sizes[i] <= 100 ? 2000 : (sizes[i] <= 1000 ? 200 : 20);
        bench_parse(label, json, iters);
        bench_print(label, json, iters, 0);
        free(json);
    }

    return 0;
}
