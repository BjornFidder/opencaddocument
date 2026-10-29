#![cfg(feature = "ifcx")]
use ifccad::{dimension::*, *};
use opencaddocument::{Dwg, DwgWriter, Dxf, DxfWriter, Format, Ifcx};
use serde_json::json;

#[test]
fn ordinary_new_kinds_survive_both_typed_directions() {
    let cases = [
        json!({"kind":"radius","center":[0.,0.,0.],"curvePoint":[3.,4.,0.],"leaderLength":0.}),
        json!({"kind":"diameter","firstDiameterPoint":[-3.,-4.,0.],"secondDiameterPoint":[3.,4.,0.],"leaderLength":0.}),
        json!({"kind":"angularTwoLines","line1Start":[0.,0.,0.],"line1End":[1.,0.,0.],"line2Start":[0.,0.,0.],"line2End":[0.,1.,0.],"ray1Sign":"positive","ray2Sign":"positive","traversal":"counterclockwise","dimensionArcPoint":[5.,5.]}),
        json!({"kind":"angularThreePoints","vertex":[0.,0.,0.],"firstPoint":[1.,0.,0.],"secondPoint":[0.,1.,0.],"traversal":"counterclockwise","dimensionArcPoint":[5.,5.]}),
        json!({"kind":"ordinate","origin":[0.,0.,0.],"featurePoint":[3.,4.,0.],"axisAngle":0.,"leaderEnd":[10.,10.],"valueMode":"absolute"}),
        json!({"kind":"arcLength","center":[0.,0.,0.],"radius":5.,"startParameter":0.,"sweep":{"kind":"partial","radians":std::f64::consts::FRAC_PI_2},"dimensionArcPoint":[6.,6.]}),
        json!({"kind":"joggedRadius","center":[0.,0.,0.],"curvePoint":[3.,4.,0.],"displayCenter":[1.,1.],"jogPoint":[2.,2.],"jogAngle":0.5}),
    ];
    for case in cases {
        let mut drawing = load_ifccad_bytes(
            include_bytes!("fixtures/dimension-linear-aligned.ifcx"),
            Default::default(),
        )
        .unwrap()
        .into_document();
        let dimension = drawing
            .model
            .entities
            .iter_mut()
            .filter_map(IfccadEntity::as_native_mut)
            .find_map(|e| match &mut e.kind {
                IfccadEntityKind::Dimension(v) => Some(v),
                _ => None,
            })
            .unwrap();
        let definition: DimensionDefinition = serde_json::from_value(case).unwrap();
        dimension.definition = definition.clone();
        dimension.associations.clear();
        dimension.graphics = None;
        dimension.placement = IfccadPlacement {
            origin: [0.; 3],
            x_axis: [1., 0., 0.],
            y_axis: [0., 1., 0.],
        };
        validate_ifccad_document(&drawing).unwrap();
        let imported = Ifcx.import_ifccad(&drawing).unwrap();
        let paths = [
            (
                "ascii",
                DxfWriter::new(&imported.document).write_to_vec().unwrap(),
            ),
            (
                "binary",
                DxfWriter::new_binary(&imported.document)
                    .write_to_vec()
                    .unwrap(),
            ),
            ("dwg", DwgWriter::write_to_vec(&imported.document).unwrap()),
        ];
        for (transport, bytes) in paths {
            if transport == "dwg" {
                assert_eq!(&bytes[..6], b"AC1032");
            }
            if transport == "binary" {
                assert!(bytes.starts_with(b"AutoCAD Binary DXF"));
            }
            let (readback, _) = if transport == "dwg" {
                Dwg.read(&bytes).unwrap()
            } else {
                Dxf.read(&bytes).unwrap()
            };
            let returned = Ifcx.export_ifccad(&readback).unwrap();
            validate_ifccad_document(&returned.drawing).unwrap();
            let actual = returned
                .drawing
                .model
                .entities
                .iter()
                .filter_map(IfccadEntity::as_native)
                .find_map(|e| match &e.kind {
                    IfccadEntityKind::Dimension(v) if v.definition.kind() == definition.kind() => {
                        Some(v)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| {
                    panic!(
                        "missing {:?}: import losses {:?}, export losses {:?}",
                        definition.kind(),
                        imported.losses,
                        returned.losses
                    )
                });
            let number = |v: DimensionMeasurement| match v {
                DimensionMeasurement::Distance(x) | DimensionMeasurement::Angle(x) => x,
            };
            let expected = number(evaluate_dimension(&definition).unwrap());
            let observed = number(evaluate_dimension(&actual.definition).unwrap());
            assert!(
                (expected - observed).abs() <= 2. * f64::EPSILON * expected.abs(),
                "{:?} {transport}: {expected} vs {observed}",
                definition.kind()
            );
        }
    }
}
