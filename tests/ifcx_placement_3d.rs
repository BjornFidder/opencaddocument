#![cfg(feature = "ifcx")]

#[path = "support/numerics.rs"]
mod numerics;
use ifccad::*;
use numerics::*;
use opencaddocument::{EntityType, Format, Ifcx};

#[test]
fn independent_reference_has_analytic_scale_and_free_circle_phase() {
    assert_reference_sanity();
}

#[test]
fn aa_boundary_preserves_the_first_native_world_geometry() {
    assert_three_cycles(&spatial_case("aa_x_below"), Transport::Direct, false);
}

#[test]
fn aa_x_and_y_boundaries_and_poles_preserve_world_geometry() {
    for case in cases()
        .into_iter()
        .filter(|c| c.name().starts_with("aa_") || c.name().contains("pole"))
    {
        assert_three_cycles(&case, Transport::Direct, false);
        assert_three_cycles(&case, Transport::Direct, true);
    }
}

#[test]
fn nested_mirrored_and_scaled_blocks_preserve_world_geometry() {
    for case in cases()
        .into_iter()
        .filter(|c| !c.name().starts_with("aa_") && !c.name().contains("pole"))
    {
        assert_three_cycles(&case, Transport::Direct, false);
        assert_three_cycles(&case, Transport::Direct, true);
    }
}

#[test]
fn physically_equivalent_units_preserve_the_same_world_geometry_in_mm() {
    let reference_case = spatial_case("nested_cm");
    let reference =
        native_world(&native_source(&reference_case)).ordinary_in_mm(reference_case.mm());
    for (unit, cad_unit) in [("mm", 4), ("cm", 5), ("m", 6), ("in", 1)] {
        let case = spatial_case(&format!("nested_{unit}"));
        let source = native_source(&case);
        native_world(&source)
            .ordinary_in_mm(case.mm())
            .assert_matches(&reference, 1., case.budget(), unit);
        let mut cad = read_native(&source);
        for _ in 0..3 {
            assert_eq!(cad.header.insertion_units, cad_unit);
            cad_world(&cad).ordinary_in_mm(case.mm()).assert_matches(
                &reference,
                1.,
                case.budget(),
                unit,
            );
            let (bytes, losses) = Ifcx.write(&cad).unwrap();
            assert!(losses.is_empty(), "{losses:?}");
            let native = strict_native(&bytes);
            assert_eq!(native.length_unit, unit);
            native_world(&native)
                .ordinary_in_mm(case.mm())
                .assert_matches(&reference, 1., case.budget(), unit);
            cad = read_native(&native);
        }
    }
}

#[test]
fn spatial_pictures_survive_three_real_dwg_and_dxf_cycles() {
    for case in cases().into_iter().filter(|c| c.data["transports"] == true) {
        for route in [Transport::AsciiDxf, Transport::BinaryDxf, Transport::Dwg] {
            assert_three_cycles(&case, route, false);
            assert_three_cycles(&case, route, true);
        }
    }
}

#[test]
fn cad_setters_never_silently_change_native_signed_scales() {
    for axis in 0..3 {
        for value in [-1e-14_f64, 1e-14, -1e-12, 1e-12, -2., 0.5] {
            let mut source = native_source(&spatial_case("aa_x_below"));
            let IfccadEntityKind::BlockInstance { transform, .. } =
                &mut source.model.entities[0].as_native_mut().unwrap().kind
            else {
                panic!()
            };
            transform.scale = [-2., 0.5, 1.];
            transform.scale[axis] = value;
            let expected = transform.scale;
            let bytes = encode_ifccad_document(&source).unwrap();
            strict_native(bytes.bytes());
            let (cad, losses) = Ifcx.read(bytes.bytes()).unwrap();
            let members: Vec<_> = cad.model_space_entities().collect();
            if value.abs() < 1e-12 {
                assert!(
                    members.is_empty(),
                    "axis {axis}: {value} must be omitted, not replaced"
                );
                assert!(losses.iter().any(|l|l.message.contains("scale") && l.message.contains("BlockInstance")),"{losses:?}");
            } else {
                assert!(losses.is_empty(), "{losses:?}");
                assert_eq!(members.len(), 1);
                let EntityType::Insert(i) = members[0] else {
                    panic!()
                };
                assert_eq!(
                    [i.x_scale(), i.y_scale(), i.z_scale()].map(f64::to_bits),
                    expected.map(f64::to_bits)
                );
            }
        }
    }
}

