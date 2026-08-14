# RustyCSV Benchmarks

This document presents benchmark results comparing RustyCSV's parsing and encoding performance against pure Elixir (NimbleCSV 1.3.0).

## Test Environment

- **Elixir**: 1.19.4
- **OTP**: 28
- **Hardware**: Apple Silicon M1 Pro (10 cores, 16 GB RAM)
- **RustyCSV**: 0.3.6
- **Pure Elixir baseline**: NimbleCSV 1.3.0
- **Test date**: February 2, 2026

> **Note:** All results below were collected on this specific hardware. Absolute throughput numbers will vary on different machines, but relative speedups should be broadly representative.

## Strategies Compared

| Strategy | Description | Best For |
|----------|-------------|----------|
| `:simd` | SIMD scan + boundary-based sub-binary fields (default) | General use |
| `:parallel` | SIMD scan + rayon parallel boundary extraction + sub-binaries | Large files |
| `:streaming` | Bounded-memory chunks | Unbounded files |

Both batch strategies share a single-pass SIMD structural scanner that finds every unquoted separator and row ending, then create BEAM sub-binary references into the original input. `:simd` extracts boundaries single-threaded; `:parallel` uses rayon across multiple threads, then builds sub-binary terms on the main thread.

## Throughput Benchmark Results

### Simple CSV (334 KB, 10K rows, no quotes)

| Strategy | Throughput | vs pure Elixir |
|----------|------------|--------------|
| RustyCSV (simd) | 772 ips | **3.5x faster** |
| RustyCSV (parallel) | 487 ips | **2.1x faster** |
| Pure Elixir | 233 ips | baseline |

### Quoted CSV (947 KB, 10K rows, all fields quoted with escapes)

| Strategy | Throughput | vs pure Elixir |
|----------|------------|--------------|
| RustyCSV (simd) | 449 ips | **18.6x faster** |
| RustyCSV (parallel) | 326 ips | **13.3x faster** |
| Pure Elixir | 25 ips | baseline |

### Mixed/Realistic CSV (652 KB, 10K rows)

| Strategy | Throughput | vs pure Elixir |
|----------|------------|--------------|
| RustyCSV (simd) | 497 ips | **5.2x faster** |
| RustyCSV (parallel) | 351 ips | **3.5x faster** |
| Pure Elixir | 101 ips | baseline |

### Large CSV (6.82 MB, 100K rows)

| Strategy | Throughput | vs pure Elixir |
|----------|------------|--------------|
| RustyCSV (simd) | 48.9 ips | **11.4x faster** |
| RustyCSV (parallel) | 39.3 ips | **9.1x faster** |
| Pure Elixir | 4.3 ips | baseline |

### Very Large CSV (108 MB, 1.5M rows)

| Strategy | Throughput | vs pure Elixir |
|----------|------------|--------------|
| RustyCSV (simd) | 2.5 ips | **12.7x faster** |
| RustyCSV (parallel) | 2.07 ips | **8.6x faster** |
| Pure Elixir | 0.24 ips | baseline |

## Memory Comparison

RustyCSV allocates on the Rust side (boundary vectors during parsing) while pure Elixir allocates entirely on the BEAM. With the `memory_tracking` feature enabled, we measure Rust NIF peak allocation alongside Benchee's BEAM-side measurement.

### Methodology

We measure two metrics:
1. **NIF Peak**: Peak allocation on the Rust side during parsing (requires `memory_tracking` feature)
2. **BEAM Allocation**: Memory allocated on the BEAM during parsing (what Benchee measures)

RustyCSV's BEAM-side allocation is ~1.6 KB across all strategies — just list/tuple scaffolding. The parsed field data lives as sub-binary references into the original input binary (no per-field copy).

### Decode Memory by File Type

| Scenario | Strategy | NIF Peak (RustyCSV) | BEAM (Pure Elixir) | Ratio |
|----------|----------|---------------------|------------------|-------|
| Simple CSV (334 KB) | simd | 1.41 MB | 6.04 MB | **0.23x** |
| Simple CSV (334 KB) | parallel | 1.48 MB | 6.04 MB | 0.25x |
| Quoted CSV (947 KB) | simd | 1.64 MB | 23.89 MB | **0.07x** |
| Quoted CSV (947 KB) | parallel | 1.72 MB | 23.89 MB | 0.07x |
| Mixed CSV (652 KB) | simd | 1.63 MB | 9.64 MB | **0.17x** |
| Large File (6.82 MB) | simd | 15.88 MB | 97.02 MB | **0.16x** |
| Large File (6.82 MB) | parallel | 16.38 MB | 97.02 MB | 0.17x |
| Very Large (108 MB) | simd | 240.8 MB | 1407.75 MB | **0.17x** |

