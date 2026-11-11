#![cfg(feature = "selection")]

use opencaddocument::{
    entities::{EntityType, Line, MLine},
    selection::*,
    Document, Handle, Vector3,
};
use opencaddocument::{
    objects::{Dictionary, MLineStyle, MLineStyleElement, ObjectType},
    tables::{LineType, LineTypeComplexData, LineTypeElement},
    xdata::{ExtendedData, ExtendedDataRecord, XDataValue},
};

fn capture(doc: &Document, roots: Vec<Handle>) -> DocumentSelection {
    DocumentSelection::capture(doc, &CaptureRequest::new("fixture", roots))
        .unwrap()
        .selection
}
fn mline(doc: &mut Document) -> Handle {
    doc.add_entity(EntityType::MLine(MLine::from_points(&[
        Vector3::ZERO,
        Vector3::new(10.0, 0.0, 0.0),
    ])))
    .unwrap()
}
fn style_mut(doc: &mut Document) -> &mut MLineStyle {
    let h = doc.header.current_multiline_style_handle;
    let ObjectType::MLineStyle(s) = doc.objects.get_mut(&h).unwrap() else {
        panic!("style")
    };
    s
}
fn error_kind(doc: &Document, h: Handle) -> SelectionErrorKind {
    DocumentSelection::capture(doc, &CaptureRequest::new("fixture", vec![h]))
        .unwrap_err()
        .kind
}

fn line(doc: &mut Document) -> Handle {
    doc.add_entity(EntityType::Line(Line::from_points(
        Vector3::ZERO,
        Vector3::new(2.0, 3.0, 0.0),
    )))
    .unwrap()
}

#[test]
fn empty_selection_has_no_scaffold() {
    let doc = Document::new();
    let result =
        DocumentSelection::capture(&doc, &CaptureRequest::new("drawing-a", vec![])).unwrap();
    assert!(result.selection.records().is_empty());
    assert!(result.selection.requirements().is_empty());
}

#[test]
fn owned_line_outlives_source() {
    let selection = {
        let mut doc = Document::new();
        let handle = line(&mut doc);
        let before = doc.clone();
        let result =
            DocumentSelection::capture(&doc, &CaptureRequest::new("drawing-a", vec![handle]))
                .unwrap();
        assert_eq!(doc, before);
        result.selection
    };
    assert_eq!(selection.roots().len(), 1);
    assert!(selection
        .records()
        .iter()
        .any(|r| matches!(r.payload, SelectionPayload::Line(_))));
    assert_eq!(selection.namespace(), "drawing-a");
}

#[test]
fn roots_are_explicit_unique_modelspace_drawables() {
    let mut doc = Document::new();
    let h = line(&mut doc);
    let sibling = line(&mut doc);
    let selection = DocumentSelection::capture(&doc, &CaptureRequest::new("a", vec![h]))
        .unwrap()
        .selection;
    assert!(!selection.records().iter().any(|r| r.key.handle == sibling));
    assert_eq!(
        DocumentSelection::capture(&doc, &CaptureRequest::new("a", vec![h, h]))
            .unwrap_err()
            .kind,
        SelectionErrorKind::InvalidRequest
    );
    assert!(
        DocumentSelection::capture(&doc, &CaptureRequest::new("a", vec![Handle::NULL])).is_err()
    );
    doc.get_entity_mut(h).unwrap().common_mut().owner_handle = Handle::new(999999);
    assert!(DocumentSelection::capture(&doc, &CaptureRequest::new("a", vec![h])).is_err());
}

#[test]
fn source_identity_collisions_are_rejected() {
    let mut doc = Document::new();
    let h = line(&mut doc);
    doc.layers.get_mut("0").unwrap().handle = h;
    assert_eq!(
        DocumentSelection::capture(&doc, &CaptureRequest::new("a", vec![h]))
            .unwrap_err()
            .kind,
        SelectionErrorKind::AmbiguousIdentity
    );
}

#[test]
fn namespaces_distinguish_equal_source_handles() {
    let mut doc = Document::new();
    let h = line(&mut doc);
    let a = DocumentSelection::capture(&doc, &CaptureRequest::new("a", vec![h]))
        .unwrap()
        .selection;
    let b = DocumentSelection::capture(&doc, &CaptureRequest::new("b", vec![h]))
        .unwrap()
        .selection;
    assert_eq!(a.roots(), b.roots());
    assert_ne!(a.namespace(), b.namespace());
}

