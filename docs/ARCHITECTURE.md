# RustyCSV Architecture

A purpose-built Rust NIF for ultra-fast CSV parsing in Elixir. Not a wrapper around an existing library—custom-built from the ground up for optimal BEAM integration.

## Key Innovations

### Purpose-Built, Not Wrapped

Unlike projects that wrap existing Rust crates (like the excellent `csv` crate), RustyCSV is **designed specifically for Elixir**:

- **Direct BEAM term construction** - Results go straight to Erlang terms, no intermediate serialization
- **ResourceArc integration** - Streaming parser state managed by BEAM's garbage collector
- **Dirty scheduler awareness** - Long operations run on dirty CPU schedulers
- **Sub-binary field references** - All batch strategies return BEAM sub-binaries pointing into the original input, only allocating for quote unescaping

### Parsing Strategies

| Strategy | Innovation |
|----------|------------|
| `:simd` | SIMD structural scan + boundary-based sub-binary fields (default) |
| `:parallel` | SIMD scan + rayon parallel boundary extraction + sub-binaries |
| `:streaming` | Stateful parser with bounded memory, handles multi-GB files |

### Memory Efficiency

- **Sub-binary memory model** - All batch strategies use sub-binary references (5-14x less memory than pure Elixir)
- **Streaming bounded memory** - Process 10GB+ files with ~64KB memory footprint
- **mimalloc allocator** - High-performance allocator for reduced fragmentation
- **Optional memory tracking** - Opt-in profiling with zero overhead when disabled

### Validated Correctness

- **484 ExUnit tests plus 131 Rust tests** covering RFC 4180, industry test suites, edge cases, encodings, multi-byte separators/escapes, and headers-to-maps
- **Cross-strategy validation** - All strategies produce identical output
- **NimbleCSV compatibility** - Verified identical behavior for all API functions

## Quick Start

```elixir
# Use the pre-defined RFC4180 parser (drop-in replacement for NimbleCSV.RFC4180)
alias RustyCSV.RFC4180, as: CSV

# Parse CSV (skips headers by default, like NimbleCSV)
CSV.parse_string("name,age\njohn,27\n")
#=> [["john", "27"]]

# Include headers
CSV.parse_string("name,age\njohn,27\n", skip_headers: false)
#=> [["name", "age"], ["john", "27"]]

# Choose strategy for large files with many cores
CSV.parse_string(huge_csv, strategy: :parallel)

# Stream large files (uses bounded-memory streaming parser)
"huge.csv"
|> File.stream!()
|> CSV.parse_stream()
|> Stream.each(&process/1)
|> Stream.run()

# Dump back to CSV
CSV.dump_to_iodata([["a", "b"], ["1", "2"]])
#=> "a,b\n1,2\n"
```

## NimbleCSV API Compatibility

RustyCSV is designed as a drop-in replacement for NimbleCSV. It implements the complete API:

| Function | Description | Status |
|----------|-------------|--------|
| `parse_string/2` | Parse CSV string to list of rows | ✅ |
| `parse_stream/2` | Lazily parse a stream | ✅ |
| `parse_enumerable/2` | Parse any enumerable | ✅ |
| `dump_to_iodata/1` | Convert rows to list-shaped iodata | ✅ |
| `dump_to_stream/1` | Lazily convert rows to iodata stream | ✅ |
| `to_line_stream/1` | Convert arbitrary chunks to lines | ✅ |
| `options/0` | Return the original definition options | ✅ |

### `define/2` Options

| Option | NimbleCSV | RustyCSV | Status |
|--------|-----------|----------|--------|
| `:separator` | ✅ Any | ✅ Any (single or multi-byte) | ✅ |
| `:escape` | ✅ Any | ✅ Any (single or multi-byte) | ✅ |
| `:line_separator` | ✅ | ✅ | ✅ |
| `:newlines` | ✅ | ✅ | ✅ |
| `:trim_bom` | ✅ | ✅ | ✅ |
| `:dump_bom` | ✅ | ✅ | ✅ |
| `:reserved` | ✅ | ✅ | ✅ |
| `:escape_formula` | ✅ | ✅ | ✅ |
| `:moduledoc` | ✅ | ✅ | ✅ |
| `:encoding` | ✅ | ✅ | Full support |

