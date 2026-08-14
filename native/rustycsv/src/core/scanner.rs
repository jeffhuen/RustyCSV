//! Byte-level helpers for field splitting.

/// Check if a byte is one of the separator bytes
/// Optimized for common cases of 1-3 separators
#[inline]
pub fn is_separator(byte: u8, separators: &[u8]) -> bool {
    match separators.len() {
        0 => false,
        1 => byte == separators[0],
        2 => byte == separators[0] || byte == separators[1],
        3 => byte == separators[0] || byte == separators[1] || byte == separators[2],
        _ => separators.contains(&byte),
    }
}
