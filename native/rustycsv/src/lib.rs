#![feature(portable_simd)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
//! RustyCSV - Fast CSV parsing with multiple strategies
//!
//! NIF safety: no unwrap/expect in production code. Fallible paths use match + early return.
//!
//! Strategies:
// A/B/C/F: SIMD boundary scan + hybrid sub-binary term builder (parse_string,
//          parse_string_fast, parse_string_indexed, parse_string_zero_copy).
//          These are functionally equivalent — all use the same code path.
//          Kept as separate NIFs for backward-compatible Elixir API.
// D:       Streaming chunked parser (streaming_*) — bounded memory, processes chunks.
// E:       Parallel parsing via rayon (parse_string_parallel) — multi-threaded
//          extraction after a single-threaded SIMD scan.

use rustler::types::ListIterator;
use rustler::{Atom, Binary, Encoder, Env, Error, NewBinary, NifResult, ResourceArc, Term};

mod atoms {
    rustler::atoms! {
        ok,
        error,
        mutex_poisoned,
        buffer_overflow,
        input_too_large,
        malformed_csv,
        unexpected_quote,
        trailing_garbage,
        unterminated_quote,
        non_binary_field,
    }
}

pub mod core;
mod resource;
pub mod strategy;
mod term;

/// Returns `true` when `len` exceeds the batch parsing size limit.
#[inline]
fn exceeds_batch_limit(len: usize) -> bool {
    len > core::MAX_INPUT_SIZE
}

/// Check whether `input` exceeds the batch parsing size limit.
/// Returns an `{:error, :input_too_large}` term on failure.
/// Streaming parsing is not affected — it processes data in small chunks.
fn guard_input_size<'a>(env: Env<'a>, input: &[u8]) -> Result<(), Term<'a>> {
    if exceeds_batch_limit(input.len()) {
        Err((atoms::error(), atoms::input_too_large()).encode(env))
    } else {
        Ok(())
    }
}

fn decode_field_binary<'a>(env: Env<'a>, term: Term<'a>) -> Result<Binary<'a>, Term<'a>> {
    term.decode()
        .map_err(|_| (atoms::error(), atoms::non_binary_field()).encode(env))
}

/// Separators: list of patterns. Each pattern can be multi-byte.
struct Separators {
    patterns: Vec<Vec<u8>>,
}

/// Escape: single pattern, possibly multi-byte.
struct Escape {
    bytes: Vec<u8>,
}

/// Decode separator from a Term.
/// Accepts: integer 44, binary <<44>>, or list [<<44>>, <<59>>], [<<58,58>>]
fn decode_separators<'a>(term: Term<'a>) -> NifResult<Separators> {
    // Try integer (single byte, single separator)
    if let Ok(byte) = term.decode::<u8>() {
        return Ok(Separators {
            patterns: vec![vec![byte]],
        });
    }
    // Try list of binaries (multiple separators, each possibly multi-byte)
    // Must check list BEFORE single binary.
    if let Ok(list) = term.decode::<Vec<Binary<'a>>>() {
        let patterns: Vec<Vec<u8>> = list.iter().map(|b| b.as_slice().to_vec()).collect();
        if patterns.is_empty() || patterns.iter().any(|p| p.is_empty()) {
            return Err(Error::BadArg);
        }
        return Ok(Separators { patterns });
    }
    // Try single binary (single separator, possibly multi-byte)
    if let Ok(binary) = term.decode::<Binary<'a>>() {
        let slice = binary.as_slice();
        if slice.is_empty() {
            return Err(Error::BadArg);
        }
        return Ok(Separators {
            patterns: vec![slice.to_vec()],
        });
    }
    Err(Error::BadArg)
}

/// Decode escape from a Term.
/// Accepts: integer 34 or binary <<34>> or binary <<36,36>>
fn decode_escape<'a>(term: Term<'a>) -> NifResult<Escape> {
    if let Ok(byte) = term.decode::<u8>() {
        return Ok(Escape { bytes: vec![byte] });
    }
    if let Ok(binary) = term.decode::<Binary<'a>>() {
        let slice = binary.as_slice();
        if slice.is_empty() {
            return Err(Error::BadArg);
        }
        return Ok(Escape {
            bytes: slice.to_vec(),
        });
    }
    Err(Error::BadArg)
}

/// Check if all separators and escape are single-byte (fast path eligible)
fn is_all_single_byte(separators: &Separators, escape: &Escape) -> bool {
    escape.bytes.len() == 1 && separators.patterns.iter().all(|p| p.len() == 1)
}

/// Extract single-byte separator values for fast path
fn single_byte_seps(separators: &Separators) -> Vec<u8> {
    separators.patterns.iter().map(|p| p[0]).collect()
}

use core::{InputTooLarge, Newlines, ScannedBoundaries, Violation};

/// Decode newlines from a Term.
/// Accepts: atom :default → default newlines, or list of binaries → custom newlines
fn decode_newlines<'a>(term: Term<'a>) -> NifResult<Newlines> {
    // Try atom :default
    if let Ok(s) = term.atom_to_string() {
        if s == "default" {
            return Ok(Newlines::default_newlines());
        }
        return Err(Error::BadArg);
    }
    // Try list of binaries
    if let Ok(list) = term.decode::<Vec<Binary<'a>>>() {
        let patterns: Vec<Vec<u8>> = list.iter().map(|b| b.as_slice().to_vec()).collect();
        if patterns.is_empty() || patterns.iter().any(|p| p.is_empty()) {
            return Err(Error::BadArg);
        }
        return Ok(Newlines::custom(patterns));
    }
    Err(Error::BadArg)
}

use resource::{StreamingParserEnum, StreamingParserRef, StreamingParserResource};

fn lock_parser(
    parser: &StreamingParserResource,
) -> NifResult<std::sync::MutexGuard<'_, StreamingParserEnum>> {
    parser
        .inner
        .lock()
        .map_err(|_| Error::RaiseTerm(Box::new(atoms::mutex_poisoned())))
}

use strategy::{
    contains_escape, field_needs_quoting_general, field_needs_quoting_simd,
    field_needs_quoting_simd_multi_sep, parse_csv_boundaries_general,
    parse_csv_boundaries_general_with_newlines, parse_csv_boundaries_multi_sep,
    parse_csv_boundaries_with_config, parse_csv_parallel_boundaries,
    parse_csv_parallel_boundaries_general, parse_csv_parallel_boundaries_general_with_newlines,
    parse_csv_parallel_boundaries_multi_sep, parse_csv_parallel_boundaries_with_config,
    unescape_field_general,
};
use term::{
    boundaries_to_maps_hybrid, boundaries_to_maps_hybrid_general, boundaries_to_term_hybrid,
    boundaries_to_term_hybrid_general, owned_rows_to_term,
};

fn input_too_large_term<'a>(env: Env<'a>) -> Term<'a> {
    (atoms::error(), atoms::input_too_large()).encode(env)
}

fn malformed_csv_term<'a>(env: Env<'a>, violation: Violation) -> Term<'a> {
    let category = match violation {
        Violation::UnexpectedQuote(_) => atoms::unexpected_quote(),
        Violation::TrailingGarbage(_) => atoms::trailing_garbage(),
        Violation::UnterminatedQuote(_) => atoms::unterminated_quote(),
    };
    (
        atoms::error(),
        (atoms::malformed_csv(), category, violation.position()),
    )
        .encode(env)
}

fn scanned_rows<'a>(
    env: Env<'a>,
    scanned: Result<ScannedBoundaries, InputTooLarge>,
    strict: bool,
) -> Result<Vec<Vec<(usize, usize)>>, Term<'a>> {
    let scanned = scanned.map_err(|_| input_too_large_term(env))?;
    if strict {
        if let Some(violation) = scanned.violation {
            return Err(malformed_csv_term(env, violation));
        }
    }
    Ok(scanned.rows)
}

fn scanned_boundaries_to_term<'a>(
    env: Env<'a>,
    input: Binary<'a>,
    scanned: Result<ScannedBoundaries, InputTooLarge>,
    escape: &Escape,
    strict: bool,
) -> Term<'a> {
    match scanned_rows(env, scanned, strict) {
        Ok(rows) => dispatch_boundaries_to_term(env, input, rows, escape),
        Err(error) => error,
    }
}

// ============================================================================
// Allocator Configuration
// ============================================================================

