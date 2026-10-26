//! acadrust's DWG and DXF readers, producing a [`Document`].
//!
//! `read` and `read_with_stats` return the shared model's types; everything
//! else derefs to acadrust's reader.

use std::io::{Read, Seek};
use std::path::Path;

use acadrust::{DwgReadOptions, DxfError, DxfReaderConfiguration};

use crate::{Document, ReadOutcome};

pub struct DwgReader<R: Read + Seek>(acadrust::DwgReader<R>);

impl DwgReader<std::fs::File> {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, DxfError> {
        acadrust::DwgReader::from_file(path).map(Self)
    }

    pub fn from_file_with_options<P: AsRef<Path>>(
        path: P,
        options: DwgReadOptions,
    ) -> Result<Self, DxfError> {
        acadrust::DwgReader::from_file_with_options(path, options).map(Self)
    }
}

impl<R: Read + Seek> DwgReader<R> {
    pub fn from_stream(stream: R) -> Self {
        Self(acadrust::DwgReader::from_stream(stream))
    }

    pub fn from_stream_with_options(stream: R, options: DwgReadOptions) -> Self {
        Self(acadrust::DwgReader::from_stream_with_options(stream, options))
    }

    pub fn read(&mut self) -> Result<Document, DxfError> {
        self.0.read().map(Document::from)
    }

    pub fn read_with_stats(&mut self) -> Result<ReadOutcome, DxfError> {
        self.0.read_with_stats().map(ReadOutcome::from)
    }
}

impl<R: Read + Seek> From<acadrust::DwgReader<R>> for DwgReader<R> {
    fn from(reader: acadrust::DwgReader<R>) -> Self {
        Self(reader)
    }
}

impl<R: Read + Seek> std::ops::Deref for DwgReader<R> {
    type Target = acadrust::DwgReader<R>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<R: Read + Seek> std::ops::DerefMut for DwgReader<R> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub struct DxfReader(acadrust::DxfReader);

impl DxfReader {
    pub fn from_reader<R: Read + Seek + 'static>(reader: R) -> Result<Self, DxfError> {
        acadrust::DxfReader::from_reader(reader).map(Self)
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, DxfError> {
        acadrust::DxfReader::from_file(path).map(Self)
    }

    pub fn with_configuration(self, config: DxfReaderConfiguration) -> Self {
        Self(self.0.with_configuration(config))
    }

    pub fn read(self) -> Result<Document, DxfError> {
        self.0.read().map(Document::from)
    }

    pub fn read_with_stats(self) -> Result<ReadOutcome, DxfError> {
        self.0.read_with_stats().map(ReadOutcome::from)
    }
}

impl std::ops::Deref for DxfReader {
    type Target = acadrust::DxfReader;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for DxfReader {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
