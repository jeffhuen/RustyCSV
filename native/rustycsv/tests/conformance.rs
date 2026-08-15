//! Cross-strategy conformance tests
//!
//! Each scenario runs through all parser entrypoints that should produce
//! comparable output. A new scenario automatically tests the direct batch
//! parser, the indexed-alias batch parser, parallel, zero_copy, and streaming
//! entrypoints. Failures pinpoint which entrypoint diverges.

use rustycsv::core::ScannedBoundaries;
use rustycsv::strategy::direct::{parse_csv_full_multi_sep, parse_csv_full_with_config};
use rustycsv::strategy::parallel::{parse_csv_parallel_multi_sep, parse_csv_parallel_with_config};
use rustycsv::strategy::streaming::StreamingParser;
use rustycsv::strategy::two_phase::parse_csv_indexed_with_config;
use rustycsv::strategy::zero_copy::{
    parse_csv_boundaries_multi_sep, parse_csv_boundaries_with_config,
};

use std::borrow::Cow;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn cow_to_strings(rows: Vec<Vec<Cow<'_, [u8]>>>) -> Vec<Vec<String>> {
    rows.into_iter()
        .map(|row| {
            row.into_iter()
                .map(|f| String::from_utf8_lossy(&f).to_string())
                .collect()
        })
        .collect()
}

fn owned_to_strings(rows: Vec<Vec<Vec<u8>>>) -> Vec<Vec<String>> {
    rows.into_iter()
        .map(|row| {
            row.into_iter()
                .map(|f| String::from_utf8_lossy(&f).to_string())
                .collect()
        })
        .collect()
}

/// Convert boundary positions to actual field strings by slicing from input.
fn boundaries_to_strings(input: &[u8], boundaries: ScannedBoundaries) -> Vec<Vec<String>> {
    use rustycsv::core::extract_field_cow_with_escape;
    boundaries
        .rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|(s, e)| {
                    let field = extract_field_cow_with_escape(input, s, e, b'"');
                    String::from_utf8_lossy(&field).to_string()
                })
                .collect()
        })
        .collect()
}

fn streaming_to_strings(input: &[u8], sep: u8) -> Vec<Vec<String>> {
    let mut parser = StreamingParser::with_config(sep, b'"');
    parser.feed(input).unwrap();
    let mut rows = parser.take_rows(usize::MAX);
    rows.extend(parser.finalize());
    owned_to_strings(rows)
}

// ---------------------------------------------------------------------------
// Conformance macro
// ---------------------------------------------------------------------------

/// Runs a scenario through all single-byte-separator strategies and asserts
/// they all produce `expected`.
macro_rules! conformance {
    ($name:ident, input: $input:expr, sep: $sep:expr, expected: $expected:expr) => {
        #[test]
        fn $name() {
            let input: &[u8] = $input;
            let sep: u8 = $sep;
            let expected: Vec<Vec<&str>> = $expected;
            let expected_strings: Vec<Vec<String>> = expected
                .iter()
                .map(|row| row.iter().map(|s| s.to_string()).collect())
                .collect();

            // Direct
            let direct = cow_to_strings(
                parse_csv_full_with_config(input, sep, b'"')
                    .expect("test input fits the structural index"),
            );
            assert_eq!(direct, expected_strings, "FAILED: direct");

            // Two-phase
            let two_phase = cow_to_strings(
                parse_csv_indexed_with_config(input, sep, b'"')
                    .expect("test input fits the structural index"),
            );
            assert_eq!(two_phase, expected_strings, "FAILED: two_phase");

            // Parallel
            let parallel = owned_to_strings(
                parse_csv_parallel_with_config(input, sep, b'"')
                    .expect("test input fits the structural index"),
            );
            assert_eq!(parallel, expected_strings, "FAILED: parallel");

            // Zero-copy (preserves all rows including empty)
            let zc = boundaries_to_strings(
                input,
                parse_csv_boundaries_with_config(input, sep, b'"')
                    .expect("test input fits the structural index"),
            );
            assert_eq!(zc, expected_strings, "FAILED: zero_copy");

            // Streaming
            let stream = streaming_to_strings(input, sep);
            assert_eq!(stream, expected_strings, "FAILED: streaming");
        }
    };
}

// ---------------------------------------------------------------------------
// Scenario: simple two-row CSV
// (was duplicated 10x across simd_scanner, simd_index, direct, two_phase,
//  parallel, zero_copy, streaming)
// ---------------------------------------------------------------------------

conformance!(
    simple_two_rows,
    input: b"a,b,c\n1,2,3\n",
    sep: b',',
    expected: vec![vec!["a", "b", "c"], vec!["1", "2", "3"]]
);

