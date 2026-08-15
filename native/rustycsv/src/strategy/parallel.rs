//! Parallel Parser using Rayon
//!
//! The `*_boundaries*` entry points are consumed by the NIF, so they carry any
//! quoting [`Violation`](crate::core::Violation) alongside the rows. The
//! owned-row entry points stay lenient, matching [`direct`](super::direct).
//!
//! Strategy:
//! 1. Single-threaded: SIMD structural scan → row boundaries + field separator positions
//! 2. O(n) cursor walk: collect (row_start, content_end, sep_lo, sep_hi) into a flat Vec
//! 3. Parallel: Each worker slices into the shared field_seps array — no re-scanning
//!
//! ## Evolution of field-position reuse from the structural index
//!
//! The SIMD structural scanner already finds every separator position. We tried
//! three approaches to reuse those positions instead of re-scanning with memchr:
//!
//! Approach A — Pre-collect Vec<Vec<(u32, u32)>> field bounds, then par_iter:
//!   Simple CSV: 567 → 464 ips (-18%)
//!   Large 7MB:  40.6 → 38.0 ips (-6%)
//!   Cause: 10K+ inner Vec allocations for per-row field bounds.
//!
//! Approach B — Share &StructuralIndex, each worker calls fields_in_row() (binary search):
//!   Simple CSV: 567 → 503 ips (-11%)
//!   Large 7MB:  40.6 → 35.7 ips (-12%)
//!   Very Large: 1.99 → 1.80 ips (-10%)
//!   Cause: Two partition_point calls per row = O(log n) per row.
//!
//! Approach C (current) — Flat index + direct slice:
//!   O(n) cursor walk builds a single flat Vec<(u32, u32, usize, usize)> mapping
//!   each row to its slice of field_seps. Each parallel worker indexes directly
//!   into the shared &[u32] — zero per-row allocation, O(1) lookup, no re-scanning.
//!   This avoids A's allocation overhead and B's binary search overhead.
//!
//! Important: We can't build BEAM terms on worker threads, so we return
//! owned Vec<Vec<Vec<u8>>> and convert to terms on the scheduler thread.

use crate::core::{
    extract_field_owned_with_escape, scan_structural, InputTooLarge, ScannedBoundaries,
};
use rayon::prelude::*;
use std::sync::OnceLock;

static CSV_POOL: OnceLock<Option<rayon::ThreadPool>> = OnceLock::new();

pub(crate) fn get_pool() -> Option<&'static rayon::ThreadPool> {
    CSV_POOL
        .get_or_init(|| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(recommended_threads())
                .thread_name(|i| format!("rustycsv-{i}"))
                // Reduce per-thread stack from the 8 MiB default to 2 MiB.
                // CSV field extraction has shallow call stacks; the default
                // wastes ~48 MiB of virtual memory across 8 persistent threads.
                .stack_size(2 * 1024 * 1024)
                .build()
                .ok()
        })
        .as_ref()
}

/// Run a closure on the dedicated CSV thread pool, falling back to the global pool.
pub(crate) fn run_parallel<T: Send, F: FnOnce() -> T + Send>(f: F) -> T {
    match get_pool() {
        Some(pool) => pool.install(f),
        None => f(),
    }
}

/// Parse CSV in parallel, returning owned rows
pub fn parse_csv_parallel(input: &[u8]) -> Result<Vec<Vec<Vec<u8>>>, InputTooLarge> {
    parse_csv_parallel_with_config(input, b',', b'"')
}

