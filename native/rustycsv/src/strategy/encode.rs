//! CSV encoding helpers — field scanning and quoting for the encoding NIF
//!
//! The encoding NIF (encode_string in lib.rs) walks Erlang lists, scans each
//! field for characters requiring quoting, and writes all output into a single
//! flat Vec<u8> buffer that becomes one NewBinary. These helpers handle the
//! scanning ("does this field need quoting?") and quoting ("wrap + double
//! escapes").
//!
//! Scanning strategies:
//!   SIMD:    portable_simd 16/32-byte vectorized comparison (fastest)
//!   General: byte-by-byte for multi-byte separator/escape patterns

use std::simd::prelude::*;

use crate::core::simd_scanner::CHUNK;
#[cfg(target_feature = "avx2")]
use crate::core::simd_scanner::WIDE;

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
// Scanning: SIMD — portable_simd vectorized field scanning
// ==========================================================================

/// SIMD: check if field needs quoting using vectorized comparison.
#[inline]
pub fn field_needs_quoting_simd(field: &[u8], separator: u8, escape: u8, reserved: &[u8]) -> bool {
    let len = field.len();
    let mut pos = 0;

    // AVX2 wide path: 32 bytes at a time
    #[cfg(target_feature = "avx2")]
    {
        let sep_splat = Simd::<u8, WIDE>::splat(separator);
        let esc_splat = Simd::<u8, WIDE>::splat(escape);
        let lf_splat = Simd::<u8, WIDE>::splat(b'\n');
        let cr_splat = Simd::<u8, WIDE>::splat(b'\r');
        let res_splats: Vec<Simd<u8, WIDE>> = reserved
            .iter()
            .map(|&r| Simd::<u8, WIDE>::splat(r))
            .collect();

        while pos + WIDE <= len {
            let chunk = Simd::<u8, WIDE>::from_slice(&field[pos..pos + WIDE]);
            let mut hits = chunk.simd_eq(sep_splat)
                | chunk.simd_eq(esc_splat)
                | chunk.simd_eq(lf_splat)
                | chunk.simd_eq(cr_splat);
            for splat in &res_splats {
                hits |= chunk.simd_eq(*splat);
            }
            if hits.any() {
                return true;
            }
            pos += WIDE;
        }
    }

    // 16-byte path
    {
        let sep_splat = Simd::<u8, CHUNK>::splat(separator);
        let esc_splat = Simd::<u8, CHUNK>::splat(escape);
        let lf_splat = Simd::<u8, CHUNK>::splat(b'\n');
        let cr_splat = Simd::<u8, CHUNK>::splat(b'\r');
        let res_splats: Vec<Simd<u8, CHUNK>> = reserved
            .iter()
            .map(|&r| Simd::<u8, CHUNK>::splat(r))
            .collect();

        while pos + CHUNK <= len {
            let chunk = Simd::<u8, CHUNK>::from_slice(&field[pos..pos + CHUNK]);
            let mut hits = chunk.simd_eq(sep_splat)
                | chunk.simd_eq(esc_splat)
                | chunk.simd_eq(lf_splat)
                | chunk.simd_eq(cr_splat);
            for splat in &res_splats {
                hits |= chunk.simd_eq(*splat);
            }
            if hits.any() {
                return true;
            }
            pos += CHUNK;
        }
    }

    // Scalar tail
    while pos < len {
        let b = field[pos];
        if b == separator || b == escape || b == b'\n' || b == b'\r' || reserved.contains(&b) {
            return true;
        }
        pos += 1;
    }

    false
}

/// SIMD variant with multiple separators
#[inline]
pub fn field_needs_quoting_simd_multi_sep(
    field: &[u8],
    separators: &[u8],
    escape: u8,
    reserved: &[u8],
) -> bool {
    let len = field.len();
    let mut pos = 0;

    // AVX2 wide path
    #[cfg(target_feature = "avx2")]
    {
        let esc_splat = Simd::<u8, WIDE>::splat(escape);
        let lf_splat = Simd::<u8, WIDE>::splat(b'\n');
        let cr_splat = Simd::<u8, WIDE>::splat(b'\r');
        let sep_splats: Vec<Simd<u8, WIDE>> = separators
            .iter()
            .map(|&s| Simd::<u8, WIDE>::splat(s))
            .collect();
        let res_splats: Vec<Simd<u8, WIDE>> = reserved
            .iter()
            .map(|&r| Simd::<u8, WIDE>::splat(r))
            .collect();

        while pos + WIDE <= len {
            let chunk = Simd::<u8, WIDE>::from_slice(&field[pos..pos + WIDE]);
            let mut hits =
                chunk.simd_eq(esc_splat) | chunk.simd_eq(lf_splat) | chunk.simd_eq(cr_splat);
            for splat in &sep_splats {
                hits |= chunk.simd_eq(*splat);
            }
            for splat in &res_splats {
                hits |= chunk.simd_eq(*splat);
            }
            if hits.any() {
                return true;
            }
            pos += WIDE;
        }
    }

    // 16-byte path
    {
        let esc_splat = Simd::<u8, CHUNK>::splat(escape);
        let lf_splat = Simd::<u8, CHUNK>::splat(b'\n');
        let cr_splat = Simd::<u8, CHUNK>::splat(b'\r');
        let sep_splats: Vec<Simd<u8, CHUNK>> = separators
            .iter()
            .map(|&s| Simd::<u8, CHUNK>::splat(s))
            .collect();
        let res_splats: Vec<Simd<u8, CHUNK>> = reserved
            .iter()
            .map(|&r| Simd::<u8, CHUNK>::splat(r))
            .collect();

        while pos + CHUNK <= len {
            let chunk = Simd::<u8, CHUNK>::from_slice(&field[pos..pos + CHUNK]);
            let mut hits =
                chunk.simd_eq(esc_splat) | chunk.simd_eq(lf_splat) | chunk.simd_eq(cr_splat);
            for splat in &sep_splats {
                hits |= chunk.simd_eq(*splat);
            }
            for splat in &res_splats {
                hits |= chunk.simd_eq(*splat);
            }
            if hits.any() {
                return true;
            }
            pos += CHUNK;
        }
    }

    // Scalar tail
    while pos < len {
        let b = field[pos];
        if b == escape
            || b == b'\n'
            || b == b'\r'
            || separators.contains(&b)
            || reserved.contains(&b)
        {
            return true;
        }
        pos += 1;
    }

    false
}

