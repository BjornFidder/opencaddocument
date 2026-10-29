//! Stored graphics loss is separate from retention of the dimension definition.
#![cfg(feature = "ifcx")]

use ifccad::dimension::{resolve_dimension_settings, DimensionDefinition};
use ifccad::geometry_kernel::{Point2, Point3};
use ifccad::{
    encode_ifccad_document, load_ifccad_bytes, validate_ifccad_document, IfccadBlockDefinition,
    IfccadCompositionPolicy, IfccadDimension, IfccadDimensionGraphicsOrigin, IfccadDocument,
    IfccadEntity, IfccadEntityKind, IfccadPaperLayout, IfccadReadOptions,
};
use opencaddocument::{Format, Ifcx, Loss};

const GRAPHICS: &[u8] = include_bytes!("fixtures/dimension-graphics.ifcx");

fn load(bytes: &[u8]) -> IfccadDocument {
    let drawing = load_ifccad_bytes(
        bytes,
        IfccadReadOptions {
            composition_policy: IfccadCompositionPolicy::RejectConflicts,
        },
    )
    .unwrap()
    .into_document();
    validate_ifccad_document(&drawing).unwrap();
    drawing
}

fn dimensions(entities: &[IfccadEntity]) -> Vec<&IfccadDimension> {
    entities
        .iter()
        .filter_map(|entity| match &entity.as_native()?.kind {
            IfccadEntityKind::Dimension(dimension) => Some(dimension.as_ref()),
            _ => None,
        })
        .collect()
}

fn scopes(drawing: &IfccadDocument) -> Vec<(String, &[IfccadEntity])> {
    let mut scopes = vec![("Model".to_string(), drawing.model.entities.as_slice())];
    scopes.extend(
        drawing
            .blocks
            .iter()
            .map(|b| (format!("Block {}", b.name), b.entities.as_slice())),
    );
    scopes.extend(
        drawing
            .paper_layouts
            .iter()
            .map(|p| (format!("Paper {}", p.name), p.entities.as_slice())),
    );
    scopes
}

fn definition_geometry(dimension: &IfccadDimension) -> ([Point3; 3], Option<f64>, Option<f64>) {
    let (line, direction, extension) = match dimension.definition {
        DimensionDefinition::Linear {
            dimension_line_point,
            direction_angle,
            extension_direction_angle,
            ..
        } => (
            dimension_line_point,
            Some(direction_angle),
            extension_direction_angle,
        ),
        DimensionDefinition::Aligned {
            dimension_line_point,
            extension_direction_angle,
            ..
        } => (dimension_line_point, None, extension_direction_angle),
        ref other => panic!(
            "linear/aligned regression fixture unexpectedly contains {:?}",
            other.kind()
        ),
    };
    let (first, second) = dimension.definition.reference_points();
    let frame = dimension.placement.coordinate_frame().unwrap();
    let world = |[x, y, z]: [f64; 3]| {
        frame
            .try_to_scope_spatial_point(Point3::new(x, y, z))
            .unwrap()
    };
    (
        [
            world(first),
            world(second),
            frame
                .try_to_scope_point(Point2::new(line[0], line[1]))
                .unwrap(),
        ],
        direction,
        extension,
    )
}

fn check_round_trip(bytes: &[u8], expected_losses: &[Loss]) {
    let source = load(bytes);
    let (document, losses) = Ifcx.read(bytes).unwrap();
    assert_eq!(losses, expected_losses);
    assert_eq!(document.audit().error_count(), 0);
    let (written, losses) = Ifcx.write(&document).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let restored = load(&written);
    let restored_scopes = scopes(&restored);
    for (name, entities) in scopes(&source) {
        let expected = dimensions(entities);
        if expected.is_empty() {
            continue;
        }
        let (_, restored_entities) = restored_scopes.iter().find(|(n, _)| *n == name).unwrap();
        let actual = dimensions(restored_entities);
        assert_eq!(actual.len(), expected.len());
        for (before, after) in expected.into_iter().zip(actual) {
            if before.graphics.as_ref().is_some_and(|g| g.parts.is_empty()) {
                assert!(after.graphics.as_ref().unwrap().parts.is_empty());
            } else {
                assert!(after.graphics.is_none());
            }
            // The writer recentres the local frame on the dimension line.
            // Compare definition points in the owning scope, not local coordinates.
            assert_eq!(after.placement.x_axis, before.placement.x_axis);
            assert_eq!(after.placement.y_axis, before.placement.y_axis);
            assert_eq!(definition_geometry(after), definition_geometry(before));
            assert_eq!(after.label, before.label);
            assert_eq!(after.text_placement, before.text_placement);
            let style = source
                .dimension_styles
                .iter()
                .find(|s| s.id == before.style_id)
                .unwrap();
            let restored_style = restored
                .dimension_styles
                .iter()
                .find(|s| s.id == after.style_id)
                .unwrap();
            assert_eq!(restored_style.name, style.name);
            let settings = resolve_dimension_settings(
                &style.settings,
                before.definition.kind(),
                &style.type_overrides,
                &before.overrides,
            );
            let mut restored_settings = resolve_dimension_settings(
                &restored_style.settings,
                after.definition.kind(),
                &restored_style.type_overrides,
                &after.overrides,
            );
            let text = source
                .text_styles
                .iter()
                .find(|s| s.id == settings.text.style)
                .unwrap();
            let restored_text = restored
                .text_styles
                .iter()
                .find(|s| s.id == restored_settings.text.style)
                .unwrap();
            assert_eq!(restored_text.name, text.name);
            assert_eq!(restored_text.properties, text.properties);
            // Compare referenced style content, not freshly allocated IDs.
            restored_settings.text.style = settings.text.style;
            assert_eq!(restored_settings, settings);
            assert_eq!(after.associations.len(), before.associations.len());
            for (before, after) in before.associations.iter().zip(&after.associations) {
                assert_eq!(after.role, before.role);
                assert_eq!(after.selector, before.selector);
                let target = entities
                    .iter()
                    .find(|e| e.id() == before.source_entity_id)
                    .unwrap();
                let restored_target = restored_entities
                    .iter()
                    .find(|e| e.id() == after.source_entity_id)
                    .unwrap();
                assert_eq!(
                    restored_target.as_native().unwrap().kind,
                    target.as_native().unwrap().kind
                );
            }
        }
    }
    // The payload is already gone; a subsequent import must not report it again.
    let (_, losses) = Ifcx.read(&written).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
}

