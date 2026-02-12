# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.8] - 2026-02-11

### Added

- **musl/alpine compatibility** — added target-specific dependency for `mimalloc` on `musl` targets with the `local_dynamic_tls` feature enabled. This ensures stable support and prevents potential issues with thread-local storage when running in Alpine Linux containers (e.g., standard Elixir docker images).

## [0.3.7] - 2026-02-03

### Fixed

- **Disabled `memory_tracking` by default** — the `memory_tracking` Cargo feature was accidentally left enabled in the 0.3.6 release. This feature wraps every allocation/deallocation with atomic counter updates, adding measurable overhead. It is now disabled by default as intended. Enable explicitly for profiling: `default = ["mimalloc", "memory_tracking"]` in `native/rustycsv/Cargo.toml`.

## [0.3.6] - 2026-02-02

Decoding and encoding overhaul. All batch decode strategies now use boundary-based sub-binaries (zero-copy for most fields). Encoding writes a single flat binary instead of an iodata list. **3.5–19x faster** decoding, **2.5–31x faster** encoding vs pure Elixir, with **5–14x less memory** for decoding.

### Added

- **Parallel encoding option** — `dump_to_iodata(rows, strategy: :parallel)` for quoting-heavy workloads
- Encoding benchmarks (`bench/encode_bench.exs`)

### Changed

- **Boundary-based sub-binary decoding** — all batch strategies (`:simd`, `:basic`, `:indexed`, `:zero_copy`, `:parallel`) now parse field boundaries as `(start, end)` offset pairs, then create BEAM sub-binary references into the original input. Only fields requiring quote unescaping (`""` → `"`) are copied. Previously, `:simd`/`:basic`/`:indexed` used `Cow<[u8]>` (copying into `NewBinary` for every field) and `:parallel` double-copied (rayon workers via `to_vec()` + main thread via `NewBinary`).
- **Parallel strategy: boundary extraction** — rayon workers now compute boundary pairs (pure index arithmetic) instead of copying field data. The main thread builds sub-binary terms. Eliminates the double-copy bottleneck that made `:parallel` slower than NimbleCSV on small/medium files.
- **Flat binary encoding** — the encoding NIF now writes raw CSV bytes into a single binary instead of constructing an iodata list, reducing NIF peak memory 3–6x and BEAM-side allocation to 80 bytes

## [0.3.5] - 2026-02-02

Zero `unsafe` in application code. No user-facing API changes.

### Changed

- **Zero `unsafe` in application code** — all parsing, scanning, and term-building code is now fully safe Rust. The only remaining `unsafe` is the `GlobalAlloc` trait impl behind the opt-in `memory_tracking` feature flag (required by the trait).
  - **Sub-binary creation (`term.rs`)**: Replaced hand-rolled `enif_make_sub_binary` FFI call with rustler's safe `Binary::make_subbinary().into()` API, enabled by upstream PR [#719](https://github.com/rusterlium/rustler/pull/719) (`#[inline]` on `make_subbinary_unchecked` + `From<Binary> for Term`)
  - **SIMD quote detection (`simd_scanner.rs`)**: Removed `unsafe` CLMUL (x86_64) and PMULL (aarch64) `std::arch` intrinsics for prefix-XOR. All targets now use the portable shift-and-xor cascade — benchmarked with no measurable difference on 15MB/100K-row workloads
- **rustler dependency** pinned to git master pending 0.37.3 hex release

## [0.3.4] - 2026-02-01

Major internal refactor replacing all per-strategy byte-by-byte parsers with a shared single-pass SIMD structural scanner. No user-facing API changes.

### Changed

