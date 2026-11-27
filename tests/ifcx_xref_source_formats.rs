#![cfg(all(feature = "ifcx", feature = "mline-pilot"))]
use opencaddocument::{ifccad::*, *};

fn source_drawing() -> IfccadDocument {
    let mut drawing = load_ifccad_bytes(
        include_bytes!("fixtures/xref/attachment.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    drawing.external_sources[0].source = IfccadExternalSourceContent::Dxf {
        base_point: [10., 20., 0.],
        insertion_unit: Some("m".into()),
        fingerprint_guid: Some("e5b4a9af-cf20-4fc9-a30f-ffb20fd987e1".into()),
    };
    drawing
}

#[test]
fn preserving_dxf_source_keeps_typed_identity_even_with_dwg_filename() {
    let drawing = source_drawing();
    let (imported, mut context) = Ifcx.import_ifccad_preserving(&drawing).unwrap();
    let saved = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap();
    assert_eq!(saved.drawing.external_sources, drawing.external_sources);
    let expected = drawing.model.entities[0].as_native().unwrap();
    let actual = saved.drawing.model.entities[0].as_native().unwrap();
    assert_eq!(actual.id, expected.id);
    let (IfccadEntityKind::Xref(expected), IfccadEntityKind::Xref(actual)) =
        (&expected.kind, &actual.kind)
    else {
        panic!("XREF placement expected")
    };
    assert_eq!(actual.source_id, expected.source_id);
    assert_eq!(actual.transform.scale, expected.transform.scale);
    assert_eq!(
        actual.transform.placement.origin,
        expected.transform.placement.origin
    );
    let axis = |x: &IfccadXref| {
        let (c, s) = (x.transform.rotation.cos(), x.transform.rotation.sin());
        let p = &x.transform.placement;
        std::array::from_fn::<_, 6, _>(|i| {
            let k = i % 3;
            if i < 3 {
                c * p.x_axis[k] + s * p.y_axis[k]
            } else {
                -s * p.x_axis[k] + c * p.y_axis[k]
            }
        })
    };
    for (a, b) in axis(actual).into_iter().zip(axis(expected)) {
        assert!((a - b).abs() < 1e-12);
    }
}

#[test]
fn stateless_cad_projection_refuses_non_dwg_source_by_type() {
    for uri in [
        "../Refs/grondplan.dxf",
        "../Refs/grondplan.dwg",
        "../Refs/grondplan",
    ] {
        let mut drawing = source_drawing();
        drawing.external_sources[0].location =
            IfccadExternalSourceLocation::UriReference { uri: uri.into() };
        let error = Ifcx.import_ifccad(&drawing).unwrap_err().to_string();
        assert!(error.contains("DWG"), "{error}");
    }
}

#[test]
fn dxf_source_metadata_uses_actual_header_identity_without_unit_conversion() {
    let mut document = Document::new();
    document.header.model_space_insertion_base = Vector3::new(10., 20., 0.);
    document.header.insertion_units = 6;
    document.header.fingerprint_guid = "{E5B4A9AF-CF20-4FC9-A30F-FFB20FD987E1}".into();
    let source = dxf_external_source_content(&document);
    assert_eq!(source, source_drawing().external_sources[0].source);
}

#[test]
fn cad_xref_with_dxf_path_maps_to_dxf_content() {
    let mut drawing = source_drawing();
    drawing.external_sources[0].location = IfccadExternalSourceLocation::UriReference {
        uri: "../Refs/grondplan.dxf".into(),
    };
    let (imported, _) = Ifcx.import_ifccad_preserving(&drawing).unwrap();
    let exported = Ifcx.export_ifccad(&imported.document).unwrap();
    assert_eq!(exported.drawing.external_sources.len(), 1);
    assert_eq!(
        exported.drawing.external_sources[0].source,
        drawing.external_sources[0].source
    );
}
