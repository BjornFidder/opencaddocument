#![cfg(feature = "ifcx")]

use acadrust::entities::{CellContent, Table, TableCell};
use acadrust::tables::BlockRecord;
use acadrust::types::Vector3;
use acadrust::EntityType;
use opencaddocument::{Document, Ifcx};
use std::io::Cursor;

#[test]
fn table_cell_and_regular_insert_share_a_partially_converted_block() {
    use acadrust::entities::{Circle, Insert, Mesh};
    let mut document = Document::new();
    let mut add_block = |name| {
        let mut record = BlockRecord::new(name);
        record.handle = document.allocate_handle();
        record.block_entity_handle = document.allocate_handle();
        record.block_end_handle = document.allocate_handle();
        let handle = record.handle;
        document.block_records.add(record).unwrap();
        handle
    };
    let outer = add_block("outer");
    let inner = add_block("inner");
    let mut nested = Insert::new("inner", Vector3::ZERO);
    nested.common.owner_handle = outer;
    document.add_entity(EntityType::Insert(nested)).unwrap();
    let mut circle = Circle::new();
    circle.radius = 1.0;
    circle.common.owner_handle = inner;
    document.add_entity(EntityType::Circle(circle)).unwrap();
    let mut mesh = Mesh::new();
    mesh.common.owner_handle = inner;
    document.add_entity(EntityType::Mesh(mesh)).unwrap();
    document
        .add_entity(EntityType::Insert(Insert::new("outer", Vector3::ZERO)))
        .unwrap();
    let mut table = Table::new(Vector3::ZERO, 1, 1);
    table.rows[0].cells[0] = TableCell::block(outer);
    let table_handle = document
        .add_entity(EntityType::Table(Box::new(table)))
        .unwrap();
    let exported = Ifcx.export_ifccad(&document).unwrap();
    assert!(
        exported.losses.iter().any(|loss| {
            loss.handle == Some(table_handle)
                && loss.message.contains("row 1, column 1")
                && loss.message.contains("outer → inner")
                && loss.message.contains("MESH")
        }),
        "{:?}",
        exported.losses
    );
    let drawing = exported.drawing;
    let outer = drawing
        .blocks
        .iter()
        .find(|block| block.name == "outer")
        .unwrap();
    let inner = drawing
        .blocks
        .iter()
        .find(|block| block.name == "inner")
        .unwrap();
    assert_eq!(inner.entities.len(), 1);
    let tables = drawing
        .model
        .entities
        .iter()
        .filter_map(ifccad::IfccadEntity::as_native);
    for entity in tables {
        match &entity.kind {
            ifccad::IfccadEntityKind::BlockInstance { definition_id, .. } => {
                assert_eq!(*definition_id, outer.id)
            }
            ifccad::IfccadEntityKind::Table(table) => {
                let ifccad::IfccadTableCellContent::Block { block, .. } =
                    &table.grid.rows[0].cells[0].contents[0]
                else {
                    panic!("missing table block");
                };
                assert_eq!(block.definition_id, outer.id);
            }
            _ => {}
        }
    }
}

