//! One explicitly selected resource-review roundtrip; no corpus or measurement work.
use ifccad::text::{FontRequest, MTextInline};
use ifccad::*;
use opencaddocument::{Document, DwgReader, DwgWriter, DxfReader, DxfWriter, Ifcx};
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::Cursor, path::Path};
type Error = Box<dyn std::error::Error + Send + Sync>;

fn font(font: &FontRequest) -> Value {
    json!({"family":font.family,"cadFontName":font.cad_font_name,"bigFontName":font.big_font_name,
        "bold":font.bold,"italic":font.italic,"charset":font.charset,"pitch":font.pitch})
}
fn sort(rows: &mut [Value]) {
    rows.sort_by_key(Value::to_string);
}
fn projection(drawing: &IfccadDocument) -> Value {
    let mut styles = drawing
        .text_styles
        .iter()
        .map(|s| json!({"name":s.name,"font":font(&s.properties.font)}))
        .collect::<Vec<_>>();
    sort(&mut styles);
    let mut bindings: BTreeMap<IfccadSymbolSourceId, Vec<Value>> = BTreeMap::new();
    for pattern in &drawing.line_patterns {
        for (index, element) in pattern.pattern.iter().enumerate() {
            if let Some(IfccadLinePatternMarker::Symbol {
                symbol_source_id,
                shape_number,
                ..
            }) = &element.marker
            {
                bindings.entry(*symbol_source_id).or_default().push(json!([
                    pattern.name,
                    index,
                    shape_number
                ]));
            }
        }
    }
    let mut sources = drawing
        .symbol_sources
        .iter()
        .map(|s| {
            let mut uses = bindings.remove(&s.id).unwrap_or_default();
            sort(&mut uses);
            json!({"request":s.source.effective_cad_file_name(),"bindings":uses})
        })
        .collect::<Vec<_>>();
    sort(&mut sources);
    let mut text = Vec::new();
    for entity in drawing
        .model
        .entities
        .iter()
        .chain(drawing.paper_layouts.iter().flat_map(|s| &s.entities))
        .chain(drawing.blocks.iter().flat_map(|s| &s.entities))
    {
        let IfccadEntity::Native(entity) = entity else {
            continue;
        };
        let IfccadEntityKind::MText(mtext) = &entity.kind else {
            continue;
        };
        let style = &drawing
            .text_styles
            .iter()
            .find(|s| s.id == mtext.style_id)
            .unwrap()
            .properties
            .font;
        let base = mtext.character_format.font.as_ref().unwrap_or(style);
        for paragraph in &mtext.content {
            let parent = paragraph.character_format.font.as_ref().unwrap_or(base);
            for inline in &paragraph.inlines {
                match inline {
                    MTextInline::Run {
                        text: value,
                        character_format,
                    } => text.push(json!([
                        "run",
                        value,
                        font(character_format.font.as_ref().unwrap_or(parent))
                    ])),
                    MTextInline::Stack(stack) => text.push(json!([
                        "stack",
                        stack.upper,
                        stack.lower,
                        font(stack.character_format.font.as_ref().unwrap_or(parent))
                    ])),
                    _ => {}
                }
            }
        }
    }
    sort(&mut text);
    json!({"styles":styles,"sources":sources,"mTextEffectiveRequests":text})
}
fn read_cad(bytes: Vec<u8>, dwg: bool) -> Result<(Document, Value), Error> {
    let outcome = if dwg {
        DwgReader::from_stream(Cursor::new(bytes)).read_with_stats()?
    } else {
        DxfReader::from_reader(Cursor::new(bytes))?.read_with_stats()?
    };
    if outcome.stats.recovered_errors != 0 || outcome.stats.skipped_source_records != 0 {
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
    let mut report = json!({"inputName":input.file_name().unwrap().to_string_lossy(),"profile":"0.5.0","cborRevision":5,
        "sourceReader":source_reader,"sourceMappingDiagnostics":source_losses,"nativeToCadDiagnostics":losses(&converted.losses),
        "before":before,"targets":[],"scope":"Font/SHX requests and source bindings; ordinary geometry and rendering not certified by this projection"});
    let mut equal = true;
    for format in ["dwg", "ascii-dxf", "binary-dxf"] {
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
        let strict = load_ifccad_bytes(strict.bytes(), Default::default())?;
        let after = projection(strict.document());
        let same = after == report["before"];
        equal &= same;
        report["targets"].as_array_mut().unwrap().push(json!({"format":format,"file":filename,"resourceProjectionEqual":same,"reader":reader,"diagnostics":losses(&restored.losses),"after":after}));
    }
    report["allResourceProjectionsEqual"] = json!(equal);
    std::fs::write(
        output.join("resource-readback.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    if !equal {
        return Err("resource projection differs; see resource-readback.json".into());
    }
    println!("Resource requests and bindings match in three AC1032 targets.");
    Ok(())
}
