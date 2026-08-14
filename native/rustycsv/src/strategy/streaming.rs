//! Approach D: Streaming Parser
//!
//! Stateful chunked parser for processing large files with bounded memory.
//! Feed chunks of data and extract complete rows as they become available.
//!
//! Key design:
//! - Owns data (Vec<u8>) because input chunks are temporary
//! - Buffers incomplete rows until more data arrives
//! - Returns rows in batches to reduce NIF call overhead

use crate::core::{extract_field_owned_with_escape, is_separator, QuoteState, Violation};

/// Default maximum buffer size for streaming parsers (256 MB).
pub const DEFAULT_MAX_BUFFER: usize = 256 * 1024 * 1024;

/// Shrink a Vec's capacity when it greatly exceeds its length.
///
/// `Vec::drain` and `Vec::clear` preserve the original allocation. For
/// long-lived streaming parsers this causes memory to grow monotonically
/// to its peak usage and never return to the OS. This helper reclaims
/// that excess when capacity exceeds 4× length (with a 1 KiB floor,
/// measured in bytes, to avoid thrashing on small buffers).
pub(crate) fn shrink_excess<T>(v: &mut Vec<T>) {
    let len = v.len();
    let floor_elements = 1024 / std::mem::size_of::<T>().max(1);
    if v.capacity() > len.saturating_mul(4).max(floor_elements) {
        v.shrink_to(len.saturating_mul(2));
    }
}

/// Error returned when a streaming `feed()` would exceed the buffer limit.
#[derive(Copy, Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("streaming buffer overflow: feed would exceed maximum buffer size")]
#[must_use]
pub struct BufferOverflow;

/// State for streaming CSV parser
#[must_use]
pub struct StreamingParser {
    /// Buffer holding unprocessed data
    buffer: Vec<u8>,
    /// Complete rows ready to be taken
    complete_rows: Vec<Vec<Vec<u8>>>,
    /// Position where the current (incomplete) row starts
    partial_row_start: usize,
    /// Position where we left off scanning (resume point)
    scan_pos: usize,
    /// Strict quote state carried across input chunks.
    quote: QuoteState,
    /// Field separator characters (supports multiple separators for NimbleCSV compatibility)
    separators: Vec<u8>,
    /// Quote/escape character
    escape: u8,
    /// Maximum buffer size in bytes
    max_buffer_size: usize,
    /// Absolute offset represented by `buffer[0]` after compaction.
    buffer_offset: usize,
    /// First malformed-quoting condition seen in stream order.
    violation: Option<Violation>,
}

impl StreamingParser {
    /// Create a new streaming parser with default settings (comma separator, double-quote escape)
    pub fn new() -> Self {
        Self::with_config(b',', b'"')
    }

    /// Create a new streaming parser with configurable separator and escape
    pub fn with_config(separator: u8, escape: u8) -> Self {
        StreamingParser {
            buffer: Vec::new(),
            complete_rows: Vec::new(),
            partial_row_start: 0,
            scan_pos: 0,
            quote: QuoteState::new(),
            separators: vec![separator],
            escape,
            max_buffer_size: DEFAULT_MAX_BUFFER,
            buffer_offset: 0,
            violation: None,
        }
    }

    /// Create a new streaming parser with multiple separator support
    pub fn with_multi_sep(separators: &[u8], escape: u8) -> Self {
        StreamingParser {
            buffer: Vec::new(),
            complete_rows: Vec::new(),
            partial_row_start: 0,
            scan_pos: 0,
            quote: QuoteState::new(),
            separators: separators.to_vec(),
            escape,
            max_buffer_size: DEFAULT_MAX_BUFFER,
            buffer_offset: 0,
            violation: None,
        }
    }

    /// Feed a chunk of data to the parser.
    /// Returns `Err(BufferOverflow)` if the buffer would exceed `max_buffer_size`.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<(), BufferOverflow> {
        if self.buffer.len() + chunk.len() > self.max_buffer_size {
            return Err(BufferOverflow);
        }
        // Append chunk to buffer
        self.buffer.extend_from_slice(chunk);

