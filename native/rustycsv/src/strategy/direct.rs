//! Direct parsing strategies (A and B — both use the SIMD scanner)
//!
//! Strategies A (basic) and B (SIMD) are now equivalent. Both use the
//! same SIMD structural scanner to find all field separators and row
//! endings in a single pass, then extract fields via Cow slices.
//! The separate function names are retained for backward API compatibility.
//!
//! These entry points are deliberately lenient: they report
//! [`InputTooLarge`](crate::core::InputTooLarge), which makes scanning
//! impossible, but not [`Violation`](crate::core::Violation), which merely
//! means the CSV is malformed. Strictness is offered on the boundary-producing
//! paths the NIF consumes, because nothing here consumes a violation. This is
//! an intentional contract rather than an oversight.

use crate::core::{extract_field_cow_with_escape, scan_structural, InputTooLarge};
use std::borrow::Cow;

/// Parse CSV bytes into Vec of rows, each row is Vec of Cow field slices
pub fn parse_csv_full(input: &[u8]) -> Result<Vec<Vec<Cow<'_, [u8]>>>, InputTooLarge> {
    parse_csv_full_with_config(input, b',', b'"')
}

/// Parse CSV with configurable separator and escape character
pub fn parse_csv_full_with_config(
    input: &[u8],
    separator: u8,
    escape: u8,
) -> Result<Vec<Vec<Cow<'_, [u8]>>>, InputTooLarge> {
    let idx = scan_structural(input, &[separator], escape)?;
    let mut rows = Vec::with_capacity(idx.row_count());

    for row in idx.rows_with_fields() {
        let fields: Vec<Cow<'_, [u8]>> = row
            .fields
            .map(|(fs, fe)| extract_field_cow_with_escape(input, fs as usize, fe as usize, escape))
            .collect();
        rows.push(fields);
    }

    Ok(rows)
}

/// Parse CSV with multiple separator support
pub fn parse_csv_full_multi_sep<'a>(
    input: &'a [u8],
    separators: &[u8],
    escape: u8,
) -> Result<Vec<Vec<Cow<'a, [u8]>>>, InputTooLarge> {
    if separators.len() == 1 {
        return parse_csv_full_with_config(input, separators[0], escape);
    }

    let idx = scan_structural(input, separators, escape)?;
    let mut rows = Vec::with_capacity(idx.row_count());

    for row in idx.rows_with_fields() {
        let fields: Vec<Cow<'a, [u8]>> = row
            .fields
            .map(|(fs, fe)| extract_field_cow_with_escape(input, fs as usize, fe as usize, escape))
            .collect();
        rows.push(fields);
    }

    Ok(rows)
}

/// Approach A: Basic parsing (now uses SIMD scanner)
pub fn parse_csv(input: &[u8]) -> Result<Vec<Vec<Cow<'_, [u8]>>>, InputTooLarge> {
    parse_csv_full(input)
}

/// Approach A with configurable separator and escape
pub fn parse_csv_with_config(
    input: &[u8],
    separator: u8,
    escape: u8,
) -> Result<Vec<Vec<Cow<'_, [u8]>>>, InputTooLarge> {
    parse_csv_full_with_config(input, separator, escape)
}

/// Approach B: SIMD-accelerated parsing
pub fn parse_csv_fast(input: &[u8]) -> Result<Vec<Vec<Cow<'_, [u8]>>>, InputTooLarge> {
    parse_csv_full(input)
}

/// Approach B with configurable separator and escape
pub fn parse_csv_fast_with_config(
    input: &[u8],
    separator: u8,
    escape: u8,
) -> Result<Vec<Vec<Cow<'_, [u8]>>>, InputTooLarge> {
    parse_csv_full_with_config(input, separator, escape)
}

/// Approach A with multiple separator support
pub fn parse_csv_multi_sep<'a>(
    input: &'a [u8],
    separators: &[u8],
    escape: u8,
) -> Result<Vec<Vec<Cow<'a, [u8]>>>, InputTooLarge> {
    parse_csv_full_multi_sep(input, separators, escape)
}

/// Approach B with multiple separator support
pub fn parse_csv_fast_multi_sep<'a>(
    input: &'a [u8],
    separators: &[u8],
    escape: u8,
) -> Result<Vec<Vec<Cow<'a, [u8]>>>, InputTooLarge> {
    parse_csv_full_multi_sep(input, separators, escape)
}

// Tests moved to tests/conformance.rs
