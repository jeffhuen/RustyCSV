// SIMD structural CSV scanner — simdjson-style prefix-XOR quote detection
//
// Scans the entire input once, producing a StructuralIndex of all unquoted
// separators and row endings. Both batch code paths consume this index: the
// :simd family (:simd, :basic, :indexed, :zero_copy) and :parallel.
//
// ## Stabilization-safe API subset (std::simd)
//
// We use only: Simd::from_slice, splat, simd_eq, to_bitmask, bitwise ops.
// These are the most stable parts of portable_simd. We avoid: swizzle,
// scatter, gather, and any SIMD shuffles.
//
// ## Bitmask types
//
// On current nightly, `Mask::to_bitmask()` returns u64 regardless of lane
// count. We mask to the relevant bits (lower 16 for CHUNK=16, lower 32 for
// WIDE=32) and operate on u64 uniformly.
//
// ## Optimization notes
//
// - Uses `to_bitmask()` + bit extraction (not `.any()` + break) because we
//   need ALL structural positions, not just the first one per chunk.
// - Prefix-XOR for quote region detection: portable shift-and-xor cascade
//   for all targets, with CLMUL/PMULL fast paths on x86_64/aarch64.
// - AVX2 wide path (32 bytes) processes first, then 16-byte remainder,
//   then scalar tail. Same pattern as RustyJSON's skip_plain_string_bytes.

use std::simd::prelude::*;

use super::simd_index::{RowEnd, StructuralIndex};

/// Baseline SIMD chunk size (128-bit).
pub const CHUNK: usize = 16;

/// Wide chunk size for AVX2 targets.
#[cfg(target_feature = "avx2")]
pub const WIDE: usize = 32;

// ---------------------------------------------------------------------------
// Prefix-XOR: compute cumulative XOR to determine quoted regions
// ---------------------------------------------------------------------------
//
// Given a bitmask where bit i is set if position i has a quote character,
// prefix_xor(mask) produces a bitmask where bit i is set if position i is
// inside a quoted region (odd number of quotes before it).

/// Prefix-XOR via shift-and-xor cascade (works for 16 and 32 bits
/// within a u64, since upper bits are zero).
///
/// For these small bit widths the cascade is 6 dependent XOR+shift ops (~6 cycles),
/// comparable to a single CLMUL/PMULL instruction (~3-4 cycle latency + setup).
/// Using the portable version keeps the scanner free of `unsafe`.
#[inline]
fn prefix_xor(mut x: u64) -> u64 {
    x ^= x << 1;
    x ^= x << 2;
    x ^= x << 4;
    x ^= x << 8;
    x ^= x << 16;
    x ^= x << 32;
    x
}

// ---------------------------------------------------------------------------
// Bitmask extraction helpers
// ---------------------------------------------------------------------------

/// Extract set bit positions from a u64 bitmask, adding `base_pos` offset.
/// Only examines the lower `n_bits` bits. Appends to `out`.
#[inline]
fn extract_positions(mut mask: u64, base_pos: u32, out: &mut Vec<u32>) {
    while mask != 0 {
        let bit = mask.trailing_zeros();
        out.push(base_pos + bit);
        mask &= mask - 1; // clear lowest set bit
    }
}

// ---------------------------------------------------------------------------
// Core scanner
// ---------------------------------------------------------------------------

