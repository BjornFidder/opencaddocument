//! Sparse cell formats and their resource bindings.
use super::*;
use acadrust::entities::table::CellContent;
use acadrust::entities::table::{
    BorderPropertyFlags as B, BorderType, CellBorder, CellEdgeFlags as Edge, CellStyle,
    CellStylePropertyFlags as P, CellStyleType, CellValueType, ContentLayoutFlags, ValueUnitType,
};
use acadrust::objects::{
    NamedTableCellStyle, ObjectType, TableCellBorder, TableCellStyleData, TableContentFormat,
    TableFlowDirection, TableGridFormat, TableStyle,
};
use ifccad::{
    IfccadNamedTableCellStyle, IfccadTable, IfccadTableCellStyleRole as Role,
    IfccadTableFlowDirection, IfccadTableStyle, IfccadTableStyleSettings,
};
use ifccad::{
    IfccadTableAlignment as Alignment, IfccadTableBorder as Border,
    IfccadTableBorderKind as BorderKind, IfccadTableCellFormat as CellFormat,
    IfccadTableContentLayout as ContentLayout, IfccadTableMargins as Margins,
    IfccadTableValueKind as ValueKind,
};

pub(super) fn export_styles(writer: &mut Writer<'_>) -> Result<(), Error> {
    let mut styles: Vec<_> = writer
        .document
        .objects
        .iter()
        .filter_map(|(handle, object)| match object {
            ObjectType::TableStyle(style) => Some((*handle, style.clone())),
            _ => None,
        })
        .collect();
    styles.sort_by_key(|(handle, _)| *handle);
    for (handle, _) in &styles {
        writer.table_styles.insert(
            *handle,
            writer.drawing.id_counters.allocate_table_style_id()?,
        );
    }
    for (handle, mut style) in styles {
        style.ensure_cell_styles(|row| {
            row.text_style_handle
                .or_else(|| {
                    writer
                        .document
                        .text_styles
                        .get(&row.text_style_name)
                        .map(|style| style.handle)
                })
                .unwrap_or_default()
        });
        let template = if let Some(template) = style.template() {
            if let Some(ObjectType::TableTemplate(source)) = writer.document.objects.get(&template)
            {
                Some(table::grid_out(writer, source)?)
            } else {
                writer.losses.note(format!(
                    "TABLESTYLE {}: missing template target",
                    style.name
                ));
                None
            }
        } else {
            None
        };
        let settings = IfccadTableStyleSettings {
            name: style.name.clone(),
            description: (!style.description.is_empty()).then_some(style.description.clone()),
            flow_direction: match style.flow_direction {
                TableFlowDirection::Down => IfccadTableFlowDirection::TopToBottom,
                TableFlowDirection::Up => IfccadTableFlowDirection::BottomToTop,
            },
            margins: Some(Margins {
                left: Some(style.horizontal_margin),
                right: Some(style.horizontal_margin),
                top: Some(style.vertical_margin),
                bottom: Some(style.vertical_margin),
                ..Default::default()
            }),
            title_suppressed: Some(!style.has_title_row()),
            header_suppressed: Some(!style.has_header_row()),
            cell_styles: style
                .cell_styles()
                .map(|style| named_out(writer, style))
                .collect(),
            base_style: style
                .modern_style
                .as_ref()
                .map(|style| named_out(writer, style)),
            annotative: style.annotative.then_some(true),
            template,
        };
        writer.drawing.table_styles.push(IfccadTableStyle {
            id: writer.table_styles[&handle],
            settings,
        });
    }
    Ok(())
}

