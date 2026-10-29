//! Reproducible, strictly read-back development review files. No overwrite.
#[cfg(feature = "ifcx")]
#[path = "support/dimension_cases.rs"]
mod cases;

#[cfg(feature = "ifcx")]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use ifccad::*;
    use opencaddocument::{Document, DwgReader, DwgWriter, DxfReader, DxfWriter, Ifcx};
    use serde_json::json;
    use std::path::Path;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let root = Path::new(
        args.first()
            .ok_or("usage: dimension_review <fresh-output-directory> [source.dwg]")?,
    );
    std::fs::create_dir(root)?;
    fn write(root: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(name))?
            .write_all(bytes)
    }
    fn losses(v: &[opencaddocument::Loss]) -> serde_json::Value {
        json!(v
            .iter()
            .map(|x| json!({"handle":x.handle.map(|h|h.to_string()),"message":x.message}))
            .collect::<Vec<_>>())
    }
    fn read(
        bytes: &[u8],
        dwg: bool,
    ) -> Result<opencaddocument::ReadOutcome, opencaddocument::Error> {
        let stream = std::io::Cursor::new(bytes.to_vec());
        Ok(if dwg {
            DwgReader::from_stream(stream).read_with_stats()?
        } else {
            DxfReader::from_reader(stream)?.read_with_stats()?
        })
    }
    fn stats(s: &opencaddocument::ReadStats) -> serde_json::Value {
        json!({"sourceFormat":format!("{:?}",s.source_format),"sourceVersion":s.source_version,"recoveryMode":s.recovery_mode,"streamCompleted":s.stream_completed,"decodedSourceRecords":s.decoded_source_records,"skippedSourceRecords":s.skipped_source_records,"recoveredErrors":s.recovered_errors,"diagnostics":s.diagnostics.iter().map(|d|format!("{d:?}")).collect::<Vec<_>>()})
    }
    fn report(d: &IfccadDocument) -> serde_json::Value {
        use ifccad::dimension::*;
        let scopes = std::iter::once(("model", &d.model.entities))
            .chain(d.paper_layouts.iter().map(|p| ("paper", &p.entities)))
            .chain(d.blocks.iter().map(|b| ("block", &b.entities)));
        json!(scopes.flat_map(|(owner,entities)|entities.iter().filter_map(move|e| {
            let e=e.as_native()?;
            let IfccadEntityKind::Dimension(v)=&e.kind else {return None;};
            let measurement=match evaluate_dimension(&v.definition).expect("strictly validated definition") {DimensionMeasurement::Distance(value)=>json!({"kind":"distance","value":value}),DimensionMeasurement::Angle(value)=>json!({"kind":"angle","value":value})};
            let roles:Vec<_>=v.associations.iter().map(|a|json!({"role":a.role,"sourceEntityId":a.source_entity_id.to_string(),"selector":a.selector})).collect();
            Some(json!({"owner":owner,"id":e.id.to_string(),"kind":v.definition.kind(),"definition":v.definition,"placement":v.placement,"measurement":measurement,"associations":v.associations.len(),"sourceRoles":roles,"labelDefinition":v.label,"textPlacement":v.text_placement,"graphicsParts":v.graphics.as_ref().map(|g|g.parts.len())}))
        })).collect::<Vec<_>>())
    }
    fn bundle(
        root: &Path,
        name: &str,
        drawing: IfccadDocument,
        original: Option<&Document>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = root.join(name);
        std::fs::create_dir(&root)?;
        let encoded = encode_ifccad_document(&drawing)?;
        let cbor = encode_ifccad_cbor_document(&drawing)?;
        validate_ifccad_document(
            load_ifccad_bytes(encoded.bytes(), Default::default())?.document(),
        )?;
        validate_ifccad_document(
            load_ifccad_cbor_bytes(cbor.bytes(), Default::default())?.document(),
        )?;
        write(&root, "source.ifcx", encoded.bytes())?;
        write(&root, "source.cbor", cbor.bytes())?;
        let imported = Ifcx.import_ifccad(&drawing)?;
        let mut diagnostics = json!({"import":losses(&imported.losses)});
        let mut reports = json!({"source":report(&drawing)});
        for (format, bytes) in [
            ("ascii", DxfWriter::new(&imported.document).write_to_vec()?),
            (
                "binary",
                DxfWriter::new_binary(&imported.document).write_to_vec()?,
            ),
            ("dwg", DwgWriter::write_to_vec(&imported.document)?),
        ] {
            let file = match format {
                "ascii" => "returned.dxf",
                "binary" => "returned-binary.dxf",
                _ => "returned.dwg",
            };
            write(&root, file, &bytes)?;
            let outcome = read(&bytes, format == "dwg")?;
            let returned = Ifcx.export_ifccad(&outcome.document)?;
            validate_ifccad_document(&returned.drawing)?;
            let encoded = encode_ifccad_document(&returned.drawing)?;
            load_ifccad_bytes(encoded.bytes(), Default::default())?;
            write(&root, &format!("returned-{format}.ifcx"), encoded.bytes())?;
            diagnostics[format] =
                json!({"readStats":stats(&outcome.stats),"export":losses(&returned.losses)});
            reports[format] = report(&returned.drawing);
        }
        if let Some(original) = original {
            reports["originalCadDimensionCount"] = json!(original
                .entities()
                .filter(|e| matches!(e, opencaddocument::EntityType::Dimension(_)))
                .count());
            #[cfg(feature = "serde")]
            {
                reports["originalCadDimensions"] = json!(original
                    .entities()
                    .filter_map(|e| match e {
                        opencaddocument::EntityType::Dimension(d) => Some(d.as_ref()),
                        _ => None,
                    })
                    .collect::<Vec<_>>());
            }
        }
        write(
            &root,
            "losses.json",
            &serde_json::to_vec_pretty(&diagnostics)?,
        )?;
        write(
            &root,
            "semantic-comparison.json",
            &serde_json::to_vec_pretty(&reports)?,
        )?;
        Ok(())
    }
    bundle(root, "all-nine-dimensions", cases::gallery(), None)?;
    for (i, definition) in cases::definitions().into_iter().enumerate() {
        bundle(
            root,
            &format!("{:02}-{:?}", i + 1, definition.kind()),
            cases::drawing(definition),
            None,
        )?;
    }
    if let Some(source) = args.get(1) {
        let bytes = std::fs::read(source)?;
        let outcome = read(&bytes, true)?;
        let exported = Ifcx.export_ifccad(&outcome.document)?;
        write(
            root,
            "sample-source-losses.json",
            &serde_json::to_vec_pretty(
                &json!({"readStats":stats(&outcome.stats),"export":losses(&exported.losses)}),
            )?,
        )?;
        bundle(
            root,
            "sample_AC1032",
            exported.drawing,
            Some(&outcome.document),
        )?;
    }
    write(root,"README.md",b"Development review: all-nine-dimensions contains the nine kinds on a 3x3 sheet, with deliberately generated illustrative pictures. Individual cases have no stored pictures and exercise CAD host regeneration. sample_AC1032, when provided, passes through native IFCCAD before returning to CAD. Check losses.json and semantic-comparison.json; unlisted semantics are unassessed. These are development files, not a published format release. Generated picture provenance becomes SourceSnapshot after CAD readback. Open returned.dwg or returned.dxf in CAD, report product/build, recovery notices, missing/wrong dimensions, text/arrows/placement, measurements and re-save behavior. Explorer integration waits for this review.\n")?;
    Ok(())
}

#[cfg(not(feature = "ifcx"))]
fn main() {
    eprintln!("enable feature ifcx");
    std::process::exit(1);
}
