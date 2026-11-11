#![cfg(feature = "selection-storage")]

use opencaddocument::selection::{
    CaptureEvidenceCode, RequirementDisposition, SelectionPayload, SelectionSlot,
};
use opencaddocument::selection_storage::{decode, encode, SelectionStorageErrorKind as Kind};

const EMPTY: &[u8] = include_bytes!("fixtures/selection/v1/empty.msgpack");
const MIXED: &[u8] = include_bytes!("fixtures/selection/v1/mixed.msgpack");

fn frame(body: &[u8]) -> Vec<u8> {
    let mut result = b"OCADSEL\0\x00\x01\x01".to_vec();
    result.extend_from_slice(&(body.len() as u32).to_be_bytes());
    result.extend_from_slice(body);
    result
}

fn replace(body: &[u8], before: &[u8], after: &[u8]) -> Vec<u8> {
    let at = body
        .windows(before.len())
        .position(|w| w == before)
        .unwrap();
    let mut updated = body.to_vec();
    updated.splice(at..at + before.len(), after.iter().copied());
    updated
}

#[test]
fn rejects_wrong_magic_version_kind_before_body() {
    for (index, value, kind) in [
        (0, 0, Kind::InvalidEnvelope),
        (9, 2, Kind::UnsupportedVersion),
        (10, 2, Kind::UnsupportedKind),
    ] {
        let mut data = frame(&[0xc1]);
        data[index] = value;
        assert_eq!(decode(&data).unwrap_err().kind, kind);
    }
}

#[test]
fn rejects_truncation_trailing_and_multiple_values() {
    for i in 0..EMPTY.len() {
        assert_eq!(decode(&EMPTY[..i]).unwrap_err().kind, Kind::Truncated);
    }
    assert_eq!(
        decode(&frame(&[0xa3, b'a'])).unwrap_err().kind,
        Kind::Truncated
    );
    let mut extra = EMPTY.to_vec();
    extra.push(0xc0);
    assert_eq!(decode(&extra).unwrap_err().kind, Kind::TrailingData);
    extra[11..15].copy_from_slice(&111_u32.to_be_bytes());
    assert_eq!(decode(&extra).unwrap_err().kind, Kind::TrailingData);
}

#[test]
fn rejects_unbounded_marker_lengths_before_decode() {
    for marker in [0xdd, 0xdf, 0xdb] {
        assert_eq!(
            decode(&frame(&[marker, 0xff, 0xff, 0xff, 0xff]))
                .unwrap_err()
                .kind,
            Kind::LimitExceeded
        );
    }
    let mut huge = frame(&[]);
    huge[11..15].copy_from_slice(&(64_u32 * 1024 * 1024 + 1).to_be_bytes());
    assert_eq!(decode(&huge).unwrap_err().kind, Kind::LimitExceeded);
    let mut nested = vec![0x91; 17];
    nested.push(0xc0);
    assert_eq!(
        decode(&frame(&nested)).unwrap_err().kind,
        Kind::LimitExceeded
    );
}

#[test]
fn preflight_enforces_exact_depth_and_counts_map_keys() {
    let mut deepest = vec![0x91; 16];
    deepest.push(0xc0);
    let err = decode(&frame(&deepest)).unwrap_err();
    assert_eq!(err.kind, Kind::InvalidBody);
    assert_eq!(err.offset, None); // The wire shape fails after physical preflight.

    for marker in [0xdd, 0xdf, 0xdb] {
        let limit = if marker == 0xdb {
            1_048_576_u32
        } else {
            262_144
        };
        for (count, kind) in [(limit, Kind::Truncated), (limit + 1, Kind::LimitExceeded)] {
            let mut body = vec![marker];
            body.extend_from_slice(&count.to_be_bytes());
            assert_eq!(decode(&frame(&body)).unwrap_err().kind, kind);
        }
    }

    // Two max-size maps have 1,048,576 children including keys, plus
    // the maps and the outer array. The second map exhausts the budget.
    let mut body = vec![0x92, 0xdf, 0, 4, 0, 0];
    for _ in 0..262_144 {
        body.extend_from_slice(&[0xa0, 0xc0]);
    }
    body.extend_from_slice(&[0xdf, 0, 4, 0, 0]);
    let err = decode(&frame(&body)).unwrap_err();
    assert_eq!(err.kind, Kind::LimitExceeded);
    assert!(err.message.contains("remaining value budget"));
}