pub(super) fn import_styles(
    drawing: &IfccadDocument,
    document: &mut Document,
    names: &mut Names,
) -> Result<(), Error> {
    if drawing.table_styles.is_empty() {
        return Ok(());
    }
    let dictionary = document.ensure_subdictionary("ACAD_TABLESTYLE");
    for style in &drawing.table_styles {
        let existing = match document.objects.get(&dictionary) {
            Some(ObjectType::Dictionary(dictionary)) => dictionary.get(&style.settings.name),
            _ => None,
        };
        let handle = existing.unwrap_or_else(|| document.allocate_handle());
        names.table_styles.insert(style.id, handle);
    }
    for source in &drawing.table_styles {
        let settings = &source.settings;
        let mut style = TableStyle::new(&settings.name);
        style.handle = names.table_styles[&source.id];
        style.owner_handle = dictionary;
        style.description = settings.description.clone().unwrap_or_default();
        style.flow_direction = match settings.flow_direction {
            IfccadTableFlowDirection::TopToBottom => TableFlowDirection::Down,
            IfccadTableFlowDirection::BottomToTop => TableFlowDirection::Up,
        };
        style.annotative = settings.annotative.unwrap_or_default();
        style.set_title_visible(!settings.title_suppressed.unwrap_or_default());
        style.set_header_visible(!settings.header_suppressed.unwrap_or_default());
        style.modern_overrides = settings
            .cell_styles
            .iter()
            .map(|style| {
                let named = named_in(document, names, style);
                (named.id, named)
            })
            .collect();
        style.modern_style = settings
            .base_style
            .as_ref()
            .map(|style| named_in(document, names, style));
        style.sync_rows_from_cell_styles();
        if let Some(margins) = &settings.margins {
            style.horizontal_margin = margins
                .left
                .or(margins.right)
                .unwrap_or(style.horizontal_margin);
            style.vertical_margin = margins
                .top
                .or(margins.bottom)
                .unwrap_or(style.vertical_margin);
        }
        if let Some(template) = &settings.template {
            let template = IfccadTable {
                placement: IfccadPlacement {
                    origin: [0.0; 3],
                    x_axis: [1.0, 0.0, 0.0],
                    y_axis: [0.0, 1.0, 0.0],
                },
                grid: template.clone(),
                graphics: None,
            };
            let mut template = table::input(&template, document, names)?;
            template.common.handle = document.allocate_handle();
            template.common.owner_handle = style.handle;
            template.table_style_handle = Some(style.handle);
            style.modern_cell_style_handle = template.common.handle;
            document
                .objects
                .insert(template.common.handle, ObjectType::TableTemplate(template));
        }
        let handle = style.handle;
        if let Some(ObjectType::Dictionary(entries)) = document.objects.get_mut(&dictionary) {
            entries.insert(settings.name.clone(), handle);
        }
        document
            .objects
            .insert(handle, ObjectType::TableStyle(style));
    }
    Ok(())
}

const GRID_EDGES: [(i32, Edge); 6] = [
    (1, Edge::TOP),
    (2, Edge::INSIDE_HORIZONTAL),
    (4, Edge::BOTTOM),
    (8, Edge::LEFT),
    (16, Edge::INSIDE_VERTICAL),
    (32, Edge::RIGHT),
];

fn named_out(writer: &Writer<'_>, source: &NamedTableCellStyle) -> IfccadNamedTableCellStyle {
    let value = &source.cell_style;
    let content = &value.content_format;
    let properties = P::DATA_TYPE
        | P::DATA_FORMAT
        | P::ROTATION
        | P::BLOCK_SCALE
        | P::ALIGNMENT
        | P::CONTENT_COLOR
        | P::TEXT_STYLE
        | P::TEXT_HEIGHT
        | P::BACKGROUND_COLOR
        | P::CONTENT_LAYOUT
        | P::MERGE_ALL;
    let mut style = CellStyle {
        property_flags: properties,
        content_property_flags: content.property_flags | value.merge_flags,
        text_style_handle: (!content.text_style.is_null()).then_some(content.text_style),
        text_height: content.text_height,
        rotation: content.rotation,
        scale: content.block_scale,
        alignment: content.cell_alignment,
        content_color: content.content_color,
        background_color: value.background_color,
        fill_enabled: value.background_color != Color::None,
        layout_flags: ContentLayoutFlags::from_bits_truncate(value.content_layout as u32),
        margin_override_flags: value.margin_override_flags,
        margin_left: value.horizontal_margin,
        margin_right: value.right_margin,
        margin_top: value.vertical_margin,
        margin_bottom: value.bottom_margin,
        horizontal_spacing: value.horizontal_spacing,
        vertical_spacing: value.vertical_spacing,
        value_data_type: content.value_data_type,
        value_unit_type: content.value_unit_type,
        value_format: content.value_format_string.clone(),
        ..Default::default()
    };
    for grid in &value.borders {
        for (mask, edge) in GRID_EDGES {
            if grid.index_mask & mask == 0 {
                continue;
            }
            let border = CellBorder {
                border_type: BorderType::from(grid.border.border_type as i16),
                color: grid.border.color,
                line_weight: grid.border.line_weight,
                line_type_handle: (!grid.line_type.is_null()).then_some(grid.line_type),
                invisible: !grid.border.is_invisible,
                double_spacing: grid.border.double_line_spacing,
                override_flags: B::ALL,
            };
            style.applied_border_edges.insert(edge);
            match edge {
                Edge::TOP => style.top_border = border,
                Edge::RIGHT => style.right_border = border,
                Edge::BOTTOM => style.bottom_border = border,
                Edge::LEFT => style.left_border = border,
                _ => style.additional_borders.push((edge.bits(), border)),
            }
        }
    }
    let role = match source.name.to_ascii_uppercase().as_str() {
        "_TITLE" => Some(Role::Title),
        "_HEADER" => Some(Role::Header),
        "_DATA" => Some(Role::Data),
        _ => None,
    };
    IfccadNamedTableCellStyle {
        id: source.id as u32,
        name: source.name.clone(),
        role,
        format: format_out(writer, &style),
    }
}

