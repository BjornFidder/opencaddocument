#![cfg(feature = "ifcx")]
use opencaddocument::entities::{AttributeDefinition, Line};
use opencaddocument::fields::NewField;
use opencaddocument::ifccad::{self, IfccadAttributeFieldCodePart as Part, IfccadEntityKind};
use opencaddocument::{Document, EntityType, Handle, Ifcx, Vector3};

fn field_drawing(object: bool) -> (Document, Handle, Handle) {
    let mut d = Document::new();
    let host = d
        .add_entity(EntityType::AttributeDefinition(AttributeDefinition::new(
            "TITLE".into(),
            "Title".into(),
            "old".into(),
        )))
        .unwrap();
    let line = d
        .add_entity(EntityType::Line(Line::from_points(
            Vector3::ZERO,
            Vector3::new(12.5, 0., 0.),
        )))
        .unwrap();
    let mut field = if object {
        NewField::new(
            "\\AcObjProp Object(%<\\_ObjIdx 0>%).Length \\f \"%lu2%pr2\"",
            "12.50",
        )
    } else {
        NewField::new("\\AcVar Filename \\f \"%fn6\"", "doors.dwg")
    };
    if object {
        field.objects = vec![line];
    }
    d.set_text_field(host, "Value: %<\\_FldIdx 0>%", vec![field])
        .unwrap();
    (d, host, line)
}

#[test]
fn selected_fields_reopen_durably_and_rebind_object_targets() {
    for object in [false, true] {
        let (d, host, line) = field_drawing(object);
        let native = Ifcx.export_ifccad(&d).unwrap();
        let host_id = native.entity_ids[&host];
        let line_id = native.entity_ids[&line];
        let definition = native
            .drawing
            .model
            .entities
            .iter()
            .filter_map(|e| e.as_native())
            .find(|e| e.id == host_id)
            .unwrap();
        let IfccadEntityKind::AttributeDefinition(a) = &definition.kind else {
            panic!()
        };
        let fields = a.fields.clone().expect("stored field graph");
        assert_eq!(fields.records.len(), 2);
        assert!(!fields.records[0].auxiliary_values.is_empty());
        if object {
            assert!(fields.records[1]
                .code
                .iter()
                .any(|p| matches!(p,Part::Object(id) if *id==line_id)));
        }
        drop(d);
        let bytes = ifccad::encode_ifccad_cbor_document(&native.drawing).unwrap();
        let reopened = ifccad::load_ifccad_cbor_bytes(bytes.bytes(), Default::default()).unwrap();
        let returned = Ifcx.import_ifccad(reopened.document()).unwrap();
        let h = returned.entity_handles[&host_id];
        let root = returned
            .document
            .objects
            .values()
            .find_map(|o| {
                if let opencaddocument::objects::ObjectType::Dictionary(d) = o {
                    (d.owner == h).then(|| d.get("ACAD_FIELD")).flatten()
                } else {
                    None
                }
            })
            .unwrap();
        let opencaddocument::objects::ObjectType::Dictionary(fd) =
            &returned.document.objects[&root]
        else {
            panic!()
        };
        let root = fd.get("TEXT").unwrap();
        let opencaddocument::objects::ObjectType::Field(f) = &returned.document.objects[&root]
        else {
            panic!()
        };
        assert_eq!(f.child_fields.len(), 1);
        let opencaddocument::objects::ObjectType::Field(child) =
            &returned.document.objects[&f.child_fields[0]]
        else {
            panic!()
        };
        if object {
            assert_eq!(
                child.referenced_objects,
                vec![returned.entity_handles[&line_id]]
            );
        } else {
            assert!(child.referenced_objects.is_empty());
        }
        let again = Ifcx.export_ifccad(&returned.document).unwrap();
        let a = again
            .drawing
            .model
            .entities
            .iter()
            .filter_map(|e| e.as_native())
            .find_map(|e| {
                if let IfccadEntityKind::AttributeDefinition(a) = &e.kind {
                    Some(a)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(a.fields.as_ref().unwrap(), &fields);
    }
}

#[test]
fn unsupported_evaluator_refuses_field_backed_attribute_instead_of_flattening() {
    let (mut d, h, _) = field_drawing(false);
    d.set_text_field(
        h,
        "%<\\_FldIdx 0>%",
        vec![NewField::new("\\AcDiesel $(getvar,date)", "cached")],
    )
    .unwrap();
    let error = Ifcx.export_ifccad(&d).unwrap_err();
    assert!(error.to_string().contains("outside selected"), "{error}");
}

#[test]
fn field_objects_and_cached_code_survive_physical_cad() {
    for object in [false, true] {
        let (d, _, _) = field_drawing(object);
        let n = Ifcx.export_ifccad(&d).unwrap();
        let imported = Ifcx.import_ifccad(&n.drawing).unwrap();
        for format in ["dwg", "ascii", "binary"] {
            let bytes = match format {
                "dwg" => opencaddocument::DwgWriter::write_to_vec(&imported.document).unwrap(),
                "binary" => opencaddocument::DxfWriter::new_binary(&imported.document)
                    .write_to_vec()
                    .unwrap(),
                _ => opencaddocument::DxfWriter::new(&imported.document)
                    .write_to_vec()
                    .unwrap(),
            };
            let r = if format == "dwg" {
                opencaddocument::DwgReader::from_stream(std::io::Cursor::new(bytes))
                    .read_with_stats()
                    .unwrap()
            } else {
                opencaddocument::DxfReader::from_reader(std::io::Cursor::new(bytes))
                    .unwrap()
                    .read_with_stats()
                    .unwrap()
            };
            assert_eq!(r.stats.recovered_errors, 0);
            assert_eq!(r.stats.skipped_source_records, 0);
            assert!(r.stats.stream_completed);
            let after = Ifcx.export_ifccad(&r.document).unwrap();
            let a = after
                .drawing
                .model
                .entities
                .iter()
                .filter_map(|e| e.as_native())
                .find_map(|e| {
                    if let IfccadEntityKind::AttributeDefinition(a) = &e.kind {
                        Some(a)
                    } else {
                        None
                    }
                })
                .unwrap();
            let fields = a.fields.as_ref().unwrap();
            assert_eq!(fields.records.len(), 2);
            assert_eq!(
                fields.records[1].cache.display,
                if object { "12.50" } else { "doors.dwg" }
            );
            assert!(fields.records[1].code.iter().any(
                |p| matches!(p,Part::Literal(s) if s.contains(if object{"%lu2%pr2"}else{"%fn6"}))
            ));
            if object {
                let id = fields.records[1]
                    .code
                    .iter()
                    .find_map(|p| {
                        if let Part::Object(id) = p {
                            Some(*id)
                        } else {
                            None
                        }
                    })
                    .unwrap();
                let rebound = Ifcx.import_ifccad(&after.drawing).unwrap();
                assert!(matches!(
                    rebound.document.get_entity(rebound.entity_handles[&id]),
                    Some(EntityType::Line(_))
                ));
            }
            let b = ifccad::encode_ifccad_cbor_document(&after.drawing).unwrap();
            ifccad::load_ifccad_cbor_bytes(b.bytes(), Default::default()).unwrap();
        }
    }
}