/// Parse CSV in parallel with configurable separator and escape
pub fn parse_csv_parallel_with_config(
    input: &[u8],
    separator: u8,
    escape: u8,
) -> Result<Vec<Vec<Vec<u8>>>, InputTooLarge> {
    // Phase 1: SIMD structural scan → row boundaries + field separator positions
    let idx = scan_structural(input, &[separator], escape)?;
    let field_seps: &[u32] = &idx.field_seps;

    // Phase 2: O(n) cursor walk — map each row to its slice of field_seps
    let mut row_ranges: Vec<(u32, u32, usize, usize)> = Vec::with_capacity(idx.row_count());
    let mut sep_cursor: usize = 0;
    for (rs, re, _next) in idx.rows() {
        let sep_start = sep_cursor;
        while sep_cursor < field_seps.len() && field_seps[sep_cursor] < re {
            sep_cursor += 1;
        }
        row_ranges.push((rs, re, sep_start, sep_cursor));
    }

    if row_ranges.is_empty() {
        return Ok(Vec::new());
    }

    // Phase 3: Parallel field extraction — each worker slices into shared field_seps
    Ok(run_parallel(|| {
        row_ranges
            .into_par_iter()
            .map(|(rs, re, sep_lo, sep_hi)| {
                let (row_start, content_end) = (rs as usize, re as usize);
                let seps = &field_seps[sep_lo..sep_hi];
                let mut fields = Vec::with_capacity(seps.len() + 1);
                let mut pos = row_start;
                for &sep_pos in seps {
                    fields.push(extract_field_owned_with_escape(
                        input,
                        pos,
                        sep_pos as usize,
                        escape,
                    ));
                    pos = sep_pos as usize + 1;
                }
                fields.push(extract_field_owned_with_escape(
                    input,
                    pos,
                    content_end,
                    escape,
                ));

                fields
            })
            .collect()
    }))
}

/// Parse CSV in parallel with multiple separator support
pub fn parse_csv_parallel_multi_sep(
    input: &[u8],
    separators: &[u8],
    escape: u8,
) -> Result<Vec<Vec<Vec<u8>>>, InputTooLarge> {
    if separators.len() == 1 {
        return parse_csv_parallel_with_config(input, separators[0], escape);
    }

    // Phase 1: SIMD structural scan → row boundaries + field separator positions
    let idx = scan_structural(input, separators, escape)?;
    let field_seps: &[u32] = &idx.field_seps;

    // Phase 2: O(n) cursor walk — map each row to its slice of field_seps
    let mut row_ranges: Vec<(u32, u32, usize, usize)> = Vec::with_capacity(idx.row_count());
    let mut sep_cursor: usize = 0;
    for (rs, re, _next) in idx.rows() {
        let sep_start = sep_cursor;
        while sep_cursor < field_seps.len() && field_seps[sep_cursor] < re {
            sep_cursor += 1;
        }
        row_ranges.push((rs, re, sep_start, sep_cursor));
    }

    if row_ranges.is_empty() {
        return Ok(Vec::new());
    }

    // Phase 3: Parallel field extraction — each worker slices into shared field_seps
    Ok(run_parallel(|| {
        row_ranges
            .into_par_iter()
            .map(|(rs, re, sep_lo, sep_hi)| {
                let (row_start, content_end) = (rs as usize, re as usize);
                let seps = &field_seps[sep_lo..sep_hi];
                let mut fields = Vec::with_capacity(seps.len() + 1);
                let mut pos = row_start;
                for &sep_pos in seps {
                    fields.push(extract_field_owned_with_escape(
                        input,
                        pos,
                        sep_pos as usize,
                        escape,
                    ));
                    pos = sep_pos as usize + 1;
                }
                fields.push(extract_field_owned_with_escape(
                    input,
                    pos,
                    content_end,
                    escape,
                ));

                fields
            })
            .collect()
    }))
}

// ============================================================================
// Parallel Boundary Extraction (returns (start, end) pairs, no field copying)
// ============================================================================

/// Parse CSV in parallel, returning field boundaries (zero-copy)
pub fn parse_csv_parallel_boundaries(input: &[u8]) -> Result<ScannedBoundaries, InputTooLarge> {
    parse_csv_parallel_boundaries_with_config(input, b',', b'"')
}

