#![cfg(feature = "selection-storage")]

use std::collections::BTreeMap;

use opencaddocument::entities::Insert;
use opencaddocument::objects::{ObjectType, SortEntitiesTable};
use opencaddocument::selection::*;
use opencaddocument::selection_storage::{decode, encode, SelectionStorageErrorKind};
use opencaddocument::tables::BlockRecord;
use opencaddocument::{Document, Dwg, Dxf, EntityType, Format, Handle, Line, Vector3};

fn line(doc: &mut Document, x: f64) -> Handle {
    doc.add_entity(EntityType::Line(Line::from_points(
        Vector3::new(x, 0.0, 0.0),
        Vector3::new(x, 1.0, 0.0),
    )))
    .unwrap()
}

fn draw_order(doc: &Document) -> Vec<i32> {
    draw_order_in_block(doc, doc.header.model_space_block_handle)
}

fn draw_order_in_block(doc: &Document, owner: Handle) -> Vec<i32> {
    let block = doc
        .block_records
        .iter()
        .find(|b| b.handle == owner)
        .unwrap();
    let ext = doc.extension_dictionary_handle(block.handle).unwrap();
    let ObjectType::Dictionary(dictionary) = &doc.objects[&ext] else {
        panic!("missing extension dictionary")
    };
    let key = dictionary.get(SortEntitiesTable::DICTIONARY_KEY).unwrap();
    let ObjectType::SortEntitiesTable(sort) = &doc.objects[&key] else {
        panic!("missing SortEnts table")
    };
    assert_eq!(sort.owner_handle, ext);
    assert_eq!(sort.block_owner_handle, block.handle);
    let mut order = block.entity_handles.clone();
    order.sort_by_key(|h| sort.effective_sort_handle(*h).value());
    order
        .iter()
        .filter_map(|h| match doc.get_entity(*h).unwrap() {
            EntityType::Line(line) => {
                assert_eq!(line.common.owner_handle, block.handle);
                Some(line.start.x as i32)
            }
            EntityType::MLine(v) => {
                assert_eq!(v.common.owner_handle, block.handle);
                Some(v.vertices[0].position.x as i32)
            }
            EntityType::Viewport(v) if v.id == 1 => None,
            _ => panic!("expected a selected Line or MLINE"),
        })
        .collect()
}

fn layout(doc: &Document, name: &str) -> (Handle, Handle) {
    doc.objects
        .iter()
        .find_map(|(&handle, object)| match object {
            ObjectType::Layout(layout) if layout.name == name => {
                Some((handle, layout.block_record))
            }
            _ => None,
        })
        .unwrap()
}

fn sheet_line(doc: &mut Document, sheet: &str, x: f64) -> Handle {
    doc.add_entity_to_layout(
        EntityType::Line(Line::from_points(
            Vector3::new(x, 0.0, 0.0),
            Vector3::new(x, 1.0, 0.0),
        )),
        sheet,
    )
    .unwrap()
}