fn named_in(
    document: &Document,
    names: &Names,
    source: &IfccadNamedTableCellStyle,
) -> NamedTableCellStyle {
    let style = format_in(document, names, &source.format, CellStyleType::Table);
    let mut cell = TableCellStyleData {
        style_type: 5,
        data_flags: 1,
        content_layout: style.layout_flags.bits() as i32,
        background_color: if style.fill_enabled {
            style.background_color
        } else {
            Color::None
        },
        merge_flags: style.content_property_flags & P::MERGE_ALL.bits() as i32,
        margin_override_flags: style.margin_flags_to_write(),
        vertical_margin: style.margin_top,
        horizontal_margin: style.margin_left,
        bottom_margin: style.margin_bottom,
        right_margin: style.margin_right,
        horizontal_spacing: style.horizontal_spacing,
        vertical_spacing: style.vertical_spacing,
        content_format: TableContentFormat {
            property_flags: style.content_property_flags,
            property_override_flags: style.property_flags.bits() as i32,
            value_data_type: style.value_data_type,
            value_unit_type: style.value_unit_type,
            value_format_string: style.value_format,
            rotation: style.rotation,
            block_scale: style.scale,
            cell_alignment: style.alignment,
            content_color: style.content_color,
            text_style: style.text_style_handle.unwrap_or_default(),
            text_height: style.text_height,
        },
        ..Default::default()
    };
    for (mask, edge) in GRID_EDGES {
        let border = match edge {
            Edge::TOP if style.applied_border_edges.contains(edge) => Some(&style.top_border),
            Edge::RIGHT if style.applied_border_edges.contains(edge) => Some(&style.right_border),
            Edge::BOTTOM if style.applied_border_edges.contains(edge) => Some(&style.bottom_border),
            Edge::LEFT if style.applied_border_edges.contains(edge) => Some(&style.left_border),
            _ => style
                .additional_borders
                .iter()
                .find(|(mask, _)| *mask == edge.bits())
                .map(|(_, border)| border),
        };
        if let Some(border) = border {
            cell.borders.push(TableGridFormat {
                index_mask: mask,
                line_type: border.line_type_handle.unwrap_or_default(),
                border: TableCellBorder {
                    border_type: (border.border_type as i16).into(),
                    line_weight: border.line_weight,
                    color: border.color,
                    is_invisible: !border.invisible,
                    double_line_spacing: border.double_spacing,
                    ..Default::default()
                },
            });
        }
    }
    let (name, style_type) = match source.role {
        Some(Role::Title) => ("_TITLE".into(), 1),
        Some(Role::Header) => ("_HEADER".into(), 1),
        Some(Role::Data) => ("_DATA".into(), 2),
        None => (source.name.clone(), 2),
    };
    NamedTableCellStyle {
        cell_style: cell,
        name,
        id: source.id as i32,
        style_type,
    }
}