#[test]
fn table_formula_rebinds_to_a_later_table_in_both_directions() {
    use acadrust::entities::table::{CellValue, TableCellContentType};
    use acadrust::objects::{Field, ObjectType};
    let mut document = Document::new();
    let source = document
        .add_entity(EntityType::Table(Box::new(Table::new(Vector3::ZERO, 1, 1))))
        .unwrap();
    let target = document
        .add_entity(EntityType::Table(Box::new(Table::new(
            Vector3::new(20.0, 0.0, 0.0),
            1,
            1,
        ))))
        .unwrap();
    let field = document.allocate_handle();
    document.objects.insert(
        field,
        ObjectType::Field(Field {
            handle: field,
            owner: source,
            evaluator_id: "AcExpr".into(),
            code: "\\AcExpr (Table(%<\\_ObjIdx 0>%).A1 + 1)".into(),
            referenced_objects: vec![target],
            value: CellValue::number(13.0),
            value_string: "13.0".into(),
            ..Default::default()
        }),
    );
    let Some(EntityType::Table(table)) = document.get_entity_mut(source) else {
        panic!("missing table")
    };
    table.field_handles.push(field);
    let mut item = CellContent::text("13");
    item.field_handle = Some(field);
    item.content_type = TableCellContentType::Field;
    table.rows[0].cells[0].contents.push(item);

    let exported = Ifcx.export_ifccad(&document).unwrap();
    assert!(exported.losses.is_empty(), "{:?}", exported.losses);
    let bytes = ifccad::encode_ifccad_cbor_document(&exported.drawing).unwrap();
    let readback = ifccad::load_ifccad_cbor_bytes(bytes.bytes(), Default::default()).unwrap();
    let imported = Ifcx.import_ifccad(readback.document()).unwrap();
    assert!(imported.losses.is_empty(), "{:?}", imported.losses);
    let first = imported.entity_handles[&exported.entity_ids[&source]];
    let second = imported.entity_handles[&exported.entity_ids[&target]];
    let Some(EntityType::Table(table)) = imported.document.get_entity(first) else {
        panic!("missing table")
    };
    let Some(ObjectType::Field(field)) = imported.document.objects.get(&table.field_handles[0])
    else {
        panic!("missing restored field");
    };
    assert_eq!(field.owner, first);
    assert_eq!(field.referenced_objects, [second]);
    assert_eq!(imported.document.fields[&field.handle].objects, [second]);
    assert_eq!(field.code, "\\AcExpr (Table(%<\\_ObjIdx 0>%).A1 + 1)");
    assert_eq!(field.value.numeric_value, 13.0);
}

#[test]
fn table_entity_retains_ordered_cell_items_and_reuses_a_local_block() {
    let mut document = Document::new();
    let mut symbol = BlockRecord::new("symbol");
    symbol.handle = document.allocate_handle();
    symbol.block_entity_handle = document.allocate_handle();
    symbol.block_end_handle = document.allocate_handle();
    let symbol_handle = symbol.handle;
    document.block_records.add(symbol).unwrap();

    let mut table = Table::new(Vector3::new(10.0, 20.0, 0.0), 2, 2);
    table.rows[0].cells[0] = TableCell::text("Schedule");
    table.rows[1].cells[0] = TableCell::block(symbol_handle);
    table.rows[1].cells[0].contents.extend([
        CellContent::block(symbol_handle),
        CellContent::text("Two symbols"),
    ]);
    document
        .add_entity(EntityType::Table(Box::new(table)))
        .unwrap();

    let exported = Ifcx.export_ifccad(&document).unwrap();
    assert!(exported.losses.is_empty(), "{:?}", exported.losses);
    let bytes = ifccad::encode_ifccad_document(&exported.drawing).unwrap();
    let reloaded = ifccad::load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    let imported = Ifcx.import_ifccad(reloaded.document()).unwrap();
    assert!(imported.losses.is_empty(), "{:?}", imported.losses);
    let block = imported
        .document
        .block_records
        .get("symbol")
        .unwrap()
        .handle;
    let table = imported
        .document
        .model_space_entities()
        .find_map(|entity| match entity {
            EntityType::Table(table) => Some(table),
            _ => None,
        })
        .expect("roundtripped table");
    assert_eq!(table.insertion_point, Vector3::new(10.0, 20.0, 0.0));
    assert_eq!(table.rows[0].cells[0].text_value(), "Schedule");
    let items = &table.rows[1].cells[0].contents;
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].block_handle, Some(block));
    assert_eq!(items[1].block_handle, Some(block));
    assert_eq!(items[2].value.text, "Two symbols");
}

