#![cfg(feature = "ifcx")]
use ifccad::*;
use opencaddocument::{Document, DwgWriter, DxfWriter, Ifcx};
fn native() -> IfccadDocument {
    let mut d = load_ifccad_bytes(
        include_bytes!("fixtures/hello-cad.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    d.annotation_scales = vec![
        IfccadAnnotationScale {
            id: IfccadAnnotationScaleId(7),
            name: "Saved unit designation".into(),
            paper_units: 2.,
            drawing_units: 100.,
            unit_scale: true,
        },
        IfccadAnnotationScale {
            id: IfccadAnnotationScaleId(9),
            name: "Distinct same ratio".into(),
            paper_units: 1.,
            drawing_units: 50.,
            unit_scale: false,
        },
    ];
    d.id_counters.next_annotation_scale_id = 12;
    d
}
fn scales(d: &IfccadDocument) -> Vec<(String, f64, f64, bool)> {
    let mut s = d
        .annotation_scales
        .iter()
        .map(|s| (s.name.clone(), s.paper_units, s.drawing_units, s.unit_scale))
        .collect::<Vec<_>>();
    s.sort_by(|a, b| a.0.cmp(&b.0));
    s
}
fn physical(d: &Document, kind: &str) -> Document {
    let bytes = match kind {
        "dwg" => DwgWriter::write_to_vec(d).unwrap(),
        "binary" => DxfWriter::new_binary(d).write_to_vec().unwrap(),
        _ => DxfWriter::new(d).write_to_vec().unwrap(),
    };
    let outcome = if kind == "dwg" {
        opencaddocument::DwgReader::from_stream(std::io::Cursor::new(bytes))
            .read_with_stats()
            .unwrap()
    } else {
        opencaddocument::DxfReader::from_reader(std::io::Cursor::new(bytes))
            .unwrap()
            .read_with_stats()
            .unwrap()
    };
    assert_eq!(outcome.stats.recovered_errors, 0);
    assert_eq!(outcome.stats.skipped_source_records, 0);
    assert!(outcome.stats.stream_completed);
    outcome.document
}
#[test]
fn unused_scales_survive_shared_mapping_and_physical_cad() {
    let native = native();
    let imported = Ifcx.import_ifccad(&native).unwrap();
    for kind in ["memory", "dwg", "ascii", "binary"] {
        let document = if kind == "memory" {
            imported.document.clone()
        } else {
            physical(&imported.document, kind)
        };
        let exported = Ifcx.export_ifccad(&document).unwrap();
        assert_eq!(
            scales(&exported.drawing),
            scales(&native),
            "{kind}: {:?}",
            exported.losses
        );
        validate_ifccad_document(&exported.drawing).unwrap();
        let wire = encode_ifccad_document(&exported.drawing).unwrap();
        load_ifccad_bytes(wire.bytes(), Default::default()).unwrap();
    }
}
fn text_native() -> IfccadDocument {
    let mut d = native();
    d.text_styles.push(IfccadTextStyle {
        annotation: Default::default(),
        id: IfccadTextStyleId(0),
        name: "Label".into(),
        properties: ifccad::text::TextStyleProperties::new(ifccad::text::FontRequest::family(
            "Requested",
        )),
    });
    let mut e = d.model.entities[0].as_native().unwrap().clone();
    e.kind = IfccadEntityKind::Text(IfccadText {
        annotation: Some(Box::new(IfccadAnnotation {
            orientation: None,
            mode: IfccadAnnotationMode::Disabled,
            reference: Some(IfccadAnnotationContextKey {
                scale_id: IfccadAnnotationScaleId(7),
            }),
            contexts: vec![
                IfccadAnnotationContext {
                    key: IfccadAnnotationContextKey {
                        scale_id: IfccadAnnotationScaleId(7),
                    },
                    presentation: Some(IfccadTextAnnotationContext {
                        insertion: [3., 4.],
                        alignment: [5., 6.],
                        rotation: 0.25,
                        horizontal: IfccadTextAnnotationHorizontal::Fit,
                    }),
                },
                IfccadAnnotationContext {
                    key: IfccadAnnotationContextKey {
                        scale_id: IfccadAnnotationScaleId(9),
                    },
                    presentation: None,
                },
            ],
        })),
        style_id: IfccadTextStyleId(0),
        placement: IfccadPlacement {
            origin: [10., 20., 7.],
            x_axis: [1., 0., 0.],
            y_axis: [0., 1., 0.],
        },
        rotation: 0.,
        backward: false,
        upside_down: false,
        layout: ifccad::text::TextLayout::Anchored {
            horizontal: ifccad::text::TextHorizontalAlignment::Left,
            vertical: ifccad::text::TextVerticalAlignment::Baseline,
            height: 2.,
            width_factor: 1.,
        },
        oblique_angle: 0.,
        thickness: 0.,
        content: vec![ifccad::text::TextRun {
            text: "Saved".into(),
            ..Default::default()
        }],
    });
    d.model.entities = vec![IfccadEntity::Native(e)];
    recompute_ifccad_document_bounds(&mut d).unwrap();
    d
}
#[test]
fn disabled_text_snapshots_and_bare_membership_survive_all_cad_backings() {
    let native = text_native();
    let imported = Ifcx.import_ifccad(&native).unwrap();
    for kind in ["memory", "dwg", "ascii", "binary"] {
        let document = if kind == "memory" {
            imported.document.clone()
        } else {
            physical(&imported.document, kind)
        };
        let exported = Ifcx.export_ifccad(&document).unwrap();
        let text = exported
            .drawing
            .model
            .entities
            .iter()
            .find_map(|e| match &e.as_native()?.kind {
                IfccadEntityKind::Text(t) => Some(t),
                _ => None,
            })
            .unwrap();
        let a = text
            .annotation
            .as_ref()
            .unwrap_or_else(|| panic!("{kind}: missing annotation, {:?}", exported.losses));
        assert_eq!(a.mode, IfccadAnnotationMode::Disabled);
        assert_eq!(a.contexts.len(), 2);
        let ref_name = &exported
            .drawing
            .annotation_scales
            .iter()
            .find(|s| s.id == a.reference.unwrap().scale_id)
            .unwrap()
            .name;
        assert_eq!(ref_name, "Saved unit designation");
        let p = a
            .contexts
            .iter()
            .find_map(|c| c.presentation.as_ref())
            .unwrap();
        assert_eq!(p.insertion, [3., 4.]);
        assert_eq!(p.alignment, [5., 6.]);
        assert_eq!(p.rotation, 0.25);
        assert_eq!(p.horizontal, IfccadTextAnnotationHorizontal::Fit);
        assert_eq!(
            a.contexts
                .iter()
                .filter(|c| c.presentation.is_none())
                .count(),
            1
        );
        let encoded = encode_ifccad_document(&exported.drawing).unwrap();
        load_ifccad_bytes(encoded.bytes(), Default::default()).unwrap();
    }
}
#[test]
fn mtext_xyz_direction_and_manual_columns_survive_physical_cad() {
    let mut d = text_native();
    let t = d.model.entities[0].as_native_mut().unwrap();
    let IfccadEntityKind::Text(base) = &t.kind else {
        unreachable!()
    };
    t.kind = IfccadEntityKind::MText(Box::new(IfccadMText {
        style_id: base.style_id,
        placement: base.placement.clone(),
        rotation: 0.,
        backward: false,
        upside_down: false,
        height: 2.,
        attachment: ifccad::text::MTextAttachment::TopLeft,
        flow: ifccad::text::MTextFlow::default(),
        wrap_width: None,
        columns: None,
        background: None,
        character_format: Default::default(),
        paragraph_format: Default::default(),
        content: vec![ifccad::text::MTextParagraph {
            inlines: vec![ifccad::text::MTextInline::Run {
                text: "Saved".into(),
                character_format: Default::default(),
            }],
            ..Default::default()
        }],
        annotation: Some(Box::new(IfccadAnnotation {
            orientation: None,
            mode: IfccadAnnotationMode::Enabled,
            reference: None,
            contexts: vec![IfccadAnnotationContext {
                key: IfccadAnnotationContextKey {
                    scale_id: IfccadAnnotationScaleId(7),
                },
                presentation: Some(IfccadMTextAnnotationContext {
                    insertion: [3., 4., 9.],
                    x_direction: [0., 2., 1.],
                    attachment: ifccad::text::MTextAttachment::BottomRight,
                    rectangle_width: 12.,
                    rectangle_height: 17.,
                    columns: Some(IfccadMTextAnnotationColumns {
                        kind: IfccadAnnotationColumnKind::Dynamic,
                        count: 2,
                        width: 10.,
                        gutter: 1.,
                        auto_height: false,
                        flow_reversed: true,
                        heights: vec![20., 30.],
                    }),
                }),
            }],
        })),
    }));
    recompute_ifccad_document_bounds(&mut d).unwrap();
    let mut imported = Ifcx.import_ifccad(&d).unwrap();
    // These values are a derived layout cache, distinct from the retained rectangle.
    for object in imported.document.objects.values_mut() {
        if let opencaddocument::objects::ObjectType::ObjectContextData(c) = object {
            if let opencaddocument::objects::ObjectContextKind::MText(p) = &mut c.kind {
                p.extents_width = 123.;
                p.extents_height = 456.;
            }
        }
    }
    for kind in ["memory", "dwg", "ascii", "binary"] {
        let document = if kind == "memory" {
            imported.document.clone()
        } else {
            physical(&imported.document, kind)
        };
        let exported = Ifcx.export_ifccad(&document).unwrap();
        assert!(
            exported.losses.iter().any(|loss| loss
                .message
                .contains("MText annotation layout extents cache")),
            "{kind}: {:?}",
            exported.losses
        );
        let t = exported
            .drawing
            .model
            .entities
            .iter()
            .find_map(|e| match &e.as_native()?.kind {
                IfccadEntityKind::MText(t) => Some(t),
                _ => None,
            })
            .unwrap();
        let a = t
            .annotation
            .as_ref()
            .unwrap_or_else(|| panic!("{kind}: missing MText annotation"));
        assert_eq!(a.mode, IfccadAnnotationMode::Enabled);
        assert!(a.reference.is_none());
        let p = a.contexts[0].presentation.as_ref().unwrap();
        assert_eq!(p.insertion, [3., 4., 9.]);
        assert_eq!(p.x_direction, [0., 2., 1.]);
        assert_eq!(p.attachment, ifccad::text::MTextAttachment::BottomRight);
        assert_eq!(p.rectangle_width, 12.);
        assert_eq!(p.rectangle_height, 17.);
        assert_eq!(p.columns.as_ref().unwrap().heights, vec![20., 30.]);
        assert!(p.columns.as_ref().unwrap().flow_reversed);
    }
}

fn world(p: [f64; 3], f: &IfccadPlacement) -> [f64; 3] {
    let x = f.x_axis;
    let y = f.y_axis;
    let z = [
        x[1] * y[2] - x[2] * y[1],
        x[2] * y[0] - x[0] * y[2],
        x[0] * y[1] - x[1] * y[0],
    ];
    std::array::from_fn(|i| f.origin[i] + x[i] * p[0] + y[i] * p[1] + z[i] * p[2])
}
#[test]
fn all_text_horizontal_modes_keep_context_world_positions_in_oblique_frames() {
    for horizontal in [
        IfccadTextAnnotationHorizontal::Left,
        IfccadTextAnnotationHorizontal::Center,
        IfccadTextAnnotationHorizontal::Right,
        IfccadTextAnnotationHorizontal::Aligned,
        IfccadTextAnnotationHorizontal::Middle,
        IfccadTextAnnotationHorizontal::Fit,
    ] {
        let mut native = text_native();
        let e = native.model.entities[0].as_native_mut().unwrap();
        let IfccadEntityKind::Text(t) = &mut e.kind else {
            unreachable!()
        };
        t.placement.x_axis = [0., 0.6, 0.8];
        t.placement.y_axis = [0., -0.8, 0.6];
        t.annotation.as_mut().unwrap().contexts[0]
            .presentation
            .as_mut()
            .unwrap()
            .horizontal = horizontal;
        let expected_insert = world([3., 4., 0.], &t.placement);
        let expected_align = world([5., 6., 0.], &t.placement);
        let expected_direction = world(
            [0.25f64.cos(), 0.25f64.sin(), 0.],
            &IfccadPlacement {
                origin: [0.; 3],
                ..t.placement.clone()
            },
        );
        recompute_ifccad_document_bounds(&mut native).unwrap();
        let imported = Ifcx.import_ifccad(&native).unwrap();
        for kind in ["memory", "dwg", "ascii", "binary"] {
            let document = if kind == "memory" {
                imported.document.clone()
            } else {
                physical(&imported.document, kind)
            };
            let exported = Ifcx.export_ifccad(&document).unwrap();
            let t = exported
                .drawing
                .model
                .entities
                .iter()
                .find_map(|e| match &e.as_native()?.kind {
                    IfccadEntityKind::Text(t) => Some(t),
                    _ => None,
                })
                .unwrap();
            let p = t
                .annotation
                .as_ref()
                .unwrap()
                .contexts
                .iter()
                .find_map(|c| c.presentation.as_ref())
                .unwrap();
            assert_eq!(p.horizontal, horizontal);
            let actual_insert = world([p.insertion[0], p.insertion[1], 0.], &t.placement);
            let actual_align = world([p.alignment[0], p.alignment[1], 0.], &t.placement);
            let actual_direction = world(
                [p.rotation.cos(), p.rotation.sin(), 0.],
                &IfccadPlacement {
                    origin: [0.; 3],
                    ..t.placement.clone()
                },
            );
            for (actual, expected) in [
                (actual_insert, expected_insert),
                (actual_align, expected_align),
                (actual_direction, expected_direction),
            ] {
                for i in 0..3 {
                    assert!(
                        (actual[i] - expected[i]).abs() < 1e-12,
                        "{kind}: {horizontal:?}: {actual:?} vs {expected:?}"
                    );
                }
            }
        }
    }
}
#[test]
fn memberships_enable_text_only_when_an_explicit_marker_is_absent() {
    let d = text_native();
    let mut imported = Ifcx.import_ifccad(&d).unwrap().document;
    let handle = imported
        .model_space_entities()
        .find_map(|e| {
            if matches!(e, opencaddocument::EntityType::Text(_)) {
                Some(e.common().handle)
            } else {
                None
            }
        })
        .unwrap();
    imported
        .get_entity_mut(handle)
        .unwrap()
        .common_mut()
        .extended_data
        .remove_record("AcadAnnotative");
    let out = Ifcx.export_ifccad(&imported).unwrap();
    let t = out
        .drawing
        .model
        .entities
        .iter()
        .find_map(|e| match &e.as_native()?.kind {
            IfccadEntityKind::Text(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        t.annotation.as_ref().unwrap().mode,
        IfccadAnnotationMode::Enabled
    );
}
#[test]
fn dimscale_zero_distinguishes_viewport_and_annotation_and_retains_positive_dormant_factor() {
    use ifccad::dimension::DimensionPresentationScale as S;
    for (annotative, factor, expected) in [
        (false, 2., S::Fixed { factor: 2. }),
        (false, 0., S::ByViewport),
        (
            true,
            0.,
            S::ByAnnotationScale {
                dormant_factor: None,
            },
        ),
        (
            true,
            3.,
            S::ByAnnotationScale {
                dormant_factor: Some(3.),
            },
        ),
    ] {
        let mut document = Document::new();
        let style = document.dim_styles.get_mut("Standard").unwrap();
        style.annotative = annotative;
        style.dimscale = factor;
        let outcome = Ifcx.export_ifccad(&document).unwrap();
        assert!(
            !outcome
                .losses
                .iter()
                .any(|loss| loss.message.contains("annotative scaling")),
            "{:?}",
            outcome.losses
        );
        let native = outcome.drawing;
        let style = native
            .dimension_styles
            .iter()
            .find(|s| s.name == "Standard")
            .unwrap();
        assert_eq!(style.annotative, annotative);
        assert_eq!(style.settings.presentation_scale, expected);
        let imported = Ifcx.import_ifccad(&native).unwrap();
        for kind in ["memory", "dwg", "ascii", "binary"] {
            let document = if kind == "memory" {
                imported.document.clone()
            } else {
                physical(&imported.document, kind)
            };
            let native = Ifcx.export_ifccad(&document).unwrap().drawing;
            let style = native
                .dimension_styles
                .iter()
                .find(|s| s.name == "Standard")
                .unwrap();
            assert_eq!(style.annotative, annotative, "{kind}");
            assert_eq!(style.settings.presentation_scale, expected, "{kind}");
        }
    }
}
#[test]
fn incompatible_dimension_style_flag_and_sizing_is_reported() {
    use ifccad::dimension::DimensionPresentationScale as S;
    let document = Document::new();
    let mut native = Ifcx.export_ifccad(&document).unwrap().drawing;
    let style = native
        .dimension_styles
        .iter_mut()
        .find(|s| s.name == "Standard")
        .unwrap();
    style.annotative = false;
    style.settings.presentation_scale = S::ByAnnotationScale {
        dormant_factor: Some(3.),
    };
    let outcome = Ifcx.import_ifccad(&native).unwrap();
    assert!(outcome
        .losses
        .iter()
        .any(|l| l.message.contains("sizing intent conflicts")));
}
#[test]
fn definition_flags_do_not_retroactively_annotate_text_and_survive_cad() {
    let mut native = ifccad::load_ifccad_bytes(
        include_bytes!("fixtures/text-rich.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    for s in &mut native.text_styles {
        s.annotation.enabled = true;
    }
    for b in &mut native.blocks {
        b.annotation.enabled = true;
    }
    recompute_ifccad_document_bounds(&mut native).unwrap();
    let imported = Ifcx.import_ifccad(&native).unwrap();
    for kind in ["memory", "dwg", "ascii", "binary"] {
        let document = if kind == "memory" {
            imported.document.clone()
        } else {
            physical(&imported.document, kind)
        };
        let exported = Ifcx.export_ifccad(&document).unwrap();
        for s in &native.text_styles {
            assert!(
                exported
                    .drawing
                    .text_styles
                    .iter()
                    .find(|t| t.name == s.name)
                    .unwrap()
                    .annotation
                    .enabled,
                "{kind}: text style {}",
                s.name
            );
        }
        for b in &native.blocks {
            assert!(
                exported
                    .drawing
                    .blocks
                    .iter()
                    .find(|t| t.name == b.name)
                    .unwrap()
                    .annotation
                    .enabled,
                "{kind}: block {}",
                b.name
            );
        }
        for e in &exported.drawing.model.entities {
            match &e.as_native().unwrap().kind {
                IfccadEntityKind::Text(t) => assert!(t.annotation.is_none()),
                IfccadEntityKind::MText(t) => assert!(t.annotation.is_none()),
                _ => (),
            }
        }
    }
}
#[test]
fn object_and_dormant_definition_paper_orientation_survive_physical_cad() {
    let mut native = text_native();
    for s in &mut native.text_styles {
        s.annotation = IfccadAnnotationDefinition {
            enabled: false,
            orientation: IfccadAnnotationOrientation::MatchLayout,
        };
    }
    let IfccadEntityKind::Text(t) = &mut native.model.entities[0].as_native_mut().unwrap().kind
    else {
        unreachable!()
    };
    t.annotation.as_mut().unwrap().orientation = Some(IfccadAnnotationOrientation::Fixed);
    let imported = Ifcx.import_ifccad(&native).unwrap();
    for kind in ["memory", "dwg", "ascii", "binary"] {
        let document = if kind == "memory" {
            imported.document.clone()
        } else {
            physical(&imported.document, kind)
        };
        let exported = Ifcx.export_ifccad(&document).unwrap();
        let s = exported
            .drawing
            .text_styles
            .iter()
            .find(|s| s.name == "Label")
            .unwrap();
        assert!(!s.annotation.enabled);
        assert_eq!(
            s.annotation.orientation,
            IfccadAnnotationOrientation::MatchLayout,
            "{kind}"
        );
        let t = exported
            .drawing
            .model
            .entities
            .iter()
            .find_map(|e| match &e.as_native()?.kind {
                IfccadEntityKind::Text(t) => Some(t),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            t.annotation.as_ref().unwrap().orientation,
            Some(IfccadAnnotationOrientation::Fixed),
            "{kind}"
        );
    }
}
#[test]
fn paper_orientation_without_annotation_marker_does_not_enable_annotation() {
    let mut native = text_native();
    let IfccadEntityKind::Text(t) = &mut native.model.entities[0].as_native_mut().unwrap().kind
    else {
        unreachable!()
    };
    let a = t.annotation.as_mut().unwrap();
    a.contexts.clear();
    a.reference = None;
    a.orientation = Some(IfccadAnnotationOrientation::MatchLayout);
    recompute_ifccad_document_bounds(&mut native).unwrap();
    let mut document = Ifcx.import_ifccad(&native).unwrap().document;
    let handle = document
        .model_space_entities()
        .find_map(|e| {
            if matches!(e, opencaddocument::EntityType::Text(_)) {
                Some(e.common().handle)
            } else {
                None
            }
        })
        .unwrap();
    document
        .get_entity_mut(handle)
        .unwrap()
        .common_mut()
        .extended_data
        .remove_record("AcadAnnotative");
    for kind in ["memory", "dwg", "ascii", "binary"] {
        let document = if kind == "memory" {
            document.clone()
        } else {
            physical(&document, kind)
        };
        let native = Ifcx.export_ifccad(&document).unwrap().drawing;
        let t = native
            .model
            .entities
            .iter()
            .find_map(|e| match &e.as_native()?.kind {
                IfccadEntityKind::Text(t) => Some(t),
                _ => None,
            })
            .unwrap();
        let a = t.annotation.as_ref().unwrap();
        assert_eq!(a.mode, IfccadAnnotationMode::Disabled);
        assert_eq!(
            a.orientation,
            Some(IfccadAnnotationOrientation::MatchLayout)
        );
        assert!(a.contexts.is_empty());
    }
}
#[test]
fn legacy_block_markers_keep_enabled_and_orientation_when_record_markers_are_absent() {
    let mut native = load_ifccad_bytes(
        include_bytes!("fixtures/text-rich.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    for b in &mut native.blocks {
        b.annotation = IfccadAnnotationDefinition {
            enabled: true,
            orientation: IfccadAnnotationOrientation::MatchLayout,
        };
    }
    recompute_ifccad_document_bounds(&mut native).unwrap();
    let mut document = Ifcx.import_ifccad(&native).unwrap().document;
    let blocks = document
        .block_records
        .iter()
        .filter(|r| native.blocks.iter().any(|b| b.name == r.name))
        .map(|r| (r.handle, r.block_entity_handle))
        .collect::<Vec<_>>();
    assert!(!blocks.is_empty());
    for (record, start) in blocks {
        let data = document.object_xdata.remove(&record).unwrap();
        document
            .get_entity_mut(start)
            .unwrap()
            .common_mut()
            .extended_data = data;
    }
    for kind in ["memory", "dwg", "ascii", "binary"] {
        let document = if kind == "memory" {
            document.clone()
        } else {
            physical(&document, kind)
        };
        let exported = Ifcx.export_ifccad(&document).unwrap();
        for b in &native.blocks {
            assert_eq!(
                exported
                    .drawing
                    .blocks
                    .iter()
                    .find(|x| x.name == b.name)
                    .unwrap()
                    .annotation,
                b.annotation,
                "{kind}"
            );
        }
    }
}
#[test]
fn hatch_scale_and_view_collections_keep_defaults_flags_and_repeated_null_targets_through_cad() {
    let native = load_ifccad_bytes(
        include_bytes!("fixtures/annotation-hatch.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    let imported = Ifcx.import_ifccad(&native).unwrap();
    let counts = |d: &Document| {
        let mut count = [0usize; 2];
        for o in d.objects.values() {
            if let opencaddocument::objects::ObjectType::ObjectContextData(c) = o {
                match c.kind {
                    opencaddocument::objects::ObjectContextKind::HatchScale(_) => count[0] += 1,
                    opencaddocument::objects::ObjectContextKind::HatchView(_) => count[1] += 1,
                    _ => (),
                }
            }
        }
        count
    };
    assert_eq!(counts(&imported.document), [1, 2]);
    for kind in ["memory", "dwg", "ascii", "binary"] {
        let d = if kind == "memory" {
            imported.document.clone()
        } else {
            physical(&imported.document, kind)
        };
        assert_eq!(counts(&d), [1, 2], "{kind}");
        let exported = Ifcx.export_ifccad(&d).unwrap();
        let h = exported
            .drawing
            .model
            .entities
            .iter()
            .find_map(|e| match &e.as_native()?.kind {
                IfccadEntityKind::Hatch(h) => Some(h),
                _ => None,
            })
            .unwrap();
        let a = h.annotation.as_ref().unwrap();
        assert_eq!(a.mode, IfccadAnnotationMode::Disabled);
        assert_eq!(a.contexts.len(), 1);
        assert_eq!(
            a.contexts[0].presentation.as_ref().unwrap().pattern.scale,
            2.
        );
        assert_eq!(a.reference, Some(a.contexts[0].key));
        let v = h.annotation_view_contexts.as_ref().unwrap();
        assert_eq!(v.contexts.len(), 2);
        assert!(v.contexts.iter().all(|c| c.target.is_none()));
        let default = v
            .contexts
            .iter()
            .find(|c| Some(c.id) == v.reference)
            .unwrap();
        assert_eq!(default.presentation.pattern.scale, 4.);
        let IfccadHatchContextLoop::Shared { classification, .. } = &default.presentation.loops[0]
        else {
            unreachable!()
        };
        assert!(
            classification.force_annotation_visible
                && classification.orient_to_paper
                && classification.is_annotative_block
        );
        let p = &a.contexts[0].presentation.as_ref().unwrap().pattern;
        assert_eq!(p.orientation_reference, [4., 5., 6.]);
        validate_ifccad_document(&exported.drawing).unwrap();
        let bytes = encode_ifccad_document(&exported.drawing).unwrap();
        load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    }
}

#[test]
fn hatch_stored_view_and_model_window_targets_resolve_after_physical_cad() {
    let mut native = load_ifccad_bytes(
        include_bytes!("fixtures/annotation-hatch.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    // The normal route supplies a valid model window from CAD's active VPORT.
    let baseline = Ifcx.export_ifccad(&Document::new()).unwrap().drawing;
    native.model_windows = baseline.model_windows;
    native.id_counters.next_model_window_id = baseline.id_counters.next_model_window_id;
    let window = native.model_windows.first().unwrap().id;
    native.annotation_views.push(IfccadAnnotationStoredView {
        annotation_scale_id: None,
        id: IfccadAnnotationViewId(0),
        name: "Annotation camera".into(),
        space: IfccadAnnotationViewSpace::Paper,
        width: 24.,
        view: IfccadViewportView {
            center: [4., 5.],
            target: [1., 2., 3.],
            direction: [0., 0., 2.],
            height: 12.,
            twist: 0.5,
            projection: IfccadViewportProjection::Orthographic,
            lens_length_mm: Some(60.),
            front_clip: IfccadViewportDepthClip {
                mode: IfccadViewportClipMode::Disabled,
                distance: Some(7.),
            },
            back_clip: IfccadViewportDepthClip {
                mode: IfccadViewportClipMode::AtDistance,
                distance: Some(20.),
            },
        },
    });
    let h = native
        .model
        .entities
        .iter_mut()
        .find_map(|e| match e {
            IfccadEntity::Native(IfccadNativeEntity {
                kind: IfccadEntityKind::Hatch(h),
                ..
            }) => Some(h),
            _ => None,
        })
        .unwrap();
    let views = h.annotation_view_contexts.as_mut().unwrap();
    views.contexts[0].target = Some(IfccadAnnotationViewTarget::StoredView {
        id: IfccadAnnotationViewId(0),
    });
    views.contexts[1].target = Some(IfccadAnnotationViewTarget::ModelWindow { id: window });
    let imported = Ifcx.import_ifccad(&native).unwrap();
    for kind in ["memory", "dwg", "ascii", "binary"] {
        let cad = if kind == "memory" {
            imported.document.clone()
        } else {
            physical(&imported.document, kind)
        };
        let exported = Ifcx.export_ifccad(&cad).unwrap().drawing;
        assert_eq!(exported.annotation_views.len(), 1, "{kind}");
        let v = &exported.annotation_views[0];
        assert_eq!(v.name, "Annotation camera");
        assert_eq!(v.width, 24.);
        assert_eq!(v.view, native.annotation_views[0].view, "{kind}");
        assert_eq!(v.space, IfccadAnnotationViewSpace::Paper);
        let h = exported
            .model
            .entities
            .iter()
            .find_map(|e| match &e.as_native()?.kind {
                IfccadEntityKind::Hatch(h) => Some(h),
                _ => None,
            })
            .unwrap();
        let views = h.annotation_view_contexts.as_ref().unwrap();
        assert!(
            matches!(views.contexts[0].target,Some(IfccadAnnotationViewTarget::StoredView{id}) if id==v.id),
            "{kind}"
        );
        assert!(
            matches!(views.contexts[1].target,Some(IfccadAnnotationViewTarget::ModelWindow{id}) if exported.model_windows.iter().any(|w|w.id==id)),
            "{kind}"
        );
    }
}

#[test]
fn hatch_view_memberships_infer_annotation_only_without_an_explicit_flag() {
    let native = load_ifccad_bytes(
        include_bytes!("fixtures/annotation-hatch.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    let imported = Ifcx.import_ifccad(&native).unwrap();
    for empty in [false, true] {
        let mut cad = imported.document.clone();
        let h = cad
            .model_space_entities()
            .find(|e| matches!(e, opencaddocument::entities::EntityType::Hatch(_)))
            .unwrap()
            .common()
            .handle;
        cad.get_entity_mut(h)
            .unwrap()
            .common_mut()
            .extended_data
            .remove_record("AcadAnnotative");
        let ext = cad
            .get_entity(h)
            .unwrap()
            .common()
            .xdictionary_handle
            .unwrap();
        let opencaddocument::objects::ObjectType::Dictionary(d) = cad.objects.get(&ext).unwrap()
        else {
            unreachable!()
        };
        let manager = d.get("AcDbContextDataManager").unwrap();
        let opencaddocument::objects::ObjectType::Dictionary(d) =
            cad.objects.get_mut(&manager).unwrap()
        else {
            unreachable!()
        };
        d.remove("ACDB_ANNOTATIONSCALES");
        if empty {
            let collection = d.get("ACDB_ANNOTATIONSCALE_VIEW_COLLECTION").unwrap();
            let opencaddocument::objects::ObjectType::Dictionary(d) =
                cad.objects.get_mut(&collection).unwrap()
            else {
                unreachable!()
            };
            d.entries.clear();
        }
        let out = Ifcx.export_ifccad(&cad).unwrap().drawing;
        let h = out
            .model
            .entities
            .iter()
            .find_map(|e| match &e.as_native()?.kind {
                IfccadEntityKind::Hatch(h) => Some(h),
                _ => None,
            })
            .unwrap();
        if empty {
            assert!(h.annotation.is_none());
        } else {
            let a = h
                .annotation
                .as_ref()
                .expect("view memberships imply annotation");
            assert_eq!(a.mode, IfccadAnnotationMode::Enabled);
            assert!(a.contexts.is_empty());
        }
    }
}

#[test]
fn hatch_context_pattern_values_and_local_boundaries_keep_oblique_world_geometry() {
    use ifccad::geometry_kernel::hatch::*;
    let mut native = load_ifccad_bytes(
        include_bytes!("fixtures/annotation-hatch.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    let h = native
        .model
        .entities
        .iter_mut()
        .find_map(|e| match e {
            IfccadEntity::Native(IfccadNativeEntity {
                kind: IfccadEntityKind::Hatch(h),
                ..
            }) => Some(h),
            _ => None,
        })
        .unwrap();
    h.placement = IfccadPlacement {
        origin: [20., 30., 40.],
        x_axis: [1., 0., 0.],
        y_axis: [0., 0., 1.],
    };
    let p = h.annotation.as_mut().unwrap().contexts[0]
        .presentation
        .as_mut()
        .unwrap();
    p.pattern.families = vec![HatchLineFamily {
        angle: 0.,
        base_point: [2., 3.],
        offset: [4., 5.],
        dashes: vec![
            HatchDash::Dash { length: 6. },
            HatchDash::Gap { length: 7. },
            HatchDash::Dot,
        ],
    }];
    p.loops.push(IfccadHatchContextLoop::Local {
        classification: Default::default(),
        boundary: IfccadHatchContextBoundary {
            geometry: HatchBoundary2::Circle {
                center: [50., 60.],
                radius: 8.,
            },
            source_entity_ids: vec![],
        },
    });
    recompute_ifccad_document_bounds(&mut native).unwrap();
    let imported = Ifcx.import_ifccad(&native).unwrap();
    for kind in ["memory", "dwg", "ascii", "binary"] {
        let cad = if kind == "memory" {
            imported.document.clone()
        } else {
            physical(&imported.document, kind)
        };
        let out = Ifcx.export_ifccad(&cad).unwrap().drawing;
        let h = out
            .model
            .entities
            .iter()
            .find_map(|e| match &e.as_native()?.kind {
                IfccadEntityKind::Hatch(h) => Some(h),
                _ => None,
            })
            .unwrap();
        let p = h.annotation.as_ref().unwrap().contexts[0]
            .presentation
            .as_ref()
            .unwrap();
        let f = &p.pattern.families[0];
        assert_eq!(f.base_point, [22., 43.], "{kind}");
        assert_eq!(f.offset, [4., 5.]);
        assert_eq!(
            f.dashes,
            vec![
                HatchDash::Dash { length: 6. },
                HatchDash::Gap { length: 7. },
                HatchDash::Dot
            ]
        );
        assert_eq!(p.pattern.scale, 2.);
        assert_eq!(p.pattern.orientation_reference, [4., 5., 6.]);
        let IfccadHatchContextLoop::Local { boundary, .. } = p.loops.last().unwrap() else {
            unreachable!()
        };
        assert_eq!(
            boundary.geometry,
            HatchBoundary2::Circle {
                center: [70., 100.],
                radius: 8.
            },
            "{kind}"
        );
        assert_eq!(h.placement.origin, [0., 30., 0.], "{kind}");
        assert_eq!(h.placement.x_axis, [1., 0., 0.]);
        assert_eq!(h.placement.y_axis, [0., 0., 1.]);
    }
}

#[test]
fn block_contexts_preserve_full_world_transform_and_base_through_physical_cad() {
    for mode in [
        IfccadAnnotationMode::Disabled,
        IfccadAnnotationMode::Enabled,
    ] {
        let mut native = load_ifccad_bytes(
            include_bytes!("fixtures/annotation-blocks.ifcx"),
            Default::default(),
        )
        .unwrap()
        .into_document();
        native.annotation_scales = vec![IfccadAnnotationScale {
            id: IfccadAnnotationScaleId(1),
            name: "1:50".into(),
            paper_units: 1.,
            drawing_units: 50.,
            unit_scale: false,
        }];
        native.id_counters.next_annotation_scale_id = 2;
        let e = native
            .model
            .entities
            .iter_mut()
            .find_map(|e| match &mut e.as_native_mut()?.kind {
                IfccadEntityKind::BlockInstance {
                    annotation,
                    transform,
                    ..
                } => Some((annotation, transform)),
                _ => None,
            })
            .unwrap();
        e.1.placement = IfccadPlacement {
            origin: [10., 20., 30.],
            x_axis: [1., 0., 0.],
            y_axis: [0., 0., 1.],
        };
        let mut p = e.1.clone();
        p.placement.origin = [101., 202., 303.];
        p.rotation = 0.5;
        p.scale = [-2., 1e-14, 3.];
        let base = e.1.clone();
        let key = IfccadAnnotationContextKey {
            scale_id: IfccadAnnotationScaleId(1),
        };
        *e.0 = Some(Box::new(IfccadAnnotation {
            orientation: Some(IfccadAnnotationOrientation::Fixed),
            mode,
            reference: Some(key),
            contexts: vec![IfccadAnnotationContext {
                key,
                presentation: Some(IfccadBlockAnnotationContext {
                    transform: p.clone(),
                }),
            }],
        }));
        recompute_ifccad_document_bounds(&mut native).unwrap();
        let imported = Ifcx.import_ifccad(&native).unwrap();
        assert!(imported.document.objects.values().any(|o|matches!(o,opencaddocument::objects::ObjectType::ObjectContextData(c) if matches!(c.kind,opencaddocument::objects::ObjectContextKind::BlkRef{..}))));
        for kind in ["memory", "dwg", "ascii", "binary"] {
            let cad = if kind == "memory" {
                imported.document.clone()
            } else {
                physical(&imported.document, kind)
            };
            let out = Ifcx.export_ifccad(&cad).unwrap().drawing;
            let (a, t) = out
                .model
                .entities
                .iter()
                .find_map(|e| match &e.as_native()?.kind {
                    IfccadEntityKind::BlockInstance {
                        annotation: Some(a),
                        transform,
                        ..
                    } => Some((a, transform)),
                    _ => None,
                })
                .unwrap();
            assert_eq!(*t, base, "{kind}");
            assert_eq!(a.mode, mode);
            assert_eq!(a.orientation, Some(IfccadAnnotationOrientation::Fixed));
            assert_eq!(
                a.contexts[0].presentation.as_ref().unwrap().transform,
                p,
                "{kind}"
            );
            assert_eq!(a.reference, Some(a.contexts[0].key));
            validate_ifccad_document(&out).unwrap();
            let bytes = encode_ifccad_document(&out).unwrap();
            load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
        }
    }
}

#[test]
fn block_inheritance_without_memberships_stays_implicit_and_materialization_is_reported() {
    for enabled in [false, true] {
        for membership in [false, true] {
            let mut native = load_ifccad_bytes(
                include_bytes!("fixtures/annotation-blocks.ifcx"),
                Default::default(),
            )
            .unwrap()
            .into_document();
            native.annotation_scales = vec![IfccadAnnotationScale {
                id: IfccadAnnotationScaleId(1),
                name: "1:10".into(),
                paper_units: 1.,
                drawing_units: 10.,
                unit_scale: false,
            }];
            native.id_counters.next_annotation_scale_id = 2;
            let e = native
                .model
                .entities
                .iter_mut()
                .find_map(|e| match &mut e.as_native_mut()?.kind {
                    IfccadEntityKind::BlockInstance {
                        annotation,
                        definition_id,
                        ..
                    } => Some((annotation, *definition_id)),
                    _ => None,
                })
                .unwrap();
            *e.0 = Some(Box::new(IfccadAnnotation {
                mode: IfccadAnnotationMode::ByDefinition,
                orientation: Some(IfccadAnnotationOrientation::Fixed),
                reference: None,
                contexts: if membership {
                    vec![IfccadAnnotationContext {
                        key: IfccadAnnotationContextKey {
                            scale_id: IfccadAnnotationScaleId(1),
                        },
                        presentation: None,
                    }]
                } else {
                    vec![]
                },
            }));
            native
                .blocks
                .iter_mut()
                .find(|b| b.id == e.1)
                .unwrap()
                .annotation
                .enabled = enabled;
            recompute_ifccad_document_bounds(&mut native).unwrap();
            let imported = Ifcx.import_ifccad(&native).unwrap();
            assert_eq!(
                imported
                    .losses
                    .iter()
                    .any(|l| l.message.contains("inheritance")),
                membership
            );
            let cad = physical(&imported.document, "dwg");
            let out = Ifcx.export_ifccad(&cad).unwrap().drawing;
            let a = out
                .model
                .entities
                .iter()
                .find_map(|e| match &e.as_native()?.kind {
                    IfccadEntityKind::BlockInstance {
                        annotation: Some(a),
                        ..
                    } => Some(a),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                a.mode,
                if !membership {
                    IfccadAnnotationMode::ByDefinition
                } else if enabled {
                    IfccadAnnotationMode::Enabled
                } else {
                    IfccadAnnotationMode::Disabled
                }
            );
        }
    }
}
