//! Editable table grids. CAD item handles are rebound against the shared drawing.
use super::*;
use acadrust::entities::table::CellContentGeometry;
use acadrust::entities::table::{
    BreakFlowDirection, BreakOptionFlags, CellContent, CellRange, CellStateFlags, CellStyleType,
    CellType, CellValue, CellValueType, Table, TableBreakData, TableBreakRange, TableCell,
    TableCellContentType, ValueUnitType,
};
use ifccad::{
    IfccadTable, IfccadTableBlockContent, IfccadTableBreakDirection, IfccadTableBreakPart,
    IfccadTableBreakRange, IfccadTableBreaks, IfccadTableCell, IfccadTableCellContent,
    IfccadTableCellState, IfccadTableColumn, IfccadTableCustomValue, IfccadTableGrid,
    IfccadTableRange, IfccadTableRow, IfccadTableUnit, IfccadTableValue, IfccadTableValueKind,
};
use ifccad::{IfccadTableCellLayout, IfccadTableItemLayout};

pub(super) fn out(writer: &mut Writer<'_>, table: &Table) -> Result<Option<IfccadTable>, Error> {
    let result = grid_out(writer, table);
    let grid = match result {
        Ok(grid) => grid,
        Err(error) => {
            writer
                .losses
                .note(format!("ACAD_TABLE {:?}: {error}", table.common.handle));
            return Ok(None);
        }
    };
    let x_axis = unit_direction(table.horizontal_direction)?;
    let normal = unit_direction(table.normal)?;
    let y_axis = unit_direction(normal.cross(&x_axis))?;
    Ok(Some(IfccadTable {
        placement: IfccadPlacement {
            origin: array(table.insertion_point),
            x_axis: array(x_axis),
            y_axis: array(y_axis),
        },
        grid,
        graphics: dimension_graphics::capture_table(writer, table)?,
    }))
}

pub(super) fn finish_export(writer: &mut Writer<'_>) {
    let drawing = &mut writer.drawing;
    let ids = &writer.entity_ids;
    let mut dropped = Vec::new();
    for entities in drawing
        .blocks
        .iter_mut()
        .map(|block| &mut block.entities)
        .chain(
            drawing
                .paper_layouts
                .iter_mut()
                .map(|layout| &mut layout.entities),
        )
        .chain([&mut drawing.model.entities])
    {
        entities.retain_mut(|entity| {
            let Some(IfccadNativeEntity {
                id,
                kind: IfccadEntityKind::Table(table),
                ..
            }) = entity.as_native_mut()
            else {
                return true;
            };
            if let Err(error) = bind_grid(&mut table.grid, ids) {
                writer.losses.note(format!("table {id}: {error}"));
                dropped.push(*id);
                return false;
            }
            true
        });
    }
    for style in &mut drawing.table_styles {
        if let Some(template) = &mut style.settings.template {
            if let Err(error) = bind_grid(template, ids) {
                writer.losses.note(format!(
                    "table style {} template: {error}",
                    style.settings.name
                ));
                style.settings.template = None;
            }
        }
    }
    if !dropped.is_empty() {
        writer.entity_ids.retain(|_, id| !dropped.contains(id));
    }
}

fn bind_grid(grid: &mut IfccadTableGrid, ids: &HashMap<Handle, u64>) -> Result<(), Error> {
    for column in &mut grid.columns {
        for custom in &mut column.custom_data {
            bind_value(&mut custom.value, ids)?;
        }
    }
    for (row_index, row) in grid.rows.iter_mut().enumerate() {
        for custom in &mut row.custom_data {
            bind_value(&mut custom.value, ids)?;
        }
        for (column_index, cell) in row.cells.iter_mut().enumerate() {
            for custom in &mut cell.custom_data {
                bind_value(&mut custom.value, ids)?;
            }
            for (item_index, item) in cell.contents.iter_mut().enumerate() {
                let bind = match item {
                    IfccadTableCellContent::Value { value, .. } => bind_value(value, ids),
                    IfccadTableCellContent::Field { field, .. } => {
                        table_fields::bind_targets(field, ids)
                    }
                    IfccadTableCellContent::Block { block, .. } => block
                        .attributes
                        .iter_mut()
                        .try_for_each(|attribute| bind_id(&mut attribute.definition_id, ids)),
                };
                bind.map_err(|error| {
                    annotation::invalid(&format!(
                        "row {}, column {}, item {}: {error}",
                        row_index + 1,
                        column_index + 1,
                        item_index + 1
                    ))
                })?;
            }
        }
    }
    Ok(())
}