**Key insight:** RustyCSV uses **5-14x less memory** than pure Elixir. All batch strategies (including parallel) use boundary-based sub-binaries — just `Vec<Vec<(usize, usize)>>` boundary indices (16 bytes per field). Parallel has slightly higher NIF peak due to rayon thread-pool overhead.

## BEAM Reductions (Scheduler Work)

| Strategy | Reductions | vs pure Elixir |
|----------|------------|--------------|
| RustyCSV (simd) | 10,500 | 24x fewer |
| RustyCSV (parallel) | 15,100 | 17x fewer |
| Pure Elixir | 254,500 | baseline |

**What this means:**
- Low reductions = less scheduler overhead
- NIFs run outside BEAM's reduction counting
- Trade-off: NIFs can't be preempted mid-execution

## Streaming Comparison

**File:** 6.8 MB (100K rows)

### `File.stream!` Input (`parse_stream/2`)

| Parser | Mode | Time | Speedup |
|--------|------|------|---------|
| RustyCSV | line-based | 54ms | **2.2x faster** |
| Pure Elixir | line-based | 117ms | baseline |
| RustyCSV | 64KB binary chunks | 244ms | direct chunk mode |

**Result:** RustyCSV is **2.2x faster** for line-based streaming.

RustyCSV automatically detects `File.Stream` in line mode and switches to 64KB binary chunk reads, reducing stream iterations from ~100K (one per line) to ~100. The Rust NIF handles arbitrary chunk boundaries internally, so it can operate on raw binary chunks rather than pre-split lines.

### Arbitrary Binary Chunks

RustyCSV can also process arbitrary binary chunks directly (useful for network streams, compressed data, etc.). NimbleCSV's `parse_stream` expects line-delimited input, but it supports arbitrary chunks through `to_line_stream/1`. RustyCSV accepts those chunks directly without that normalization step.

## Real-World Benchmark: Amazon Settlement Reports

This section presents results from parsing Amazon SP-API settlement reports in TSV format.

### Test Data

- **Data source**: Amazon Seller Central settlement reports (TSV format)
- **Report sizes**: 1KB to 2.6MB (20 to 15,820 rows)

### Small Files (<200 rows)

| Rows | RustyCSV | Pure Elixir | String.split |
|-----:|---------:|----------:|-------------:|
| 20 | 2ms | 2ms | 2ms |
| 24 | 2ms | 2ms | 2ms |
| 36 | 2ms | 2ms | 2ms |
| 93 | 2ms | 2ms | 2ms |
| 100 | 2ms | 2ms | 2ms |
| 141 | 2ms | 3ms | 3ms |

**Conclusion**: For small files, all approaches perform equivalently (~2ms).

### Large Files (10K+ rows)

| Rows | RustyCSV | Pure Elixir | vs pure Elixir |
|-----:|---------:|----------:|-------------:|
| 9,985 | **46ms** | 64ms | 28% faster |
| 10,961 | **54ms** | 68ms | 21% faster |
| 11,246 | **60ms** | 69ms | 13% faster |
| 11,754 | **56ms** | 78ms | 28% faster |
| 13,073 | **84ms** | 96ms | 13% faster |

**Conclusion**: RustyCSV is consistently 13-28% faster than pure Elixir for large real-world files.

## Summary

### Speed Rankings by File Type

| File Type | Best Strategy | Speedup vs pure Elixir |
|-----------|---------------|----------------------|
| Simple CSV | `:simd` | 3.5x |
| Quoted CSV | `:simd` | 18.8x |
| Mixed CSV | `:simd` | 5.7x |
| Large CSV (7MB) | `:simd` | 11.5x |
| Very Large CSV (108MB) | `:simd` | 13.0x |
| Streaming (6.8MB) | `parse_stream/2` | 2.2x |
| Real-world TSV | `:simd` | 1.1-1.3x |

### Strategy Selection Guide

| Use Case | Recommended Strategy |
|----------|---------------------|
| Default / General use | `:simd` |
| Large files with many cores | `:parallel` |
| Streaming / Unbounded | `parse_stream/2` |

### Key Findings