#[test]
fn unrepresentable_marker_scale_omits_the_whole_picture() {
    let mut source = native_source(&spatial_case("aa_x_below"));
    let leaf = source
        .blocks
        .iter_mut()
        .find(|b| b.name == "Level0")
        .unwrap();
    let dim = leaf
        .entities
        .iter_mut()
        .find_map(|e| match &mut e.as_native_mut().unwrap().kind {
            IfccadEntityKind::Dimension(d) => Some(d),
            _ => None,
        })
        .unwrap();
    let IfccadDimensionGraphicsKind::BlockInstance { transform, .. } =
        &mut dim.graphics.as_mut().unwrap().parts[3].kind
    else {
        panic!()
    };
    transform.scale[0] = -1e-14;
    let bytes = encode_ifccad_document(&source).unwrap();
    strict_native(bytes.bytes());
    let (cad, losses) = Ifcx.read(bytes.bytes()).unwrap();
    assert!(
        losses.iter().any(|l| l.message.contains("graphics")),
        "{losses:?}"
    );
    let dimensions: Vec<_> = cad
        .entities()
        .filter_map(|e| match e {
            EntityType::Dimension(d) => Some(d),
            _ => None,
        })
        .collect();
    assert_eq!(dimensions.len(), 1);
    assert!(dimensions[0].base().block_name.is_empty());
    assert_eq!(
        cad.entities()
            .filter(|e| matches!(e, EntityType::Insert(_)))
            .count(),
        1
    );
}

#[test]
fn model_paper_and_shared_block_owners_keep_local_semantics() {
    let case = spatial_case("nested_cm");
    let mut source = native_source(&case);
    let template = strict_native(include_bytes!("fixtures/hello-paper-layouts.ifcx"));
    let mut paper = template.paper_layouts[0].clone();
    paper.id = 2;
    paper.tab_index = 1;
    paper.name = "Placement sheet".into();
    paper.canvas = None;
    paper.entities = source.model.entities.clone();
    paper.entities[0].as_native_mut().unwrap().id =
        source.id_counters.allocate_entity_id().unwrap();
    source.paper_layouts = vec![paper];
    source.id_counters.next_layout_id = 3;
    for route in [
        Transport::Direct,
        Transport::AsciiDxf,
        Transport::BinaryDxf,
        Transport::Dwg,
    ] {
        let mut cad = read_native(&source);
        for _ in 0..3 {
            cad = transport(&cad, route);
            case.assert_world(&cad_world(&cad), "native", "model owner");
            let sheet = cad
                .objects
                .values()
                .find_map(|o| match o {
                    opencaddocument::objects::ObjectType::Layout(l)
                        if l.name == "Placement sheet" =>
                    {
                        Some(l)
                    }
                    _ => None,
                })
                .unwrap();
            let record = cad
                .block_records
                .iter()
                .find(|b| b.handle == sheet.block_record)
                .unwrap();
            case.assert_world(
                &cad_owner_world(&cad, Some(&record.name)),
                "native",
                "paper owner",
            );
            let (bytes, losses) = Ifcx.write(&cad).unwrap();
            assert!(losses.is_empty(), "{route:?}: {losses:?}");
            let native = strict_native(&bytes);
            assert_eq!(native.model.entities.len(), 1);
            assert_eq!(native.paper_layouts.len(), 1);
            assert_eq!(native.paper_layouts[0].entities.len(), 1);
            let model = &native.model.entities[0].as_native().unwrap().kind;
            let paper = &native.paper_layouts[0].entities[0]
                .as_native()
                .unwrap()
                .kind;
            let (
                IfccadEntityKind::BlockInstance {
                    definition_id: a, ..
                },
                IfccadEntityKind::BlockInstance {
                    definition_id: b, ..
                },
            ) = (model, paper)
            else {
                panic!()
            };
            assert_eq!(a, b, "shared definition must remain shared");
            case.assert_world(&native_world(&native), "native", "native model owner");
            let mut sheet = native.clone();
            sheet.model.entities = native.paper_layouts[0].entities.clone();
            case.assert_world(&native_world(&sheet), "native", "native paper owner");
            cad = read_native(&native);
        }
    }
}