#[cfg(feature = "serde")]
#[test]
fn ordinary_model_serde_and_storage_are_separate() {
    use opencaddocument::entities::Line;

    let selection = decode(MIXED).unwrap();
    let SelectionPayload::Line(line) = &selection.records()[0].payload else {
        panic!("expected first record to be a Line");
    };
    let model_json = serde_json::to_vec(line).unwrap();
    let restored: Line = serde_json::from_slice(&model_json).unwrap();
    assert_eq!(restored.thickness.to_bits(), line.thickness.to_bits());
    assert_eq!(decode(&model_json).unwrap_err().kind, Kind::InvalidEnvelope);
    assert_eq!(encode(&selection).unwrap(), MIXED);
}

#[test]
fn rejects_forbidden_markers_and_invalid_utf8() {
    for body in [
        &[0xa1, 0xff][..],
        &[0xc4, 1, 0][..],
        &[0xd4, 1, 0][..],
        &[0xca, 0, 0, 0, 0][..],
        &[0x81, 1, 2][..],
        &[0xc1][..],
    ] {
        assert_eq!(decode(&frame(body)).unwrap_err().kind, Kind::InvalidBody);
    }
}

#[test]
fn reads_independent_empty_v1() {
    let selection = decode(EMPTY).unwrap();
    assert_eq!(selection.namespace(), "fixture-v1");
    assert!(selection.roots().is_empty());
    assert!(selection.records().is_empty());
    assert!(selection.requirements().is_empty());
    assert!(selection.capture_report().is_empty());
    assert_eq!(selection.context().insertion_units, 4);
    assert_eq!(
        selection.context().linetype_scale.to_bits(),
        1.0_f64.to_bits()
    );
    assert_eq!(encode(&selection).unwrap(), EMPTY);
}

#[test]
fn reads_independent_mixed_v1() {
    let selection = decode(MIXED).unwrap();
    assert_eq!(selection.namespace(), "fixture-v1");
    assert_eq!(selection.context().insertion_units, 0);
    assert_eq!(
        selection
            .roots()
            .iter()
            .map(|k| k.handle.value())
            .collect::<Vec<_>>(),
        [16, 9_007_199_254_740_993, 18]
    );
    assert_eq!(selection.records().len(), 6);
    assert_eq!(selection.requirements().len(), 15);
    let SelectionPayload::Line(line) = &selection.records()[0].payload else {
        panic!()
    };
    assert_eq!(line.thickness.to_bits(), (-0.0_f64).to_bits());
    assert_eq!(
        line.common.line_weight,
        opencaddocument::types::LineWeight::Value(-1)
    );
    let SelectionPayload::MLine(ml) = &selection.records()[1].payload else {
        panic!()
    };
    assert!(ml.style_handle.is_none());
    assert_eq!(ml.common.linetype_handle.unwrap().value(), 0);
    assert_eq!(ml.flags.bits(), -32767);
    assert_eq!(
        ml.vertices[0].segments[0].parameters[0].to_bits(),
        0x7ff8000000000042
    );
    assert_eq!(
        ml.vertices[0].segments[1].area_fill_parameters[0].to_bits(),
        0x7ff0000000000000
    );
    assert_eq!(
        selection
            .requirements()
            .iter()
            .filter(|r| r.slot == SelectionSlot::MLineStyle
                && matches!(r.disposition,
            RequirementDisposition::Included(k) if k.handle.value() == 34))
            .count(),
        2
    );
    assert_eq!(
        selection
            .requirements()
            .iter()
            .filter(|r| r.slot == SelectionSlot::Layer
                && matches!(r.disposition,
            RequirementDisposition::External(k) if k.handle.value() == 48))
            .count(),
        2
    );
    assert_eq!(
        selection
            .capture_report()
            .iter()
            .map(|e| (e.code, e.message.as_str()))
            .collect::<Vec<_>>(),
        [
            (CaptureEvidenceCode::UnknownUnits, "fixture unknown units"),
            (
                CaptureEvidenceCode::ResolvedSourceDefault,
                "fixture source default"
            ),
            (
                CaptureEvidenceCode::MissingDependency,
                "fixture dangling pattern"
            ),
        ]
    );
    assert_eq!(encode(&selection).unwrap(), MIXED);
    assert_eq!(encode(&decode(MIXED).unwrap()).unwrap(), MIXED);
}

