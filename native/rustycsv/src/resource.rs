// ResourceArc wrapper for streaming parser
//
// This allows the streaming parser state to persist across NIF calls.
// Supports both single-byte (fast path) and general (multi-byte) parsers.

use crate::core::Newlines;
use crate::strategy::{GeneralStreamingParser, GeneralStreamingParserNewlines, StreamingParser};
use rustler::ResourceArc;
use std::sync::Mutex;

/// Enum dispatching between single-byte and general streaming parsers
pub enum StreamingParserEnum {
    SingleByte(StreamingParser),
    General(GeneralStreamingParser),
    GeneralNewlines(GeneralStreamingParserNewlines),
}

impl StreamingParserEnum {
    pub fn feed(&mut self, chunk: &[u8]) -> Result<(), crate::strategy::streaming::BufferOverflow> {
        match self {
            StreamingParserEnum::SingleByte(p) => p.feed(chunk),
            StreamingParserEnum::General(p) => p.feed(chunk),
            StreamingParserEnum::GeneralNewlines(p) => p.feed(chunk),
        }
    }

    pub fn set_max_buffer_size(&mut self, max: usize) {
        match self {
            StreamingParserEnum::SingleByte(p) => p.set_max_buffer_size(max),
            StreamingParserEnum::General(p) => p.set_max_buffer_size(max),
            StreamingParserEnum::GeneralNewlines(p) => p.set_max_buffer_size(max),
        }
    }

    pub fn take_rows(&mut self, max: usize) -> Vec<Vec<Vec<u8>>> {
        match self {
            StreamingParserEnum::SingleByte(p) => p.take_rows(max),
            StreamingParserEnum::General(p) => p.take_rows(max),
            StreamingParserEnum::GeneralNewlines(p) => p.take_rows(max),
        }
    }

    pub fn available_rows(&self) -> usize {
        match self {
            StreamingParserEnum::SingleByte(p) => p.available_rows(),
            StreamingParserEnum::General(p) => p.available_rows(),
            StreamingParserEnum::GeneralNewlines(p) => p.available_rows(),
        }
    }

    pub fn has_partial(&self) -> bool {
        match self {
            StreamingParserEnum::SingleByte(p) => p.has_partial(),
            StreamingParserEnum::General(p) => p.has_partial(),
            StreamingParserEnum::GeneralNewlines(p) => p.has_partial(),
        }
    }

    pub fn buffer_size(&self) -> usize {
        match self {
            StreamingParserEnum::SingleByte(p) => p.buffer_size(),
            StreamingParserEnum::General(p) => p.buffer_size(),
            StreamingParserEnum::GeneralNewlines(p) => p.buffer_size(),
        }
    }

    pub fn finalize(&mut self) -> Vec<Vec<Vec<u8>>> {
        match self {
            StreamingParserEnum::SingleByte(p) => p.finalize(),
            StreamingParserEnum::General(p) => p.finalize(),
            StreamingParserEnum::GeneralNewlines(p) => p.finalize(),
        }
    }
}

/// Wrapper for StreamingParser that can be stored in a ResourceArc
#[must_use]
pub struct StreamingParserResource {
    pub inner: Mutex<StreamingParserEnum>,
}

impl StreamingParserResource {
    pub fn new() -> Self {
        StreamingParserResource {
            inner: Mutex::new(StreamingParserEnum::SingleByte(StreamingParser::new())),
        }
    }

    pub fn with_config(separator: u8, escape: u8) -> Self {
        StreamingParserResource {
            inner: Mutex::new(StreamingParserEnum::SingleByte(
                StreamingParser::with_config(separator, escape),
            )),
        }
    }

    pub fn with_multi_sep(separators: &[u8], escape: u8) -> Self {
        StreamingParserResource {
            inner: Mutex::new(StreamingParserEnum::SingleByte(
                StreamingParser::with_multi_sep(separators, escape),
            )),
        }
    }

    pub fn with_general(separators: Vec<Vec<u8>>, escape: Vec<u8>) -> Self {
        StreamingParserResource {
            inner: Mutex::new(StreamingParserEnum::General(GeneralStreamingParser::new(
                separators, escape,
            ))),
        }
    }

    pub fn with_general_newlines(
        separators: Vec<Vec<u8>>,
        escape: Vec<u8>,
        newlines: Newlines,
    ) -> Self {
        StreamingParserResource {
            inner: Mutex::new(StreamingParserEnum::GeneralNewlines(
                GeneralStreamingParserNewlines::new(separators, escape, newlines),
            )),
        }
    }
}

impl Default for StreamingParserResource {
    fn default() -> Self {
        Self::new()
    }
}

/// Type alias for the ResourceArc
pub type StreamingParserRef = ResourceArc<StreamingParserResource>;