pub(super) fn format_out(writer: &Writer<'_>, source: &CellStyle) -> CellFormat {
    let sets = |property| source.sets(property);
    let mut margins = Margins::default();
    margins.left =
        (sets(P::MARGIN_LEFT) || source.margin_override_flags != 0).then_some(source.margin_left);
    margins.top =
        (sets(P::MARGIN_TOP) || source.margin_override_flags != 0).then_some(source.margin_top);
    margins.right =
        (sets(P::MARGIN_RIGHT) || source.margin_override_flags != 0).then_some(source.margin_right);
    margins.bottom = (sets(P::MARGIN_BOTTOM) || source.margin_override_flags != 0)
        .then_some(source.margin_bottom);
    margins.horizontal_spacing = (sets(P::MARGIN_HORIZONTAL_SPACING)
        || source.margin_override_flags != 0)
        .then_some(source.horizontal_spacing);
    margins.vertical_spacing = (sets(P::MARGIN_VERTICAL_SPACING)
        || source.margin_override_flags != 0)
        .then_some(source.vertical_spacing);
    let edges = source.applied_border_edges;
    let border = |flag: Edge, value: &CellBorder| {
        (edges.contains(flag) || !value.override_flags.is_empty())
            .then(|| border_out(writer, value))
    };
    let additional = |flag: Edge| {
        source
            .additional_borders
            .iter()
            .find(|(mask, _)| mask & flag.bits() != 0)
            .map(|(_, value)| border_out(writer, value))
    };
    CellFormat {
        text_height: sets(P::TEXT_HEIGHT).then_some(source.text_height),
        rotation: sets(P::ROTATION).then_some(source.rotation),
        block_scale: sets(P::BLOCK_SCALE).then_some(source.scale),
        text_style_id: sets(P::TEXT_STYLE)
            .then(|| {
                source
                    .text_style_handle
                    .and_then(|handle| writer.style_handles.get(&handle))
                    .or_else(|| {
                        writer
                            .styles
                            .get(&source.text_style_name.to_ascii_lowercase())
                    })
                    .map(|style| style.0)
            })
            .flatten(),
        content_color: sets(P::CONTENT_COLOR)
            .then(|| color_mode_out(source.content_color))
            .flatten(),
        fill_color: sets(P::BACKGROUND_COLOR)
            .then(|| color_mode_out(source.background_color))
            .flatten(),
        fill_enabled: sets(P::BACKGROUND_COLOR).then_some(source.fill_enabled),
        alignment: sets(P::ALIGNMENT)
            .then(|| alignment_out(source.alignment))
            .flatten(),
        auto_fit: sets(P::AUTO_SCALE)
            .then_some(source.content_property_flags as u32 & P::AUTO_SCALE.bits() != 0),
        value_format: sets(P::DATA_FORMAT).then(|| source.value_format.clone()),
        value_type: sets(P::DATA_TYPE)
            .then(|| value_kind_out(source.value_data_type))
            .flatten(),
        value_unit: sets(P::DATA_TYPE).then(|| {
            table::unit_out(ValueUnitType::from(source.value_unit_type as u32))
                .unwrap_or(ifccad::IfccadTableUnit::Unitless)
        }),
        margins: (margins != Margins::default()).then_some(margins),
        content_layout: sets(P::CONTENT_LAYOUT)
            .then(|| layout_out(source.layout_flags))
            .flatten(),
        top_border: border(Edge::TOP, &source.top_border),
        right_border: border(Edge::RIGHT, &source.right_border),
        bottom_border: border(Edge::BOTTOM, &source.bottom_border),
        left_border: border(Edge::LEFT, &source.left_border),
        horizontal_inner_border: additional(Edge::INSIDE_HORIZONTAL),
        vertical_inner_border: additional(Edge::INSIDE_VERTICAL),
        merge_on_insert: sets(P::MERGE_ALL)
            .then_some(source.content_property_flags as u32 & P::MERGE_ALL.bits() != 0),
    }
}