#[test]
fn rejects_nested_unknown_duplicate_missing_fields() {
    let body = &MIXED[15..];
    let unknown = replace(body, b"\xaacolor_name", b"\xaaraw_record");
    let duplicate = replace(body, b"\xaacolor_name", b"\xa5color");
    let missing = replace(body, b"\x92\xa5mline\x8a", b"\x92\xa5mline\x89");
    let missing = replace(&missing, b"\xacstyle_handle\xc0", b"");
    for changed in [unknown, duplicate, missing] {
        assert_eq!(
            decode(&frame(&changed)).unwrap_err().kind,
            Kind::InvalidBody
        );
    }
}

#[test]
fn rejects_positional_maps_and_unknown_tuple_tags() {
    let body = &MIXED[15..];
    let positional = replace(body, b"\xa6common\xde\x00\x10", b"\xa6common\xdc\x00\x20");
    let wrong_tag = replace(body, b"\x91\xa8by_layer", b"\x91\xa8by_wrong");
    let extra_arg = replace(body, b"\x91\xa8by_layer", b"\x92\xa8by_layer\x01");
    for changed in [positional, wrong_tag, extra_arg] {
        assert_eq!(
            decode(&frame(&changed)).unwrap_err().kind,
            Kind::InvalidBody
        );
    }
}

#[test]
fn accepts_permuted_keys_and_wide_integer_markers() {
    let body = &EMPTY[15..];
    let ns_len = 1 + "namespace".len() + 1 + "fixture-v1".len();
    let mut permuted = vec![0x86];
    permuted.extend_from_slice(&body[1 + ns_len..]);
    permuted.extend_from_slice(&body[1..1 + ns_len]);
    let permuted = replace(
        &permuted,
        b"\xafinsertion_units\x04",
        b"\xafinsertion_units\xcd\x00\x04",
    );
    assert_eq!(encode(&decode(&frame(&permuted)).unwrap()).unwrap(), EMPTY);
}

#[test]
fn rejects_inconsistent_keys_slots_and_closure() {
    let body = &MIXED[15..];
    let changes = [
        replace(body, b"\xa6handle\x10", b"\xa6handle\x00"),
        replace(
            body,
            b"\xa4line\x85\xa6common\xde\x00\x10\xa6handle\x10",
            b"\xa4line\x85\xa6common\xde\x00\x10\xa6handle\x11",
        ),
        replace(
            body,
            b"\xb6style_element_linetype\x01",
            b"\xb6style_element_linetype\x02",
        ),
        replace(
            body,
            b"\xb6style_element_linetype\x01",
            b"\xb6style_element_linetype\x00",
        ),
        replace(
            body,
            b"\xa7context\xabmodel_space",
            b"\xa7context\xa8by_layer",
        ),
    ];
    for (i, change) in changes.iter().enumerate() {
        assert_eq!(
            decode(&frame(change)).unwrap_err().kind,
            Kind::InvalidSelection,
            "mutation {i}"
        );
    }
}

#[test]
fn rejects_cross_kind_identity_collision_and_conflicting_owners() {
    let body = &MIXED[15..];
    let collision = replace(
        body,
        b"\x82\xa3key\x82\xa4kind\xa8linetype\xa6handle\x21",
        b"\x82\xa3key\x82\xa4kind\xa8linetype\xa6handle\x20",
    );
    let collision = replace(
        &collision,
        b"\x92\xa8linetype\x86\xa6handle\x21",
        b"\x92\xa8linetype\x86\xa6handle\x20",
    );
    let err = decode(&frame(&collision)).unwrap_err();
    assert_eq!(err.kind, Kind::InvalidSelection);
    assert_eq!(err.path.as_deref(), Some("/records/4/key"));

    let owner = replace(body, b"\xacowner_handle\x05", b"\xacowner_handle\x07");
    let err = decode(&frame(&owner)).unwrap_err();
    assert_eq!(err.kind, Kind::InvalidSelection);
    assert!(err.message.contains("conflicting contextual owner"));
}

#[test]
fn rejects_missing_slot_and_unmatched_reference_name() {
    let body = &MIXED[15..];
    let start = b"\x85\xa6source\x82\xa4kind\xa6entity\xa6handle\x10";
    let first = body.windows(start.len()).position(|w| w == start).unwrap();
    let next = first + start.len();
    let second = next
        + body[next..]
            .windows(start.len())
            .position(|w| w == start)
            .unwrap();
    let without_owner = replace(body, &body[first..second], b"");
    let without_owner = replace(
        &without_owner,
        b"\xacrequirements\x9f",
        b"\xacrequirements\x9e",
    );
    let err = decode(&frame(&without_owner)).unwrap_err();
    assert_eq!(err.kind, Kind::InvalidSelection);
    assert!(err.message.contains("required slots"));

    let mismatch = replace(body, b"\xa8linetype\xa4Dash", b"\xa8linetype\xa5Other");
    let mismatch = replace(
        &mismatch,
        b"\xabsource_name\xa4Dash",
        b"\xabsource_name\xa5Other",
    );
    let err = decode(&frame(&mismatch)).unwrap_err();
    assert_eq!(err.kind, Kind::InvalidSelection);
    assert!(err.message.contains("name disagrees"));
}

