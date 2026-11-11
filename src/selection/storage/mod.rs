#![doc = include_str!("../../../docs/selection-storage.md")]

use super::DocumentSelection;

mod frame;
mod validate;
mod wire;

/// Category of an envelope, wire or selection validation error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SelectionStorageErrorKind {
    InvalidEnvelope,
    UnsupportedVersion,
    UnsupportedKind,
    Truncated,
    TrailingData,
    LimitExceeded,
    InvalidBody,
    InvalidSelection,
}

/// A failure to encode or reopen a selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionStorageError {
    pub kind: SelectionStorageErrorKind,
    /// Byte offset from the beginning of the envelope, when available.
    pub offset: Option<usize>,
    /// JSON Pointer into the V1 body, when available.
    pub path: Option<String>,
    pub message: String,
}

impl std::fmt::Display for SelectionStorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)?;
        if let Some(offset) = self.offset {
            write!(f, " at byte {offset}")?;
        }
        if let Some(path) = &self.path {
            write!(f, " ({path})")?;
        }
        Ok(())
    }
}

impl std::error::Error for SelectionStorageError {}

type Result<T> = std::result::Result<T, SelectionStorageError>;

fn error(kind: SelectionStorageErrorKind, message: impl Into<String>) -> SelectionStorageError {
    SelectionStorageError {
        kind,
        offset: None,
        path: None,
        message: message.into(),
    }
}

/// Write one versioned V1 selection.
pub fn encode(selection: &DocumentSelection) -> Result<Vec<u8>> {
    validate::selection(selection)?;
    wire::write_v1(selection)
}

/// Read one versioned selection into owned model values.
pub fn decode(bytes: &[u8]) -> Result<DocumentSelection> {
    let selection = wire::read_v1(frame::body(bytes)?)?;
    validate::selection(&selection)?;
    Ok(selection)
}
