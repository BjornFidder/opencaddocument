#![cfg(feature = "ifcx")]
#[path = "../examples/support/dimension_cases.rs"]
mod cases;
use ifccad::{dimension::*, *};
use opencaddocument::{Document, DwgWriter, DxfWriter, Ifcx};
fn physical(d: &Document, kind: &str) -> Document {
    let bytes = match kind {
        "dwg" => DwgWriter::write_to_vec(d).unwrap(),
        "binary" => DxfWriter::new_binary(d).write_to_vec().unwrap(),
        _ => DxfWriter::new(d).write_to_vec().unwrap(),
    };
    let out = if kind == "dwg" {
        opencaddocument::DwgReader::from_stream(std::io::Cursor::new(bytes))
            .read_with_stats()
            .unwrap()
    } else {
        opencaddocument::DxfReader::from_reader(std::io::Cursor::new(bytes))
            .unwrap()
            .read_with_stats()
            .unwrap()
    };
    assert_eq!(out.stats.recovered_errors, 0);
    assert_eq!(out.stats.skipped_source_records, 0);
    assert!(out.stats.stream_completed);
    out.document
}
fn geometry(kind: DimensionKind, p: [f64; 3]) -> IfccadDimensionContextGeometry {
    use IfccadDimensionContextGeometry as G;
    match kind {
        DimensionKind::Linear => G::Linear {
            dimension_line_point: p,
        },
        DimensionKind::Aligned => G::Aligned {
            dimension_line_point: p,
        },
        DimensionKind::AngularTwoLines => G::AngularTwoLines {
            dimension_arc_point: p,
        },
        DimensionKind::AngularThreePoints => G::AngularThreePoints {
            dimension_arc_point: p,
        },
        DimensionKind::Radius => G::Radius { curve_point: p },
        DimensionKind::Diameter => G::Diameter {
            first_diameter_point: p,
            second_diameter_point: [40., 50., 60.],
        },
        DimensionKind::Ordinate => G::Ordinate {
            feature_point: p,
            leader_end: [40., 50., 60.],
        },
        DimensionKind::ArcLength => G::ArcLength {
            dimension_arc_point: p,
        },
        DimensionKind::JoggedRadius => G::JoggedRadius {
            display_center: p,
            jog_point: [40., 50., 60.],
        },
    }
}
fn points(g: &IfccadDimensionContextGeometry) -> Vec<[f64; 3]> {
    use IfccadDimensionContextGeometry::*;
    match g {
        Linear {
            dimension_line_point,
        }
        | Aligned {
            dimension_line_point,
        } => vec![*dimension_line_point],
        AngularTwoLines {
            dimension_arc_point,
        }
        | AngularThreePoints {
            dimension_arc_point,
        }
        | ArcLength {
            dimension_arc_point,
        } => vec![*dimension_arc_point],
        Radius { curve_point } => vec![*curve_point],
        Diameter {
            first_diameter_point,
            second_diameter_point,
        } => vec![*first_diameter_point, *second_diameter_point],
        Ordinate {
            feature_point,
            leader_end,
        } => vec![*feature_point, *leader_end],
        JoggedRadius {
            display_center,
            jog_point,
        } => vec![*display_center, *jog_point],
    }
}
fn position(p: [f64; 3], f: &IfccadPlacement) -> [f64; 3] {
    let x = f.x_axis;
    let y = f.y_axis;
    let z = [
        x[1] * y[2] - x[2] * y[1],
        x[2] * y[0] - x[0] * y[2],
        x[0] * y[1] - x[1] * y[0],
    ];
    std::array::from_fn(|i| f.origin[i] + p[0] * x[i] + p[1] * y[i] + p[2] * z[i])
}
fn dimensions(d: &IfccadDocument) -> Vec<&IfccadDimension> {
    d.model
        .entities
        .iter()
        .filter_map(|e| match &e.as_native()?.kind {
            IfccadEntityKind::Dimension(v) => Some(v.as_ref()),
            _ => None,
        })
        .collect()
}
#[test]
fn all_nine_dimension_kinds_keep_two_xyz_contexts_layout_and_independent_pictures_through_physical_cad(
) {
    let mut native = cases::gallery();
    assert_eq!(dimensions(&native).len(), 9);
    let _ = cases::dimension(&native);
    native.annotation_scales = (1..=2)
        .map(|i| IfccadAnnotationScale {
            id: IfccadAnnotationScaleId(i),
            name: format!("1:{}", i * 50),
            paper_units: 1.,
            drawing_units: (i * 50) as f64,
            unit_scale: false,
        })
        .collect();
    native.id_counters.next_annotation_scale_id = 3;
    for entity in &mut native.model.entities {
        let Some(IfccadNativeEntity {
            kind: IfccadEntityKind::Dimension(d),
            ..
        }) = entity.as_native_mut()
        else {
            continue;
        };
        let layout = IfccadDimensionContextLayout {
            line_outside: IfccadStoredBoolOverride {
                active: false,
                value: true,
            },
            suppress_arrows_outside: IfccadStoredBoolOverride {
                active: true,
                value: false,
            },
            fit_switch: IfccadStoredBoolOverride {
                active: false,
                value: true,
            },
            force_text_inside: IfccadStoredBoolOverride {
                active: true,
                value: true,
            },
            text_movement_switch: IfccadStoredBoolOverride {
                active: true,
                value: false,
            },
            second_arrow_present: true,
            flip_first: false,
            flip_second: true,
        };
        let mut picture = d.graphics.clone().unwrap();
        picture.origin = IfccadDimensionGraphicsOrigin::SourceSnapshot;
        d.annotation = Some(Box::new(IfccadAnnotation {
            mode: IfccadAnnotationMode::Enabled,
            orientation: Some(IfccadAnnotationOrientation::Fixed),
            reference: Some(IfccadAnnotationContextKey {
                scale_id: IfccadAnnotationScaleId(2),
            }),
            contexts: (1..=2)
                .map(|i| {
                    let mut picture = picture.clone();
                    if i == 2 {
                        picture.parts.pop();
                    }
                    IfccadAnnotationContext {
                        key: IfccadAnnotationContextKey {
                            scale_id: IfccadAnnotationScaleId(i),
                        },
                        presentation: Some(IfccadDimensionAnnotationContext {
                            text_location: [100. + i as f64, 200.],
                            user_text_location: false,
                            text_rotation: 0.25,
                            layout,
                            geometry: geometry(d.definition.kind(), [10. + i as f64, 20., 30.]),
                            graphics: Some(picture),
                        }),
                    }
                })
                .collect(),
        }));
    }
    recompute_ifccad_document_bounds(&mut native).unwrap();
    let imported = Ifcx.import_ifccad(&native).unwrap();
    assert_eq!(imported.document.objects.values().filter(|o|matches!(o,opencaddocument::objects::ObjectType::ObjectContextData(c) if matches!(c.kind,opencaddocument::objects::ObjectContextKind::Dim(_)))).count(),18);
    for kind in ["memory", "dwg", "ascii", "binary"] {
        let cad = if kind == "memory" {
            imported.document.clone()
        } else {
            physical(&imported.document, kind)
        };
        let exported = Ifcx.export_ifccad(&cad).unwrap().drawing;
        assert_eq!(dimensions(&exported).len(), 9);
        for (source, actual) in dimensions(&native).into_iter().zip(dimensions(&exported)) {
            assert_eq!(source.definition.kind(), actual.definition.kind());
            assert_eq!(
                source.graphics.as_ref().unwrap().parts.len(),
                actual.graphics.as_ref().unwrap().parts.len()
            );
            let a = actual.annotation.as_ref().unwrap();
            assert_eq!(a.mode, IfccadAnnotationMode::Enabled);
            assert_eq!(a.contexts.len(), 2);
            assert_eq!(a.reference, Some(a.contexts[1].key));
            for (source, c) in source
                .annotation
                .as_ref()
                .unwrap()
                .contexts
                .iter()
                .zip(&a.contexts)
            {
                let p = source.presentation.as_ref().unwrap();
                let q = c.presentation.as_ref().unwrap();
                assert_eq!(p.layout, q.layout);
                assert_eq!(q.text_rotation, p.text_rotation);
                assert_eq!(q.user_text_location, p.user_text_location);
                assert_eq!(
                    p.graphics.as_ref().unwrap().parts.len(),
                    q.graphics.as_ref().unwrap().parts.len()
                );
                for (x, y) in points(&p.geometry).into_iter().zip(points(&q.geometry)) {
                    let x = position(x, &source_frame(&native, actual.definition.kind()));
                    let y = position(y, &actual.placement);
                    for k in 0..3 {
                        assert!(
                            (x[k] - y[k]).abs() < 1e-10,
                            "{kind} {:?}: {x:?} -> {y:?}",
                            actual.definition.kind()
                        );
                    }
                }
            }
        }
        validate_ifccad_document(&exported).unwrap();
        let bytes = encode_ifccad_document(&exported).unwrap();
        load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
        let bytes = encode_ifccad_cbor_document(&exported).unwrap();
        load_ifccad_cbor_bytes(bytes.bytes(), Default::default()).unwrap();
    }
}
fn source_frame(d: &IfccadDocument, kind: DimensionKind) -> IfccadPlacement {
    dimensions(d)
        .into_iter()
        .find(|d| d.definition.kind() == kind)
        .unwrap()
        .placement
        .clone()
}
