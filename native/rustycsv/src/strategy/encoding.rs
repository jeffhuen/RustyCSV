//! UTF-8 → target encoding converters
//!
//! Pure-Rust implementations for converting UTF-8 encoded bytes to other
//! character encodings. No external crate dependencies.

use thiserror::Error;

/// Error returned when UTF-8 input cannot be represented in the target encoding.
#[derive(Debug, Eq, Error, PartialEq)]
pub enum EncodingError {
    #[error("input is not valid UTF-8")]
    InvalidUtf8,
    #[error("input contains a character that cannot be represented in Latin-1")]
    UnmappableLatin1,
}

/// Target encoding for output conversion.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EncodingTarget {
    Utf8,
    Latin1,
    Utf16Le,
    Utf16Be,
    Utf32Le,
    Utf32Be,
}

/// Convert UTF-8 bytes to the target encoding.
///
/// For `Utf8`, returns a copy of the input (caller can optimise this away).
/// For other encodings, decodes UTF-8 and re-encodes to the target.
pub fn encode_utf8_to_target(
    input: &[u8],
    target: EncodingTarget,
) -> Result<Vec<u8>, EncodingError> {
    match target {
        EncodingTarget::Utf8 => Ok(input.to_vec()),
        EncodingTarget::Latin1 => utf8_to_latin1(input),
        EncodingTarget::Utf16Le => utf8_to_utf16(input, false),
        EncodingTarget::Utf16Be => utf8_to_utf16(input, true),
        EncodingTarget::Utf32Le => utf8_to_utf32(input, false),
        EncodingTarget::Utf32Be => utf8_to_utf32(input, true),
    }
}

/// Convert UTF-8 to Latin-1 (ISO-8859-1).
///
/// Fast path: if all bytes are ASCII (< 0x80), return a copy as-is.
/// Otherwise decode UTF-8 char-by-char and emit single bytes.
/// Returns an error rather than silently replacing unmappable codepoints.
pub fn utf8_to_latin1(input: &[u8]) -> Result<Vec<u8>, EncodingError> {
    // Fast check: if all ASCII, shortcut
    if input.iter().all(|&b| b < 0x80) {
        return Ok(input.to_vec());
    }

    let s = std::str::from_utf8(input).map_err(|_| EncodingError::InvalidUtf8)?;

    if s.chars().any(|ch| ch as u32 > 255) {
        return Err(EncodingError::UnmappableLatin1);
    }

    let mut out = Vec::with_capacity(s.len());
    for ch in s.chars() {
        out.push(ch as u8);
    }
    Ok(out)
}

/// Convert UTF-8 to UTF-16 (little-endian or big-endian).
pub fn utf8_to_utf16(input: &[u8], big_endian: bool) -> Result<Vec<u8>, EncodingError> {
    let s = std::str::from_utf8(input).map_err(|_| EncodingError::InvalidUtf8)?;

    let mut out = Vec::with_capacity(s.len() * 2);
    for code_unit in s.encode_utf16() {
        let bytes = if big_endian {
            code_unit.to_be_bytes()
        } else {
            code_unit.to_le_bytes()
        };
        out.extend_from_slice(&bytes);
    }
    Ok(out)
}

/// Convert UTF-8 to UTF-32 (little-endian or big-endian).
pub fn utf8_to_utf32(input: &[u8], big_endian: bool) -> Result<Vec<u8>, EncodingError> {
    let s = std::str::from_utf8(input).map_err(|_| EncodingError::InvalidUtf8)?;

    let mut out = Vec::with_capacity(s.len() * 4);
    for ch in s.chars() {
        let cp = ch as u32;
        let bytes = if big_endian {
            cp.to_be_bytes()
        } else {
            cp.to_le_bytes()
        };
        out.extend_from_slice(&bytes);
    }
    Ok(out)
}

// ==========================================================================
// Extend variants — append encoded bytes directly into an existing buffer
// ==========================================================================

