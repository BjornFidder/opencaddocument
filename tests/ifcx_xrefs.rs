#![cfg(all(feature = "ifcx", feature = "mline-pilot"))]
use opencaddocument::{ifccad::*, *};

fn drawing() -> IfccadDocument {
    load_ifccad_bytes(
        include_bytes!("fixtures/xref/attachment.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document()
}

#[test]
fn offline_xref_roundtrip_keeps_source_and_effective_placement() {
    let d = drawing();
    let (imported, mut context) = Ifcx.import_ifccad_preserving(&d).unwrap();
    assert!(imported.losses.is_empty(), "{:?}", imported.losses);
    let source = imported
        .document
        .block_records
        .get("grondplan")
        .expect("XREF definition");
    assert!(source.flags.is_xref);
    assert_eq!(source.base_point, Vector3::new(10., 20., 0.));
    assert_eq!(source.xref_path, "../Referenties/grondplan.dwg");
    let x = imported
        .document
        .model_space_entities()
        .find_map(|e| match e {
            EntityType::Insert(i) => Some(i),
            _ => None,
        })
        .unwrap();
    assert_eq!(x.insert_point, Vector3::new(10000., 5000., 0.));
    assert_eq!([x.x_scale(), x.y_scale(), x.z_scale()], [500.; 3]);
    let saved = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap();
    assert_eq!(saved.drawing.external_sources, d.external_sources);
    assert_eq!(
        saved.drawing.model.entities[0].id(),
        d.model.entities[0].id()
    );
    let IfccadEntityKind::Xref(saved) = &saved.drawing.model.entities[0].as_native().unwrap().kind
    else {
        panic!()
    };
    assert_eq!(saved.scale, 500.);
    for (format, keep_guid) in [(&Dwg as &dyn Format, true), (&Dxf as &dyn Format, false)] {
        let (bytes, losses) = format.write(&imported.document).unwrap();
        assert!(losses.is_empty(), "{losses:?}");
        let (cad, _) = format.read(&bytes).unwrap();
        let back = Ifcx.export_ifccad(&cad).unwrap();
        assert_eq!(back.drawing.external_sources.len(), 1);
        let IfccadExternalSourceContent::Dwg {
            base_point,
            fingerprint_guid,
            ..
        } = &back.drawing.external_sources[0].source
        else {
            panic!("DWG source expected")
        };
        assert_eq!(*base_point, [10., 20., 0.]);
        assert_eq!(fingerprint_guid.is_some(), keep_guid);
        let IfccadEntityKind::Xref(x) = &back.drawing.model.entities[0].as_native().unwrap().kind
        else {
            panic!()
        };
        assert_eq!(x.scale, 500.);
        assert!((x.placement.x_axis[1] - 1.).abs() < 1e-12);
        validate_ifccad_document(&back.drawing).unwrap();
    }
}

#[test]
fn loaded_xref_is_never_flattened_into_native_model() {
    let d = drawing();
    let (mut imported, mut context) = Ifcx.import_ifccad_preserving(&d).unwrap();
    let handle = imported
        .document
        .block_records
        .get("grondplan")
        .unwrap()
        .handle;
    let mut line = entities::Line::from_coords(10., 20., 0., 12., 20., 0.);
    line.common.owner_handle = handle;
    let mut generated = tables::Layer::new("grondplan|SOURCE");
    generated.handle = imported.document.allocate_handle();
    generated.flags.xref_dependent = true;
    generated.xref_block_record_handle = handle;
    imported.document.layers.add(generated).unwrap();
    imported
        .document
        .add_entity(EntityType::Line(line))
        .unwrap();
    let back = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap();
    assert!(back.drawing.blocks.is_empty());
    assert!(!back.drawing.layers.iter().any(|l| l.name.contains('|')));
    assert_eq!(back.drawing.model.entities.len(), 1);
    assert_eq!(back.drawing.external_sources, d.external_sources);
    let mut local = entities::Line::from_coords(0., 0., 0., 1., 0., 0.);
    local.common.layer = "grondplan|SOURCE".into();
    imported
        .document
        .add_entity(EntityType::Line(local))
        .unwrap();
    let with_local = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap();
    assert!(with_local
        .losses
        .iter()
        .any(|loss| loss.message.contains("external dependent layer")));
}

#[test]
fn source_without_insert_is_retained_and_deletion_does_not_reuse_id() {
    let mut d = drawing();
    d.model.entities.clear();
    let (imported, mut context) = Ifcx.import_ifccad_preserving(&d).unwrap();
    let mut back = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap()
        .drawing;
    assert_eq!(back.external_sources, d.external_sources);
    back.external_sources.clear();
    assert_eq!(back.id_counters.allocate_external_source_id().unwrap().0, 2);
}

#[test]
fn unsupported_cad_attachment_variants_are_explicit_losses() {
    for variant in ["overlay", "array", "paper", "block"] {
        let imported = Ifcx.import_ifccad(&drawing()).unwrap();
        let mut cad = imported.document;
        let handle = *imported.entity_handles.get(&1000).unwrap();
        if variant == "overlay" {
            cad.block_records
                .get_mut("grondplan")
                .unwrap()
                .flags
                .is_xref_overlay = true;
        }
        if variant == "array" {
            let EntityType::Insert(i) = cad.get_entity_mut(handle).unwrap() else {
                panic!()
            };
            i.row_count = 2;
        }
        if variant == "paper" {
            let owner = cad
                .block_records
                .iter()
                .find(|b| b.is_paper_space())
                .unwrap()
                .handle;
            cad.get_entity_mut(handle)
                .unwrap()
                .common_mut()
                .owner_handle = owner;
        }
        if variant == "block" {
            let mut record = tables::BlockRecord::new("LOCAL");
            record.handle = cad.allocate_handle();
            let owner = record.handle;
            cad.block_records.add(record).unwrap();
            cad.get_entity_mut(handle)
                .unwrap()
                .common_mut()
                .owner_handle = owner;
        }
        let out = Ifcx.export_ifccad(&cad).unwrap();
        assert!(
            out.losses.iter().any(|l| l.message.contains("XREF")),
            "{variant}: {:?}",
            out.losses
        );
        assert!(!out
            .drawing
            .model
            .entities
            .iter()
            .filter_map(IfccadEntity::as_native)
            .any(|e| matches!(e.kind, IfccadEntityKind::Xref(_))));
    }
}

#[test]
fn unknown_uri_is_retained_in_editing_but_has_no_silent_cad_export() {
    let mut d = drawing();
    d.external_sources[0].location = IfccadExternalSourceLocation::UriReference {
        uri: "custom:opaque".into(),
    };
    let (imported, mut context) = Ifcx.import_ifccad_preserving(&d).unwrap();
    let back = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap();
    assert_eq!(back.drawing.external_sources, d.external_sources);
    assert!(Dwg.write(&imported.document).is_err());
    assert!(Dxf.write(&imported.document).is_err());
}

#[test]
fn explicit_path_edit_and_undo_keep_source_id_and_original_uri_spelling() {
    let mut d = drawing();
    d.external_sources[0].location = IfccadExternalSourceLocation::UriReference {
        uri: "../Referenties/%67rondplan.dwg".into(),
    };
    let (mut imported, mut context) = Ifcx.import_ifccad_preserving(&d).unwrap();
    let original = imported
        .document
        .block_records
        .get("grondplan")
        .unwrap()
        .xref_path
        .clone();
    imported
        .document
        .block_records
        .get_mut("grondplan")
        .unwrap()
        .xref_path = "../Andere/new.dwg".into();
    let outcome = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap();
    assert!(outcome.losses.is_empty(), "{:?}", outcome.losses);
    let saved = outcome.drawing;
    assert_eq!(saved.external_sources[0].id, d.external_sources[0].id);
    assert_eq!(
        saved.external_sources[0].location,
        IfccadExternalSourceLocation::UriReference {
            uri: "../Andere/new.dwg".into()
        }
    );
    imported
        .document
        .block_records
        .get_mut("grondplan")
        .unwrap()
        .xref_path = original;
    let undone = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap()
        .drawing;
    assert_eq!(undone.external_sources, d.external_sources);
}

#[test]
fn physical_dependency_metadata_has_explicit_native_loss_diagnostics() {
    let mut cad = Ifcx.import_ifccad(&drawing()).unwrap().document;
    cad.file_dependencies[0].version_guid = Some("{11111111-2222-3333-4444-555555555555}".into());
    let out = Ifcx.export_ifccad(&cad).unwrap();
    assert!(out
        .losses
        .iter()
        .any(|l| l.message.contains("FileDepList metadata")));
    cad.file_dependencies[0].version_guid = None;
    cad.file_dependencies[0].file_name = "unbound.dwg".into();
    let out = Ifcx.export_ifccad(&cad).unwrap();
    assert!(out
        .losses
        .iter()
        .any(|l| l.message.contains("unbound FileDepList")));
}
