//! Approach C: Two-Phase Index-then-Extract Parser
//!
//! Phase 1: SIMD structural scan → StructuralIndex (replaces build_index)
//! Phase 2: Extract data using the index
//!
//! Benefits: Better cache utilization, can skip rows, predictable memory usage

use crate::core::{
    extract_field, extract_field_cow, extract_field_cow_with_escape, scan_structural,
    StructuralIndex,
};
use std::borrow::Cow;

/// Represents a field's position within a row
#[derive(Copy, Clone, Debug)]
pub struct FieldBound {
    pub start: usize,
    pub end: usize,
}

/// Index of all row and field boundaries in a CSV
#[derive(Debug)]
pub struct CsvIndex {
    #[allow(dead_code)]
    pub row_bounds: Vec<(usize, usize)>,
    pub field_bounds: Vec<Vec<FieldBound>>,
}

impl CsvIndex {
    #[allow(dead_code)]
    pub fn row_count(&self) -> usize {
        self.row_bounds.len()
    }

    #[allow(dead_code)]
    pub fn field_count(&self, row: usize) -> usize {
        self.field_bounds.get(row).map(|f| f.len()).unwrap_or(0)
    }
}

/// Convert a StructuralIndex into a CsvIndex (for backward compat with extract functions)
fn structural_to_csv_index(idx: &StructuralIndex, input: &[u8]) -> CsvIndex {
    let mut row_bounds = Vec::with_capacity(idx.row_count());
    let mut field_bounds = Vec::with_capacity(idx.row_count());

    for row in idx.rows_with_fields() {
        let rs = row.start as usize;
        let re = row.content_end as usize;
        row_bounds.push((rs, re));

        let fields: Vec<FieldBound> = row
            .fields
            .map(|(fs, fe)| FieldBound {
                start: fs as usize,
                end: fe as usize,
            })
            .collect();
        field_bounds.push(fields);
    }

    // Drop empty trailing row if it's just an empty field from trailing newline
    // (preserve compat with old behavior that skipped empty-only rows via last_field check)
    if let Some(last_fields) = field_bounds.last() {
        if last_fields.len() == 1 {
            let f = &last_fields[0];
            if f.start == f.end && f.start == input.len() {
                field_bounds.pop();
                row_bounds.pop();
            }
        }
    }

    CsvIndex {
        row_bounds,
        field_bounds,
    }
}

/// Phase 1: Build an index of row and field boundaries
#[allow(dead_code)]
pub fn build_index(input: &[u8]) -> CsvIndex {
    build_index_with_config(input, b',', b'"')
}

/// Phase 1: Build an index with configurable separator and escape
pub fn build_index_with_config(input: &[u8], separator: u8, escape: u8) -> CsvIndex {
    let idx = scan_structural(input, &[separator], escape);
    structural_to_csv_index(&idx, input)
}

/// Phase 2: Extract all fields using the index (zero-copy when possible)
#[allow(dead_code)]
pub fn extract_all<'a>(input: &'a [u8], index: &CsvIndex) -> Vec<Vec<Cow<'a, [u8]>>> {
    extract_all_with_escape(input, index, b'"')
}

/// Phase 2: Extract all fields with configurable escape character
pub fn extract_all_with_escape<'a>(
    input: &'a [u8],
    index: &CsvIndex,
    escape: u8,
) -> Vec<Vec<Cow<'a, [u8]>>> {
    index
        .field_bounds
        .iter()
        .map(|row_fields| {
            row_fields
                .iter()
                .map(|bound| extract_field_cow_with_escape(input, bound.start, bound.end, escape))
                .collect()
        })
        .collect()
}

/// Phase 2: Extract all fields using the index (borrowed, no quote unescaping)
#[allow(dead_code)]
pub fn extract_all_borrowed<'a>(input: &'a [u8], index: &CsvIndex) -> Vec<Vec<&'a [u8]>> {
    index
        .field_bounds
        .iter()
        .map(|row_fields| {
            row_fields
                .iter()
                .map(|bound| extract_field(input, bound.start, bound.end))
                .collect()
        })
        .collect()
}

/// Extract a range of rows (for pagination/streaming use cases)
#[allow(dead_code)]
pub fn extract_rows<'a>(
    input: &'a [u8],
    index: &CsvIndex,
    start_row: usize,
    count: usize,
) -> Vec<Vec<Cow<'a, [u8]>>> {
    let end_row = (start_row + count).min(index.row_count());

    index.field_bounds[start_row..end_row]
        .iter()
        .map(|row_fields| {
            row_fields
                .iter()
                .map(|bound| extract_field_cow(input, bound.start, bound.end))
                .collect()
        })
        .collect()
}

/// Combined parse using two-phase approach
pub fn parse_csv_indexed(input: &[u8]) -> Vec<Vec<Cow<'_, [u8]>>> {
    parse_csv_indexed_with_config(input, b',', b'"')
}

/// Combined parse with configurable separator and escape
pub fn parse_csv_indexed_with_config(
    input: &[u8],
    separator: u8,
    escape: u8,
) -> Vec<Vec<Cow<'_, [u8]>>> {
    let index = build_index_with_config(input, separator, escape);
    extract_all_with_escape(input, &index, escape)
}

/// Build an index with multiple separator support
pub fn build_index_multi_sep(input: &[u8], separators: &[u8], escape: u8) -> CsvIndex {
    if separators.len() == 1 {
        return build_index_with_config(input, separators[0], escape);
    }

    let idx = scan_structural(input, separators, escape);
    structural_to_csv_index(&idx, input)
}

/// Combined parse with multiple separator support
pub fn parse_csv_indexed_multi_sep<'a>(
    input: &'a [u8],
    separators: &[u8],
    escape: u8,
) -> Vec<Vec<Cow<'a, [u8]>>> {
    let index = build_index_multi_sep(input, separators, escape);
    extract_all_with_escape(input, &index, escape)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_strings(rows: Vec<Vec<Cow<'_, [u8]>>>) -> Vec<Vec<String>> {
        rows.into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|f| String::from_utf8_lossy(&f).to_string())
                    .collect()
            })
            .collect()
    }

    // Common scenarios moved to tests/conformance.rs.
    // Only unique two-phase-specific tests remain here.

    #[test]
    fn test_extract_rows_range() {
        let input = b"a,b\nc,d\ne,f\n";
        let index = build_index(input);
        let rows = to_strings(extract_rows(input, &index, 1, 1));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], vec!["c", "d"]);
    }
}
