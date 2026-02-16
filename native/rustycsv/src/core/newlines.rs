/// Custom newline support for CSV parsing.
///
/// When `is_default` is true, the standard \r\n and \n handling is used
/// (SIMD-optimized paths). When false, the general byte-by-byte parser
/// checks against the custom patterns.

#[derive(Debug, Clone)]
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
}
