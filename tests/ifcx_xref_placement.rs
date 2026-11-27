#![cfg(all(feature = "ifcx", feature = "mline-pilot"))]
use opencaddocument::{ifccad::*, *};

#[test]
fn oblique_signed_nonuniform_cad_xref_is_not_discarded() {
    let drawing = load_ifccad_bytes(
        include_bytes!("fixtures/xref/attachment.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    let imported = Ifcx.import_ifccad(&drawing).unwrap();
    let handle = *imported.entity_handles.get(&1000).unwrap();
    let mut cad = imported.document;
    let EntityType::Insert(insert) = cad.get_entity_mut(handle).unwrap() else {
        panic!()
    };
    insert.insert_point = Vector3::new(4., 5., 6.);
    insert.normal = Vector3::new(0., 1., 0.);
    insert.rotation = 0.5;
    insert.set_x_scale(-2.);
    insert.set_y_scale(3.);
    insert.set_z_scale(4.);
    let out = Ifcx.export_ifccad(&cad).unwrap();
    let x = out
        .drawing
        .model
        .entities
        .iter()
        .find_map(|e| match &e.as_native()?.kind {
            IfccadEntityKind::Xref(x) => Some(x),
            _ => None,
        })
        .unwrap();
    assert_eq!(x.transform.placement.origin, [-4., 6., 5.]);
    assert_eq!(x.transform.scale, [-2., 3., 4.]);
    assert_eq!(x.transform.rotation, 0.5);
    assert_eq!(x.transform.placement.x_axis, [-1., 0., 0.]);
    assert_eq!(x.transform.placement.y_axis, [0., 0., 1.]);
    assert!(
        out.drawing.model.entities.iter().any(|e| matches!(
            e.as_native().map(|e| &e.kind),
            Some(IfccadEntityKind::Xref(_))
        )),
        "XREF was lost: {:?}",
        out.losses
    );
}

#[path = "support/xref_placement.rs"]
mod support;
use support::mixed_owners;

fn xrefs(d: &IfccadDocument) -> Vec<&IfccadXref> {
    d.model
        .entities
        .iter()
        .chain(d.paper_layouts.iter().flat_map(|p| &p.entities))
        .chain(d.blocks.iter().flat_map(|b| &b.entities))
        .filter_map(|e| match &e.as_native()?.kind {
            IfccadEntityKind::Xref(x) => Some(x),
            _ => None,
        })
        .collect()
}

// Independent evaluation in owner coordinates; compare meaning rather than frame decomposition.
fn point(x: &IfccadXref, p: [f64; 3], base: [f64; 3]) -> [f64; 3] {
    let t = &x.transform;
    let q = std::array::from_fn::<_, 3, _>(|i| (p[i] - base[i]) * t.scale[i]);
    let (c, s) = (t.rotation.cos(), t.rotation.sin());
    let q = [c * q[0] - s * q[1], s * q[0] + c * q[1], q[2]];
    let a = t.placement.x_axis;
    let b = t.placement.y_axis;
    let z = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    std::array::from_fn(|i| t.placement.origin[i] + a[i] * q[0] + b[i] * q[1] + z[i] * q[2])
}
fn assert_owners(d: &IfccadDocument, sources: usize) {
    assert_eq!(d.external_sources.len(), sources);
    assert_eq!(xrefs(d).len(), 5);
    assert_eq!(
        d.paper_layouts
            .iter()
            .filter(|p| p.entities.iter().any(|e| matches!(
                e.as_native().map(|e| &e.kind),
                Some(IfccadEntityKind::Xref(_))
            )))
            .count(),
        2
    );
    for name in ["LOCAL", "UNUSED"] {
        assert_eq!(
            d.blocks
                .iter()
                .find(|b| b.name == name)
                .unwrap()
                .entities
                .len(),
            1
        );
    }
    assert_eq!(
        xrefs(d)
            .iter()
            .filter(|x| x.reference_type == IfccadXrefReferenceType::Overlay)
            .count(),
        2
    );
    for x in xrefs(d) {
        let p = point(x, [10., 20., 0.], [10., 20., 0.]);
        assert_eq!(p, [10., 20., 30.]);
        let p = point(x, [11., 20., 0.], [10., 20., 0.]);
        for (a, b) in p
            .into_iter()
            .zip([11.755165123780746, 20., 29.041148922791592])
        {
            assert!((a - b).abs() < 1e-10, "{p:?}");
        }
    }
    validate_ifccad_document(d).unwrap();
    let bytes = encode_ifccad_document(d).unwrap();
    load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
}
#[test]
fn all_owners_and_relationships_survive_preserving_and_physical_cad() {
    let d = mixed_owners();
    assert_owners(&d, 1);
    let (imported, mut context) = Ifcx.import_ifccad_preserving(&d).unwrap();
    assert!(imported.losses.is_empty(), "{:?}", imported.losses);
    assert_eq!(imported.document.file_dependencies.len(), 1);
    assert_eq!(imported.document.file_dependencies[0].reference_count, 5);
    let back = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap();
    assert!(back.losses.is_empty(), "{:?}", back.losses);
    assert_owners(&back.drawing, 1);
    assert_eq!(back.drawing.external_sources, d.external_sources);
    for (a, b) in xrefs(&back.drawing).iter().zip(xrefs(&d)) {
        assert_eq!(a.source_id, b.source_id);
    }
    for format in [&Dxf as &dyn Format, &Dwg as &dyn Format] {
        let (bytes, losses) = format.write(&imported.document).unwrap();
        assert!(losses.is_empty(), "{losses:?}");
        let (cad, _) = format.read(&bytes).unwrap();
        let out = Ifcx.export_ifccad(&cad).unwrap();
        assert!(
            !out.losses.iter().any(|l| l.message.contains("XREF")),
            "{:?}",
            out.losses
        );
        assert!(
            !out.losses.iter().any(|l| l.message.contains("FileDepList")),
            "generated metadata must not cause a loss: {:?}",
            out.losses
        );
        assert_owners(&out.drawing, 2);
    }
}

#[test]
fn fully_oblique_signed_ocs_insert_has_independent_world_witnesses() {
    for scales in [[-2., 3., 4.], [2., -3., 4.], [2., 3., -4.], [-2., -3., 4.]] {
        let d = load_ifccad_bytes(
            include_bytes!("fixtures/xref/attachment.ifcx"),
            Default::default(),
        )
        .unwrap()
        .into_document();
        let mut cad = Ifcx.import_ifccad(&d).unwrap().document;
        let h = cad
            .model_space_entities()
            .find(|e| matches!(e, EntityType::Insert(_)))
            .unwrap()
            .common()
            .handle;
        let EntityType::Insert(i) = cad.get_entity_mut(h).unwrap() else {
            panic!()
        };
        i.insert_point = Vector3::new(4., 5., 6.);
        i.normal = Vector3::new(1., 2., 3.);
        i.rotation = 0.5;
        i.set_x_scale(scales[0]);
        i.set_y_scale(scales[1]);
        i.set_z_scale(scales[2]);
        let x = [-2. / 5f64.sqrt(), 1. / 5f64.sqrt(), 0.];
        let y = [-3. / 70f64.sqrt(), -6. / 70f64.sqrt(), 5. / 70f64.sqrt()];
        let z = [1. / 14f64.sqrt(), 2. / 14f64.sqrt(), 3. / 14f64.sqrt()];
        let origin = std::array::from_fn::<_, 3, _>(|k| 4. * x[k] + 5. * y[k] + 6. * z[k]);
        for format in [&Dxf as &dyn Format, &Dwg as &dyn Format] {
            let (bytes, losses) = format.write(&cad).unwrap();
            assert!(losses.is_empty());
            let (read, _) = format.read(&bytes).unwrap();
            let out = Ifcx.export_ifccad(&read).unwrap();
            assert!(out.losses.is_empty(), "{:?}", out.losses);
            let refs = xrefs(&out.drawing);
            assert_eq!(refs.len(), 1);
            let r = refs[0];
            for (a, b) in r.transform.placement.origin.into_iter().zip(origin) {
                assert!((a - b).abs() < 1e-10);
            }
            for axis in 0..3 {
                let mut p = [10., 20., 0.];
                p[axis] += 1.;
                let actual = point(r, p, [10., 20., 0.]);
                let want = std::array::from_fn::<_, 3, _>(|k| {
                    origin[k]
                        + scales[axis]
                            * match axis {
                                0 => 0.5f64.cos() * x[k] + 0.5f64.sin() * y[k],
                                1 => -0.5f64.sin() * x[k] + 0.5f64.cos() * y[k],
                                _ => z[k],
                            }
                });
                for (a, b) in actual.into_iter().zip(want) {
                    assert!((a - b).abs() < 1e-10, "axis{axis}: {actual:?} != {want:?}");
                }
            }
            validate_ifccad_document(&out.drawing).unwrap();
        }
    }
}
#[test]
fn setter_projection_reports_loss_on_every_axis_instead_of_clamping_silently() {
    for axis in 0..3 {
        let mut d = load_ifccad_bytes(
            include_bytes!("fixtures/xref/attachment.ifcx"),
            Default::default(),
        )
        .unwrap()
        .into_document();
        let IfccadEntityKind::Xref(x) = &mut d.model.entities[0].as_native_mut().unwrap().kind
        else {
            panic!()
        };
        x.transform.scale[axis] = -1e-30;
        validate_ifccad_document(&d).unwrap();
        let imported = Ifcx.import_ifccad(&d).unwrap();
        assert!(
            imported
                .losses
                .iter()
                .any(|l| l.message.contains("XREF scale cannot be represented")),
            "{:?}",
            imported.losses
        );
        assert!(!imported
            .document
            .model_space_entities()
            .any(|e| matches!(e, EntityType::Insert(_))));
    }
}