#[test]
fn two_mlines_share_one_style_and_resources() {
    let mut doc = Document::new();
    let a = doc.add_entity(EntityType::MLine(MLine::new())).unwrap();
    let b = doc.add_entity(EntityType::MLine(MLine::new())).unwrap();
    let selection = DocumentSelection::capture(&doc, &CaptureRequest::new("a", vec![a, b]))
        .unwrap()
        .selection;
    assert_eq!(
        selection
            .records()
            .iter()
            .filter(|r| matches!(r.payload, SelectionPayload::MLineStyle(_)))
            .count(),
        1
    );
    assert_eq!(
        selection
            .requirements()
            .iter()
            .filter(|r| r.slot == SelectionSlot::MLineStyle)
            .count(),
        2
    );
}

#[test]
fn external_layer_preserves_each_root_slot() {
    let mut doc = Document::new();
    let a = mline(&mut doc);
    let b = mline(&mut doc);
    let layer = SelectionRecordKey {
        kind: SelectionRecordKind::Layer,
        handle: doc.layers.get("0").unwrap().handle,
    };
    let mut request = CaptureRequest::new("fixture", vec![a, b]);
    request.external.insert(layer);
    let selection = DocumentSelection::capture(&doc, &request)
        .unwrap()
        .selection;
    assert!(!selection.records().iter().any(|r| r.key == layer));
    let ports: Vec<_> = selection
        .requirements()
        .iter()
        .filter(|r| r.disposition == RequirementDisposition::External(layer))
        .collect();
    assert_eq!(ports.len(), 2);
    assert_eq!(ports[0].source.handle, a);
    assert_eq!(ports[1].source.handle, b);
    // The external layer's pattern is not implicitly included.
    assert!(!selection
        .records()
        .iter()
        .any(|r| matches!(r.payload, SelectionPayload::LineType(_))));
}

#[test]
fn missing_reference_is_a_gap_not_an_external_port() {
    let mut doc = Document::new();
    let h = line(&mut doc);
    doc.get_entity_mut(h).unwrap().common_mut().layer_handle = Some(Handle::new(999999));
    let selection = capture(&doc, vec![h]);
    let r = selection
        .requirements()
        .iter()
        .find(|r| r.slot == SelectionSlot::Layer)
        .unwrap();
    assert_eq!(r.disposition, RequirementDisposition::Missing);
    assert_eq!(r.source_handle, Some(Handle::new(999999)));
    assert_eq!(r.source_name.as_deref(), Some("0"));
    assert!(!selection
        .records()
        .iter()
        .any(|r| matches!(r.payload, SelectionPayload::Layer(_))));
    assert!(selection
        .capture_report()
        .iter()
        .any(|e| e.code == CaptureEvidenceCode::MissingDependency
            && e.slot == Some(SelectionSlot::Layer)));
}

#[test]
fn requested_nonexistent_dependency_still_records_a_gap() {
    let mut doc = Document::new();
    let h = line(&mut doc);
    let missing = Handle::new(999999);
    doc.get_entity_mut(h).unwrap().common_mut().layer_handle = Some(missing);
    let mut request = CaptureRequest::new("fixture", vec![h]);
    request.external.insert(SelectionRecordKey {
        kind: SelectionRecordKind::Layer,
        handle: missing,
    });
    let selection = DocumentSelection::capture(&doc, &request)
        .unwrap()
        .selection;
    assert!(selection.requirements().iter().any(
        |r| r.slot == SelectionSlot::Layer && r.disposition == RequirementDisposition::Missing
    ));
}

#[test]
fn source_style_default_is_explicit() {
    let mut doc = Document::new();
    let h = mline(&mut doc);
    let style = doc.header.current_multiline_style_handle;
    let selection = capture(&doc, vec![h]);
    assert!(selection
        .capture_report()
        .iter()
        .any(|e| e.code == CaptureEvidenceCode::ResolvedSourceDefault));
    let r = selection
        .requirements()
        .iter()
        .find(|r| r.slot == SelectionSlot::MLineStyle)
        .unwrap();
    assert_eq!(r.source_handle, Some(style));
    let SelectionPayload::MLine(m) = &selection.records()[0].payload else {
        panic!("MLINE")
    };
    assert_eq!(m.style_handle, None);
    doc.header.current_multiline_style_handle = Handle::NULL;
    let selection = capture(&doc, vec![h]);
    assert!(selection
        .requirements()
        .iter()
        .any(|r| r.slot == SelectionSlot::MLineStyle
            && r.disposition == RequirementDisposition::Missing));
}