pub(super) fn item_out(writer: &Writer<'_>, item: &CellContent) -> Option<Box<CellFormat>> {
    let mut flags =
        P::from_bits_truncate((item.format_property_flags | item.format_override_flags) as u32);
    flags.set(
        P::ROTATION,
        item.rotation != 0.0 || flags.contains(P::ROTATION),
    );
    flags.set(
        P::BLOCK_SCALE,
        item.scale != 1.0 || flags.contains(P::BLOCK_SCALE),
    );
    flags.set(
        P::DATA_FORMAT,
        !item.value_format.is_empty() || flags.contains(P::DATA_FORMAT),
    );
    if flags.is_empty() {
        return None;
    }
    let style = CellStyle {
        property_flags: flags,
        content_property_flags: item.format_property_flags,
        value_data_type: item.format_value_data_type,
        value_unit_type: item.format_value_unit_type,
        value_format: item.value_format.clone(),
        content_color: item.color,
        text_style_handle: item.text_style_handle,
        text_style_name: item.text_style_name.clone(),
        text_height: item.text_height,
        rotation: item.rotation,
        scale: item.scale,
        alignment: item.alignment,
        ..Default::default()
    };
    Some(Box::new(format_out(writer, &style)))
}

pub(super) fn item_in(
    document: &Document,
    names: &Names,
    item: &mut CellContent,
    format: &CellFormat,
) {
    let style = format_in(document, names, format, CellStyleType::Cell);
    item.format_override_flags = style.property_flags.bits() as i32;
    item.format_property_flags = style.content_property_flags;
    item.format_value_data_type = style.value_data_type;
    item.format_value_unit_type = style.value_unit_type;
    item.value_format = style.value_format;
    item.color = style.content_color;
    item.text_style_handle = style.text_style_handle;
    item.text_style_name = style.text_style_name;
    item.text_height = style.text_height;
    item.rotation = style.rotation;
    item.scale = style.scale;
    item.alignment = style.alignment;
}

pub(super) fn format_in(
    document: &Document,
    names: &Names,
    source: &CellFormat,
    style_type: CellStyleType,
) -> CellStyle {
    let mut style = CellStyle {
        style_type,
        ..Default::default()
    };
    let flags = &mut style.property_flags;
    if let Some(value) = source.text_height {
        flags.insert(P::TEXT_HEIGHT);
        style.text_height = value;
    }
    if let Some(value) = source.rotation {
        flags.insert(P::ROTATION);
        style.rotation = value;
    }
    if let Some(value) = source.block_scale {
        flags.insert(P::BLOCK_SCALE);
        style.scale = value;
    }
    if let Some(id) = source.text_style_id {
        flags.insert(P::TEXT_STYLE);
        let id = IfccadTextStyleId(id);
        style.text_style_handle = names.style_handles.get(&id).copied();
        style.text_style_name = names.styles.get(&id).cloned().unwrap_or_default();
    }
    if let Some(color) = &source.content_color {
        flags.insert(P::CONTENT_COLOR);
        style.content_color = color_mode_in(color);
    }
    if source.fill_enabled.is_some() || source.fill_color.is_some() {
        flags.insert(P::BACKGROUND_COLOR);
        style.fill_enabled = source.fill_enabled.unwrap_or(true);
        style.background_color = source
            .fill_color
            .as_ref()
            .map_or(Color::None, color_mode_in);
    }
    if let Some(value) = source.alignment {
        flags.insert(P::ALIGNMENT);
        style.alignment = alignment_in(value);
    }
    if let Some(value) = source.auto_fit {
        flags.insert(P::AUTO_SCALE);
        if value {
            style.content_property_flags |= P::AUTO_SCALE.bits() as i32;
        }
    }
    if let Some(value) = &source.value_format {
        flags.insert(P::DATA_FORMAT);
        style.value_format = value.clone();
    }
    if source.value_type.is_some() || source.value_unit.is_some() {
        flags.insert(P::DATA_TYPE);
        style.value_data_type = source.value_type.map(value_kind_in).unwrap_or_default();
        style.value_unit_type = table::unit_in(source.value_unit) as i32;
    }
    if let Some(margins) = &source.margins {
        if let Some(value) = margins.left {
            flags.insert(P::MARGIN_LEFT);
            style.margin_left = value;
        }
        if let Some(value) = margins.top {
            flags.insert(P::MARGIN_TOP);
            style.margin_top = value;
        }
        if let Some(value) = margins.right {
            flags.insert(P::MARGIN_RIGHT);
            style.margin_right = value;
        }
        if let Some(value) = margins.bottom {
            flags.insert(P::MARGIN_BOTTOM);
            style.margin_bottom = value;
        }
        if let Some(value) = margins.horizontal_spacing {
            flags.insert(P::MARGIN_HORIZONTAL_SPACING);
            style.horizontal_spacing = value;
        }
        if let Some(value) = margins.vertical_spacing {
            flags.insert(P::MARGIN_VERTICAL_SPACING);
            style.vertical_spacing = value;
        }
    }
    if let Some(value) = source.content_layout {
        flags.insert(P::CONTENT_LAYOUT);
        style.layout_flags = match value {
            ContentLayout::Flow => ContentLayoutFlags::FLOW,
            ContentLayout::Horizontal => ContentLayoutFlags::STACKED_HORIZONTAL,
            ContentLayout::Vertical => ContentLayoutFlags::STACKED_VERTICAL,
        };
    }
    if let Some(value) = source.merge_on_insert {
        flags.insert(P::MERGE_ALL);
        if value {
            style.content_property_flags |= P::MERGE_ALL.bits() as i32;
        }
    }
    for (edge, destination, value) in [
        (Edge::TOP, &mut style.top_border, &source.top_border),
        (Edge::RIGHT, &mut style.right_border, &source.right_border),
        (
            Edge::BOTTOM,
            &mut style.bottom_border,
            &source.bottom_border,
        ),
        (Edge::LEFT, &mut style.left_border, &source.left_border),
    ] {
        if let Some(value) = value {
            style.applied_border_edges.insert(edge);
            *destination = border_in(document, names, value);
        }
    }
    for (edge, value) in [
        (Edge::INSIDE_HORIZONTAL, &source.horizontal_inner_border),
        (Edge::INSIDE_VERTICAL, &source.vertical_inner_border),
    ] {
        if let Some(value) = value {
            style
                .additional_borders
                .push((edge.bits(), border_in(document, names, value)));
        }
    }
    style
}

