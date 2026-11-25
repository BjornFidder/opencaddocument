#![cfg(all(feature = "ifcx", feature = "mline-pilot"))]
use opencaddocument::{ifccad::*, Ifcx};

#[test]
fn xref_same_source_attach_overlay_preserving_roundtrip() {
    let mut drawing = load_ifccad_bytes(
        include_bytes!("fixtures/xref/attachment.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    let mut overlay = drawing
        .model
        .entities
        .iter()
        .filter_map(IfccadEntity::as_native)
        .find(|e| matches!(e.kind, IfccadEntityKind::Xref(_)))
        .unwrap()
        .clone();
    overlay.id = drawing.id_counters.allocate_entity_id().unwrap();
    let IfccadEntityKind::Xref(x) = &mut overlay.kind else {
        panic!()
    };
    x.reference_type = IfccadXrefReferenceType::Overlay;
    x.placement.origin[0] += 100.;
    drawing.model.entities.push(IfccadEntity::Native(overlay));
    let mut unused = drawing.external_sources[0].clone();
    unused.id = drawing.id_counters.allocate_external_source_id().unwrap();
    unused.name = format!("{}$XREF_OVERLAY", unused.name);
    drawing.external_sources.push(unused);
    let (mut imported, mut context) = Ifcx.import_ifccad_preserving(&drawing).unwrap();
    let refs: Vec<_> = imported
        .document
        .block_records
        .iter()
        .filter(|r| r.flags.is_xref)
        .collect();
    assert_eq!(refs.len(), 3);
    assert_eq!(refs.iter().filter(|r| r.flags.is_xref_overlay).count(), 1);
    let saved = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap();
    assert_eq!(saved.drawing.external_sources, drawing.external_sources);
    assert_eq!(
        saved.drawing.model.entities.len(),
        drawing.model.entities.len()
    );
    for (a, b) in saved
        .drawing
        .model
        .entities
        .iter()
        .zip(&drawing.model.entities)
    {
        assert_eq!(a.id(), b.id());
        let (IfccadEntityKind::Xref(a), IfccadEntityKind::Xref(b)) =
            (&a.as_native().unwrap().kind, &b.as_native().unwrap().kind)
        else {
            panic!()
        };
        assert_eq!(
            (a.source_id, a.reference_type, a.scale, a.placement.origin),
            (b.source_id, b.reference_type, b.scale, b.placement.origin)
        );
        for (a, b) in a
            .placement
            .x_axis
            .iter()
            .chain(&a.placement.y_axis)
            .zip(b.placement.x_axis.iter().chain(&b.placement.y_axis))
        {
            assert!((a - b).abs() < 1e-12);
        }
    }
    let bytes = encode_ifccad_document(&saved.drawing).unwrap();
    validate_ifccad_document(
        load_ifccad_bytes(bytes.bytes(), Default::default())
            .unwrap()
            .document(),
    )
    .unwrap();
    for record in imported
        .document
        .block_records
        .iter_mut()
        .filter(|r| r.flags.is_xref)
    {
        record.flags.is_xref_overlay = true;
    }
    let saved = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap();
    assert_eq!(saved.drawing.external_sources, drawing.external_sources);
    for entity in saved
        .drawing
        .model
        .entities
        .iter()
        .filter_map(IfccadEntity::as_native)
    {
        if let IfccadEntityKind::Xref(x) = &entity.kind {
            assert_eq!(x.reference_type, IfccadXrefReferenceType::Overlay);
        }
    }
}

#[test]
fn overlay_type_survives_physical_dwg_and_dxf() {
    use opencaddocument::{Dwg, Dxf, Format};
    let mut drawing = load_ifccad_bytes(
        include_bytes!("fixtures/xref/attachment.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    let IfccadEntityKind::Xref(x) = &mut drawing.model.entities[0].as_native_mut().unwrap().kind
    else {
        panic!()
    };
    x.reference_type = IfccadXrefReferenceType::Overlay;
    let (imported, _) = Ifcx.import_ifccad_preserving(&drawing).unwrap();
    for format in [&Dwg as &dyn Format, &Dxf as &dyn Format] {
        let (bytes, _) = format.write(&imported.document).unwrap();
        let (cad, _) = format.read(&bytes).unwrap();
        let back = Ifcx.export_ifccad(&cad).unwrap();
        let IfccadEntityKind::Xref(x) = &back.drawing.model.entities[0].as_native().unwrap().kind
        else {
            panic!()
        };
        assert_eq!(x.reference_type, IfccadXrefReferenceType::Overlay);
        assert_eq!(x.scale, 500.);
        validate_ifccad_document(&back.drawing).unwrap();
    }
}