// When memory_tracking is enabled, wrap the allocator to track usage
#[cfg(feature = "memory_tracking")]
mod tracking {
    use std::alloc::{GlobalAlloc, Layout};
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
    pub static PEAK_ALLOCATED: AtomicUsize = AtomicUsize::new(0);

    pub struct TrackingAllocator;

    #[cfg(feature = "mimalloc")]
    static UNDERLYING: mimalloc::MiMalloc = mimalloc::MiMalloc;

    #[cfg(not(feature = "mimalloc"))]
    static UNDERLYING: std::alloc::System = std::alloc::System;

    // SAFETY: `GlobalAlloc` requires that a returned block be valid for the
    // requested layout, that the allocator never unwind, and that `dealloc`
    // receive a pointer and layout previously produced by this same allocator.
    // Every one of those obligations is discharged by `UNDERLYING`, which is
    // either mimalloc or the system allocator; this type only wraps those calls
    // in relaxed atomic counters. The counters are statistics rather than
    // allocator state, so a torn reading skews a profiling number and cannot
    // affect memory safety, and neither counter path can panic or unwind.
    unsafe impl GlobalAlloc for TrackingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: `layout` is forwarded unchanged from our caller, who
            // already owes `GlobalAlloc::alloc` a non-zero-size layout.
            let ptr = unsafe { UNDERLYING.alloc(layout) };
            if !ptr.is_null() {
                let current = ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
                let mut peak = PEAK_ALLOCATED.load(Ordering::Relaxed);
                while current > peak {
                    match PEAK_ALLOCATED.compare_exchange_weak(
                        peak,
                        current,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => break,
                        Err(p) => peak = p,
                    }
                }
            }
            ptr
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            ALLOCATED.fetch_sub(layout.size(), Ordering::Relaxed);
            // SAFETY: `ptr` and `layout` are forwarded unchanged from our
            // caller, who already owes `GlobalAlloc::dealloc` a pointer this
            // allocator produced for exactly this layout. Every such pointer
            // came from `UNDERLYING::alloc` above, so `UNDERLYING` is the
            // correct allocator to hand it back to.
            unsafe { UNDERLYING.dealloc(ptr, layout) }
        }
    }
}

#[cfg(feature = "memory_tracking")]
#[global_allocator]
static GLOBAL: tracking::TrackingAllocator = tracking::TrackingAllocator;

// When memory_tracking is disabled, use mimalloc directly (no overhead)
#[cfg(all(feature = "mimalloc", not(feature = "memory_tracking")))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

// ============================================================================
// Memory Tracking NIFs (only available when memory_tracking feature is enabled)
// ============================================================================

#[cfg(feature = "memory_tracking")]
use std::sync::atomic::Ordering;

/// Get current Rust heap allocation in bytes (requires memory_tracking feature)
#[cfg(feature = "memory_tracking")]
#[rustler::nif]
fn get_rust_memory() -> usize {
    tracking::ALLOCATED.load(Ordering::SeqCst)
}

/// Get peak Rust heap allocation since last reset (requires memory_tracking feature)
#[cfg(feature = "memory_tracking")]
#[rustler::nif]
fn get_rust_memory_peak() -> usize {
    tracking::PEAK_ALLOCATED.load(Ordering::SeqCst)
}

/// Reset memory stats (requires memory_tracking feature)
#[cfg(feature = "memory_tracking")]
#[rustler::nif]
fn reset_rust_memory_stats() -> (usize, usize) {
    let current = tracking::ALLOCATED.load(Ordering::SeqCst);
    let peak = tracking::PEAK_ALLOCATED.swap(current, Ordering::SeqCst);
    (current, peak)
}

/// Stub: returns 0 when memory_tracking is disabled
#[cfg(not(feature = "memory_tracking"))]
#[rustler::nif]
fn get_rust_memory() -> usize {
    0
}

/// Stub: returns 0 when memory_tracking is disabled
#[cfg(not(feature = "memory_tracking"))]
#[rustler::nif]
fn get_rust_memory_peak() -> usize {
    0
}

/// Stub: returns (0, 0) when memory_tracking is disabled
#[cfg(not(feature = "memory_tracking"))]
#[rustler::nif]
fn reset_rust_memory_stats() -> (usize, usize) {
    (0, 0)
}

// ============================================================================
// Strategy A: Batch Parser (legacy name — uses same SIMD path as B/C/F)
// ============================================================================

/// Parse CSV string into list of rows.
///
/// Equivalent to `parse_string_fast`, `parse_string_indexed`, and
/// `parse_string_zero_copy` — all use the same SIMD boundary scan
/// and hybrid sub-binary term builder internally.
///
/// Returns `{:error, :input_too_large}` if the input exceeds 4 GiB.
#[rustler::nif(schedule = "DirtyCpu")]
fn parse_string<'a>(env: Env<'a>, input: Binary<'a>) -> NifResult<Term<'a>> {
    if let Err(t) = guard_input_size(env, input.as_slice()) {
        return Ok(t);
    }
    let scanned = parse_csv_boundaries_with_config(input.as_slice(), b',', b'"');
    let escape = Escape { bytes: vec![b'"'] };
    Ok(scanned_boundaries_to_term(
        env, input, scanned, &escape, true,
    ))
}

/// Parse CSV with configurable separator(s), escape, and newlines.
///
/// Returns `{:error, :input_too_large}` if the input exceeds 4 GiB.
#[rustler::nif(schedule = "DirtyCpu")]
fn parse_string_with_config<'a>(
    env: Env<'a>,
    input: Binary<'a>,
    sep_term: Term<'a>,
    esc_term: Term<'a>,
    newlines_term: Term<'a>,
    strict: bool,
) -> NifResult<Term<'a>> {
    if let Err(t) = guard_input_size(env, input.as_slice()) {
        return Ok(t);
    }
    let separators = decode_separators(sep_term)?;
    let escape = decode_escape(esc_term)?;
    let newlines = decode_newlines(newlines_term)?;
    let scanned = dispatch_boundary_parse(input.as_slice(), &separators, &escape, &newlines);
    Ok(scanned_boundaries_to_term(
        env, input, scanned, &escape, strict,
    ))
}

// ============================================================================
// Strategy B: Batch Parser (legacy name — same SIMD path as A/C/F)
// ============================================================================

/// Parse CSV using SIMD structural scanner.
///
/// Equivalent to `parse_string` — both use the same code path.
///
/// Returns `{:error, :input_too_large}` if the input exceeds 4 GiB.
#[rustler::nif(schedule = "DirtyCpu")]
fn parse_string_fast<'a>(env: Env<'a>, input: Binary<'a>) -> NifResult<Term<'a>> {
    if let Err(t) = guard_input_size(env, input.as_slice()) {
        return Ok(t);
    }
    let scanned = parse_csv_boundaries_with_config(input.as_slice(), b',', b'"');
    let escape = Escape { bytes: vec![b'"'] };
    Ok(scanned_boundaries_to_term(
        env, input, scanned, &escape, true,
    ))
}

/// Parse using SIMD with configurable separator(s), escape, and newlines.
///
/// Returns `{:error, :input_too_large}` if the input exceeds 4 GiB.
#[rustler::nif(schedule = "DirtyCpu")]
fn parse_string_fast_with_config<'a>(
    env: Env<'a>,
    input: Binary<'a>,
    sep_term: Term<'a>,
    esc_term: Term<'a>,
    newlines_term: Term<'a>,
    strict: bool,
) -> NifResult<Term<'a>> {
    if let Err(t) = guard_input_size(env, input.as_slice()) {
        return Ok(t);
    }
    let separators = decode_separators(sep_term)?;
    let escape = decode_escape(esc_term)?;
    let newlines = decode_newlines(newlines_term)?;
    let scanned = dispatch_boundary_parse(input.as_slice(), &separators, &escape, &newlines);
    Ok(scanned_boundaries_to_term(
        env, input, scanned, &escape, strict,
    ))
}

// ============================================================================
// Strategy C: Batch Parser (legacy name — same SIMD path as A/B/F)
// ============================================================================

