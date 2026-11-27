//! Custom newline support for CSV parsing.

/// A set of newline patterns to recognise when splitting rows.
///
/// When `is_default` is true, the standard \r\n and \n handling is used
/// (SIMD-optimized paths). When false, the general byte-by-byte parser
/// checks against the custom patterns.
#[derive(Clone, Debug)]
#[must_use]
pub struct Newlines {
    /// Newline patterns sorted longest-first for greedy matching.
    pub patterns: Vec<Vec<u8>>,
    /// True when patterns are the default ["\r\n", "\n"].
    pub is_default: bool,
}

impl Newlines {
    /// Default newlines: \r\n and \n (handled by existing optimized code paths).
    pub fn default_newlines() -> Self {
        Newlines {
            patterns: vec![b"\r\n".to_vec(), b"\n".to_vec()],
            is_default: true,
        }
    }

    /// Custom newline patterns. Sorts longest-first for greedy matching.
    pub fn custom(mut patterns: Vec<Vec<u8>>) -> Self {
        // Sort longest-first so greedy matching works correctly
        patterns.sort_by_key(|b| std::cmp::Reverse(b.len()));
        Newlines {
            patterns,
            is_default: false,
        }
    }

    /// Maximum pattern length (used for chunk-boundary safety in streaming).
    #[must_use]
    pub fn max_pattern_len(&self) -> usize {
        self.patterns.iter().map(|p| p.len()).max().unwrap_or(1)
    }
}

/// 1-based line and column position corresponding to a byte offset.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[must_use]
pub struct LineCol {
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number (Unicode scalar count from the start of the line).
    pub column: usize,
}

/// Count default newlines (`\r\n` and `\n`) in `input`.
///
/// Both `\r\n` and `\n` end with `\n`, while bare `\r` is field data.
#[inline]
#[must_use]
pub fn count_default_newlines(input: &[u8]) -> usize {
    input.iter().filter(|&&b| b == b'\n').count()
}

/// Count newline occurrences in `input` according to `newlines`.
#[must_use]
pub fn count_newlines(input: &[u8], newlines: &Newlines) -> usize {
    if newlines.is_default {
        return count_default_newlines(input);
    }

    let mut count = 0;
    let mut pos = 0;
    while pos < input.len() {
        let nl_len = match_newline(input, pos, newlines);
        if nl_len > 0 {
            count += 1;
            pos += nl_len;
        } else {
            pos += 1;
        }
    }
    count
}

/// Compute the 1-based `(line, column)` of `byte_offset` in `input` for default newlines.
///
/// `byte_offset` is clamped to `input.len()`. Columns count Unicode scalar
/// values from the start of the line without allocating or panicking on
/// invalid UTF-8 bytes.
pub fn line_col_at_default(input: &[u8], byte_offset: usize) -> LineCol {
    let prefix = &input[..byte_offset.min(input.len())];
    let mut lines_before = 0;
    let mut last_line_start = 0;
    for (i, &b) in prefix.iter().enumerate() {
        if b == b'\n' {
            lines_before += 1;
            last_line_start = i + 1;
        }
    }
    line_col_from_prefix(lines_before, &prefix[last_line_start..])
}

/// Compute the 1-based `(line, column)` of `byte_offset` in `input` according to `newlines`.
///
/// `byte_offset` is clamped to `input.len()`. Columns count Unicode scalar
/// values from the start of the line without allocating or panicking on
/// invalid UTF-8 bytes.
pub fn line_col_at(input: &[u8], byte_offset: usize, newlines: &Newlines) -> LineCol {
    if newlines.is_default {
        return line_col_at_default(input, byte_offset);
    }

    let prefix = &input[..byte_offset.min(input.len())];
    let mut lines_before = 0;
    let mut last_line_start = 0;
    let mut pos = 0;
    while pos < prefix.len() {
        let nl_len = match_newline(prefix, pos, newlines);
        if nl_len > 0 {
            lines_before += 1;
            pos += nl_len;
            last_line_start = pos;
        } else {
            pos += 1;
        }
    }
    line_col_from_prefix(lines_before, &prefix[last_line_start..])
}

fn line_col_from_prefix(lines_before: usize, line_prefix: &[u8]) -> LineCol {
    let column_offset: usize = line_prefix
        .utf8_chunks()
        .map(|chunk| chunk.valid().chars().count() + usize::from(!chunk.invalid().is_empty()))
        .sum();
    LineCol {
        line: lines_before + 1,
        column: column_offset + 1,
    }
}

