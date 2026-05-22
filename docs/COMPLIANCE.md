# RustyCSV Compliance & Validation

RustyCSV takes correctness seriously. With **464 tests** across multiple test suites, including industry-standard validation suites used by CSV parsers across multiple languages, RustyCSV is one of the most thoroughly tested CSV libraries available for Elixir.

This document describes RFC 4180 compliance and the validation methodology.

## RFC 4180 Compliance

RustyCSV.RFC4180 is fully compliant with [RFC 4180](https://tools.ietf.org/html/rfc4180) (Common Format and MIME Type for Comma-Separated Values).

### RFC 4180 Requirements

| Section | Requirement | Status |
|---------|-------------|--------|
| 2.1 | Records separated by line breaks (CRLF) | ✅ Accepts CRLF and LF; outputs CRLF |
| 2.2 | Last record may or may not have trailing line break | ✅ |
| 2.3 | Optional header line | ✅ Via `skip_headers` and `headers:` options |
| 2.4 | Each record should have same number of fields | ✅ Parses variable-width rows |
| 2.5 | Spaces are part of the field | ✅ Preserved exactly |
| 2.6 | Fields may be enclosed in double quotes | ✅ |
| 2.6 | Fields containing CRLF must be quoted | ✅ |
| 2.6 | Fields containing double quotes must be quoted | ✅ |
| 2.6 | Fields containing commas must be quoted | ✅ |
| 2.7 | Double quotes escaped by doubling (`""`) | ✅ |

### Line Ending Behavior

**Parsing:**
- Accepts both CRLF (`\r\n`) and LF (`\n`) as record separators
- Preserves embedded CRLF/LF inside quoted fields exactly as-is

**Dumping:**
- Uses CRLF (`\r\n`) as the record separator (RFC 4180 compliant)
- Matches NimbleCSV.RFC4180 output exactly

### Differences from Strict RFC 4180

RustyCSV makes one practical concession shared by most CSV implementations:

1. **Accepts LF line endings** - RFC 4180 specifies CRLF, but LF-only files are common on Unix systems. RustyCSV parses both.

### Bare Carriage Return (`\r`)

A bare `\r` not followed by `\n` is treated as field data, not a line ending. This matches:
- The RFC 4180 ABNF grammar (bare `\r` is not in `TEXTDATA`, only valid inside quoted fields)
- NimbleCSV (only `\r\n` and `\n` are line endings)
- Go `encoding/csv`, Ruby CSV, PostgreSQL COPY

Python's `csv` module differs — it treats bare `\r` as a line ending via universal newline handling.

---

## Industry Test Suites

RustyCSV validates correctness against two industry-standard CSV test suites.

### csv-spectrum (Acid Test)

**Source:** https://github.com/max-mapper/csv-spectrum

The csv-spectrum suite is a widely-used "acid test" for CSV parsers, providing CSV files with JSON expected outputs for verification.

**Note:** The csv-spectrum repository's raw files have LF line endings due to git normalization. Our test fixtures match the actual content served by GitHub, and we verify that both RustyCSV and NimbleCSV produce identical output for these files.

| Test File | Edge Case | Status |
|-----------|-----------|--------|
| `simple.csv` | Basic parsing | ✅ |
| `simple_crlf.csv` | CRLF line endings | ✅ |
| `comma_in_quotes.csv` | Commas inside quoted fields | ✅ |
| `escaped_quotes.csv` | Doubled quotes (`""` → `"`) | ✅ |
| `newlines.csv` | LF inside quoted fields | ✅ |
| `newlines_crlf.csv` | CRLF inside quoted fields | ✅ |
| `quotes_and_newlines.csv` | Combined edge cases | ✅ |
| `empty.csv` | Headers only (LF) | ✅ |
| `empty_crlf.csv` | Headers only (CRLF) | ✅ |
| `utf8.csv` | Unicode content | ✅ |
| `json.csv` | JSON-like content in fields | ✅ |
| `location_coordinates.csv` | Numeric/coordinate data | ✅ |

**Test file:** `test/csv_spectrum_test.exs`

### csv-test-data (RFC 4180 Focused)

**Source:** https://github.com/sineemore/csv-test-data

A comprehensive RFC 4180-focused test suite with both valid and invalid CSV cases.

#### Valid Cases

| Test File | Edge Case | Status |
|-----------|-----------|--------|
| `simple-lf.csv` | Basic with LF endings | ✅ |
| `simple-crlf.csv` | Basic with CRLF endings | ✅ |
| `quotes-with-comma.csv` | Commas in quoted fields | ✅ |
| `quotes-with-escaped-quote.csv` | Escaped quotes | ✅ |
| `quotes-with-newline.csv` | Newlines in quoted fields | ✅ |
| `quotes-with-space.csv` | Spaces in quoted fields | ✅ |
| `quotes-empty.csv` | Empty quoted fields | ✅ |
| `empty-field.csv` | Empty unquoted fields | ✅ |
| `one-column.csv` | Single column | ✅ |
| `empty-one-column.csv` | Single empty column | ✅ |
| `leading-space.csv` | Leading spaces preserved | ✅ |
| `trailing-space.csv` | Trailing spaces preserved | ✅ |
| `trailing-newline.csv` | File ends with newline | ✅ |
| `utf8.csv` | UTF-8 encoded content | ✅ |
| `header-simple.csv` | Basic with header row | ✅ |
| `header-no-rows.csv` | Headers only, no data | ✅ |
| `all-empty.csv` | All empty fields | ✅ |

**Test file:** `test/rfc4180_test_data_test.exs`

---

## Edge Case Tests (PapaParse-inspired)

**Source:** https://github.com/mholt/PapaParse/blob/master/tests/test-cases.js

A comprehensive edge case test suite inspired by PapaParse, covering malformed input, unusual delimiters, and stress testing.

| Category | Test Cases |
|----------|-----------|
| Basic parsing | Empty input, single field, delimiter-only |
| Whitespace | Edges, tabs, quoted whitespace |
| Quoted fields | Delimiters, newlines, escaped quotes |
| Empty fields | Leading, trailing, consecutive |
| Line endings | LF, CRLF, mixed, no trailing |
| Field counts | Ragged rows, single/many columns |
| Unicode | UTF-8, emoji, mixed scripts, BOM |
| Special chars | Null bytes, control chars, backslash |
| Large data | 100K char fields, 1000 rows, 500 columns |
| Strategy consistency | All strategies produce identical output |

**Test file:** `test/edge_cases_test.exs`

---

## Cross-Strategy Validation

All public batch strategy atoms must produce identical output for the same input.
This is verified by running the shared batch suites across all five public batch
atoms. `parse_stream/2` is validated separately against batch parsing because it
uses a different stateful parser.

| Entry Point | Description | Validates Against |
|-------------|-------------|-------------------|
| `:basic` | Public alias of the SIMD batch path | Shared batch suites |
| `:simd` | SIMD batch path (default) | Shared batch suites |
| `:indexed` | Public alias of the SIMD batch path | Shared batch suites |
| `:parallel` | Parallel batch path via rayon | Shared batch suites |
| `:zero_copy` | Public alias of the SIMD batch path | Shared batch suites |
| `parse_stream/2` | Stateful streaming parser | Stream-vs-batch consistency tests |

```elixir
# Shared batch-strategy atom matrix
for strategy <- [:basic, :simd, :indexed, :parallel, :zero_copy] do
  test "all tests pass with #{strategy} strategy" do
    for name <- test_files do
      result = CSV.parse_string(csv, strategy: strategy)
      assert result == expected
    end
  end
end
```

---

## NimbleCSV Compatibility

RustyCSV is designed as a drop-in replacement for NimbleCSV. Compatibility is verified by:

1. **API compatibility tests** - All NimbleCSV API functions work identically
2. **Output matching** - `dump_to_iodata/1` produces identical output to NimbleCSV
3. **Round-trip tests** - Parse → dump → parse produces identical data
4. **Full-file validation** - 100K-row CSV parsed through both libraries produces identical row-by-row output

**Test file:** `test/nimble_csv_compat_test.exs`

```elixir
# Verify dump output matches NimbleCSV exactly
test "dump output matches NimbleCSV" do
  data = [["a", "b"], ["1", "2"]]
  assert RustyCSV.RFC4180.dump_to_iodata(data) ==
         NimbleCSV.RFC4180.dump_to_iodata(data)
end
```

### Known Behavioral Differences

Any input NimbleCSV accepts, RustyCSV also accepts with identical output. Two edge cases differ:

**1. Unquoted fields containing double-quote characters**

Input: ` "a",b\n` (space before quote)

| Parser | Behavior |
|--------|----------|
| NimbleCSV | Raises `ParseError` |
| RustyCSV | Parses as `[" \"a\"", "b"]` |

Both behaviors are defensible for malformed input. NimbleCSV treats any `"` as significant. RustyCSV treats a field as quoted only if it starts with the escape character, matching Python's `csv` module and Go's `encoding/csv` (with `LazyQuotes`).

**2. `parse_stream/2` with non-line-delimited chunks**

The two libraries use different streaming architectures. NimbleCSV's `parse_stream` expects each element of the input enumerable to be a complete line. RustyCSV's streaming parser accepts arbitrary chunk boundaries because the Rust NIF maintains parse state across `feed()` calls.

This difference is invisible for the standard use case (`File.stream! |> parse_stream`), where both produce identical output. It only surfaces when manually constructing a stream of non-line-delimited binary chunks.

---

## Running Compliance Tests

```bash
# Run all tests including compliance suites
mix test

# Run only compliance tests
mix test test/csv_spectrum_test.exs test/rfc4180_test_data_test.exs

# Run with specific strategy
mix test --only strategy:parallel
```

---

## Test Fixtures

Test fixtures are stored in `test/fixtures/`:

```
test/fixtures/
├── csv-spectrum/           # csv-spectrum acid test suite
│   ├── *.csv              # CSV test files
│   └── *.json             # Expected JSON outputs
└── csv-test-data/         # RFC 4180 test suite
    ├── *.csv              # Valid/invalid CSV files
    └── *.json             # Expected outputs
```

---

## Test Summary

| Suite | Tests | Purpose |
|-------|-------|---------|
| Core tests | 36 | Basic functionality and NimbleCSV compatibility |
| csv-spectrum | 17 | Industry acid test |
| csv-test-data | 23 | RFC 4180 compliance |
| Edge cases | 53 | Stress testing and malformed input |
| Encoding | 20 | UTF-16, UTF-32, Latin-1 conversion |
| Multi-separator | 19 | Multiple single-byte separator support |
| Multi-byte separator | 13 | Multi-byte separator support (`::`, `||`, mixed) |
| Multi-byte escape | 12 | Multi-byte escape support (`$$`) |
| Native API | 40 | NIF-level separator/escape encoding |
| Headers-to-maps | 97 | `headers:` option, cross-strategy consistency, stream parity |
| Custom newlines | 18 | Custom newline parsing and streaming |
| Streaming safety | 12 | Buffer overflow, mutex poisoning, concurrent access |
| Concurrent access | 7 | Multi-process streaming safety |
| **Total** | **464** | |

---

## Additional Test Resources

The following resources provide additional CSV test cases that may be valuable for future validation:

### W3C CSVW Test Suite
The W3C CSV on the Web (CSVW) test suite contains 550+ tests for CSV validation and conversion to JSON/RDF. While focused on metadata and semantic representation, the parsing tests are valuable.
- https://w3c.github.io/csvw/tests/

### csv-fuzz (Fuzzing)
Fuzzing-based testing using Jazzer to find crashes, exceptions, and memory issues.
- https://github.com/centic9/csv-fuzz

---

## References

- [RFC 4180](https://tools.ietf.org/html/rfc4180) - Common Format and MIME Type for CSV
- [csv-spectrum](https://github.com/max-mapper/csv-spectrum) - CSV acid test suite
- [csv-test-data](https://github.com/sineemore/csv-test-data) - RFC 4180 test data
- [PapaParse](https://github.com/mholt/PapaParse) - JavaScript CSV parser with comprehensive test suite
- [W3C CSVW Tests](https://w3c.github.io/csvw/tests/) - W3C CSV on the Web test suite
- [NimbleCSV](https://github.com/dashbitco/nimble_csv) - Elixir CSV library (compatibility target)
