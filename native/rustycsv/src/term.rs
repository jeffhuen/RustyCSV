//! Shared term building utilities for converting Rust data to Elixir terms

use rustler::{Binary, Env, NewBinary, Term};

/// Convert a list of byte-like fields to an Elixir cons-list of binaries.
/// Works with any iterator of `AsRef<[u8]>` items (Vec<u8>, Cow<[u8]>, &[u8], etc).
fn fields_to_term_inner<'a>(
    env: Env<'a>,
    fields: impl DoubleEndedIterator<Item = impl AsRef<[u8]>>,
) -> Term<'a> {
    let mut list = Term::list_new_empty(env);
    for field in fields.rev() {
        let bytes = field.as_ref();
        let mut binary = NewBinary::new(env, bytes.len());
        binary.as_mut_slice().copy_from_slice(bytes);
        let binary_term: Term = binary.into();
        list = list.list_prepend(binary_term);
    }
    list
}

/// Convert owned rows to Elixir term (for streaming/parallel parsers)
pub fn owned_rows_to_term<'a>(env: Env<'a>, rows: Vec<Vec<Vec<u8>>>) -> Term<'a> {
    let mut list = Term::list_new_empty(env);
    for row in rows.into_iter().rev() {
        list = list.list_prepend(owned_fields_to_term(env, row));
    }
    list
}

/// Convert owned fields to an Elixir list of binaries
pub fn owned_fields_to_term<'a>(env: Env<'a>, fields: Vec<Vec<u8>>) -> Term<'a> {
    fields_to_term_inner(env, fields.into_iter())
}

// ============================================================================
// Zero-Copy Sub-Binary Support
// ============================================================================

/// Create a sub-binary term referencing the original input (zero-copy).
/// Returns an empty binary if bounds are invalid (should never happen — indicates parser bug).
#[inline]
fn make_subbinary<'a>(env: Env<'a>, input: &Binary<'a>, start: usize, len: usize) -> Term<'a> {
    input
        .make_subbinary(start, len)
        .map(|b| b.into())
        .unwrap_or_else(|_| {
            debug_assert!(
                false,
                "make_subbinary out of bounds: start={start} len={len}"
            );
            NewBinary::new(env, 0).into()
        })
}

pub(crate) use crate::core::unescape_field;

/// Convert a single field to a term, using sub-binary when possible (hybrid Cow approach)
/// - Unquoted fields: sub-binary (zero-copy)
/// - Quoted without escapes: sub-binary of inner content (zero-copy)
/// - Quoted with escapes: copy and unescape (must allocate)
#[inline]
fn field_to_term_hybrid<'a>(
    env: Env<'a>,
    input: &Binary<'a>,
    start: usize,
    end: usize,
    escape: u8,
) -> Term<'a> {
    if start >= end {
        let binary = NewBinary::new(env, 0);
        return binary.into();
    }

    let field = &input.as_slice()[start..end];

    // Check if quoted
    if field.len() >= 2 && field[0] == escape && field[field.len() - 1] == escape {
        let inner = &field[1..field.len() - 1];

        if inner.contains(&escape) {
            // Must copy and unescape: "val""ue" -> val"ue
            let unescaped = unescape_field(inner, escape);
            let mut binary = NewBinary::new(env, unescaped.len());
            binary.as_mut_slice().copy_from_slice(&unescaped);
            return binary.into();
        } else {
            // Quoted but no escapes: sub-binary of inner content
            return make_subbinary(env, input, start + 1, end - start - 2);
        }
    }

    // Unquoted: direct sub-binary
    make_subbinary(env, input, start, end - start)
}

/// Convert field boundaries to Elixir terms using hybrid sub-binary/copy approach
/// boundaries: Vec of rows, each row is Vec of (start, end) pairs
pub fn boundaries_to_term_hybrid<'a>(
    env: Env<'a>,
    input: Binary<'a>,
    boundaries: Vec<Vec<(usize, usize)>>,
    escape: u8,
) -> Term<'a> {
    let mut list = Term::list_new_empty(env);

    for row in boundaries.into_iter().rev() {
        let mut row_list = Term::list_new_empty(env);
        for (start, end) in row.into_iter().rev() {
            let field_term = field_to_term_hybrid(env, &input, start, end, escape);
            row_list = row_list.list_prepend(field_term);
        }
        list = list.list_prepend(row_list);
    }

    list
}

// ============================================================================
// General Multi-Byte Escape Support (for zero-copy path)
// ============================================================================

use rustler::types::atom;
use rustler::Encoder;

use crate::strategy::{contains_escape, unescape_field_general};

