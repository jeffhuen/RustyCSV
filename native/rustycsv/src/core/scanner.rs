//! Byte-level helpers for field splitting and strict quote validation.

use super::Violation;

/// Scalar quote state shared by general and streaming parsers.
///
/// Delimiter recognition stays with each parser because separators and
/// newlines have different representations there. This type only records the
/// state transitions that define NimbleCSV-compatible quoting.
#[derive(Copy, Clone)]
pub(crate) struct QuoteState {
    in_quotes: bool,
    at_field_start: bool,
    after_closing_quote: bool,
    violation: Option<Violation>,
}

impl QuoteState {
    #[inline]
    pub(crate) const fn new() -> Self {
        Self {
            in_quotes: false,
            at_field_start: true,
            after_closing_quote: false,
            violation: None,
        }
    }

    #[inline]
    pub(crate) const fn in_quotes(self) -> bool {
        self.in_quotes
    }

    #[inline]
    pub(crate) const fn violation(self) -> Option<Violation> {
        self.violation
    }

    /// Consume an escape sequence. `doubled` is only meaningful inside a
    /// quoted field and keeps the parser quoted when true.
    #[inline]
    pub(crate) fn on_escape(&mut self, pos: usize, doubled: bool) {
        if self.in_quotes {
            if !doubled {
                self.in_quotes = false;
                self.after_closing_quote = true;
            }
        } else {
            if !self.at_field_start && self.violation.is_none() {
                self.violation = Some(Violation::UnexpectedQuote(pos));
            }
            self.in_quotes = true;
            self.at_field_start = false;
            self.after_closing_quote = false;
        }
    }

    #[inline]
    pub(crate) fn on_delimiter(&mut self) {
        self.at_field_start = true;
        self.after_closing_quote = false;
    }

    #[inline]
    pub(crate) fn on_data(&mut self, pos: usize) {
        if self.after_closing_quote && self.violation.is_none() {
            self.violation = Some(Violation::TrailingGarbage(pos));
        }
        self.at_field_start = false;
        self.after_closing_quote = false;
    }

    #[inline]
    pub(crate) fn finish(mut self, end: usize) -> Option<Violation> {
        if self.in_quotes && self.violation.is_none() {
            self.violation = Some(Violation::UnterminatedQuote(end));
        }
        self.violation
    }
}

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
