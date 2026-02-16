// General multi-byte separator and escape strategy
//
// This module handles arbitrary-length separators and escape sequences.
// It is only used when at least one separator or the escape is multi-byte.
// For single-byte cases, the existing optimized strategies are used instead.
//
// No SIMD — clean byte-by-byte with starts_with checks. This is acceptable
// since multi-byte delimiters are uncommon.

use std::borrow::Cow;

use super::streaming::shrink_excess;
use crate::core::newlines::{match_newline, Newlines};

// ============================================================================
// Helpers
// ============================================================================

/// Check if any separator matches at position. Returns separator length if matched.
#[inline]
fn matches_separator(data: &[u8], pos: usize, separators: &[Vec<u8>]) -> Option<usize> {
    for sep in separators {
        if data[pos..].starts_with(sep) {
            return Some(sep.len());
        }
    }
    None
}

/// Check if escape sequence starts at position.
#[inline]
fn starts_with_escape(data: &[u8], pos: usize, escape: &[u8]) -> bool {
    pos + escape.len() <= data.len() && data[pos..pos + escape.len()] == *escape
}

/// Unescape doubled multi-byte escape sequences in a field's inner content.
/// E.g., for escape `$$`: `val$$$$ue` → `val$$ue`
pub fn unescape_field_general(inner: &[u8], escape: &[u8]) -> Vec<u8> {
    let esc_len = escape.len();
    let mut result = Vec::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        if i + 2 * esc_len <= inner.len()
            && inner[i..i + esc_len] == *escape
            && inner[i + esc_len..i + 2 * esc_len] == *escape
        {
            result.extend_from_slice(escape);
            i += 2 * esc_len;
        } else {
            result.push(inner[i]);
            i += 1;
        }
    }
    result
}

/// Check if inner content contains the escape sequence
pub fn contains_escape(inner: &[u8], escape: &[u8]) -> bool {
    if escape.len() == 1 {
        return inner.contains(&escape[0]);
    }
    inner.windows(escape.len()).any(|w| w == escape)
}

/// Extract a field with multi-byte escape support (Cow version)
pub fn extract_field_cow_general<'a>(
    input: &'a [u8],
    start: usize,
    end: usize,
    escape: &[u8],
) -> Cow<'a, [u8]> {
    if start >= end {
        return Cow::Borrowed(&[]);
    }

    let field = &input[start..end];
    let esc_len = escape.len();

    // Check if quoted (starts and ends with escape)
    if field.len() >= 2 * esc_len
        && field[..esc_len] == *escape
        && field[field.len() - esc_len..] == *escape
    {
        let inner = &field[esc_len..field.len() - esc_len];

        if contains_escape(inner, escape) {
            // Must unescape doubled escapes
            Cow::Owned(unescape_field_general(inner, escape))
        } else {
            // Quoted but no escapes inside
            Cow::Borrowed(inner)
        }
    } else {
        // Unquoted
        Cow::Borrowed(field)
    }
}

/// Extract a field with multi-byte escape support (owned version)
pub fn extract_field_owned_general(
    input: &[u8],
    start: usize,
    end: usize,
    escape: &[u8],
) -> Vec<u8> {
    extract_field_cow_general(input, start, end, escape).into_owned()
}

// ============================================================================
// Strategy A/B: General direct parsing (Cow)
// ============================================================================

/// General parser: handles any separator/escape lengths.
pub fn parse_csv_general<'a>(
    input: &'a [u8],
    separators: &[Vec<u8>],
    escape: &[u8],
) -> Vec<Vec<Cow<'a, [u8]>>> {
    let mut rows = Vec::new();
    let mut pos = 0;

    while pos < input.len() {
        let (row, next_pos) = parse_row_general(input, pos, separators, escape);
        rows.push(row);
        pos = next_pos;
    }

    rows
}

fn parse_row_general<'a>(
    input: &'a [u8],
    start: usize,
    separators: &[Vec<u8>],
    escape: &[u8],
) -> (Vec<Cow<'a, [u8]>>, usize) {
    let mut fields = Vec::new();
    let mut pos = start;
    let mut field_start = start;
    let mut in_quotes = false;
    let esc_len = escape.len();

    while pos < input.len() {
        if in_quotes {
            if starts_with_escape(input, pos, escape) {
                // Check for doubled escape (escaped escape)
                if starts_with_escape(input, pos + esc_len, escape) {
                    pos += 2 * esc_len;
                    continue;
                }
                in_quotes = false;
                pos += esc_len;
            } else {
                pos += 1;
            }
        } else if starts_with_escape(input, pos, escape) {
            in_quotes = true;
            pos += esc_len;
        } else if let Some(sep_len) = matches_separator(input, pos, separators) {
            fields.push(extract_field_cow_general(input, field_start, pos, escape));
            pos += sep_len;
            field_start = pos;
        } else if input[pos] == b'\n' {
            fields.push(extract_field_cow_general(input, field_start, pos, escape));
            pos += 1;
            return (fields, pos);
        } else if input[pos] == b'\r' && pos + 1 < input.len() && input[pos + 1] == b'\n' {
            // CRLF: end of row. Bare \r is data per RFC 4180.
            fields.push(extract_field_cow_general(input, field_start, pos, escape));
            pos += 2;
            return (fields, pos);
        } else {
            pos += 1;
        }
    }

    // Handle last field (no trailing newline)
    if field_start <= input.len() {
        fields.push(extract_field_cow_general(input, field_start, pos, escape));
    }

    (fields, pos)
}

// ============================================================================
// Strategy C: General two-phase index-then-extract
// ============================================================================

/// Field boundary for general parsing
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct GeneralFieldBound {
    pub start: usize,
    pub end: usize,
}