/// Parse CSV using the SIMD boundary scan.
///
/// Equivalent to `parse_string` — both use the same code path.
///
/// Returns `{:error, :input_too_large}` if the input exceeds 4 GiB.
#[rustler::nif(schedule = "DirtyCpu")]
fn parse_string_indexed<'a>(env: Env<'a>, input: Binary<'a>) -> NifResult<Term<'a>> {
    if let Err(t) = guard_input_size(env, input.as_slice()) {
        return Ok(t);
    }
    let scanned = parse_csv_boundaries_with_config(input.as_slice(), b',', b'"');
    let escape = Escape { bytes: vec![b'"'] };
    Ok(scanned_boundaries_to_term(
        env, input, scanned, &escape, true,
    ))
}

/// Parse using two-phase with configurable separator(s), escape, and newlines.
///
/// Returns `{:error, :input_too_large}` if the input exceeds 4 GiB.
#[rustler::nif(schedule = "DirtyCpu")]
fn parse_string_indexed_with_config<'a>(
    env: Env<'a>,
    input: Binary<'a>,
    sep_term: Term<'a>,
    esc_term: Term<'a>,
    newlines_term: Term<'a>,
    strict: bool,
) -> NifResult<Term<'a>> {
    if let Err(t) = guard_input_size(env, input.as_slice()) {
        return Ok(t);
    }
    let separators = decode_separators(sep_term)?;
    let escape = decode_escape(esc_term)?;
    let newlines = decode_newlines(newlines_term)?;
    let scanned = dispatch_boundary_parse(input.as_slice(), &separators, &escape, &newlines);
    Ok(scanned_boundaries_to_term(
        env, input, scanned, &escape, strict,
    ))
}

// ============================================================================
// Strategy D: Streaming Parser
// ============================================================================

/// Create a new streaming parser with default settings
#[rustler::nif]
fn streaming_new() -> StreamingParserRef {
    ResourceArc::new(StreamingParserResource::new())
}

/// Create a new streaming parser with configurable separator(s), escape, and newlines
#[rustler::nif]
fn streaming_new_with_config<'a>(
    sep_term: Term<'a>,
    esc_term: Term<'a>,
    newlines_term: Term<'a>,
    strict: bool,
) -> NifResult<StreamingParserRef> {
    let separators = decode_separators(sep_term)?;
    let escape = decode_escape(esc_term)?;
    let newlines = decode_newlines(newlines_term)?;

    if !newlines.is_default {
        return Ok(ResourceArc::new(
            StreamingParserResource::with_general_newlines(
                separators.patterns,
                escape.bytes,
                newlines,
                strict,
            ),
        ));
    }

    Ok(if is_all_single_byte(&separators, &escape) {
        let esc = escape.bytes[0];
        let sep_bytes = single_byte_seps(&separators);
        if sep_bytes.len() == 1 {
            ResourceArc::new(StreamingParserResource::with_config(
                sep_bytes[0],
                esc,
                strict,
            ))
        } else {
            ResourceArc::new(StreamingParserResource::with_multi_sep(
                &sep_bytes, esc, strict,
            ))
        }
    } else {
        ResourceArc::new(StreamingParserResource::with_general(
            separators.patterns,
            escape.bytes,
            strict,
        ))
    })
}

/// Feed a chunk of data to the streaming parser
#[rustler::nif(schedule = "DirtyCpu")]
fn streaming_feed<'a>(
    env: Env<'a>,
    parser: StreamingParserRef,
    chunk: Binary,
) -> NifResult<Term<'a>> {
    let strict = parser.strict;
    let mut inner = lock_parser(&parser)?;
    inner
        .feed(chunk.as_slice())
        .map_err(|_| Error::RaiseTerm(Box::new(atoms::buffer_overflow())))?;
    if strict {
        if let Some(violation) = inner.violation() {
            return Ok(malformed_csv_term(env, violation));
        }
    }
    Ok((inner.available_rows(), inner.buffer_size()).encode(env))
}

/// Take up to `max` rows from the streaming parser
#[rustler::nif(schedule = "DirtyCpu")]
fn streaming_next_rows<'a>(
    env: Env<'a>,
    parser: StreamingParserRef,
    max: usize,
) -> NifResult<Term<'a>> {
    let strict = parser.strict;
    let mut inner = lock_parser(&parser)?;
    if strict {
        if let Some(violation) = inner.violation() {
            return Ok(malformed_csv_term(env, violation));
        }
    }
    let rows = inner.take_rows(max);
    Ok(owned_rows_to_term(env, rows))
}

/// Finalize the streaming parser (get remaining partial row)
#[rustler::nif(schedule = "DirtyCpu")]
fn streaming_finalize<'a>(env: Env<'a>, parser: StreamingParserRef) -> NifResult<Term<'a>> {
    let strict = parser.strict;
    let mut inner = lock_parser(&parser)?;
    let rows = inner.finalize();
    if strict {
        if let Some(violation) = inner.violation() {
            return Ok(malformed_csv_term(env, violation));
        }
    }
    Ok(owned_rows_to_term(env, rows))
}

/// Get streaming parser status (available_rows, buffer_size, has_partial)
#[rustler::nif]
fn streaming_status(parser: StreamingParserRef) -> NifResult<(usize, usize, bool)> {
    let inner = lock_parser(&parser)?;
    Ok((
        inner.available_rows(),
        inner.buffer_size(),
        inner.has_partial(),
    ))
}

/// Set the maximum buffer size (in bytes) for the streaming parser.
/// Default is 256 MB. Raises on overflow during `streaming_feed/2`.
#[rustler::nif]
fn streaming_set_max_buffer(parser: StreamingParserRef, max: usize) -> NifResult<Atom> {
    let mut inner = lock_parser(&parser)?;
    inner.set_max_buffer_size(max);
    Ok(atoms::ok())
}

// ============================================================================
// Strategy E: Parallel Parser
// ============================================================================

/// Dispatch to the right parallel boundary parser based on separator/escape/newlines config.
fn dispatch_parallel_boundary_parse(
    bytes: &[u8],
    separators: &Separators,
    escape: &Escape,
    newlines: &Newlines,
) -> Result<ScannedBoundaries, InputTooLarge> {
    if !newlines.is_default {
        return parse_csv_parallel_boundaries_general_with_newlines(
            bytes,
            &separators.patterns,
            &escape.bytes,
            newlines,
        );
    }
    if is_all_single_byte(separators, escape) {
        let esc = escape.bytes[0];
        let sep_bytes = single_byte_seps(separators);
        if sep_bytes.len() == 1 {
            parse_csv_parallel_boundaries_with_config(bytes, sep_bytes[0], esc)
        } else {
            parse_csv_parallel_boundaries_multi_sep(bytes, &sep_bytes, esc)
        }
    } else {
        parse_csv_parallel_boundaries_general(bytes, &separators.patterns, &escape.bytes)
    }
}

/// Parse CSV in parallel using rayon thread pool (boundary-based sub-binaries).
///
/// Returns `{:error, :input_too_large}` if the input exceeds 4 GiB.
#[rustler::nif(schedule = "DirtyCpu")]
fn parse_string_parallel<'a>(env: Env<'a>, input: Binary<'a>) -> NifResult<Term<'a>> {
    if let Err(t) = guard_input_size(env, input.as_slice()) {
        return Ok(t);
    }
    let scanned = parse_csv_parallel_boundaries(input.as_slice());
    let escape = Escape { bytes: vec![b'"'] };
    Ok(scanned_boundaries_to_term(
        env, input, scanned, &escape, true,
    ))
}

/// Parse CSV in parallel with configurable separator(s), escape, and newlines.
///
/// Returns `{:error, :input_too_large}` if the input exceeds 4 GiB.
#[rustler::nif(schedule = "DirtyCpu")]
fn parse_string_parallel_with_config<'a>(
    env: Env<'a>,
    input: Binary<'a>,
    sep_term: Term<'a>,
    esc_term: Term<'a>,
    newlines_term: Term<'a>,
    strict: bool,
) -> NifResult<Term<'a>> {
    if let Err(t) = guard_input_size(env, input.as_slice()) {
        return Ok(t);
    }
    let separators = decode_separators(sep_term)?;
    let escape = decode_escape(esc_term)?;
    let newlines = decode_newlines(newlines_term)?;
    let scanned =
        dispatch_parallel_boundary_parse(input.as_slice(), &separators, &escape, &newlines);
    Ok(scanned_boundaries_to_term(
        env, input, scanned, &escape, strict,
    ))
}

