//! The logical API shares conversion behavior with the IFCX file adapter.
#![cfg(feature = "ifcx")]

use std::collections::BTreeSet;

use opencaddocument::entities::{Line, Spline};
use opencaddocument::ifccad::{self, IfccadEntity};
use opencaddocument::objects::ObjectType;
use opencaddocument::types::{Handle, Vector3};
use opencaddocument::{Document, EntityType, Format, Ifcx};

fn line_document() -> (Document, Handle) {
    let mut document = Document::new();
    let handle = document
        .add_entity(EntityType::Line(Line::from_points(
            Vector3::new(1.0, 2.0, 0.0),
            Vector3::new(4.0, 5.0, 0.0),
        )))
        .unwrap();
    (document, handle)
}

#[test]
fn typed_import_matches_file_reader_for_fixtures() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "ifcx"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty());
    let mut saw_losses = false;
    for path in paths {
        let bytes = std::fs::read(&path).unwrap();
        let drawing = ifccad::load_ifccad_bytes(&bytes, Default::default())
            .unwrap()
            .into_document();
        let typed = Ifcx.import_ifccad(&drawing).unwrap();
        let (document, losses) = Ifcx.read(&bytes).unwrap();
        assert_eq!(typed.losses, losses, "{}", path.display());
        saw_losses |= !losses.is_empty();
        // CAD storage order across different Paper owners may vary. Compare
        // the logical drawing, which retains each scope's entity order.
        let typed_projection = Ifcx.export_ifccad(&typed.document).unwrap();
        let mut file_projection = Ifcx.export_ifccad(&document).unwrap();
        file_projection
            .drawing
            .header
            .timestamp
            .clone_from(&typed_projection.drawing.header.timestamp);
        assert_eq!(
            typed_projection.drawing,
            file_projection.drawing,
            "{}",
            path.display()
        );
        assert_eq!(
            typed_projection.losses,
            file_projection.losses,
            "{}",
            path.display()
        );
    }
    assert!(saw_losses, "fixtures must exercise nonempty import losses");
}

#[test]
fn typed_export_matches_file_writer_and_strictly_reopens() {
    let (mut document, handle) = line_document();
    let EntityType::Line(line) = document.get_entity_mut(handle).unwrap() else {
        panic!("expected line");
    };
    line.thickness = 2.0;
    let typed = Ifcx.export_ifccad(&document).unwrap();
    assert!(typed.losses.is_empty());
    let imported = Ifcx.import_ifccad(&typed.drawing).unwrap();
    let converted = imported
        .document
        .get_entity(imported.entity_handles[&typed.entity_ids[&handle]])
        .unwrap();
    let EntityType::Line(line) = converted else {
        panic!("expected line");
    };
    assert_eq!(line.start, Vector3::new(1.0, 2.0, 0.0));
    assert_eq!(line.end, Vector3::new(4.0, 5.0, 0.0));
    assert_eq!(line.thickness, 2.0);
    let (bytes, losses) = Ifcx.write(&document).unwrap();
    assert_eq!(typed.losses, losses);
    let mut file = ifccad::load_ifccad_bytes(&bytes, Default::default())
        .unwrap()
        .into_document();
    // Each export creates its own timestamp; it is not mapping evidence.
    file.header
        .timestamp
        .clone_from(&typed.drawing.header.timestamp);
    assert_eq!(typed.drawing, file);
    let encoded = ifccad::encode_ifccad_document(&typed.drawing).unwrap();
    let reopened = ifccad::load_ifccad_bytes(encoded.bytes(), Default::default()).unwrap();
    assert_eq!(reopened.document(), &typed.drawing);
}