pub(super) fn bind_value(
    value: &mut IfccadTableValue,
    ids: &HashMap<Handle, u64>,
) -> Result<(), Error> {
    if let IfccadTableValue::Object(id) = value {
        bind_id(id, ids)?;
    }
    Ok(())
}

fn bind_id(id: &mut u64, ids: &HashMap<Handle, u64>) -> Result<(), Error> {
    *id = ids
        .get(&Handle::from(*id))
        .copied()
        .ok_or_else(|| annotation::invalid("unresolved native table object target"))?;
    Ok(())
}

pub(super) fn finish_import(
    drawing: &IfccadDocument,
    document: &mut Document,
    handles: &mut HashMap<u64, Handle>,
    losses: &mut Losses,
) -> Result<(), Error> {
    for entity in drawing
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
        .filter_map(IfccadEntity::as_native)
    {
        let IfccadEntityKind::Table(source) = &entity.kind else {
            continue;
        };
        let Some(handle) = handles.get(&entity.id).copied() else {
            continue;
        };
        let Some(EntityType::Table(table)) = document.get_entity(handle) else {
            continue;
        };
        let mut table = table.clone();
        match restore_grid(&source.grid, &mut table, document, handles) {
            Ok(()) => {
                if let Some(EntityType::Table(slot)) = document.get_entity_mut(handle) {
                    *slot = table;
                }
            }
            Err(error) => {
                losses.note(format!("table {}: {error}", entity.id));
                document.remove_entity(handle);
                handles.remove(&entity.id);
            }
        }
    }
    Ok(())
}

fn restore_grid(
    grid: &IfccadTableGrid,
    table: &mut Table,
    document: &mut Document,
    handles: &HashMap<u64, Handle>,
) -> Result<(), Error> {
    let resolve = |id: u64| {
        handles
            .get(&id)
            .copied()
            .ok_or_else(|| annotation::invalid("unresolved CAD table object target"))
    };
    let rebind = |value: &mut CellValue| -> Result<(), Error> {
        if let Some(id) = value.handle_value {
            value.handle_value = Some(resolve(id.value())?);
        }
        Ok(())
    };
    for column in &mut table.columns {
        for custom in &mut column.custom_data_items {
            rebind(&mut custom.value)?;
        }
    }
    let mut roots = Vec::new();
    for (row, source_row) in table.rows.iter_mut().zip(&grid.rows) {
        for custom in &mut row.custom_data_items {
            rebind(&mut custom.value)?;
        }
        for (cell, source_cell) in row.cells.iter_mut().zip(&source_row.cells) {
            for custom in &mut cell.custom_data_items {
                rebind(&mut custom.value)?;
            }
            for (item, source) in cell.contents.iter_mut().zip(&source_cell.contents) {
                match source {
                    IfccadTableCellContent::Value { .. } => rebind(&mut item.value)?,
                    IfccadTableCellContent::Block { block, .. } => {
                        item.attributes = block
                            .attributes
                            .iter()
                            .enumerate()
                            .map(|(index, attribute)| {
                                Ok(acadrust::entities::table::TableAttribute {
                                    definition_handle: resolve(attribute.definition_id)?,
                                    value: attribute.value.clone(),
                                    index: index as i32,
                                })
                            })
                            .collect::<Result<_, Error>>()?;
                    }
                    IfccadTableCellContent::Field { field, .. } => {
                        let handle =
                            table_fields::restore(document, field, table.common.handle, handles)?;
                        item.field_handle = Some(handle);
                        roots.push(handle);
                    }
                }
            }
        }
    }
    table.field_handles = roots;
    Ok(())
}

