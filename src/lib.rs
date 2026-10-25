//! The shared drawing model of the OpenCAD projects.
//!
//! The model is DWG-centred: [`Document`] holds a drawing the way DWG
//! describes it, and every other format converts to and from it through the
//! [`Format`] contract. Where formats describe the same concept differently,
//! the model keeps one representation and converts at the edge (see
//! [`Placement`]).

mod format;
mod placement;

pub use acadrust;
pub use format::{snap, Dwg, Dxf, Error, Format, Loss};
pub use placement::Placement;

/// A drawing. For now this is acadrust's document unchanged; the model's own
/// type replaces the alias once it starts to differ.
pub type Document = acadrust::CadDocument;