/// Returns length of matched newline at `pos`, or 0 if no match.
#[inline]
pub fn match_newline(input: &[u8], pos: usize, newlines: &Newlines) -> usize {
    for pattern in &newlines.patterns {
        if input[pos..].starts_with(pattern) {
            return pattern.len();
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_newlines_match_real_input() {
        let nl = Newlines::default_newlines();
        assert!(nl.is_default);
        // Must match both \r\n and \n in actual input
        assert_eq!(match_newline(b"abc\r\ndef", 3, &nl), 2);
        assert_eq!(match_newline(b"abc\ndef", 3, &nl), 1);
        // Must NOT match bare \r
        assert_eq!(match_newline(b"abc\rdef", 3, &nl), 0);
        // \r\n must be ordered before \n so greedy match returns 2, not 1
        assert_eq!(nl.patterns[0], b"\r\n".to_vec());
    }

    #[test]
    fn test_max_pattern_len_used_for_streaming_safety() {
        // Streaming uses max_pattern_len to know when it can't fully check
        // for a newline near the chunk boundary. A wrong value causes silent
        // data corruption. Verify it tracks the actual longest pattern.
        let nl = Newlines::custom(vec![b"|".to_vec(), b"<br>".to_vec()]);
        assert_eq!(nl.max_pattern_len(), 4);

        // Single pattern
        let nl2 = Newlines::custom(vec![b"||".to_vec()]);
        assert_eq!(nl2.max_pattern_len(), 2);

        // Default newlines: longest is \r\n (2 bytes)
        let nl3 = Newlines::default_newlines();
        assert_eq!(nl3.max_pattern_len(), 2);

        // Empty patterns list (should not happen in practice, but must not panic)
        let nl4 = Newlines {
            patterns: vec![],
            is_default: false,
        };
        assert_eq!(nl4.max_pattern_len(), 1); // unwrap_or(1) fallback
    }

    #[test]
    fn test_custom_newlines() {
        let nl = Newlines::custom(vec![b"|".to_vec(), b"<br>".to_vec()]);
        assert!(!nl.is_default);
        // Should be sorted longest-first
        assert_eq!(nl.patterns[0], b"<br>".to_vec());
        assert_eq!(nl.patterns[1], b"|".to_vec());
    }

    #[test]
    fn test_match_newline_pipe() {
        let nl = Newlines::custom(vec![b"|".to_vec()]);
        let input = b"hello|world";
        assert_eq!(match_newline(input, 5, &nl), 1);
        assert_eq!(match_newline(input, 0, &nl), 0);
    }

    #[test]
    fn test_match_newline_multi_byte() {
        let nl = Newlines::custom(vec![b"<br>".to_vec()]);
        let input = b"hello<br>world";
        assert_eq!(match_newline(input, 5, &nl), 4);
        assert_eq!(match_newline(input, 0, &nl), 0);
    }

    #[test]
    fn test_match_newline_greedy() {
        // With patterns "|" and "||", "||" should match first at a "||" position
        let nl = Newlines::custom(vec![b"|".to_vec(), b"||".to_vec()]);
        let input = b"a||b";
        assert_eq!(match_newline(input, 1, &nl), 2); // "||" matches (longest first)
    }

    #[test]
    fn line_col_at_default_tracks_lf_crlf_bare_cr_and_utf8() {
        assert_eq!(line_col_at_default(b"", 0), LineCol { line: 1, column: 1 });
        assert_eq!(
            line_col_at_default(b"a,b\nx\"y\n", 5),
            LineCol { line: 2, column: 2 }
        );
        assert_eq!(
            line_col_at_default(b"a,b\r\nx\"y\r\n", 6),
            LineCol { line: 2, column: 2 }
        );
        // Bare CR is field data, not a line break
        assert_eq!(
            line_col_at_default(b"a\r\"b\"\n", 2),
            LineCol { line: 1, column: 3 }
        );
        // UTF-8 multibyte chars count as one column each
        let utf8 = "名前,x\"y\n".as_bytes();
        assert_eq!(line_col_at_default(utf8, 8), LineCol { line: 1, column: 5 });
        // Invalid UTF-8 does not panic and counts deterministically
        let invalid = &[0xff, b'x', b'"', b'y'];
        assert_eq!(
            line_col_at_default(invalid, 2),
            LineCol { line: 1, column: 3 }
        );
        // EOF after trailing newline is next line, column 1; clamps out-of-bounds offset
        assert_eq!(
            line_col_at_default(b"a,b\n\"x,2\n", 9),
            LineCol { line: 3, column: 1 }
        );
        assert_eq!(
            line_col_at_default(b"a,b\n\"x,2", 99),
            LineCol { line: 2, column: 5 }
        );
    }

    #[test]
    fn line_col_at_custom_newlines_counts_matched_patterns() {
        let br = Newlines::custom(vec![b"<br>".to_vec()]);
        assert_eq!(
            line_col_at(b"a,b<br>x\"y<br>", 8, &br),
            LineCol { line: 2, column: 2 }
        );

        let pipes = Newlines::custom(vec![b"|".to_vec(), b"||".to_vec()]);
        assert_eq!(count_newlines(b"a||b|c", &pipes), 2);
        assert_eq!(
            line_col_at(b"a||b|x\"y", 6, &pipes),
            LineCol { line: 3, column: 2 }
        );
    }
}