// ---------------------------------------------------------------------------
// Scenario: quoted field containing comma
// (was duplicated 5x)
// ---------------------------------------------------------------------------

conformance!(
    quoted_field_with_comma,
    input: b"a,\"b,c\",d\n",
    sep: b',',
    expected: vec![vec!["a", "b,c", "d"]]
);

// ---------------------------------------------------------------------------
// Scenario: CRLF line endings
// (was duplicated 5x)
// ---------------------------------------------------------------------------

conformance!(
    crlf_line_endings,
    input: b"a,b\r\nc,d\r\n",
    sep: b',',
    expected: vec![vec!["a", "b"], vec!["c", "d"]]
);

// ---------------------------------------------------------------------------
// Scenario: no trailing newline
// (was duplicated 5x)
// ---------------------------------------------------------------------------

conformance!(
    no_trailing_newline,
    input: b"a,b\nc,d",
    sep: b',',
    expected: vec![vec!["a", "b"], vec!["c", "d"]]
);

// ---------------------------------------------------------------------------
// Scenario: escaped/doubled quotes
// (was duplicated 7x across field.rs, simd_scanner, direct, two_phase, streaming)
// ---------------------------------------------------------------------------

conformance!(
    escaped_doubled_quotes,
    input: b"a,\"say \"\"hi\"\"\",c\n",
    sep: b',',
    expected: vec![vec!["a", "say \"hi\"", "c"]]
);

// ---------------------------------------------------------------------------
// Scenario: multiline quoted field
// (was duplicated 4x)
// ---------------------------------------------------------------------------

conformance!(
    multiline_quoted_field,
    input: b"a,\"line1\nline2\",c\n",
    sep: b',',
    expected: vec![vec!["a", "line1\nline2", "c"]]
);

// ---------------------------------------------------------------------------
// Scenario: empty input
// (was duplicated 2x)
// ---------------------------------------------------------------------------

conformance!(
    empty_input,
    input: b"",
    sep: b',',
    expected: vec![]
);

// ---------------------------------------------------------------------------
// Scenario: empty lines
// ---------------------------------------------------------------------------

conformance!(
    empty_lines,
    input: b"a\n\nb\n",
    sep: b',',
    expected: vec![vec!["a"], vec![""], vec!["b"]]
);

// ---------------------------------------------------------------------------
// Multi-separator conformance (direct + parallel + zero_copy)
// ---------------------------------------------------------------------------

macro_rules! conformance_multi_sep {
    ($name:ident, input: $input:expr, seps: $seps:expr, expected: $expected:expr) => {
        #[test]
        fn $name() {
            let input: &[u8] = $input;
            let seps: &[u8] = $seps;
            let expected: Vec<Vec<&str>> = $expected;
            let expected_strings: Vec<Vec<String>> = expected
                .iter()
                .map(|row| row.iter().map(|s| s.to_string()).collect())
                .collect();

            // Direct
            let direct = cow_to_strings(
                parse_csv_full_multi_sep(input, seps, b'"')
                    .expect("test input fits the structural index"),
            );
            assert_eq!(direct, expected_strings, "FAILED: direct multi_sep");

            // Parallel
            let parallel = owned_to_strings(
                parse_csv_parallel_multi_sep(input, seps, b'"')
                    .expect("test input fits the structural index"),
            );
            assert_eq!(parallel, expected_strings, "FAILED: parallel multi_sep");

            // Zero-copy
            let zc = boundaries_to_strings(
                input,
                parse_csv_boundaries_multi_sep(input, seps, b'"')
                    .expect("test input fits the structural index"),
            );
            assert_eq!(zc, expected_strings, "FAILED: zero_copy multi_sep");
        }
    };
}

conformance_multi_sep!(
    multi_sep_semicolon_tab,
    input: b"a;b\tc\n1;2\t3\n",
    seps: b";\t",
    expected: vec![vec!["a", "b", "c"], vec!["1", "2", "3"]]
);

// ---------------------------------------------------------------------------
// General multi-byte conformance (runs same input through all general strategies)
// ---------------------------------------------------------------------------

use rustycsv::core::newlines::Newlines;
use rustycsv::strategy::general::{
    parse_csv_boundaries_general, parse_csv_boundaries_general_with_newlines, parse_csv_general,
    parse_csv_general_with_newlines, parse_csv_indexed_general,
    parse_csv_indexed_general_with_newlines, parse_csv_parallel_general,
    parse_csv_parallel_general_with_newlines, GeneralStreamingParser,
    GeneralStreamingParserNewlines,
};