fn paper_selection_roundtrips(format: &dyn Format) {
    let bytes = {
        let mut source = Document::new();
        source.add_layout("Source B").unwrap();
        let roots = [("Layout1", 90.0), ("Source B", 100.0)]
            .into_iter()
            .flat_map(|(sheet, x)| {
                let line = sheet_line(&mut source, sheet, x);
                let mut mline = opencaddocument::entities::MLine::from_points(&[
                    Vector3::new(x + 1.0, 0.0, 0.0),
                    Vector3::new(x + 1.0, 2.0, 0.0),
                ]);
                mline.vertices[0].segments[0].parameters = vec![x / 10.0];
                let mline = source
                    .add_entity_to_layout(EntityType::MLine(mline), sheet)
                    .unwrap();
                [line, mline]
            })
            .collect();
        let (cad, losses) = format.write(&source).unwrap();
        assert!(losses.is_empty(), "{losses:?}");
        let (source, losses) = format.read(&cad).unwrap();
        assert!(losses.is_empty(), "{losses:?}");
        let mut request = CaptureRequest::new("two sheets", roots);
        request.external.insert(SelectionRecordKey {
            kind: SelectionRecordKind::Layer,
            handle: source.layers.get("0").unwrap().handle,
        });
        let selection = DocumentSelection::capture(&source, &request)
            .unwrap()
            .selection;
        let owners: Vec<_> = selection
            .requirements()
            .iter()
            .filter(|r| r.slot == SelectionSlot::Owner)
            .collect();
        assert_eq!(owners.len(), 4);
        let (owner_groups, _) = owners.as_chunks::<2>();
        for (requirements, name) in owner_groups.iter().zip(["Layout1", "Source B"]) {
            assert!(requirements.iter().all(|r| {
                r.disposition == RequirementDisposition::Context(ContextRole::PaperSpace)
                    && r.source_handle == Some(layout(&source, name).1)
            }));
        }
        let bytes = encode(&selection).unwrap();
        assert_eq!(&bytes[8..10], &[0, 2]);
        assert_eq!(decode(&bytes).unwrap(), selection);
        let mut wrong_version = bytes.clone();
        wrong_version[9] = 1;
        assert!(decode(&wrong_version).is_err());
        bytes
    };
    let selection = decode(&bytes).unwrap();

    let mut target = Document::new();
    target.add_layout("Target B").unwrap();
    let layer = target.layers.get("0").unwrap().handle;
    let anchor_a = sheet_line(&mut target, "Layout1", 10.0);
    let anchor_b = sheet_line(&mut target, "Target B", 20.0);
    let (layout_a, owner_a) = layout(&target, "Layout1");
    let (layout_b, _) = layout(&target, "Target B");
    let style = selection
        .records()
        .iter()
        .find(|r| matches!(r.payload, SelectionPayload::MLineStyle(_)))
        .unwrap()
        .key;
    let roots = |indices: std::ops::Range<usize>, anchor| {
        selection.roots()[indices]
            .iter()
            .map(|&root| RootReconstruction {
                root,
                transform: SelectionPlacement::Identity,
                order: SelectionDrawOrder::Before(anchor),
                layer: Some(layer),
                linetype: None,
                appearance: RootAppearanceOverrides::default(),
            })
            .collect()
    };
    let mut request = PaperSpaceReconstructionRequest {
        layout: owner_a, // A block handle is distinct from the Layout handle.
        coordinates: CoordinatePolicy::DrawingCoordinates,
        roots: roots(0..2, anchor_a),
        resources: BTreeMap::from([(
            style,
            ResourceAction::Create {
                name: "Restored sheets".into(),
            },
        )]),
        external: vec![],
    };
    assert!(matches!(
        selection.reconstruct_paperspace(&target, &request),
        Err(ReconstructionError::UnsafeTarget(_))
    ));
    request.layout = layout_a;
    request.roots = roots(0..4, anchor_a);
    assert!(matches!(
        selection.reconstruct_paperspace(&target, &request),
        Err(ReconstructionError::InvalidRequest(_))
    ));
    request.roots = roots(0..2, anchor_b);
    let blocked = selection.reconstruct_paperspace(&target, &request).unwrap();
    assert!(blocked.roots.iter().all(|r| matches!(&r.status,
        RootReconstructionStatus::Blocked { reasons } if reasons[0].kind == ReconstructionBlockerKind::InvalidOrderAnchor
    )));
    assert_eq!(blocked.document, target);
    assert_eq!(blocked.document.next_handle(), target.next_handle());
    assert!(blocked.resources.is_empty());

    request.roots = roots(0..2, anchor_a);
    let first = selection.reconstruct_paperspace(&target, &request).unwrap();
    assert!(
        first
            .roots
            .iter()
            .all(|r| matches!(r.status, RootReconstructionStatus::Applied { .. })),
        "{:?}",
        first.roots
    );
    request.layout = layout_b;
    request.roots = roots(2..4, anchor_b);
    request
        .resources
        .insert(style, ResourceAction::Reuse(first.resources[&style]));
    let second = selection
        .reconstruct_paperspace(&first.document, &request)
        .unwrap();
    assert!(
        second
            .roots
            .iter()
            .all(|r| matches!(r.status, RootReconstructionStatus::Applied { .. })),
        "{:?}",
        second.roots
    );
    let (cad, losses) = format.write(&second.document).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let (read, losses) = format.read(&cad).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let audit = read.audit();
    assert_eq!(audit.error_count(), 0, "{audit:?}");
    let ObjectType::Dictionary(styles) = &read.objects[&read.header.acad_mlinestyle_dict_handle]
    else {
        panic!("missing style dictionary");
    };
    let style = styles.get("Restored sheets").unwrap();
    for (sheet, order, parameters) in [
        ("Layout1", [90, 91, 10], [9.0]),
        ("Target B", [100, 101, 20], [10.0]),
    ] {
        let owner = layout(&read, sheet).1;
        assert_eq!(
            draw_order_in_block(&read, owner),
            order,
            "{} {sheet}",
            format.name()
        );
        let mlines: Vec<_> = read
            .entities()
            .filter_map(|e| match e {
                EntityType::MLine(v) if v.common.owner_handle == owner => Some(v),
                _ => None,
            })
            .collect();
        assert_eq!(mlines.len(), 1);
        assert_eq!(mlines[0].style_handle, Some(style));
        assert_eq!(mlines[0].vertices[0].segments[0].parameters, parameters);
    }
}