#[test]
fn dangling_style_does_not_fall_back() {
    let mut doc = Document::new();
    let h = mline(&mut doc);
    let EntityType::MLine(m) = doc.get_entity_mut(h).unwrap() else {
        panic!()
    };
    m.style_handle = Some(Handle::new(999999));
    let selection = capture(&doc, vec![h]);
    assert!(!selection
        .records()
        .iter()
        .any(|r| matches!(r.payload, SelectionPayload::MLineStyle(_))));
    assert!(selection
        .requirements()
        .iter()
        .any(|r| r.slot == SelectionSlot::MLineStyle
            && r.disposition == RequirementDisposition::Missing));
    assert!(!selection
        .capture_report()
        .iter()
        .any(|e| e.code == CaptureEvidenceCode::ResolvedSourceDefault));
}

#[test]
fn name_handle_conflict_is_not_repaired() {
    let mut doc = Document::new();
    let h = mline(&mut doc);
    let EntityType::MLine(m) = doc.get_entity_mut(h).unwrap() else {
        panic!()
    };
    m.style_name = "Other".into();
    assert_eq!(error_kind(&doc, h), SelectionErrorKind::ReferenceConflict);
    let layer = doc.layers.get("0").unwrap().handle;
    let c = doc.get_entity_mut(h).unwrap().common_mut();
    c.layer = "Other".into();
    c.layer_handle = Some(layer);
    assert_eq!(error_kind(&doc, h), SelectionErrorKind::ReferenceConflict);
}

#[test]
fn style_registration_does_not_capture_other_entries() {
    let mut doc = Document::new();
    let h = mline(&mut doc);
    let mut extra = MLineStyle::new("Unused");
    extra.handle = doc.allocate_handle();
    extra.owner = doc.header.acad_mlinestyle_dict_handle;
    let extra_handle = extra.handle;
    doc.objects
        .insert(extra_handle, ObjectType::MLineStyle(extra));
    let dict = doc.header.acad_mlinestyle_dict_handle;
    let ObjectType::Dictionary(d) = doc.objects.get_mut(&dict).unwrap() else {
        panic!()
    };
    d.add_entry("Unused", extra_handle);
    let selection = capture(&doc, vec![h]);
    assert!(!selection
        .records()
        .iter()
        .any(|r| r.key.handle == extra_handle));
    assert!(selection
        .requirements()
        .iter()
        .any(|r| r.slot == SelectionSlot::StyleRegistration
            && r.disposition
                == RequirementDisposition::Context(ContextRole::MLineStyleDictionary)));
}

#[test]
fn style_registration_requires_the_actual_dictionary_role() {
    let mut doc = Document::new();
    let h = mline(&mut doc);
    let nod = doc.header.named_objects_dict_handle;
    let ObjectType::Dictionary(d) = doc.objects.get_mut(&nod).unwrap() else {
        panic!()
    };
    d.entries
        .iter_mut()
        .find(|(name, _)| name == "ACAD_MLINESTYLE")
        .unwrap()
        .1 = Handle::new(999999);
    assert_eq!(error_kind(&doc, h), SelectionErrorKind::ReferenceConflict);
}