// ============================================================================
// Strategy F: Batch Parser (legacy name — same SIMD path as A/B/C)
// ============================================================================

/// Parse CSV using the SIMD boundary scan.
///
/// Equivalent to `parse_string` — both use the same code path.
///
/// Returns `{:error, :input_too_large}` if the input exceeds 4 GiB.
#[rustler::nif(schedule = "DirtyCpu")]
fn parse_string_zero_copy<'a>(env: Env<'a>, input: Binary<'a>) -> NifResult<Term<'a>> {
    if let Err(t) = guard_input_size(env, input.as_slice()) {
        return Ok(t);
    }
    let bytes = input.as_slice();
    let scanned = parse_csv_boundaries_with_config(bytes, b',', b'"');
    let escape = Escape { bytes: vec![b'"'] };
    Ok(scanned_boundaries_to_term(
        env, input, scanned, &escape, true,
    ))
}

/// Parse CSV using zero-copy with configurable separator(s), escape, and newlines.
///
/// Returns `{:error, :input_too_large}` if the input exceeds 4 GiB.
#[rustler::nif(schedule = "DirtyCpu")]
fn parse_string_zero_copy_with_config<'a>(
    env: Env<'a>,
    input: Binary<'a>,
    sep_term: Term<'a>,
    esc_term: Term<'a>,
    newlines_term: Term<'a>,
    strict: bool,
) -> NifResult<Term<'a>> {
    if let Err(t) = guard_input_size(env, input.as_slice()) {
        return Ok(t);
    }
    let separators = decode_separators(sep_term)?;
    let escape = decode_escape(esc_term)?;
    let newlines = decode_newlines(newlines_term)?;
    let scanned = dispatch_boundary_parse(input.as_slice(), &separators, &escape, &newlines);
    Ok(scanned_boundaries_to_term(
        env, input, scanned, &escape, strict,
    ))
}

// ============================================================================
// Headers-to-Maps NIFs
// ============================================================================

/// Header mode: either auto (first row = keys) or explicit key terms
enum HeaderMode<'a> {
    Auto,
    Explicit(Vec<Term<'a>>),
}

/// Decode header_mode term: atom :true → Auto, list → Explicit(Vec<Term>)
fn decode_header_mode<'a>(header_mode: Term<'a>) -> NifResult<HeaderMode<'a>> {
    // Try atom :true
    if let Ok(s) = header_mode.atom_to_string() {
        if s == "true" {
            return Ok(HeaderMode::Auto);
        }
        return Err(Error::BadArg);
    }
    // Try list of terms (strings or atoms)
    if let Ok(iter) = header_mode.decode::<ListIterator<'a>>() {
        let keys: Vec<Term<'a>> = iter.collect();
        if keys.is_empty() {
            return Err(Error::BadArg);
        }
        return Ok(HeaderMode::Explicit(keys));
    }
    Err(Error::BadArg)
}

/// Dispatch to boundary-returning parser based on separator/escape/newlines config
fn dispatch_boundary_parse(
    bytes: &[u8],
    separators: &Separators,
    escape: &Escape,
    newlines: &Newlines,
) -> Result<ScannedBoundaries, InputTooLarge> {
    if !newlines.is_default {
        return parse_csv_boundaries_general_with_newlines(
            bytes,
            &separators.patterns,
            &escape.bytes,
            newlines,
        );
    }
    if is_all_single_byte(separators, escape) {
        let esc = escape.bytes[0];
        let sep_bytes = single_byte_seps(separators);
        if sep_bytes.len() == 1 {
            parse_csv_boundaries_with_config(bytes, sep_bytes[0], esc)
        } else {
            parse_csv_boundaries_multi_sep(bytes, &sep_bytes, esc)
        }
    } else {
        parse_csv_boundaries_general(bytes, &separators.patterns, &escape.bytes)
    }
}

/// Dispatch between single-byte and general escape for term construction
fn dispatch_boundaries_to_term<'a>(
    env: Env<'a>,
    input: Binary<'a>,
    boundaries: Vec<Vec<(usize, usize)>>,
    escape: &Escape,
) -> Term<'a> {
    if escape.bytes.len() == 1 {
        boundaries_to_term_hybrid(env, input, boundaries, escape.bytes[0])
    } else {
        boundaries_to_term_hybrid_general(env, input, boundaries, &escape.bytes)
    }
}

/// Dispatch between single-byte and general escape for maps construction
fn dispatch_boundaries_to_maps<'a>(
    env: Env<'a>,
    input: Binary<'a>,
    keys: &[Term<'a>],
    boundaries: &[Vec<(usize, usize)>],
    escape: &Escape,
) -> Term<'a> {
    if escape.bytes.len() == 1 {
        boundaries_to_maps_hybrid(env, input, keys, boundaries, escape.bytes[0])
    } else {
        boundaries_to_maps_hybrid_general(env, input, keys, boundaries, &escape.bytes)
    }
}

/// Extract header row from boundaries into key terms
fn boundary_row_to_key_terms<'a>(
    env: Env<'a>,
    input: &Binary<'a>,
    row: &[(usize, usize)],
    escape: &Escape,
) -> Vec<Term<'a>> {
    let input_bytes = input.as_slice();
    if escape.bytes.len() == 1 {
        let esc = escape.bytes[0];
        row.iter()
            .map(|&(start, end)| {
                let field = &input_bytes[start..end];
                let content =
                    if field.len() >= 2 && field[0] == esc && field[field.len() - 1] == esc {
                        let inner = &field[1..field.len() - 1];
                        if inner.contains(&esc) {
                            term::unescape_field(inner, esc)
                        } else {
                            inner.to_vec()
                        }
                    } else {
                        field.to_vec()
                    };
                let mut binary = NewBinary::new(env, content.len());
                binary.as_mut_slice().copy_from_slice(&content);
                let t: Term = binary.into();
                t
            })
            .collect()
    } else {
        let esc = &escape.bytes;
        let esc_len = esc.len();
        row.iter()
            .map(|&(start, end)| {
                let field = &input_bytes[start..end];
                let content = if field.len() >= 2 * esc_len
                    && field[..esc_len] == *esc.as_slice()
                    && field[field.len() - esc_len..] == *esc.as_slice()
                {
                    let inner = &field[esc_len..field.len() - esc_len];
                    if contains_escape(inner, esc) {
                        unescape_field_general(inner, esc)
                    } else {
                        inner.to_vec()
                    }
                } else {
                    field.to_vec()
                };
                let mut binary = NewBinary::new(env, content.len());
                binary.as_mut_slice().copy_from_slice(&content);
                let t: Term = binary.into();
                t
            })
            .collect()
    }
}

/// Parse CSV and return list of maps. Dispatches to strategy internally.
///
/// Returns `{:error, :input_too_large}` if the input exceeds 4 GiB.
#[expect(
    clippy::too_many_arguments,
    reason = "the NIF signature mirrors the stable Elixir argument list"
)]
#[rustler::nif(schedule = "DirtyCpu")]
fn parse_to_maps<'a>(
    env: Env<'a>,
    input: Binary<'a>,
    sep_term: Term<'a>,
    esc_term: Term<'a>,
    newlines_term: Term<'a>,
    strategy: Term<'a>,
    header_mode_term: Term<'a>,
    skip_first: bool,
    strict: bool,
) -> NifResult<Term<'a>> {
    if let Err(t) = guard_input_size(env, input.as_slice()) {
        return Ok(t);
    }
    let separators = decode_separators(sep_term)?;
    let escape = decode_escape(esc_term)?;
    let newlines = decode_newlines(newlines_term)?;
    let header_mode = decode_header_mode(header_mode_term)?;
    let strategy_str = strategy.atom_to_string().map_err(|_| Error::BadArg)?;
    let bytes = input.as_slice();

    match strategy_str.as_str() {
        "basic" | "simd" | "indexed" | "zero_copy" => {
            let all_boundaries = match scanned_rows(
                env,
                dispatch_boundary_parse(bytes, &separators, &escape, &newlines),
                strict,
            ) {
                Ok(rows) => rows,
                Err(error) => return Ok(error),
            };
            if all_boundaries.is_empty() {
                return Ok(Term::list_new_empty(env));
            }

            match header_mode {
                HeaderMode::Auto => {
                    let key_terms =
                        boundary_row_to_key_terms(env, &input, &all_boundaries[0], &escape);
                    Ok(dispatch_boundaries_to_maps(
                        env,
                        input,
                        &key_terms,
                        &all_boundaries[1..],
                        &escape,
                    ))
                }
                HeaderMode::Explicit(key_terms) => {
                    let start = if skip_first { 1 } else { 0 };
                    Ok(dispatch_boundaries_to_maps(
                        env,
                        input,
                        &key_terms,
                        &all_boundaries[start..],
                        &escape,
                    ))
                }
            }
        }
        _ => Err(Error::BadArg),
    }
}

