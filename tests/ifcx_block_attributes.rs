#![cfg(feature = "ifcx")]
use opencaddocument::entities::{AttributeDefinition, AttributeEntity, Insert, MText};
use opencaddocument::ifccad::{self, IfccadEntityKind};
use opencaddocument::tables::BlockRecord;
use opencaddocument::{Document, EntityType, Handle, Ifcx, Vector3};

fn drawing() -> (Document, Handle, Handle) {
    let mut doc = Document::new();
    let mut record = BlockRecord::new("Door");
    record.handle = doc.allocate_handle();
    record.block_entity_handle = doc.allocate_handle();
    record.block_end_handle = doc.allocate_handle();
    record.base_point = Vector3::new(7., 9., 0.);
    let owner = record.handle;
    doc.block_records.add(record).unwrap();
    let mut def = AttributeDefinition::new("CODE".into(), "Door code".into(), "default".into());
    def.common.owner_handle = owner;
    def.flags.verify = true;
    let def = doc
        .add_entity(EntityType::AttributeDefinition(def))
        .unwrap();
    let mut value = AttributeEntity::simple("RENAMED", "D-101");
    value.attdef_handle = def;
    value.insertion_point = Vector3::new(101., 202., 0.);
    value.flags.invisible = true;
    value.lock_position = true;
    value.common.handle = doc.allocate_handle();
    let mut insert = Insert::new("Door", Vector3::new(100., 200., 0.));
    insert.rotation = 0.3;
    insert.set_x_scale(-2.);
    insert.set_y_scale(3.);
    insert.attributes.push(value);
    let ins = doc.add_entity(EntityType::Insert(insert)).unwrap();
    (doc, def, ins)
}

#[test]
fn definitions_values_and_reliable_binding_map_both_directions() {
    let (doc, def, ins) = drawing();
    let native = Ifcx.export_ifccad(&doc).unwrap();
    let def_id = native.entity_ids[&def];
    let insert_id = native.entity_ids[&ins];
    let instance = native
        .drawing
        .model
        .entities
        .iter()
        .filter_map(|e| e.as_native())
        .find(|e| e.id == insert_id)
        .unwrap();
    let IfccadEntityKind::BlockInstance { attributes, .. } = &instance.kind else {
        panic!("INSERT")
    };
    assert_eq!(attributes.len(), 1);
    assert_eq!(attributes[0].value.tag, "RENAMED");
    assert_eq!(attributes[0].value.definition_id, Some(def_id));
    assert_eq!(attributes[0].value.text.placement.origin, [101., 202., 0.]);
    assert_eq!(attributes[0].value.text.thickness, 0.);
    assert!(attributes[0].value.flags.invisible);
    assert!(attributes[0].value.lock_position);
    assert!(attributes[0].visible, "separate common visibility");
    let bytes = ifccad::encode_ifccad_document(&native.drawing).unwrap();
    let reopened = ifccad::load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    let returned = Ifcx.import_ifccad(reopened.document()).unwrap();
    let EntityType::Insert(i) = returned
        .document
        .get_entity(returned.entity_handles[&insert_id])
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(i.attributes.len(), 1);
    assert_eq!(i.attributes[0].value, "D-101");
    assert_eq!(
        i.attributes[0].insertion_point,
        Vector3::new(101., 202., 0.)
    );
    assert_eq!(
        i.attributes[0].attdef_handle,
        returned.entity_handles[&def_id]
    );
}

#[test]
fn empty_embedded_mtext_and_outer_fallback_remain_independent() {
    let (mut doc, _, ins) = drawing();
    if let Some(EntityType::Insert(i)) = doc.get_entity_mut(ins) {
        let a = &mut i.attributes[0];
        a.is_multiline = true;
        a.embedded_mtext = Some(Box::new(MText::with_value(
            "",
            Vector3::new(110., 210., 0.),
        )));
    }
    let native = Ifcx.export_ifccad(&doc).unwrap();
    let IfccadEntityKind::BlockInstance { attributes, .. } =
        &native.drawing.model.entities[0].as_native().unwrap().kind
    else {
        panic!()
    };
    assert_eq!(attributes[0].value.text.content[0].text, "D-101");
    let m = attributes[0].value.m_text.as_ref().expect("MText");
    assert!(m.text.content.iter().all(|p| p.inlines.is_empty()));
    let returned = Ifcx.import_ifccad(&native.drawing).unwrap();
    let handle = returned.entity_handles[&native.entity_ids[&ins]];
    let EntityType::Insert(i) = returned.document.get_entity(handle).unwrap() else {
        panic!()
    };
    assert_eq!(i.attributes[0].value, "D-101");
    assert_eq!(i.attributes[0].embedded_mtext.as_ref().unwrap().value, "");
}

