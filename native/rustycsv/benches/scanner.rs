//! Microbenchmark for the SIMD structural scanner.
//!
//! Isolates `scan_structural` from the NIF and term-building layers so that
//! changes to the chunk loop can be measured directly. Rule 8 requires a
//! recorded baseline before any performance-affecting change; save one with
//! `cargo bench --bench scanner -- --save-baseline <name>` and compare later
//! runs with `--baseline <name>`.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rustycsv::core::scan_structural;

const SEPARATORS: &[u8] = b",";
const ESCAPE: u8 = b'"';

/// Rows with no quoting at all: the cheapest path through the scanner.
fn plain_csv(rows: usize) -> String {
    let mut out = String::with_capacity(rows * 48);
    for i in 0..rows {
        out.push_str(&format!("{i},alice{i},engineering,{i}0000,2024-01-15\n"));
    }
    out
}

/// Every field quoted, with embedded separators and doubled quotes. Exercises
/// the prefix-XOR quote-region path on every chunk.
fn quoted_csv(rows: usize) -> String {
    let mut out = String::with_capacity(rows * 80);
    for i in 0..rows {
        out.push_str(&format!(
            "\"{i}\",\"last, first {i}\",\"say \"\"hi\"\" {i}\",\"a,b,c\",\"2024-01-15\"\n"
        ));
    }
    out
}

/// Realistic mix: most fields bare, roughly one in five quoted.
fn mixed_csv(rows: usize) -> String {
    let mut out = String::with_capacity(rows * 60);
    for i in 0..rows {
        if i % 5 == 0 {
            out.push_str(&format!(
                "{i},\"Doe, Jane\",engineering,{i}0000,2024-01-15\n"
            ));
        } else {
            out.push_str(&format!("{i},alice{i},engineering,{i}0000,2024-01-15\n"));
        }
    }
    out
}

fn bench_scan(c: &mut Criterion) {
    let inputs = [
        ("plain", plain_csv(20_000)),
        ("quoted", quoted_csv(20_000)),
        ("mixed", mixed_csv(20_000)),
    ];

    let mut group = c.benchmark_group("scan_structural");
    for (name, input) in &inputs {
        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), input, |b, input| {
            b.iter(|| scan_structural(std::hint::black_box(input.as_bytes()), SEPARATORS, ESCAPE));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_scan);
criterion_main!(benches);