/// Build an index of row/field boundaries with multi-byte separators/escape
pub fn build_index_general(
    input: &[u8],
    separators: &[Vec<u8>],
    escape: &[u8],
) -> Vec<Vec<GeneralFieldBound>> {
    let mut all_fields = Vec::new();
    let mut pos = 0;

    while pos < input.len() {
        let (fields, next_pos) = index_row_general(input, pos, separators, escape);
        if !fields.is_empty() {
            all_fields.push(fields);
        }
        pos = next_pos;
    }

    all_fields
}

fn index_row_general(
    input: &[u8],
    start: usize,
    separators: &[Vec<u8>],
    escape: &[u8],
) -> (Vec<GeneralFieldBound>, usize) {
    let mut fields = Vec::new();
    let mut pos = start;
    let mut field_start = start;
    let mut in_quotes = false;
    let esc_len = escape.len();

    while pos < input.len() {
        if in_quotes {
            if starts_with_escape(input, pos, escape) {
                if starts_with_escape(input, pos + esc_len, escape) {
                    pos += 2 * esc_len;
                    continue;
                }
                in_quotes = false;
                pos += esc_len;
            } else {
                pos += 1;
            }
        } else if starts_with_escape(input, pos, escape) {
            in_quotes = true;
            pos += esc_len;
        } else if let Some(sep_len) = matches_separator(input, pos, separators) {
            fields.push(GeneralFieldBound {
                start: field_start,
                end: pos,
            });
            pos += sep_len;
            field_start = pos;
        } else if input[pos] == b'\n' {
            fields.push(GeneralFieldBound {
                start: field_start,
                end: pos,
            });
            pos += 1;
            return (fields, pos);
        } else if input[pos] == b'\r' && pos + 1 < input.len() && input[pos + 1] == b'\n' {
            // CRLF: end of row. Bare \r is data per RFC 4180.
            fields.push(GeneralFieldBound {
                start: field_start,
                end: pos,
            });
            pos += 2;
            return (fields, pos);
        } else {
            pos += 1;
        }
    }

    // End of input
    if field_start < input.len() || !fields.is_empty() {
        fields.push(GeneralFieldBound {
            start: field_start,
            end: input.len(),
        });
    }

    (fields, pos)
}

/// Combined parse using two-phase approach with multi-byte support
pub fn parse_csv_indexed_general<'a>(
    input: &'a [u8],
    separators: &[Vec<u8>],
    escape: &[u8],
) -> Vec<Vec<Cow<'a, [u8]>>> {
    let index = build_index_general(input, separators, escape);
    index
        .iter()
        .map(|row_fields| {
            row_fields
                .iter()
                .map(|bound| extract_field_cow_general(input, bound.start, bound.end, escape))
                .collect()
        })
        .collect()
}

// ============================================================================
// Strategy E: General parallel parsing
// ============================================================================

/// Core row-start finder: walks input with quote tracking, calling `check_newline`
/// at each unquoted position. `check_newline(input, pos)` returns the newline length
/// (0 if no newline at that position). Monomorphized per call site — zero overhead.
fn find_row_starts_general_inner(
    input: &[u8],
    escape: &[u8],
    check_newline: impl Fn(&[u8], usize) -> usize,
) -> Vec<usize> {
    let mut starts = Vec::with_capacity(input.len() / 50 + 1);
    starts.push(0);
    let mut pos = 0;
    let mut in_quotes = false;
    let esc_len = escape.len();

    while pos < input.len() {
        if in_quotes {
            if starts_with_escape(input, pos, escape) {
                if starts_with_escape(input, pos + esc_len, escape) {
                    pos += 2 * esc_len;
                    continue;
                }
                in_quotes = false;
                pos += esc_len;
            } else {
                pos += 1;
            }
        } else if starts_with_escape(input, pos, escape) {
            in_quotes = true;
            pos += esc_len;
        } else {
            let nl_len = check_newline(input, pos);
            if nl_len > 0 {
                pos += nl_len;
                if pos < input.len() {
                    starts.push(pos);
                }
            } else {
                pos += 1;
            }
        }
    }

    starts
}

/// Default newline check: \n (len 1) or \r\n (len 2). Bare \r is data per RFC 4180.
#[inline]
fn default_newline_len(input: &[u8], pos: usize) -> usize {
    if input[pos] == b'\n' {
        1
    } else if input[pos] == b'\r' && pos + 1 < input.len() && input[pos + 1] == b'\n' {
        2
    } else {
        0
    }
}

/// Find all row start positions with multi-byte escape
pub fn find_row_starts_general(input: &[u8], escape: &[u8]) -> Vec<usize> {
    find_row_starts_general_inner(input, escape, default_newline_len)
}

/// Parse a single line into owned fields with multi-byte separator/escape support
fn parse_line_fields_owned_general(
    line: &[u8],
    separators: &[Vec<u8>],
    escape: &[u8],
) -> Vec<Vec<u8>> {
    let mut fields = Vec::with_capacity(8);
    let mut pos = 0;
    let mut field_start = 0;
    let mut in_quotes = false;
    let esc_len = escape.len();

    while pos < line.len() {
        if in_quotes {
            if starts_with_escape(line, pos, escape) {
                if starts_with_escape(line, pos + esc_len, escape) {
                    pos += 2 * esc_len;
                    continue;
                }
                in_quotes = false;
                pos += esc_len;
            } else {
                pos += 1;
            }
        } else if starts_with_escape(line, pos, escape) {
            in_quotes = true;
            pos += esc_len;
        } else if let Some(sep_len) = matches_separator(line, pos, separators) {
            fields.push(extract_field_owned_general(line, field_start, pos, escape));
            pos += sep_len;
            field_start = pos;
        } else {
            pos += 1;
        }
    }

    // Last field
    fields.push(extract_field_owned_general(line, field_start, pos, escape));

    fields
}

