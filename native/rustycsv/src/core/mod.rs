//! Core primitives for CSV parsing

pub mod field;
pub mod newlines;
pub mod scanner;
pub mod simd_index;
pub mod simd_scanner;

pub use field::*;
pub use newlines::*;
pub use scanner::*;
#[allow(unused_imports)]
pub use simd_index::RowEnd;
pub use simd_index::{
    InputTooLarge, ScannedBoundaries, StructuralIndex, Violation, MAX_INPUT_SIZE,
};
pub use simd_scanner::scan_structural;
pub use simd_scanner::CHUNK;
#[cfg(target_feature = "avx2")]
pub use simd_scanner::WIDE;