/// Scan the input and produce a `StructuralIndex`.
///
/// `separators` are the field delimiter bytes (e.g., &[b',']).
/// `escape` is the quote/escape byte (e.g., b'"').
///
/// # Panics
///
/// Positions are stored as `u32`. Inputs larger than `u32::MAX` (4 GiB)
/// will silently produce incorrect results due to truncation.
/// **Callers must validate input length before calling this function.**
/// The NIF layer enforces this via `guard_input_size`.
pub fn scan_structural(input: &[u8], separators: &[u8], escape: u8) -> StructuralIndex {
    let est_seps = input.len() / 10 + 16;
    let est_rows = input.len() / 50 + 4;
    let mut field_seps: Vec<u32> = Vec::with_capacity(est_seps);
    let mut row_ends: Vec<RowEnd> = Vec::with_capacity(est_rows);

    let mut pos: usize = 0;
    let mut quote_carry: u64 = 0; // 0 or 1: parity of quotes seen so far

    // -----------------------------------------------------------------------
    // AVX2 wide path: 32-byte chunks
    // -----------------------------------------------------------------------
    #[cfg(target_feature = "avx2")]
    {
        let esc_splat = Simd::<u8, WIDE>::splat(escape);
        let lf_splat = Simd::<u8, WIDE>::splat(b'\n');
        let cr_splat = Simd::<u8, WIDE>::splat(b'\r');

        let sep_splats: Vec<Simd<u8, WIDE>> = separators
            .iter()
            .map(|&s| Simd::<u8, WIDE>::splat(s))
            .collect();

        const MASK_32: u64 = (1u64 << 32) - 1;

        while pos + WIDE <= input.len() {
            let chunk = Simd::<u8, WIDE>::from_slice(&input[pos..pos + WIDE]);
            let base = pos as u32;

            let esc_mask = chunk.simd_eq(esc_splat).to_bitmask() & MASK_32;

            let raw_quoted = prefix_xor(esc_mask) & MASK_32;
            let quoted = raw_quoted ^ (quote_carry.wrapping_neg() & MASK_32);

            quote_carry ^= (esc_mask.count_ones() as u64) & 1;

            let not_quoted = !quoted & MASK_32;

            let mut sep_bits: u64 = 0;
            for splat in &sep_splats {
                sep_bits |= chunk.simd_eq(*splat).to_bitmask() & MASK_32;
            }
            extract_positions(sep_bits & not_quoted, base, &mut field_seps);

            let lf_bits = chunk.simd_eq(lf_splat).to_bitmask() & not_quoted;
            let cr_bits = chunk.simd_eq(cr_splat).to_bitmask() & not_quoted;

            emit_row_ends(input, pos, lf_bits, cr_bits, &mut row_ends);

            pos += WIDE;
        }
    }

    // -----------------------------------------------------------------------
    // 16-byte chunks
    // -----------------------------------------------------------------------
    {
        let esc_splat = Simd::<u8, CHUNK>::splat(escape);
        let lf_splat = Simd::<u8, CHUNK>::splat(b'\n');
        let cr_splat = Simd::<u8, CHUNK>::splat(b'\r');

        let sep_splats: Vec<Simd<u8, CHUNK>> = separators
            .iter()
            .map(|&s| Simd::<u8, CHUNK>::splat(s))
            .collect();

        const MASK_16: u64 = (1u64 << 16) - 1;

        while pos + CHUNK <= input.len() {
            let chunk = Simd::<u8, CHUNK>::from_slice(&input[pos..pos + CHUNK]);
            let base = pos as u32;

            let esc_mask = chunk.simd_eq(esc_splat).to_bitmask() & MASK_16;

            let raw_quoted = prefix_xor(esc_mask) & MASK_16;
            let quoted = raw_quoted ^ (quote_carry.wrapping_neg() & MASK_16);

            quote_carry ^= (esc_mask.count_ones() as u64) & 1;

            let not_quoted = !quoted & MASK_16;

            let mut sep_bits: u64 = 0;
            for splat in &sep_splats {
                sep_bits |= chunk.simd_eq(*splat).to_bitmask() & MASK_16;
            }
            extract_positions(sep_bits & not_quoted, base, &mut field_seps);

            let lf_bits = chunk.simd_eq(lf_splat).to_bitmask() & not_quoted;
            let cr_bits = chunk.simd_eq(cr_splat).to_bitmask() & not_quoted;

            emit_row_ends(input, pos, lf_bits, cr_bits, &mut row_ends);

            pos += CHUNK;
        }
    }

    // -----------------------------------------------------------------------
    // Scalar tail
    // -----------------------------------------------------------------------
    scan_scalar_tail(
        input,
        pos,
        separators,
        escape,
        quote_carry != 0,
        &mut field_seps,
        &mut row_ends,
    );

    StructuralIndex {
        field_seps,
        row_ends,
        input_len: input.len() as u32,
    }
}