/// Parse CSV in parallel with multi-byte separator/escape support
pub fn parse_csv_parallel_general(
    input: &[u8],
    separators: &[Vec<u8>],
    escape: &[u8],
) -> Vec<Vec<Vec<u8>>> {
    use super::parallel::run_parallel;
    use rayon::prelude::*;

    let row_starts = find_row_starts_general(input, escape);

    if row_starts.is_empty() {
        return Vec::new();
    }

    let &last_start = match row_starts.last() {
        Some(v) => v,
        None => return Vec::new(),
    };
    let row_ranges: Vec<(usize, usize)> = row_starts
        .windows(2)
        .map(|w| (w[0], w[1]))
        .chain(std::iter::once((last_start, input.len())))
        .collect();

    let separators_vec: Vec<Vec<u8>> = separators.to_vec();
    let escape_vec: Vec<u8> = escape.to_vec();

    run_parallel(|| {
        row_ranges
            .into_par_iter()
            .filter_map(|(start, end)| {
                // Strip trailing line ending (\n or \r\n). Bare \r is data per RFC 4180.
                let mut line_end = end;
                if line_end > start && input[line_end - 1] == b'\n' {
                    line_end -= 1;
                    if line_end > start && input[line_end - 1] == b'\r' {
                        line_end -= 1;
                    }
                }

                if line_end <= start {
                    return None;
                }

                let line = &input[start..line_end];
                let fields = parse_line_fields_owned_general(line, &separators_vec, &escape_vec);

                if fields.is_empty() || (fields.len() == 1 && fields[0].is_empty()) {
                    None
                } else {
                    Some(fields)
                }
            })
            .collect()
    })
}

// ============================================================================
// Strategy F: General zero-copy boundaries
// ============================================================================

/// Parse CSV returning field boundaries with multi-byte separator/escape
pub fn parse_csv_boundaries_general(
    input: &[u8],
    separators: &[Vec<u8>],
    escape: &[u8],
) -> Vec<Vec<(usize, usize)>> {
    let mut rows = Vec::with_capacity(input.len() / 50 + 1);
    let mut pos = 0;

    while pos < input.len() {
        let (boundaries, next_pos) = parse_row_boundaries_general(input, pos, separators, escape);
        if !boundaries.is_empty() {
            rows.push(boundaries);
        }
        pos = next_pos;
    }

    rows
}

fn parse_row_boundaries_general(
    input: &[u8],
    start: usize,
    separators: &[Vec<u8>],
    escape: &[u8],
) -> (Vec<(usize, usize)>, usize) {
    let mut boundaries = Vec::with_capacity(8);
    let mut pos = start;
    let mut field_start = start;
    let mut in_quotes = false;
    let esc_len = escape.len();

    while pos < input.len() {
        if in_quotes {
            if starts_with_escape(input, pos, escape) {
                if starts_with_escape(input, pos + esc_len, escape) {
                    pos += 2 * esc_len;
                    continue;
                }
                in_quotes = false;
                pos += esc_len;
            } else {
                pos += 1;
            }
        } else if starts_with_escape(input, pos, escape) {
            in_quotes = true;
            pos += esc_len;
        } else if let Some(sep_len) = matches_separator(input, pos, separators) {
            boundaries.push((field_start, pos));
            pos += sep_len;
            field_start = pos;
        } else if input[pos] == b'\n' {
            let field_end = if pos > field_start && input[pos - 1] == b'\r' {
                pos - 1
            } else {
                pos
            };
            boundaries.push((field_start, field_end));
            return (boundaries, pos + 1);
        } else if input[pos] == b'\r' && pos + 1 < input.len() && input[pos + 1] == b'\n' {
            // CRLF: end of row. Bare \r is data per RFC 4180.
            boundaries.push((field_start, pos));
            return (boundaries, pos + 2);
        } else {
            pos += 1;
        }
    }

    // End of input
    if field_start < input.len() || !boundaries.is_empty() {
        boundaries.push((field_start, input.len()));
    }

    (boundaries, input.len())
}

// ============================================================================
// Strategy D: General streaming parser
// ============================================================================

/// Streaming parser that handles multi-byte separators and escapes
#[must_use]
pub struct GeneralStreamingParser {
    buffer: Vec<u8>,
    complete_rows: Vec<Vec<Vec<u8>>>,
    partial_row_start: usize,
    scan_pos: usize,
    in_quotes: bool,
    separators: Vec<Vec<u8>>,
    escape: Vec<u8>,
    max_buffer_size: usize,
}

