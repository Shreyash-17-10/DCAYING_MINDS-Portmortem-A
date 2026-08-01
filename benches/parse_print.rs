//! Benchmarks for cjson-rs's parse/print hot paths, using the same
//! `tests/fixtures/inputs/testN` documents used in the ported test suite
//! (real-world-shaped JSON: nested objects, arrays, mixed types), plus a
//! synthetic large document for scaling behavior.
//!
//! Run with: cargo bench
//! A companion C benchmark (benches/c_bench/bench.c) exercises the original
//! cJSON.c the same way, over the same fixtures, so the two can be compared
//! head-to-head — see BENCHMARK_REPORT.md for the numbers and methodology.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::fs;
use std::path::Path;

use cjson_rs::parse::parse;
use cjson_rs::print::{print, print_unformatted};

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/inputs")
        .join(name);
    fs::read_to_string(path).unwrap_or_else(|e| panic!("missing fixture {name}: {e}"))
}

/// Builds a synthetic large JSON array-of-objects document for scaling
/// benchmarks, since the ported fixtures are all small (<3KB) hand-written
/// examples from upstream.
fn synthetic_large(n: usize) -> String {
    let mut s = String::from("[");
    for i in 0..n {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            r#"{{"id":{i},"name":"item-{i}","active":{},"score":{:.3},"tags":["a","b","c"]}}"#,
            i % 2 == 0,
            i as f64 * 1.5
        ));
    }
    s.push(']');
    s
}

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse");
    for name in ["test1", "test5", "test10"] {
        let json = fixture(name);
        group.throughput(Throughput::Bytes(json.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &json, |b, json| {
            b.iter(|| parse(black_box(json)).unwrap());
        });
    }

    for n in [100usize, 1_000, 10_000] {
        let json = synthetic_large(n);
        group.throughput(Throughput::Bytes(json.len() as u64));
        group.bench_with_input(BenchmarkId::new("synthetic_array", n), &json, |b, json| {
            b.iter(|| parse(black_box(json)).unwrap());
        });
    }
    group.finish();
}

fn bench_print(c: &mut Criterion) {
    let mut group = c.benchmark_group("print");
    for name in ["test1", "test5", "test10"] {
        let json = fixture(name);
        let value = parse(&json).unwrap();
        group.bench_with_input(BenchmarkId::new("formatted", name), &value, |b, value| {
            b.iter(|| print(black_box(value)).unwrap())
        });
        group.bench_with_input(BenchmarkId::new("unformatted", name), &value, |b, value| {
            b.iter(|| print_unformatted(black_box(value)).unwrap())
        });
    }

    for n in [100usize, 1_000, 10_000] {
        let json = synthetic_large(n);
        let value = parse(&json).unwrap();
        group.throughput(Throughput::Bytes(json.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("synthetic_array_unformatted", n),
            &value,
            |b, value| b.iter(|| print_unformatted(black_box(value)).unwrap()),
        );
    }
    group.finish();
}

fn bench_round_trip(c: &mut Criterion) {
    let mut group = c.benchmark_group("round_trip");
    for n in [100usize, 1_000, 10_000] {
        let json = synthetic_large(n);
        group.throughput(Throughput::Bytes(json.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &json, |b, json| {
            b.iter(|| {
                let v = parse(black_box(json)).unwrap();
                print_unformatted(black_box(&v)).unwrap()
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_parse, bench_print, bench_round_trip);
criterion_main!(benches);