fn border_out(writer: &Writer<'_>, value: &CellBorder) -> Border {
    let flags = value.override_flags;
    Border {
        visible: flags.contains(B::INVISIBILITY).then_some(!value.invisible),
        color: flags
            .contains(B::COLOR)
            .then(|| color_mode_out(value.color))
            .flatten(),
        line_weight: flags
            .contains(B::LINE_WEIGHT)
            .then(|| match value.line_weight {
                LineWeight::ByLayer => Some(IfccadMode::ByLayer),
                LineWeight::ByBlock => Some(IfccadMode::ByBlock),
                weight => weight_out(weight).map(IfccadMode::Explicit),
            })
            .flatten(),
        line_pattern: flags
            .contains(B::LINE_TYPE)
            .then(|| {
                let name = value
                    .line_type_handle
                    .and_then(|handle| {
                        writer
                            .document
                            .line_types
                            .iter()
                            .find(|line| line.handle == handle)
                    })
                    .map(|line| line.name.to_ascii_lowercase());
                match name.as_deref() {
                    None | Some("bylayer") => Some(IfccadMode::ByLayer),
                    Some("byblock") => Some(IfccadMode::ByBlock),
                    Some(name) => writer
                        .line_patterns
                        .get(name)
                        .copied()
                        .map(IfccadMode::Explicit),
                }
            })
            .flatten(),
        line_kind: flags
            .contains(B::BORDER_TYPE)
            .then_some(match value.border_type {
                BorderType::Single => BorderKind::Single,
                BorderType::Double => BorderKind::Double,
            }),
        double_spacing: flags
            .contains(B::DOUBLE_LINE_SPACING)
            .then_some(value.double_spacing),
    }
}

fn border_in(document: &Document, names: &Names, value: &Border) -> CellBorder {
    let mut border = CellBorder::default();
    if let Some(visible) = value.visible {
        border.override_flags.insert(B::INVISIBILITY);
        border.invisible = !visible;
    }
    if let Some(color) = &value.color {
        border.override_flags.insert(B::COLOR);
        border.color = color_mode_in(color);
    }
    if let Some(weight) = &value.line_weight {
        border.override_flags.insert(B::LINE_WEIGHT);
        border.line_weight = match weight {
            IfccadMode::ByLayer => LineWeight::ByLayer,
            IfccadMode::ByBlock => LineWeight::ByBlock,
            IfccadMode::Explicit(weight) => weight_in(*weight),
        };
    }
    if let Some(pattern) = &value.line_pattern {
        border.override_flags.insert(B::LINE_TYPE);
        let name = match pattern {
            IfccadMode::ByLayer => Some("ByLayer"),
            IfccadMode::ByBlock => Some("ByBlock"),
            IfccadMode::Explicit(id) => names.line_patterns.get(id).map(String::as_str),
        };
        border.line_type_handle = name
            .and_then(|name| document.line_types.get(name))
            .map(|line| line.handle);
    }
    if let Some(kind) = value.line_kind {
        border.override_flags.insert(B::BORDER_TYPE);
        border.border_type = match kind {
            BorderKind::Single => BorderType::Single,
            BorderKind::Double => BorderType::Double,
        };
    }
    if let Some(spacing) = value.double_spacing {
        border.override_flags.insert(B::DOUBLE_LINE_SPACING);
        border.double_spacing = spacing;
    }
    border
}