/// Convert a single field to a term with multi-byte escape, using sub-binary when possible
#[inline]
fn field_to_term_hybrid_general<'a>(
    env: Env<'a>,
    input: &Binary<'a>,
    start: usize,
    end: usize,
    escape: &[u8],
) -> Term<'a> {
    if start >= end {
        let binary = NewBinary::new(env, 0);
        return binary.into();
    }

    let field = &input.as_slice()[start..end];
    let esc_len = escape.len();

    // Check if quoted (starts and ends with escape)
    if field.len() >= 2 * esc_len
        && field[..esc_len] == *escape
        && field[field.len() - esc_len..] == *escape
    {
        let inner = &field[esc_len..field.len() - esc_len];

        if contains_escape(inner, escape) {
            // Must copy and unescape
            let unescaped = unescape_field_general(inner, escape);
            let mut binary = NewBinary::new(env, unescaped.len());
            binary.as_mut_slice().copy_from_slice(&unescaped);
            return binary.into();
        } else {
            // Quoted but no escapes: sub-binary of inner content
            return make_subbinary(env, input, start + esc_len, end - start - 2 * esc_len);
        }
    }

    // Unquoted: direct sub-binary
    make_subbinary(env, input, start, end - start)
}

/// Convert field boundaries to Elixir terms with multi-byte escape support
pub fn boundaries_to_term_hybrid_general<'a>(
    env: Env<'a>,
    input: Binary<'a>,
    boundaries: Vec<Vec<(usize, usize)>>,
    escape: &[u8],
) -> Term<'a> {
    let mut list = Term::list_new_empty(env);

    for row in boundaries.into_iter().rev() {
        let mut row_list = Term::list_new_empty(env);
        for (start, end) in row.into_iter().rev() {
            let field_term = field_to_term_hybrid_general(env, &input, start, end, escape);
            row_list = row_list.list_prepend(field_term);
        }
        list = list.list_prepend(row_list);
    }

    list
}

// ============================================================================
// Map Builders (for headers-to-maps feature)
// ============================================================================

/// Build a map from key/value term arrays.
/// Uses fast O(n) path, falls back to incremental for duplicate keys.
fn make_map<'a>(env: Env<'a>, keys: &[Term<'a>], values: &[Term<'a>]) -> Term<'a> {
    match Term::map_from_term_arrays(env, keys, values) {
        Ok(map) => map,
        Err(_) => {
            // Duplicate keys: build incrementally (last value wins)
            let mut map = Term::map_new(env);
            for (k, v) in keys.iter().zip(values.iter()) {
                map = map.map_put(*k, *v).unwrap_or(map);
            }
            map
        }
    }
}

/// Generic map builder: iterates rows in reverse, converts each field to a Term,
/// fills missing columns with nil, and builds a cons-list of maps.
fn rows_to_maps_inner<'a, R>(
    env: Env<'a>,
    keys: &[Term<'a>],
    rows: impl DoubleEndedIterator<Item = R>,
    field_count: impl Fn(&R) -> usize,
    field_to_term: impl Fn(Env<'a>, &R, usize) -> Term<'a>,
) -> Term<'a> {
    let num_keys = keys.len();
    let nil_term = atom::nil().encode(env);
    let mut value_terms = vec![nil_term; num_keys];
    let mut list = Term::list_new_empty(env);

    for row in rows.rev() {
        let row_len = field_count(&row);
        for (i, val) in value_terms.iter_mut().enumerate() {
            *val = if i < row_len {
                field_to_term(env, &row, i)
            } else {
                nil_term
            };
        }
        list = list.list_prepend(make_map(env, keys, &value_terms));
    }
    list
}

/// Convert boundary rows to maps with sub-binary hybrid approach (single-byte escape).
pub fn boundaries_to_maps_hybrid<'a>(
    env: Env<'a>,
    input: Binary<'a>,
    keys: &[Term<'a>],
    boundaries: &[Vec<(usize, usize)>],
    escape: u8,
) -> Term<'a> {
    rows_to_maps_inner(
        env,
        keys,
        boundaries.iter(),
        |row| row.len(),
        |env, row, i| {
            let (start, end) = row[i];
            field_to_term_hybrid(env, &input, start, end, escape)
        },
    )
}

/// Convert boundary rows to maps with multi-byte escape hybrid approach.
pub fn boundaries_to_maps_hybrid_general<'a>(
    env: Env<'a>,
    input: Binary<'a>,
    keys: &[Term<'a>],
    boundaries: &[Vec<(usize, usize)>],
    escape: &[u8],
) -> Term<'a> {
    rows_to_maps_inner(
        env,
        keys,
        boundaries.iter(),
        |row| row.len(),
        |env, row, i| {
            let (start, end) = row[i];
            field_to_term_hybrid_general(env, &input, start, end, escape)
        },
    )
}