/// Parse CSV in parallel with configurable separator and escape, returning boundaries
pub fn parse_csv_parallel_boundaries_with_config(
    input: &[u8],
    separator: u8,
    escape: u8,
) -> Result<ScannedBoundaries, InputTooLarge> {
    // Phase 1: SIMD structural scan → row boundaries + field separator positions
    let idx = scan_structural(input, &[separator], escape)?;
    let field_seps: &[u32] = &idx.field_seps;

    // Phase 2: O(n) cursor walk — map each row to its slice of field_seps
    let mut row_ranges: Vec<(u32, u32, usize, usize)> = Vec::with_capacity(idx.row_count());
    let mut sep_cursor: usize = 0;
    for (rs, re, _next) in idx.rows() {
        let sep_start = sep_cursor;
        while sep_cursor < field_seps.len() && field_seps[sep_cursor] < re {
            sep_cursor += 1;
        }
        row_ranges.push((rs, re, sep_start, sep_cursor));
    }

    if row_ranges.is_empty() {
        return Ok(ScannedBoundaries::well_formed(Vec::new()));
    }

    // Phase 3: Parallel boundary extraction — just push (start, end) tuples
    let rows = run_parallel(|| {
        row_ranges
            .into_par_iter()
            .map(|(rs, re, sep_lo, sep_hi)| {
                let (row_start, content_end) = (rs as usize, re as usize);
                let seps = &field_seps[sep_lo..sep_hi];
                let mut fields = Vec::with_capacity(seps.len() + 1);
                let mut pos = row_start;
                for &sep_pos in seps {
                    fields.push((pos, sep_pos as usize));
                    pos = sep_pos as usize + 1;
                }
                fields.push((pos, content_end));

                fields
            })
            .collect()
    });

    Ok(ScannedBoundaries {
        rows,
        violation: idx.violation,
    })
}

/// Parse CSV in parallel with multiple separator support, returning boundaries
pub fn parse_csv_parallel_boundaries_multi_sep(
    input: &[u8],
    separators: &[u8],
    escape: u8,
) -> Result<ScannedBoundaries, InputTooLarge> {
    if separators.len() == 1 {
        return parse_csv_parallel_boundaries_with_config(input, separators[0], escape);
    }

    // Phase 1: SIMD structural scan → row boundaries + field separator positions
    let idx = scan_structural(input, separators, escape)?;
    let field_seps: &[u32] = &idx.field_seps;

    // Phase 2: O(n) cursor walk — map each row to its slice of field_seps
    let mut row_ranges: Vec<(u32, u32, usize, usize)> = Vec::with_capacity(idx.row_count());
    let mut sep_cursor: usize = 0;
    for (rs, re, _next) in idx.rows() {
        let sep_start = sep_cursor;
        while sep_cursor < field_seps.len() && field_seps[sep_cursor] < re {
            sep_cursor += 1;
        }
        row_ranges.push((rs, re, sep_start, sep_cursor));
    }

    if row_ranges.is_empty() {
        return Ok(ScannedBoundaries::well_formed(Vec::new()));
    }

    // Phase 3: Parallel boundary extraction
    let rows = run_parallel(|| {
        row_ranges
            .into_par_iter()
            .map(|(rs, re, sep_lo, sep_hi)| {
                let (row_start, content_end) = (rs as usize, re as usize);
                let seps = &field_seps[sep_lo..sep_hi];
                let mut fields = Vec::with_capacity(seps.len() + 1);
                let mut pos = row_start;
                for &sep_pos in seps {
                    fields.push((pos, sep_pos as usize));
                    pos = sep_pos as usize + 1;
                }
                fields.push((pos, content_end));

                fields
            })
            .collect()
    });

    Ok(ScannedBoundaries {
        rows,
        violation: idx.violation,
    })
}

/// Configure rayon thread pool size based on system
pub fn recommended_threads() -> usize {
    std::thread::available_parallelism()
        .map(|p| p.get().min(8))
        .unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Common scenarios moved to tests/conformance.rs.
    // Only unique parallel-specific tests remain here.

    #[test]
    fn test_parallel_many_rows() {
        let mut input = Vec::new();
        for i in 0..1000 {
            input.extend_from_slice(format!("{},{},{}\n", i, i + 1, i + 2).as_bytes());
        }

        let rows = parse_csv_parallel(&input).expect("test input fits the structural index");
        assert_eq!(rows.len(), 1000);
        assert_eq!(rows[0], vec![b"0".to_vec(), b"1".to_vec(), b"2".to_vec()]);
        assert_eq!(
            rows[999],
            vec![b"999".to_vec(), b"1000".to_vec(), b"1001".to_vec()]
        );
    }
}
