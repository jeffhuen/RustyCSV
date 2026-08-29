# RustyCSV

**Ultra-fast CSV parsing and encoding for Elixir.** A purpose-built Rust NIF with SIMD acceleration, parallel parsing, and bounded-memory streaming. Drop-in replacement for NimbleCSV.

[![Hex.pm](https://img.shields.io/hexpm/v/rusty_csv.svg)](https://hex.pm/packages/rusty_csv)
[![Tests](https://img.shields.io/badge/tests-422%20ExUnit%20%2B%20128%20Rust-brightgreen.svg)]()
[![RFC 4180](https://img.shields.io/badge/RFC%204180-compliant-blue.svg)]()

## Why RustyCSV?

**The Problem**: CSV parsing in Elixir can be optimized further:

1. **Speed**: Pure Elixir parsing, while well-optimized, can't match native code with SIMD acceleration for large files.

2. **Flexibility**: Different workloads benefit from different strategies—parallel processing for huge files, streaming for unbounded data.

3. **Direct binary chunk streaming**: RustyCSV's `parse_stream/2` processes arbitrary binary chunks without first converting them to a line-oriented stream.

**Why not wrap an existing Rust CSV library?** The excellent [csv](https://docs.rs/csv) crate is designed for Rust workflows, not BEAM integration. Wrapping it would require serializing data between Rust and Erlang formats—adding overhead and losing the benefits of direct term construction.

**RustyCSV's approach**: The Rust NIF is built specifically for BEAM integration. It parses directly into BEAM terms and keeps optional features modular:

1. **Bounded-memory streaming** - Process multi-GB files without retaining the full input; memory is bounded by the configured buffer and longest row
2. **Sub-binary field references** - Near-zero BEAM allocation; fields reference the input binary directly
3. **Multiple strategies** - Choose SIMD, parallel, or streaming based on your workload
4. **Reduced scheduler load** - Parsing NIFs run on dirty CPU schedulers
5. **NimbleCSV-compatible API** - Supported public functions are covered by the upstream semantic test suite

## Feature Comparison

| Feature | RustyCSV | Pure Elixir (NimbleCSV) |
|---------|----------|-----------|
| **Parsing strategies** | 3 (SIMD, parallel, streaming) | 1 |
| **SIMD acceleration** | ✅ via `std::simd` portable SIMD | ❌ |
| **Parallel parsing** | ✅ via rayon | ❌ |
| **Arbitrary binary chunks** | Directly through `parse_stream/2` | Through `to_line_stream/1` before `parse_stream/2` |
| **Multi-separator support** | ✅ `[",", ";"]`, `"::"` | ✅ |
| **Encoding support** | ✅ UTF-8, UTF-16, Latin-1, UTF-32 | ✅ |
| **Memory model** | Sub-binary references | Sub-binary references |
| **NIF encoding** | ✅ List-shaped iodata | List-shaped iodata |
| **High-performance allocator** | Opt-in mimalloc | System |
| **Drop-in replacement** | ✅ Same API | - |
| **Headers-to-maps** | ✅ `headers: true` or explicit keys | ❌ |
| **RFC 4180 compliant** | ✅ 422 ExUnit + 128 Rust tests | ✅ |
| **[Benchmark (7MB CSV)](https://rusty-csv.hexdocs.pm/benchmark.html#large-csv-6-82-mb-100k-rows)** | ~20ms | ~233ms |

## Purpose-Built for Elixir

RustyCSV isn't a wrapper around an existing Rust CSV library. It is built specifically for Elixir/BEAM integration:

- **Boundary-based sub-binary fields** - SIMD scanner finds field boundaries, then creates BEAM sub-binary references directly (zero-copy for clean fields, copy only when unescaping `""` → `"`)
- **Dirty scheduler aware** - parsing NIFs run on dirty CPU schedulers rather than normal BEAM schedulers
- **ResourceArc-based streaming** - stateful parser properly integrated with BEAM's garbage collector
- **Direct term building** - parsing results go straight to BEAM terms; encoding builds one binary per row without per-field BEAM terms

### Parsing Strategies

Choose the right tool for the job:

| Strategy | Use Case | How It Works |
|----------|----------|--------------|
| `:simd` | **Default.** Fastest for most files | Single-pass SIMD structural scanner via `std::simd` |
| `:parallel` | Very large or quote-heavy files; benchmark against `:simd` | Multi-threaded row parsing via `rayon` |
| `:streaming` | Unbounded/huge files | Bounded-memory chunk processing |

Parsing is strict by default, matching NimbleCSV's malformed-quote behavior.
Existing RustyCSV applications that intentionally accept dirty exports can opt
back into the pre-0.4 behavior:

```elixir
CSV.parse_string(csv, strict: false)
CSV.parse_stream(chunks, strict: false)
```

**Memory Model:**

All batch strategies use boundary-based sub-binaries — the SIMD scanner finds field boundaries, then creates BEAM sub-binary references that point into the original input binary. Only fields requiring quote unescaping (`""` → `"`) are copied.

| Strategy | Memory Model | Input Binary | Best When |
|----------|--------------|--------------|-----------|
| `:simd` | Sub-binary | Kept alive until fields GC'd | Default — fast, low memory |
| `:parallel` | Sub-binary | Kept alive until fields GC'd | Large files, many cores |
| `:streaming` | Copy (chunked) | Freed per chunk | Unbounded files |

```elixir
# Strategy selection
CSV.parse_string(data)                           # Uses :simd (default)
CSV.parse_string(huge_data, strategy: :parallel) # Parallel field extraction via rayon
File.stream!("huge.csv") |> CSV.parse_stream()   # Bounded memory
```

## Installation

```elixir
def deps do
  [{:rusty_csv, "~> 0.4.6"}]
end
```

Precompiled NIFs are provided for supported targets through RustlerPrecompiled,
so normal installation does not require Rust. Building locally or targeting an
unsupported platform requires Rust nightly for `std::simd`; see
[SIMD and Rust Nightly](#simd-and-rust-nightly).

| Build | Selection | SIMD width on x86-64 |
|-------|-----------|----------------------|
| Portable precompiled NIF | Default | 128-bit |
| AVX2 precompiled NIF | `RUSTYCSV_CPU=avx2` | 256-bit |
| Local source build | `target-cpu=native` | 128-, 256-, or 512-bit |

### Use the precompiled AVX2 NIF

The AVX2 artifact targets x86-64-v3. It does not require Rust. Select it when
you compile the dependency:

```bash
RUSTYCSV_CPU=avx2 mix deps.compile rusty_csv --force
```

RustyCSV does not inspect the build or deployment CPU. Use this option only
when every deployment CPU supports x86-64-v3. An incompatible NIF can fail
with an illegal-instruction error. Set the variable in the environment that
builds your Elixir release; setting it only when the release starts is too late.

### Build for your CPU

A local source build can use the full SIMD width of the build CPU. Run this
command after `mix deps.get`:

```bash
RUSTFLAGS="-C target-cpu=native" FORCE_RUSTYCSV_BUILD=1 mix deps.compile rusty_csv --force
```

On Alpine Linux or another musl target, keep the dynamic-library flag that
`RUSTFLAGS` replaces:

```bash
RUSTFLAGS="-C target-cpu=native -C target-feature=-crt-static" FORCE_RUSTYCSV_BUILD=1 mix deps.compile rusty_csv --force
```

On x86-64, RustyCSV selects 256-bit SIMD when the CPU supports AVX2. It selects
512-bit SIMD when the CPU supports AVX-512F and AVX-512BW. Other CPUs keep the
portable 128-bit width.

Build the NIF on the deployment host or on a machine with the same CPU
features. `target-cpu=native` records the build CPU's instruction set in the
binary. A NIF built on a newer CPU can fail with an illegal-instruction error
on an older CPU.

Wider SIMD does not make every workload faster. Benchmark your data. To compare
AVX2 with AVX-512 on an AVX-512 host, build the AVX2 version with:

```bash
RUSTFLAGS="-C target-cpu=x86-64-v3" FORCE_RUSTYCSV_BUILD=1 mix deps.compile rusty_csv --force
```

## Quick Start

```elixir
alias RustyCSV.RFC4180, as: CSV

# Parse CSV (skips headers by default, like NimbleCSV)
CSV.parse_string("name,age\njohn,27\njane,30\n")
#=> [["john", "27"], ["jane", "30"]]

# Include headers
CSV.parse_string(csv, skip_headers: false)
#=> [["name", "age"], ["john", "27"], ["jane", "30"]]

# Stream large files with bounded memory
"huge.csv"
|> File.stream!()
|> CSV.parse_stream()
|> Stream.each(&process_row/1)
|> Stream.run()

# Parse to maps with headers
CSV.parse_string("name,age\njohn,27\njane,30\n", headers: true)
#=> [%{"name" => "john", "age" => "27"}, %{"name" => "jane", "age" => "30"}]

# With atom keys
CSV.parse_string("name,age\njohn,27\n", headers: [:name, :age])
#=> [%{name: "john", age: "27"}]

# Dump back to CSV
CSV.dump_to_iodata([["name", "age"], ["john", "27"]])
#=> [["name,age\r\n"], ["john,27\r\n"]]
```

## Drop-in NimbleCSV Replacement

```elixir
# Before
alias NimbleCSV.RFC4180, as: CSV

# After
alias RustyCSV.RFC4180, as: CSV

# That's it. Same API, 3-9x faster on typical workloads.
```

All NimbleCSV functions are supported:

| Function | Description |
|----------|-------------|
| `parse_string/2` | Parse CSV string to list of rows (or maps with `headers:`) |
| `parse_stream/2` | Lazily parse a stream (or maps with `headers:`) |
| `parse_enumerable/2` | Parse any enumerable |
| `dump_to_iodata/2` | Convert rows to list-shaped iodata (`strategy: :parallel` for quoting-heavy data) |
| `dump_to_stream/1` | Lazily convert rows to a stream of list-shaped row iodata |
| `to_line_stream/1` | Convert arbitrary chunks to lines |
| `options/0` | Return the original definition options |

## Benchmarks

The most recent published benchmark run used RustyCSV 0.3.6, Elixir 1.19,
OTP 28, and an Apple M1 Pro. It measured **3.5x faster** parsing for simple
synthetic CSV and **18.6x faster** for quote-heavy CSV.

The same run measured **13-28% faster** parsing on Amazon settlement-report TSV
files with roughly 10,000 or more rows. Results vary with data shape and hardware.

```bash
mix run bench/decode_bench.exs
```

See the [benchmark methodology and results](https://rusty-csv.hexdocs.pm/benchmark.html).

### When to Use RustyCSV

| Scenario | Recommendation |
|----------|----------------|
| **Most batch workloads** | Use `:simd` (default) |
| **Very large or quote-heavy files** | Try `:parallel` and benchmark it against `:simd` on representative data |
| **Huge or unbounded input** | Use `parse_stream/2` for bounded memory |
| **Memory-sensitive parsing** | Use `:simd` for sub-binary field references and fewer copied binaries |
| **High-throughput APIs** | Start with `:simd`; parsing runs on dirty CPU schedulers |
| **Small files (<100KB)** | Either library is reasonable |
| **Need pure Elixir** | Use NimbleCSV |

## Custom Parsers

Define parsers with custom separators and options:

```elixir
# TSV parser
RustyCSV.define(MyApp.TSV,
  separator: "\t",
  escape: "\"",
  line_separator: "\n"
)

# Pipe-separated
RustyCSV.define(MyApp.PSV,
  separator: "|",
  escape: "\"",
  line_separator: "\n"
)

MyApp.TSV.parse_string("a\tb\tc\n1\t2\t3\n")
#=> [["1", "2", "3"]]
```

### Define Options

| Option | Description | Default |
|--------|-------------|---------|
| `:separator` | Field separator(s) — string or list of strings (multi-byte OK) | `","` |
| `:escape` | Quote/escape sequence (multi-byte OK) | `"\""` |
| `:line_separator` | Line ending for dumps | `"\r\n"` |
| `:newlines` | Accepted line endings | `["\r\n", "\n"]` |
| `:encoding` | Character encoding (see below) | `:utf8` |
| `:trim_bom` | Remove BOM when parsing | `false` |
| `:dump_bom` | Add BOM when dumping | `false` |
| `:escape_formula` | Opt-in formula neutralization; matched fields are prefixed and quoted | `nil` |
| `:strategy` | Default parsing strategy | `:simd` |

With `:escape_formula` enabled, RustyCSV intentionally quotes every matched
field and encodes the complete prefixed value in one pass. Formula-disabled
dumping remains byte-compatible with NimbleCSV; see
[Compliance and Validation](https://rusty-csv.hexdocs.pm/compliance.html#formula-neutralization).

### Multi-Separator Support

For files with inconsistent delimiters (common in European locales), specify multiple separators:

```elixir
# Accept both comma and semicolon as delimiters
RustyCSV.define(MyApp.FlexibleCSV,
  separator: [",", ";"],
  escape: "\""
)

# Parse files with mixed separators
MyApp.FlexibleCSV.parse_string("a,b;c\n1;2,3\n", skip_headers: false)
#=> [["a", "b", "c"], ["1", "2", "3"]]

# Dumping uses only the FIRST separator
MyApp.FlexibleCSV.dump_to_iodata([["x", "y", "z"]]) |> IO.iodata_to_binary()
#=> "x,y,z\n"
```

Separators and escape sequences can be multi-byte:

```elixir
# Double-colon separator
RustyCSV.define(MyApp.DoubleColon,
  separator: "::",
  escape: "\""
)

# Multi-byte escape
RustyCSV.define(MyApp.DollarEscape,
  separator: ",",
  escape: "$$"
)

# Mix single-byte and multi-byte separators
RustyCSV.define(MyApp.Mixed,
  separator: [",", "::"],
  escape: "\""
)
```

### Headers-to-Maps

Return rows as maps instead of lists using the `:headers` option:

```elixir
# First row becomes string keys
CSV.parse_string("name,age\njohn,27\njane,30\n", headers: true)
#=> [%{"name" => "john", "age" => "27"}, %{"name" => "jane", "age" => "30"}]

# Explicit atom keys (first row skipped by default)
CSV.parse_string("name,age\njohn,27\n", headers: [:name, :age])
#=> [%{name: "john", age: "27"}]

# Explicit string keys
CSV.parse_string("name,age\njohn,27\n", headers: ["n", "a"])
#=> [%{"n" => "john", "a" => "27"}]

# Works with streaming too
"huge.csv"
|> File.stream!()
|> CSV.parse_stream(headers: true)
|> Stream.each(&process_map/1)
|> Stream.run()
```

Edge cases: fewer columns than headers fills with `nil`, extra columns are ignored, duplicate headers use last value, empty headers become `""`.

Key interning is done Rust-side for `parse_string` — header terms are allocated once and reused across all rows. Streaming uses Elixir-side `Stream.transform` for map conversion.

### Encoding Support

RustyCSV supports character encoding conversion:

```elixir
# UTF-16 Little Endian (Excel/Windows exports)
RustyCSV.define(MyApp.Spreadsheet,
  separator: "\t",
  encoding: {:utf16, :little},
  trim_bom: true,
  dump_bom: true
)

# Or use the pre-defined spreadsheet parser
alias RustyCSV.Spreadsheet
Spreadsheet.parse_string(utf16_data)
```

| Encoding | Description |
|----------|-------------|
| `:utf8` | UTF-8 (default, no conversion overhead) |
| `:latin1` | ISO-8859-1 / Latin-1 |
| `{:utf16, :little}` | UTF-16 Little Endian |
| `{:utf16, :big}` | UTF-16 Big Endian |
| `{:utf32, :little}` | UTF-32 Little Endian |
| `{:utf32, :big}` | UTF-32 Big Endian |

## RFC 4180 Compliance

RustyCSV is **fully RFC 4180 compliant** and validated against industry-standard test suites:

| Test Suite | Status |
|------------|--------|
| [csv-spectrum](https://github.com/max-mapper/csv-spectrum) | ✅ All pass |
| [csv-test-data](https://github.com/sineemore/csv-test-data) | ✅ All pass |
| Edge cases (PapaParse-inspired) | ✅ All pass |
| Core + NimbleCSV compat | ✅ All pass |
| Encoding (UTF-16, Latin-1, etc.) | ✅ All pass |
| Multi-separator support | ✅ All pass |
| Multi-byte separator | ✅ All pass |
| Multi-byte escape | ✅ All pass |
| Native API separator/escape | ✅ All pass |
| Headers-to-maps | ✅ All pass |
| Custom newlines | ✅ All pass |
| Streaming safety | ✅ All pass |
| Concurrent access | ✅ All pass |
| 0.4.1 parity regressions | ✅ All pass |
| **ExUnit suite total** | **422, including 5 properties** |

See [Compliance and Validation](https://rusty-csv.hexdocs.pm/compliance.html) for full details.

## How It Works

### Why Not Wrap the Rust `csv` Crate?

The Rust ecosystem has excellent CSV libraries like [csv](https://docs.rs/csv) and [polars](https://docs.rs/polars). But wrapping them for Elixir has overhead:

1. Parse CSV → Rust data structures (allocation)
2. Convert Rust structs → Erlang terms (allocation + serialization)
3. Return to BEAM

RustyCSV eliminates the middle step by parsing directly into BEAM terms:

1. Parse CSV → Erlang terms directly (single pass)
2. Return to BEAM

### Strategy Implementations

All batch strategies share a single-pass SIMD structural scanner that finds field boundaries, then create BEAM sub-binary references directly.

| Strategy | Scanning | Term Building | Memory | Best For |
|----------|----------|---------------|--------|----------|
| `:simd` | SIMD structural scanner via [`std::simd`](https://doc.rust-lang.org/std/simd/index.html) | Boundary → sub-binary | O(n) | Default, fastest for most files |
| `:parallel` | SIMD structural scanner | Boundary → sub-binary | O(n) | Large files with many cores |
| `:streaming` | Byte-by-byte | Copy (chunked) | O(chunk) | Unbounded/huge files |

**Shared across batch strategies (`:simd`, `:parallel`):**
- Single-pass SIMD structural scanner (finds all unquoted separators and row endings in one sweep)
- Boundary-based sub-binary field references (near-zero BEAM allocation)
- Hybrid unescaping: sub-binaries for clean fields, copy only when `""` → `"` unescaping needed
- Direct Erlang term construction via Rustler (no serde)
- Optional [mimalloc](https://github.com/microsoft/mimalloc) allocator, opt-in only (see [allocator safety](docs/ALLOCATOR_SAFETY.md))

**`:parallel` specifics:**
- Runs on dirty CPU schedulers to avoid blocking BEAM
- Rayon workers compute boundary pairs (pure index arithmetic on the shared structural index) — no data copying
- Main thread builds BEAM sub-binary terms from boundaries (Env is not thread-safe, so term construction is serial)

**`:streaming` specifics:**
- `ResourceArc` integrates parser state with BEAM GC
- Tracks quote state across chunk boundaries
- Copies field data (since input chunks are temporary)

## NIF-Accelerated Encoding

RustyCSV returns one top-level iodata list per row from `dump_to_iodata/2`
and list-shaped row iodata from `dump_to_stream/1`, matching NimbleCSV's public
shape. Each row is one contiguous binary, avoiding one BEAM term per field. Use
`IO.iodata_to_binary/1` when a flat binary is required.

See the [encoding benchmark results](https://rusty-csv.hexdocs.pm/benchmark.html#encoding-benchmark-results) for throughput and memory numbers.

### Encoding Strategies

`dump_to_iodata/2` accepts a `:strategy` option:

```elixir
# Default: single-threaded encoder returning list-shaped iodata.
# SIMD scan for quoting, writes directly to a reusable row buffer.
# Best for most workloads.
CSV.dump_to_iodata(rows)

# Parallel: multi-threaded encoding via rayon.
# Copies field data into Rust-owned memory, encodes rows across worker threads.
# Faster when fields frequently need quoting (commas, quotes, newlines in values).
CSV.dump_to_iodata(rows, strategy: :parallel)
```

| Encoding Strategy | Best For | Output |
|-------------------|----------|--------|
| *default* | Most data — clean fields, moderate quoting | One list-wrapped binary per row |
| `:parallel` | Quoting-heavy data (user-generated content, free-text with embedded commas/quotes/newlines) | One list-wrapped binary per row |

### High-Throughput Concurrent Exports

RustyCSV's encoding NIF runs on BEAM dirty CPU schedulers, making it well-suited for concurrent export workloads (e.g., thousands of users downloading CSV reports simultaneously in a Phoenix application):

```elixir
# Phoenix controller — concurrent CSV download
def export(conn, %{"id" => id}) do
  rows = MyApp.Reports.fetch_rows(id)
  csv = MyCSV.dump_to_iodata(rows)

  conn
  |> put_resp_content_type("text/csv")
  |> put_resp_header("content-disposition", ~s(attachment; filename="report.csv"))
  |> send_resp(200, csv)
end
```

For very large exports where you want bounded memory, use chunked NIF encoding:

```elixir
# Chunked encoding — bounded memory with NIF speed
def stream_export(conn, %{"id" => id}) do
  conn = conn
  |> put_resp_content_type("text/csv")
  |> put_resp_header("content-disposition", ~s(attachment; filename="report.csv"))
  |> send_chunked(200)

  MyApp.Reports.stream_rows(id)
  |> Stream.chunk_every(5_000)
  |> Stream.each(fn chunk ->
    csv = MyCSV.dump_to_iodata(chunk)
    {:ok, _conn} = Plug.Conn.chunk(conn, csv)
  end)
  |> Stream.run()

  conn
end
```

**Key characteristics for concurrent workloads:**

- Each NIF call is independent — no shared mutable state between requests
- Dirty CPU schedulers prevent encoding from blocking normal BEAM schedulers
- Allocator contention is the main scaling limit on the `:parallel` strategies; the opt-in mimalloc build recovers that throughput where a bundled allocator is safe (see [allocator safety](docs/ALLOCATOR_SAFETY.md))
- The real bottleneck is typically DB queries and connection pool sizing, not CSV encoding

## Architecture

RustyCSV is built with a modular Rust architecture:

```
native/rustycsv/src/
├── lib.rs                 # NIF entry points, separator/escape decoding, dispatch
├── core/
│   ├── simd_scanner.rs    # Single-pass SIMD structural scanner (prefix-XOR quote detection)
│   ├── simd_index.rs      # StructuralIndex, RowIter, RowFieldIter, FieldIter
│   ├── scanner.rs         # Byte-level helpers (separator matching)
│   ├── field.rs           # Field extraction, quote handling
│   └── newlines.rs        # Custom newline support
├── strategy/
│   ├── direct.rs          # Basic + SIMD strategies (single-byte)
│   ├── two_phase.rs       # Indexed strategy (single-byte)
│   ├── streaming.rs       # Stateful streaming parser (single-byte)
│   ├── parallel.rs        # Rayon-based parallel parsing (single-byte)
│   ├── zero_copy.rs       # Sub-binary reference parsing (single-byte)
│   ├── general.rs         # Multi-byte separator/escape (all strategies)
│   ├── encode.rs          # SIMD field scanning, quoting helpers
│   └── encoding.rs        # UTF-8 → target encoding converters (UTF-16, Latin-1, etc.)
├── term.rs                # BEAM term building (sub-binary + copy fallback)
└── resource.rs            # ResourceArc for streaming state
```

See the [architecture documentation](https://rusty-csv.hexdocs.pm/architecture.html) for detailed implementation notes.

## Memory Efficiency

The streaming parser does not retain the full input. Its memory use is bounded
by the configured buffer limit, the longest buffered row, and rows retained by
the caller:

```elixir
# Process a 10GB file without loading it all into memory
File.stream!("huge.csv", [], 65_536)
|> CSV.parse_stream()
|> Stream.each(&process/1)
|> Stream.run()
```

### Streaming Buffer Limit

The streaming parser enforces a maximum internal buffer size (default **256 MB**)
to prevent unbounded memory growth when parsing data without newlines or with
very long rows. If a feed exceeds this limit, a `:buffer_overflow` exception is raised.

To adjust the limit, pass `:max_buffer_size` (in bytes):

```elixir
# Increase for files with very long rows
CSV.parse_stream(stream, max_buffer_size: 512 * 1024 * 1024)

# Decrease to fail fast on malformed input
CSV.parse_stream(stream, max_buffer_size: 10 * 1024 * 1024)

# Also works on direct streaming APIs
RustyCSV.Streaming.stream_file("data.csv", max_buffer_size: 1_073_741_824)
```

### Bundled allocators

RustyCSV ships with no bundled memory allocator. Bundling one is a whole-VM
decision rather than a library one: two NIFs that each statically link mimalloc
silently corrupt each other's heaps on macOS and take the VM down. RustyCSV was
one half of exactly that pair — it bundled mimalloc by default through 0.4.4.

[mimalloc](https://github.com/microsoft/mimalloc) remains available as an opt-in
cargo feature, and it is worth real throughput on the multi-threaded paths, but
read [allocator safety](docs/ALLOCATOR_SAFETY.md) and audit the other NIFs in
your release before enabling it:

```bash
RUSTYCSV_ALLOCATOR=mimalloc FORCE_RUSTYCSV_BUILD=1 mix compile
```

The published precompiled artifacts never bundle an allocator.

### Optional Memory Tracking (Benchmarking Only)

For profiling Rust-side memory during development and benchmarking. Not intended for production — it wraps every allocation with atomic counter updates, adding overhead. This is also the only source of `unsafe` in the codebase (required by the `GlobalAlloc` trait). Temporarily add `memory_tracking` to the default features in `native/rustycsv/Cargo.toml`, then force a local build:

```toml
# In native/rustycsv/Cargo.toml
[features]
default = ["memory_tracking"]
```

```bash
FORCE_RUSTYCSV_BUILD=true mix compile --force
```

Then use:

```elixir
RustyCSV.Native.reset_rust_memory_stats()
result = CSV.parse_string(large_csv)
peak = RustyCSV.Native.get_rust_memory_peak()
IO.puts("Peak Rust memory: #{peak / 1_000_000} MB")
```

When disabled (default), these functions return `0` with zero overhead.

## SIMD and Rust Nightly

RustyCSV uses Rust's [`std::simd`](https://doc.rust-lang.org/std/simd/index.html) portable SIMD, which currently requires nightly via `#![feature(portable_simd)]`. However, RustyCSV only uses the stabilization-safe subset of the API:

- `Simd::from_slice`, `splat`, `simd_eq`, bitwise ops (`&`, `|`, `!`)
- `Mask::to_bitmask()` for extracting bit positions

We deliberately avoid the APIs that are [blocking stabilization](https://github.com/rust-lang/portable-simd/issues/364): swizzle, scatter/gather, and lane-count generics (`LaneCount<N>: SupportedLaneCount`). The items blocking the `portable_simd` [tracking issue](https://github.com/rust-lang/rust/issues/86656) — mask semantics, supported vector size limits, and swizzle design — are unrelated to the operations we use. When `std::simd` stabilizes, RustyCSV will work on stable Rust with no code changes.

The prefix-XOR quote detection uses a portable shift-and-xor cascade rather than architecture-specific intrinsics. It handles the 16-, 32-, and 64-bit masks without `unsafe` code.

## Development

```bash
# Install dependencies
mix deps.get

# Compile (includes Rust NIF)
mix compile

# Run tests (422 ExUnit tests)
mix test

# Run benchmarks
mix run bench/decode_bench.exs
mix run bench/encode_bench.exs

# Code quality
mix credo --strict
mix dialyzer
```

## License

MIT License - see LICENSE file for details.

---

**RustyCSV** - Purpose-built Rust NIF for ultra-fast CSV parsing in Elixir.