        // Process buffer to find complete rows
        self.process_buffer();
        Ok(())
    }

    /// Set the maximum buffer size in bytes.
    pub fn set_max_buffer_size(&mut self, max: usize) {
        self.max_buffer_size = max;
    }

    /// Process the buffer to extract complete rows
    fn process_buffer(&mut self) {
        // Resume from where we left off scanning
        let mut pos = self.scan_pos;
        let escape = self.escape;

        while pos < self.buffer.len() {
            let byte = self.buffer[pos];
            let absolute_pos = self.buffer_offset + pos;

            if self.quote.in_quotes() {
                if byte == escape {
                    if pos + 1 >= self.buffer.len() {
                        break;
                    }
                    let doubled = self.buffer[pos + 1] == escape;
                    self.quote.on_escape(absolute_pos, doubled);
                    pos += if doubled { 2 } else { 1 };
                } else {
                    pos += 1;
                }
            } else if byte == escape {
                self.quote.on_escape(absolute_pos, false);
                pos += 1;
            } else if is_separator(byte, &self.separators) {
                self.quote.on_delimiter();
                pos += 1;
            } else if byte == b'\n' {
                // Found end of row
                self.quote.on_delimiter();
                self.push_complete_row(self.partial_row_start, pos);
                pos += 1;
                self.partial_row_start = pos;
            } else if byte == b'\r' {
                // Only treat \r as line ending when followed by \n (CRLF).
                // Bare \r is data per RFC 4180 and NimbleCSV behavior.
                if pos + 1 < self.buffer.len() {
                    if self.buffer[pos + 1] == b'\n' {
                        // CRLF: end of row
                        self.quote.on_delimiter();
                        self.push_complete_row(self.partial_row_start, pos);
                        pos += 2; // skip \r\n
                        self.partial_row_start = pos;
                    } else {
                        // Bare \r followed by non-\n: treat as data
                        self.quote.on_data(absolute_pos);
                        pos += 1;
                    }
                } else {
                    // \r at end of buffer: can't tell if \n follows.
                    // Stop scanning here; next feed() will resolve it.
                    break;
                }
            } else {
                self.quote.on_data(absolute_pos);
                pos += 1;
            }

            if self.violation.is_none() {
                self.violation = self.quote.violation();
            }
        }

        // Save scan position for resuming later
        self.scan_pos = pos;

        // Compact buffer: remove processed data to prevent unbounded growth
        if self.partial_row_start > 0 && self.partial_row_start >= self.buffer.len() / 2 {
            self.compact_buffer();
        }
    }

    /// Parse a row from buffer range into owned fields
    fn parse_row_owned(&self, start: usize, end: usize) -> (Vec<Vec<u8>>, Option<Violation>) {
        if start >= end {
            return (Vec::new(), None);
        }

        let line = &self.buffer[start..end];
        let mut fields = Vec::new();
        let mut pos = 0;
        let mut field_start = 0;
        let mut quote = QuoteState::new();
        let separators = &self.separators;
        let escape = self.escape;

        while pos < line.len() {
            let byte = line[pos];

            if quote.in_quotes() {
                if byte == escape {
                    let doubled = pos + 1 < line.len() && line[pos + 1] == escape;
                    quote.on_escape(pos, doubled);
                    pos += if doubled { 2 } else { 1 };
                } else {
                    pos += 1;
                }
            } else if byte == escape {
                quote.on_escape(pos, false);
                pos += 1;
            } else if is_separator(byte, separators) {
                fields.push(extract_field_owned_with_escape(
                    line,
                    field_start,
                    pos,
                    escape,
                ));
                pos += 1;
                field_start = pos;
                quote.on_delimiter();
            } else {
                quote.on_data(pos);
                pos += 1;
            }
        }

        // Last field
        fields.push(extract_field_owned_with_escape(
            line,
            field_start,
            pos,
            escape,
        ));

        (fields, quote.finish(line.len()))
    }

    fn push_complete_row(&mut self, start: usize, end: usize) {
        let (row, violation) = self.parse_row_owned(start, end);
        if self.violation.is_none() {
            self.violation = violation.map(|v| v.rebase(self.buffer_offset + start));
        }
        if !row.is_empty() {
            self.complete_rows.push(row);
        }
    }

    /// Compact buffer by removing already-processed data
    fn compact_buffer(&mut self) {
        if self.partial_row_start > 0 {
            self.buffer.drain(0..self.partial_row_start);
            self.buffer_offset += self.partial_row_start;
            // Adjust positions after compaction
            self.scan_pos -= self.partial_row_start;
            self.partial_row_start = 0;
            // Release excess capacity to prevent unbounded memory growth.
            // Vec::drain preserves the original allocation even after removing
            // most data, causing the buffer to hold its peak capacity forever.
            shrink_excess(&mut self.buffer);
        }
    }

    /// Take up to `max` complete rows from the parser
    pub fn take_rows(&mut self, max: usize) -> Vec<Vec<Vec<u8>>> {
        let take_count = max.min(self.complete_rows.len());
        let rows: Vec<_> = self.complete_rows.drain(0..take_count).collect();
        // Prevent complete_rows from retaining peak capacity after draining
        shrink_excess(&mut self.complete_rows);
        rows
    }

    /// Check how many complete rows are available
    #[must_use]
    pub fn available_rows(&self) -> usize {
        self.complete_rows.len()
    }

    /// Check if there's a partial row in the buffer
    #[must_use]
    pub fn has_partial(&self) -> bool {
        self.partial_row_start < self.buffer.len()
    }

    /// Get the size of buffered data (for memory monitoring)
    #[must_use]
    pub fn buffer_size(&self) -> usize {
        self.buffer.len()
    }

    /// First strict quoting violation seen so far.
    #[must_use]
    pub fn violation(&self) -> Option<Violation> {
        self.violation
    }

    /// Finalize parsing - treat any remaining data as the last row
    pub fn finalize(&mut self) -> Vec<Vec<Vec<u8>>> {
        // Process any remaining partial row
        if self.partial_row_start < self.buffer.len() {
            self.push_complete_row(self.partial_row_start, self.buffer.len());
        }
        if self.violation.is_none() {
            self.violation = self.quote.finish(self.buffer_offset + self.buffer.len());
        }

        // Release the buffer — parsing is done, no need to hold this memory
        self.buffer_offset += self.buffer.len();
        self.buffer = Vec::new();
        self.partial_row_start = 0;
        self.buffer_offset = 0;
        self.scan_pos = 0;

        // Take all remaining rows
        std::mem::take(&mut self.complete_rows)
    }

    /// Reset the parser state
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        // Use = Vec::new() instead of .clear() to actually release memory
        self.buffer = Vec::new();
        self.complete_rows = Vec::new();
        self.partial_row_start = 0;
        self.scan_pos = 0;
        self.quote = QuoteState::new();
        self.buffer_offset = 0;
        self.violation = None;
        // separator and escape are preserved
    }

    /// Get the separators
    #[allow(dead_code)]
    pub fn separators(&self) -> &[u8] {
        &self.separators
    }

    /// Get the first separator (for backward compatibility)
    #[allow(dead_code)]
    pub fn separator(&self) -> u8 {
        self.separators.first().copied().unwrap_or(b',')
    }

    /// Get the escape character
    #[allow(dead_code)]
    pub fn escape(&self) -> u8 {
        self.escape
    }
}

