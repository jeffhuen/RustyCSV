// Structural index for SIMD-scanned CSV
//
// Produced by simd_scanner, consumed by all strategies.
// Positions use u32 to halve memory vs usize on 64-bit systems.
//
// **Limit**: inputs must not exceed u32::MAX (4 GiB). The NIF boundary
// enforces this via `guard_input_size`; if you call `scan_structural`
// from Rust code directly, you must validate the input length yourself.

/// A newline terminator position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct RowEnd {
    /// Byte position of terminator start (\n or \r in \r\n).
    pub pos: u32,
    /// 1 for \n, 2 for \r\n.
    pub len: u8,
}

/// Where a quoting rule was broken, and which rule it was.
///
/// The scanner parses leniently and records violations rather than stopping,
/// so a caller that does not ask for strict behaviour pays nothing and sees
/// the same rows it always did. `RustyCSV.ParseError` is raised at the Elixir
/// boundary from this value; the scanner itself never panics.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Violation {
    /// A quote opened a field somewhere other than the start of a field.
    /// Byte position of the offending quote.
    UnexpectedQuote(u32),
    /// A closing quote was followed by something other than a separator, a
    /// row terminator, another quote, or end of input. Byte position of the
    /// offending byte, not of the quote.
    TrailingGarbage(u32),
    /// Input ended while still inside a quoted field.
    UnterminatedQuote,
}

/// Structural index: positions of all unquoted separators and row endings.
#[derive(Debug)]
#[must_use]
pub struct StructuralIndex {
    /// Positions of unquoted field separators (commas, tabs, etc.).
    pub field_seps: Vec<u32>,
    /// Positions of unquoted row terminators.
    pub row_ends: Vec<RowEnd>,
    /// Total input length.
    pub input_len: u32,
    /// First quoting rule broken, in byte order, or `None` for well-formed
    /// input. Populated on every scan; acting on it is the caller's choice.
    pub violation: Option<Violation>,
}

impl StructuralIndex {
    /// Iterate over rows as (row_start, row_content_end, next_row_start) triples.
    ///
    /// `row_content_end` excludes the line terminator.
    /// `next_row_start` is past the terminator.
    #[inline]
    pub fn rows(&self) -> RowIter<'_> {
        RowIter {
            index: self,
            row_idx: 0,
            pos: 0,
        }
    }

    /// Return field (start, end) pairs for a row bounded by `row_start..row_content_end`.
    ///
    /// Uses binary search into `field_seps` to find separators in the range.
    #[inline]
    pub fn fields_in_row(&self, row_start: u32, row_content_end: u32) -> FieldIter<'_> {
        // Find first separator >= row_start
        let lo = self.field_seps.partition_point(|&s| s < row_start);
        // Find first separator >= row_content_end (all seps before this are in the row)
        let hi = self.field_seps.partition_point(|&s| s < row_content_end);

        FieldIter {
            seps: &self.field_seps[lo..hi],
            row_start,
            row_content_end,
            idx: 0,
            done: false,
        }
    }

    /// Number of rows.
    #[inline]
    #[must_use]
    pub fn row_count(&self) -> usize {
        let n = self.row_ends.len();
        // If there's content after the last row_end (no trailing newline), there's one more row.
        if n == 0 {
            if self.input_len > 0 {
                1
            } else {
                0
            }
        } else {
            let last = &self.row_ends[n - 1];
            let after_last = last.pos as usize + last.len as usize;
            if after_last < self.input_len as usize {
                n + 1
            } else {
                n
            }
        }
    }

    /// Iterate over rows with their field separators, using a linear cursor.
    ///
    /// More efficient than `rows()` + `fields_in_row()` for sequential access:
    /// O(total_seps) across all rows vs O(rows * log(total_seps)) with binary search.
    #[inline]
    pub fn rows_with_fields(&self) -> RowFieldIter<'_> {
        RowFieldIter {
            index: self,
            row_idx: 0,
            pos: 0,
            sep_cursor: 0,
        }
    }

    /// Convenience: get all row start positions (for parallel strategy).
    #[allow(dead_code)]
    pub fn row_starts(&self) -> Vec<usize> {
        let mut starts = Vec::with_capacity(self.row_count());
        for (row_start, _, _) in self.rows() {
            starts.push(row_start as usize);
        }
        starts
    }
}

/// Iterator over rows in a `StructuralIndex`.
pub struct RowIter<'a> {
    index: &'a StructuralIndex,
    row_idx: usize,
    pos: u32,
}