#[test]
#[ignore = "requires the external sample_AC1032 files"]
fn sample_ac1032_table_shapes_and_cell_item_counts_survive_production_exchange() {
    use opencaddocument::{DwgReader, DwgWriter, DxfReader, DxfWriter};
    let directory =
        std::path::PathBuf::from(std::env::var_os("ACADRUST_TABLE_SAMPLE_DIR").unwrap());
    let shape = |document: &Document| {
        let mut tables: Vec<_> = document
            .entities()
            .filter_map(|entity| match entity {
                EntityType::Table(table) => Some((
                    table.rows.len(),
                    table.columns.len(),
                    table
                        .canonical_merged_ranges()
                        .iter()
                        .map(|range| {
                            (
                                range.top_row,
                                range.left_col,
                                range.bottom_row,
                                range.right_col,
                            )
                        })
                        .collect::<Vec<_>>(),
                    table
                        .rows
                        .iter()
                        .flat_map(|row| &row.cells)
                        .map(|cell| cell.contents.len())
                        .collect::<Vec<_>>(),
                )),
                _ => None,
            })
            .collect();
        tables.sort();
        tables
    };
    let picture_sizes = |document: &Document| {
        let mut result: Vec<_> = document
            .entities()
            .filter_map(|entity| {
                let EntityType::Table(table) = entity else {
                    return None;
                };
                let name = table
                    .block_record_handle
                    .and_then(|handle| {
                        document
                            .block_records
                            .iter()
                            .find(|block| block.handle == handle)
                    })
                    .map(|block| block.name.as_str())
                    .unwrap_or(&table.block_name);
                Some((table.rows.len(), document.entities_in_block(name).count()))
            })
            .collect();
        result.sort();
        result
    };
    let named_styles = |document: &Document| {
        use acadrust::objects::ObjectType;
        let mut result: Vec<_> = document
            .entities()
            .filter_map(|entity| {
                let EntityType::Table(table) = entity else {
                    return None;
                };
                let style = table.table_style_handle.and_then(|handle| {
                    match document.objects.get(&handle) {
                        Some(ObjectType::TableStyle(style)) => Some(style.name.clone()),
                        _ => None,
                    }
                });
                Some((table.rows.len(), style))
            })
            .collect();
        result.sort();
        result
    };
    for (file, dwg, binary) in [
        ("sample_AC1032.dwg", true, false),
        ("sample_AC1032_ascii.dxf", false, false),
        ("sample_AC1032_binary.dxf", false, true),
    ] {
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let read = |bytes: Vec<u8>| -> Document {
            if dwg {
                DwgReader::from_stream(Cursor::new(bytes)).read().unwrap()
            } else {
                DxfReader::from_reader(Cursor::new(bytes))
                    .unwrap()
                    .read()
                    .unwrap()
            }
        };
        let document = read(bytes);
        let expected = shape(&document);
        assert_eq!(expected.len(), 2);
        let exported = Ifcx
            .export_ifccad(&document)
            .unwrap_or_else(|error| panic!("{file} export: {error}"));
        let expected_pictures = picture_sizes(&document);
        let mut native_pictures: Vec<_> = exported
            .drawing
            .model
            .entities
            .iter()
            .filter_map(ifccad::IfccadEntity::as_native)
            .filter_map(|entity| match &entity.kind {
                ifccad::IfccadEntityKind::Table(table) => Some((
                    table.grid.rows.len(),
                    table
                        .graphics
                        .as_ref()
                        .map_or(0, |graphics| graphics.parts.len()),
                )),
                _ => None,
            })
            .collect();
        native_pictures.sort();
        let table_losses: Vec<_> = exported
            .losses
            .iter()
            .filter(|loss| {
                loss.message.to_ascii_lowercase().contains("table")
                    && !loss.message.contains("MLeader")
            })
            .collect();
        assert_eq!(
            native_pictures, expected_pictures,
            "{file} native picture parts: {table_losses:?}"
        );
        let bytes = ifccad::encode_ifccad_cbor_document(&exported.drawing).unwrap();
        let ifcx = ifccad::encode_ifccad_document(&exported.drawing).unwrap();
        let json_readback = ifccad::load_ifccad_bytes(ifcx.bytes(), Default::default()).unwrap();
        let loaded = ifccad::load_ifccad_cbor_bytes(bytes.bytes(), Default::default()).unwrap();
        assert_same_table_content(json_readback.document(), loaded.document(), file);
        let mut imported = Ifcx.import_ifccad(loaded.document()).unwrap().document;
        assert_eq!(
            shape(&imported),
            expected,
            "{file} native roundtrip: {:?}",
            exported.losses
        );
        imported.version = acadrust::types::DxfVersion::AC1032;
        let cad = if dwg {
            DwgWriter::write_to_vec(&imported).unwrap()
        } else if binary {
            DxfWriter::new_binary(&imported).write_to_vec().unwrap()
        } else {
            DxfWriter::new(&imported).write_to_vec().unwrap()
        };
        let readback = read(cad.clone());
        assert_eq!(shape(&readback), expected, "{file} physical roundtrip");
        assert_eq!(
            picture_sizes(&readback),
            expected_pictures,
            "{file} physical picture parts"
        );
        assert_eq!(
            named_styles(&readback),
            named_styles(&document),
            "{file} style assignments"
        );
        if !dwg {
            let check_e2 = |document: &Document| {
                let block = document.block_records.get("my_block").unwrap().handle;
                let table = document
                    .entities()
                    .find_map(|entity| match entity {
                        EntityType::Table(table) if table.rows.len() == 4 => Some(table),
                        _ => None,
                    })
                    .unwrap();
                let items = &table.rows[1].cells[4].contents;
                assert_eq!(items[0].block_handle, Some(block));
                assert_eq!(items[1].block_handle, Some(block));
                assert_eq!(items[2].value.text, "Cell with 2 blocks");
            };
            check_e2(&readback);
        }
        if let Some(directory) = review_directory() {
            let tag = file.replace('.', "-");
            std::fs::write(directory.join(format!("{tag}.ifcx")), ifcx.bytes()).unwrap();
            std::fs::write(directory.join(format!("{tag}.ifccad.cbor")), bytes.bytes()).unwrap();
            std::fs::write(directory.join(format!("roundtrip-{file}")), cad).unwrap();
            let losses = exported
                .losses
                .iter()
                .map(|loss| loss.message.as_str())
                .collect::<Vec<_>>();
            std::fs::write(
                directory.join(format!("{tag}-diagnostics.txt")),
                losses.join("\n"),
            )
            .unwrap();
        }
    }
}