/// Incremental scan for the streaming parser.
///
/// Scans `input[start..]` with the given carry state.
/// Returns the updated carry state (true = currently in quotes).
#[allow(dead_code)]
pub fn scan_structural_incremental(
    input: &[u8],
    start: usize,
    separators: &[u8],
    escape: u8,
    in_quotes: bool,
    field_seps: &mut Vec<u32>,
    row_ends: &mut Vec<RowEnd>,
) -> bool {
    let mut pos = start;
    let mut quote_carry: u64 = if in_quotes { 1 } else { 0 };

    {
        let esc_splat = Simd::<u8, CHUNK>::splat(escape);
        let lf_splat = Simd::<u8, CHUNK>::splat(b'\n');
        let cr_splat = Simd::<u8, CHUNK>::splat(b'\r');

        let sep_splats: Vec<Simd<u8, CHUNK>> = separators
            .iter()
            .map(|&s| Simd::<u8, CHUNK>::splat(s))
            .collect();

        const MASK_16: u64 = (1u64 << 16) - 1;

        while pos + CHUNK <= input.len() {
            let chunk = Simd::<u8, CHUNK>::from_slice(&input[pos..pos + CHUNK]);
            let base = pos as u32;

            let esc_mask = chunk.simd_eq(esc_splat).to_bitmask() & MASK_16;
            let raw_quoted = prefix_xor(esc_mask) & MASK_16;
            let quoted = raw_quoted ^ (quote_carry.wrapping_neg() & MASK_16);
            quote_carry ^= (esc_mask.count_ones() as u64) & 1;
            let not_quoted = !quoted & MASK_16;

            let mut sep_bits: u64 = 0;
            for splat in &sep_splats {
                sep_bits |= chunk.simd_eq(*splat).to_bitmask() & MASK_16;
            }
            extract_positions(sep_bits & not_quoted, base, field_seps);

            let lf_bits = chunk.simd_eq(lf_splat).to_bitmask() & not_quoted;
            let cr_bits = chunk.simd_eq(cr_splat).to_bitmask() & not_quoted;
            emit_row_ends(input, pos, lf_bits, cr_bits, row_ends);

            pos += CHUNK;
        }
    }

    scan_scalar_tail(
        input,
        pos,
        separators,
        escape,
        quote_carry != 0,
        field_seps,
        row_ends,
    )
}

// ---------------------------------------------------------------------------
// Row-end emission from bitmasks
// ---------------------------------------------------------------------------

/// Emit RowEnd entries from LF and CR bitmasks (u64, only lower bits used).
///
/// For each \n bit: check if preceded by \r → emit RowEnd { pos: \r_pos, len: 2 }
/// else emit RowEnd { pos: \n_pos, len: 1 }.
///
/// \r bits that are NOT followed by \n are bare \r = data (per RFC 4180), ignored.
#[inline]
fn emit_row_ends(
    input: &[u8],
    chunk_start: usize,
    mut lf_bits: u64,
    _cr_bits: u64,
    out: &mut Vec<RowEnd>,
) {
    while lf_bits != 0 {
        let bit = lf_bits.trailing_zeros() as usize;
        let abs_pos = chunk_start + bit;
        if abs_pos > 0 && input[abs_pos - 1] == b'\r' {
            out.push(RowEnd {
                pos: (abs_pos - 1) as u32,
                len: 2,
            });
        } else {
            out.push(RowEnd {
                pos: abs_pos as u32,
                len: 1,
            });
        }
        lf_bits &= lf_bits - 1;
    }
}

// ---------------------------------------------------------------------------
// Scalar fallback for tail bytes
// ---------------------------------------------------------------------------

/// Scalar scan for remaining bytes after SIMD processing.
/// Returns the final `in_quotes` state.
fn scan_scalar_tail(
    input: &[u8],
    start: usize,
    separators: &[u8],
    escape: u8,
    mut in_quotes: bool,
    field_seps: &mut Vec<u32>,
    row_ends: &mut Vec<RowEnd>,
) -> bool {
    let mut pos = start;

    while pos < input.len() {
        let byte = input[pos];

        if in_quotes {
            if byte == escape {
                if pos + 1 < input.len() && input[pos + 1] == escape {
                    pos += 2;
                    continue;
                }
                in_quotes = false;
            }
            pos += 1;
        } else if byte == escape {
            in_quotes = true;
            pos += 1;
        } else if byte == b'\n' {
            if pos > 0 && input[pos - 1] == b'\r' {
                row_ends.push(RowEnd {
                    pos: (pos - 1) as u32,
                    len: 2,
                });
            } else {
                row_ends.push(RowEnd {
                    pos: pos as u32,
                    len: 1,
                });
            }
            pos += 1;
        } else if is_sep_scalar(byte, separators) {
            field_seps.push(pos as u32);
            pos += 1;
        } else {
            pos += 1;
        }
    }

    in_quotes
}

