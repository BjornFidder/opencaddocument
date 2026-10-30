#![cfg(feature = "ifcx")]
use opencaddocument::{types::Vector3, *};
#[test]
fn circle_thickness_is_retained_in_both_directions() {
    let mut d = Document::new();
    let mut c = Circle::from_center_radius(Vector3::ZERO, 2.);
    c.thickness = -2.;
    d.add_entity(EntityType::Circle(c)).unwrap();
    let native = Ifcx.export_ifccad(&d).unwrap();
    let k = &native.drawing.model.entities[0].as_native().unwrap().kind;
    assert!(matches!(k,ifccad::IfccadEntityKind::Circle{thickness,..} if *thickness == -2.));
    let back = Ifcx.import_ifccad(&native.drawing).unwrap();
    let EntityType::Circle(c) = back.document.model_space_entities().next().unwrap() else {
        panic!()
    };
    assert_eq!(c.thickness, -2.);
}

#[test]
fn classic_ascii_defaults_and_explicit_zero_keep_effective_widths() {
    let (cad, _) = Dxf
        .read(include_bytes!("fixtures/extrusion-classic.dxf"))
        .unwrap();
    let out = Ifcx.export_ifccad(&cad).unwrap();
    let k = &out.drawing.model.entities[0].as_native().unwrap().kind;
    let ifccad::IfccadEntityKind::PlanarPolyline {
        widths,
        vertices,
        thickness,
        ..
    } = k
    else {
        panic!("classic polyline omitted")
    };
    assert_eq!(*thickness, 4.);
    assert_eq!(
        widths.effective_widths(vertices.len()).unwrap(),
        vec![[0.25, 0.5], [0., 0.5], [0.25, 0.]]
    );
    assert!(out
        .losses
        .iter()
        .any(|l| l.message.contains("default/override")));
    // DWG independently transports the codec's numeric vertex widths.
    let bytes = Dwg.write(&cad).unwrap().0;
    let (read, _) = Dwg.read(&bytes).unwrap();
    let out = Ifcx.export_ifccad(&read).unwrap();
    let ifccad::IfccadEntityKind::PlanarPolyline {
        widths,
        vertices,
        thickness,
        ..
    } = &out.drawing.model.entities[0].as_native().unwrap().kind
    else {
        panic!("classic DWG omitted")
    };
    assert_eq!(*thickness, 4.);
    assert_eq!(
        widths.effective_widths(vertices.len()).unwrap(),
        vec![[0.25, 0.5], [0., 0.5], [0.25, 0.]]
    );
}
