//! The shared drawing model of the OpenCAD projects.
//!
//! The model is DWG-centred: [`Document`] holds a drawing the way DWG
//! describes it, and every other format converts to and from it through the
//! [`Format`] contract. Where formats describe the same concept differently,
//! the model keeps one representation and converts at the edge (see
//! [`Placement`]).

mod format;
#[cfg(feature = "ifcx")]
mod ifcx;
mod placement;
mod read;

// Everything acadrust offers is part of the model's surface, so callers name
// one crate. Items defined here take precedence over the glob.
pub use acadrust;
pub use acadrust::*;
pub use format::{snap, Dwg, Dxf, Error, Format, Loss};
#[cfg(feature = "ifcx")]
/// The IFCCAD model and IO library used by [`Ifcx`].
pub use ifccad;
#[cfg(feature = "ifcx")]
pub use ifcx::{IfccadExportOutcome, IfccadImportOutcome, Ifcx};
pub use placement::Placement;
pub use read::{DwgReader, DxfReader};

/// A drawing.
///
/// DWG-centred: it holds what acadrust's `CadDocument` holds and derefs to it,
/// so everything that works on a `CadDocument` works on a `Document`. What only
/// the shared model carries is added here, and converters for other formats
/// read and write a `Document`, never acadrust's type directly.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(transparent)
)]
pub struct Document(acadrust::CadDocument);

// acadrust's methods that take or return a whole document, typed as
// `Document` so callers never see the acadrust type.
impl Document {
    pub fn new() -> Self {
        Self(acadrust::CadDocument::new())
    }

    pub fn with_version(version: acadrust::DxfVersion) -> Self {
        Self(acadrust::CadDocument::with_version(version))
    }

    pub fn snapshot_structure(&mut self) -> Self {
        Self(self.0.snapshot_structure())
    }

    pub fn swap_structure(&mut self, snapshot: Self) -> Self {
        Self(self.0.swap_structure(snapshot.0))
    }

    pub fn align_added_entity_structure(
        before: &mut Self,
        after: &Self,
        added_handles: &[acadrust::Handle],
    ) {
        acadrust::CadDocument::align_added_entity_structure(&mut before.0, &after.0, added_handles)
    }

    /// The acadrust document, for the DWG/DXF readers and writers.
    pub fn into_inner(self) -> acadrust::CadDocument {
        self.0
    }
}

impl std::ops::Deref for Document {
    type Target = acadrust::CadDocument;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Document {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<acadrust::CadDocument> for Document {
    fn from(document: acadrust::CadDocument) -> Self {
        Self(document)
    }
}

impl From<Document> for acadrust::CadDocument {
    fn from(document: Document) -> Self {
        document.0
    }
}

/// What a read produced: the drawing and the reader's statistics.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReadOutcome {
    pub document: Document,
    pub stats: acadrust::ReadStats,
}

impl ReadOutcome {
    pub fn new(document: impl Into<Document>, stats: acadrust::ReadStats) -> Self {
        acadrust::ReadOutcome::new(document.into().into_inner(), stats).into()
    }
}

impl From<acadrust::ReadOutcome> for ReadOutcome {
    fn from(outcome: acadrust::ReadOutcome) -> Self {
        Self {
            document: outcome.document.into(),
            stats: outcome.stats,
        }
    }
}
