#![cfg(all(feature = "ifcx", feature = "mline-pilot"))]
use opencaddocument::{ifccad::*, Ifcx};

fn source() -> IfccadDocument {
    let mut source = load_ifccad_bytes(
        include_bytes!("fixtures/hello-cad.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    source.length_unit = "m".into();
    source
}

#[test]
fn native_source_metadata_has_zero_origin_and_no_invented_fingerprint() {
    let source = source();
    let content = opencaddocument::ifccad_external_source_content(&source).unwrap();
    assert_eq!(
        content,
        IfccadExternalSourceContent::Ifccad {
            base_point: [0.; 3],
            insertion_unit: Some("m".into()),
        }
    );
    assert_eq!(content.fingerprint_guid(), None);
}

#[test]
fn native_source_with_its_own_external_sources_preserves_context() {
    let source = load_ifccad_bytes(
        include_bytes!("fixtures/xref/attachment.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    let content = opencaddocument::ifccad_external_source_content(&source).unwrap();
    assert_eq!(content.base_point(), [0.; 3]);
    let (imported, mut context) = Ifcx.import_ifccad_preserving(&source).unwrap();
    let saved = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap();
    assert_eq!(saved.drawing.external_sources, source.external_sources);
}

#[test]
fn native_source_association_survives_edit_projection_and_refuses_cad_export() {
    let mut main = load_ifccad_bytes(
        include_bytes!("fixtures/xref/attachment.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    main.external_sources[0].source =
        opencaddocument::ifccad_external_source_content(&source()).unwrap();
    let (imported, mut context) = Ifcx.import_ifccad_preserving(&main).unwrap();
    let saved = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap();
    assert_eq!(saved.drawing.external_sources, main.external_sources);
    assert!(Ifcx
        .import_ifccad(&main)
        .unwrap_err()
        .to_string()
        .contains("DWG"));
}