/// Parallel variant for parse_to_maps on dirty CPU scheduler.
///
/// Returns `{:error, :input_too_large}` if the input exceeds 4 GiB.
#[expect(
    clippy::too_many_arguments,
    reason = "the NIF signature mirrors the stable Elixir argument list"
)]
#[rustler::nif(schedule = "DirtyCpu")]
fn parse_to_maps_parallel<'a>(
    env: Env<'a>,
    input: Binary<'a>,
    sep_term: Term<'a>,
    esc_term: Term<'a>,
    newlines_term: Term<'a>,
    header_mode_term: Term<'a>,
    skip_first: bool,
    strict: bool,
) -> NifResult<Term<'a>> {
    if let Err(t) = guard_input_size(env, input.as_slice()) {
        return Ok(t);
    }
    let separators = decode_separators(sep_term)?;
    let escape = decode_escape(esc_term)?;
    let newlines = decode_newlines(newlines_term)?;
    let header_mode = decode_header_mode(header_mode_term)?;
    let bytes = input.as_slice();

    let all_boundaries = match scanned_rows(
        env,
        dispatch_parallel_boundary_parse(bytes, &separators, &escape, &newlines),
        strict,
    ) {
        Ok(rows) => rows,
        Err(error) => return Ok(error),
    };

    if all_boundaries.is_empty() {
        return Ok(Term::list_new_empty(env));
    }

    match header_mode {
        HeaderMode::Auto => {
            let key_terms = boundary_row_to_key_terms(env, &input, &all_boundaries[0], &escape);
            Ok(dispatch_boundaries_to_maps(
                env,
                input,
                &key_terms,
                &all_boundaries[1..],
                &escape,
            ))
        }
        HeaderMode::Explicit(key_terms) => {
            let start = if skip_first { 1 } else { 0 };
            Ok(dispatch_boundaries_to_maps(
                env,
                input,
                &key_terms,
                &all_boundaries[start..],
                &escape,
            ))
        }
    }
}

// ============================================================================
// Encoding NIFs
// ============================================================================

/// Decode line_separator from a Term. Accepts binary or atom :default → "\n"
fn decode_line_separator<'a>(term: Term<'a>) -> NifResult<Vec<u8>> {
    if let Ok(s) = term.atom_to_string() {
        if s == "default" {
            return Ok(b"\n".to_vec());
        }
        return Err(Error::BadArg);
    }
    if let Ok(binary) = term.decode::<Binary<'a>>() {
        return Ok(binary.as_slice().to_vec());
    }
    Err(Error::BadArg)
}

// ============================================================================
// Formula Escaping + Encoding Target
// ============================================================================

use strategy::encoding::EncodingTarget;

/// Configuration for formula injection prevention.
/// Each rule maps a trigger byte (first byte of a field) to a replacement prefix.
struct FormulaConfig {
    rules: Vec<(u8, Vec<u8>)>,
}

impl FormulaConfig {
    fn none() -> Self {
        FormulaConfig { rules: Vec::new() }
    }

    fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Check if a field's first byte triggers a formula rule.
    /// Returns the replacement prefix if triggered, None otherwise.
    #[inline(always)]
    fn check(&self, field: &[u8]) -> Option<&[u8]> {
        if field.is_empty() {
            return None;
        }
        let first = field[0];
        for (trigger, replacement) in &self.rules {
            if first == *trigger {
                return Some(replacement);
            }
        }
        None
    }
}

/// Decode formula config from an Elixir term.
/// - `nil` atom → FormulaConfig::none()
/// - list of `{byte, binary}` tuples → FormulaConfig with rules
fn decode_formula_config<'a>(term: Term<'a>) -> NifResult<FormulaConfig> {
    // Check for nil atom
    if let Ok(s) = term.atom_to_string() {
        if s == "nil" {
            return Ok(FormulaConfig::none());
        }
        return Err(Error::BadArg);
    }

    // Decode as list of {integer, binary} tuples
    let list: ListIterator<'a> = term.decode().map_err(|_| Error::BadArg)?;
    let mut rules = Vec::new();
    for item in list {
        let tuple = rustler::types::tuple::get_tuple(item).map_err(|_| Error::BadArg)?;
        if tuple.len() != 2 {
            return Err(Error::BadArg);
        }
        let trigger: u8 = tuple[0].decode().map_err(|_| Error::BadArg)?;
        let replacement: Binary<'a> = tuple[1].decode().map_err(|_| Error::BadArg)?;
        rules.push((trigger, replacement.as_slice().to_vec()));
    }
    Ok(FormulaConfig { rules })
}

/// Decode encoding target from an Elixir term.
/// - `:utf8` → EncodingTarget::Utf8
/// - `:latin1` → EncodingTarget::Latin1
/// - `{:utf16, :little}` → EncodingTarget::Utf16Le
/// - `{:utf16, :big}` → EncodingTarget::Utf16Be
/// - `{:utf32, :little}` → EncodingTarget::Utf32Le
/// - `{:utf32, :big}` → EncodingTarget::Utf32Be
fn decode_encoding_target(term: Term) -> NifResult<EncodingTarget> {
    // Try atom
    if let Ok(s) = term.atom_to_string() {
        return match s.as_str() {
            "utf8" => Ok(EncodingTarget::Utf8),
            "latin1" => Ok(EncodingTarget::Latin1),
            _ => Err(Error::BadArg),
        };
    }

    // Try tuple {atom, atom}
    let tuple = rustler::types::tuple::get_tuple(term).map_err(|_| Error::BadArg)?;
    if tuple.len() != 2 {
        return Err(Error::BadArg);
    }
    let enc = tuple[0].atom_to_string().map_err(|_| Error::BadArg)?;
    let endian = tuple[1].atom_to_string().map_err(|_| Error::BadArg)?;

    match (enc.as_str(), endian.as_str()) {
        ("utf16", "little") => Ok(EncodingTarget::Utf16Le),
        ("utf16", "big") => Ok(EncodingTarget::Utf16Be),
        ("utf32", "little") => Ok(EncodingTarget::Utf32Le),
        ("utf32", "big") => Ok(EncodingTarget::Utf32Be),
        _ => Err(Error::BadArg),
    }
}

/// Decode reserved characters from an Erlang list of single-byte binaries.
/// Returns a Vec<u8> of bytes that should trigger quoting.
fn decode_reserved<'a>(term: Term<'a>) -> NifResult<Vec<u8>> {
    let list: ListIterator<'a> = term.decode().map_err(|_| Error::BadArg)?;
    let mut reserved = Vec::new();
    for item in list {
        let bin: Binary<'a> = item.decode().map_err(|_| Error::BadArg)?;
        let bytes = bin.as_slice();
        // Each reserved entry should be a single byte
        if bytes.len() == 1 {
            reserved.push(bytes[0]);
        } else {
            // Multi-byte reserved patterns: add all bytes individually
            reserved.extend_from_slice(bytes);
        }
    }
    Ok(reserved)
}

/// Post-processing mode for encoding. Determines what extra work is needed
/// beyond basic CSV field quoting.
enum PostProcess {
    /// UTF-8, no formula escaping (current fast path — identical behavior)
    None,
    /// UTF-8 + formula escaping
    FormulaOnly(FormulaConfig),
    /// Non-UTF-8, no formula escaping
    EncodingOnly(EncodingTarget),
    /// Both formula escaping and non-UTF-8 encoding
    Full(FormulaConfig, EncodingTarget),
}

impl PostProcess {
    fn from(formula: FormulaConfig, encoding: EncodingTarget) -> Self {
        match (formula.is_empty(), encoding) {
            (true, EncodingTarget::Utf8) => PostProcess::None,
            (false, EncodingTarget::Utf8) => PostProcess::FormulaOnly(formula),
            (true, enc) => PostProcess::EncodingOnly(enc),
            (false, enc) => PostProcess::Full(formula, enc),
        }
    }
}