#[test]
fn rejects_inconsistent_evidence() {
    let body = &MIXED[15..];
    for changed in [
        replace(body, b"\xb7resolved_source_default", b"\xadunknown_units"),
        replace(body, b"\xb2missing_dependency", b"\xb1excluded_graphics"),
    ] {
        assert_eq!(
            decode(&frame(&changed)).unwrap_err().kind,
            Kind::InvalidSelection
        );
    }
}

#[test]
fn preserves_gaps_and_evidence_without_rebinding() {
    let body = &MIXED[15..];
    let body = replace(body, b"\xa8linetype\xa5GHOST", b"\xa8linetype\xa4Dash");
    let body = replace(
        &body,
        b"\xabsource_name\xa5GHOST",
        b"\xabsource_name\xa4Dash",
    );
    let selection = decode(&frame(&body)).unwrap();
    let gap = selection
        .requirements()
        .iter()
        .find(|r| r.source.handle.value() == 18 && r.slot == SelectionSlot::LineType)
        .unwrap();
    assert_eq!(gap.source_name.as_deref(), Some("Dash"));
    assert_eq!(gap.source_handle.unwrap().value(), 68);
    assert_eq!(gap.disposition, RequirementDisposition::Missing);
}

#[test]
fn capture_drop_reopen_preserves_supported_projection() {
    use opencaddocument::{
        entities::{EntityType, Line, MLine, MLineFlags, MLineSegment},
        objects::{MLineStyleElement, ObjectType},
        selection::{CaptureRequest, DocumentSelection, SelectionRecordKey, SelectionRecordKind},
        tables::{LineType, LineTypeElement},
        types::{Color, LineWeight},
        Document, Handle, Vector3,
    };

    let (bytes, high, gap, style, layer) = {
        let mut source = Document::new();
        let mut pattern = LineType::new("Dash");
        pattern.handle = source.allocate_handle();
        pattern.elements = vec![LineTypeElement::dash(1.0), LineTypeElement::space(0.5)];
        pattern.pattern_length = 1.5;
        source.line_types.add(pattern).unwrap();
        let style = source.header.current_multiline_style_handle;
        let ObjectType::MLineStyle(s) = source.objects.get_mut(&style).unwrap() else {
            panic!()
        };
        s.elements = vec![
            MLineStyleElement::full(-0.5, Color::ByLayer, "Dash"),
            MLineStyleElement::full(0.5, Color::ByBlock, "Dash"),
        ];

        let mut line = Line::from_points(Vector3::ZERO, Vector3::new(1.0, 2.0, 0.0));
        line.thickness = -0.0;
        line.common.line_weight = LineWeight::Value(-1);
        let l = source.add_entity(EntityType::Line(line)).unwrap();

        let mut ml = MLine::from_points(&[Vector3::ZERO, Vector3::new(1.0, 0.0, 0.0)]);
        ml.common.handle = Handle::new((1_u64 << 53) + 1);
        ml.flags = MLineFlags::from_bits_retain(-32767);
        ml.style_handle = None;
        ml.vertices[0].segments = vec![MLineSegment {
            parameters: vec![f64::from_bits(0x7ff8000000000042), -0.0],
            area_fill_parameters: vec![f64::INFINITY],
        }];
        let high = source.add_entity(EntityType::MLine(ml)).unwrap();

        let mut ml = MLine::new();
        ml.common.linetype = "Dash".into();
        ml.common.linetype_handle = Some(Handle::new(u64::MAX));
        ml.style_handle = Some(style);
        let gap = source.add_entity(EntityType::MLine(ml)).unwrap();
        let layer = SelectionRecordKey {
            kind: SelectionRecordKind::Layer,
            handle: source.layers.get("0").unwrap().handle,
        };
        let mut request = CaptureRequest::new("draw A", vec![l, high, gap]);
        request.external.insert(layer);
        let capture = DocumentSelection::capture(&source, &request)
            .unwrap()
            .selection;
        (encode(&capture).unwrap(), high, gap, style, layer)
    };

    let selection = decode(&bytes).unwrap();
    assert_eq!(selection.namespace(), "draw A");
    assert_eq!(selection.roots()[1].handle, high);
    assert_eq!(selection.roots()[2].handle, gap);
    let SelectionPayload::Line(line) = &selection.records()[0].payload else {
        panic!()
    };
    assert_eq!(line.thickness.to_bits(), (-0.0_f64).to_bits());
    assert_eq!(line.common.line_weight, LineWeight::Value(-1));
    let SelectionPayload::MLine(ml) = &selection.records()[1].payload else {
        panic!()
    };
    assert!(ml.style_handle.is_none());
    assert_eq!(ml.flags.bits(), -32767);
    assert_eq!(
        ml.vertices[0].segments[0].parameters[0].to_bits(),
        0x7ff8000000000042
    );
    assert_eq!(
        ml.vertices[0].segments[0].parameters[1].to_bits(),
        (-0.0_f64).to_bits()
    );
    assert_eq!(
        ml.vertices[0].segments[0].area_fill_parameters[0],
        f64::INFINITY
    );
    assert_eq!(
        selection
            .records()
            .iter()
            .filter(|r| r.key.handle == style)
            .count(),
        1
    );
    assert_eq!(
        selection
            .requirements()
            .iter()
            .filter(|r| r.disposition == RequirementDisposition::External(layer))
            .count(),
        3
    );
    let r = selection
        .requirements()
        .iter()
        .find(|r| r.source.handle == gap && r.slot == SelectionSlot::LineType)
        .unwrap();
    assert_eq!(r.source_handle.unwrap().value(), u64::MAX);
    assert_eq!(r.disposition, RequirementDisposition::Missing);
    assert!(selection
        .capture_report()
        .iter()
        .any(|e| e.code == CaptureEvidenceCode::ResolvedSourceDefault
            && e.source.is_some_and(|k| k.handle == high)));
    assert_eq!(encode(&selection).unwrap(), bytes);
}

