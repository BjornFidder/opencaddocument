#![cfg(feature = "ifcx")]
#[allow(dead_code)]
#[path = "support/numerics.rs"]
mod numerics;
use ifccad::*;
use opencaddocument::{EntityType, Format, Ifcx};

fn source() -> IfccadDocument {
    let mut d = numerics::native_source(&numerics::spatial_case("aa_x_below"));
    d.model.entities = d
        .blocks
        .iter()
        .find(|b| b.name == "Level0")
        .unwrap()
        .entities
        .clone();
    d.blocks.retain(|b| b.name == "Marker");
    d
}
fn dimension(d: &mut IfccadDocument) -> &mut IfccadDimension {
    d.model
        .entities
        .iter_mut()
        .find_map(|e| match &mut e.as_native_mut().unwrap().kind {
            IfccadEntityKind::Dimension(d) => Some(d.as_mut()),
            _ => None,
        })
        .unwrap()
}
fn single_line(d: &mut IfccadDocument) {
    let dim = dimension(d);
    let mut part = dim.graphics.as_ref().unwrap().parts[0].clone();
    part.kind = IfccadDimensionGraphicsKind::LineSegment {
        start: [0.1, 0., 0.],
        end: [1., 0., 0.],
        thickness: 0.,
        extrusion_direction: [0., 0., 1.],
    };
    dim.graphics.as_mut().unwrap().parts = vec![part];
}

#[test]
fn finite_picture_rounding_follows_the_corresponding_ordinary_primitive() {
    for origin in [1e12, 1e16] {
        let mut d = source();
        single_line(&mut d);
        dimension(&mut d).placement.origin = [origin, 0., 0.];
        let bytes = encode_ifccad_document(&d).unwrap();
        numerics::strict_native(bytes.bytes());
        let (cad, losses) = Ifcx.read(bytes.bytes()).unwrap();
        let picture = cad
            .model_space_entities()
            .find_map(|e| match e {
                EntityType::Dimension(d) => Some(&d.base().block_name),
                _ => None,
            })
            .unwrap();
        assert!(!picture.is_empty(), "{losses:?}");
        let members: Vec<_> = cad.entities_in_block(picture).collect();
        assert_eq!(members.len(), 1);
        let EntityType::Line(line) = members[0] else {
            panic!()
        };
        assert_eq!(line.start.x.to_bits(), (origin + 0.1).to_bits());
        assert_eq!(line.end.x.to_bits(), (origin + 1.).to_bits());
        let mut ordinary = opencaddocument::Document::new();
        ordinary
            .add_entity(EntityType::Line(
                opencaddocument::entities::Line::from_points(line.start, line.end),
            ))
            .unwrap();
        let (native, losses) = Ifcx.write(&ordinary).unwrap();
        assert!(losses.is_empty(), "{losses:?}");
        numerics::strict_native(&native);
    }
}

#[test]
fn invalid_native_frames_still_fail_production_load() {
    let d = source();
    let encoded = encode_ifccad_document(&d).unwrap();
    let mut graph: serde_json::Value = serde_json::from_slice(encoded.bytes()).unwrap();
    let node = graph["data"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|n| n["attributes"].get("ifccad::dimension").is_some())
        .unwrap();
    node["attributes"]["ifccad::geom::placement"]["xAxis"] = serde_json::json!([2., 0., 0.]);
    let bytes = serde_json::to_vec(&graph).unwrap();
    assert!(load_ifccad_bytes(
        &bytes,
        IfccadReadOptions {
            composition_policy: IfccadCompositionPolicy::RejectConflicts
        }
    )
    .is_err());
}