fn new_binary_term<'a>(env: Env<'a>, bytes: &[u8]) -> Term<'a> {
    let mut binary = NewBinary::new(env, bytes.len());
    binary.as_mut_slice().copy_from_slice(bytes);
    binary.into()
}

/// Encode rows to CSV, returning one binary per row as iodata.
///
/// Each row is encoded into a reusable buffer and copied into one NewBinary.
/// This preserves NimbleCSV's top-level row shape without constructing a BEAM
/// term for every field and separator.
///
/// Formula escaping and non-UTF-8 encoding are handled via the PostProcess enum:
/// - None: UTF-8, no formula (fast path)
/// - FormulaOnly: prefix triggered fields with replacement bytes
/// - EncodingOnly: convert all output to target encoding
/// - Full: both formula escaping and encoding conversion
#[allow(clippy::too_many_arguments)]
#[rustler::nif(schedule = "DirtyCpu")]
fn encode_string<'a>(
    env: Env<'a>,
    rows_term: Term<'a>,
    sep_term: Term<'a>,
    esc_term: Term<'a>,
    line_sep_term: Term<'a>,
    formula_term: Term<'a>,
    encoding_term: Term<'a>,
    reserved_term: Term<'a>,
) -> NifResult<Term<'a>> {
    let separators = decode_separators(sep_term)?;
    let escape = decode_escape(esc_term)?;
    let line_separator = decode_line_separator(line_sep_term)?;
    let formula = decode_formula_config(formula_term)?;
    let encoding = decode_encoding_target(encoding_term)?;
    let reserved = decode_reserved(reserved_term)?;
    let post = PostProcess::from(formula, encoding);

    let rows_iter: ListIterator<'a> = rows_term.decode().map_err(|_| Error::BadArg)?;

    match post {
        PostProcess::None => encode_string_none(
            env,
            rows_iter,
            &separators,
            &escape,
            &line_separator,
            &reserved,
        ),
        PostProcess::FormulaOnly(formula) => encode_string_formula(
            env,
            rows_iter,
            &separators,
            &escape,
            &line_separator,
            &formula,
            &reserved,
        ),
        PostProcess::EncodingOnly(target) => encode_string_encoding(
            env,
            rows_iter,
            &separators,
            &escape,
            &line_separator,
            target,
            &reserved,
        ),
        PostProcess::Full(formula, target) => encode_string_full(
            env,
            rows_iter,
            &separators,
            &escape,
            &line_separator,
            &formula,
            target,
            &reserved,
        ),
    }
}

/// PostProcess::None — UTF-8, no formula.
/// Reusable Vec<u8> row buffer → one NewBinary per row.
fn encode_string_none<'a>(
    env: Env<'a>,
    rows_iter: ListIterator<'a>,
    separators: &Separators,
    escape: &Escape,
    line_separator: &[u8],
    reserved: &[u8],
) -> NifResult<Term<'a>> {
    use strategy::encode::{write_quoted_field, write_quoted_field_general};

    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut row_terms = Vec::new();

    if is_all_single_byte(separators, escape) {
        let esc = escape.bytes[0];
        let sep_bytes = single_byte_seps(separators);
        let dump_sep = sep_bytes[0];
        let multi_sep = sep_bytes.len() > 1;

        for row_term in rows_iter {
            let field_iter: ListIterator<'a> = row_term.decode().map_err(|_| Error::BadArg)?;
            let mut first = true;
            for field_term in field_iter {
                if !first {
                    buf.push(dump_sep);
                }
                first = false;
                let field_bin = match decode_field_binary(env, field_term) {
                    Ok(bin) => bin,
                    Err(term) => return Ok(term),
                };
                let field_bytes = field_bin.as_slice();
                let needs_quoting = if multi_sep {
                    field_needs_quoting_simd_multi_sep(field_bytes, &sep_bytes, esc, reserved)
                } else {
                    field_needs_quoting_simd(field_bytes, dump_sep, esc, reserved)
                };
                if needs_quoting {
                    write_quoted_field(&mut buf, field_bytes, esc);
                } else {
                    buf.extend_from_slice(field_bytes);
                }
            }
            buf.extend_from_slice(line_separator);
            row_terms.push(new_binary_term(env, &buf));
            buf.clear();
        }
    } else {
        let sep_pattern = &separators.patterns[0];
        let esc_pattern = &escape.bytes;

        for row_term in rows_iter {
            let field_iter: ListIterator<'a> = row_term.decode().map_err(|_| Error::BadArg)?;
            let mut first = true;
            for field_term in field_iter {
                if !first {
                    buf.extend_from_slice(sep_pattern);
                }
                first = false;
                let field_bin = match decode_field_binary(env, field_term) {
                    Ok(bin) => bin,
                    Err(term) => return Ok(term),
                };
                let field_bytes = field_bin.as_slice();
                if field_needs_quoting_general(field_bytes, sep_pattern, esc_pattern, reserved) {
                    write_quoted_field_general(&mut buf, field_bytes, esc_pattern);
                } else {
                    buf.extend_from_slice(field_bytes);
                }
            }
            buf.extend_from_slice(line_separator);
            row_terms.push(new_binary_term(env, &buf));
            buf.clear();
        }
    }

    Ok(row_terms.encode(env))
}

/// PostProcess::FormulaOnly — UTF-8 + formula escaping.
/// Reusable Vec<u8> row buffer → one NewBinary per row.
///
/// NimbleCSV semantics:
/// - Formula triggered + clean field → prefix ++ field (no quotes)
/// - Formula triggered + dirty field → esc ++ prefix ++ escaped_inner ++ esc
fn encode_string_formula<'a>(
    env: Env<'a>,
    rows_iter: ListIterator<'a>,
    separators: &Separators,
    escape: &Escape,
    line_separator: &[u8],
    formula: &FormulaConfig,
    reserved: &[u8],
) -> NifResult<Term<'a>> {
    use strategy::encode::{
        write_quoted_field, write_quoted_field_general, write_quoted_field_inner,
        write_quoted_field_inner_general,
    };

    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut row_terms = Vec::new();

    if is_all_single_byte(separators, escape) {
        let esc = escape.bytes[0];
        let sep_bytes = single_byte_seps(separators);
        let dump_sep = sep_bytes[0];
        let multi_sep = sep_bytes.len() > 1;

        for row_term in rows_iter {
            let field_iter: ListIterator<'a> = row_term.decode().map_err(|_| Error::BadArg)?;
            let mut first = true;
            for field_term in field_iter {
                if !first {
                    buf.push(dump_sep);
                }
                first = false;
                let field_bin = match decode_field_binary(env, field_term) {
                    Ok(bin) => bin,
                    Err(term) => return Ok(term),
                };
                let field_bytes = field_bin.as_slice();
                let needs_quoting = if multi_sep {
                    field_needs_quoting_simd_multi_sep(field_bytes, &sep_bytes, esc, reserved)
                } else {
                    field_needs_quoting_simd(field_bytes, dump_sep, esc, reserved)
                };

                if let Some(prefix) = formula.check(field_bytes) {
                    if needs_quoting {
                        buf.push(esc);
                        buf.extend_from_slice(prefix);
                        write_quoted_field_inner(&mut buf, field_bytes, esc);
                        buf.push(esc);
                    } else {
                        buf.extend_from_slice(prefix);
                        buf.extend_from_slice(field_bytes);
                    }
                } else if needs_quoting {
                    write_quoted_field(&mut buf, field_bytes, esc);
                } else {
                    buf.extend_from_slice(field_bytes);
                }
            }
            buf.extend_from_slice(line_separator);
            row_terms.push(new_binary_term(env, &buf));
            buf.clear();
        }
    } else {
        let sep_pattern = &separators.patterns[0];
        let esc_pattern = &escape.bytes;

        for row_term in rows_iter {
            let field_iter: ListIterator<'a> = row_term.decode().map_err(|_| Error::BadArg)?;
            let mut first = true;
            for field_term in field_iter {
                if !first {
                    buf.extend_from_slice(sep_pattern);
                }
                first = false;
                let field_bin = match decode_field_binary(env, field_term) {
                    Ok(bin) => bin,
                    Err(term) => return Ok(term),
                };
                let field_bytes = field_bin.as_slice();
                let needs_quoting =
                    field_needs_quoting_general(field_bytes, sep_pattern, esc_pattern, reserved);

                if let Some(prefix) = formula.check(field_bytes) {
                    if needs_quoting {
                        buf.extend_from_slice(esc_pattern);
                        buf.extend_from_slice(prefix);
                        write_quoted_field_inner_general(&mut buf, field_bytes, esc_pattern);
                        buf.extend_from_slice(esc_pattern);
                    } else {
                        buf.extend_from_slice(prefix);
                        buf.extend_from_slice(field_bytes);
                    }
                } else if needs_quoting {
                    write_quoted_field_general(&mut buf, field_bytes, esc_pattern);
                } else {
                    buf.extend_from_slice(field_bytes);
                }
            }
            buf.extend_from_slice(line_separator);
            row_terms.push(new_binary_term(env, &buf));
            buf.clear();
        }
    }

    Ok(row_terms.encode(env))
}