### Migration

```elixir
# Before
alias NimbleCSV.RFC4180, as: CSV

# After
alias RustyCSV.RFC4180, as: CSV

# All function calls work identically.
```

### RustyCSV Extensions

Beyond the NimbleCSV API:

```elixir
# Choose parsing strategy
CSV.parse_string(data, strategy: :parallel)

# Return maps instead of lists
CSV.parse_string(data, headers: true)
#=> [%{"name" => "john", "age" => "27"}]

CSV.parse_string(data, headers: [:name, :age])
#=> [%{name: "john", age: "27"}]
```

## Parsing Strategies

Both batch strategies (`:simd`, `:parallel`) share a single-pass SIMD structural scanner (`scan_structural`) that finds every unquoted separator and row ending in one sweep, producing a `StructuralIndex`. Both use boundary-based sub-binary output — they parse field boundaries, then create BEAM sub-binary references into the original input. `:simd` extracts boundaries on one thread; `:parallel` uses rayon to extract boundaries across multiple threads, then builds sub-binary terms on the main thread.

| Strategy | Description | Best For |
|----------|-------------|----------|
| `:simd` | SIMD scan + boundary-based sub-binary fields (default) | Most files - fastest general purpose |
| `:parallel` | SIMD scan + rayon parallel boundary extraction + sub-binaries | Large files with many cores |
| `:streaming` | Stateful chunked parser | Unbounded files, bounded memory |

### Strategy Selection Guide

```
File Size        Recommended Strategy
─────────────────────────────────────────────────────────────
< 500 MB         :simd (default)
500 MB+          :simd or :parallel (multi-core)
Unbounded        streaming (parse_stream)
```

### Memory Model Trade-offs

| Strategy | Memory Model | Input Binary | Best When |
|----------|--------------|--------------|-----------|
| `:simd` | Sub-binary | Kept alive until fields GC'd | General use |
| `:parallel` | Sub-binary | Kept alive until fields GC'd | Multi-core, large files |
| `:streaming` | Copy (chunked) | Freed per chunk | Unbounded files |

## Project Structure

```
native/rustycsv/src/
├── lib.rs                 # NIF entry points, separator/escape decoding, dispatch
├── core/
│   ├── mod.rs            # Re-exports
│   ├── simd_scanner.rs   # Single-pass SIMD structural scanner (prefix-XOR quote detection)
│   ├── simd_index.rs     # StructuralIndex, RowIter, RowFieldIter, FieldIter
│   ├── scanner.rs        # Byte-level helpers (separator matching)
│   ├── field.rs          # Field extraction, quote handling
│   └── newlines.rs       # Custom newline support
├── strategy/
│   ├── mod.rs            # Strategy exports
│   ├── direct.rs         # A/B: Basic and SIMD strategies (consume StructuralIndex)
│   ├── two_phase.rs      # C: Index-then-extract (StructuralIndex → CsvIndex bridge)
│   ├── streaming.rs      # D: Stateful chunked parser (single-byte fast path)
│   ├── parallel.rs       # E: Rayon-based parallel (StructuralIndex for row ranges)
│   ├── zero_copy.rs      # F: Sub-binary boundary parsing (consume StructuralIndex)
│   ├── general.rs        # Multi-byte separator/escape support (all strategies)
│   ├── encode.rs         # SIMD field scanning, quoting helpers for encoding
│   └── encoding.rs       # UTF-8 → target encoding converters (UTF-16, Latin-1, etc.)
├── term.rs               # Term building (lists + maps, copy + sub-binary, multi-byte escape)
└── resource.rs           # ResourceArc for streaming parser (single-byte + general)

lib/
├── rusty_csv.ex          # Main module with define/2 macro, types, specs
├── rusty_csv/
│   ├── native.ex         # NIF stubs with full documentation
│   └── streaming.ex      # Elixir streaming interface
```

## Implementation Details

### SIMD Structural Scanner

All batch strategies share a single-pass SIMD structural scanner (`scan_structural` in `simd_scanner.rs`) inspired by simdjson's approach. It processes the entire input once and produces a `StructuralIndex` containing the positions of all unquoted separators and row endings.