impl GeneralStreamingParser {
    pub fn new(separators: Vec<Vec<u8>>, escape: Vec<u8>) -> Self {
        use super::streaming::DEFAULT_MAX_BUFFER;
        GeneralStreamingParser {
            buffer: Vec::new(),
            complete_rows: Vec::new(),
            partial_row_start: 0,
            scan_pos: 0,
            in_quotes: false,
            separators,
            escape,
            max_buffer_size: DEFAULT_MAX_BUFFER,
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Result<(), super::streaming::BufferOverflow> {
        if self.buffer.len() + chunk.len() > self.max_buffer_size {
            return Err(super::streaming::BufferOverflow);
        }
        self.buffer.extend_from_slice(chunk);
        self.process_buffer();
        Ok(())
    }

    pub fn set_max_buffer_size(&mut self, max: usize) {
        self.max_buffer_size = max;
    }

    fn process_buffer(&mut self) {
        let mut pos = self.scan_pos;
        let esc_len = self.escape.len();

        while pos < self.buffer.len() {
            if self.in_quotes {
                if starts_with_escape(&self.buffer, pos, &self.escape) {
                    if starts_with_escape(&self.buffer, pos + esc_len, &self.escape) {
                        pos += 2 * esc_len;
                        continue;
                    }
                    self.in_quotes = false;
                    pos += esc_len;
                } else {
                    pos += 1;
                }
            } else if starts_with_escape(&self.buffer, pos, &self.escape) {
                self.in_quotes = true;
                pos += esc_len;
            } else if self.buffer[pos] == b'\n' {
                let row_end = pos;
                let row = self.parse_row_owned(self.partial_row_start, row_end);
                if !row.is_empty() {
                    self.complete_rows.push(row);
                }
                pos += 1;
                self.partial_row_start = pos;
                self.in_quotes = false;
            } else if self.buffer[pos] == b'\r' {
                // Only treat \r as line ending when followed by \n (CRLF).
                // Bare \r is data per RFC 4180 and NimbleCSV behavior.
                if pos + 1 < self.buffer.len() {
                    if self.buffer[pos + 1] == b'\n' {
                        let row_end = pos;
                        let row = self.parse_row_owned(self.partial_row_start, row_end);
                        if !row.is_empty() {
                            self.complete_rows.push(row);
                        }
                        pos += 2;
                        self.partial_row_start = pos;
                        self.in_quotes = false;
                    } else {
                        pos += 1;
                    }
                } else {
                    break;
                }
            } else {
                pos += 1;
            }
        }

        self.scan_pos = pos;

        if self.partial_row_start > 0 && self.partial_row_start >= self.buffer.len() / 2 {
            self.compact_buffer();
        }
    }

    fn parse_row_owned(&self, start: usize, end: usize) -> Vec<Vec<u8>> {
        if start >= end {
            return Vec::new();
        }

        let line = &self.buffer[start..end];
        parse_line_fields_owned_general(line, &self.separators, &self.escape)
    }

    fn compact_buffer(&mut self) {
        if self.partial_row_start > 0 {
            self.buffer.drain(0..self.partial_row_start);
            self.scan_pos -= self.partial_row_start;
            self.partial_row_start = 0;
            shrink_excess(&mut self.buffer);
        }
    }

    pub fn take_rows(&mut self, max: usize) -> Vec<Vec<Vec<u8>>> {
        let take_count = max.min(self.complete_rows.len());
        let rows: Vec<_> = self.complete_rows.drain(0..take_count).collect();
        shrink_excess(&mut self.complete_rows);
        rows
    }

    #[must_use]
    pub fn available_rows(&self) -> usize {
        self.complete_rows.len()
    }

    #[must_use]
    pub fn has_partial(&self) -> bool {
        self.partial_row_start < self.buffer.len()
    }

    #[must_use]
    pub fn buffer_size(&self) -> usize {
        self.buffer.len()
    }

    pub fn finalize(&mut self) -> Vec<Vec<Vec<u8>>> {
        if self.partial_row_start < self.buffer.len() {
            let row = self.parse_row_owned(self.partial_row_start, self.buffer.len());
            if !row.is_empty() {
                self.complete_rows.push(row);
            }
        }
        // Release the buffer — parsing is done
        self.buffer = Vec::new();
        self.partial_row_start = 0;
        self.scan_pos = 0;
        std::mem::take(&mut self.complete_rows)
    }
}

// ============================================================================
// Custom Newline Variants
// ============================================================================
// These functions are only called when newlines are custom (non-default).
// They replace hardcoded \n/\r\n checks with match_newline() calls.

/// General parser with custom newlines.
pub fn parse_csv_general_with_newlines<'a>(
    input: &'a [u8],
    separators: &[Vec<u8>],
    escape: &[u8],
    newlines: &Newlines,
) -> Vec<Vec<Cow<'a, [u8]>>> {
    let mut rows = Vec::new();
    let mut pos = 0;

    while pos < input.len() {
        let (row, next_pos) =
            parse_row_general_with_newlines(input, pos, separators, escape, newlines);
        rows.push(row);
        pos = next_pos;
    }

    rows
}

fn parse_row_general_with_newlines<'a>(
    input: &'a [u8],
    start: usize,
    separators: &[Vec<u8>],
    escape: &[u8],
    newlines: &Newlines,
) -> (Vec<Cow<'a, [u8]>>, usize) {
    let mut fields = Vec::new();
    let mut pos = start;
    let mut field_start = start;
    let mut in_quotes = false;
    let esc_len = escape.len();

    while pos < input.len() {
        if in_quotes {
            if starts_with_escape(input, pos, escape) {
                if starts_with_escape(input, pos + esc_len, escape) {
                    pos += 2 * esc_len;
                    continue;
                }
                in_quotes = false;
                pos += esc_len;
            } else {
                pos += 1;
            }
        } else if starts_with_escape(input, pos, escape) {
            in_quotes = true;
            pos += esc_len;
        } else if let Some(sep_len) = matches_separator(input, pos, separators) {
            fields.push(extract_field_cow_general(input, field_start, pos, escape));
            pos += sep_len;
            field_start = pos;
        } else {
            let nl_len = match_newline(input, pos, newlines);
            if nl_len > 0 {
                fields.push(extract_field_cow_general(input, field_start, pos, escape));
                pos += nl_len;
                return (fields, pos);
            }
            pos += 1;
        }
    }

    // Handle last field (no trailing newline)
    if field_start <= input.len() {
        fields.push(extract_field_cow_general(input, field_start, pos, escape));
    }

    (fields, pos)
}