#[inline]
fn is_sep_scalar(byte: u8, separators: &[u8]) -> bool {
    match separators.len() {
        1 => byte == separators[0],
        2 => byte == separators[0] || byte == separators[1],
        3 => byte == separators[0] || byte == separators[1] || byte == separators[2],
        _ => separators.contains(&byte),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Common scenarios moved to tests/conformance.rs.
    // Only unique SIMD-specific tests (boundary edge cases, carry propagation,
    // scalar tail, incremental API) remain here.

    fn scan(input: &[u8]) -> StructuralIndex {
        scan_structural(input, b",", b'"')
    }

    // =======================================================================
    // prefix_xor correctness
    // =======================================================================

    #[test]
    fn test_prefix_xor_known_values() {
        // prefix_xor(mask) computes a cumulative XOR: bit i of the result is
        // set iff an odd number of bits at positions 0..=i are set in the input.
        // This is the core algorithm for quote region detection.

        // Reference: compute prefix XOR bit-by-bit
        fn prefix_xor_reference(mask: u64, bits: usize) -> u64 {
            let mut result = 0u64;
            let mut parity = 0u64;
            for i in 0..bits {
                parity ^= (mask >> i) & 1;
                result |= parity << i;
            }
            result
        }

        // Verify portable implementation against reference for interesting masks
        let test_masks: &[u64] = &[
            0,      // no quotes
            1,      // single quote at position 0
            0b11,   // two adjacent quotes (open+close, cancels out)
            0b101,  // quotes at 0 and 2
            0b1000, // single quote at position 3
            0b1001, // quotes at 0 and 3
            0xFF,   // 8 consecutive quotes
            0xAAAA, // alternating bits
            0x8001, // quotes at 0 and 15
            0xFFFF, // all 16 bits
        ];

        for &mask in test_masks {
            let expected = prefix_xor_reference(mask, 16);
            assert_eq!(
                prefix_xor(mask) & 0xFFFF,
                expected,
                "prefix_xor wrong for mask {mask:#018b}"
            );
        }

        // Spot-check semantics for CSV:
        // Single quote at pos 0: everything after is "in quotes"
        assert_eq!(prefix_xor(1) & 0xFFFF, 0xFFFF);
        // Two quotes at pos 0,1 (open then close): only pos 0 is "in quotes"
        assert_eq!(prefix_xor(0b11) & 0xFFFF, 1);
        // Quote at pos 0 and pos 5 (open...close): positions 0-4 in quotes
        assert_eq!(prefix_xor(0b100001) & 0xFFFF, 0b011111);
    }

    // =======================================================================
    // Quote region: separators and newlines inside quotes must be suppressed
    // =======================================================================

    #[test]
    fn test_quote_suppresses_separator() {
        // a,"b,c",d\n — comma at position 4 is inside quotes
        let input = b"a,\"b,c\",d\n";
        // positions: a=0 ,=1 "=2 b=3 ,=4 c=5 "=6 ,=7 d=8 \n=9
        let idx = scan(input);

        assert_eq!(
            idx.field_seps,
            vec![1, 7],
            "comma at pos 4 must be suppressed"
        );
        assert_eq!(idx.row_ends, vec![RowEnd { pos: 9, len: 1 }]);
    }

    #[test]
    fn test_quote_suppresses_newline() {
        // a,"b\nc",d\n — newline at position 4 is inside quotes
        let input = b"a,\"b\nc\",d\n";
        // positions: a=0 ,=1 "=2 b=3 \n=4 c=5 "=6 ,=7 d=8 \n=9
        let idx = scan(input);

        assert_eq!(idx.field_seps, vec![1, 7]);
        assert_eq!(
            idx.row_ends,
            vec![RowEnd { pos: 9, len: 1 }],
            "\\n at pos 4 inside quotes must not produce a row end"
        );
    }

    #[test]
    fn test_crlf_inside_quotes_suppressed() {
        // a,"b\r\nc",d\n — \r\n at positions 4-5 is inside quotes
        let input = b"a,\"b\r\nc\",d\n";
        // positions: a=0 ,=1 "=2 b=3 \r=4 \n=5 c=6 "=7 ,=8 d=9 \n=10
        let idx = scan(input);

        assert_eq!(idx.field_seps, vec![1, 8]);
        assert_eq!(
            idx.row_ends,
            vec![RowEnd { pos: 10, len: 1 }],
            "\\r\\n inside quotes must not produce a row end"
        );
    }

    #[test]
    fn test_doubled_quotes_keep_subsequent_separator_suppressed() {
        // "say ""hi""",done\n — doubled quotes inside a quoted field
        // The comma at pos 12 is the real separator; quotes toggle in/out
        // but net effect keeps the field intact.
        let input = b"\"say \"\"hi\"\"\",done\n";
        // positions: "=0 s=1 a=2 y=3 _=4 "=5 "=6 h=7 i=8 "=9 "=10 "=11 ,=12 d=13 o=14 n=15 e=16 \n=17
        let idx = scan(input);

        assert_eq!(
            idx.field_seps,
            vec![12],
            "only the comma between fields should be detected"
        );
        assert_eq!(idx.row_ends, vec![RowEnd { pos: 17, len: 1 }]);
    }

    // =======================================================================
    // CRLF detection: RowEnd pos and len must be exact
    // =======================================================================

    #[test]
    fn test_crlf_produces_row_end_len_2() {
        let input = b"a,b\r\nc,d\n";
        // positions: a=0 ,=1 b=2 \r=3 \n=4 c=5 ,=6 d=7 \n=8
        let idx = scan(input);

        assert_eq!(idx.field_seps, vec![1, 6]);
        assert_eq!(
            idx.row_ends,
            vec![RowEnd { pos: 3, len: 2 }, RowEnd { pos: 8, len: 1 }],
            "CRLF must produce pos=\\r, len=2"
        );
    }

    #[test]
    fn test_bare_cr_is_data() {
        let input = b"a\rb\n";
        // positions: a=0 \r=1 b=2 \n=3
        let idx = scan(input);

        assert_eq!(idx.field_seps, vec![], "no separators");
        assert_eq!(
            idx.row_ends,
            vec![RowEnd { pos: 3, len: 1 }],
            "bare \\r is data, not a row ending"
        );
    }

    #[test]
    fn test_cr_at_chunk_boundary_then_lf() {
        // \r at byte 15 (end of first 16-byte chunk), \n at byte 16 (start of next)
        let mut input = vec![b'x'; 15];
        input.push(b'\r');
        input.push(b'\n');
        input.extend_from_slice(b"y\n");
        // positions: x*15=0..14 \r=15 \n=16 y=17 \n=18

        let idx = scan_structural(&input, b",", b'"');
        assert_eq!(idx.field_seps, vec![]);
        assert_eq!(
            idx.row_ends,
            vec![RowEnd { pos: 15, len: 2 }, RowEnd { pos: 18, len: 1 }],
            "CRLF split across chunks must still produce len=2"
        );
    }

    // =======================================================================
    // Carry propagation: quoted field spanning SIMD chunks
    // =======================================================================

    #[test]
    fn test_carry_across_chunk_boundary() {
        // Quote opens in first 16-byte chunk, closes in the next region.
        // The comma at position 1 is before the quote (real separator).
        // All content between the quotes must be suppressed.
        let mut input = Vec::new();
        input.extend_from_slice(b"x,\"0123456789ab"); // 16 bytes: x=0 ,=1 "=2 then data
        input.extend_from_slice(b"cdefghij\",y\n"); // "=23 ,=24 y=25 \n=26
                                                    // Total: 28 bytes — 16-byte SIMD chunk + 12-byte scalar tail

        let idx = scan_structural(&input, b",", b'"');

        assert_eq!(
            idx.field_seps,
            vec![1, 24],
            "carry must propagate: commas inside the quoted field must be suppressed"
        );
        assert_eq!(idx.row_ends, vec![RowEnd { pos: 26, len: 1 }]);

        // Verify via FieldIter too
        let fields: Vec<_> = idx.fields_in_row(0, 26).collect();
        assert_eq!(fields, vec![(0, 1), (2, 24), (25, 26)]);
    }

    #[test]
    fn test_even_quote_count_carry_is_zero() {
        // Two quotes in the first chunk (open and close): carry should be 0.
        // Second chunk sees unquoted content with separators.
        let mut input = Vec::new();
        input.extend_from_slice(b"\"abcdefghijklm\""); // 16 bytes: "=0, data, "=15 (wait that's 15 chars + 1 = 16 but I need quotes at 0 and 14)
                                                       // Let me be precise: `"abcdefghijklm"` = " a b c d e f g h i j k l m " = 15 bytes, not 16.
                                                       // Use `"abcdefghijklmn"` = 16 bytes: "=0 data=1..14 "=15
        input.clear();
        input.extend_from_slice(b"\"abcdefghijklmn\""); // 17 bytes, but I need exactly 16 for first chunk
                                                        // Actually: `"0123456789abc"` = " 0 1 2 3 4 5 6 7 8 9 a b c " = 15 bytes.
                                                        // For 16 bytes: `"0123456789abcd"` = 16 bytes with " at 0 and " at 15.
        input.clear();
        input.extend_from_slice(b"\"0123456789abcd\""); // This is 17 bytes: " at 0, data at 1-14, " at 15... wait no:
                                                        // b"\"0123456789abcd\"" = \" produces one byte (the literal quote)
                                                        // So: quote, 0-9 (10), a-d (4), quote = 16 bytes total. Yes!
                                                        // positions: "=0 0=1 1=2 ... d=14 "=15
                                                        // Two quotes in this chunk → even → carry = 0
        input.clear();
        input.extend_from_slice(b"\"0123456789abcd\"");
        input.extend_from_slice(b",x,y\n");
        // continuation: ,=16 x=17 ,=18 y=19 \n=20

        let idx = scan_structural(&input, b",", b'"');

        assert_eq!(
            idx.field_seps,
            vec![16, 18],
            "even quotes → carry=0, so separators in next chunk must be detected"
        );
        assert_eq!(idx.row_ends, vec![RowEnd { pos: 20, len: 1 }]);
    }

    #[test]
    fn test_odd_quote_count_carry() {
        // Quote opens in first chunk, doesn't close until second region.
        // 1 quote in first chunk → odd → carry = 1.
        let mut input = vec![b'"'];
        input.extend_from_slice(&[b'x'; 14]); // fill to 15 bytes
                                              // Now we're at 15 bytes. Add a comma inside the quote to verify suppression.
        input.push(b','); // position 15, inside quotes
                          // 16 bytes: "=0 x*14=1..14 ,=15 → this is one SIMD chunk
                          // carry after this chunk: 1 quote → carry = 1

        // Second chunk (scalar tail): close quote, real separator, data, newline
        input.extend_from_slice(b"\",y\n"); // "=16 ,=17 y=18 \n=19

        let idx = scan_structural(&input, b",", b'"');

        assert_eq!(
            idx.field_seps,
            vec![17],
            "comma at pos 15 is inside quotes (carry=1), comma at 17 is real"
        );
        assert_eq!(idx.row_ends, vec![RowEnd { pos: 19, len: 1 }]);
    }

    // =======================================================================
    // Scalar tail and small inputs
    // =======================================================================

    #[test]
    fn test_input_shorter_than_chunk() {
        let input = b"a,b\n";
        let idx = scan(input);

        assert_eq!(idx.field_seps, vec![1]);
        assert_eq!(idx.row_ends, vec![RowEnd { pos: 3, len: 1 }]);
        assert_eq!(
            idx.fields_in_row(0, 3).collect::<Vec<_>>(),
            vec![(0, 1), (2, 3)]
        );
    }

    #[test]
    fn test_single_field_no_separator_no_newline() {
        let input = b"hello";
        let idx = scan(input);

        assert_eq!(idx.field_seps, vec![]);
        assert_eq!(idx.row_ends, vec![]);
        assert_eq!(
            idx.row_count(),
            1,
            "trailing content without newline is one row"
        );
        let fields: Vec<_> = idx.fields_in_row(0, 5).collect();
        assert_eq!(fields, vec![(0, 5)]);
    }

    #[test]
    fn test_empty_quoted_field() {
        let input = b"a,\"\",c\n";
        // positions: a=0 ,=1 "=2 "=3 ,=4 c=5 \n=6
        let idx = scan(input);

        assert_eq!(idx.field_seps, vec![1, 4]);
        assert_eq!(idx.row_ends, vec![RowEnd { pos: 6, len: 1 }]);
    }

    // =======================================================================
    // Multi-separator
    // =======================================================================

    #[test]
    fn test_multiple_separators() {
        let input = b"a;b\tc\n";
        let idx = scan_structural(input, b";\t", b'"');

        assert_eq!(idx.field_seps, vec![1, 3]);
        assert_eq!(idx.row_ends, vec![RowEnd { pos: 5, len: 1 }]);
        assert_eq!(
            idx.fields_in_row(0, 5).collect::<Vec<_>>(),
            vec![(0, 1), (2, 3), (4, 5)]
        );
    }

    // =======================================================================
    // Sustained multi-chunk processing
    // =======================================================================

    #[test]
    fn test_large_input_all_separators_correct() {
        // 100 identical rows, verify EVERY separator and row_end is correct
        let line = b"aaa,bbb,ccc\n"; // 12 bytes, seps at offsets 3 and 7
        let mut input = Vec::new();
        for _ in 0..100 {
            input.extend_from_slice(line);
        }
        let idx = scan_structural(&input, b",", b'"');

        assert_eq!(idx.row_ends.len(), 100);

        // Verify every separator position
        let expected_seps: Vec<u32> = (0..100u32)
            .flat_map(|r| vec![r * 12 + 3, r * 12 + 7])
            .collect();
        assert_eq!(idx.field_seps, expected_seps);

        // Verify every row end
        for (i, re) in idx.row_ends.iter().enumerate() {
            assert_eq!(re.pos, (i as u32) * 12 + 11, "row {i} end position");
            assert_eq!(re.len, 1, "row {i} end length");
        }
    }

    // =======================================================================
    // Incremental scan API
    // =======================================================================

    #[test]
    fn test_incremental_scan_exact_output() {
        let input = b"a,b\nc,d\n";
        let mut seps = Vec::new();
        let mut ends = Vec::new();

        let carry = scan_structural_incremental(input, 0, b",", b'"', false, &mut seps, &mut ends);

        assert!(!carry);
        assert_eq!(seps, vec![1, 5]);
        assert_eq!(
            ends,
            vec![RowEnd { pos: 3, len: 1 }, RowEnd { pos: 7, len: 1 }]
        );
    }

    #[test]
    fn test_incremental_with_in_quotes_true() {
        // Simulate resuming mid-quoted-field: in_quotes=true means we're inside a quote
        // from a previous chunk. Separator should be suppressed until closing quote.
        let input = b"inside,more\",real\n";
        // positions: i=0 n=1 s=2 i=3 d=4 e=5 ,=6 m=7 o=8 r=9 e=10 "=11 ,=12 r=13 e=14 a=15 l=16 \n=17
        // With in_quotes=true: everything before the " at 11 is inside quotes.
        // comma at 6: suppressed (in quotes)
        // " at 11: closes the quote
        // comma at 12: real separator
        let mut seps = Vec::new();
        let mut ends = Vec::new();

        let carry = scan_structural_incremental(input, 0, b",", b'"', true, &mut seps, &mut ends);

        assert!(!carry, "quote closed at pos 11, should not be in quotes");
        assert_eq!(
            seps,
            vec![12],
            "comma at 6 is inside quotes, only 12 is real"
        );
        assert_eq!(ends, vec![RowEnd { pos: 17, len: 1 }]);
    }

    // =======================================================================
    // Input size limit: u32 positions cap input at 4 GiB
    // =======================================================================

    #[test]
    fn test_input_len_stored_as_u32() {
        // Verify that StructuralIndex.input_len is u32, so inputs at the
        // u32::MAX boundary produce correct values while larger inputs
        // would wrap. The NIF layer guards against this with guard_input_size.
        let idx = scan(b"a,b\n");
        assert_eq!(idx.input_len, 4_u32);

        // Verify the as-u32 cast at the max boundary is safe
        let max: usize = u32::MAX as usize;
        assert_eq!(max as u32, u32::MAX);

        // max + 1 would wrap to 0 — this is the truncation the guard prevents
        let overflow: usize = max + 1;
        assert_eq!(
            overflow as u32, 0,
            "u32 wraparound confirms why guard is needed"
        );
    }
}
