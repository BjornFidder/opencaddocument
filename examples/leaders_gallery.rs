//! One grouped Leader-family gallery and one optional bounded practice roundtrip.
use opencaddocument::entities::{
    multileader::{BlockAttribute, LeaderLinePropertyOverrideFlags, StartEndPointPair},
    AttributeDefinition, Leader, LeaderCreationType, MultiLeader, Tolerance,
};
use opencaddocument::ifccad::*;
use opencaddocument::{
    tables, CheckedProxyGraphicRecord, CheckedProxyGraphics, Color, Document, DwgReader, DwgWriter,
    DxfReader, DxfWriter, EntityType, Error, Ifcx, ProxyGraphicCommand, Vector3,
};
use std::{io::Cursor, path::Path};

pub fn gallery() -> Result<IfccadDocument, Error> {
    let mut document = Document::new();
    let tolerance = document.add_entity(EntityType::Tolerance(Tolerance::with_text(
        Vector3::new(30., 80., 0.),
        "{\\Fgdt;j}%%v0.1%%vA",
    )))?;
    document.add_entity(EntityType::Leader(
        Leader::two_point(Vector3::new(0., 100., 0.), Vector3::new(30., 80., 0.))
            .with_creation_type(LeaderCreationType::WithTolerance)
            .with_annotation(tolerance),
    ))?;
    document.add_entity(EntityType::Leader(
        Leader::two_point(Vector3::new(0., 60., 0.), Vector3::new(30., 80., 0.))
            .with_spline_path()
            .with_creation_type(LeaderCreationType::WithTolerance)
            .with_annotation(tolerance),
    ))?;
    let mut text = MultiLeader::with_text(
        "MLeader\\PIndependent content",
        Vector3::new(70., 75., 0.),
        vec![Vector3::new(45., 100., 0.), Vector3::new(60., 75., 0.)],
    );
    text.text_height = 2.5;
    text.context.text_height = 2.5;
    text.context.text_width = 45.;
    text.context.leader_roots[0].leader_index = 47;
    text.context.leader_roots[0].lines[0].index = 901;
    text.context.leader_roots[0].lines[0].override_flags =
        LeaderLinePropertyOverrideFlags::LINE_COLOR;
    text.context.leader_roots[0].lines[0].line_color = Color::Index(3);
    text.context.leader_roots[0].lines[0].break_points = vec![StartEndPointPair::new(
        Vector3::new(50., 90., 0.),
        Vector3::new(51., 88., 0.),
    )];
    text.common.graphic_data = Some(
        CheckedProxyGraphics {
            records: vec![CheckedProxyGraphicRecord::Command(
                ProxyGraphicCommand::Polyline(vec![
                    Vector3::new(45., 100., 0.),
                    Vector3::new(60., 75., 0.),
                    Vector3::new(70., 75., 0.),
                ]),
            )],
        }
        .encode_checked()?,
    );
    document.add_entity(EntityType::MultiLeader(Box::new(text)))?;
    let mut record = tables::BlockRecord::new("LEADER_REVIEW_LABEL");
    record.handle = document.allocate_handle();
    record.block_entity_handle = document.allocate_handle();
    record.block_end_handle = document.allocate_handle();
    let owner = record.handle;
    document.block_records.add(record)?;
    let mut definition =
        AttributeDefinition::new("CODE".into(), "Leader label".into(), "Default".into());
    definition.common.owner_handle = owner;
    definition.height = 2.5;
    let definition = document.add_entity(EntityType::AttributeDefinition(definition))?;
    let mut block = MultiLeader::new();
    block.set_block_content(owner, Vector3::new(45., 35., 0.));
    block.context.block_content_scale = Vector3::new(-1., 1., 1.);
    block.context.transform_matrix[12] = 45.;
    block.context.transform_matrix[13] = 35.;
    block.block_attributes.push(BlockAttribute {
        attribute_definition_handle: Some(definition),
        index: 17,
        width: 1.,
        text: "L-42".into(),
    });
    let root = block.add_leader_root();
    root.connection_point = Vector3::new(45., 35., 0.);
    root.create_line(vec![Vector3::new(15., 15., 0.), Vector3::new(30., 35., 0.)]);
    document.add_entity(EntityType::MultiLeader(Box::new(block)))?;
    let mut drawing = Ifcx.export_ifccad(&document)?.drawing;
    drawing.annotation_scales.push(IfccadAnnotationScale {
        id: IfccadAnnotationScaleId(50),
        name: "Leader review 1:50".into(),
        paper_units: 1.,
        drawing_units: 50.,
        unit_scale: false,
    });
    drawing.id_counters.next_annotation_scale_id = 51;
    for entity in &mut drawing.model.entities {
        let IfccadEntity::Native(entity) = entity else {
            continue;
        };
        if let IfccadEntityKind::MultiLeader(value) = &mut entity.kind {
            if value.presentation.content.m_text.is_some() {
                let key = IfccadAnnotationContextKey {
                    scale_id: IfccadAnnotationScaleId(50),
                };
                let mut saved = value.presentation.clone();
                saved.scale_factor = 2.;
                value.annotation = Some(IfccadAnnotation {
                    mode: IfccadAnnotationMode::Enabled,
                    orientation: None,
                    reference: Some(key),
                    contexts: vec![IfccadAnnotationContext {
                        key,
                        presentation: Some(saved),
                    }],
                });
            }
        }
    }
    if let Some(mut occurrence)=drawing.model.entities.iter().find(|e|matches!(e.as_native().map(|n|&n.kind),Some(IfccadEntityKind::MultiLeader(v))if v.presentation.content.block.is_some())).cloned(){
        let id=drawing.id_counters.allocate_entity_id()?;occurrence.as_native_mut().unwrap().id=id;
        if let Some(paper)=drawing.paper_layouts.first_mut(){paper.entities.push(occurrence);}
    }
    validate_ifccad_document(&drawing)?;
    Ok(drawing)
}
fn read(bytes: Vec<u8>, format: &str) -> Result<Document, Error> {
    let outcome = if format == "dwg" {
        DwgReader::from_stream(Cursor::new(bytes)).read_with_stats()?
    } else {
        DxfReader::from_reader(Cursor::new(bytes))?.read_with_stats()?
    };
    if outcome.stats.recovered_errors != 0
        || outcome.stats.skipped_source_records != 0
        || !outcome.stats.stream_completed
    {
        return Err(format!("{format} did not read strictly: {:?}", outcome.stats).into());
    }
    Ok(outcome.document)
}
fn counts(drawing: &IfccadDocument) -> serde_json::Value {
    let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
    let mut snapshots = 0;
    let mut commands = 0;
    for e in drawing
        .model
        .entities
        .iter()
        .chain(drawing.paper_layouts.iter().flat_map(|p| &p.entities))
        .chain(drawing.blocks.iter().flat_map(|b| &b.entities))
    {
        let Some(e) = e.as_native() else { continue };
        let (kind, graphics) = match &e.kind {
            IfccadEntityKind::Leader(v) => ("Leader", v.graphics.as_ref()),
            IfccadEntityKind::MultiLeader(v) => ("MultiLeader", v.graphics.as_ref()),
            IfccadEntityKind::Tolerance(v) => ("Tolerance", v.graphics.as_ref()),
            _ => continue,
        };
        *kinds.entry(kind).or_default() += 1;
        if let Some(g) = graphics {
            snapshots += 1;
            commands += g.commands.len();
        }
    }
    serde_json::json!({"kinds":kinds,"snapshots":snapshots,"commands":commands})
}
fn save(drawing: &IfccadDocument, out: &Path, stem: &str) -> Result<serde_json::Value, Error> {
    let json = encode_ifccad_document(drawing)?;
    load_ifccad_bytes(json.bytes(), Default::default())?;
    std::fs::write(out.join(format!("{stem}.ifcx")), json.bytes())?;
    let cbor = encode_ifccad_cbor_document(drawing)?;
    load_ifccad_cbor_bytes(cbor.bytes(), Default::default())?;
    std::fs::write(out.join(format!("{stem}.ifccad.cbor")), cbor.bytes())?;
    let imported = Ifcx.import_ifccad(drawing)?;
    let mut transports = Vec::new();
    for format in ["ascii", "binary", "dwg"] {
        let bytes = match format {
            "dwg" => DwgWriter::write_to_vec(&imported.document)?,
            "binary" => DxfWriter::new_binary(&imported.document).write_to_vec()?,
            _ => DxfWriter::new(&imported.document).write_to_vec()?,
        };
        std::fs::write(
            out.join(format!(
                "{stem}-{format}.{}",
                if format == "dwg" { "dwg" } else { "dxf" }
            )),
            &bytes,
        )?;
        let returned = Ifcx.export_ifccad(&read(bytes, format)?)?;
        let wire = encode_ifccad_document(&returned.drawing)?;
        load_ifccad_bytes(wire.bytes(), Default::default())?;
        std::fs::write(
            out.join(format!("{stem}-returned-{format}.ifcx")),
            wire.bytes(),
        )?;
        transports.push(serde_json::json!({"format":format,"counts":counts(&returned.drawing),"losses":returned.losses.iter().map(|l|&l.message).collect::<Vec<_>>()}));
    }
    Ok(
        serde_json::json!({"native":counts(drawing),"importLosses":imported.losses.iter().map(|l|&l.message).collect::<Vec<_>>(),"transports":transports}),
    )
}
#[cfg(not(test))]
fn main() -> Result<(), Error> {
    let mut args = std::env::args().skip(1);
    let out = args.next().ok_or("output directory required")?;
    let out = Path::new(&out);
    std::fs::create_dir_all(out)?;
    let mut drawing = gallery()?;
    let mut report = serde_json::json!({"gallery":save(&drawing,out,"leaders-gallery")?,"tolerancePayload":"open; H1 self-generated codec experiment only"});
    let style = drawing.dimension_styles[0].id;
    if let Some(value) =
        drawing
            .model
            .entities
            .iter_mut()
            .find_map(|e| match &mut e.as_native_mut()?.kind {
                IfccadEntityKind::MultiLeader(v) if v.presentation.content.m_text.is_some() => {
                    Some(v)
                }
                _ => None,
            })
    {
        let placement = value
            .presentation
            .content
            .m_text
            .take()
            .unwrap()
            .text
            .placement;
        value.settings.common.content_kind = IfccadMultiLeaderContentKind::Tolerance;
        value.annotation = None;
        value.graphics = None;
        value.presentation.content.tolerance = Some(Box::new(IfccadMultiLeaderToleranceContent {
            placement,
            dimension_style: style,
            overrides: Default::default(),
            notation: IfccadToleranceNotation::CadFeatureControlTextV1 {
                code: "{\\Fgdt;j}%%v0.1%%vA".into(),
            },
        }));
        let encoded = encode_ifccad_document(&drawing)?;
        let loaded = load_ifccad_bytes(encoded.bytes(), Default::default())?;
        validate_ifccad_document(loaded.document())?;
        std::fs::write(out.join("leaders-tolerance-open.ifcx"), encoded.bytes())?;
    }
    let mut has_practice = false;
    if let Some(source) = args.next() {
        has_practice = true;
        let bytes = std::fs::read(&source)?;
        let cad = read(
            bytes,
            if source.to_ascii_lowercase().ends_with(".dwg") {
                "dwg"
            } else {
                "ascii"
            },
        )?;
        let native = Ifcx.export_ifccad(&cad)?;
        report["practice"] = save(&native.drawing, out, "sample_AC1032")?;
        report["practiceSource"] = source.into();
        report["practiceExportLosses"] =
            serde_json::json!(native.losses.iter().map(|l| &l.message).collect::<Vec<_>>());
    }
    std::fs::write(
        out.join(if has_practice {
            "review-report.json"
        } else {
            "gallery-review-report.json"
        }),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