// ==========================================================================
// Scanning: General — multi-byte separator/escape (scalar)
// ==========================================================================

/// Multi-byte: check if field needs quoting
#[inline]
pub fn field_needs_quoting_general(
    field: &[u8],
    separator: &[u8],
    escape: &[u8],
    reserved: &[u8],
) -> bool {
    // Check for newlines and reserved bytes
    for &b in field {
        if b == b'\n' || b == b'\r' || reserved.contains(&b) {
            return true;
        }
    }
    // Check for separator pattern
    if separator.len() == 1 {
        if field.contains(&separator[0]) {
            return true;
        }
    } else if field.len() >= separator.len() {
        for w in field.windows(separator.len()) {
            if w == separator {
                return true;
            }
        }
    }
    // Check for escape pattern
    if escape.len() == 1 {
        if field.contains(&escape[0]) {
            return true;
        }
    } else if field.len() >= escape.len() {
        for w in field.windows(escape.len()) {
            if w == escape {
                return true;
            }
        }
    }
    false
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
    fn test_simd_field_needs_quoting() {
        // Short field — scalar tail
        assert!(field_needs_quoting_simd(b"a,b", b',', b'"', &[]));
        assert!(!field_needs_quoting_simd(b"abc", b',', b'"', &[]));

        // Medium field (>= 16 bytes) — SIMD path
        let clean = b"abcdefghijklmnopqrstuvwxyz";
        assert!(!field_needs_quoting_simd(clean, b',', b'"', &[]));

        let dirty = b"abcdefghijklmno,qrstuvwxyz";
        assert!(field_needs_quoting_simd(dirty, b',', b'"', &[]));
    }

    #[test]
    fn test_simd_field_needs_quoting_multi_sep() {
        assert!(field_needs_quoting_simd_multi_sep(b"a,b", b",;", b'"', &[]));
        assert!(field_needs_quoting_simd_multi_sep(b"a;b", b",;", b'"', &[]));
        assert!(!field_needs_quoting_simd_multi_sep(
            b"abc",
            b",;",
            b'"',
            &[]
        ));
    }

    #[test]
    fn test_field_needs_quoting_general() {
        assert!(field_needs_quoting_general(b"a::b", b"::", b"$$", &[]));
        assert!(field_needs_quoting_general(b"a$$b", b"::", b"$$", &[]));
        assert!(field_needs_quoting_general(b"a\nb", b"::", b"$$", &[]));
        assert!(!field_needs_quoting_general(b"hello", b"::", b"$$", &[]));
    }

    #[test]
    fn test_field_needs_quoting_newlines() {
        assert!(field_needs_quoting_simd(b"line1\nline2", b',', b'"', &[]));
        assert!(field_needs_quoting_simd(b"line1\r\nline2", b',', b'"', &[]));
        assert!(field_needs_quoting_simd(b"line1\rline2", b',', b'"', &[]));
    }

    #[test]
    fn test_field_needs_quoting_escape_char() {
        assert!(field_needs_quoting_simd(b"say \"hello\"", b',', b'"', &[]));
        assert!(!field_needs_quoting_simd(b"say hello", b',', b'"', &[]));
    }

    #[test]
    fn test_empty_field() {
        assert!(!field_needs_quoting_simd(b"", b',', b'"', &[]));
        assert!(!field_needs_quoting_general(b"", b"::", b"$$", &[]));
    }

    #[test]
    fn test_reserved_chars_simd() {
        // Without reserved, $ doesn't trigger quoting
        assert!(!field_needs_quoting_simd(b"price$100", b',', b'"', &[]));
        // With reserved, $ triggers quoting
        assert!(field_needs_quoting_simd(b"price$100", b',', b'"', b"$"));

        // SIMD path (>= 16 bytes)
        let field = b"abcdefghijklmno$qrstuvwxyz";
        assert!(!field_needs_quoting_simd(field, b',', b'"', &[]));
        assert!(field_needs_quoting_simd(field, b',', b'"', b"$"));

        // Multiple reserved chars
        assert!(field_needs_quoting_simd(b"a=b", b',', b'"', b"$="));
    }

    #[test]
    fn test_reserved_chars_multi_sep() {
        assert!(!field_needs_quoting_simd_multi_sep(
            b"a$b",
            b",;",
            b'"',
            &[]
        ));
        assert!(field_needs_quoting_simd_multi_sep(
            b"a$b", b",;", b'"', b"$"
        ));
    }

    #[test]
    fn test_reserved_chars_general() {
        assert!(!field_needs_quoting_general(
            b"price$100",
            b"::",
            b"$$",
            &[]
        ));
        assert!(field_needs_quoting_general(
            b"price@100",
            b"::",
            b"$$",
            b"@"
        ));
    }
}
