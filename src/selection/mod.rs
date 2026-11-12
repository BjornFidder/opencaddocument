#![doc = include_str!("../../docs/selection.md")]

mod capture;
pub mod reconstruct;
mod references;

pub use reconstruct::*;
#[cfg(feature = "selection-storage")]
pub mod storage;

use crate::{
    entities::{Line, MLine},
    objects::MLineStyle,
    tables::{Layer, LineType},
    Document, Handle,
};
use std::collections::BTreeSet;

/// The source domain of a captured identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SelectionRecordKind {
    Entity,
    Layer,
    LineType,
    Object,
}

/// A source-local identity, qualified by the selection's namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SelectionRecordKey {
    pub kind: SelectionRecordKind,
    pub handle: Handle,
}

/// Explicit roots and deliberate dependency boundaries.
#[derive(Debug, Clone)]
pub struct CaptureRequest {
    pub namespace: String,
    pub roots: Vec<Handle>,
    /// Existing dependencies whose content the caller will bind externally.
    /// Roots cannot be external. Unused keys are rejected.
    pub external: BTreeSet<SelectionRecordKey>,
}
impl CaptureRequest {
    pub fn new(namespace: impl Into<String>, roots: Vec<Handle>) -> Self {
        Self {
            namespace: namespace.into(),
            roots,
            external: BTreeSet::new(),
        }
    }
}

/// Existing model types, owned independently of the source document.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectionPayload {
    Line(Box<Line>),
    MLine(Box<MLine>),
    MLineStyle(Box<MLineStyle>),
    Layer(Box<Layer>),
    LineType(Box<LineType>),
}
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionRecord {
    pub key: SelectionRecordKey,
    pub payload: SelectionPayload,
}

/// Located reference roles; repeated incoming requirements are retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionSlot {
    Owner,
    Layer,
    LineType,
    MLineStyle,
    StyleElementLineType(usize),
    StyleRegistration,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextRole {
    ModelSpace,
    MLineStyleDictionary,
    ByLayer,
    ByBlock,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequirementDisposition {
    Included(SelectionRecordKey),
    External(SelectionRecordKey),
    Missing,
    Context(ContextRole),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionRequirement {
    pub source: SelectionRecordKey,
    pub slot: SelectionSlot,
    /// Original/effective source reference. Does not refer to a destination.
    pub source_handle: Option<Handle>,
    pub source_name: Option<String>,
    pub disposition: RequirementDisposition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureEvidenceCode {
    MissingDependency,
    ExcludedRawRecord,
    ExcludedGraphics,
    ExcludedDerivedData,
    ResolvedSourceDefault,
    UnknownUnits,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureEvidence {
    pub source: Option<SelectionRecordKey>,
    pub slot: Option<SelectionSlot>,
    pub code: CaptureEvidenceCode,
    pub message: String,
}
/// Effective source-wide settings; no destination defaults are consulted.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionContext {
    pub insertion_units: i16,
    pub linetype_scale: f64,
}

/// Immutable capture with its sole authoritative evidence report.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentSelection {
    namespace: String,
    roots: Vec<SelectionRecordKey>,
    records: Vec<SelectionRecord>,
    requirements: Vec<SelectionRequirement>,
    report: Vec<CaptureEvidence>,
    context: SelectionContext,
}
#[must_use]
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureOutcome {
    pub selection: DocumentSelection,
}
impl DocumentSelection {
    /// Reconstruct the requested roots into an independent candidate.
    pub fn reconstruct_available(
        &self,
        target: &Document,
        request: &ReconstructionRequest,
    ) -> Result<ReconstructionOutcome, ReconstructionError> {
        reconstruct::run(self, target, request)
    }

    pub fn capture(
        document: &Document,
        request: &CaptureRequest,
    ) -> Result<CaptureOutcome, SelectionError> {
        capture::capture(document, request)
    }
    pub fn namespace(&self) -> &str {
        &self.namespace
    }
    pub fn roots(&self) -> &[SelectionRecordKey] {
        &self.roots
    }
    pub fn records(&self) -> &[SelectionRecord] {
        &self.records
    }
    pub fn requirements(&self) -> &[SelectionRequirement] {
        &self.requirements
    }
    pub fn capture_report(&self) -> &[CaptureEvidence] {
        &self.report
    }
    pub fn context(&self) -> &SelectionContext {
        &self.context
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionErrorKind {
    InvalidRequest,
    AmbiguousIdentity,
    ReferenceConflict,
    UnsupportedContent,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionError {
    pub kind: SelectionErrorKind,
    pub source: Option<SelectionRecordKey>,
    pub slot: Option<SelectionSlot>,
    pub message: String,
}
impl std::fmt::Display for SelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} at {:?}/{:?}: {}",
            self.kind, self.source, self.slot, self.message
        )
    }
}
impl std::error::Error for SelectionError {}