/// Two-phase indexed parser with custom newlines.
pub fn parse_csv_indexed_general_with_newlines<'a>(
    input: &'a [u8],
    separators: &[Vec<u8>],
    escape: &[u8],
    newlines: &Newlines,
) -> Vec<Vec<Cow<'a, [u8]>>> {
    let index = build_index_general_with_newlines(input, separators, escape, newlines);
    index
        .iter()
        .map(|row_fields| {
            row_fields
                .iter()
                .map(|bound| extract_field_cow_general(input, bound.start, bound.end, escape))
                .collect()
        })
        .collect()
}

fn build_index_general_with_newlines(
    input: &[u8],
    separators: &[Vec<u8>],
    escape: &[u8],
    newlines: &Newlines,
) -> Vec<Vec<GeneralFieldBound>> {
    let mut all_fields = Vec::new();
    let mut pos = 0;

    while pos < input.len() {
        let (fields, next_pos) =
            index_row_general_with_newlines(input, pos, separators, escape, newlines);
        if !fields.is_empty() {
            all_fields.push(fields);
        }
        pos = next_pos;
    }

    all_fields
}

fn index_row_general_with_newlines(
    input: &[u8],
    start: usize,
    separators: &[Vec<u8>],
    escape: &[u8],
    newlines: &Newlines,
) -> (Vec<GeneralFieldBound>, usize) {
    let mut fields = Vec::new();
    let mut pos = start;
    let mut field_start = start;
    let mut in_quotes = false;
    let esc_len = escape.len();

    while pos < input.len() {
        if in_quotes {
            if starts_with_escape(input, pos, escape) {
                if starts_with_escape(input, pos + esc_len, escape) {
                    pos += 2 * esc_len;
                    continue;
                }
                in_quotes = false;
                pos += esc_len;
            } else {
                pos += 1;
            }
        } else if starts_with_escape(input, pos, escape) {
            in_quotes = true;
            pos += esc_len;
        } else if let Some(sep_len) = matches_separator(input, pos, separators) {
            fields.push(GeneralFieldBound {
                start: field_start,
                end: pos,
            });
            pos += sep_len;
            field_start = pos;
        } else {
            let nl_len = match_newline(input, pos, newlines);
            if nl_len > 0 {
                fields.push(GeneralFieldBound {
                    start: field_start,
                    end: pos,
                });
                pos += nl_len;
                return (fields, pos);
            }
            pos += 1;
        }
    }

    // End of input
    if field_start < input.len() || !fields.is_empty() {
        fields.push(GeneralFieldBound {
            start: field_start,
            end: input.len(),
        });
    }

    (fields, pos)
}

/// Find row start positions with custom newlines.
pub fn find_row_starts_general_with_newlines(
    input: &[u8],
    escape: &[u8],
    newlines: &Newlines,
) -> Vec<usize> {
    find_row_starts_general_inner(input, escape, |input, pos| {
        match_newline(input, pos, newlines)
    })
}

/// Parallel parser with custom newlines.
pub fn parse_csv_parallel_general_with_newlines(
    input: &[u8],
    separators: &[Vec<u8>],
    escape: &[u8],
    newlines: &Newlines,
) -> Vec<Vec<Vec<u8>>> {
    use super::parallel::run_parallel;
    use rayon::prelude::*;

    let row_starts = find_row_starts_general_with_newlines(input, escape, newlines);

    if row_starts.is_empty() {
        return Vec::new();
    }

    let &last_start = match row_starts.last() {
        Some(v) => v,
        None => return Vec::new(),
    };
    let row_ranges: Vec<(usize, usize)> = row_starts
        .windows(2)
        .map(|w| (w[0], w[1]))
        .chain(std::iter::once((last_start, input.len())))
        .collect();

    let separators_vec: Vec<Vec<u8>> = separators.to_vec();
    let escape_vec: Vec<u8> = escape.to_vec();
    let newlines_clone = newlines.clone();

    run_parallel(|| {
        row_ranges
            .into_par_iter()
            .filter_map(|(start, end)| {
                // Strip trailing newline pattern
                let mut line_end = end;
                for pattern in newlines_clone.patterns.iter() {
                    if line_end >= start + pattern.len()
                        && &input[line_end - pattern.len()..line_end] == pattern.as_slice()
                    {
                        line_end -= pattern.len();
                        break;
                    }
                }

                if line_end <= start {
                    return None;
                }

                let line = &input[start..line_end];
                let fields = parse_line_fields_owned_general(line, &separators_vec, &escape_vec);

                if fields.is_empty() || (fields.len() == 1 && fields[0].is_empty()) {
                    None
                } else {
                    Some(fields)
                }
            })
            .collect()
    })
}

// ============================================================================
// Parallel Boundary Extraction for multi-byte separator/escape
// ============================================================================

/// Parse a single line into field boundaries (start, end) without extracting field data.
/// Like `parse_line_fields_owned_general` but returns positions instead of owned bytes.
fn parse_line_boundaries_general(
    line: &[u8],
    separators: &[Vec<u8>],
    escape: &[u8],
) -> Vec<(usize, usize)> {
    let mut boundaries = Vec::with_capacity(8);
    let mut pos = 0;
    let mut field_start = 0;
    let mut in_quotes = false;
    let esc_len = escape.len();

    while pos < line.len() {
        if in_quotes {
            if starts_with_escape(line, pos, escape) {
                if starts_with_escape(line, pos + esc_len, escape) {
                    pos += 2 * esc_len;
                    continue;
                }
                in_quotes = false;
                pos += esc_len;
            } else {
                pos += 1;
            }
        } else if starts_with_escape(line, pos, escape) {
            in_quotes = true;
            pos += esc_len;
        } else if let Some(sep_len) = matches_separator(line, pos, separators) {
            boundaries.push((field_start, pos));
            pos += sep_len;
            field_start = pos;
        } else {
            pos += 1;
        }
    }

    // Last field
    boundaries.push((field_start, pos));

    boundaries
}