macro_rules! conformance_general {
    ($name:ident, input: $input:expr, seps: $seps:expr, esc: $esc:expr, expected: $expected:expr) => {
        #[test]
        fn $name() {
            let input: &[u8] = $input;
            let seps: Vec<Vec<u8>> = $seps;
            let esc: Vec<u8> = $esc;
            let expected: Vec<Vec<&str>> = $expected;
            let expected_strings: Vec<Vec<String>> = expected
                .iter()
                .map(|row| row.iter().map(|s| s.to_string()).collect())
                .collect();

            // Direct
            let direct = cow_to_strings(parse_csv_general(input, &seps, &esc));
            assert_eq!(direct, expected_strings, "FAILED: general direct");

            // Indexed
            let indexed = cow_to_strings(parse_csv_indexed_general(input, &seps, &esc));
            assert_eq!(indexed, expected_strings, "FAILED: general indexed");

            // Parallel
            let parallel = owned_to_strings(parse_csv_parallel_general(input, &seps, &esc));
            assert_eq!(parallel, expected_strings, "FAILED: general parallel");

            // Boundaries
            // Note: boundaries returns raw positions, verify count and field count
            let boundaries = parse_csv_boundaries_general(input, &seps, &esc)
                .expect("test input fits the structural index");
            assert_eq!(
                boundaries.rows.len(),
                expected_strings.len(),
                "FAILED: general boundaries row count"
            );

            // Streaming
            let mut parser = GeneralStreamingParser::new(seps.clone(), esc.clone());
            parser.feed(input).unwrap();
            let mut rows = parser.take_rows(usize::MAX);
            rows.extend(parser.finalize());
            let stream = owned_to_strings(rows);
            assert_eq!(stream, expected_strings, "FAILED: general streaming");
        }
    };
}

conformance_general!(
    general_double_colon_separator,
    input: b"a::b::c\n1::2::3\n",
    seps: vec![b"::".to_vec()],
    esc: b"\"".to_vec(),
    expected: vec![vec!["a", "b", "c"], vec!["1", "2", "3"]]
);

// ---------------------------------------------------------------------------
// Custom newline conformance (runs same input through all newline-aware strategies)
// ---------------------------------------------------------------------------

macro_rules! conformance_custom_newline {
    ($name:ident, input: $input:expr, seps: $seps:expr, esc: $esc:expr, nl: $nl:expr, expected: $expected:expr) => {
        #[test]
        fn $name() {
            let input: &[u8] = $input;
            let seps: Vec<Vec<u8>> = $seps;
            let esc: Vec<u8> = $esc;
            let nl: Newlines = $nl;
            let expected: Vec<Vec<&str>> = $expected;
            let expected_strings: Vec<Vec<String>> = expected
                .iter()
                .map(|row| row.iter().map(|s| s.to_string()).collect())
                .collect();

            // Direct with newlines
            let direct = cow_to_strings(parse_csv_general_with_newlines(input, &seps, &esc, &nl));
            assert_eq!(direct, expected_strings, "FAILED: custom_nl direct");

            // Indexed with newlines
            let indexed = cow_to_strings(parse_csv_indexed_general_with_newlines(
                input, &seps, &esc, &nl,
            ));
            assert_eq!(indexed, expected_strings, "FAILED: custom_nl indexed");

            // Parallel with newlines
            let parallel = owned_to_strings(parse_csv_parallel_general_with_newlines(
                input, &seps, &esc, &nl,
            ));
            assert_eq!(parallel, expected_strings, "FAILED: custom_nl parallel");

            // Boundaries with newlines
            let boundaries = parse_csv_boundaries_general_with_newlines(input, &seps, &esc, &nl)
                .expect("test input fits the structural index");
            assert_eq!(
                boundaries.rows.len(),
                expected_strings.len(),
                "FAILED: custom_nl boundaries row count"
            );

            // Streaming with newlines
            let mut parser =
                GeneralStreamingParserNewlines::new(seps.clone(), esc.clone(), nl.clone());
            parser.feed(input).unwrap();
            let mut rows = parser.take_rows(usize::MAX);
            rows.extend(parser.finalize());
            let stream = owned_to_strings(rows);
            assert_eq!(stream, expected_strings, "FAILED: custom_nl streaming");
        }
    };
}

conformance_custom_newline!(
    custom_newline_pipe,
    input: b"a,b|1,2|",
    seps: vec![b",".to_vec()],
    esc: b"\"".to_vec(),
    nl: Newlines::custom(vec![b"|".to_vec()]),
    expected: vec![vec!["a", "b"], vec!["1", "2"]]
);