#[test]
fn paper_selection_roundtrips_dwg() {
    paper_selection_roundtrips(&Dwg);
}

#[test]
fn paper_selection_roundtrips_dxf() {
    paper_selection_roundtrips(&Dxf);
}

fn add_definition(doc: &mut Document, name: &str) -> Handle {
    let mut record = BlockRecord::new(name);
    record.handle = doc.allocate_handle();
    record.block_entity_handle = doc.allocate_handle();
    record.block_end_handle = doc.allocate_handle();
    let handle = record.handle;
    doc.block_records.add(record).unwrap();
    handle
}

fn add_nested_definitions(doc: &mut Document) -> [Handle; 2] {
    add_definition(doc, "Inner");
    add_definition(doc, "Outer");
    add_member(
        doc,
        EntityType::Insert(Insert::new("Inner", Vector3::new(4.0, 5.0, 0.0))),
        "Outer",
    );
    [50.0, 60.0].map(|x| {
        doc.add_entity(EntityType::Insert(Insert::new(
            "Outer",
            Vector3::new(x, 10.0, 0.0),
        )))
        .unwrap()
    })
}

fn add_member(doc: &mut Document, mut entity: EntityType, definition: &str) -> Handle {
    entity.common_mut().owner_handle = doc.block_records.get(definition).unwrap().handle;
    doc.add_entity(entity).unwrap()
}

fn nested_roots(doc: &mut Document) -> [Handle; 2] {
    let mut line = Line::from_points(Vector3::new(90.0, 0.0, 0.0), Vector3::new(90.0, 2.0, 0.0));
    line.common.linetype = "ByBlock".into();
    let mut mline = opencaddocument::entities::MLine::from_points(&[
        Vector3::new(91.0, 0.0, 0.0),
        Vector3::new(91.0, 2.0, 0.0),
    ]);
    mline.vertices[0].segments[0].parameters = vec![2.5];
    [EntityType::Line(line), EntityType::MLine(mline)]
        .map(|entity| add_member(doc, entity, "Inner"))
}

#[test]
fn captures_nested_block_members_by_their_direct_owner() {
    let mut source = Document::new();
    add_nested_definitions(&mut source);
    let roots = nested_roots(&mut source);
    let selection =
        DocumentSelection::capture(&source, &CaptureRequest::new("nested", roots.into()))
            .unwrap()
            .selection;
    let expected = source.block_records.get("Inner").unwrap().handle;
    let owners: Vec<_> = selection
        .requirements()
        .iter()
        .filter(|r| r.slot == SelectionSlot::Owner)
        .map(|r| (r.source_handle, r.source_name.as_deref(), &r.disposition))
        .collect();
    let disposition = RequirementDisposition::Context(ContextRole::BlockDefinition);
    assert_eq!(owners, [(Some(expected), Some("Inner"), &disposition); 2]);
}