impl<'a> Iterator for RowIter<'a> {
    /// (row_start, row_content_end, next_row_start)
    type Item = (u32, u32, u32);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.row_idx < self.index.row_ends.len() {
            let re = &self.index.row_ends[self.row_idx];
            let start = self.pos;
            let content_end = re.pos;
            let next = re.pos + re.len as u32;
            self.pos = next;
            self.row_idx += 1;
            Some((start, content_end, next))
        } else {
            // Possible trailing row without terminator
            if self.pos < self.index.input_len {
                let start = self.pos;
                let end = self.index.input_len;
                self.pos = end;
                self.row_idx += 1;
                Some((start, end, end))
            } else {
                None
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.index.row_count().saturating_sub(self.row_idx);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for RowIter<'_> {}

/// A single row from the cursor-based iterator, with its field bounds.
pub struct Row<'a> {
    pub start: u32,
    pub content_end: u32,
    pub fields: FieldIter<'a>,
}

/// Iterator over rows with their field separator slices (cursor-based).
///
/// Uses a running cursor through `field_seps` instead of binary search.
pub struct RowFieldIter<'a> {
    index: &'a StructuralIndex,
    row_idx: usize,
    pos: u32,
    sep_cursor: usize,
}

impl<'a> Iterator for RowFieldIter<'a> {
    type Item = Row<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (start, content_end) = if self.row_idx < self.index.row_ends.len() {
            let re = &self.index.row_ends[self.row_idx];
            let start = self.pos;
            let content_end = re.pos;
            self.pos = re.pos + re.len as u32;
            self.row_idx += 1;
            (start, content_end)
        } else if self.pos < self.index.input_len {
            let start = self.pos;
            let end = self.index.input_len;
            self.pos = end;
            self.row_idx += 1;
            (start, end)
        } else {
            return None;
        };

        // Advance cursor past separators in this row
        let sep_start = self.sep_cursor;
        while self.sep_cursor < self.index.field_seps.len()
            && self.index.field_seps[self.sep_cursor] < content_end
        {
            self.sep_cursor += 1;
        }