fn color_mode_out(value: Color) -> Option<IfccadMode<IfccadColor>> {
    match value {
        Color::ByLayer => Some(IfccadMode::ByLayer),
        Color::ByBlock => Some(IfccadMode::ByBlock),
        other => color_out(other).map(IfccadMode::Explicit),
    }
}

fn color_mode_in(value: &IfccadMode<IfccadColor>) -> Color {
    match value {
        IfccadMode::ByLayer => Color::ByLayer,
        IfccadMode::ByBlock => Color::ByBlock,
        IfccadMode::Explicit(value) => color_in(value),
    }
}

fn alignment_out(value: i32) -> Option<Alignment> {
    Some(match value {
        1 => Alignment::TopLeft,
        2 => Alignment::TopCenter,
        3 => Alignment::TopRight,
        4 => Alignment::MiddleLeft,
        5 => Alignment::MiddleCenter,
        6 => Alignment::MiddleRight,
        7 => Alignment::BottomLeft,
        8 => Alignment::BottomCenter,
        9 => Alignment::BottomRight,
        _ => return None,
    })
}

fn alignment_in(value: Alignment) -> i32 {
    match value {
        Alignment::TopLeft => 1,
        Alignment::TopCenter => 2,
        Alignment::TopRight => 3,
        Alignment::MiddleLeft => 4,
        Alignment::MiddleCenter => 5,
        Alignment::MiddleRight => 6,
        Alignment::BottomLeft => 7,
        Alignment::BottomCenter => 8,
        Alignment::BottomRight => 9,
    }
}

fn layout_out(value: ContentLayoutFlags) -> Option<ContentLayout> {
    match value {
        ContentLayoutFlags::FLOW => Some(ContentLayout::Flow),
        ContentLayoutFlags::STACKED_HORIZONTAL => Some(ContentLayout::Horizontal),
        ContentLayoutFlags::STACKED_VERTICAL => Some(ContentLayout::Vertical),
        _ => None,
    }
}

fn value_kind_out(code: i32) -> Option<ValueKind> {
    Some(match CellValueType::from(code as u32) {
        CellValueType::Long => ValueKind::Integer,
        CellValueType::Double => ValueKind::Real,
        CellValueType::String => ValueKind::Text,
        CellValueType::Date => ValueKind::Date,
        CellValueType::Point2D => ValueKind::Point2D,
        CellValueType::Point3D => ValueKind::Point3D,
        CellValueType::Handle => ValueKind::Object,
        CellValueType::Buffer => ValueKind::Binary,
        CellValueType::ResultBuffer => ValueKind::ResultBuffer,
        CellValueType::General => ValueKind::General,
        CellValueType::Unknown if code == 0 => ValueKind::UnspecifiedNumber,
        _ => return None,
    })
}

fn value_kind_in(value: ValueKind) -> i32 {
    let cad = match value {
        ValueKind::Integer => CellValueType::Long,
        ValueKind::Real => CellValueType::Double,
        ValueKind::Text => CellValueType::String,
        ValueKind::Date => CellValueType::Date,
        ValueKind::Point2D => CellValueType::Point2D,
        ValueKind::Point3D => CellValueType::Point3D,
        ValueKind::Object => CellValueType::Handle,
        ValueKind::Binary => CellValueType::Buffer,
        ValueKind::ResultBuffer => CellValueType::ResultBuffer,
        ValueKind::General => CellValueType::General,
        ValueKind::UnspecifiedNumber => CellValueType::Unknown,
        ValueKind::Boolean => CellValueType::Unknown,
    };
    cad as i32
}