/// Encode UTF-8 bytes and append to output buffer (no intermediate allocation).
pub fn encode_utf8_extend(
    out: &mut Vec<u8>,
    input: &[u8],
    target: EncodingTarget,
) -> Result<(), EncodingError> {
    match target {
        EncodingTarget::Utf8 => {
            out.extend_from_slice(input);
            Ok(())
        }
        EncodingTarget::Latin1 => extend_latin1(out, input),
        EncodingTarget::Utf16Le => extend_utf16(out, input, false),
        EncodingTarget::Utf16Be => extend_utf16(out, input, true),
        EncodingTarget::Utf32Le => extend_utf32(out, input, false),
        EncodingTarget::Utf32Be => extend_utf32(out, input, true),
    }
}

fn extend_latin1(out: &mut Vec<u8>, input: &[u8]) -> Result<(), EncodingError> {
    if input.iter().all(|&b| b < 0x80) {
        out.extend_from_slice(input);
        return Ok(());
    }
    let s = std::str::from_utf8(input).map_err(|_| EncodingError::InvalidUtf8)?;
    if s.chars().any(|ch| ch as u32 > 255) {
        return Err(EncodingError::UnmappableLatin1);
    }
    out.reserve(s.len());
    for ch in s.chars() {
        out.push(ch as u8);
    }
    Ok(())
}

fn extend_utf16(out: &mut Vec<u8>, input: &[u8], big_endian: bool) -> Result<(), EncodingError> {
    let s = std::str::from_utf8(input).map_err(|_| EncodingError::InvalidUtf8)?;
    out.reserve(s.len() * 2);
    for code_unit in s.encode_utf16() {
        let bytes = if big_endian {
            code_unit.to_be_bytes()
        } else {
            code_unit.to_le_bytes()
        };
        out.extend_from_slice(&bytes);
    }
    Ok(())
}

fn extend_utf32(out: &mut Vec<u8>, input: &[u8], big_endian: bool) -> Result<(), EncodingError> {
    let s = std::str::from_utf8(input).map_err(|_| EncodingError::InvalidUtf8)?;
    out.reserve(s.len() * 4);
    for ch in s.chars() {
        let cp = ch as u32;
        let bytes = if big_endian {
            cp.to_be_bytes()
        } else {
            cp.to_le_bytes()
        };
        out.extend_from_slice(&bytes);
    }
    Ok(())
}