        Some(Row {
            start,
            content_end,
            fields: FieldIter {
                seps: &self.index.field_seps[sep_start..self.sep_cursor],
                row_start: start,
                row_content_end: content_end,
                idx: 0,
                done: false,
            },
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.index.row_count().saturating_sub(self.row_idx);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for RowFieldIter<'_> {}

/// Iterator over fields in a single row.
pub struct FieldIter<'a> {
    seps: &'a [u32],
    row_start: u32,
    row_content_end: u32,
    idx: usize,
    done: bool,
}

impl<'a> Iterator for FieldIter<'a> {
    /// (field_start, field_end) — byte positions in the input.
    type Item = (u32, u32);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        let field_start = if self.idx == 0 {
            self.row_start
        } else {
            // Previous separator + 1
            self.seps[self.idx - 1] + 1
        };

        if self.idx < self.seps.len() {
            let field_end = self.seps[self.idx];
            self.idx += 1;
            Some((field_start, field_end))
        } else {
            // Last field: ends at row_content_end
            self.done = true;
            Some((field_start, self.row_content_end))
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.done {
            return (0, Some(0));
        }
        let remaining = (self.seps.len() + 1).saturating_sub(self.idx);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for FieldIter<'_> {}

#[cfg(test)]
mod tests {
    use super::*;

    // Common scenarios moved to tests/conformance.rs.
    // Only unique StructuralIndex-specific tests remain here.

    fn make_index(seps: Vec<u32>, ends: Vec<RowEnd>, len: u32) -> StructuralIndex {
        StructuralIndex {
            field_seps: seps,
            row_ends: ends,
            input_len: len,
            violation: None,
        }
    }

    #[test]
    fn test_single_field_no_sep() {
        // "abc\n" = 4 bytes
        let idx = make_index(vec![], vec![RowEnd { pos: 3, len: 1 }], 4);
        assert_eq!(idx.row_count(), 1);

        let fields: Vec<_> = idx.fields_in_row(0, 3).collect();
        assert_eq!(fields, vec![(0, 3)]);
    }

    #[test]
    fn test_row_starts() {
        // "a\nb\nc\n" = 6 bytes
        let idx = make_index(
            vec![],
            vec![
                RowEnd { pos: 1, len: 1 },
                RowEnd { pos: 3, len: 1 },
                RowEnd { pos: 5, len: 1 },
            ],
            6,
        );
        assert_eq!(idx.row_starts(), vec![0, 2, 4]);
    }

    #[test]
    fn test_row_count_trailing_content() {
        // "a,b" (no trailing newline) — 3 bytes, no row_ends
        let idx = make_index(vec![1], vec![], 3);
        assert_eq!(
            idx.row_count(),
            1,
            "content after last row_end counts as a row"
        );

        // Empty input
        let idx = make_index(vec![], vec![], 0);
        assert_eq!(idx.row_count(), 0);

        // "a\nb" — trailing row without newline
        let idx = make_index(vec![], vec![RowEnd { pos: 1, len: 1 }], 3);
        assert_eq!(idx.row_count(), 2);

        // "a\n" — no trailing content
        let idx = make_index(vec![], vec![RowEnd { pos: 1, len: 1 }], 2);
        assert_eq!(idx.row_count(), 1);
    }

    /// Collect all field bounds per row via the cursor-based iterator.
    fn collect_fields_cursor(idx: &StructuralIndex) -> Vec<Vec<(u32, u32)>> {
        idx.rows_with_fields()
            .map(|row| row.fields.collect())
            .collect()
    }

    /// Collect all field bounds per row via the binary-search iterator.
    fn collect_fields_bsearch(idx: &StructuralIndex) -> Vec<Vec<(u32, u32)>> {
        idx.rows()
            .map(|(rs, re, _next)| idx.fields_in_row(rs, re).collect())
            .collect()
    }

    #[test]
    fn test_rows_with_fields_matches_binary_search() {
        // "a,b,c\nd,e\nf\n" = 12 bytes
        // seps: 1,3 (row 0), 7 (row 1), none (row 2)
        // row_ends: 5(len 1), 9(len 1), 11(len 1)
        let idx = make_index(
            vec![1, 3, 7],
            vec![
                RowEnd { pos: 5, len: 1 },
                RowEnd { pos: 9, len: 1 },
                RowEnd { pos: 11, len: 1 },
            ],
            12,
        );

        let cursor = collect_fields_cursor(&idx);
        let bsearch = collect_fields_bsearch(&idx);

        assert_eq!(
            cursor, bsearch,
            "cursor-based and binary-search iterators must produce identical results"
        );

        assert_eq!(cursor.len(), 3);
        assert_eq!(cursor[0], vec![(0, 1), (2, 3), (4, 5)]); // a,b,c
        assert_eq!(cursor[1], vec![(6, 7), (8, 9)]); // d,e
        assert_eq!(cursor[2], vec![(10, 11)]); // f
    }

    #[test]
    fn test_rows_with_fields_trailing_row_no_newline() {
        // "a,b\nc" — trailing row without newline
        let idx = make_index(vec![1], vec![RowEnd { pos: 3, len: 1 }], 5);

        let cursor = collect_fields_cursor(&idx);
        let bsearch = collect_fields_bsearch(&idx);

        assert_eq!(cursor, bsearch);
        assert_eq!(cursor.len(), 2);
        assert_eq!(cursor[0], vec![(0, 1), (2, 3)]); // a,b
        assert_eq!(cursor[1], vec![(4, 5)]); // c
    }

    // --- ExactSizeIterator correctness tests ---

    #[test]
    fn row_iter_exact_size_with_trailing_row() {
        // "a\nb" — 2 rows, second has no trailing newline
        let idx = make_index(vec![], vec![RowEnd { pos: 1, len: 1 }], 3);
        let mut iter = idx.rows();

        assert_eq!(iter.len(), 2);
        let _ = iter.next(); // consume row 1
        assert_eq!(iter.len(), 1);
        let _ = iter.next(); // consume trailing row
        assert_eq!(iter.len(), 0);
        assert!(iter.next().is_none());
    }

    #[test]
    fn row_iter_exact_size_no_trailing_row() {
        // "a\n" — 1 row with trailing newline
        let idx = make_index(vec![], vec![RowEnd { pos: 1, len: 1 }], 2);
        let mut iter = idx.rows();

        assert_eq!(iter.len(), 1);
        let _ = iter.next();
        assert_eq!(iter.len(), 0);
        assert!(iter.next().is_none());
    }

    #[test]
    fn row_field_iter_exact_size_with_trailing_row() {
        // "a\nb" — 2 rows, second has no trailing newline
        let idx = make_index(vec![], vec![RowEnd { pos: 1, len: 1 }], 3);
        let mut iter = idx.rows_with_fields();

        assert_eq!(iter.len(), 2);
        let _ = iter.next();
        assert_eq!(iter.len(), 1);
        let _ = iter.next(); // trailing row
        assert_eq!(iter.len(), 0);
        assert!(iter.next().is_none());
    }

    #[test]
    fn field_iter_exact_size_single_field() {
        // Row "abc" — 1 field, no separators
        let idx = make_index(vec![], vec![RowEnd { pos: 3, len: 1 }], 4);
        let mut fields = idx.fields_in_row(0, 3);

        assert_eq!(fields.len(), 1);
        let _ = fields.next(); // consume the only field
        assert_eq!(fields.len(), 0);
        assert!(fields.next().is_none());
    }

    #[test]
    fn field_iter_exact_size_multiple_fields() {
        // Row "a,b,c" — 3 fields, seps at 1 and 3
        let idx = make_index(vec![1, 3], vec![RowEnd { pos: 5, len: 1 }], 6);
        let mut fields = idx.fields_in_row(0, 5);

        assert_eq!(fields.len(), 3);
        let _ = fields.next(); // field "a"
        assert_eq!(fields.len(), 2);
        let _ = fields.next(); // field "b"
        assert_eq!(fields.len(), 1);
        let _ = fields.next(); // field "c" (last, sets done=true)
        assert_eq!(fields.len(), 0);
        assert!(fields.next().is_none());
    }
}