#[test]
fn maps_identify_converted_entities_in_model_paper_and_blocks() {
    let bytes = include_bytes!("fixtures/layouts-viewports.ifcx");
    let drawing = ifccad::load_ifccad_bytes(bytes, Default::default())
        .unwrap()
        .into_document();
    assert!(!drawing.model.entities.is_empty());
    assert!(drawing.paper_layouts.iter().any(|p| !p.entities.is_empty()));
    assert!(drawing.blocks.iter().any(|b| !b.entities.is_empty()));
    let imported = Ifcx.import_ifccad(&drawing).unwrap();
    let mut expected = BTreeSet::new();
    let scopes = std::iter::once(&drawing.model.entities)
        .chain(drawing.paper_layouts.iter().map(|p| &p.entities))
        .chain(drawing.blocks.iter().map(|b| &b.entities));
    for entity in scopes.flatten() {
        expected.insert(entity.id());
        assert!(imported
            .document
            .get_entity(imported.entity_handles[&entity.id()])
            .is_some());
    }
    assert_eq!(
        imported
            .entity_handles
            .keys()
            .copied()
            .collect::<BTreeSet<_>>(),
        expected
    );
    for entity in &drawing.model.entities {
        let handle = imported.entity_handles[&entity.id()];
        assert!(imported
            .document
            .model_space_entities()
            .any(|e| e.common().handle == handle));
    }
    for block in &drawing.blocks {
        for entity in &block.entities {
            let handle = imported.entity_handles[&entity.id()];
            assert!(imported
                .document
                .entities_in_block(&block.name)
                .any(|e| e.common().handle == handle));
        }
    }
    for paper in &drawing.paper_layouts {
        let owner = imported
            .document
            .objects
            .values()
            .find_map(|object| match object {
                ObjectType::Layout(layout) if layout.name == paper.name => {
                    Some(layout.block_record)
                }
                _ => None,
            })
            .unwrap();
        for entity in &paper.entities {
            let handle = imported.entity_handles[&entity.id()];
            assert_eq!(
                imported
                    .document
                    .get_entity(handle)
                    .unwrap()
                    .common()
                    .owner_handle,
                owner
            );
        }
    }

    let exported = Ifcx.export_ifccad(&imported.document).unwrap();
    let again = Ifcx.import_ifccad(&exported.drawing).unwrap();
    for (handle, id) in &exported.entity_ids {
        let source = imported.document.get_entity(*handle).unwrap();
        let target = again.document.get_entity(again.entity_handles[id]).unwrap();
        assert_eq!(
            source.as_entity().entity_type(),
            target.as_entity().entity_type()
        );
    }
    assert_eq!(exported.entity_ids.len(), again.entity_handles.len());
}

#[test]
fn export_map_omits_unsupported_entities_and_reports_the_loss() {
    let (mut document, line) = line_document();
    let spline = document
        .add_entity(EntityType::Spline(Spline::new()))
        .unwrap();
    let typed = Ifcx.export_ifccad(&document).unwrap();
    assert!(typed.entity_ids.contains_key(&line));
    assert!(!typed.entity_ids.contains_key(&spline));
    assert!(!typed.losses.is_empty());
    let entity = typed.drawing.model.entities.first().unwrap();
    assert_eq!(typed.entity_ids[&line], entity.id());
    let IfccadEntity::Native(entity) = entity else {
        panic!("expected native line");
    };
    assert!(matches!(
        entity.kind,
        ifccad::IfccadEntityKind::LineSegment { .. }
    ));
}

#[test]
fn import_rejects_invalid_logical_input() {
    let mut drawing = Ifcx.export_ifccad(&Document::new()).unwrap().drawing;
    drawing.length_unit = "invalid".into();
    assert!(Ifcx.import_ifccad(&drawing).is_err());
}

#[test]
fn export_rejects_invalid_logical_output() {
    let (mut document, handle) = line_document();
    let EntityType::Line(line) = document.get_entity_mut(handle).unwrap() else {
        panic!("expected line");
    };
    line.start.x = f64::NAN;
    assert!(Ifcx.export_ifccad(&document).is_err());
}