/// Parse CSV in parallel with multi-byte separator/escape, returning boundaries
pub fn parse_csv_parallel_boundaries_general(
    input: &[u8],
    separators: &[Vec<u8>],
    escape: &[u8],
) -> Vec<Vec<(usize, usize)>> {
    use super::parallel::run_parallel;
    use rayon::prelude::*;

    let row_starts = find_row_starts_general(input, escape);

    if row_starts.is_empty() {
        return Vec::new();
    }

    let &last_start = match row_starts.last() {
        Some(v) => v,
        None => return Vec::new(),
    };
    let row_ranges: Vec<(usize, usize)> = row_starts
        .windows(2)
        .map(|w| (w[0], w[1]))
        .chain(std::iter::once((last_start, input.len())))
        .collect();

    let separators_vec: Vec<Vec<u8>> = separators.to_vec();
    let escape_vec: Vec<u8> = escape.to_vec();

    run_parallel(|| {
        row_ranges
            .into_par_iter()
            .filter_map(|(start, end)| {
                // Strip trailing line ending (\n or \r\n). Bare \r is data per RFC 4180.
                let mut line_end = end;
                if line_end > start && input[line_end - 1] == b'\n' {
                    line_end -= 1;
                    if line_end > start && input[line_end - 1] == b'\r' {
                        line_end -= 1;
                    }
                }

                if line_end <= start {
                    return None;
                }

                let line = &input[start..line_end];
                let line_boundaries =
                    parse_line_boundaries_general(line, &separators_vec, &escape_vec);

                if line_boundaries.is_empty()
                    || (line_boundaries.len() == 1 && line_boundaries[0].0 >= line_boundaries[0].1)
                {
                    return None;
                }

                // Adjust offsets from line-relative to input-relative
                let adjusted: Vec<(usize, usize)> = line_boundaries
                    .into_iter()
                    .map(|(s, e)| (s + start, e + start))
                    .collect();

                Some(adjusted)
            })
            .collect()
    })
}

/// Parallel boundary parser with custom newlines.
pub fn parse_csv_parallel_boundaries_general_with_newlines(
    input: &[u8],
    separators: &[Vec<u8>],
    escape: &[u8],
    newlines: &Newlines,
) -> Vec<Vec<(usize, usize)>> {
    use super::parallel::run_parallel;
    use rayon::prelude::*;

    let row_starts = find_row_starts_general_with_newlines(input, escape, newlines);

    if row_starts.is_empty() {
        return Vec::new();
    }

    let &last_start = match row_starts.last() {
        Some(v) => v,
        None => return Vec::new(),
    };
    let row_ranges: Vec<(usize, usize)> = row_starts
        .windows(2)
        .map(|w| (w[0], w[1]))
        .chain(std::iter::once((last_start, input.len())))
        .collect();

    let separators_vec: Vec<Vec<u8>> = separators.to_vec();
    let escape_vec: Vec<u8> = escape.to_vec();
    let newlines_clone = newlines.clone();

    run_parallel(|| {
        row_ranges
            .into_par_iter()
            .filter_map(|(start, end)| {
                // Strip trailing newline pattern
                let mut line_end = end;
                for pattern in newlines_clone.patterns.iter() {
                    if line_end >= start + pattern.len()
                        && &input[line_end - pattern.len()..line_end] == pattern.as_slice()
                    {
                        line_end -= pattern.len();
                        break;
                    }
                }

                if line_end <= start {
                    return None;
                }

                let line = &input[start..line_end];
                let line_boundaries =
                    parse_line_boundaries_general(line, &separators_vec, &escape_vec);

                if line_boundaries.is_empty()
                    || (line_boundaries.len() == 1 && line_boundaries[0].0 >= line_boundaries[0].1)
                {
                    return None;
                }

                // Adjust offsets from line-relative to input-relative
                let adjusted: Vec<(usize, usize)> = line_boundaries
                    .into_iter()
                    .map(|(s, e)| (s + start, e + start))
                    .collect();

                Some(adjusted)
            })
            .collect()
    })
}

/// Zero-copy boundaries with custom newlines.
pub fn parse_csv_boundaries_general_with_newlines(
    input: &[u8],
    separators: &[Vec<u8>],
    escape: &[u8],
    newlines: &Newlines,
) -> Vec<Vec<(usize, usize)>> {
    let mut rows = Vec::with_capacity(input.len() / 50 + 1);
    let mut pos = 0;

    while pos < input.len() {
        let (boundaries, next_pos) =
            parse_row_boundaries_general_with_newlines(input, pos, separators, escape, newlines);
        if !boundaries.is_empty() {
            rows.push(boundaries);
        }
        pos = next_pos;
    }

    rows
}

fn parse_row_boundaries_general_with_newlines(
    input: &[u8],
    start: usize,
    separators: &[Vec<u8>],
    escape: &[u8],
    newlines: &Newlines,
) -> (Vec<(usize, usize)>, usize) {
    let mut boundaries = Vec::with_capacity(8);
    let mut pos = start;
    let mut field_start = start;
    let mut in_quotes = false;
    let esc_len = escape.len();

    while pos < input.len() {
        if in_quotes {
            if starts_with_escape(input, pos, escape) {
                if starts_with_escape(input, pos + esc_len, escape) {
                    pos += 2 * esc_len;
                    continue;
                }
                in_quotes = false;
                pos += esc_len;
            } else {
                pos += 1;
            }
        } else if starts_with_escape(input, pos, escape) {
            in_quotes = true;
            pos += esc_len;
        } else if let Some(sep_len) = matches_separator(input, pos, separators) {
            boundaries.push((field_start, pos));
            pos += sep_len;
            field_start = pos;
        } else {
            let nl_len = match_newline(input, pos, newlines);
            if nl_len > 0 {
                boundaries.push((field_start, pos));
                pos += nl_len;
                return (boundaries, pos);
            }
            pos += 1;
        }
    }

    // End of input
    if field_start < input.len() || !boundaries.is_empty() {
        boundaries.push((field_start, input.len()));
    }

    (boundaries, input.len())
}