fn review_directory() -> Option<std::path::PathBuf> {
    std::env::var_os("IFCCAD_TABLE_REVIEW_DIR").map(|path| {
        let path = std::path::PathBuf::from(path);
        std::fs::create_dir_all(&path).unwrap();
        path
    })
}

fn assert_same_table_content(
    first: &ifccad::IfccadDocument,
    second: &ifccad::IfccadDocument,
    file: &str,
) {
    fn tables(
        drawing: &ifccad::IfccadDocument,
    ) -> std::collections::BTreeMap<u64, &ifccad::IfccadTable> {
        drawing
            .model
            .entities
            .iter()
            .chain(
                drawing
                    .paper_layouts
                    .iter()
                    .flat_map(|layout| &layout.entities),
            )
            .chain(drawing.blocks.iter().flat_map(|block| &block.entities))
            .filter_map(ifccad::IfccadEntity::as_native)
            .filter_map(|entity| match &entity.kind {
                ifccad::IfccadEntityKind::Table(table) => Some((entity.id, table.as_ref())),
                _ => None,
            })
            .collect()
    }
    let a = tables(first);
    let b = tables(second);
    assert!(a.keys().eq(b.keys()), "{file}: table identities changed");
    for (id, a) in a {
        let b = b[&id];
        assert!(
            a.placement == b.placement,
            "{file} table {id}: placement changed"
        );
        assert!(a.grid == b.grid, "{file} table {id}: grid changed");
        assert!(
            a.graphics == b.graphics,
            "{file} table {id}: graphics changed"
        );
    }
    assert!(
        first.table_styles == second.table_styles,
        "{file}: table styles changed"
    );
    assert!(
        first.table_data_links == second.table_data_links,
        "{file}: datalinks changed"
    );
    assert!(
        first.data_tables == second.data_tables,
        "{file}: data tables changed"
    );
}