#[test]
fn attributes_survive_physical_cad_and_strict_native_readback() {
    let (mut doc, _, ins) = drawing();
    if let Some(EntityType::Insert(i)) = doc.get_entity_mut(ins) {
        let a = &mut i.attributes[0];
        a.is_multiline = true;
        a.embedded_mtext = Some(Box::new(MText::with_value(
            "Room\\PSecond line",
            Vector3::new(110., 210., 0.),
        )));
    }
    for format in ["dwg", "ascii", "binary"] {
        let b = match format {
            "dwg" => opencaddocument::DwgWriter::write_to_vec(&doc).unwrap(),
            "binary" => opencaddocument::DxfWriter::new_binary(&doc)
                .write_to_vec()
                .unwrap(),
            _ => opencaddocument::DxfWriter::new(&doc)
                .write_to_vec()
                .unwrap(),
        };
        let r = if format == "dwg" {
            opencaddocument::DwgReader::from_stream(std::io::Cursor::new(b))
                .read_with_stats()
                .unwrap()
        } else {
            opencaddocument::DxfReader::from_reader(std::io::Cursor::new(b))
                .unwrap()
                .read_with_stats()
                .unwrap()
        };
        assert_eq!(r.stats.recovered_errors, 0, "{format}");
        assert_eq!(r.stats.skipped_source_records, 0, "{format}");
        assert!(r.stats.stream_completed);
        let native = Ifcx.export_ifccad(&r.document).unwrap();
        let wire = ifccad::encode_ifccad_document(&native.drawing).unwrap();
        ifccad::load_ifccad_bytes(wire.bytes(), Default::default()).unwrap();
        let a = native
            .drawing
            .model
            .entities
            .iter()
            .filter_map(|e| e.as_native())
            .find_map(|e| {
                if let IfccadEntityKind::BlockInstance { attributes, .. } = &e.kind {
                    Some(&attributes[0])
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(a.value.tag, "RENAMED", "{format}");
        assert_eq!(a.value.text.placement.origin, [101., 202., 0.], "{format}");
        assert!(a.value.flags.invisible);
        assert!(a.value.lock_position);
        assert!(a.value.m_text.is_some(), "{format}");
    }
}

#[test]
fn legacy_secondary_line_relationships_fold_into_one_mtext_attribute() {
    let (mut doc, _, ins) = drawing();
    if let Some(EntityType::Insert(i)) = doc.get_entity_mut(ins) {
        let a = &mut i.attributes[0];
        a.is_multiline = true;
        a.embedded_mtext = Some(Box::new(MText::with_value(
            "First\\PSecond",
            Vector3::new(101., 202., 0.),
        )));
    }
    let bytes = opencaddocument::DxfWriter::new(&doc)
        .with_version(opencaddocument::types::DxfVersion::AC1027)
        .write_to_vec()
        .unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.contains("ACAD_MLATT"));
    let read = opencaddocument::DxfReader::from_reader(std::io::Cursor::new(bytes))
        .unwrap()
        .read_with_stats()
        .unwrap();
    assert_eq!(read.stats.recovered_errors, 0);
    assert_eq!(read.stats.skipped_source_records, 0);
    assert!(read.stats.stream_completed);
    let n = Ifcx.export_ifccad(&read.document).unwrap();
    let a = n
        .drawing
        .model
        .entities
        .iter()
        .filter_map(|e| e.as_native())
        .find_map(|e| {
            if let IfccadEntityKind::BlockInstance { attributes, .. } = &e.kind {
                Some(attributes)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].value.tag, "RENAMED");
    assert!(a[0].value.m_text.is_some());
    let bytes = ifccad::encode_ifccad_document(&n.drawing).unwrap();
    ifccad::load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
}