fn fixture_losses(snapshot: bool, generated: bool) -> Vec<Loss> {
    // The original snapshot's helper is on explicit layer 0, not Defpoints.
    // The empty Generated picture now transfers, with its provenance loss.
    let mut messages = Vec::new();
    if snapshot {
        messages.push("dimension 9007199254740994 graphics: explicit layer 0 would acquire CAD inheritance: not converted");
    }
    if generated {
        messages.push("dimension 9007199254740995 graphics provenance (Generated): not converted");
    }
    messages
        .into_iter()
        .map(|message| Loss {
            handle: None,
            message: message.into(),
        })
        .collect()
}

#[test]
fn importing_stored_dimension_graphics_reports_loss() {
    let source = load(GRAPHICS);
    let dimensions = dimensions(&source.model.entities);
    assert_eq!(dimensions.len(), 2);
    assert!(dimensions.iter().all(|d| d.graphics.is_some()));
    assert_eq!(
        dimensions[0].graphics.as_ref().unwrap().origin,
        IfccadDimensionGraphicsOrigin::SourceSnapshot
    );
    assert_eq!(
        dimensions[1].graphics.as_ref().unwrap().origin,
        IfccadDimensionGraphicsOrigin::Generated
    );
    check_round_trip(GRAPHICS, &fixture_losses(true, true));
}

#[test]
fn dimensions_without_stored_graphics_do_not_report_graphics_loss() {
    let mut source = load(GRAPHICS);
    for entity in &mut source.model.entities {
        if let IfccadEntityKind::Dimension(dimension) = &mut entity.as_native_mut().unwrap().kind {
            dimension.graphics = None;
        }
    }
    let encoded = encode_ifccad_document(&source).unwrap();
    check_round_trip(encoded.bytes(), &[]);
}

#[test]
fn a_single_stored_picture_reports_one_loss_for_either_origin() {
    // The fixture has sourceSnapshot/linear and generated/aligned graphics.
    for remove in 0..2 {
        let mut source = load(GRAPHICS);
        let dimension = source
            .model
            .entities
            .iter_mut()
            .filter_map(|entity| match &mut entity.as_native_mut()?.kind {
                IfccadEntityKind::Dimension(dimension) => Some(dimension),
                _ => None,
            })
            .nth(remove)
            .unwrap();
        dimension.graphics = None;
        let encoded = encode_ifccad_document(&source).unwrap();
        check_round_trip(encoded.bytes(), &fixture_losses(remove != 0, remove != 1));
    }
}

#[test]
fn stored_dimension_graphics_in_block_members_report_loss() {
    let mut source = load(GRAPHICS);
    source.blocks.push(IfccadBlockDefinition {
        id: 1,
        name: "Dimensions".to_string(),
        description: String::new(),
        anonymous: false,
        explodable: true,
        uniform_scaling: false,
        base_point: [0.0; 3],
        insertion_unit: source.length_unit.clone(),
        bounds: None,
        bounds_quality: None,
        entities: std::mem::take(&mut source.model.entities),
    });
    source.id_counters.next_block_id = 2;
    let encoded = encode_ifccad_document(&source).unwrap();
    check_round_trip(encoded.bytes(), &fixture_losses(true, true));
}

#[test]
fn stored_dimension_graphics_in_paper_layouts_report_loss() {
    let mut source = load(GRAPHICS);
    source.paper_layouts.push(IfccadPaperLayout {
        id: 2,
        name: "Dimensions".to_string(),
        tab_index: 1,
        settings: Default::default(),
        canvas: None,
        bounds: None,
        bounds_quality: None,
        entities: std::mem::take(&mut source.model.entities),
    });
    source.id_counters.next_layout_id = 3;
    let encoded = encode_ifccad_document(&source).unwrap();
    check_round_trip(encoded.bytes(), &fixture_losses(true, true));
}