#[test]
fn grouped_gallery_reopens_through_ifcx_cbor_and_all_three_ac1032_writers() {
    use acadrust::entities::table::{
        BreakOptionFlags, CellRange, CellStyle, CellStylePropertyFlags, CellValue,
        TableCellContentType,
    };
    use acadrust::entities::{BoundaryEdge, BoundaryPath, Circle, Hatch, Insert, PolylineEdge};
    use acadrust::objects::{
        ClassObject, ClassObjectData, DataLink, DataTable, DataTableColumn, DataTableValue, Field,
        ObjectType, TableStyle,
    };
    use acadrust::types::{DxfVersion, Vector2};
    use opencaddocument::{DwgReader, DwgWriter, DxfReader, DxfWriter};

    let mut document = Document::with_version(DxfVersion::AC1032);
    let styles = document.ensure_subdictionary("ACAD_TABLESTYLE");
    let mut style = TableStyle::new("Schedule");
    style.handle = document.allocate_handle();
    style.owner_handle = styles;
    style.set_all_text_heights(0.32);
    style.ensure_cell_styles(|_| acadrust::Handle::NULL);
    let note = style.create_cell_style("Note", Some("_DATA")).unwrap();
    let style_handle = style.handle;
    let review_table = |origin, rows, widths: &[f64]| {
        let mut table = Table::new(origin, rows, widths.len());
        table.table_style_handle = Some(style_handle);
        for (column, width) in table.columns.iter_mut().zip(widths) {
            column.width = *width;
        }
        for row in &mut table.rows {
            row.height = 1.2;
        }
        table
    };
    let mut template = review_table(Vector3::ZERO, 1, &[5.5, 4.0]);
    template.rows[0].cells[0] = TableCell::text("Start a schedule");
    template.common.handle = document.allocate_handle();
    template.common.owner_handle = style.handle;
    template.table_style_handle = Some(style.handle);
    style.modern_cell_style_handle = template.common.handle;
    document
        .objects
        .insert(template.common.handle, ObjectType::TableTemplate(template));
    if let Some(ObjectType::Dictionary(dictionary)) = document.objects.get_mut(&styles) {
        dictionary.insert(&style.name, style.handle);
    }
    document
        .objects
        .insert(style.handle, ObjectType::TableStyle(style));

    let link_owner = document.ensure_subdictionary("ACAD_DATALINK");
    let link = document.allocate_handle();
    if let Some(ObjectType::Dictionary(dictionary)) = document.objects.get_mut(&link_owner) {
        dictionary.insert("Stock list", link);
    }
    document.objects.insert(
        link,
        ObjectType::ClassObject(ClassObject {
            handle: link,
            owner: link_owner,
            reactors: vec![link_owner],
            data: ClassObjectData::DataLink(DataLink {
                data_adapter: "AcExcel".into(),
                connection_string: "stock.xlsx!Items!A1:B2".into(),
                description: "Stored stock link".into(),
                update_option: 2,
                hard_owner: link_owner,
                ..Default::default()
            }),
            ..Default::default()
        }),
    );
    let data_owner = document.ensure_subdictionary("GALLERY_DATA");
    let data_handle = document.allocate_handle();
    if let Some(ObjectType::Dictionary(dictionary)) = document.objects.get_mut(&data_owner) {
        dictionary.insert("Labels", data_handle);
    }
    document.objects.insert(
        data_handle,
        ObjectType::ClassObject(ClassObject {
            handle: data_handle,
            owner: data_owner,
            reactors: vec![data_owner],
            data: ClassObjectData::DataTable(DataTable {
                name: "Labels".into(),
                flags: 2,
                row_count: 2,
                columns: vec![DataTableColumn {
                    value_type: 3,
                    name: "Text".into(),
                    rows: ["First", "Second"]
                        .into_iter()
                        .map(|text| DataTableValue {
                            text: text.into(),
                            ..Default::default()
                        })
                        .collect(),
                }],
            }),
            ..Default::default()
        }),
    );
    for (dxf, cpp) in [
        ("DATALINK", "AcDbDataLink"),
        ("DATATABLE", "AcDbDataTable"),
        ("FIELD", "AcDbField"),
    ] {
        if !document.classes.contains(dxf) {
            document
                .classes
                .add_or_update(acadrust::classes::DxfClass::new(dxf, cpp));
        }
    }

    let mut block = BlockRecord::new("Schedule mark");
    block.handle = document.allocate_handle();
    block.block_entity_handle = document.allocate_handle();
    block.block_end_handle = document.allocate_handle();
    let symbol = block.handle;
    document.block_records.add(block).unwrap();
    let mut mark = Circle::new();
    mark.common.owner_handle = symbol;
    mark.radius = 0.18;
    document.add_entity(EntityType::Circle(mark)).unwrap();
    let mut owner_block = BlockRecord::new("Block-owned table");
    owner_block.handle = document.allocate_handle();
    owner_block.block_entity_handle = document.allocate_handle();
    owner_block.block_end_handle = document.allocate_handle();
    let owner = owner_block.handle;
    document.block_records.add(owner_block).unwrap();
    let mut block_table = review_table(Vector3::ZERO, 1, &[5.5]);
    block_table.common.owner_handle = owner;
    block_table.rows[0].cells[0] = TableCell::text("Table in block");
    document
        .add_entity(EntityType::Table(Box::new(block_table)))
        .unwrap();
    document
        .add_entity(EntityType::Insert(Insert::new(
            "Block-owned table",
            Vector3::new(0.0, -1.5, 0.0),
        )))
        .unwrap();

    let mut first = review_table(Vector3::new(0.0, 7.0, 0.0), 5, &[5.5, 4.0, 4.0]);
    first.name = "Schedule".into();
    first.table_style_handle = Some(style_handle);
    first.rows[0].cells[0] = TableCell::text("Schedule");
    first.merge_cells(CellRange::new(0, 0, 0, 2));
    first.rows[1].cells[0] = TableCell::block(symbol);
    first.rows[1].cells[0]
        .contents
        .extend([CellContent::block(symbol), CellContent::text("Symbols")]);
    first.rows[1].cells[0].style_id = note;
    first.rows[1].cells[0].style = Some(CellStyle {
        property_flags: CellStylePropertyFlags::TEXT_HEIGHT,
        text_height: 0.32,
        ..Default::default()
    });
    let numeric_cell = |value: f64| {
        let mut cell = TableCell::text(&value.to_string());
        cell.contents[0].value = CellValue::number(value);
        cell
    };
    first.rows[2].cells[0] = numeric_cell(9.0);
    first.rows[2].cells[1] = numeric_cell(12.0);
    first.rows[2].cells[2] = TableCell::text("Cached stock");
    for (cell, label) in
        first.rows[3]
            .cells
            .iter_mut()
            .zip(["Hatch area", "Other table + 1", "Stock total"])
    {
        *cell = TableCell::text(label);
    }
    first.rows[2].cells[0].has_linked_data = true;
    first.rows[2].cells[0].data_link_handle = Some(link);
    first.rows[2].cells[0].data_link_rows = 1;
    first.rows[2].cells[0].data_link_columns = 2;
    first.break_options = BreakOptionFlags::ENABLE_BREAKS | BreakOptionFlags::REPEAT_TOP_LABELS;
    first.break_spacing = 0.5;
    let first = document
        .add_entity(EntityType::Table(Box::new(first)))
        .unwrap();

    let mut later = review_table(Vector3::new(16.0, 7.0, 0.0), 2, &[5.5]);
    later.name = "Quantities".into();
    later.rows[0].cells[0] = TableCell::text("Quantities");
    later.rows[1].cells[0] = numeric_cell(12.0);
    let later = document
        .add_entity(EntityType::Table(Box::new(later)))
        .unwrap();

    let mut hatch = Hatch::new();
    let mut boundary = BoundaryPath::external();
    boundary.add_edge(BoundaryEdge::Polyline(PolylineEdge::new(
        vec![
            Vector2::new(16.0, 0.0),
            Vector2::new(18.0, 0.0),
            Vector2::new(18.0, 3.0),
            Vector2::new(16.0, 3.0),
        ],
        true,
    )));
    hatch.paths.push(boundary);
    let hatch = document
        .add_entity(EntityType::Hatch(Box::new(hatch)))
        .unwrap();

    let fields = [
        (
            1,
            "AcExpr",
            "\\AcExpr (Table(%<\\_ObjIdx 0>%).A2 + 1)",
            Some(later),
            13.0,
        ),
        (2, "AcExpr", "\\AcExpr (Sum(A3:B3))", None, 21.0),
        (
            0,
            "AcObjProp",
            "\\AcObjProp Object(%<\\_ObjIdx 0>%).Area",
            Some(hatch),
            6.0,
        ),
    ];
    for (index, evaluator, code, target, value) in fields {
        let handle = document.allocate_handle();
        document.objects.insert(
            handle,
            ObjectType::Field(Field {
                handle,
                owner: first,
                evaluator_id: evaluator.into(),
                code: code.into(),
                referenced_objects: target.into_iter().collect(),
                value: CellValue::number(value),
                value_string: value.to_string(),
                ..Default::default()
            }),
        );
        let Some(EntityType::Table(table)) = document.get_entity_mut(first) else {
            panic!("missing table")
        };
        let mut item = CellContent::text(&value.to_string());
        item.content_type = TableCellContentType::Field;
        item.field_handle = Some(handle);
        table.rows[4].cells[index].contents.push(item);
        table.field_handles.push(handle);
    }
    document.add_layout("Review sheet").unwrap();
    let mut paper = review_table(Vector3::new(12.0, 12.0, 0.0), 1, &[5.5]);
    paper.rows[0].cells[0] = TableCell::text("PaperSpace");
    document
        .add_entity_to_layout(EntityType::Table(Box::new(paper)), "Review sheet")
        .unwrap();

    let exported = Ifcx.export_ifccad(&document).unwrap();
    assert!(exported.losses.is_empty(), "{:?}", exported.losses);
    let ifcx = ifccad::encode_ifccad_document(&exported.drawing).unwrap();
    let cbor = ifccad::encode_ifccad_cbor_document(&exported.drawing).unwrap();
    let ifcx_read = ifccad::load_ifccad_bytes(ifcx.bytes(), Default::default()).unwrap();
    let cbor_read = ifccad::load_ifccad_cbor_bytes(cbor.bytes(), Default::default()).unwrap();
    assert_eq!(ifcx_read.document(), cbor_read.document());
    let mut imported = Ifcx.import_ifccad(cbor_read.document()).unwrap().document;
    imported.version = DxfVersion::AC1032;
    let outputs = [
        ("gallery.dwg", DwgWriter::write_to_vec(&imported).unwrap()),
        (
            "gallery-ascii.dxf",
            DxfWriter::new(&imported).write_to_vec().unwrap(),
        ),
        (
            "gallery-binary.dxf",
            DxfWriter::new_binary(&imported).write_to_vec().unwrap(),
        ),
    ];
    for (file, cad) in &outputs {
        let read = if file.ends_with(".dwg") {
            DwgReader::from_stream(Cursor::new(cad)).read().unwrap()
        } else {
            DxfReader::from_reader(Cursor::new(cad.clone()))
                .unwrap()
                .read()
                .unwrap()
        };
        let roundtrip = Ifcx.export_ifccad(&read).unwrap();
        let drawing = &roundtrip.drawing;
        assert_eq!(drawing.data_tables.len(), 1, "{file}");
        assert_eq!(drawing.table_data_links.len(), 1, "{file}");
        assert!(drawing.table_styles.iter().any(|style| style.settings.name == "Schedule" && style.settings.template.is_some()), "{file}");
        let tables: Vec<_> = drawing
            .model
            .entities
            .iter()
            .filter_map(ifccad::IfccadEntity::as_native)
            .filter_map(|entity| match &entity.kind {
                ifccad::IfccadEntityKind::Table(table) => Some(table),
                _ => None,
            })
            .collect();
        assert_eq!(tables.len(), 2, "{file}: {:?}", roundtrip.losses);
        assert_eq!(tables[0].grid.merged_ranges.len(), 1, "{file}");
        assert_eq!(tables[0].grid.rows[1].cells[0].contents.len(), 3, "{file}");
        assert_eq!(
            tables[0].grid.rows[4]
                .cells
                .iter()
                .map(|cell| cell.contents.len())
                .sum::<usize>(),
            3,
            "{file}"
        );
        assert_eq!(tables[0].grid.data_links.len(), 1, "{file}");
    }
    if let Some(directory) = review_directory() {
        std::fs::write(directory.join("gallery.ifcx"), ifcx.bytes()).unwrap();
        std::fs::write(directory.join("gallery.ifccad.cbor"), cbor.bytes()).unwrap();
        for (file, bytes) in outputs {
            std::fs::write(directory.join(file), bytes).unwrap();
        }
    }
}