/// PostProcess::EncodingOnly — non-UTF-8, no formula.
/// Reusable Vec<u8> row buffer with scratch space → one NewBinary per row.
fn encode_string_encoding<'a>(
    env: Env<'a>,
    rows_iter: ListIterator<'a>,
    separators: &Separators,
    escape: &Escape,
    line_separator: &[u8],
    target: EncodingTarget,
    reserved: &[u8],
) -> NifResult<Term<'a>> {
    use strategy::encode::{write_quoted_field, write_quoted_field_general};
    use strategy::encoding::encode_utf8_extend;

    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut scratch: Vec<u8> = Vec::with_capacity(256);
    let mut row_terms = Vec::new();

    // Pre-encode separator and line_separator into buf-compatible bytes
    let mut sep_encoded: Vec<u8> = Vec::new();
    encode_utf8_extend(&mut sep_encoded, &separators.patterns[0], target);
    let mut ls_encoded: Vec<u8> = Vec::new();
    encode_utf8_extend(&mut ls_encoded, line_separator, target);

    if is_all_single_byte(separators, escape) {
        let esc = escape.bytes[0];
        let sep_bytes = single_byte_seps(separators);
        let dump_sep = sep_bytes[0];
        let multi_sep = sep_bytes.len() > 1;

        for row_term in rows_iter {
            let field_iter: ListIterator<'a> = row_term.decode().map_err(|_| Error::BadArg)?;
            let mut first = true;
            for field_term in field_iter {
                if !first {
                    buf.extend_from_slice(&sep_encoded);
                }
                first = false;
                let field_bin = match decode_field_binary(env, field_term) {
                    Ok(bin) => bin,
                    Err(term) => return Ok(term),
                };
                let field_bytes = field_bin.as_slice();
                let needs_quoting = if multi_sep {
                    field_needs_quoting_simd_multi_sep(field_bytes, &sep_bytes, esc, reserved)
                } else {
                    field_needs_quoting_simd(field_bytes, dump_sep, esc, reserved)
                };

                let utf8_src: &[u8] = if needs_quoting {
                    scratch.clear();
                    write_quoted_field(&mut scratch, field_bytes, esc);
                    &scratch
                } else {
                    field_bytes
                };
                encode_utf8_extend(&mut buf, utf8_src, target);
            }
            buf.extend_from_slice(&ls_encoded);
            row_terms.push(new_binary_term(env, &buf));
            buf.clear();
        }
    } else {
        let sep_pattern = &separators.patterns[0];
        let esc_pattern = &escape.bytes;

        for row_term in rows_iter {
            let field_iter: ListIterator<'a> = row_term.decode().map_err(|_| Error::BadArg)?;
            let mut first = true;
            for field_term in field_iter {
                if !first {
                    buf.extend_from_slice(&sep_encoded);
                }
                first = false;
                let field_bin = match decode_field_binary(env, field_term) {
                    Ok(bin) => bin,
                    Err(term) => return Ok(term),
                };
                let field_bytes = field_bin.as_slice();

                let utf8_src: &[u8] =
                    if field_needs_quoting_general(field_bytes, sep_pattern, esc_pattern, reserved)
                    {
                        scratch.clear();
                        write_quoted_field_general(&mut scratch, field_bytes, esc_pattern);
                        &scratch
                    } else {
                        field_bytes
                    };
                encode_utf8_extend(&mut buf, utf8_src, target);
            }
            buf.extend_from_slice(&ls_encoded);
            row_terms.push(new_binary_term(env, &buf));
            buf.clear();
        }
    }

    Ok(row_terms.encode(env))
}

/// PostProcess::Full — both formula escaping and non-UTF-8 encoding.
/// Reusable Vec<u8> row buffer with scratch space → one NewBinary per row.
#[allow(clippy::too_many_arguments)]
fn encode_string_full<'a>(
    env: Env<'a>,
    rows_iter: ListIterator<'a>,
    separators: &Separators,
    escape: &Escape,
    line_separator: &[u8],
    formula: &FormulaConfig,
    target: EncodingTarget,
    reserved: &[u8],
) -> NifResult<Term<'a>> {
    use strategy::encode::{
        write_quoted_field, write_quoted_field_general, write_quoted_field_inner,
        write_quoted_field_inner_general,
    };
    use strategy::encoding::encode_utf8_extend;

    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut scratch: Vec<u8> = Vec::with_capacity(256);
    let mut row_terms = Vec::new();

    let mut sep_encoded: Vec<u8> = Vec::new();
    encode_utf8_extend(&mut sep_encoded, &separators.patterns[0], target);
    let mut ls_encoded: Vec<u8> = Vec::new();
    encode_utf8_extend(&mut ls_encoded, line_separator, target);

    if is_all_single_byte(separators, escape) {
        let esc = escape.bytes[0];
        let sep_bytes = single_byte_seps(separators);
        let dump_sep = sep_bytes[0];
        let multi_sep = sep_bytes.len() > 1;

        for row_term in rows_iter {
            let field_iter: ListIterator<'a> = row_term.decode().map_err(|_| Error::BadArg)?;
            let mut first = true;
            for field_term in field_iter {
                if !first {
                    buf.extend_from_slice(&sep_encoded);
                }
                first = false;
                let field_bin = match decode_field_binary(env, field_term) {
                    Ok(bin) => bin,
                    Err(term) => return Ok(term),
                };
                let field_bytes = field_bin.as_slice();
                let needs_quoting = if multi_sep {
                    field_needs_quoting_simd_multi_sep(field_bytes, &sep_bytes, esc, reserved)
                } else {
                    field_needs_quoting_simd(field_bytes, dump_sep, esc, reserved)
                };

                if let Some(prefix) = formula.check(field_bytes) {
                    if needs_quoting {
                        // Dirty + formula: encoded_esc ++ raw_prefix ++ encoded_inner ++ encoded_esc
                        encode_utf8_extend(&mut buf, &[esc], target);
                        buf.extend_from_slice(prefix);
                        scratch.clear();
                        write_quoted_field_inner(&mut scratch, field_bytes, esc);
                        encode_utf8_extend(&mut buf, &scratch, target);
                        encode_utf8_extend(&mut buf, &[esc], target);
                    } else {
                        // Clean + formula: raw_prefix ++ encoded_field
                        buf.extend_from_slice(prefix);
                        encode_utf8_extend(&mut buf, field_bytes, target);
                    }
                } else {
                    let utf8_src: &[u8] = if needs_quoting {
                        scratch.clear();
                        write_quoted_field(&mut scratch, field_bytes, esc);
                        &scratch
                    } else {
                        field_bytes
                    };
                    encode_utf8_extend(&mut buf, utf8_src, target);
                }
            }
            buf.extend_from_slice(&ls_encoded);
            row_terms.push(new_binary_term(env, &buf));
            buf.clear();
        }
    } else {
        let sep_pattern = &separators.patterns[0];
        let esc_pattern = &escape.bytes;

        for row_term in rows_iter {
            let field_iter: ListIterator<'a> = row_term.decode().map_err(|_| Error::BadArg)?;
            let mut first = true;
            for field_term in field_iter {
                if !first {
                    buf.extend_from_slice(&sep_encoded);
                }
                first = false;
                let field_bin = match decode_field_binary(env, field_term) {
                    Ok(bin) => bin,
                    Err(term) => return Ok(term),
                };
                let field_bytes = field_bin.as_slice();
                let needs_quoting =
                    field_needs_quoting_general(field_bytes, sep_pattern, esc_pattern, reserved);

                if let Some(prefix) = formula.check(field_bytes) {
                    if needs_quoting {
                        encode_utf8_extend(&mut buf, esc_pattern, target);
                        buf.extend_from_slice(prefix);
                        scratch.clear();
                        write_quoted_field_inner_general(&mut scratch, field_bytes, esc_pattern);
                        encode_utf8_extend(&mut buf, &scratch, target);
                        encode_utf8_extend(&mut buf, esc_pattern, target);
                    } else {
                        buf.extend_from_slice(prefix);
                        encode_utf8_extend(&mut buf, field_bytes, target);
                    }
                } else {
                    let utf8_src: &[u8] = if needs_quoting {
                        scratch.clear();
                        write_quoted_field_general(&mut scratch, field_bytes, esc_pattern);
                        &scratch
                    } else {
                        field_bytes
                    };
                    encode_utf8_extend(&mut buf, utf8_src, target);
                }
            }
            buf.extend_from_slice(&ls_encoded);
            row_terms.push(new_binary_term(env, &buf));
            buf.clear();
        }
    }

    Ok(row_terms.encode(env))
}