pub(super) fn grid_out(writer: &mut Writer<'_>, table: &Table) -> Result<IfccadTableGrid, Error> {
    let style_id = table
        .table_style_handle
        .and_then(|handle| writer.table_styles.get(&handle))
        .copied();
    let columns = table
        .columns
        .iter()
        .map(|column| {
            Ok(IfccadTableColumn {
                name: nonempty(&column.name),
                width: column.width,
                style_id: (style_id.is_some() && column.style_id != 0)
                    .then_some(column.style_id as u32),
                format: column
                    .style
                    .as_ref()
                    .map(|format| table_style::format_out(writer, format)),
                custom_value: (column.custom_data != 0).then_some(column.custom_data),
                custom_data: custom_out(&column.custom_data_items)?,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let mut rows = Vec::with_capacity(table.rows.len());
    let mut data_links = Vec::new();
    for (row_index, row) in table.rows.iter().enumerate() {
        let mut cells = Vec::with_capacity(row.cells.len());
        for (column_index, cell) in row.cells.iter().enumerate() {
            if cell.has_linked_data {
                let link_id = cell
                    .data_link_handle
                    .and_then(|handle| writer.table_data_links.get(&handle))
                    .copied()
                    .ok_or_else(|| annotation::invalid("missing table datalink resource"))?;
                let last = |start: usize, count: i32| {
                    usize::try_from(count)
                        .ok()
                        .and_then(|count| count.checked_sub(1))
                        .and_then(|offset| start.checked_add(offset))
                        .ok_or_else(|| annotation::invalid("invalid table datalink dimensions"))
                };
                data_links.push(ifccad::IfccadTableDataLinkRange {
                    link_id,
                    range: IfccadTableRange {
                        top_row: row_index,
                        left_column: column_index,
                        bottom_row: last(row_index, cell.data_link_rows)?,
                        right_column: last(column_index, cell.data_link_columns)?,
                    },
                });
            }
            let contents =
                cell.contents
                    .iter()
                    .enumerate()
                    .map(|(item_index, item)| {
                        item_out(writer, table.common.handle, (row_index, column_index), item)
                            .map_err(|error| {
                                annotation::invalid(&format!(
                                    "row {}, column {}, item {}: {error}",
                                    row_index + 1,
                                    column_index + 1,
                                    item_index + 1
                                ))
                            })
                    })
                    .collect::<Result<_, _>>()?;
            cells.push(IfccadTableCell {
                layout: layout_out(cell),
                contents,
                tooltip: nonempty(&cell.tooltip),
                state: CELL_STATES
                    .iter()
                    .filter(|(flag, _)| cell.state.contains(*flag))
                    .map(|(_, state)| *state)
                    .collect(),
                custom_value: (cell.custom_data != 0).then_some(cell.custom_data),
                custom_data: custom_out(&cell.custom_data_items)?,
                style_id: (style_id.is_some() && cell.style_id != 0)
                    .then_some(cell.style_id as u32),
                format: cell
                    .style
                    .as_ref()
                    .map(|format| table_style::format_out(writer, format)),
            });
        }
        rows.push(IfccadTableRow {
            height: row.height,
            cells,
            custom_value: (row.custom_data != 0).then_some(row.custom_data),
            custom_data: custom_out(&row.custom_data_items)?,
            style_id: (style_id.is_some() && row.style_id != 0).then_some(row.style_id as u32),
            format: row
                .style
                .as_ref()
                .map(|format| table_style::format_out(writer, format)),
        });
    }
    Ok(IfccadTableGrid {
        name: nonempty(&table.name),
        description: nonempty(&table.description),
        columns,
        rows,
        merged_ranges: table
            .canonical_merged_ranges()
            .iter()
            .map(range_out)
            .collect(),
        style_id,
        data_links,
        flow_direction: None,
        format: table
            .base_style
            .as_ref()
            .map(|format| table_style::format_out(writer, format)),
        breaks: breaks_out(table)?,
    })
}

fn item_out(
    writer: &mut Writer<'_>,
    owner: Handle,
    cell: (usize, usize),
    item: &CellContent,
) -> Result<IfccadTableCellContent, Error> {
    let format = table_style::item_out(writer, item);
    if item.content_type == TableCellContentType::Block {
        let record = item
            .block_handle
            .and_then(|handle| {
                writer
                    .document
                    .block_records
                    .iter()
                    .find(|record| record.handle == handle)
            })
            .ok_or_else(|| annotation::invalid("missing table cell block target"))?;
        let id = writer
            .block(&record.name)?
            .ok_or_else(|| annotation::invalid("ineligible table cell block target"))?;
        for message in skipped_block_members(writer, &record.name) {
            writer.losses.retained_note(Loss {
                handle: Some(owner),
                message: format!(
                    "table {owner} row {}, column {}: {message}",
                    cell.0 + 1,
                    cell.1 + 1
                ),
            });
        }
        return Ok(IfccadTableCellContent::Block {
            block: Box::new(IfccadTableBlockContent {
                definition_id: id,
                attributes: item
                    .attributes
                    .iter()
                    .map(|attribute| ifccad::IfccadTableBlockAttribute {
                        definition_id: attribute.definition_handle.value(),
                        value: attribute.value.clone(),
                    })
                    .collect(),
            }),
            format,
        });
    }
    if item.content_type == TableCellContentType::Field {
        let handle = item
            .field_handle
            .ok_or_else(|| annotation::invalid("missing table FIELD handle"))?;
        let field = table_fields::capture(writer.document, handle, owner)?;
        return Ok(IfccadTableCellContent::Field {
            field: Box::new(field),
            display: nonempty(item.value.display()),
            format,
        });
    }
    Ok(IfccadTableCellContent::Value {
        value: value_out(&item.value)?,
        display: nonempty(&item.value.formatted_value),
        unit: unit_out(item.value.unit_type),
        value_format: nonempty(&item.value.format),
        format,
    })
}

fn skipped_block_members(writer: &Writer<'_>, block: &str) -> Vec<String> {
    let mut losses = Vec::new();
    let mut pending = vec![(block.to_owned(), Vec::<String>::new())];
    while let Some((name, mut ancestors)) = pending.pop() {
        if ancestors
            .iter()
            .any(|ancestor| ancestor.eq_ignore_ascii_case(&name))
        {
            continue;
        }
        ancestors.push(name.clone());
        for entity in writer.document.entities_in_block(&name) {
            if !writer.entity_ids.contains_key(&entity.common().handle) {
                losses.push(format!(
                    "block {} skipped {} {}",
                    ancestors.join(" → "),
                    entity.as_entity().entity_type(),
                    entity.common().handle
                ));
            } else if let EntityType::Insert(insert) = entity {
                pending.push((insert.block_name.clone(), ancestors.clone()));
            }
        }
    }
    losses
}

pub(super) fn value_out(value: &CellValue) -> Result<IfccadTableValue, Error> {
    use IfccadTableValue as V;
    use IfccadTableValueKind as K;
    let unset = value.flags & 1 != 0;
    let (kind, data) = match value.value_type {
        CellValueType::String => (K::Text, V::Text(value.text.clone())),
        CellValueType::Long
            if unset
                || (value.numeric_value.fract() == 0.0
                    && value.numeric_value >= f64::from(i32::MIN)
                    && value.numeric_value <= f64::from(i32::MAX)) =>
        {
            (K::Integer, V::Integer(value.numeric_value as i64))
        }
        CellValueType::Double => (K::Real, V::Real(value.numeric_value)),
        CellValueType::Point2D => (
            K::Point2D,
            V::Point2D([value.point_value.x, value.point_value.y]),
        ),
        CellValueType::Point3D => (K::Point3D, V::Point3D(array(value.point_value))),
        CellValueType::Date => (K::Date, V::Date(value.binary_value.clone())),
        CellValueType::Buffer => (K::Binary, V::Binary(value.binary_value.clone())),
        CellValueType::ResultBuffer => {
            (K::ResultBuffer, V::ResultBuffer(value.binary_value.clone()))
        }
        CellValueType::General => (K::General, V::General(value.text.clone())),
        CellValueType::Handle => (
            K::Object,
            V::Object(value.handle_value.map_or(0, |handle| handle.value())),
        ),
        CellValueType::Unknown if value.raw_type_code == 0 && !value.text.is_empty() => {
            (K::Text, V::Text(value.text.clone()))
        }
        CellValueType::Unknown
            if value.raw_type_code == 0
                && (unset
                    || (value.numeric_value.fract() == 0.0
                        && value.numeric_value >= f64::from(i32::MIN)
                        && value.numeric_value <= f64::from(i32::MAX))) =>
        {
            (
                K::UnspecifiedNumber,
                V::UnspecifiedNumber(value.numeric_value as i32),
            )
        }
        other => {
            return Err(annotation::invalid(&format!(
                "unsupported table value type {other:?} ({})",
                value.raw_type_code
            )))
        }
    };
    Ok(if unset { V::Unset(kind) } else { data })
}

pub(super) fn input(
    source: &IfccadTable,
    document: &Document,
    names: &Names,
) -> Result<Table, Error> {
    let grid = &source.grid;
    let mut table = Table::new(
        vector(source.placement.origin),
        grid.rows.len(),
        grid.columns.len(),
    );
    table.horizontal_direction = vector(source.placement.x_axis);
    table.normal = table
        .horizontal_direction
        .cross(&vector(source.placement.y_axis));
    table.name = grid.name.clone().unwrap_or_default();
    table.description = grid.description.clone().unwrap_or_default();
    table.table_style_handle = grid
        .style_id
        .and_then(|id| names.table_styles.get(&id))
        .copied();
    let format = |source, scope| table_style::format_in(document, names, source, scope);
    table.base_style = grid
        .format
        .as_ref()
        .map(|source| format(source, CellStyleType::Table));
    for (column, value) in table.columns.iter_mut().zip(&grid.columns) {
        column.width = value.width;
        column.name = value.name.clone().unwrap_or_default();
        column.custom_data = value.custom_value.unwrap_or_default();
        column.custom_data_items = custom_in(&value.custom_data)?;
        column.style_id = value.style_id.unwrap_or_default() as i32;
        column.style = value
            .format
            .as_ref()
            .map(|source| format(source, CellStyleType::Column));
    }
    for (row, value) in table.rows.iter_mut().zip(&grid.rows) {
        row.height = value.height;
        row.custom_data = value.custom_value.unwrap_or_default();
        row.custom_data_items = custom_in(&value.custom_data)?;
        row.style_id = value.style_id.unwrap_or_default() as i32;
        row.style = value
            .format
            .as_ref()
            .map(|source| format(source, CellStyleType::Row));
        for (cell, data) in row.cells.iter_mut().zip(&value.cells) {
            *cell = TableCell::new();
            if let Some(layout) = &data.layout {
                layout_in(cell, layout);
            }
            cell.tooltip = data.tooltip.clone().unwrap_or_default();
            cell.custom_data = data.custom_value.unwrap_or_default();
            cell.custom_data_items = custom_in(&data.custom_data)?;
            cell.style_id = data.style_id.unwrap_or_default() as i32;
            cell.style = data
                .format
                .as_ref()
                .map(|source| format(source, CellStyleType::Cell));
            for (flag, state) in CELL_STATES {
                cell.state.set(flag, data.state.contains(&state));
            }
            for item in &data.contents {
                let (mut content, format) = match item {
                    IfccadTableCellContent::Block { block, format } => {
                        let handle = names
                            .blocks
                            .get(&block.definition_id)
                            .and_then(|name| document.block_records.get(name))
                            .ok_or_else(|| annotation::invalid("missing table cell block target"))?
                            .handle;
                        (CellContent::block(handle), format)
                    }
                    IfccadTableCellContent::Value {
                        value,
                        display,
                        unit,
                        value_format,
                        format,
                    } => {
                        let mut content = CellContent::new();
                        content.content_type = TableCellContentType::Value;
                        content.value = value_in(value)?;
                        content.value.formatted_value = display.clone().unwrap_or_default();
                        content.value.unit_type = unit_in(*unit);
                        content.value.format = value_format.clone().unwrap_or_default();
                        (content, format)
                    }
                    IfccadTableCellContent::Field {
                        display, format, ..
                    } => {
                        let mut content = CellContent::text(display.as_deref().unwrap_or_default());
                        content.content_type = TableCellContentType::Field;
                        (content, format)
                    }
                };
                if let Some(format) = format {
                    table_style::item_in(document, names, &mut content, format);
                }
                cell.contents.push(content);
            }
            if cell
                .contents
                .first()
                .is_some_and(|item| item.content_type == TableCellContentType::Block)
            {
                cell.cell_type = CellType::Block;
            }
        }
    }
    table.merged_ranges = grid
        .merged_ranges
        .iter()
        .map(|range| {
            CellRange::new(
                range.top_row,
                range.left_column,
                range.bottom_row,
                range.right_column,
            )
        })
        .collect();
    table.sync_merged_ranges_from_cells();
    for link in &grid.data_links {
        let target = &mut table.rows[link.range.top_row].cells[link.range.left_column];
        let dimension = |first, last| {
            i32::try_from(last - first + 1)
                .map_err(|_| annotation::invalid("table datalink dimension exceeds CAD range"))
        };
        target.has_linked_data = true;
        target.data_link_handle = names.table_data_links.get(&link.link_id).copied();
        target.data_link_rows = dimension(link.range.top_row, link.range.bottom_row)?;
        target.data_link_columns = dimension(link.range.left_column, link.range.right_column)?;
    }
    if let Some(breaks) = &grid.breaks {
        breaks_in(&mut table, breaks)?;
    }
    Ok(table)
}

fn layout_out(cell: &TableCell) -> Option<IfccadTableCellLayout> {
    let items = if cell.geometries.is_empty() {
        cell.contents
            .iter()
            .filter_map(|item| item.geometry.as_ref())
            .map(item_layout_out)
            .collect::<Vec<_>>()
    } else {
        cell.geometries.iter().map(item_layout_out).collect()
    };
    let anchor = cell.geometry.as_ref().map(item_layout_out);
    if items.is_empty()
        && anchor.is_none()
        && cell.geometry_scale == 1.0
        && cell.geometry_data_flag == 0
        && cell.geometry_flags == 0
        && cell.geometry_width_with_gap == 0.0
        && cell.geometry_height_with_gap == 0.0
    {
        return None;
    }
    Some(IfccadTableCellLayout {
        scale: cell.geometry_scale,
        width_with_gap: cell.geometry_width_with_gap,
        height_with_gap: cell.geometry_height_with_gap,
        data_flags: cell.geometry_data_flag,
        flags: cell.geometry_flags,
        anchor,
        items,
    })
}

fn item_layout_out(item: &CellContentGeometry) -> IfccadTableItemLayout {
    IfccadTableItemLayout {
        distance_to_top_left: array(item.distance_to_top_left),
        distance_to_center: array(item.distance_to_center),
        width: item.width,
        height: item.height,
        outer_width: item.outer_width,
        outer_height: item.outer_height,
        flags: item.flags,
    }
}

fn layout_in(cell: &mut TableCell, layout: &IfccadTableCellLayout) {
    let item_in = |item: &IfccadTableItemLayout| CellContentGeometry {
        distance_to_top_left: vector(item.distance_to_top_left),
        distance_to_center: vector(item.distance_to_center),
        width: item.width,
        height: item.height,
        outer_width: item.outer_width,
        outer_height: item.outer_height,
        flags: item.flags,
    };
    cell.geometry_scale = layout.scale;
    cell.geometry_width_with_gap = layout.width_with_gap;
    cell.geometry_height_with_gap = layout.height_with_gap;
    cell.geometry_data_flag = layout.data_flags;
    cell.geometry_flags = layout.flags;
    cell.geometry = layout.anchor.as_ref().map(item_in);
    cell.geometries = layout.items.iter().map(item_in).collect();
}

const CELL_STATES: [(CellStateFlags, IfccadTableCellState); 7] = [
    (
        CellStateFlags::CONTENT_LOCKED,
        IfccadTableCellState::ContentLocked,
    ),
    (
        CellStateFlags::CONTENT_READ_ONLY,
        IfccadTableCellState::ContentReadOnly,
    ),
    (CellStateFlags::LINKED, IfccadTableCellState::Linked),
    (
        CellStateFlags::CONTENT_MODIFIED_AFTER_UPDATE,
        IfccadTableCellState::ContentModifiedAfterUpdate,
    ),
    (
        CellStateFlags::FORMAT_LOCKED,
        IfccadTableCellState::FormatLocked,
    ),
    (
        CellStateFlags::FORMAT_READ_ONLY,
        IfccadTableCellState::FormatReadOnly,
    ),
    (
        CellStateFlags::FORMAT_MODIFIED_AFTER_UPDATE,
        IfccadTableCellState::FormatModifiedAfterUpdate,
    ),
];

pub(super) fn custom_out(
    items: &[acadrust::entities::table::TableCustomData],
) -> Result<Vec<IfccadTableCustomValue>, Error> {
    items
        .iter()
        .map(|item| {
            Ok(IfccadTableCustomValue {
                name: item.name.clone(),
                value: value_out(&item.value)?,
            })
        })
        .collect()
}

pub(super) fn custom_in(
    items: &[IfccadTableCustomValue],
) -> Result<Vec<acadrust::entities::table::TableCustomData>, Error> {
    items
        .iter()
        .map(|item| {
            Ok(acadrust::entities::table::TableCustomData {
                name: item.name.clone(),
                value: value_in(&item.value)?,
            })
        })
        .collect()
}

fn breaks_out(table: &Table) -> Result<Option<IfccadTableBreaks>, Error> {
    use BreakOptionFlags as F;
    let flags = table.break_options;
    if flags.is_empty()
        && table.break_spacing == 0.0
        && table.break_data.is_empty()
        && table.break_ranges.is_empty()
        && table.break_flow_direction == BreakFlowDirection::Right
    {
        return Ok(None);
    }
    let ranges = table
        .break_ranges
        .iter()
        .map(|range| {
            let row =
                |v| usize::try_from(v).map_err(|_| annotation::invalid("negative table break row"));
            Ok(IfccadTableBreakRange {
                position: array(range.position),
                start_row: row(range.start_row)?,
                end_row: row(range.end_row)?,
            })
        })
        .collect::<Result<_, Error>>()?;
    Ok(Some(IfccadTableBreaks {
        direction: match table.break_flow_direction {
            BreakFlowDirection::Right => IfccadTableBreakDirection::Right,
            BreakFlowDirection::Vertical => IfccadTableBreakDirection::Down,
            BreakFlowDirection::Left => IfccadTableBreakDirection::Left,
        },
        spacing: table.break_spacing,
        enabled: flags.contains(F::ENABLE_BREAKS),
        manual_positions: flags.contains(F::ALLOW_MANUAL_POSITIONS),
        manual_heights: flags.contains(F::ALLOW_MANUAL_HEIGHTS),
        repeat_top_labels: flags.contains(F::REPEAT_TOP_LABELS),
        repeat_bottom_labels: flags.contains(F::REPEAT_BOTTOM_LABELS),
        parts: table
            .break_data
            .iter()
            .map(|part| IfccadTableBreakPart {
                position: array(part.position),
                height: part.height,
                flags: part.flags,
            })
            .collect(),
        ranges,
    }))
}

fn breaks_in(table: &mut Table, breaks: &IfccadTableBreaks) -> Result<(), Error> {
    use BreakOptionFlags as F;
    table.break_flow_direction = match breaks.direction {
        IfccadTableBreakDirection::Right => BreakFlowDirection::Right,
        IfccadTableBreakDirection::Down => BreakFlowDirection::Vertical,
        IfccadTableBreakDirection::Left => BreakFlowDirection::Left,
    };
    table.break_spacing = breaks.spacing;
    for (flag, enabled) in [
        (F::ENABLE_BREAKS, breaks.enabled),
        (F::ALLOW_MANUAL_POSITIONS, breaks.manual_positions),
        (F::ALLOW_MANUAL_HEIGHTS, breaks.manual_heights),
        (F::REPEAT_TOP_LABELS, breaks.repeat_top_labels),
        (F::REPEAT_BOTTOM_LABELS, breaks.repeat_bottom_labels),
    ] {
        table.break_options.set(flag, enabled);
    }
    table.break_data = breaks
        .parts
        .iter()
        .map(|part| TableBreakData {
            position: vector(part.position),
            height: part.height,
            flags: part.flags,
        })
        .collect();
    table.break_ranges = breaks
        .ranges
        .iter()
        .map(|range| {
            let row = |v| {
                i32::try_from(v)
                    .map_err(|_| annotation::invalid("table break row exceeds CAD range"))
            };
            Ok(TableBreakRange {
                position: vector(range.position),
                start_row: row(range.start_row)?,
                end_row: row(range.end_row)?,
            })
        })
        .collect::<Result<_, Error>>()?;
    Ok(())
}

pub(super) fn value_in(value: &IfccadTableValue) -> Result<CellValue, Error> {
    use CellValueType as Cad;
    use IfccadTableValue as V;
    use IfccadTableValueKind as K;
    let mut data = CellValue::new();
    data.value_type = match value {
        V::Text(text) => {
            data.text = text.clone();
            Cad::String
        }
        V::Integer(integer) => {
            data.numeric_value = f64::from(
                i32::try_from(*integer)
                    .map_err(|_| annotation::invalid("table integer exceeds CAD range"))?,
            );
            Cad::Long
        }
        V::Real(number) => {
            data.numeric_value = *number;
            Cad::Double
        }
        V::Point2D([x, y]) => {
            data.point_value = Vector3::new(*x, *y, 0.0);
            data.data_size = 16;
            Cad::Point2D
        }
        V::Point3D(point) => {
            data.point_value = vector(*point);
            data.data_size = 24;
            Cad::Point3D
        }
        V::Date(date) => {
            data.binary_value = date.clone();
            data.data_size = i32::try_from(date.len())
                .map_err(|_| annotation::invalid("table date too large"))?;
            Cad::Date
        }
        V::Binary(bytes) => {
            data.binary_value = bytes.clone();
            Cad::Buffer
        }
        V::ResultBuffer(bytes) => {
            data.binary_value = bytes.clone();
            Cad::ResultBuffer
        }
        V::General(text) => {
            data.text = text.clone();
            Cad::General
        }
        V::Object(id) => {
            data.handle_value = Some(Handle::from(*id));
            Cad::Handle
        }
        V::UnspecifiedNumber(number) => {
            data.numeric_value = f64::from(*number);
            Cad::Unknown
        }
        V::Boolean(_) => {
            return Err(annotation::invalid(
                "table Boolean has no CAD cell value type",
            ))
        }
        V::Unset(kind) => {
            data.flags |= 1;
            match kind {
                K::Text => Cad::String,
                K::Integer => Cad::Long,
                K::Real => Cad::Double,
                K::Point2D => Cad::Point2D,
                K::Point3D => Cad::Point3D,
                K::Date => Cad::Date,
                K::Binary => Cad::Buffer,
                K::ResultBuffer => Cad::ResultBuffer,
                K::General => Cad::General,
                K::Object => Cad::Handle,
                K::UnspecifiedNumber => Cad::Unknown,
                K::Boolean => {
                    return Err(annotation::invalid(
                        "table Boolean has no CAD cell value type",
                    ))
                }
            }
        }
    };
    data.raw_type_code = data.value_type as i32;
    Ok(data)
}

fn nonempty(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.into())
}

fn range_out(range: &CellRange) -> IfccadTableRange {
    IfccadTableRange {
        top_row: range.top_row,
        left_column: range.left_col,
        bottom_row: range.bottom_row,
        right_column: range.right_col,
    }
}

pub(super) fn unit_out(unit: ValueUnitType) -> Option<IfccadTableUnit> {
    use IfccadTableUnit as U;
    match unit {
        ValueUnitType::NoUnits => None,
        ValueUnitType::Distance => Some(U::Distance),
        ValueUnitType::Angle => Some(U::Angle),
        ValueUnitType::Area => Some(U::Area),
        ValueUnitType::Volume => Some(U::Volume),
        ValueUnitType::Currency => Some(U::Currency),
        ValueUnitType::Percentage => Some(U::Percentage),
    }
}

pub(super) fn unit_in(unit: Option<IfccadTableUnit>) -> ValueUnitType {
    use IfccadTableUnit as U;
    match unit {
        None | Some(U::Unitless) => ValueUnitType::NoUnits,
        Some(U::Distance) => ValueUnitType::Distance,
        Some(U::Angle) => ValueUnitType::Angle,
        Some(U::Area) => ValueUnitType::Area,
        Some(U::Volume) => ValueUnitType::Volume,
        Some(U::Currency) => ValueUnitType::Currency,
        Some(U::Percentage) => ValueUnitType::Percentage,
    }
}