impl Default for StreamingParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Common scenarios moved to tests/conformance.rs.
    // Only unique streaming-specific tests remain here.

    #[test]
    fn test_streaming_chunked() {
        let mut parser = StreamingParser::new();
        parser.feed(b"a,b,").unwrap();
        assert_eq!(parser.available_rows(), 0);

        parser.feed(b"c\n1,2,3\n").unwrap();
        assert_eq!(parser.available_rows(), 2);

        let rows = parser.take_rows(10);
        assert_eq!(rows[0], vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    }

    #[test]
    fn test_streaming_quoted_across_chunks() {
        let mut parser = StreamingParser::new();
        parser.feed(b"a,\"hello ").unwrap();
        assert_eq!(parser.available_rows(), 0);

        parser.feed(b"world\",c\n").unwrap();
        assert_eq!(parser.available_rows(), 1);

        let rows = parser.take_rows(10);
        assert_eq!(
            rows[0],
            vec![b"a".to_vec(), b"hello world".to_vec(), b"c".to_vec()]
        );
    }

    #[test]
    fn test_streaming_finalize() {
        let mut parser = StreamingParser::new();
        parser.feed(b"a,b,c\n1,2,3").unwrap();

        let rows1 = parser.take_rows(10);
        assert_eq!(rows1.len(), 1);

        let rows2 = parser.finalize();
        assert_eq!(rows2.len(), 1);
        assert_eq!(rows2[0], vec![b"1".to_vec(), b"2".to_vec(), b"3".to_vec()]);
    }

    #[test]
    fn test_take_rows_partial() {
        let mut parser = StreamingParser::new();
        parser.feed(b"a\nb\nc\nd\n").unwrap();

        let rows1 = parser.take_rows(2);
        assert_eq!(rows1.len(), 2);

        let rows2 = parser.take_rows(10);
        assert_eq!(rows2.len(), 2);
    }

    #[test]
    fn test_streaming_bare_cr_is_data() {
        let mut parser = StreamingParser::new();
        parser.feed(b"a\rb\n").unwrap();

        let rows = parser.take_rows(10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], vec![b"a\rb".to_vec()]);
    }

    #[test]
    fn test_streaming_bare_cr_at_chunk_boundary() {
        let mut parser = StreamingParser::new();
        parser.feed(b"a,b\r").unwrap();
        assert_eq!(parser.available_rows(), 0);

        parser.feed(b"\nc,d\n").unwrap();
        assert_eq!(parser.available_rows(), 2);

        let rows = parser.take_rows(10);
        assert_eq!(rows[0], vec![b"a".to_vec(), b"b".to_vec()]);
        assert_eq!(rows[1], vec![b"c".to_vec(), b"d".to_vec()]);
    }

    #[test]
    fn test_streaming_bare_cr_at_chunk_boundary_not_crlf() {
        let mut parser = StreamingParser::new();
        parser.feed(b"a\r").unwrap();
        assert_eq!(parser.available_rows(), 0);

        parser.feed(b"b\n").unwrap();
        assert_eq!(parser.available_rows(), 1);

        let rows = parser.take_rows(10);
        assert_eq!(rows[0], vec![b"a\rb".to_vec()]);
    }

    #[test]
    fn violations_survive_chunk_boundaries_and_compaction() {
        let mut parser = StreamingParser::new();
        parser.feed(b"a,b\nc,d\n").unwrap();
        assert_eq!(parser.buffer_size(), 0);

        parser.feed(b"x").unwrap();
        parser.feed(b"\"y,2\n").unwrap();
        assert_eq!(parser.violation(), Some(Violation::UnexpectedQuote(9)));
    }

    #[test]
    fn unterminated_quote_is_reported_only_at_finalize() {
        let mut parser = StreamingParser::new();
        parser.feed(b"a,b\n").unwrap();
        parser.feed(b"\"x").unwrap();
        assert_eq!(parser.violation(), None);

        parser.finalize();
        assert_eq!(parser.violation(), Some(Violation::UnterminatedQuote(6)));
    }

    // --- Memory management tests ---

    #[test]
    fn shrink_excess_reclaims_when_capacity_exceeds_threshold() {
        let mut v: Vec<u8> = Vec::with_capacity(8192);
        v.push(1);
        // capacity=8192, len=1 → 8192 > 1*4.max(1024) → should shrink
        shrink_excess(&mut v);
        assert!(
            v.capacity() < 8192,
            "capacity should have been reduced from 8192, got {}",
            v.capacity()
        );
        assert_eq!(v.len(), 1);
        assert_eq!(v[0], 1);
    }

    #[test]
    fn shrink_excess_does_not_shrink_below_floor() {
        let mut v: Vec<u8> = Vec::with_capacity(1024);
        // capacity=1024, len=0 → 1024 <= 0*4.max(1024) → should NOT shrink
        shrink_excess(&mut v);
        assert_eq!(
            v.capacity(),
            1024,
            "capacity at the floor should not be shrunk"
        );
    }

    #[test]
    fn shrink_excess_preserves_when_ratio_acceptable() {
        let mut v: Vec<u8> = Vec::with_capacity(4000);
        v.extend_from_slice(&[0u8; 2000]);
        // capacity=4000, len=2000 → 4000 <= 2000*4.max(1024) → should NOT shrink
        let cap_before = v.capacity();
        shrink_excess(&mut v);
        assert_eq!(v.capacity(), cap_before);
    }

    #[test]
    fn shrink_excess_byte_floor_for_large_elements() {
        // For Vec<Vec<Vec<u8>>>, size_of::<T>() = 24 bytes on 64-bit
        // floor_elements = 1024 / 24 = 42
        let mut v: Vec<Vec<Vec<u8>>> = Vec::with_capacity(200);
        // capacity=200, len=0 → 200 > 0*4.max(42) → should shrink
        shrink_excess(&mut v);
        assert!(
            v.capacity() < 200,
            "should shrink large-element Vec past byte-based floor"
        );
    }

    #[test]
    fn finalize_releases_buffer() {
        let mut parser = StreamingParser::new();
        parser.feed(b"a,b,c\n1,2,3").unwrap();
        assert!(parser.buffer_size() > 0);

        let rows = parser.finalize();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            parser.buffer_size(),
            0,
            "buffer should be released after finalize"
        );
    }

    #[test]
    fn reset_releases_memory() {
        let mut parser = StreamingParser::new();
        parser.feed(b"a,b,c\n1,2,3\n").unwrap();
        let _ = parser.take_rows(10);

        parser.reset();
        assert_eq!(parser.buffer_size(), 0);
        assert_eq!(parser.available_rows(), 0);
        assert!(!parser.has_partial());
    }
}
