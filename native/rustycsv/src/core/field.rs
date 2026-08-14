//! Field extraction and quote handling

use std::borrow::Cow;

/// Unescape doubled escape chars in a field's inner content: "" -> "
pub fn unescape_field(inner: &[u8], escape: u8) -> Vec<u8> {
    let mut result = Vec::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        if inner[i] == escape && i + 1 < inner.len() && inner[i + 1] == escape {
            result.push(escape);
            i += 2;
        } else {
            result.push(inner[i]);
            i += 1;
        }
    }
    result
}

/// Extract a field from input, stripping surrounding quotes and unescaping doubled quotes.
/// Returns Cow::Borrowed when no unescaping needed, Cow::Owned when we had to allocate.
#[inline]
pub fn extract_field_cow(input: &[u8], start: usize, end: usize) -> Cow<'_, [u8]> {
    extract_field_cow_with_escape(input, start, end, b'"')
}

/// Extract a field with configurable escape character.
#[inline]
pub fn extract_field_cow_with_escape(
    input: &[u8],
    start: usize,
    end: usize,
    escape: u8,
) -> Cow<'_, [u8]> {
    if start >= end {
        return Cow::Borrowed(&[]);
    }

    let field = &input[start..end];

    // Not quoted - return as-is
    if field.len() < 2 || field[0] != escape || field[field.len() - 1] != escape {
        return Cow::Borrowed(field);
    }

    // Quoted - strip quotes and check for escaped quotes
    let inner = &field[1..field.len() - 1];

    // Fast path: no escaped quotes inside
    if !inner.contains(&escape) {
        return Cow::Borrowed(inner);
    }

    // Slow path: unescape doubled escape chars
    Cow::Owned(unescape_field(inner, escape))
}

/// Extract a field from input, stripping surrounding quotes if present.
/// NOTE: This does NOT unescape doubled quotes. Use extract_field_cow for full compliance.
#[inline]
pub fn extract_field(input: &[u8], start: usize, end: usize) -> &[u8] {
    extract_field_with_escape(input, start, end, b'"')
}

/// Extract a field with configurable escape character (no unescaping).
#[inline]
pub fn extract_field_with_escape(input: &[u8], start: usize, end: usize, escape: u8) -> &[u8] {
    if start >= end {
        return &[];
    }

    let field = &input[start..end];

    // Strip surrounding escape chars if present
    if field.len() >= 2 && field[0] == escape && field[field.len() - 1] == escape {
        &field[1..field.len() - 1]
    } else {
        field
    }
}

/// Extract a field with configurable escape character, returning owned data
pub fn extract_field_owned_with_escape(
    input: &[u8],
    start: usize,
    end: usize,
    escape: u8,
) -> Vec<u8> {
    if start >= end {
        return Vec::new();
    }

    let field = &input[start..end];

    // Not quoted
    if field.len() < 2 || field[0] != escape || field[field.len() - 1] != escape {
        return field.to_vec();
    }

    // Quoted - need to unescape doubled escape chars
    let inner = &field[1..field.len() - 1];

    // Fast path: no escaped chars
    if !inner.contains(&escape) {
        return inner.to_vec();
    }

    // Slow path: unescape doubled escape chars
    unescape_field(inner, escape)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_field_simple() {
        assert_eq!(extract_field(b"hello", 0, 5), b"hello");
    }

    #[test]
    fn test_extract_field_empty_and_degenerate() {
        // All extract variants must handle start >= end without panicking.
        // This matters because the structural index can produce zero-length
        // fields (e.g., "a,,b" has an empty field between the commas).

        // start == end: empty field
        assert_eq!(extract_field(b"abc", 1, 1), b"");
        assert_eq!(extract_field_cow(b"abc", 1, 1).as_ref(), b"");
        assert_eq!(extract_field_owned_with_escape(b"abc", 1, 1, b'"'), b"");

        // start > end: degenerate, must not panic
        assert_eq!(extract_field(b"abc", 2, 1), b"");
        assert_eq!(extract_field_cow(b"abc", 2, 1).as_ref(), b"");
        assert_eq!(extract_field_owned_with_escape(b"abc", 2, 1, b'"'), b"");

        // start == end == 0 on empty input
        assert_eq!(extract_field(b"", 0, 0), b"");
        assert_eq!(extract_field_cow(b"", 0, 0).as_ref(), b"");
        assert_eq!(extract_field_owned_with_escape(b"", 0, 0, b'"'), b"");
    }

    #[test]
    fn test_extract_field_quoted() {
        assert_eq!(extract_field(b"\"hello\"", 0, 7), b"hello");
    }

    #[test]
    fn test_extract_field_cow_escaped() {
        let result = extract_field_cow_with_escape(b"\"hello \"\"world\"\"\"", 0, 17, b'"');
        assert_eq!(result.as_ref(), b"hello \"world\"");
    }

    #[test]
    fn test_extract_field_owned_escaped() {
        let result = extract_field_owned_with_escape(b"\"hello \"\"world\"\"\"", 0, 17, b'"');
        assert_eq!(result, b"hello \"world\"");
    }
}
