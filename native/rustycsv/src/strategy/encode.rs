//! CSV encoding helpers — field scanning and quoting for the encoding NIF
//!
//! The encoding NIF (encode_string in lib.rs) walks Erlang lists, scans each
//! field for characters requiring quoting, and writes all output into a single
//! flat Vec<u8> buffer that becomes one NewBinary. These helpers handle the
//! scanning ("does this field need quoting?") and quoting ("wrap + double
//! escapes").
//!
//! Scanning strategies:
//!   SIMD:    portable_simd 16-byte vectorized comparison (fastest)
//!   General: byte-by-byte for multi-byte separator/escape patterns

use std::simd::prelude::*;

use crate::core::simd_scanner::CHUNK;

// ==========================================================================
// Quoting: wrap field in escape chars, double internal escapes
// ==========================================================================

/// Write a field that needs quoting: escape_char + field_with_doubled_escapes + escape_char
#[inline]
pub fn write_quoted_field(out: &mut Vec<u8>, field: &[u8], escape: u8) {
    out.push(escape);
    let mut i = 0;
    while i < field.len() {
        let b = field[i];
        out.push(b);
        if b == escape {
            out.push(escape); // double the escape character
        }
        i += 1;
    }
    out.push(escape);
}

/// Write a field that needs quoting with multi-byte escape sequence
#[inline]
pub fn write_quoted_field_general(out: &mut Vec<u8>, field: &[u8], escape: &[u8]) {
    out.extend_from_slice(escape);
    let esc_len = escape.len();
    let mut i = 0;
    while i < field.len() {
        if i + esc_len <= field.len() && field[i..i + esc_len] == *escape {
            out.extend_from_slice(escape);
            out.extend_from_slice(escape); // doubled
            i += esc_len;
        } else {
            out.push(field[i]);
            i += 1;
        }
    }
    out.extend_from_slice(escape);
}

/// Write field content with doubled escapes, WITHOUT surrounding escape bytes.
/// Used when the caller needs to insert a formula prefix between the opening
/// escape and the field content.
#[inline]
pub fn write_quoted_field_inner(out: &mut Vec<u8>, field: &[u8], escape: u8) {
    let mut i = 0;
    while i < field.len() {
        let b = field[i];
        out.push(b);
        if b == escape {
            out.push(escape);
        }
        i += 1;
    }
}

/// Write field content with doubled multi-byte escapes, WITHOUT surrounding escape bytes.
#[inline]
pub fn write_quoted_field_inner_general(out: &mut Vec<u8>, field: &[u8], escape: &[u8]) {
    let esc_len = escape.len();
    let mut i = 0;
    while i < field.len() {
        if i + esc_len <= field.len() && field[i..i + esc_len] == *escape {
            out.extend_from_slice(escape);
            out.extend_from_slice(escape); // doubled
            i += esc_len;
        } else {
            out.push(field[i]);
            i += 1;
        }
    }
}

// ==========================================================================
// Scanning: reserved-pattern matching
// ==========================================================================

/// Reserved patterns that trigger CSV field quoting.
pub struct ReservedPatterns {
    single_bytes: Vec<u8>,
    multi_bytes: Vec<Vec<u8>>,
}

impl ReservedPatterns {
    pub fn new(patterns: Vec<Vec<u8>>) -> Self {
        let (single, multi): (Vec<_>, Vec<_>) =
            patterns.into_iter().partition(|pattern| pattern.len() == 1);
        let single_bytes: Vec<u8> = single.into_iter().map(|pattern| pattern[0]).collect();
        let multi_bytes = multi
            .into_iter()
            .filter(|pattern| !pattern.iter().any(|byte| single_bytes.contains(byte)))
            .collect();

        Self {
            single_bytes,
            multi_bytes,
        }
    }
}

/// Check whether a field contains any configured reserved pattern.
#[inline]
pub fn field_needs_quoting(field: &[u8], reserved: &ReservedPatterns) -> bool {
    let len = field.len();
    let mut pos = 0;

    // 16-byte path
    if let Some((&first, rest)) = reserved.single_bytes.split_first() {
        while pos + CHUNK <= len {
            let chunk = Simd::<u8, CHUNK>::from_slice(&field[pos..pos + CHUNK]);
            let mut hits = chunk.simd_eq(Simd::splat(first));
            for &byte in rest {
                hits |= chunk.simd_eq(Simd::splat(byte));
            }
            if hits.any() {
                return true;
            }
            pos += CHUNK;
        }
    }

    // Scalar tail
    while pos < len {
        if reserved.single_bytes.contains(&field[pos]) {
            return true;
        }
        pos += 1;
    }

    reserved.multi_bytes.iter().any(|pattern| {
        field.len() >= pattern.len() && field.windows(pattern.len()).any(|window| window == pattern)
    })
}

// ==========================================================================
// Tests
// ==========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_quoted_field() {
        let mut out = Vec::new();
        write_quoted_field(&mut out, b"hello", b'"');
        assert_eq!(out, b"\"hello\"");

        let mut out = Vec::new();
        write_quoted_field(&mut out, b"say \"hi\"", b'"');
        assert_eq!(out, b"\"say \"\"hi\"\"\"");
    }

    #[test]
    fn test_write_quoted_field_general() {
        let mut out = Vec::new();
        write_quoted_field_general(&mut out, b"hello", b"$$");
        assert_eq!(out, b"$$hello$$");

        let mut out = Vec::new();
        write_quoted_field_general(&mut out, b"a$$b", b"$$");
        assert_eq!(out, b"$$a$$$$b$$");
    }

    #[test]
    fn configured_reserved_patterns_trigger_quoting() {
        let reserved = ReservedPatterns::new(vec![b",".to_vec(), b"\"".to_vec(), b"::".to_vec()]);

        assert!(field_needs_quoting(b"a,b", &reserved));
        assert!(field_needs_quoting(b"say \"hello\"", &reserved));
        assert!(field_needs_quoting(b"a::b", &reserved));
        assert!(!field_needs_quoting(b"abc", &reserved));
        assert!(!field_needs_quoting(b"", &reserved));

        let wide = b"abcdefghijklmno,qrstuvwxyz";
        assert!(field_needs_quoting(wide, &reserved));
    }

    #[test]
    fn single_byte_patterns_subsume_multi_byte_patterns() {
        let reserved =
            ReservedPatterns::new(vec![b"\r\n".to_vec(), b"\n".to_vec(), b"::".to_vec()]);

        assert_eq!(reserved.single_bytes, vec![b'\n']);
        assert_eq!(reserved.multi_bytes, vec![b"::".to_vec()]);
    }
}
