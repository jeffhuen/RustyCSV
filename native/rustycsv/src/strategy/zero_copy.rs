//! Zero-Copy Strategy: Returns field boundaries for sub-binary term construction
//!
//! Instead of copying field data, this strategy returns (start, end) positions
//! that can be used to create BEAM sub-binaries referencing the original input.
//! Uses the SIMD structural scanner for fast boundary detection.

use crate::core::scan_structural;

/// Parse CSV and return field boundaries (zero-copy approach)
#[allow(dead_code)]
pub fn parse_csv_boundaries(input: &[u8]) -> Vec<Vec<(usize, usize)>> {
    parse_csv_boundaries_with_config(input, b',', b'"')
}

/// Parse CSV with configurable separator and escape, returning boundaries
pub fn parse_csv_boundaries_with_config(
    input: &[u8],
    separator: u8,
    escape: u8,
) -> Vec<Vec<(usize, usize)>> {
    let idx = scan_structural(input, &[separator], escape);
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

/// Parse CSV with multiple separator support, returning boundaries
pub fn parse_csv_boundaries_multi_sep(
    input: &[u8],
    separators: &[u8],
    escape: u8,
) -> Vec<Vec<(usize, usize)>> {
    if separators.len() == 1 {
        return parse_csv_boundaries_with_config(input, separators[0], escape);
    }

    let idx = scan_structural(input, separators, escape);
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

/// Fast path for quote-free CSV
#[allow(dead_code)]
pub fn parse_csv_boundaries_simple(input: &[u8], separator: u8) -> Vec<Vec<(usize, usize)>> {
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
        let boundaries = parse_csv_boundaries(input);
        assert_eq!(boundaries.len(), 1);
        // Field with escaped quote: positions 2-8
        assert_eq!(boundaries[0], vec![(0, 1), (2, 8), (9, 10)]);
    }
}
