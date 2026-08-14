//! Zero-Copy Strategy: Returns field boundaries for sub-binary term construction
//!
//! Instead of copying field data, this strategy returns (start, end) positions
//! that can be used to create BEAM sub-binaries referencing the original input.
//! Uses the SIMD structural scanner for fast boundary detection.
//!
//! These are boundary-producing entry points consumed by the NIF, so they carry
//! any quoting [`Violation`](crate::core::Violation) alongside the rows rather
//! than discarding it. Malformed CSV still yields usable boundaries; only the
//! caller decides whether to treat the violation as fatal.

use crate::core::{scan_structural, InputTooLarge, ScannedBoundaries, StructuralIndex};

/// Collect one `(start, end)` pair per field, dropping empty rows.
fn boundaries_of(idx: &StructuralIndex) -> Vec<Vec<(usize, usize)>> {
    let mut rows = Vec::with_capacity(idx.row_count());

    for row in idx.rows_with_fields() {
        let boundaries: Vec<(usize, usize)> = row
            .fields
            .map(|(fs, fe)| (fs as usize, fe as usize))
            .collect();

        if !boundaries.is_empty() {
            rows.push(boundaries);
        }
    }

    rows
}

/// Parse CSV and return field boundaries (zero-copy approach)
///
/// # Errors
///
/// Returns [`InputTooLarge`] when `input` exceeds
/// [`MAX_INPUT_SIZE`](crate::core::MAX_INPUT_SIZE).
#[allow(dead_code)]
pub fn parse_csv_boundaries(input: &[u8]) -> Result<ScannedBoundaries, InputTooLarge> {
    parse_csv_boundaries_with_config(input, b',', b'"')
}

/// Parse CSV with configurable separator and escape, returning boundaries
///
/// # Errors
///
/// Returns [`InputTooLarge`] when `input` exceeds
/// [`MAX_INPUT_SIZE`](crate::core::MAX_INPUT_SIZE).
pub fn parse_csv_boundaries_with_config(
    input: &[u8],
    separator: u8,
    escape: u8,
) -> Result<ScannedBoundaries, InputTooLarge> {
    let idx = scan_structural(input, &[separator], escape)?;
    Ok(ScannedBoundaries {
        rows: boundaries_of(&idx),
        violation: idx.violation,
    })
}

/// Parse CSV with multiple separator support, returning boundaries
///
/// # Errors
///
/// Returns [`InputTooLarge`] when `input` exceeds
/// [`MAX_INPUT_SIZE`](crate::core::MAX_INPUT_SIZE).
pub fn parse_csv_boundaries_multi_sep(
    input: &[u8],
    separators: &[u8],
    escape: u8,
) -> Result<ScannedBoundaries, InputTooLarge> {
    if separators.len() == 1 {
        return parse_csv_boundaries_with_config(input, separators[0], escape);
    }

    let idx = scan_structural(input, separators, escape)?;
    Ok(ScannedBoundaries {
        rows: boundaries_of(&idx),
        violation: idx.violation,
    })
}

/// Fast path for quote-free CSV
///
/// # Errors
///
/// Returns [`InputTooLarge`] when `input` exceeds
/// [`MAX_INPUT_SIZE`](crate::core::MAX_INPUT_SIZE).
#[allow(dead_code)]
pub fn parse_csv_boundaries_simple(
    input: &[u8],
    separator: u8,
) -> Result<ScannedBoundaries, InputTooLarge> {
    // Uses the same SIMD path; the scanner handles quote-free input efficiently
    parse_csv_boundaries_with_config(input, separator, b'"')
}

#[cfg(test)]
mod tests {
    use super::*;

    // Common scenarios moved to tests/conformance.rs.
    // Only unique zero-copy-specific tests remain here.

    #[test]
    fn test_boundaries_escaped() {
        let input = b"a,\"b\"\"c\",d\n";
        let scanned = parse_csv_boundaries(input).expect("input is well within the size limit");
        assert_eq!(scanned.rows.len(), 1);
        // Field with escaped quote: positions 2-8
        assert_eq!(scanned.rows[0], vec![(0, 1), (2, 8), (9, 10)]);
    }

    #[test]
    fn well_formed_input_reports_no_violation() {
        let scanned = parse_csv_boundaries(b"a,b\n1,2\n").expect("within the size limit");
        assert_eq!(scanned.violation, None);
    }

    #[test]
    fn malformed_input_still_yields_rows_alongside_its_violation() {
        let scanned = parse_csv_boundaries(b"a,b\nx\"y,2\n").expect("within the size limit");
        assert!(
            scanned.violation.is_some(),
            "a quote mid-field breaks the opening-quote rule"
        );
        assert!(
            !scanned.rows.is_empty(),
            "lenient callers must still receive boundaries"
        );
    }
}