#[test]
fn rejects_external_record_conflicts_and_unreachable_records() {
    let body = &MIXED[15..];
    let included_layer = b"\xa8included\x82\xa4kind\xa5layer\xa6handle\x20";
    let external_layer = b"\xa8external\x82\xa4kind\xa5layer\xa6handle\x20";
    let conflict = decode(&frame(&replace(body, included_layer, external_layer))).unwrap_err();
    assert_eq!(conflict.kind, Kind::InvalidSelection);
    assert!(conflict.message.contains("external endpoint"));

    let record_start = b"\x82\xa3key\x82\xa4kind\xa8linetype\xa6handle\x21";
    let record_end = b"\x82\xa3key\x82\xa4kind\xa6object\xa6handle\x22";
    let start = body
        .windows(record_start.len())
        .position(|w| w == record_start)
        .unwrap();
    let end = body
        .windows(record_end.len())
        .position(|w| w == record_end)
        .unwrap();
    let mut records = b"\xa7records\x91".to_vec();
    records.extend_from_slice(&body[start..end]);
    let orphan = decode(&frame(&replace(&EMPTY[15..], b"\xa7records\x90", &records))).unwrap_err();
    assert_eq!(orphan.kind, Kind::InvalidSelection);
    assert!(orphan.message.contains("not reachable"));
}

#[test]
fn rejects_duplicate_roots_and_missing_included_records() {
    let body = &MIXED[15..];
    let high = b"\x82\xa4kind\xa6entity\xa6handle\xcf\x00\x20\x00\x00\x00\x00\x00\x01";
    let first = b"\x82\xa4kind\xa6entity\xa6handle\x10";
    let duplicate = decode(&frame(&replace(body, high, first))).unwrap_err();
    assert_eq!(duplicate.kind, Kind::InvalidSelection);
    assert_eq!(duplicate.path.as_deref(), Some("/roots/1"));

    let dangling = replace(body, b"\xaclayer_handle\x20", b"\xaclayer_handle\x28");
    let dangling = replace(
        &dangling,
        b"\xadsource_handle\x20",
        b"\xadsource_handle\x28",
    );
    let dangling = replace(
        &dangling,
        b"\xa8included\x82\xa4kind\xa5layer\xa6handle\x20",
        b"\xa8included\x82\xa4kind\xa5layer\xa6handle\x28",
    );
    let missing = decode(&frame(&dangling)).unwrap_err();
    assert_eq!(missing.kind, Kind::InvalidSelection);
    assert!(missing.message.contains("included endpoint has no record"));
}