fn mtext_picture(value: &str) -> opencaddocument::Document {
    let mut cad = numerics::read_native(&source());
    let name = cad
        .model_space_entities()
        .find_map(|e| match e {
            EntityType::Dimension(d) => Some(d.base().block_name.clone()),
            _ => None,
        })
        .unwrap();
    let member = cad.entities_in_block(&name).next().unwrap();
    let common = member.common().clone();
    let mut m = opencaddocument::entities::MText::new();
    m.common = common.clone();
    m.value = value.into();
    m.style = "Dimension text".into();
    m.insertion_point = opencaddocument::types::Vector3::new(16., 18., 0.);
    m.height = 2.5;
    m.rectangle_width = 80.;
    *cad.get_entity_mut(common.handle).unwrap() = EntityType::MText(Box::new(m));
    cad
}

#[test]
fn mtext_equivalent_braces_keep_the_picture_in_both_directions() {
    for value in [
        r"\A1;240",
        r"{\A1;240}",
        r"{\C1;red {\Lunderlined}\l normal}\P\pxqc;centre",
    ] {
        let cad = mtext_picture(value);
        let (bytes, losses) = Ifcx.write(&cad).unwrap();
        let mut native = numerics::strict_native(&bytes);
        let graphics = dimension(&mut native)
            .graphics
            .as_ref()
            .unwrap_or_else(|| panic!("{value}: {losses:?}"));
        assert_eq!(graphics.parts.len(), 4);
        assert!(matches!(
            graphics.parts[0].kind,
            IfccadDimensionGraphicsKind::MText(_)
        ));
        let restored = numerics::read_native(&native);
        let (bytes, losses) = Ifcx.write(&restored).unwrap();
        let mut native = numerics::strict_native(&bytes);
        assert_eq!(
            dimension(&mut native)
                .graphics
                .as_ref()
                .unwrap()
                .parts
                .len(),
            4,
            "{losses:?}"
        );
    }
}

#[test]
fn unknown_or_malformed_mtext_codes_cannot_be_normalized_away() {
    for value in [
        r"\Zsecret;240",
        r"\Hbad;240",
        r"\fArial|z9;240",
        r"\pxzz9;240",
        r"\C999;240",
    ] {
        let cad = mtext_picture(value);
        let (bytes, losses) = Ifcx.write(&cad).unwrap();
        let mut native = numerics::strict_native(&bytes);
        assert!(
            dimension(&mut native).graphics.is_none(),
            "{value}: {losses:?}"
        );
        assert!(losses.iter().any(|l| l.message.contains("graphics")));
    }
    let cad = mtext_picture(r"literal \\Zsecret;240");
    let (bytes, losses) = Ifcx.write(&cad).unwrap();
    let mut native = numerics::strict_native(&bytes);
    assert!(dimension(&mut native).graphics.is_some(), "{losses:?}");
}

#[test]
fn one_rounding_note_does_not_trigger_picture_omission() {
    let mut d = source();
    single_line(&mut d);
    dimension(&mut d).placement.origin = [1e12, 0., 0.];
    let extra = dimension(&mut d).graphics.as_ref().unwrap().parts[0].clone();
    dimension(&mut d)
        .graphics
        .as_mut()
        .unwrap()
        .parts
        .push(extra);
    let encoded = encode_ifccad_document(&d).unwrap();
    let (cad, losses) = Ifcx.read(encoded.bytes()).unwrap();
    let notes: Vec<_> = losses
        .iter()
        .filter(|l| l.message.contains("retained with numerical rounding"))
        .collect();
    assert_eq!(notes.len(), 1, "{losses:?}");
    assert!(notes[0].message.contains("p0"));
    assert!(notes[0].message.contains("position"));
    assert!(!notes[0].message.contains("not converted"));
    assert!(!notes[0].message.contains("tolerance"));
    assert!(cad
        .model_space_entities()
        .any(|e| matches!(e,EntityType::Dimension(d) if !d.base().block_name.is_empty())));

    let mut exact = source();
    single_line(&mut exact);
    let dim = dimension(&mut exact);
    dim.graphics.as_mut().unwrap().parts[0].kind = IfccadDimensionGraphicsKind::LineSegment {
        start: [0., 0., 0.],
        end: [1., 0., 0.],
        thickness: 0.,
        extrusion_direction: [0., 0., 1.],
    };
    let encoded = encode_ifccad_document(&exact).unwrap();
    let (_, losses) = Ifcx.read(encoded.bytes()).unwrap();
    assert!(
        !losses
            .iter()
            .any(|l| l.message.contains("numerical rounding")),
        "{losses:?}"
    );
}