#[test]
#[ignore = "requires the external LibreDWG DATATABLE sample"]
fn standalone_datatable_keeps_its_ordered_columns_and_dictionary_owner() {
    use acadrust::objects::{ClassObjectData, ObjectType};
    use opencaddocument::{DxfReader, DxfWriter};
    let directory =
        std::path::PathBuf::from(std::env::var_os("ACADRUST_TABLE_SAMPLE_DIR").unwrap());
    let source = directory.join("../libre/2010/gh209_1.dxf");
    let document = DxfReader::from_file(source).unwrap().read().unwrap();
    let table = |document: &Document| {
        document
            .objects
            .values()
            .find_map(|object| match object {
                ObjectType::ClassObject(object) => match &object.data {
                    ClassObjectData::DataTable(data) => Some(data.clone()),
                    _ => None,
                },
                _ => None,
            })
            .unwrap()
    };
    let expected = table(&document);
    let exported = Ifcx.export_ifccad(&document).unwrap();
    let data = &exported.drawing.data_tables[0].settings;
    assert_eq!(data.row_count, 15);
    assert_eq!(data.columns.len(), 4);
    assert_eq!(
        data.owner.as_ref().unwrap().key,
        "MAP_DISPLAY_CUSTOMOBJ_REG_TABLE"
    );
    let encoded = ifccad::encode_ifccad_document(&exported.drawing).unwrap();
    let reopened = ifccad::load_ifccad_bytes(encoded.bytes(), Default::default()).unwrap();
    let mut imported = Ifcx.import_ifccad(reopened.document()).unwrap().document;
    imported.version = acadrust::types::DxfVersion::AC1032;
    let bytes = DxfWriter::new(&imported).write_to_vec().unwrap();
    let reopened = DxfReader::from_reader(Cursor::new(bytes))
        .unwrap()
        .read()
        .unwrap();
    assert_eq!(table(&reopened), expected);
}