1. **Quoted fields show largest gains** — 18.6x faster due to SIMD prefix-XOR quote detection handling all escapes in a single pass

2. **5-14x less memory than pure Elixir** — Boundary-based parsing uses only 16 bytes per field (offset pairs) on the Rust side, then near-free sub-binary references on the BEAM side. Pure Elixir allocates full copies of every field.

3. **BEAM reductions are minimal** — 17-24x fewer reductions than pure Elixir, reducing scheduler load (but NIFs can't be preempted)

4. **Streaming is 2.2x faster** — RustyCSV auto-optimizes `File.Stream` to binary chunk mode. Also supports arbitrary binary chunks for non-file streams.

5. **Real-world vs synthetic** — Synthetic benchmarks show 3.5-19x gains; real-world TSV shows 13-28% gains due to simpler data patterns.

## Encoding Benchmark Results

The default `dump_to_iodata` path returns a single flat binary. Parallel encoding and BOM-enabled modules may return list-shaped iodata. See the [README](../README.md#nif-accelerated-encoding) for usage details and how this differs from pure Elixir.

### Throughput

| Scenario | Output Size | RustyCSV ips | Pure Elixir ips | Speedup |
|----------|-------------|-------------|---------------|---------|
| Plain UTF-8 — DB export (10K rows × 8 cols) | 709 KB | 638.9 | 253.3 | **2.5x** |
| Plain UTF-8 — DB export (100K rows × 8 cols) | 7.1 MB | 65.9 | 18.3 | **3.6x** |
| Plain UTF-8 — User content (10K rows, heavy quoting) | 955 KB | 717.4 | 140.7 | **5.1x** |
| Plain UTF-8 — Wide table (10K rows × 50 cols) | 2.9 MB | 141.7 | 32.4 | **4.4x** |
| Formula UTF-8 — DB export (10K rows) | 709 KB | 582.7 | 181.1 | **3.2x** |
| Formula UTF-8 — Formula-heavy (10K rows, ~40% trigger) | 484 KB | 964.7 | 285.3 | **3.4x** |
| UTF-16 LE — DB export (10K rows) | 1.4 MB | 379.9 | 12.1 | **31.5x** |
| Formula + UTF-16 LE — Formula-heavy (10K rows) | 964 KB | 565.3 | 19.5 | **28.9x** |

### Memory

| Scenario | NIF Peak (RustyCSV) | BEAM (Pure Elixir) | Ratio |
|----------|---------------------|------------------|-------|
| Plain UTF-8 — DB export (10K rows) | 1.5 MB | 5.1 MB | **0.3x** |
| Plain UTF-8 — DB export (100K rows) | 12.0 MB | 51.4 MB | **0.2x** |
| Plain UTF-8 — User content (heavy quoting) | 1.5 MB | 5.9 MB | **0.3x** |
| Plain UTF-8 — Wide table (50 cols) | 6.0 MB | 30.5 MB | **0.2x** |
| Formula UTF-8 — DB export (10K rows) | 1.5 MB | 8.2 MB | **0.2x** |
| Formula UTF-8 — Formula-heavy | 769 KB | 5.4 MB | **0.1x** |
| UTF-16 LE — DB export (10K rows) | 3.0 MB | 52.8 MB | **0.1x** |
| Formula + UTF-16 LE — Formula-heavy | 1.5 MB | 37.4 MB | **0.04x** |

RustyCSV's BEAM-side allocation is 80 bytes across all scenarios. NIF peak memory is proportional to the output size.

### Encoding Summary

| Encoding Path | Speedup vs pure Elixir | Memory Ratio |
|---------------|----------------------|--------------|
| Plain UTF-8 | 2.5–5.1x faster | 0.2–0.3x |
| Formula UTF-8 | 3.2–3.4x faster | 0.1–0.2x |
| UTF-16 LE | 31.5x faster | 0.1x |
| Formula + UTF-16 LE | 28.9x faster | 0.04x |

Non-UTF-8 encoding shows the largest gains due to single-pass encoding of the entire output buffer.

## Running the Benchmarks

```bash
# Decode benchmark (all strategies)
mix run bench/decode_bench.exs

# Encode benchmark (all encoding paths)
mix run bench/encode_bench.exs
```

For memory tracking details, enable the `memory_tracking` feature:

```toml
# In native/rustycsv/Cargo.toml
[features]
default = ["mimalloc", "memory_tracking"]
```

Then rebuild: `FORCE_RUSTYCSV_BUILD=true mix compile --force`