**How it works:**

1. Load 16-byte chunks (or 32-byte on AVX2) into SIMD registers
2. Compare against separator, quote, `\n`, and `\r` characters simultaneously
3. Use **prefix-XOR** on the quote bitmask to determine which positions are inside quoted regions — a cumulative XOR where bit *i* is set if there's an odd number of quotes before position *i*
4. Mask out quoted positions, then extract the remaining separator and newline positions into `Vec<u32>` arrays
5. A `quote_carry` bit tracks quote parity across chunk boundaries

The prefix-XOR uses a portable shift-and-xor cascade on all targets (6 XOR+shift ops on a u64). Architecture-specific intrinsics (CLMUL, PMULL) were evaluated but removed — benchmarks showed no measurable difference for the 16/32-bit masks used in CSV scanning, and removing them keeps the entire scanner free of `unsafe` code.

**`std::simd` API surface:** The scanner uses only the stabilization-safe subset of `portable_simd`: `Simd::from_slice`, `splat`, `simd_eq`, `to_bitmask`, and bitwise ops. It avoids the APIs [blocking stabilization](https://github.com/rust-lang/portable-simd/issues/364) (swizzle, scatter/gather, lane-count generics). No `std::arch` intrinsics are used.

**Output — `StructuralIndex`:**

```rust
pub struct StructuralIndex {
    pub field_seps: Vec<u32>,   // positions of unquoted separators
    pub row_ends: Vec<RowEnd>,  // positions of unquoted row terminators
    pub input_len: u32,
}
```

Positions use `u32` (4 GB cap) to halve memory vs `usize` on 64-bit. Strategies consume the index via `rows_with_fields()`, a cursor-based iterator that yields `(row_start, row_content_end, FieldIter)` tuples by advancing a linear cursor through `field_seps` — O(total_seps) across all rows instead of O(rows × log(seps)) with binary search.

### Quote Handling with Sub-Binaries

All batch strategies properly handle CSV quote escaping (doubled quotes `""` → `"`). The boundary-based approach uses a hybrid strategy:

- **Unquoted fields** → sub-binary reference into original input (zero-copy)
- **Quoted without escapes** → sub-binary of inner content, stripping outer quotes (zero-copy)
- **Quoted with escapes** → copy and unescape `""` → `"` (must allocate)

```rust
fn field_to_term_hybrid(env, input: &Binary, start, end, escape) -> Term {
    let field = &input.as_slice()[start..end];
    if needs_unescaping(field) {
        // Must copy and unescape: "val""ue" -> val"ue
        return copy_and_unescape(field);
    }
    // Zero-copy: create sub-binary reference
    input.make_subbinary(start, len).unwrap().into()
}
```

### Strategy A/B: Direct Parsing (Boundary-Based)

All four non-parallel batch strategies (`:simd`, `:basic`, `:indexed`, `:zero_copy`) use the same path: call `scan_structural` to build a `StructuralIndex`, parse field boundaries into `Vec<Vec<(usize, usize)>>`, then create BEAM sub-binary terms via `boundaries_to_term_hybrid`. They are functionally identical; the named variants are retained for API compatibility.

### Strategy C: Two-Phase Index-then-Extract

Now uses the same boundary-based sub-binary path as `:simd`. The two-phase strategy functions (`parse_csv_indexed*`) remain in the codebase but the NIF dispatches through the unified boundary path. The `:indexed` name is retained for API compatibility.

### Strategy D: Streaming Parser

Stateful parser wrapped in `ResourceArc` for NIF resource management. The resource
holds a `StreamingParserEnum` that dispatches between the single-byte fast path
and the general multi-byte path:

```rust
pub enum StreamingParserEnum {
    SingleByte(StreamingParser),
    General(GeneralStreamingParser),
}
```

Both variants share the same interface (feed, take_rows, finalize, etc.) and are
selected at creation time based on separator/escape lengths.

Key features:
- Owns data (`Vec<u8>`) because input chunks are temporary
- Tracks `scan_pos` to resume parsing where it left off
- Preserves quote state across chunks
- Enforces a configurable maximum buffer size (default 256 MB) to prevent unbounded
  memory growth; raises `:buffer_overflow` if exceeded
- Mutex-protected access with poisoning recovery (raises `:mutex_poisoned` instead
  of panicking the VM)

### Strategy E: Parallel Parser

Uses rayon for multi-threaded boundary extraction on a **dedicated thread pool** (`rustycsv-*`
threads, capped at 8) to avoid contention with other Rayon users in the same VM:

1. **Single-threaded**: `scan_structural` → `StructuralIndex` (row boundaries + field separator positions)
2. **O(n) cursor walk**: Collect `(row_start, content_end, sep_lo, sep_hi)` into a flat `Vec`, mapping each row to its slice of `field_seps`
3. **Parallel**: Each worker computes `(start, end)` boundary pairs for its rows by indexing into the shared `&[u32]` field_seps slice — no data copying, no per-row allocation
4. **Single-threaded**: Main thread builds BEAM sub-binary terms from the collected boundaries via `boundaries_to_term_hybrid`

Uses `DirtyCpu` scheduler to avoid blocking normal BEAM schedulers.

**Evolution of field-position reuse from the structural index:**

The SIMD structural scanner already finds every separator position. Three approaches were benchmarked for reusing those positions instead of re-scanning with memchr:

- **Approach A** — Pre-collect `Vec<Vec<(u32, u32)>>` field bounds: 10K+ inner Vec allocations cost more than the memchr scan (-18% on simple CSV).
- **Approach B** — Binary search via `fields_in_row()`: Two `partition_point` calls per row add O(log n) overhead (-11% on simple CSV).
- **Approach C** (current) — Flat index + direct slice: O(n) cursor walk builds a single flat Vec mapping each row to its slice of `field_seps`. Each worker indexes into the shared `&[u32]` with O(1) lookup. This avoids A's allocation overhead and B's binary search overhead.

**Note**: Workers compute boundary pairs only (pure arithmetic) — BEAM sub-binary terms are built on the main thread since `Env` is not thread-safe. The `make_subbinary` call is O(1) per field, so the serial term construction is not a bottleneck.

### Strategy F: Sub-Binary Field Construction (All Batch Strategies)

All batch strategies now use the same sub-binary field construction path. The `boundaries_to_term_hybrid` function in `term.rs` creates BEAM terms from parsed field boundaries:

```rust
fn field_to_term_hybrid(env, input: &Binary, start, end, escape) -> Term {
    let field = &input.as_slice()[start..end];

    // Check if quoted with escapes
    if needs_unescaping(field) {
        // Must copy and unescape: "val""ue" -> val"ue
        return copy_and_unescape(field);
    }

    // Zero-copy: create sub-binary reference (safe API)
    input.make_subbinary(start, len).unwrap().into()
}
```

**Hybrid approach**:
- Unquoted fields → sub-binary (zero-copy)
- Quoted without escapes → sub-binary of inner content (zero-copy)
- Quoted with escapes → copy and unescape (must allocate)

**Trade-off**: Sub-binaries keep the parent binary alive until all field references are garbage collected. This is the right default since the memory savings (5-14x vs pure Elixir) far outweigh the delayed input GC.

### Headers-to-Maps

Two dedicated NIFs (`parse_to_maps`, `parse_to_maps_parallel`) return rows as Elixir maps instead of lists. They reuse all existing parsing strategy code — only the term conversion layer differs.

**Architecture:**
```
headers: false (default)  →  existing NIFs  →  boundaries_to_term_hybrid (list of lists)
headers: true/[...]       →  new NIFs       →  boundaries_to_maps_hybrid (list of maps)
```

**Key interning**: Header terms are allocated once in Rust and reused for every row, avoiding repeated binary allocation for map keys.

**Header modes**:
- `headers: true` — first CSV row parsed as string keys, remaining rows become maps
- `headers: [atoms/strings]` — explicit key terms passed from Elixir, optionally skipping the first row

**Streaming**: Uses Elixir-side `Stream.transform` for map conversion rather than a new NIF, since the streaming parser already yields one row at a time and the map wrapping overhead is negligible.

**Edge case handling** (all in Rust):
- Fewer columns than keys → `nil` fill
- More columns than keys → extra columns ignored
- Duplicate keys → last value wins (incremental map building fallback)

### NIF-Accelerated Encoding

`dump_to_iodata` dispatches to the `encode_string` Rust NIF by default, which
writes each row into a reusable `Vec<u8>` and returns one `NewBinary` per row.
The Elixir wrapper places each row binary in its own iodata list, preserving
NimbleCSV's top-level row shape.

**Why one binary per row instead of per-field iodata:**

The encoder avoids a BEAM term for every field, separator, and newline. It
builds each row in a reusable Rust buffer, then copies that row into one BEAM
binary. This keeps the public row-level shape while bounding term construction
to one binary and one list cell per row.

**Architecture:**

```
Input: Erlang list of lists (rows of binary fields)
  │
  ├─ For each field:
  │    ├─ SIMD scan: needs quoting? (16-32 bytes/cycle)
  │    ├─ Check formula trigger (if escape_formula configured)
  │    ├─ Write to Vec<u8>: raw bytes / quoted bytes / formula-prefixed
  │    └─ If non-UTF-8: encode field bytes to target encoding
  │
  └─ memcpy row Vec<u8> → NewBinary (one BEAM binary per row)
```

**Four PostProcess modes** (zero-overhead dispatch via enum):

| Mode | Formula | Encoding | Behavior |
|------|---------|----------|----------|
| `None` | No | UTF-8 | Write field bytes / quoted bytes directly |
| `FormulaOnly` | Yes | UTF-8 | Prefix triggered fields, quote if dirty |
| `EncodingOnly` | No | Non-UTF-8 | Encode each field + separators to target |
| `Full` | Yes | Non-UTF-8 | Formula prefix (raw) + encoded content |

See [BENCHMARK.md](BENCHMARK.md#encoding-benchmark-results) for throughput and memory numbers.

---

## Performance Optimizations

### mimalloc Allocator

RustyCSV uses [mimalloc](https://github.com/microsoft/mimalloc) as the default allocator:

```rust
#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
```

Benefits:
- 10-20% faster allocation for many small objects
- Reduced fragmentation
- No tracking overhead in default configuration

To disable mimalloc (for exotic build targets):
```toml
[dependencies]
rusty_csv = { version = "0.1", default-features = false }
```

### Optional Memory Tracking (Benchmarking Only)

For profiling Rust-side memory during development and benchmarking, enable the `memory_tracking` feature in `native/rustycsv/Cargo.toml`. This is not intended for production — it wraps every allocation/deallocation with atomic counter updates, adding overhead. It is also the only source of `unsafe` in the codebase (required by the `GlobalAlloc` trait).

```toml
[features]
default = ["mimalloc", "memory_tracking"]
```

This wraps the allocator with tracking overhead:

```rust
#[cfg(feature = "memory_tracking")]
#[global_allocator]
static GLOBAL: tracking::TrackingAllocator = tracking::TrackingAllocator;
```

When enabled, these functions return actual values:
- `RustyCSV.Native.get_rust_memory/0` - Current allocation
- `RustyCSV.Native.get_rust_memory_peak/0` - Peak allocation
- `RustyCSV.Native.reset_rust_memory_stats/0` - Reset and get stats

When disabled (default), they return `0` with zero overhead.

### Pre-allocated Vectors

The structural scanner pre-allocates vectors with capacity estimates based on input size:

```rust
let est_seps = input.len() / 10 + 16;  // ~1 separator per 10 bytes
let est_rows = input.len() / 50 + 4;   // ~1 row per 50 bytes
```

This reduces reallocation overhead during the scan pass.

---

## Background: Why a Rust NIF?

### Why a NIF Over Pure Elixir?

Pure Elixir CSV parsing is fast — binary pattern matching and sub-binary references are well-optimized on the BEAM. A Rust NIF adds:

1. **SIMD structural scanning** - Process 16-32 bytes per cycle for delimiter/quote detection
2. **Multiple strategies** - Choose the right tool for each workload
3. **Streaming support** - Process arbitrarily large files with bounded memory
4. **Reduced scheduler load** - Offload parsing to native code
5. **Memory efficiency** - Sub-binary references use 5-14x less memory than pure Elixir
6. **NIF safety** - Dirty schedulers for long operations

## NIF Safety

### The 1ms Rule

NIFs should complete in under 1ms to avoid blocking schedulers.

| Approach | Used By | Description |
|----------|---------|-------------|
| Dirty Schedulers | All batch NIFs, `streaming_feed`, `streaming_next_rows`, `streaming_finalize` | Separate from normal schedulers |
| Chunked Processing | streaming | Return control between chunks |
| Stateful Resource | streaming | Let Elixir control iteration |
| Fast SIMD | all others | Complete quickly via hardware acceleration |

All 12 NIFs that process unbounded input run on dirty CPU schedulers. Only trivial O(1)
NIFs (`streaming_new`, `streaming_status`, `streaming_set_max_buffer`, memory tracking)
remain on the normal scheduler.

### Memory Safety

All application code is **zero `unsafe`**. The only `unsafe` in the codebase is the `GlobalAlloc` trait impl behind the opt-in `memory_tracking` feature flag — this is a development-only benchmarking tool for profiling Rust-side allocations, not intended for production use. The `unsafe` is required by the `GlobalAlloc` trait definition and cannot be avoided. It is disabled by default and adds measurable overhead when enabled.

- All batch strategies create sub-binary references via rustler's safe `Binary::make_subbinary` API (bounds-checked, returns `NifResult`)
- Parallel strategy extracts boundaries on rayon workers, builds sub-binary terms on the main thread (Env is not thread-safe)
- SIMD scanner uses `std::simd` portable SIMD with no `std::arch` intrinsics
- Streaming parser uses `ResourceArc` with proper cleanup
- Streaming buffer is capped at 256 MB by default (configurable via `:max_buffer_size`)
- Mutex poisoning on streaming resources raises `:mutex_poisoned` instead of panicking
- mimalloc wrapped in tracking allocator for observability
- Dedicated rayon thread pool avoids contention with other Rayon users

## Benchmark Results

- **Synthetic benchmarks**: 3.5x-13x faster than pure Elixir for typical data, up to 19x for heavily quoted CSVs
- **Real-world TSV**: 13-28% faster than pure Elixir (10K+ rows)
- **Streaming**: 2.2x faster than pure Elixir for line-based streams; also handles arbitrary binary chunks

The speedup varies by data complexity—quoted fields with escapes show the largest gains.

See [BENCHMARK.md](BENCHMARK.md) for detailed methodology, real-world results, and raw data.

## Documentation

RustyCSV includes comprehensive documentation for hexdocs:

- **Module docs**: Detailed guides with examples
- **Type specs**: `@type`, `@typedoc` for all public types
- **Function specs**: `@spec` for all public functions
- **Examples**: Runnable examples in docstrings
- **Callbacks**: Full behaviour definition for generated modules

## Compliance & Validation

RustyCSV is validated against industry-standard CSV test suites to ensure correctness:

- **RFC 4180 Compliance** - Full compliance with the CSV specification
- **csv-spectrum** - Industry "acid test" for CSV parsers (12 test cases)
- **csv-test-data** - Comprehensive RFC 4180 test suite (17+ test cases)
- **Cross-strategy validation** - All strategies produce identical output

See [COMPLIANCE.md](COMPLIANCE.md) for full details on test suites and validation methodology.

## Future Work

- **Error positions**: Line/column numbers in ParseError

## References

- [RFC 4180](https://tools.ietf.org/html/rfc4180) - CSV specification
- [csv-spectrum](https://github.com/max-mapper/csv-spectrum) - CSV acid test suite
- [csv-test-data](https://github.com/sineemore/csv-test-data) - RFC 4180 test data
- [NimbleCSV Source](https://github.com/dashbitco/nimble_csv)
- [BEAM Binary Handling](https://www.erlang.org/doc/efficiency_guide/binaryhandling.html)
- [simdjson](https://github.com/simdjson/simdjson) - Inspiration for structural index and prefix-XOR quote detection
- [std::simd](https://doc.rust-lang.org/std/simd/index.html) - Portable SIMD (used in structural scanner)
- [rayon crate](https://docs.rs/rayon/latest/rayon/) - Parallel iteration
- [mimalloc](https://github.com/microsoft/mimalloc) - High-performance allocator
