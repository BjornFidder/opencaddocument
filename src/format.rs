use std::io::Cursor;

use acadrust::{DwgWriter, DxfWriter, Handle};

use crate::{Document, DwgReader, DxfReader};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Something a conversion could not carry over.
#[derive(Debug, Clone, PartialEq)]
pub struct Loss {
    /// The object it concerns, when there is one.
    pub handle: Option<Handle>,
    pub message: String,
}

/// The contract every format converter implements: read a file into a
/// [`Document`] and write a document back, reporting what was lost either way.
pub trait Format {
    /// Short name, such as `"DWG"`.
    fn name(&self) -> &str;
    /// File extensions it reads and writes, lowercase and without the dot.
    fn extensions(&self) -> &[&str];
    fn read(&self, bytes: &[u8]) -> Result<(Document, Vec<Loss>), Error>;
    fn write(&self, document: &Document) -> Result<(Vec<u8>, Vec<Loss>), Error>;
}

/// Round `value` to a multiple of `step`.
pub fn snap(value: f64, step: f64) -> f64 {
    // `+ 0.0` turns -0.0 into 0.0, so a value hovering around zero keeps
    // one spelling.
    (value / step).round() * step + 0.0
}

/// DWG, the model's native format: nothing is lost either way.
pub struct Dwg;

impl Format for Dwg {
    fn name(&self) -> &str {
        "DWG"
    }

    fn extensions(&self) -> &[&str] {
        &["dwg", "dwt", "dws"]
    }

    fn read(&self, bytes: &[u8]) -> Result<(Document, Vec<Loss>), Error> {
        let document = DwgReader::from_stream(Cursor::new(bytes.to_vec())).read()?;
        Ok((document, Vec::new()))
    }

    fn write(&self, document: &Document) -> Result<(Vec<u8>, Vec<Loss>), Error> {
        Ok((DwgWriter::write_to_vec(document)?, Vec::new()))
    }
}

/// DXF, the text form of the same model.
pub struct Dxf;

impl Format for Dxf {
    fn name(&self) -> &str {
        "DXF"
    }

    fn extensions(&self) -> &[&str] {
        &["dxf"]
    }

    fn read(&self, bytes: &[u8]) -> Result<(Document, Vec<Loss>), Error> {
        let document = DxfReader::from_reader(Cursor::new(bytes.to_vec()))?.read()?;
        Ok((document, Vec::new()))
    }

    fn write(&self, document: &Document) -> Result<(Vec<u8>, Vec<Loss>), Error> {
        Ok((DxfWriter::new(document).write_to_vec()?, Vec::new()))
    }
}