/// Streaming parser with custom newline support.
#[must_use]
pub struct GeneralStreamingParserNewlines {
    buffer: Vec<u8>,
    complete_rows: Vec<Vec<Vec<u8>>>,
    partial_row_start: usize,
    scan_pos: usize,
    in_quotes: bool,
    separators: Vec<Vec<u8>>,
    escape: Vec<u8>,
    newlines: Newlines,
    max_buffer_size: usize,
}

impl GeneralStreamingParserNewlines {
    pub fn new(separators: Vec<Vec<u8>>, escape: Vec<u8>, newlines: Newlines) -> Self {
        use super::streaming::DEFAULT_MAX_BUFFER;
        GeneralStreamingParserNewlines {
            buffer: Vec::new(),
            complete_rows: Vec::new(),
            partial_row_start: 0,
            scan_pos: 0,
            in_quotes: false,
            separators,
            escape,
            newlines,
            max_buffer_size: DEFAULT_MAX_BUFFER,
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Result<(), super::streaming::BufferOverflow> {
        if self.buffer.len() + chunk.len() > self.max_buffer_size {
            return Err(super::streaming::BufferOverflow);
        }
        self.buffer.extend_from_slice(chunk);
        self.process_buffer();
        Ok(())
    }

    pub fn set_max_buffer_size(&mut self, max: usize) {
        self.max_buffer_size = max;
    }

    fn process_buffer(&mut self) {
        let mut pos = self.scan_pos;
        let esc_len = self.escape.len();
        let max_nl_len = self.newlines.max_pattern_len();

        while pos < self.buffer.len() {
            if self.in_quotes {
                if starts_with_escape(&self.buffer, pos, &self.escape) {
                    if starts_with_escape(&self.buffer, pos + esc_len, &self.escape) {
                        pos += 2 * esc_len;
                        continue;
                    }
                    self.in_quotes = false;
                    pos += esc_len;
                } else {
                    pos += 1;
                }
            } else if starts_with_escape(&self.buffer, pos, &self.escape) {
                self.in_quotes = true;
                pos += esc_len;
            } else {
                // Chunk-boundary safety: if we can't fully check the longest newline
                // pattern, break and wait for more data.
                if pos + max_nl_len > self.buffer.len() {
                    // Check shorter patterns that do fit
                    let nl_len = match_newline(&self.buffer, pos, &self.newlines);
                    if nl_len > 0 {
                        let row_end = pos;
                        let row = self.parse_row_owned(self.partial_row_start, row_end);
                        if !row.is_empty() {
                            self.complete_rows.push(row);
                        }
                        pos += nl_len;
                        self.partial_row_start = pos;
                        self.in_quotes = false;
                    } else {
                        break;
                    }
                } else {
                    let nl_len = match_newline(&self.buffer, pos, &self.newlines);
                    if nl_len > 0 {
                        let row_end = pos;
                        let row = self.parse_row_owned(self.partial_row_start, row_end);
                        if !row.is_empty() {
                            self.complete_rows.push(row);
                        }
                        pos += nl_len;
                        self.partial_row_start = pos;
                        self.in_quotes = false;
                    } else {
                        pos += 1;
                    }
                }
            }
        }

        self.scan_pos = pos;

        if self.partial_row_start > 0 && self.partial_row_start >= self.buffer.len() / 2 {
            self.compact_buffer();
        }
    }

    fn parse_row_owned(&self, start: usize, end: usize) -> Vec<Vec<u8>> {
        if start >= end {
            return Vec::new();
        }

        let line = &self.buffer[start..end];
        parse_line_fields_owned_general(line, &self.separators, &self.escape)
    }

    fn compact_buffer(&mut self) {
        if self.partial_row_start > 0 {
            self.buffer.drain(0..self.partial_row_start);
            self.scan_pos -= self.partial_row_start;
            self.partial_row_start = 0;
            shrink_excess(&mut self.buffer);
        }
    }

    pub fn take_rows(&mut self, max: usize) -> Vec<Vec<Vec<u8>>> {
        let take_count = max.min(self.complete_rows.len());
        let rows: Vec<_> = self.complete_rows.drain(0..take_count).collect();
        shrink_excess(&mut self.complete_rows);
        rows
    }

    #[must_use]
    pub fn available_rows(&self) -> usize {
        self.complete_rows.len()
    }

    #[must_use]
    pub fn has_partial(&self) -> bool {
        self.partial_row_start < self.buffer.len()
    }

    #[must_use]
    pub fn buffer_size(&self) -> usize {
        self.buffer.len()
    }

    pub fn finalize(&mut self) -> Vec<Vec<Vec<u8>>> {
        if self.partial_row_start < self.buffer.len() {
            let row = self.parse_row_owned(self.partial_row_start, self.buffer.len());
            if !row.is_empty() {
                self.complete_rows.push(row);
            }
        }
        // Release the buffer — parsing is done
        self.buffer = Vec::new();
        self.partial_row_start = 0;
        self.scan_pos = 0;
        std::mem::take(&mut self.complete_rows)
    }
}

// ============================================================================
// Tests
// ============================================================================

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

    // Common cross-strategy scenarios moved to tests/conformance.rs.
    // Only unique general-specific tests remain here.

    #[test]
    fn test_general_mixed_separators() {
        let input = b"a,b::c\n1::2,3\n";
        let seps = vec![b",".to_vec(), b"::".to_vec()];
        let esc = b"\"".to_vec();
        let result = to_strings(parse_csv_general(input, &seps, &esc));
        assert_eq!(result, vec![vec!["a", "b", "c"], vec!["1", "2", "3"]]);
    }

    #[test]
    fn test_general_quoted_with_separator() {
        let input = b"\"a::b\"::c\n";
        let seps = vec![b"::".to_vec()];
        let esc = b"\"".to_vec();
        let result = to_strings(parse_csv_general(input, &seps, &esc));
        assert_eq!(result, vec![vec!["a::b", "c"]]);
    }

    // --- Multi-byte escape tests ---

    #[test]
    fn test_general_multi_byte_escape() {
        let input = b"$$hello$$::world\n";
        let seps = vec![b"::".to_vec()];
        let esc = b"$$".to_vec();
        let result = to_strings(parse_csv_general(input, &seps, &esc));
        assert_eq!(result, vec![vec!["hello", "world"]]);
    }

    #[test]
    fn test_general_multi_byte_escape_doubled() {
        // $$val$$$$ue$$ → val$$ue
        let input = b"$$val$$$$ue$$::other\n";
        let seps = vec![b"::".to_vec()];
        let esc = b"$$".to_vec();
        let result = to_strings(parse_csv_general(input, &seps, &esc));
        assert_eq!(result, vec![vec!["val$$ue", "other"]]);
    }

    #[test]
    fn test_streaming_general_chunked() {
        let seps = vec![b"::".to_vec()];
        let esc = b"\"".to_vec();
        let mut parser = GeneralStreamingParser::new(seps, esc);
        parser.feed(b"a::b::").unwrap();
        assert_eq!(parser.available_rows(), 0);
        parser.feed(b"c\n1::2::3\n").unwrap();
        assert_eq!(parser.available_rows(), 2);

        let rows = parser.take_rows(10);
        assert_eq!(rows[0], vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    }

    #[test]
    fn test_streaming_general_finalize() {
        let seps = vec![b"::".to_vec()];
        let esc = b"\"".to_vec();
        let mut parser = GeneralStreamingParser::new(seps, esc);
        parser.feed(b"a::b\n1::2").unwrap();
        let rows1 = parser.take_rows(10);
        assert_eq!(rows1.len(), 1);

        let rows2 = parser.finalize();
        assert_eq!(rows2.len(), 1);
        assert_eq!(rows2[0], vec![b"1".to_vec(), b"2".to_vec()]);
    }

    // --- Custom newline tests ---

    #[test]
    fn test_custom_newline_multi_byte() {
        let nl = Newlines::custom(vec![b"<br>".to_vec()]);
        let input = b"a,b<br>1,2<br>";
        let seps = vec![b",".to_vec()];
        let esc = b"\"".to_vec();
        let result = to_strings(parse_csv_general_with_newlines(input, &seps, &esc, &nl));
        assert_eq!(result, vec![vec!["a", "b"], vec!["1", "2"]]);
    }

    #[test]
    fn test_custom_newline_no_trailing() {
        let nl = Newlines::custom(vec![b"|".to_vec()]);
        let input = b"a,b|1,2";
        let seps = vec![b",".to_vec()];
        let esc = b"\"".to_vec();
        let result = to_strings(parse_csv_general_with_newlines(input, &seps, &esc, &nl));
        assert_eq!(result, vec![vec!["a", "b"], vec!["1", "2"]]);
    }

    #[test]
    fn test_custom_newline_quoted_field() {
        let nl = Newlines::custom(vec![b"|".to_vec()]);
        let input = b"\"a|b\",c|1,2|";
        let seps = vec![b",".to_vec()];
        let esc = b"\"".to_vec();
        let result = to_strings(parse_csv_general_with_newlines(input, &seps, &esc, &nl));
        assert_eq!(result, vec![vec!["a|b", "c"], vec!["1", "2"]]);
    }

    #[test]
    fn test_custom_newline_streaming_chunked() {
        let nl = Newlines::custom(vec![b"|".to_vec()]);
        let seps = vec![b",".to_vec()];
        let esc = b"\"".to_vec();
        let mut parser = GeneralStreamingParserNewlines::new(seps, esc, nl);
        parser.feed(b"a,b|1,").unwrap();
        assert_eq!(parser.available_rows(), 1);
        parser.feed(b"2|3,4|").unwrap();
        assert_eq!(parser.available_rows(), 3);
        let rows = parser.take_rows(10);
        assert_eq!(rows[0], vec![b"a".to_vec(), b"b".to_vec()]);
        assert_eq!(rows[1], vec![b"1".to_vec(), b"2".to_vec()]);
        assert_eq!(rows[2], vec![b"3".to_vec(), b"4".to_vec()]);
    }

    #[test]
    fn test_custom_newline_streaming_multi_byte() {
        let nl = Newlines::custom(vec![b"<br>".to_vec()]);
        let seps = vec![b",".to_vec()];
        let esc = b"\"".to_vec();
        let mut parser = GeneralStreamingParserNewlines::new(seps, esc, nl);
        parser.feed(b"a,b<br>1,2<br>").unwrap();
        let rows = parser.take_rows(10);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec![b"a".to_vec(), b"b".to_vec()]);
        assert_eq!(rows[1], vec![b"1".to_vec(), b"2".to_vec()]);
    }
}