#[test]
fn late_marker_failure_rolls_back_picture_resources_and_retained_notes() {
    let mut d = source();
    d.blocks[0].anonymous = true;
    d.blocks[0].name = "*U1".into();
    let dim = dimension(&mut d);
    let parts = &mut dim.graphics.as_mut().unwrap().parts;
    let marker = parts[3].clone();
    let line = parts[0].clone();
    *parts = vec![marker, line.clone(), line];
    parts[1].kind = IfccadDimensionGraphicsKind::LineSegment {
        start: [1e12, 0., 0.],
        end: [1e12, 1., 0.],
        thickness: 0.,
        extrusion_direction: [0., 0., 1.],
    };
    let (s, c) = 0.37_f64.sin_cos();
    dim.placement.x_axis = [c, s, 0.];
    dim.placement.y_axis = [-s, c, 0.];
    let encoded = encode_ifccad_document(&d).unwrap();
    let (mut cad, losses) = Ifcx.read(encoded.bytes()).unwrap();
    let name = cad
        .model_space_entities()
        .find_map(|e| match e {
            EntityType::Dimension(d) => Some(d.base().block_name.clone()),
            _ => None,
        })
        .unwrap();
    assert!(!name.is_empty(), "{losses:?}");
    let last = cad.entities_in_block(&name).last().unwrap().common().handle;
    cad.get_entity_mut(last).unwrap().common_mut().shadow_flags = 1;
    let (bytes, losses) = Ifcx.write(&cad).unwrap();
    let mut native = numerics::strict_native(&bytes);
    assert!(dimension(&mut native).graphics.is_none());
    assert!(native.blocks.iter().all(|b| b.name != "*U1"));
    assert!(
        losses
            .iter()
            .any(|l| l.message.contains("graphics") && l.message.contains("not converted")),
        "{losses:?}"
    );
    assert!(
        !losses
            .iter()
            .any(|l| l.message.contains("retained with numerical rounding")),
        "{losses:?}"
    );
}

#[test]
fn rounded_text_and_marker_frames_do_not_require_extent_proofs() {
    let mut d = numerics::native_source(&numerics::spatial_case("wide_nested"));
    d.model.entities = d
        .blocks
        .iter()
        .find(|b| b.name == "Level0")
        .unwrap()
        .entities
        .clone();
    d.blocks
        .retain(|b| matches!(b.name.as_str(), "Marker" | "Picture counterparts"));
    let dim = dimension(&mut d);
    let old = &dim.graphics.as_ref().unwrap().parts;
    let parts = vec![old[9].clone(), old[3].clone()];
    dim.graphics.as_mut().unwrap().parts = parts;
    let (s, c) = 0.37_f64.sin_cos();
    dim.placement.x_axis = [c, s, 0.];
    dim.placement.y_axis = [-s, c, 0.];
    let encoded = encode_ifccad_document(&d).unwrap();
    numerics::strict_native(encoded.bytes());
    let (cad, losses) = Ifcx.read(encoded.bytes()).unwrap();
    numerics::assert_retained_or_empty(&losses);
    let name = cad
        .model_space_entities()
        .find_map(|e| match e {
            EntityType::Dimension(d) => Some(&d.base().block_name),
            _ => None,
        })
        .unwrap();
    assert!(!name.is_empty());
    let parts: Vec<_> = cad.entities_in_block(name).collect();
    assert_eq!(parts.len(), 2);
    let EntityType::Text(text) = parts[0] else {
        panic!()
    };
    let EntityType::Insert(marker) = parts[1] else {
        panic!()
    };
    assert_eq!(text.insertion_point.x, 16. + (2. * c - 4. * s));
    assert_eq!(text.insertion_point.y, 8. + (2. * s + 4. * c));
    assert_eq!(marker.insert_point.x, 16. + (9. * c - 11. * s));
    assert_eq!(marker.insert_point.y, 8. + (9. * s + 11. * c));
    let (bytes, losses) = Ifcx.write(&cad).unwrap();
    numerics::assert_retained_or_empty(&losses);
    let mut native = numerics::strict_native(&bytes);
    assert_eq!(
        dimension(&mut native)
            .graphics
            .as_ref()
            .unwrap()
            .parts
            .len(),
        2
    );
}