// ==========================================================================
// Tests
// ==========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utf8_passthrough() {
        let input = b"hello,world\n";
        let result = encode_utf8_to_target(input, EncodingTarget::Utf8);
        assert_eq!(result, Ok(input.to_vec()));
    }

    #[test]
    fn test_latin1_ascii() {
        let input = b"hello";
        let result = utf8_to_latin1(input);
        assert_eq!(result, Ok(b"hello".to_vec()));
    }

    #[test]
    fn test_latin1_with_accents() {
        // "caf\u{e9}" in UTF-8 is [99, 97, 102, 195, 169]
        let input = "caf\u{e9}".as_bytes();
        let result = utf8_to_latin1(input);
        assert_eq!(result, Ok(vec![99, 97, 102, 0xe9]));
    }

    #[test]
    fn test_latin1_unmappable() {
        // U+0100 (Latin Extended-A) is not in Latin-1
        let input = "\u{0100}".as_bytes();
        assert_eq!(utf8_to_latin1(input), Err(EncodingError::UnmappableLatin1));
    }

    #[test]
    fn test_utf16_le_ascii() {
        let input = b"AB";
        let result = utf8_to_utf16(input, false);
        assert_eq!(result, Ok(vec![0x41, 0x00, 0x42, 0x00]));
    }

    #[test]
    fn test_utf16_be_ascii() {
        let input = b"AB";
        let result = utf8_to_utf16(input, true);
        assert_eq!(result, Ok(vec![0x00, 0x41, 0x00, 0x42]));
    }

    #[test]
    fn test_utf16_le_surrogate_pair() {
        // U+1F600 (grinning face) requires a surrogate pair in UTF-16
        let input = "\u{1F600}".as_bytes();
        let result = utf8_to_utf16(input, false);
        // U+1F600 => surrogate pair: D83D DE00
        assert_eq!(result, Ok(vec![0x3D, 0xD8, 0x00, 0xDE]));
    }

    #[test]
    fn test_utf32_le_ascii() {
        let input = b"A";
        let result = utf8_to_utf32(input, false);
        assert_eq!(result, Ok(vec![0x41, 0x00, 0x00, 0x00]));
    }

    #[test]
    fn test_utf32_be_ascii() {
        let input = b"A";
        let result = utf8_to_utf32(input, true);
        assert_eq!(result, Ok(vec![0x00, 0x00, 0x00, 0x41]));
    }

    #[test]
    fn test_utf32_le_emoji() {
        let input = "\u{1F600}".as_bytes();
        let result = utf8_to_utf32(input, false);
        assert_eq!(result, Ok(vec![0x00, 0xF6, 0x01, 0x00]));
    }

    #[test]
    fn test_empty_input() {
        assert_eq!(encode_utf8_to_target(b"", EncodingTarget::Utf8), Ok(vec![]));
        assert_eq!(
            encode_utf8_to_target(b"", EncodingTarget::Latin1),
            Ok(vec![])
        );
        assert_eq!(
            encode_utf8_to_target(b"", EncodingTarget::Utf16Le),
            Ok(vec![])
        );
        assert_eq!(
            encode_utf8_to_target(b"", EncodingTarget::Utf32Be),
            Ok(vec![])
        );
    }

    // extend variant tests — verify parity with allocating versions

    #[test]
    fn test_extend_utf8() {
        let mut out = vec![0xAA]; // pre-existing data
        assert_eq!(
            encode_utf8_extend(&mut out, b"hello", EncodingTarget::Utf8),
            Ok(())
        );
        assert_eq!(out, &[0xAA, b'h', b'e', b'l', b'l', b'o']);
    }

    #[test]
    fn test_extend_latin1() {
        let mut out = Vec::new();
        assert_eq!(
            encode_utf8_extend(&mut out, "caf\u{e9}".as_bytes(), EncodingTarget::Latin1),
            Ok(())
        );
        assert_eq!(Ok(out), utf8_to_latin1("caf\u{e9}".as_bytes()));
    }

    #[test]
    fn test_extend_utf16_le() {
        let mut out = Vec::new();
        assert_eq!(
            encode_utf8_extend(&mut out, b"AB", EncodingTarget::Utf16Le),
            Ok(())
        );
        assert_eq!(Ok(out), utf8_to_utf16(b"AB", false));
    }

    #[test]
    fn test_extend_utf16_be() {
        let mut out = Vec::new();
        assert_eq!(
            encode_utf8_extend(&mut out, b"AB", EncodingTarget::Utf16Be),
            Ok(())
        );
        assert_eq!(Ok(out), utf8_to_utf16(b"AB", true));
    }

    #[test]
    fn test_extend_utf32_le() {
        let mut out = Vec::new();
        assert_eq!(
            encode_utf8_extend(&mut out, "\u{1F600}".as_bytes(), EncodingTarget::Utf32Le),
            Ok(())
        );
        assert_eq!(Ok(out), utf8_to_utf32("\u{1F600}".as_bytes(), false));
    }

    #[test]
    fn test_extend_utf32_be() {
        let mut out = Vec::new();
        assert_eq!(
            encode_utf8_extend(&mut out, b"A", EncodingTarget::Utf32Be),
            Ok(())
        );
        assert_eq!(Ok(out), utf8_to_utf32(b"A", true));
    }

    #[test]
    fn test_extend_appends() {
        // Verify extend appends rather than replacing
        let mut out = vec![1, 2, 3];
        assert_eq!(
            encode_utf8_extend(&mut out, b"X", EncodingTarget::Utf16Le),
            Ok(())
        );
        assert_eq!(out, &[1, 2, 3, 0x58, 0x00]);
    }

    #[test]
    fn invalid_utf8_and_unmappable_latin1_are_errors() {
        assert_eq!(
            encode_utf8_to_target(&[0xff], EncodingTarget::Utf16Le),
            Err(EncodingError::InvalidUtf8)
        );
        assert_eq!(
            encode_utf8_to_target("日本".as_bytes(), EncodingTarget::Latin1),
            Err(EncodingError::UnmappableLatin1)
        );
    }
}