/// Encode rows to CSV in parallel using rayon, returning iodata (list of binaries).
///
/// Architecture:
/// - Phase 1 (main thread): Walk Erlang lists, copy field bytes into owned Vecs
/// - Phase 2 (rayon): Parallel CSV encoding — each row produces a flat Vec<u8>
/// - Phase 3 (main thread): Wrap each row as a NewBinary, preserving row order
///
/// Only supports single-byte separator/escape (the common case for parallel workloads).
/// Falls back to BadArg for multi-byte separator/escape — callers should use
/// encode_string for those configurations.
#[allow(clippy::too_many_arguments)]
#[rustler::nif(schedule = "DirtyCpu")]
fn encode_string_parallel<'a>(
    env: Env<'a>,
    rows_term: Term<'a>,
    sep_term: Term<'a>,
    esc_term: Term<'a>,
    line_sep_term: Term<'a>,
    formula_term: Term<'a>,
    encoding_term: Term<'a>,
    reserved_term: Term<'a>,
) -> NifResult<Term<'a>> {
    use rayon::prelude::*;
    use strategy::encode::{
        field_needs_quoting_simd, write_quoted_field, write_quoted_field_inner,
    };
    use strategy::encoding::encode_utf8_to_target;
    use strategy::parallel::run_parallel;

    let separators = decode_separators(sep_term)?;
    let escape = decode_escape(esc_term)?;
    let line_separator = decode_line_separator(line_sep_term)?;
    let formula = decode_formula_config(formula_term)?;
    let encoding = decode_encoding_target(encoding_term)?;
    let reserved = decode_reserved(reserved_term)?;

    // Only support single-byte sep/esc for the parallel path
    if !is_all_single_byte(&separators, &escape) {
        return Err(Error::BadArg);
    }

    let esc = escape.bytes[0];
    let dump_sep = single_byte_seps(&separators)[0];
    let has_formula = !formula.is_empty();
    let needs_encoding = encoding != EncodingTarget::Utf8;

    // Clone formula rules into an Arc for sharing across rayon threads
    let formula_rules: Vec<(u8, Vec<u8>)> = formula.rules;

    // Phase 1: Extract all field data into owned Rust structures (main thread)
    let rows_iter: ListIterator<'a> = rows_term.decode().map_err(|_| Error::BadArg)?;
    let mut all_rows: Vec<Vec<Vec<u8>>> = Vec::new();

    for row_term in rows_iter {
        let field_iter: ListIterator<'a> = row_term.decode().map_err(|_| Error::BadArg)?;
        let mut row_fields: Vec<Vec<u8>> = Vec::new();
        for field_term in field_iter {
            let field_bin = match decode_field_binary(env, field_term) {
                Ok(bin) => bin,
                Err(term) => return Ok(term),
            };
            row_fields.push(field_bin.as_slice().to_vec());
        }
        all_rows.push(row_fields);
    }

    if all_rows.is_empty() {
        return Ok(Term::list_new_empty(env));
    }

    // Pre-encode separator and line_separator for non-UTF-8
    let sep_bytes_encoded: Vec<u8> = if needs_encoding {
        encode_utf8_to_target(&[dump_sep], encoding)
    } else {
        vec![dump_sep]
    };
    let ls_encoded: Vec<u8> = if needs_encoding {
        encode_utf8_to_target(&line_separator, encoding)
    } else {
        line_separator.clone()
    };

    let encoded_rows: Vec<Vec<u8>> = run_parallel(|| {
        all_rows
            .par_iter()
            .map(|row| {
                let mut out = Vec::with_capacity(128);
                for (i, field) in row.iter().enumerate() {
                    if i > 0 {
                        if needs_encoding {
                            out.extend_from_slice(&sep_bytes_encoded);
                        } else {
                            out.push(dump_sep);
                        }
                    }

                    // Check formula trigger
                    let formula_prefix: Option<&[u8]> = if has_formula && !field.is_empty() {
                        let first = field[0];
                        formula_rules
                            .iter()
                            .find(|(trigger, _)| *trigger == first)
                            .map(|(_, replacement)| replacement.as_slice())
                    } else {
                        None
                    };

                    let needs_quoting = field_needs_quoting_simd(field, dump_sep, esc, &reserved);

                    if let Some(prefix) = formula_prefix {
                        if needs_quoting {
                            if needs_encoding {
                                // [encoded_esc, raw_prefix, encoded_inner, encoded_esc]
                                let encoded_esc = encode_utf8_to_target(&[esc], encoding);
                                let mut inner_buf = Vec::with_capacity(field.len() + 8);
                                write_quoted_field_inner(&mut inner_buf, field, esc);
                                let encoded_inner = encode_utf8_to_target(&inner_buf, encoding);
                                out.extend_from_slice(&encoded_esc);
                                out.extend_from_slice(prefix);
                                out.extend_from_slice(&encoded_inner);
                                out.extend_from_slice(&encoded_esc);
                            } else {
                                // FormulaOnly: prefix inside quotes
                                out.push(esc);
                                out.extend_from_slice(prefix);
                                write_quoted_field_inner(&mut out, field, esc);
                                out.push(esc);
                            }
                        } else if needs_encoding {
                            // Clean + formula + encoding: prefix raw, field encoded
                            out.extend_from_slice(prefix);
                            let encoded = encode_utf8_to_target(field, encoding);
                            out.extend_from_slice(&encoded);
                        } else {
                            // Clean + formula, no encoding
                            out.extend_from_slice(prefix);
                            out.extend_from_slice(field);
                        }
                        continue;
                    }

                    if needs_quoting {
                        let mut buf = Vec::with_capacity(field.len() + 8);
                        write_quoted_field(&mut buf, field, esc);
                        if needs_encoding {
                            let encoded = encode_utf8_to_target(&buf, encoding);
                            out.extend_from_slice(&encoded);
                        } else {
                            out.extend_from_slice(&buf);
                        }
                    } else if needs_encoding {
                        let encoded = encode_utf8_to_target(field, encoding);
                        out.extend_from_slice(&encoded);
                    } else {
                        out.extend_from_slice(field);
                    }
                }
                out.extend_from_slice(&ls_encoded);
                out
            })
            .collect()
    });

    let row_terms: Vec<Term<'a>> = encoded_rows
        .into_iter()
        .map(|row| new_binary_term(env, &row))
        .collect();

    Ok(row_terms.encode(env))
}

// ============================================================================
// NIF Initialization
// ============================================================================

rustler::init!("Elixir.RustyCSV.Native");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exceeds_batch_limit_at_boundary() {
        // Exactly at limit — must pass
        assert!(!exceeds_batch_limit(u32::MAX as usize));
        // One byte over — must reject
        assert!(exceeds_batch_limit(u32::MAX as usize + 1));
    }

    #[test]
    fn exceeds_batch_limit_zero_and_small() {
        assert!(!exceeds_batch_limit(0));
        assert!(!exceeds_batch_limit(1));
        assert!(!exceeds_batch_limit(1024 * 1024));
    }

    #[test]
    fn max_batch_input_size_equals_u32_max() {
        assert_eq!(core::MAX_INPUT_SIZE, u32::MAX as usize);
    }
}
