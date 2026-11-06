//! One explicitly selected block-attribute review roundtrip; no corpus or measurement work.
use ifccad::*;
use opencaddocument::{Document, DwgReader, DwgWriter, DxfReader, DxfWriter, Ifcx};
use serde_json::{json, Value};
use std::{io::Cursor, path::Path};
type Error = Box<dyn std::error::Error + Send + Sync>;

fn projection(d: &IfccadDocument) -> Value {
    let wire = encode_ifccad_document(d).unwrap();
    let v: Value = serde_json::from_slice(wire.bytes()).unwrap();
    v
}
fn read_cad(bytes: Vec<u8>, dwg: bool) -> Result<(Document, Value), Error> {
    let outcome = if dwg {
        DwgReader::from_stream(Cursor::new(bytes)).read_with_stats()?
    } else {
        DxfReader::from_reader(Cursor::new(bytes))?.read_with_stats()?
    };
    if !outcome.stats.stream_completed
        || outcome.stats.recovered_errors != 0
        || outcome.stats.skipped_source_records != 0
    {
        return Err("CAD reader recovered or skipped source records".into());
    }
    let stats = json!({"recoveredErrors":outcome.stats.recovered_errors,"skippedSourceRecords":outcome.stats.skipped_source_records,
        "diagnostics":outcome.stats.diagnostics.iter().map(|d|format!("{d:?}")).collect::<Vec<_>>()});
    Ok((outcome.document, stats))
}
fn losses(values: &[opencaddocument::Loss]) -> Vec<Value> {
    values
        .iter()
        .map(|l| json!({"handle":l.handle.map(|h|format!("{h:?}")),"message":l.message}))
        .collect()
}
fn main() -> Result<(), Error> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        return Err("usage: font_symbol_review INPUT NEW_OUTPUT_DIRECTORY".into());
    }
    let input = Path::new(&args[1]);
    let output = Path::new(&args[2]);
    if output.exists() {
        return Err("review output directory must be new".into());
    }
    let bytes = std::fs::read(input)?;
    let extension = input
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let (native, source_losses, source_reader) = if extension == "ifcx" {
        (
            load_ifccad_bytes(&bytes, Default::default())?.into_document(),
            Vec::new(),
            json!({"nativeStrict":true}),
        )
    } else {
        let (cad, reader) = read_cad(bytes, extension == "dwg")?;
        let exported = Ifcx.export_ifccad(&cad)?;
        (exported.drawing, losses(&exported.losses), reader)
    };
    let encoded = encode_ifccad_document(&native)?;
    let native = load_ifccad_bytes(encoded.bytes(), Default::default())?.into_document();
    let cbor = encode_ifccad_cbor_document(&native)?;
    load_ifccad_cbor_bytes(cbor.bytes(), Default::default())?;
    let converted = Ifcx.import_ifccad(&native)?;
    let mut document = converted.document;
    document.version = opencaddocument::types::DxfVersion::AC1032;
    std::fs::create_dir_all(output)?;
    std::fs::write(output.join("drawing.ifcx"), encoded.bytes())?;
    std::fs::write(output.join("drawing.ifccad.cbor"), cbor.bytes())?;
    let before = projection(&native);
    let mut report = json!({"inputName":input.file_name().unwrap().to_string_lossy(),"profile":"0.6.0","cborRevision":6,
        "sourceReader":source_reader,"sourceMappingDiagnostics":source_losses,"nativeToCadDiagnostics":losses(&converted.losses),
        "before":before,"targets":[],"scope":"Attribute records before and after physical AC1032 readback. Raw projections include local resource/field IDs; semantic comparison is recorded separately. Ordinary geometry/rendering are not certified."});
    let mut equal = true;
    for format in if extension == "ifcx" {
        vec!["dwg", "ascii-dxf", "binary-dxf"]
    } else {
        vec!["dwg", "ascii-dxf"]
    } {
        let bytes = match format {
            "dwg" => DwgWriter::write_to_vec(&document)?,
            "binary-dxf" => DxfWriter::new_binary(&document)
                .with_version(opencaddocument::types::DxfVersion::AC1032)
                .write_to_vec()?,
            _ => DxfWriter::new(&document)
                .with_version(opencaddocument::types::DxfVersion::AC1032)
                .write_to_vec()?,
        };
        let filename = if format == "dwg" {
            "roundtrip.dwg"
        } else if format == "ascii-dxf" {
            "roundtrip-ascii.dxf"
        } else {
            "roundtrip-binary.dxf"
        };
        std::fs::write(output.join(filename), &bytes)?;
        let (reopened, reader) = read_cad(bytes, format == "dwg")?;
        let restored = Ifcx.export_ifccad(&reopened)?;
        let strict = encode_ifccad_document(&restored.drawing)?;
        std::fs::write(
            output.join(format!("readback-{format}.ifcx")),
            strict.bytes(),
        )?;
        let strict = load_ifccad_bytes(strict.bytes(), Default::default())?;
        let after = projection(strict.document());
        let same = after == report["before"];
        equal &= same;
        report["targets"].as_array_mut().unwrap().push(json!({"format":format,"file":filename,"rawAttributeProjectionEqual":same,"reader":reader,"diagnostics":losses(&restored.losses),"after":after}));
    }
    report["allRawAttributeProjectionsEqual"] = json!(equal);
    std::fs::write(
        output.join("attribute-readback.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;

    println!("Review files written; raw attribute equality: {equal}");
    Ok(())
}