- **SIMD structural scanner** — all six parsing strategies now share a single `scan_structural` pass that finds every unquoted separator and row ending in one sweep. Uses `std::simd` portable SIMD (128-bit on all targets, 256-bit on AVX2). Requires Rust nightly (`#![feature(portable_simd)]`), but only uses the [stabilization-safe API subset](https://github.com/rust-lang/portable-simd/issues/364) — no swizzle, scatter/gather, or lane-count generics.
- **`:parallel` strategy overhauled** — phase 1 now uses the shared SIMD scan instead of a separate sequential row-boundary pass

### Performance

- `:zero_copy` — up to 15% faster on small payloads, up to 31% on large
- `:simd` / `:basic` — 25-35% faster across mixed and large workloads
- `:parallel` — 2.4-3.7x faster, now competitive at all file sizes (previously only beneficial at 500MB+)
- Streaming — 2.2x faster than NimbleCSV (was roughly even)
- vs NimbleCSV: 3.7x (simple) to 17.9x (quoted) to 12.5x (108MB)

## [0.3.3] - 2026-01-29

Internal safety hardening and scheduler improvements. No new user-facing features — all changes are on by default with zero configuration required.

### Changed

- **NIF panic safety** — all Rust NIF code paths now use explicit error handling instead of panics, eliminating the possibility of panic-induced lock poisoning or inconsistent state under any input

## [0.3.2] - 2026-01-29

> **⚠️ Note:** Streaming parsers now enforce a 256 MB buffer cap. If your workload
> streams chunks larger than 256 MB without any newline characters, `streaming_feed/2`
> will raise `:buffer_overflow`. This is unlikely to affect real-world CSV data, but
> if needed you can raise the limit with the `:max_buffer_size` option:
>
> ```elixir
> CSV.parse_stream(stream, max_buffer_size: 512 * 1024 * 1024)
> ```

### Added

- **Bounded streaming buffer** — streaming parsers now enforce a maximum buffer size (default 256 MB) to prevent unbounded memory growth when no newlines are encountered
  - `streaming_feed/2` raises `:buffer_overflow` if the buffer would exceed the limit
  - `streaming_set_max_buffer/2` — new NIF to configure the limit per parser instance
  - Configurable via `:max_buffer_size` option on `parse_stream/2`, `stream_file/2`, `stream_enumerable/2`, and `stream_device/2`
- **Dedicated rayon thread pool** — parallel parsing (`parse_string_parallel`, `parse_to_maps_parallel`, and general multi-byte parallel) now runs on a named `rustycsv-*` thread pool instead of the global rayon pool, avoiding contention with other Rayon users in the same VM
- **Atoms module** — internal `mod atoms` block for DRY atom definitions (`ok`, `error`, `mutex_poisoned`, `buffer_overflow`)

### Changed

- **Dirty CPU scheduling** — 12 NIFs that process unbounded input now run on dirty CPU schedulers to avoid blocking normal BEAM schedulers: `parse_string`, `parse_string_with_config`, `parse_string_fast`, `parse_string_fast_with_config`, `parse_string_indexed`, `parse_string_indexed_with_config`, `parse_string_zero_copy`, `parse_string_zero_copy_with_config`, `parse_to_maps`, `streaming_feed`, `streaming_next_rows`, `streaming_finalize`

### Fixed

- **Mutex poisoning recovery** — streaming parser NIFs now return a `:mutex_poisoned` exception instead of panicking if a previous call panicked while holding the lock
- **Sub-binary bounds check** — `make_subbinary` now validates `start + len <= input_len` with a `debug_assert!` in dev/test builds and a release-mode safety net that returns an empty binary instead of undefined behavior

## [0.3.1] - 2026-01-28

### Added

- **Custom newline support** — pass `newlines` option through to the Rust parser so custom line terminators work for parsing, not just dumping
  - `newlines: ["|"]` — single-byte custom newline
  - `newlines: ["<br>"]` — multi-byte custom newline
  - `newlines: ["<br>", "|"]` — multiple custom newlines
  - Default `["\r\n", "\n"]` routes through existing SIMD-optimized paths — zero performance impact
  - Custom newlines route through the general byte-by-byte parser
  - Works with all strategies: `:basic`, `:simd`, `:indexed`, `:parallel`, `:zero_copy`
  - Works with streaming (`parse_stream/2`)
  - Works with headers-to-maps (`headers: true`)

### Fixed

- **`escape_formula` uses configured replacement** — no longer hardcodes `\t` prefix; respects the map's replacement value (e.g. `%{["@", "+"] => "'"}` now prepends `'` instead of `\t`)
- **`escape_chars` uses configured newlines** — custom newlines and `line_separator` now trigger quoting during dump instead of hardcoded `\n`/`\r`
- **`options/0` normalizes separator to a list** — always returns separator as a list (e.g. `[","]`) to match NimbleCSV behavior
- **`parse_enumerable` avoids eager concatenation** — delegates to `parse_stream` instead of `Enum.join`, keeping peak memory proportional to result + one chunk
- **Integer codepoints accepted for `:separator` and `:escape`** — e.g. `separator: ?,, escape: ?"` now works for NimbleCSV compatibility

## [0.3.0] - 2026-01-28

### Added

- **Headers-to-maps** — return rows as Elixir maps instead of lists
  - `headers: true` — first row becomes string keys
  - `headers: [:name, :age]` — explicit atom keys
  - `headers: ["n", "a"]` — explicit string keys
  - Works with `parse_string/2` (Rust-side map construction) and `parse_stream/2`
    (Elixir-side `Stream.transform`)
  - Rust-side key interning: header terms allocated once and reused across all rows
  - Edge cases: fewer columns → `nil`, extra columns → ignored, duplicate headers → last wins
  - All 5 batch strategies and streaming supported
  - 97 new tests including cross-strategy consistency and parse_string/parse_stream agreement

- **Multi-separator support** — multiple separator characters for NimbleCSV compatibility
  - `separator: [",", ";"]` — accepts a list of separator strings
  - **Parsing**: Any separator in the list is recognized as a field delimiter
  - **Dumping**: Only the **first** separator is used for output (deterministic)
  - Uses SIMD-optimized `memchr2`/`memchr3` for 2-3 single-byte separators, with fallback for 4+
  - Works with all parsing strategies and streaming
  - Backward compatible: single separator string still works as before

### Fixed

- **Multi-byte separator and escape support** - Separators and escape sequences are no longer
  restricted to single bytes, completing NimbleCSV parity
  - `separator: "::"` or `separator: "||"` — multi-byte separators now work
  - `separator: [",", "::"]` — lists can mix single-byte and multi-byte separators
  - `escape: "$$"` — multi-byte escape sequences now work
  - Single-byte cases are unchanged — the existing SIMD-optimized code paths are
    used when all separators and the escape are single bytes (zero performance regression)
  - Multi-byte cases use a new general-purpose byte-by-byte parser
  - All 6 strategies and streaming support multi-byte separators and escapes

## [0.2.0] - 2026-01-25

### Added

- **`:zero_copy` strategy** - New parsing strategy using BEAM sub-binary references
  - Zero-copy for unquoted and simply-quoted fields
  - Hybrid approach: only copies when quote unescaping is needed (`""` → `"`)
  - Matches NimbleCSV's memory model while keeping SIMD scanning speed
  - Trade-off: sub-binaries keep parent binary alive until GC

- **SIMD-accelerated row boundary scanning** - `memchr3` for parallel strategy
  - Replaces byte-by-byte scanning with hardware-accelerated jumps
  - Only examines positions where quotes or newlines appear
  - Properly handles RFC 4180 escaped quotes

- **mimalloc allocator** - High-performance memory allocator (enabled by default)
  - 10-20% faster allocation for many small objects
  - Reduced memory fragmentation
  - Zero tracking overhead in default configuration

- **Optional memory tracking** - Opt-in profiling via `memory_tracking` Cargo feature
  - When disabled (default): `get_rust_memory/0` etc. return `0` with zero overhead
  - When enabled: full allocation tracking for profiling
  - Enable with `default = ["mimalloc", "memory_tracking"]` in Cargo.toml

### Changed

- Memory tracking is now opt-in instead of always-on (removes ~5-10% overhead)
- Pre-allocated vectors throughout parsing paths for reduced reallocation
- Updated ARCHITECTURE.md with comprehensive strategy documentation
- Six parsing strategies now available (was five)

### Performance

- `:parallel` strategy benefits from SIMD row boundary scanning
- `:zero_copy` strategy eliminates copy overhead for clean CSV data
- All strategies benefit from mimalloc and pre-allocation improvements

### Fixed

- **Benchmark methodology** - Corrected unfair streaming comparison (NimbleCSV now uses line-based streams)
- **Memory claims** - Honest metrics showing both BEAM and Rust allocations
- **`:parallel` threshold** - Updated from 100MB+ to 500MB+ based on actual crossover testing
- Documentation now accurately reflects 3.5x-9x speedups (up to 18x for quoted data)

## [0.1.0] - 2025-01-25

### Added

- Initial release
- Five parsing strategies: `:simd`, `:parallel`, `:streaming`, `:indexed`, `:basic`
- Full NimbleCSV API compatibility
- RFC 4180 compliance with 147 tests
- Configurable separators (CSV, TSV, PSV, etc.)
- Bounded-memory streaming for large files
- Character encoding support: UTF-8, UTF-16 (LE/BE), UTF-32 (LE/BE), Latin-1
- Pre-defined `RustyCSV.Spreadsheet` parser for Excel-compatible UTF-16 LE TSV
- Rust memory tracking for profiling (now opt-in, see Unreleased)
- Comprehensive documentation

### Parsing Strategies

- **`:simd`** - SIMD-accelerated delimiter scanning via `memchr` (default)
- **`:parallel`** - Multi-threaded parsing via `rayon` for 500MB+ files with complex quoting
- **`:streaming`** - Stateful chunked parser for unbounded files
- **`:indexed`** - Two-phase index-then-extract for row range access
- **`:basic`** - Simple byte-by-byte parsing for debugging

### Encoding Support

- `:utf8` - UTF-8 (default, zero overhead)
- `:latin1` - ISO-8859-1 / Latin-1
- `{:utf16, :little}` - UTF-16 Little Endian (Excel/Windows)
- `{:utf16, :big}` - UTF-16 Big Endian
- `{:utf32, :little}` - UTF-32 Little Endian
- `{:utf32, :big}` - UTF-32 Big Endian

### Validation

- csv-spectrum acid test suite (12 tests)
- csv-test-data RFC 4180 suite (17 tests)
- PapaParse-inspired edge cases (53 tests)
- Encoding conversion tests (20 tests)
- Cross-strategy consistency validation
- NimbleCSV output compatibility verification