#[test]
fn overflowing_picture_output_still_omits_atomically() {
    let mut d = source();
    single_line(&mut d);
    let mut cad = numerics::read_native(&d);
    let handle = cad
        .model_space_entities()
        .find_map(|e| matches!(e, EntityType::Dimension(_)).then_some(e.common().handle))
        .unwrap();
    let EntityType::Dimension(dim) = cad.get_entity_mut(handle).unwrap() else {
        panic!()
    };
    let name = dim.base().block_name.clone();
    let opencaddocument::entities::Dimension::Linear(dim) = dim.as_mut() else {
        panic!()
    };
    dim.definition_point = opencaddocument::types::Vector3::new(-1e308, 0., 0.);
    dim.base.definition_point = dim.definition_point;
    dim.first_point = dim.definition_point;
    dim.second_point = opencaddocument::types::Vector3::new(-9e307, 0., 0.);
    let handle = cad.entities_in_block(&name).next().unwrap().common().handle;
    let EntityType::Line(line) = cad.get_entity_mut(handle).unwrap() else {
        panic!()
    };
    line.start = opencaddocument::types::Vector3::new(1e308, 0., 0.);
    line.end = line.start;
    let (bytes, losses) = Ifcx.write(&cad).unwrap();
    let mut native = numerics::strict_native(&bytes);
    assert!(dimension(&mut native).graphics.is_none());
    assert!(
        losses
            .iter()
            .any(|l| l.message.contains("nonfinite picture geometry")),
        "{losses:?}"
    );
    assert!(!losses
        .iter()
        .any(|l| l.message.contains("retained with numerical rounding")));
}

#[test]
fn shared_marker_pictures_get_at_most_one_numeric_note_each() {
    let mut d = source();
    dimension(&mut d).placement.origin = [1e12, 0., 0.];
    let dim = dimension(&mut d);
    dim.graphics.as_mut().unwrap().parts[0].kind = IfccadDimensionGraphicsKind::LineSegment {
        start: [0.1, 0., 0.],
        end: [1., 0., 0.],
        thickness: 0.,
        extrusion_direction: [0., 0., 1.],
    };
    let mut second = d
        .model
        .entities
        .iter()
        .find(|e| matches!(e.as_native().unwrap().kind, IfccadEntityKind::Dimension(_)))
        .unwrap()
        .clone();
    second.as_native_mut().unwrap().id = d.id_counters.allocate_entity_id().unwrap();
    d.model.entities.push(second);
    let ids: Vec<_> = d
        .model
        .entities
        .iter()
        .filter(|e| matches!(e.as_native().unwrap().kind, IfccadEntityKind::Dimension(_)))
        .map(|e| e.id().to_string())
        .collect();
    let encoded = encode_ifccad_document(&d).unwrap();
    let (cad, losses) = Ifcx.read(encoded.bytes()).unwrap();
    let notes: Vec<_> = losses
        .iter()
        .filter(|l| l.message.contains("retained with numerical rounding"))
        .collect();
    assert_eq!(notes.len(), 2, "{losses:?}");
    for id in ids {
        assert_eq!(
            notes
                .iter()
                .filter(|n| n.message.contains(&format!("dimension {id} graphics")))
                .count(),
            1
        );
    }
    assert_eq!(
        cad.block_records
            .iter()
            .filter(|b| b.name == "Marker")
            .count(),
        1
    );
    assert_eq!(
        cad.model_space_entities()
            .filter(|e| matches!(e,EntityType::Dimension(d) if !d.base().block_name.is_empty()))
            .count(),
        2
    );
}
