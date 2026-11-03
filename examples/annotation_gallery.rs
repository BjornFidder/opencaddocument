//! Compact manual-review gallery for the pinned annotation development pair.
#[path = "support/dimension_cases.rs"]
mod dimension_cases;
use ifccad::geometry_kernel::hatch::HatchBoundary2;
use ifccad::{dimension::*, text::*, *};
use opencaddocument::{Document, DwgReader, DwgWriter, DxfReader, DxfWriter, Ifcx};
use std::{fs, io::Cursor, path::Path};

fn key(id: u64) -> IfccadAnnotationContextKey {
    IfccadAnnotationContextKey {
        scale_id: IfccadAnnotationScaleId(id),
    }
}
fn annotation<P>(presentations: Vec<P>) -> Box<IfccadAnnotation<P>> {
    Box::new(IfccadAnnotation {
        mode: IfccadAnnotationMode::Enabled,
        orientation: Some(IfccadAnnotationOrientation::Fixed),
        reference: Some(key(1)),
        contexts: presentations
            .into_iter()
            .enumerate()
            .map(|(i, p)| IfccadAnnotationContext {
                key: key(i as u64 + 1),
                presentation: Some(p),
            })
            .collect(),
    })
}
fn read(bytes: &[u8]) -> IfccadDocument {
    load_ifccad_bytes(bytes, Default::default())
        .unwrap()
        .into_document()
}
fn geometry(d: &DimensionDefinition) -> IfccadDimensionContextGeometry {
    let mut v = serde_json::to_value(d).unwrap();
    let kind = v["kind"].as_str().unwrap().to_owned();
    let names: &[&str] = match d.kind() {
        DimensionKind::Linear | DimensionKind::Aligned => &["dimensionLinePoint"],
        DimensionKind::AngularTwoLines
        | DimensionKind::AngularThreePoints
        | DimensionKind::ArcLength => &["dimensionArcPoint"],
        DimensionKind::Radius => &["curvePoint"],
        DimensionKind::Diameter => &["firstDiameterPoint", "secondDiameterPoint"],
        DimensionKind::Ordinate => &["featurePoint", "leaderEnd"],
        DimensionKind::JoggedRadius => &["displayCenter", "jogPoint"],
    };
    let mut result = serde_json::json!({"kind":kind});
    for name in names {
        let p = v[*name].as_array_mut().unwrap();
        if p.len() == 2 {
            p.push(0.into());
        }
        result[*name] = v[*name].clone();
    }
    serde_json::from_value(result).unwrap()
}
fn gallery() -> IfccadDocument {
    let mut d = read(include_bytes!("../tests/fixtures/annotation-blocks.ifcx"));
    d.header.id = "annotation-contexts-review".into();
    d.header.author = "OpenAEC".into();
    d.header.timestamp = "2026-11-02T00:00:00Z".into();
    d.length_unit = "cm".into();
    d.annotation_scales = (1..=2)
        .map(|i| IfccadAnnotationScale {
            id: IfccadAnnotationScaleId(i),
            name: format!("1:{i}"),
            paper_units: 1.,
            drawing_units: i as f64,
            unit_scale: i == 1,
        })
        .collect();
    d.id_counters.next_annotation_scale_id = 3;
    d.annotation_settings = Some(IfccadDrawingAnnotationSettings {
        insert_as_annotative: Some(false),
        model_linetype_scaling: Some(IfccadModelLinetypeAnnotationMode::ByAnnotationScale),
    });
    d.model.annotation_state = Some(IfccadLayoutAnnotationState {
        current_scale_id: Some(key(1).scale_id),
        visibility: Some(IfccadAnnotationVisibility::All),
    });
    for block in &mut d.blocks {
        block.annotation.enabled = block.id == 2;
    }
    let mut column = 0;
    d.model.entities.retain_mut(|e| {
        let Some(n) = e.as_native_mut() else {
            return false;
        };
        let IfccadEntityKind::BlockInstance {
            annotation: a,
            transform,
            ..
        } = &mut n.kind
        else {
            return false;
        };
        transform.placement.origin = [column as f64 * 30., -90., 0.];
        transform.rotation = 0.;
        transform.scale = [1.; 3];
        let mut second = transform.clone();
        second.scale = [2.; 3];
        *a = Some(annotation(vec![
            IfccadBlockAnnotationContext {
                transform: transform.clone(),
            },
            IfccadBlockAnnotationContext { transform: second },
        ]));
        column += 1;
        true
    });
    let dims = dimension_cases::gallery();
    let _ = dimension_cases::dimension(&dims);
    d.dimension_styles = dims.dimension_styles;
    d.text_styles = dims.text_styles;
    d.id_counters.next_dimension_style_id = dims.id_counters.next_dimension_style_id;
    d.id_counters.next_text_style_id = dims.id_counters.next_text_style_id;
    for s in &mut d.dimension_styles {
        s.annotative = true;
        s.settings.presentation_scale = DimensionPresentationScale::ByAnnotationScale {
            dormant_factor: None,
        };
    }
    for s in &mut d.text_styles {
        s.annotation.enabled = true;
    }
    for e in dims.model.entities {
        let IfccadEntity::Native(mut n) = e else {
            continue;
        };
        let IfccadEntityKind::Dimension(dim) = &mut n.kind else {
            continue;
        };
        n.id = d.id_counters.allocate_entity_id().unwrap();
        let p = dim.graphics.as_mut().unwrap();
        p.origin = IfccadDimensionGraphicsOrigin::SourceSnapshot;
        let stored = IfccadStoredBoolOverride {
            active: false,
            value: true,
        };
        let presentation = IfccadDimensionAnnotationContext {
            text_location: [4., 6.],
            user_text_location: true,
            text_rotation: 0.,
            layout: IfccadDimensionContextLayout {
                line_outside: stored,
                suppress_arrows_outside: stored,
                fit_switch: stored,
                force_text_inside: stored,
                text_movement_switch: stored,
                second_arrow_present: false,
                flip_first: false,
                flip_second: false,
            },
            geometry: geometry(&dim.definition),
            graphics: dim.graphics.clone(),
        };
        dim.annotation = Some(annotation(vec![presentation.clone(), presentation]));
        d.model.entities.push(IfccadEntity::Native(n));
    }
    let template = d.model.entities[0].as_native().unwrap().clone();
    let style_id = d.text_styles[0].id;
    for (i, horizontal) in [
        IfccadTextAnnotationHorizontal::Left,
        IfccadTextAnnotationHorizontal::Center,
        IfccadTextAnnotationHorizontal::Right,
        IfccadTextAnnotationHorizontal::Aligned,
        IfccadTextAnnotationHorizontal::Middle,
        IfccadTextAnnotationHorizontal::Fit,
    ]
    .into_iter()
    .enumerate()
    {
        let mut n = template.clone();
        n.id = d.id_counters.allocate_entity_id().unwrap();
        n.layer_id = 0;
        n.kind = IfccadEntityKind::Text(IfccadText {
            style_id,
            placement: IfccadPlacement {
                origin: [(i % 3) as f64 * 30., -65. - (i / 3) as f64 * 10., 0.],
                x_axis: [1., 0., 0.],
                y_axis: [0., 1., 0.],
            },
            rotation: 0.,
            backward: false,
            upside_down: false,
            layout: TextLayout::Anchored {
                horizontal: TextHorizontalAlignment::Left,
                vertical: TextVerticalAlignment::Baseline,
                height: 1.,
                width_factor: 1.,
            },
            oblique_angle: 0.,
            thickness: 0.,
            content: vec![TextRun {
                text: format!("TEXT {horizontal:?}"),
                ..Default::default()
            }],
            annotation: Some(annotation(
                (1..=2)
                    .map(|i| IfccadTextAnnotationContext {
                        insertion: [0., i as f64],
                        alignment: [12., i as f64],
                        rotation: 0.,
                        horizontal,
                    })
                    .collect(),
            )),
        });
        d.model.entities.push(IfccadEntity::Native(n));
    }
    let mut n = template.clone();
    n.id = d.id_counters.allocate_entity_id().unwrap();
    n.layer_id = 0;
    n.kind = IfccadEntityKind::MText(Box::new(IfccadMText {
        style_id,
        placement: IfccadPlacement {
            origin: [65., -85., 0.],
            x_axis: [1., 0., 0.],
            y_axis: [0., 1., 0.],
        },
        rotation: 0.,
        backward: false,
        upside_down: false,
        height: 1.,
        attachment: MTextAttachment::TopLeft,
        flow: Default::default(),
        wrap_width: Some(18.),
        columns: None,
        background: None,
        character_format: Default::default(),
        paragraph_format: Default::default(),
        content: vec![MTextParagraph {
            inlines: vec![MTextInline::Run {
                text: "MTEXT saved contexts".into(),
                character_format: Default::default(),
            }],
            ..Default::default()
        }],
        annotation: Some(annotation(
            (1..=2)
                .map(|i| IfccadMTextAnnotationContext {
                    insertion: [0., i as f64, 0.],
                    x_direction: [1., 0., 0.],
                    attachment: MTextAttachment::TopLeft,
                    rectangle_width: 18.,
                    rectangle_height: 10.,
                    columns: Some(IfccadMTextAnnotationColumns {
                        kind: IfccadAnnotationColumnKind::Dynamic,
                        count: 2,
                        width: 8.,
                        gutter: 1.,
                        auto_height: false,
                        flow_reversed: false,
                        heights: vec![10., 10.],
                    }),
                })
                .collect(),
        )),
    }));
    d.model.entities.push(IfccadEntity::Native(n));
    let hatch = read(include_bytes!("../tests/fixtures/annotation-hatch.ifcx"));
    for e in hatch.model.entities {
        let IfccadEntity::Native(mut n) = e else {
            continue;
        };
        let IfccadEntityKind::Hatch(h) = &mut n.kind else {
            continue;
        };
        n.id = d.id_counters.allocate_entity_id().unwrap();
        h.placement.origin = [85., -100., 0.];
        h.association_enabled = false;
        for l in &mut h.loops {
            l.source_entity_ids.clear();
        }
        // Keep the contextual island near its base for a compact manual gallery.
        let nearby = |p: &mut IfccadHatchAnnotationContext| {
            for l in &mut p.loops {
                if let IfccadHatchContextLoop::Local { boundary, .. } = l {
                    if let HatchBoundary2::Circle { center, .. } = &mut boundary.geometry {
                        *center = [0.; 2];
                    }
                }
            }
        };
        for c in &mut h.annotation.as_mut().unwrap().contexts {
            if let Some(p) = &mut c.presentation {
                nearby(p);
            }
        }
        for c in &mut h.annotation_view_contexts.as_mut().unwrap().contexts {
            nearby(&mut c.presentation);
        }
        let mut active = n.clone();
        active.id = d.id_counters.allocate_entity_id().unwrap();
        if let IfccadEntityKind::Hatch(h) = &mut active.kind {
            h.placement.origin[0] = 65.;
            h.annotation.as_mut().unwrap().mode = IfccadAnnotationMode::Enabled;
            let mut second = h.annotation.as_ref().unwrap().contexts[0].clone();
            second.key = key(2);
            second.presentation.as_mut().unwrap().pattern.scale = 3.;
            h.annotation.as_mut().unwrap().contexts.push(second);
            h.annotation_view_contexts.as_mut().unwrap().contexts[0].target =
                Some(IfccadAnnotationViewTarget::StoredView {
                    id: IfccadAnnotationViewId(0),
                });
        }
        // The second specimen deliberately keeps Disabled with dormant contexts.
        d.model.entities.push(IfccadEntity::Native(active));
        d.model.entities.push(IfccadEntity::Native(n));
    }
    let workspace = read(include_bytes!(
        "../tests/fixtures/annotation-workspace.ifcx"
    ));
    d.ucs_definitions = workspace.ucs_definitions;
    d.id_counters.next_ucs_id = workspace.id_counters.next_ucs_id;
    d.paper_layouts = workspace.paper_layouts;
    d.id_counters.next_layout_id = workspace.id_counters.next_layout_id;
    for p in &mut d.paper_layouts {
        p.name = "Annotation review".into();
        p.annotation_state = Some(IfccadLayoutAnnotationState {
            current_scale_id: Some(key(2).scale_id),
            visibility: Some(IfccadAnnotationVisibility::CurrentScaleOnly),
        });
        for e in &mut p.entities {
            let Some(n) = e.as_native_mut() else { continue };
            n.id = d.id_counters.allocate_entity_id().unwrap();
            if let IfccadEntityKind::Viewport(v) = &mut n.kind {
                v.paper_clip.enabled = false;
                v.paper_clip.boundary_entity_id = None;
                v.annotation_scale_id = Some(key(1).scale_id);
                v.view.center = [40., -45.];
                v.view.height = 120.;
                v.view.target = [0.; 3];
                d.annotation_views.push(IfccadAnnotationStoredView {
                    id: IfccadAnnotationViewId(0),
                    name: "Gallery saved view".into(),
                    space: IfccadAnnotationViewSpace::Model,
                    width: 110.,
                    view: v.view.clone(),
                    annotation_scale_id: Some(key(2).scale_id),
                });
            }
        }
    }
    recompute_ifccad_document_bounds(&mut d).unwrap();
    validate_ifccad_document(&d).unwrap();
    d
}
fn cad_read(bytes: Vec<u8>, dwg: bool) -> Document {
    let result = if dwg {
        DwgReader::from_stream(Cursor::new(bytes))
            .read_with_stats()
            .unwrap()
    } else {
        DxfReader::from_reader(Cursor::new(bytes))
            .unwrap()
            .read_with_stats()
            .unwrap()
    };
    assert!(result.stats.stream_completed);
    assert_eq!(result.stats.recovered_errors, 0);
    assert_eq!(result.stats.skipped_source_records, 0);
    result.document
}
fn losses(label: &str, values: &[opencaddocument::Loss]) -> String {
    format!(
        "{label}\n{}\n",
        values
            .iter()
            .map(|v| format!("{:?}: {}", v.handle, v.message))
            .collect::<Vec<_>>()
            .join("\n")
    )
}
fn annotation_signature(d: &IfccadDocument) -> serde_json::Value {
    let bytes = encode_ifccad_document(d).unwrap();
    let v: serde_json::Value = serde_json::from_slice(bytes.bytes()).unwrap();
    let mut values = vec![];
    for node in v["data"].as_array().unwrap() {
        let a = node["attributes"].as_object().unwrap();
        for name in [
            "ifccad::annotation",
            "ifccad::hatchAnnotationViews",
            "ifccad::annotationScale",
            "ifccad::annotationView",
        ] {
            if let Some(value) = a.get(name) {
                values.push(serde_json::json!([name, value]));
            }
        }
        for name in [
            "ifccad::drawing",
            "ifccad::layout",
            "ifccad::viewport",
            "ifccad::textStyle",
            "ifccad::blockDefinition",
            "ifccad::dimensionStyle",
        ] {
            for field in [
                "annotationSettings",
                "annotationState",
                "annotationScale",
                "annotation",
                "annotative",
            ] {
                if let Some(value) = a.get(name).and_then(|v| v.get(field)) {
                    values.push(serde_json::json!([name, field, value]));
                }
            }
        }
    }
    serde_json::Value::Array(values)
}
fn main() {
    let path = std::env::args().nth(1).expect("output directory");
    let out = Path::new(&path);
    fs::create_dir_all(out).unwrap();
    let native = gallery();
    fs::write(
        out.join("gallery-native.ifcx"),
        encode_ifccad_document(&native).unwrap().bytes(),
    )
    .unwrap();
    let input = Ifcx.import_ifccad(&native).unwrap();
    let mut notes = losses("native -> CAD", &input.losses);
    for (ext, bytes) in [
        ("dwg", DwgWriter::write_to_vec(&input.document).unwrap()),
        (
            "dxf",
            DxfWriter::new(&input.document).write_to_vec().unwrap(),
        ),
    ] {
        fs::write(out.join(format!("gallery-input.{ext}")), &bytes).unwrap();
        let cad = cad_read(bytes, ext == "dwg");
        let exported = Ifcx.export_ifccad(&cad).unwrap();
        notes += &losses(&format!("{ext} -> native"), &exported.losses);
        let json = encode_ifccad_document(&exported.drawing).unwrap();
        let loaded = load_ifccad_bytes(json.bytes(), Default::default())
            .unwrap()
            .into_document();
        fs::write(out.join(format!("gallery-from-{ext}.ifcx")), json.bytes()).unwrap();
        let cbor = encode_ifccad_cbor_document(&loaded).unwrap();
        load_ifccad_cbor_bytes(cbor.bytes(), Default::default()).unwrap();
        fs::write(
            out.join(format!("gallery-from-{ext}.ifccad.cbor")),
            cbor.bytes(),
        )
        .unwrap();
        let imported = Ifcx.import_ifccad(&loaded).unwrap();
        notes += &losses(&format!("native from {ext} -> CAD"), &imported.losses);
        let bytes = if ext == "dwg" {
            DwgWriter::write_to_vec(&imported.document).unwrap()
        } else {
            DxfWriter::new(&imported.document).write_to_vec().unwrap()
        };
        fs::write(out.join(format!("gallery-roundtrip.{ext}")), &bytes).unwrap();
        let final_cad = cad_read(bytes, ext == "dwg");
        let final_native = Ifcx.export_ifccad(&final_cad).unwrap();
        validate_ifccad_document(&final_native.drawing).unwrap();
        assert_eq!(
            annotation_signature(&loaded),
            annotation_signature(&final_native.drawing),
            "{ext}: annotation fields changed during the native/CAD roundtrip"
        );
        assert_eq!(final_native.drawing.annotation_scales.len(), 2);
        assert_eq!(
            final_native.drawing.model.entities.len(),
            loaded.model.entities.len()
        );
    }
    fs::write(out.join("diagnostics.txt"), notes).unwrap();
    println!("Review files written to {}", out.display());
}