#[test]
fn style_pattern_closure_retains_offsets_and_shared_resources() {
    let mut doc = Document::new();
    let h = mline(&mut doc);
    let mut pattern = LineType::new("Dash");
    pattern.handle = doc.allocate_handle();
    pattern.elements = vec![LineTypeElement::dash(0.75), LineTypeElement::space(0.5)];
    pattern.pattern_length = 1.25;
    let pattern_handle = pattern.handle;
    doc.line_types.add(pattern).unwrap();
    style_mut(&mut doc).elements = vec![
        MLineStyleElement::full(-0.3, opencaddocument::Color::ByLayer, "Dash"),
        MLineStyleElement::full(0.4, opencaddocument::Color::ByLayer, "Dash"),
    ];
    let selection = capture(&doc, vec![h]);
    assert_eq!(
        selection
            .records()
            .iter()
            .filter(|r| r.key.handle == pattern_handle)
            .count(),
        1
    );
    assert_eq!(
        selection
            .requirements()
            .iter()
            .filter(|r| r.disposition
                == RequirementDisposition::Included(SelectionRecordKey {
                    kind: SelectionRecordKind::LineType,
                    handle: pattern_handle
                }))
            .count(),
        2
    );
    let style = selection
        .records()
        .iter()
        .find_map(|r| {
            if let SelectionPayload::MLineStyle(s) = &r.payload {
                Some(s)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(style.elements[0].offset.to_bits(), (-0.3f64).to_bits());
}

#[test]
fn unrelated_unsupported_content_is_not_captured() {
    let mut doc = Document::new();
    let h = line(&mut doc);
    let other = doc
        .add_entity(EntityType::Unknown(
            opencaddocument::entities::UnknownEntity::new("PRIVATE"),
        ))
        .unwrap();
    doc.get_entity_mut(other)
        .unwrap()
        .common_mut()
        .reactors
        .push(Handle::new(777777));
    assert_eq!(capture(&doc, vec![h]).roots().len(), 1);
    assert_eq!(
        error_kind(&doc, other),
        SelectionErrorKind::UnsupportedContent
    );
}

fn xdata() -> ExtendedData {
    let mut x = ExtendedData::new();
    let mut r = ExtendedDataRecord::new("PRIVATE");
    r.add_value(XDataValue::Handle(Handle::new(777777)));
    x.add_record(r);
    x
}

#[test]
fn selected_xdata_and_dictionary_attachments_are_rejected() {
    let mut doc = Document::new();
    let h = line(&mut doc);
    doc.get_entity_mut(h).unwrap().common_mut().extended_data = xdata();
    assert_eq!(error_kind(&doc, h), SelectionErrorKind::UnsupportedContent);
    doc.get_entity_mut(h).unwrap().common_mut().extended_data = ExtendedData::new();
    let mut d = Dictionary::new();
    d.handle = doc.allocate_handle();
    d.owner = h;
    doc.objects.insert(d.handle, ObjectType::Dictionary(d));
    assert_eq!(error_kind(&doc, h), SelectionErrorKind::UnsupportedContent);
}

#[test]
fn style_and_layer_side_metadata_are_not_dropped() {
    let mut doc = Document::new();
    let h = mline(&mut doc);
    let style = doc.header.current_multiline_style_handle;
    doc.object_xdata.insert(style, xdata());
    assert_eq!(error_kind(&doc, h), SelectionErrorKind::UnsupportedContent);
    doc.object_xdata.clear();
    let layer = doc.layers.get("0").unwrap().handle;
    doc.add_object_reactor(layer, Handle::new(777777));
    assert_eq!(error_kind(&doc, h), SelectionErrorKind::UnsupportedContent);
}

#[test]
fn unsupported_complex_linetype_and_xref_are_rejected() {
    let mut doc = Document::new();
    let h = line(&mut doc);
    doc.line_types.get_mut("Continuous").unwrap().elements = vec![LineTypeElement {
        length: 1.0,
        complex: Some(LineTypeComplexData::default()),
    }];
    assert_eq!(error_kind(&doc, h), SelectionErrorKind::UnsupportedContent);
    doc.line_types
        .get_mut("Continuous")
        .unwrap()
        .elements
        .clear();
    doc.layers.get_mut("0").unwrap().flags.xref_dependent = true;
    assert_eq!(error_kind(&doc, h), SelectionErrorKind::UnsupportedContent);
}

#[test]
fn raw_and_graphic_backing_are_excluded_with_evidence() {
    let mut doc = Document::new();
    let h = line(&mut doc);
    let c = doc.get_entity_mut(h).unwrap().common_mut();
    let raw = std::sync::Arc::new(opencaddocument::entities::RawRecord {
        data: vec![1, 2, 3],
        handle_bits: 5,
        version: opencaddocument::DxfVersion::AC1032,
    });
    c.raw_record = Some(raw.clone());
    c.graphic_data = Some(vec![4, 5]);
    c.has_ds_data = true;
    let before = doc.clone();
    let count = std::sync::Arc::strong_count(&raw);
    let selection = capture(&doc, vec![h]);
    assert_eq!(doc, before);
    let SelectionPayload::Line(l) = &selection.records()[0].payload else {
        panic!()
    };
    assert!(l.common.raw_record.is_none());
    assert!(l.common.graphic_data.is_none());
    assert!(!l.common.has_ds_data);
    assert_eq!(std::sync::Arc::strong_count(&raw), count);
    for code in [
        CaptureEvidenceCode::ExcludedRawRecord,
        CaptureEvidenceCode::ExcludedGraphics,
        CaptureEvidenceCode::ExcludedDerivedData,
    ] {
        assert!(selection.capture_report().iter().any(|e| e.code == code));
    }
}

#[test]
fn typed_values_keep_float_bits_and_wide_handles() {
    let mut doc = Document::new();
    let wide = Handle::new((1u64 << 60) + 123);
    let nan = f64::from_bits(0x7ff8000000000042);
    let mut l = Line::new();
    l.common.handle = wide;
    l.start.x = -0.0;
    l.end.z = nan;
    doc.add_entity(EntityType::Line(l)).unwrap();
    let selection = capture(&doc, vec![wide]);
    assert_eq!(selection.roots()[0].handle, wide);
    let SelectionPayload::Line(l) = &selection.records()[0].payload else {
        panic!()
    };
    assert_eq!(l.start.x.to_bits(), (-0.0f64).to_bits());
    assert_eq!(l.end.z.to_bits(), nan.to_bits());
}

#[test]
fn source_entity_lookup_must_agree_with_payload_identity() {
    let mut doc = Document::new();
    let h = line(&mut doc);
    doc.get_entity_mut(h).unwrap().common_mut().handle = Handle::new(999999);
    // Keep the old identity occupied in another domain. A lookup using its
    // stale entity-index entry must not produce a record with the wrong payload.
    doc.layers.get_mut("0").unwrap().handle = h;
    assert!(DocumentSelection::capture(&doc, &CaptureRequest::new("fixture", vec![h])).is_err());
}

#[test]
fn explicit_owned_mode_is_supported_when_owner_is_modelspace() {
    let mut doc = Document::new();
    let h = line(&mut doc);
    doc.get_entity_mut(h).unwrap().common_mut().entity_mode = Some(0);
    let selection = capture(&doc, vec![h]);
    let SelectionPayload::Line(l) = &selection.records()[0].payload else {
        panic!()
    };
    assert_eq!(l.common.entity_mode, Some(0));
}

#[test]
fn contradictory_block_membership_is_rejected() {
    let mut doc = Document::new();
    let h = line(&mut doc);
    doc.block_records
        .get_mut("*Paper_Space")
        .unwrap()
        .entity_handles
        .push(h);
    assert!(DocumentSelection::capture(&doc, &CaptureRequest::new("fixture", vec![h])).is_err());
}

#[test]
fn missing_named_pattern_and_style_registration_are_located() {
    let mut doc = Document::new();
    let h = mline(&mut doc);
    style_mut(&mut doc).elements[0].linetype = "MissingPattern".into();
    let dict = doc.header.acad_mlinestyle_dict_handle;
    let ObjectType::Dictionary(d) = doc.objects.get_mut(&dict).unwrap() else {
        panic!()
    };
    d.entries.clear();
    let selection = capture(&doc, vec![h]);
    for slot in [
        SelectionSlot::StyleElementLineType(0),
        SelectionSlot::StyleRegistration,
    ] {
        assert!(selection
            .requirements()
            .iter()
            .any(|r| r.slot == slot && r.disposition == RequirementDisposition::Missing));
    }
}

#[test]
fn external_style_stops_its_closure_and_retains_root_requirements() {
    let mut doc = Document::new();
    let a = mline(&mut doc);
    let b = mline(&mut doc);
    let style = SelectionRecordKey {
        kind: SelectionRecordKind::Object,
        handle: doc.header.current_multiline_style_handle,
    };
    let mut request = CaptureRequest::new("fixture", vec![b, a]);
    request.external.insert(style);
    let selection = DocumentSelection::capture(&doc, &request)
        .unwrap()
        .selection;
    assert_eq!(selection.roots()[0].handle, b);
    assert_eq!(selection.roots()[1].handle, a);
    assert_eq!(
        selection
            .requirements()
            .iter()
            .filter(|r| r.disposition == RequirementDisposition::External(style))
            .count(),
        2
    );
    assert!(!selection.records().iter().any(|r| r.key == style));
    assert!(!selection
        .requirements()
        .iter()
        .any(|r| r.slot == SelectionSlot::StyleRegistration));
    request.external.insert(SelectionRecordKey {
        kind: SelectionRecordKind::Object,
        handle: Handle::new(999999),
    });
    assert_eq!(
        DocumentSelection::capture(&doc, &request).unwrap_err().kind,
        SelectionErrorKind::InvalidRequest
    );
}

#[test]
fn default_style_uses_actual_source_header_instead_of_standard() {
    let mut doc = Document::new();
    let h = mline(&mut doc);
    let mut style = MLineStyle::new("Custom");
    style.handle = doc.allocate_handle();
    style.owner = doc.header.acad_mlinestyle_dict_handle;
    let custom = style.handle;
    doc.objects.insert(custom, ObjectType::MLineStyle(style));
    let dict = doc.header.acad_mlinestyle_dict_handle;
    let ObjectType::Dictionary(d) = doc.objects.get_mut(&dict).unwrap() else {
        panic!()
    };
    d.add_entry("Custom", custom);
    doc.header.current_multiline_style_handle = custom;
    let EntityType::MLine(m) = doc.get_entity_mut(h).unwrap() else {
        panic!()
    };
    m.style_handle = Some(Handle::NULL);
    m.style_name = "Custom".into();
    let selection = capture(&doc, vec![h]);
    assert!(selection
        .requirements()
        .iter()
        .any(|r| r.slot == SelectionSlot::MLineStyle
            && r.disposition
                == RequirementDisposition::Included(SelectionRecordKey {
                    kind: SelectionRecordKind::Object,
                    handle: custom
                })));
}

#[test]
fn unsupported_common_resource_references_do_not_disappear() {
    for variant in 0..7 {
        let mut doc = Document::new();
        let h = line(&mut doc);
        let c = doc.get_entity_mut(h).unwrap().common_mut();
        match variant {
            0 => c.material_handle = Some(Handle::new(999999)),
            1 => c.full_visual_style_handle = Some(Handle::new(999999)),
            2 => c.color_book_handle = Some(Handle::new(999999)),
            3 => c.plotstyle_flags = 3,
            4 => c.extended_data.raw_dwg_eed.push((999999, vec![1, 2])),
            5 => c.r12_metadata = Some(Default::default()),
            _ => c.reactors.push(Handle::new(999999)),
        }
        assert_eq!(error_kind(&doc, h), SelectionErrorKind::UnsupportedContent);
    }
}

#[test]
fn context_dictionary_is_not_an_external_payload_boundary() {
    let mut doc = Document::new();
    let h = mline(&mut doc);
    let dict = doc.header.acad_mlinestyle_dict_handle;
    doc.objects.remove(&dict);
    let mut request = CaptureRequest::new("fixture", vec![h]);
    request.external.insert(SelectionRecordKey {
        kind: SelectionRecordKind::Object,
        handle: dict,
    });
    assert_eq!(
        DocumentSelection::capture(&doc, &request).unwrap_err().kind,
        SelectionErrorKind::InvalidRequest
    );
}

#[test]
fn capture_accepts_roots_from_production_readers_with_external_layer() {
    use opencaddocument::{Dwg, Dxf, Format};
    let mut source = Document::new();
    line(&mut source);
    mline(&mut source);
    // This qualifies capture inputs, not reconstruction or a preservation write.
    for format in [&Dxf as &dyn Format, &Dwg as &dyn Format] {
        let (bytes, losses) = format.write(&source).unwrap();
        assert!(losses.is_empty());
        let (read, losses) = format.read(&bytes).unwrap();
        assert!(losses.is_empty());
        let roots = read
            .entities()
            .filter(|e| matches!(e, EntityType::Line(_) | EntityType::MLine(_)))
            .map(|e| e.common().handle)
            .collect::<Vec<_>>();
        if !read.layers.get("0").unwrap().plotstyle_handle.is_null() {
            assert_eq!(
                DocumentSelection::capture(&read, &CaptureRequest::new("fixture", roots.clone()))
                    .unwrap_err()
                    .kind,
                SelectionErrorKind::UnsupportedContent
            );
        }
        let mut request = CaptureRequest::new("fixture", roots);
        // CAD IO installs a Normal plot-style reference on layer 0. Its object
        // dependency is outside D1. As in an IFCCAD caller, bind the layer
        // externally rather than discarding that reference from an owned copy.
        request.external.insert(SelectionRecordKey {
            kind: SelectionRecordKind::Layer,
            handle: read.layers.get("0").unwrap().handle,
        });
        let selection = DocumentSelection::capture(&read, &request)
            .unwrap_or_else(|error| panic!("{}: {error}", format.name()))
            .selection;
        assert_eq!(selection.roots().len(), 2);
        #[cfg(feature = "selection-storage")]
        {
            use opencaddocument::selection_storage::{decode, encode};
            let bytes = encode(&selection).unwrap();
            drop(read);
            let reopened = decode(&bytes).unwrap();
            assert_eq!(reopened, selection);
            assert_eq!(encode(&reopened).unwrap(), bytes);
        }
        assert!(selection
            .records()
            .iter()
            .any(|r| matches!(r.payload, SelectionPayload::MLineStyle(_))));
    }
}
