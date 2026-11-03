#![cfg(feature = "ifcx")]
use ifccad::*;
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
fn selection(d: &IfccadDocument, id: Option<IfccadAnnotationScaleId>) -> Option<&str> {
    id.map(|id| {
        d.annotation_scales
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .name
            .as_str()
    })
}
#[test]
fn drawing_and_scoped_annotation_policies_keep_false_absence_and_distinct_scale_owners_through_cad()
{
    for insert in [false, true] {
        let mut native = load_ifccad_bytes(
            include_bytes!("fixtures/annotation-workspace.ifcx"),
            Default::default(),
        )
        .unwrap()
        .into_document();
        native.annotation_scales = vec![
            IfccadAnnotationScale {
                id: IfccadAnnotationScaleId(1),
                name: "1:20".into(),
                paper_units: 1.,
                drawing_units: 20.,
                unit_scale: false,
            },
            IfccadAnnotationScale {
                id: IfccadAnnotationScaleId(2),
                name: "1:50".into(),
                paper_units: 1.,
                drawing_units: 50.,
                unit_scale: false,
            },
        ];
        native.id_counters.next_annotation_scale_id = 3;
        native.annotation_settings = Some(IfccadDrawingAnnotationSettings {
            insert_as_annotative: Some(insert),
            model_linetype_scaling: Some(IfccadModelLinetypeAnnotationMode::ByAnnotationScale),
        });
        native.model.annotation_state = Some(IfccadLayoutAnnotationState {
            current_scale_id: Some(IfccadAnnotationScaleId(1)),
            visibility: Some(IfccadAnnotationVisibility::CurrentScaleOnly),
        });
        let mut other = native.paper_layouts[0].clone();
        other.id = native.id_counters.allocate_layout_id().unwrap();
        other.name = "Other sheet".into();
        other.tab_index = 2;
        other.entities.clear();
        other.canvas = None;
        other.annotation_state = Some(IfccadLayoutAnnotationState {
            current_scale_id: None,
            visibility: Some(IfccadAnnotationVisibility::CurrentScaleOnly),
        });
        native.paper_layouts.push(other);
        native.paper_layouts[0].annotation_state = Some(IfccadLayoutAnnotationState {
            current_scale_id: Some(IfccadAnnotationScaleId(2)),
            visibility: Some(IfccadAnnotationVisibility::All),
        });
        let v = native.paper_layouts[0]
            .entities
            .iter_mut()
            .find_map(|e| match &mut e.as_native_mut()?.kind {
                IfccadEntityKind::Viewport(v) => Some(v),
                _ => None,
            })
            .unwrap();
        v.annotation_scale_id = Some(IfccadAnnotationScaleId(1));
        let view = v.view.clone();
        let width = v.frame.width;
        native.annotation_views.push(IfccadAnnotationStoredView {
            annotation_scale_id: Some(IfccadAnnotationScaleId(2)),
            id: IfccadAnnotationViewId(0),
            name: "Saved camera".into(),
            space: IfccadAnnotationViewSpace::Model,
            width,
            view,
        });
        recompute_ifccad_document_bounds(&mut native).unwrap();
        let imported = Ifcx.import_ifccad(&native).unwrap();
        assert_eq!(
            imported.document.drawing_variable("ANNOTATIVEDWG"),
            Some(if insert { "1" } else { "0" })
        );
        assert_eq!(
            imported.document.drawing_variable("CANNOSCALE"),
            Some("1:20")
        );
        for kind in ["memory", "dwg", "ascii", "binary"] {
            let cad = if kind == "memory" {
                imported.document.clone()
            } else {
                physical(&imported.document, kind)
            };
            let out = Ifcx.export_ifccad(&cad).unwrap().drawing;
            assert_eq!(
                out.annotation_settings, native.annotation_settings,
                "{kind}"
            );
            let m = out.model.annotation_state.unwrap();
            assert_eq!(selection(&out, m.current_scale_id), Some("1:20"));
            assert_eq!(
                m.visibility,
                Some(IfccadAnnotationVisibility::CurrentScaleOnly)
            );
            let p = out
                .paper_layouts
                .iter()
                .find(|p| p.name == native.paper_layouts[0].name)
                .unwrap();
            let state = p.annotation_state.unwrap();
            assert_eq!(selection(&out, state.current_scale_id), Some("1:50"));
            assert_eq!(state.visibility, Some(IfccadAnnotationVisibility::All));
            let v = p
                .entities
                .iter()
                .find_map(|e| match &e.as_native()?.kind {
                    IfccadEntityKind::Viewport(v) => Some(v),
                    _ => None,
                })
                .unwrap();
            assert_eq!(selection(&out, v.annotation_scale_id), Some("1:20"));
            let p = out
                .paper_layouts
                .iter()
                .find(|p| p.name == "Other sheet")
                .unwrap();
            let state = p.annotation_state.unwrap();
            assert_eq!(state.current_scale_id, None);
            assert_eq!(
                state.visibility,
                Some(IfccadAnnotationVisibility::CurrentScaleOnly)
            );
            assert_eq!(
                selection(&out, out.annotation_views[0].annotation_scale_id),
                Some("1:50")
            );
            validate_ifccad_document(&out).unwrap();
            let bytes = encode_ifccad_document(&out).unwrap();
            load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
            let bytes = encode_ifccad_cbor_document(&out).unwrap();
            load_ifccad_cbor_bytes(bytes.bytes(), Default::default()).unwrap();
        }
    }
}