#[test]
fn rejects_external_or_inconsistent_block_owners() {
    let mut doc = Document::new();
    add_nested_definitions(&mut doc);
    let [line, _] = nested_roots(&mut doc);
    let request = CaptureRequest::new("invalid owner", vec![line]);
    doc.block_records
        .get_mut("Inner")
        .unwrap()
        .flags
        .is_external = true;
    let err = DocumentSelection::capture(&doc, &request).unwrap_err();
    assert_eq!(err.kind, SelectionErrorKind::UnsupportedContent);
    assert_eq!(err.slot, Some(SelectionSlot::Owner));

    doc.block_records
        .get_mut("Inner")
        .unwrap()
        .flags
        .is_external = false;
    doc.get_entity_mut(line).unwrap().common_mut().owner_handle =
        doc.block_records.get("Outer").unwrap().handle;
    let err = DocumentSelection::capture(&doc, &request).unwrap_err();
    assert_eq!(err.kind, SelectionErrorKind::ReferenceConflict);
    assert_eq!(err.slot, Some(SelectionSlot::Owner));
}

fn nested_selection_roundtrips(format: &dyn Format) {
    let mut target = Document::new();
    add_definition(&mut target, "Unrelated");
    let [moving, stationary] = add_nested_definitions(&mut target);
    let Some(EntityType::Insert(insert)) = target.get_entity_mut(stationary) else {
        panic!("expected the stationary INSERT");
    };
    let stationary_rotation = std::f64::consts::FRAC_PI_6;
    insert.rotation = stationary_rotation;
    insert.set_x_scale(1.5);

    let bytes = {
        let mut source = Document::new();
        add_nested_definitions(&mut source);
        let [line, mline] = nested_roots(&mut source);
        let outer_line = add_member(
            &mut source,
            EntityType::Line(Line::from_points(
                Vector3::ZERO,
                Vector3::new(1.0, 0.0, 0.0),
            )),
            "Outer",
        );
        let (cad, losses) = format.write(&source).unwrap();
        assert!(losses.is_empty(), "{losses:?}");
        let (source, losses) = format.read(&cad).unwrap();
        assert!(losses.is_empty(), "{losses:?}");
        let mut request = CaptureRequest::new("nested source", vec![line, mline, outer_line]);
        request.external.insert(SelectionRecordKey {
            kind: SelectionRecordKind::Layer,
            handle: source.layers.get("0").unwrap().handle,
        });
        let selection = DocumentSelection::capture(&source, &request)
            .unwrap()
            .selection;
        let bytes = encode(&selection).unwrap();
        assert_eq!(&bytes[8..10], &[0, 3]);
        assert_eq!(decode(&bytes).unwrap(), selection);
        for old_version in [1, 2] {
            let mut backdated = bytes.clone();
            backdated[9] = old_version;
            let err = decode(&backdated).unwrap_err();
            assert_eq!(err.kind, SelectionStorageErrorKind::InvalidSelection);
            assert!(err.message.contains("later storage version"));
        }
        bytes
    };
    let selection = decode(&bytes).unwrap();
    let Some(EntityType::Insert(insert)) = target.get_entity_mut(moving) else {
        panic!("expected the moving INSERT");
    };
    let moved_point = Vector3::new(35.0, -15.0, 4.0);
    let moved_rotation = std::f64::consts::FRAC_PI_3;
    insert.insert_point = moved_point;
    insert.rotation = moved_rotation;
    let inner = target.block_records.get("Inner").unwrap().handle;
    let source_owner = selection
        .requirements()
        .iter()
        .find(|r| r.slot == SelectionSlot::Owner)
        .unwrap()
        .source_handle;
    assert_ne!(source_owner, Some(inner));
    let outer = target.block_records.get("Outer").unwrap();
    let outer_insert = outer.entity_handles[0];
    let anchor = add_member(
        &mut target,
        EntityType::Line(Line::from_points(
            Vector3::new(10.0, 0.0, 0.0),
            Vector3::new(10.0, 1.0, 0.0),
        )),
        "Inner",
    );
    let layer = target.layers.get("0").unwrap().handle;
    let style = selection
        .records()
        .iter()
        .find(|r| matches!(r.payload, SelectionPayload::MLineStyle(_)))
        .unwrap()
        .key;
    let target_style = target.header.current_multiline_style_handle;
    let placements = |roots: &[SelectionRecordKey], anchor| {
        roots
            .iter()
            .map(|&root| RootReconstruction {
                root,
                transform: SelectionPlacement::Identity,
                order: SelectionDrawOrder::Before(anchor),
                layer: Some(layer),
                linetype: None,
                appearance: RootAppearanceOverrides::default(),
            })
            .collect()
    };
    let mut request = BlockDefinitionReconstructionRequest {
        block_record: target.header.model_space_block_handle,
        coordinates: CoordinatePolicy::DrawingCoordinates,
        roots: placements(&selection.roots()[..2], anchor),
        resources: BTreeMap::from([(style, ResourceAction::Reuse(target_style))]),
        external: vec![],
    };
    assert!(matches!(
        selection.reconstruct_in_block_definition(&target, &request),
        Err(ReconstructionError::UnsafeTarget(_))
    ));
    request.block_record = inner;
    request.roots = placements(selection.roots(), anchor);
    assert!(matches!(
        selection.reconstruct_in_block_definition(&target, &request),
        Err(ReconstructionError::InvalidRequest(_))
    ));
    request.roots = placements(&selection.roots()[..2], outer_insert);
    let blocked = selection
        .reconstruct_in_block_definition(&target, &request)
        .unwrap();
    assert!(blocked.roots.iter().all(|r| matches!(&r.status, RootReconstructionStatus::Blocked { reasons } if reasons[0].kind == ReconstructionBlockerKind::InvalidOrderAnchor)));
    assert_eq!(blocked.document, target);
    assert_eq!(blocked.document.next_handle(), target.next_handle());

    request.roots = placements(&selection.roots()[..2], anchor);
    let outcome = selection
        .reconstruct_in_block_definition(&target, &request)
        .unwrap();
    assert!(
        outcome
            .roots
            .iter()
            .all(|r| matches!(r.status, RootReconstructionStatus::Applied { .. })),
        "{:?}",
        outcome.roots
    );
    assert_eq!(outcome.resources[&style], target_style);
    for insert in target
        .entities()
        .filter(|e| matches!(e, EntityType::Insert(_)))
    {
        assert_eq!(
            outcome.document.get_entity(insert.common().handle),
            Some(insert)
        );
    }

    let (cad, losses) = format.write(&outcome.document).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let (read, losses) = format.read(&cad).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let inner = read.block_records.get("Inner").unwrap();
    let outer = read.block_records.get("Outer").unwrap();
    assert_eq!(draw_order_in_block(&read, inner.handle), [90, 91, 10]);
    let line = inner
        .entity_handles
        .iter()
        .find_map(|&h| match read.get_entity(h) {
            Some(EntityType::Line(line)) if line.start.x == 90.0 => Some(line),
            _ => None,
        })
        .unwrap();
    assert!(line.common.linetype.eq_ignore_ascii_case("ByBlock"));
    assert_eq!(line.end, Vector3::new(90.0, 2.0, 0.0));
    let mline = inner
        .entity_handles
        .iter()
        .find_map(|&h| match read.get_entity(h) {
            Some(EntityType::MLine(mline)) => Some(mline),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        mline.style_handle,
        Some(read.header.current_multiline_style_handle)
    );
    assert_eq!(mline.vertices[0].segments[0].parameters, [2.5]);

    let [nested] = outer.entity_handles.as_slice() else {
        panic!("expected one outer block member");
    };
    let Some(EntityType::Insert(nested)) = read.get_entity(*nested) else {
        panic!("expected the nested INSERT");
    };
    assert_eq!(nested.common.owner_handle, outer.handle);
    assert_eq!(nested.block_name, "Inner");
    assert_eq!(nested.insert_point, Vector3::new(4.0, 5.0, 0.0));
    let mut model_inserts: Vec<_> = read
        .entities()
        .filter_map(|e| match e {
            EntityType::Insert(insert)
                if insert.common.owner_handle == read.header.model_space_block_handle =>
            {
                Some(insert)
            }
            _ => None,
        })
        .collect();
    model_inserts.sort_by(|a, b| a.insert_point.x.total_cmp(&b.insert_point.x));
    let [moved, stationary] = model_inserts.as_slice() else {
        panic!("expected two ModelSpace INSERTs");
    };
    for (insert, point, rotation, x_scale) in [
        (moved, moved_point, moved_rotation, 1.0),
        (
            stationary,
            Vector3::new(60.0, 10.0, 0.0),
            stationary_rotation,
            1.5,
        ),
    ] {
        assert_eq!(insert.block_name, "Outer");
        assert_eq!(insert.insert_point, point);
        assert!(
            (insert.rotation - rotation).abs() < 1e-12,
            "{} INSERT at {point:?}: rotation {} != {rotation}",
            format.name(),
            insert.rotation
        );
        assert_eq!(
            (insert.x_scale(), insert.y_scale(), insert.z_scale()),
            (x_scale, 1.0, 1.0)
        );
    }
    let audit = read.audit();
    assert_eq!(audit.error_count(), 0, "{audit:?}");
}

#[test]
fn nested_selection_roundtrips_dwg() {
    nested_selection_roundtrips(&Dwg);
}

#[test]
fn nested_selection_roundtrips_dxf() {
    nested_selection_roundtrips(&Dxf);
}

fn selection_v1_roundtrips_middle_insert(format: &dyn Format) {
    let bytes = {
        let mut source = Document::new();
        let root = line(&mut source, 90.0);
        let mut mline = opencaddocument::entities::MLine::from_points(&[
            Vector3::new(91.0, 0.0, 0.0),
            Vector3::new(91.0, 2.0, 0.0),
        ]);
        mline.vertices[0].segments[0].parameters = vec![2.5];
        let multiline = source.add_entity(EntityType::MLine(mline.clone())).unwrap();
        let blocked = source.add_entity(EntityType::MLine(mline)).unwrap();
        let mut request = CaptureRequest::new("stored", vec![root, multiline, blocked]);
        request.external.insert(SelectionRecordKey {
            kind: SelectionRecordKind::Layer,
            handle: source.layers.get("0").unwrap().handle,
        });
        encode(
            &DocumentSelection::capture(&source, &request)
                .unwrap()
                .selection,
        )
        .unwrap()
    };
    let selection = decode(&bytes).unwrap();
    let mut target = Document::new();
    let a = line(&mut target, 10.0);
    line(&mut target, 20.0);
    let c = line(&mut target, 30.0);
    let owner = target.header.model_space_block_handle;
    let ext = target.ensure_extension_dictionary(owner);
    let table = target.allocate_handle();
    let mut sort = SortEntitiesTable::for_block(owner);
    sort.owner_handle = ext;
    sort.handle = table;
    sort.add_entry(c, Handle::new(1));
    target
        .objects
        .insert(table, ObjectType::SortEntitiesTable(sort));
    let Some(ObjectType::Dictionary(dict)) = target.objects.get_mut(&ext) else {
        panic!()
    };
    dict.add_entry(SortEntitiesTable::DICTIONARY_KEY, table);

    let style = selection
        .records()
        .iter()
        .find_map(|r| matches!(r.payload, SelectionPayload::MLineStyle(_)).then_some(r.key))
        .unwrap();
    let layer = target.layers.get("0").unwrap().handle;
    let request = ReconstructionRequest {
        model_space: owner,
        coordinates: CoordinatePolicy::DrawingCoordinates,
        roots: selection
            .roots()
            .iter()
            .enumerate()
            .map(|(i, &root)| RootReconstruction {
                root,
                transform: SelectionPlacement::Identity,
                order: SelectionDrawOrder::Before(a),
                layer: (i < 2).then_some(layer),
                linetype: None,
                appearance: RootAppearanceOverrides::default(),
            })
            .collect(),
        resources: BTreeMap::from([(
            style,
            ResourceAction::Create {
                name: "Restored".into(),
            },
        )]),
        external: vec![],
    };
    let candidate = selection.reconstruct_available(&target, &request).unwrap();
    assert!(matches!(
        candidate.roots[0].status,
        RootReconstructionStatus::Applied { .. }
    ));
    assert!(matches!(
        candidate.roots[1].status,
        RootReconstructionStatus::Applied { .. }
    ));
    assert!(matches!(
        candidate.roots[2].status,
        RootReconstructionStatus::Blocked { .. }
    ));
    assert_eq!(draw_order(&target), [30, 10, 20]);
    assert_eq!(draw_order(&candidate.document), [30, 90, 91, 10, 20]);
    let (cad, write_losses) = format.write(&candidate.document).unwrap();
    assert!(write_losses.is_empty(), "{write_losses:?}");
    let (read, read_losses) = format.read(&cad).unwrap();
    assert!(read_losses.is_empty(), "{read_losses:?}");
    assert_eq!(draw_order(&read), [30, 90, 91, 10, 20], "{}", format.name());
    let v = read
        .entities()
        .find_map(|e| match e {
            EntityType::MLine(v) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(v.style_name, "Restored");
    assert_eq!(v.vertices[0].segments[0].parameters, [2.5]);
    let dictionary = read.header.acad_mlinestyle_dict_handle;
    let ObjectType::Dictionary(dict) = &read.objects[&dictionary] else {
        panic!()
    };
    let h = dict.get("Restored").unwrap();
    assert_eq!(v.style_handle, Some(h));
    assert!(
        matches!(&read.objects[&h], ObjectType::MLineStyle(style) if style.owner == dictionary)
    );
    let audit = read.audit();
    assert_eq!(audit.error_count(), 0, "{audit:?}");
}

#[test]
fn middle_insert_roundtrips_dxf() {
    selection_v1_roundtrips_middle_insert(&Dxf);
}

#[test]
fn middle_insert_roundtrips_dwg() {
    selection_v1_roundtrips_middle_insert(&Dwg);
}

#[cfg(feature = "ifcx")]
#[test]
fn reconstructs_into_a_real_logical_ifccad_import() {
    use opencaddocument::tables::Layer;
    use opencaddocument::Ifcx;

    let mut native = Document::new();
    let first = line(&mut native, 10.0);
    let second = line(&mut native, 20.0);
    let mut layer = Layer::new("Current");
    layer.handle = native.allocate_handle();
    let h = layer.handle;
    native.layers.add(layer).unwrap();
    let second_common = native.get_entity_mut(second).unwrap().common_mut();
    second_common.layer = "Current".into();
    second_common.layer_handle = Some(h);
    let exported = Ifcx.export_ifccad(&native).unwrap();
    let imported = Ifcx.import_ifccad(&exported.drawing).unwrap();
    let first = imported.entity_handles[&exported.entity_ids[&first]];
    let second = imported.entity_handles[&exported.entity_ids[&second]];
    let current = imported.document.layers.get("Current").unwrap().handle;

    let mut source = Document::new();
    let root = line(&mut source, 90.0);
    let mut capture = CaptureRequest::new("stored", vec![root]);
    capture.external.insert(SelectionRecordKey {
        kind: SelectionRecordKind::Layer,
        handle: source.layers.get("0").unwrap().handle,
    });
    let stored = DocumentSelection::capture(&source, &capture)
        .unwrap()
        .selection;
    let selection = decode(&encode(&stored).unwrap()).unwrap();
    let request = ReconstructionRequest {
        model_space: imported.document.header.model_space_block_handle,
        coordinates: CoordinatePolicy::DrawingCoordinates,
        roots: vec![RootReconstruction {
            root: selection.roots()[0],
            transform: SelectionPlacement::Identity,
            order: SelectionDrawOrder::Before(second),
            layer: Some(current),
            linetype: None,
            appearance: RootAppearanceOverrides::default(),
        }],
        resources: BTreeMap::new(),
        external: vec![],
    };
    let out = selection
        .reconstruct_available(&imported.document, &request)
        .unwrap();
    assert!(
        matches!(out.roots[0].status, RootReconstructionStatus::Applied { layer, .. } if layer == current)
    );
    assert_eq!(draw_order(&out.document), [10, 90, 20]);
    for handle in [first, second] {
        assert_eq!(
            out.document.get_entity(handle),
            imported.document.get_entity(handle)
        );
    }
}
