//! Ordinary LEADER/FCF mapping. Target links are resolved after owner graphs.
use super::dimension;
use super::*;
use acadrust::entities::{
    HooklineDirection, Leader, LeaderCreationType, LeaderPathType, Tolerance,
};
use ifccad::*;

// One source variable can set two native slots. Their explicit presence is
// preserved even when the stored value equals the dimension style.
macro_rules! override_fields {
    ($apply:ident)=>{$apply!{
        40=>presentation_scale,41=>arrows.size,
        42=>extension_lines.first.offset,42=>extension_lines.second.offset,
        44=>extension_lines.first.extension,44=>extension_lines.second.extension,
        45=>dimension_line.extension,46=>numbers.primary.rounding_increment,
        69=>text.background,70=>text.background,
        73=>text.inside_orientation,74=>text.outside_orientation,
        75=>extension_lines.first.suppressed,76=>extension_lines.second.suppressed,
        77=>text.attachment,78=>numbers.primary.zero_suppression,
        90=>numbers.primary.arc_symbol,140=>text.height,141=>arrows.center_mark,
        142=>arrows.tick_size,144=>numbers.primary.measurement_factor,
        144=>numbers.primary.application,147=>text.gap,
        172=>fit.keep_dimension_line_outside,174=>fit.force_text_inside,
        176=>dimension_line.appearance.color,
        177=>extension_lines.first.appearance.color,177=>extension_lines.second.appearance.color,
        178=>text.appearance.color,271=>numbers.primary.precision,
        275=>numbers.primary.angle_format,276=>numbers.primary.fraction_layout,
        277=>numbers.primary.format,278=>numbers.primary.decimal_separator,
        279=>text.movement,281=>dimension_line.suppress_first,282=>dimension_line.suppress_second,
        289=>fit.preferred_placement,290=>extension_lines.first.length,290=>extension_lines.second.length,
        340=>text.style,342=>arrows.first,342=>arrows.second,343=>arrows.first,344=>arrows.second,
        345=>dimension_line.appearance.line_pattern,
        346=>extension_lines.first.appearance.line_pattern,347=>extension_lines.second.appearance.line_pattern,
        371=>dimension_line.appearance.line_weight,
        372=>extension_lines.first.appearance.line_weight,372=>extension_lines.second.appearance.line_weight
    }};
}

pub(super) fn arrow_out(handle: Handle, writer: &mut Writer) -> Result<IfccadLeaderArrow, Error> {
    if handle.is_null() {
        return Ok(IfccadLeaderArrow::DefaultClosedFilled);
    }
    let record = writer
        .document
        .block_records
        .iter()
        .find(|b| b.handle == handle)
        .ok_or_else(|| annotation::invalid("leader arrow references a missing block"))?;
    let name = record.name.clone();
    if name.eq_ignore_ascii_case("_NONE")
        && record.base_point == Vector3::ZERO
        && record.description.is_empty()
        && !record.flags.is_xref
        && !record.flags.is_xref_overlay
        && writer.document.entities_in_block(&name).next().is_none()
    {
        return Ok(IfccadLeaderArrow::None);
    }
    let definition_id = writer
        .block(&name)?
        .ok_or_else(|| annotation::invalid("leader arrow block cannot be represented"))?;
    // Even a standard-looking name can identify a user-authored block.
    Ok(IfccadLeaderArrow::Symbol { definition_id })
}
pub(super) fn arrow_in(
    arrow: IfccadLeaderArrow,
    names: &Names,
    document: &Document,
) -> Result<Handle, Error> {
    match arrow {
        IfccadLeaderArrow::DefaultClosedFilled => Ok(Handle::NULL),
        IfccadLeaderArrow::None => document
            .block_records
            .get("_NONE")
            .map(|b| b.handle)
            .ok_or_else(|| annotation::invalid("missing prepared empty NONE arrow definition")),
        IfccadLeaderArrow::Symbol { definition_id } => names
            .blocks
            .get(&definition_id)
            .and_then(|name| document.block_records.get(name))
            .map(|b| b.handle)
            .ok_or_else(|| annotation::invalid("unresolved native leader arrow block")),
    }
}

pub(super) fn prepare_none_arrow(
    drawing: &IfccadDocument,
    document: &mut Document,
) -> Result<(), Error> {
    let none = |arrow: IfccadLeaderArrow| arrow == IfccadLeaderArrow::None;
    let presentation = |p: &IfccadMultiLeaderPresentation| {
        p.groups
            .iter()
            .flat_map(|g| &g.lines)
            .any(|l| none(l.settings.arrow))
    };
    let needed = drawing
        .dimension_styles
        .iter()
        .any(|s| none(s.leader_arrow))
        || drawing
            .multi_leader_styles
            .iter()
            .any(|s| none(s.common.line.arrow))
        || drawing
            .model
            .entities
            .iter()
            .chain(drawing.paper_layouts.iter().flat_map(|p| &p.entities))
            .chain(drawing.blocks.iter().flat_map(|b| &b.entities))
            .any(|e| match e.as_native().map(|e| &e.kind) {
                Some(IfccadEntityKind::Leader(v)) => v.arrow.is_some_and(none),
                Some(IfccadEntityKind::MultiLeader(v)) => {
                    none(v.settings.common.line.arrow)
                        || presentation(&v.presentation)
                        || v.legacy_arrow_overrides.iter().any(|a| none(a.arrow))
                        || v.annotation.as_ref().is_some_and(|a| {
                            a.contexts
                                .iter()
                                .any(|c| c.presentation.as_ref().is_some_and(presentation))
                        })
                }
                _ => false,
            });
    if !needed {
        return Ok(());
    }
    if let Some(source) = drawing
        .blocks
        .iter()
        .find(|b| b.name.eq_ignore_ascii_case("_NONE"))
    {
        if !source.entities.is_empty()
            || source.base_point != [0.; 3]
            || !source.description.is_empty()
        {
            return Err(annotation::invalid(
                "explicit absent arrow conflicts with a custom nonempty _NONE block",
            ));
        }
    }
    if document.block_records.get("_NONE").is_some() {
        return Ok(());
    }
    let mut record = BlockRecord::new("_NONE");
    record.handle = document.allocate_handle();
    record.block_entity_handle = document.allocate_handle();
    record.block_end_handle = document.allocate_handle();
    let mut start = Block::new("_NONE", Vector3::ZERO);
    start.common.handle = record.block_entity_handle;
    start.common.owner_handle = record.handle;
    let mut end = BlockEnd::new();
    end.common.handle = record.block_end_handle;
    end.common.owner_handle = record.handle;
    document.block_records.add(record)?;
    document.add_entity(EntityType::Block(start))?;
    document.add_entity(EntityType::BlockEnd(end))?;
    Ok(())
}

pub(super) fn style_out(
    writer: &mut Writer,
    name: &str,
    common: &EntityCommon,
) -> Result<
    (
        IfccadDimensionStyleId,
        IfccadDimensionOverrides,
        Option<IfccadLeaderArrow>,
    ),
    Error,
> {
    let (id, base) = writer
        .dim_styles
        .get(&name.to_ascii_lowercase())
        .cloned()
        .ok_or_else(|| {
            annotation::invalid(&format!("leader/FCF missing dimension style {name}"))
        })?;
    let source = writer
        .document
        .dim_styles
        .get(name)
        .ok_or_else(|| annotation::invalid("missing source dimension style"))?;
    let effective = dimension::with_overrides(source, common);
    let present = dimension::override_codes(common);
    let arrow = (present.contains(&341) || effective.dimldrblk != source.dimldrblk)
        .then(|| arrow_out(effective.dimldrblk, writer))
        .transpose()?;
    let values = dimension::settings_out(&effective, writer)
        .ok_or_else(|| annotation::invalid("unrepresentable leader/FCF style settings"))?;
    let mut overrides = dimension::overrides_between(&base, &values);
    let mut represented = std::collections::BTreeSet::from([341]);
    macro_rules! preserve {($($code:literal=>$($field:ident).+),*$(,)?)=>{$(if present.contains(&$code){overrides.$($field).+ = Some(values.$($field).+.clone());represented.insert($code);})*};}
    override_fields!(preserve);
    for code in present.difference(&represented) {
        writer.losses.note(format!("leader/FCF {} DSTYLE variable {code}: raw variable/presence has no independent native slot and is omitted",common.handle));
    }
    Ok((id, overrides, arrow))
}
fn style_in(
    id: IfccadDimensionStyleId,
    overrides: &IfccadDimensionOverrides,
    arrow: Option<IfccadLeaderArrow>,
    names: &Names,
    document: &Document,
    common: &mut EntityCommon,
) -> Result<String, Error> {
    let (name, settings, _) = names
        .dim_styles
        .get(&id)
        .ok_or_else(|| annotation::invalid("unresolved leader dimension style"))?;
    let base = document
        .dim_styles
        .get(name)
        .ok_or_else(|| annotation::invalid("missing imported dimension style"))?;
    let mut settings = settings.clone();
    overrides.apply(&mut settings);
    let mut effective = dimension::style_in(name, &settings, names, document);
    if let Some(arrow) = arrow {
        effective.dimldrblk = arrow_in(arrow, names, document)?;
    }
    let mut present = std::collections::BTreeSet::new();
    macro_rules! preserve {($($code:literal=>$($field:ident).+),*$(,)?)=>{$(if overrides.$($field).+.is_some(){present.insert($code);})*};}
    override_fields!(preserve);
    if arrow.is_some() {
        present.insert(341);
    }
    if let Some(record) = dimension::overrides_record_selected(base, &effective, &present) {
        common.extended_data.upsert_record(record);
    }
    Ok(name.clone())
}

// LEADER vertices/vectors use owner WCS; the native values use this frame once.
pub(super) fn local_direction(frame: &IfccadPlacement, value: Vector3) -> [f64; 3] {
    let x = vector(frame.x_axis);
    let y = vector(frame.y_axis);
    let z = x.cross(&y);
    [value.dot(&x), value.dot(&y), value.dot(&z)]
}
pub(super) fn local_point(frame: &IfccadPlacement, value: Vector3) -> [f64; 3] {
    local_direction(frame, value - vector(frame.origin))
}
pub(super) fn world_direction(frame: &IfccadPlacement, value: [f64; 3]) -> Vector3 {
    let x = vector(frame.x_axis);
    let y = vector(frame.y_axis);
    x * value[0] + y * value[1] + x.cross(&y) * value[2]
}
pub(super) fn world_point(frame: &IfccadPlacement, value: [f64; 3]) -> Vector3 {
    vector(frame.origin) + world_direction(frame, value)
}
pub(super) fn local_frame(frame: &IfccadPlacement, value: &IfccadPlacement) -> IfccadPlacement {
    IfccadPlacement {
        origin: local_point(frame, vector(value.origin)),
        x_axis: local_direction(frame, vector(value.x_axis)),
        y_axis: local_direction(frame, vector(value.y_axis)),
    }
}
pub(super) fn world_frame(frame: &IfccadPlacement, value: &IfccadPlacement) -> IfccadPlacement {
    IfccadPlacement {
        origin: array(world_point(frame, value.origin)),
        x_axis: array(world_direction(frame, value.x_axis)),
        y_axis: array(world_direction(frame, value.y_axis)),
    }
}

pub(super) fn leader_out(writer: &mut Writer, value: &Leader) -> Result<IfccadEntityKind, Error> {
    let (dimension_style, overrides, arrow) =
        style_out(writer, &value.dimension_style, &value.common)?;
    let placement = placement_out(Vector3::ZERO, value.normal, 0.);
    let points = value
        .vertices
        .iter()
        .map(|p| local_point(&placement, *p))
        .collect();
    let defaults = Leader::new();
    if value.override_color != defaults.override_color
        || value.dimension_gap != defaults.dimension_gap
        || value.arrowhead_type != defaults.arrowhead_type
        || value.arrow_size != defaults.arrow_size
        || value.byblock_color != defaults.byblock_color
        || value.dwg_unknown_bit1
        || value.dwg_unknown_bit2
        || value.dwg_unknown_bit3
        || value.dwg_unknown_bit4
        || value.dwg_unknown_bit5
        || value.dwg_unknown_short1 != 0
    {
        writer.losses.note(format!(
            "LEADER {} has additional DWG-only cached/unknown values",
            value.common.handle
        ));
    }
    Ok(IfccadEntityKind::Leader(Box::new(IfccadLeader {
        path: match value.path_type {
            LeaderPathType::StraightLine => IfccadLeaderPath::StraightSegments(points),
            LeaderPathType::Spline => IfccadLeaderPath::SplineThroughVertices(points),
        },
        annotation_kind: match value.creation_type {
            LeaderCreationType::WithText => IfccadLeaderAnnotationKind::MText,
            LeaderCreationType::WithTolerance => IfccadLeaderAnnotationKind::Tolerance,
            LeaderCreationType::WithBlock => IfccadLeaderAnnotationKind::Block,
            LeaderCreationType::NoAnnotation => IfccadLeaderAnnotationKind::None,
        },
        annotation_entity: None,
        dimension_style,
        overrides,
        arrow,
        arrow_enabled: value.arrow_enabled,
        hook_enabled: value.hookline_enabled,
        hook_direction: match value.hookline_direction {
            HooklineDirection::Opposite => IfccadLeaderHookDirection::OppositeHorizontal,
            HooklineDirection::Same => IfccadLeaderHookDirection::AlongHorizontal,
        },
        annotation_box: Some([value.text_width, value.text_height]),
        horizontal_direction: local_direction(&placement, value.horizontal_direction),
        block_offset: local_direction(&placement, value.block_offset),
        annotation_offset: local_direction(&placement, value.annotation_offset),
        origin: local_point(&placement, value.origin),
        placement,
        annotation: None,
        graphics: None,
    })))
}
pub(super) fn leader_in(
    value: &IfccadLeader,
    document: &Document,
    names: &Names,
) -> Result<EntityType, Error> {
    let mut target = Leader::new();
    target.dimension_style = style_in(
        value.dimension_style,
        &value.overrides,
        value.arrow,
        names,
        document,
        &mut target.common,
    )?;
    target.vertices = value
        .path
        .vertices()
        .iter()
        .map(|p| world_point(&value.placement, *p))
        .collect();
    target.path_type = match value.path {
        IfccadLeaderPath::StraightSegments(_) => LeaderPathType::StraightLine,
        IfccadLeaderPath::SplineThroughVertices(_) => LeaderPathType::Spline,
    };
    target.creation_type = match value.annotation_kind {
        IfccadLeaderAnnotationKind::None => LeaderCreationType::NoAnnotation,
        IfccadLeaderAnnotationKind::MText => LeaderCreationType::WithText,
        IfccadLeaderAnnotationKind::Block => LeaderCreationType::WithBlock,
        IfccadLeaderAnnotationKind::Tolerance => LeaderCreationType::WithTolerance,
    };
    target.arrow_enabled = value.arrow_enabled;
    target.hookline_enabled = value.hook_enabled;
    target.hookline_direction = match value.hook_direction {
        IfccadLeaderHookDirection::OppositeHorizontal => HooklineDirection::Opposite,
        IfccadLeaderHookDirection::AlongHorizontal => HooklineDirection::Same,
    };
    if let Some([width, height]) = value.annotation_box {
        target.text_width = width;
        target.text_height = height;
    }
    target.normal = vector(value.placement.x_axis).cross(&vector(value.placement.y_axis));
    target.horizontal_direction = world_direction(&value.placement, value.horizontal_direction);
    target.block_offset = world_direction(&value.placement, value.block_offset);
    target.annotation_offset = world_direction(&value.placement, value.annotation_offset);
    target.origin = world_point(&value.placement, value.origin);
    Ok(EntityType::Leader(target))
}
pub(super) fn tolerance_out(
    writer: &mut Writer,
    value: &Tolerance,
) -> Result<IfccadEntityKind, Error> {
    let (dimension_style, overrides, _) =
        style_out(writer, &value.dimension_style_name, &value.common)?;
    let normal = value.normal.normalize();
    let x = value.direction.normalize();
    if x.dot(&normal).abs() > 1e-10 {
        return Err(annotation::invalid(
            "FCF direction is not in its normal plane",
        ));
    }
    if value.dwg_unknown_short != 0 {
        writer.losses.note(format!(
            "TOLERANCE {} unknown DWG short",
            value.common.handle
        ));
    }
    Ok(IfccadEntityKind::Tolerance(Box::new(IfccadTolerance {
        placement: IfccadPlacement {
            origin: array(value.insertion_point),
            x_axis: array(x),
            y_axis: array(normal.cross(&x)),
        },
        dimension_style,
        overrides,
        notation: IfccadToleranceNotation::CadFeatureControlTextV1 {
            code: value.text.clone(),
        },
        annotation: None,
        graphics: None,
    })))
}
pub(super) fn tolerance_in(
    value: &IfccadTolerance,
    document: &Document,
    names: &Names,
) -> Result<EntityType, Error> {
    let mut target = Tolerance::with_text(vector(value.placement.origin), value.notation.code());
    target.direction = vector(value.placement.x_axis);
    target.normal = target.direction.cross(&vector(value.placement.y_axis));
    target.dimension_style_name = style_in(
        value.dimension_style,
        &value.overrides,
        None,
        names,
        document,
        &mut target.common,
    )?;
    target.dimension_style_handle = document
        .dim_styles
        .get(&target.dimension_style_name)
        .map(|s| s.handle);
    Ok(EntityType::Tolerance(target))
}
pub(super) fn link_export(writer: &mut Writer) -> Result<(), Error> {
    let ids = &writer.entity_ids;
    let targets: HashMap<_, _> = writer
        .document
        .entities()
        .filter_map(|e| match e {
            EntityType::Leader(l) if !l.annotation_handle.is_null() => {
                Some((l.common.handle, l.annotation_handle))
            }
            _ => None,
        })
        .collect();
    for entity in writer
        .drawing
        .blocks
        .iter_mut()
        .flat_map(|b| &mut b.entities)
        .chain(
            writer
                .drawing
                .paper_layouts
                .iter_mut()
                .flat_map(|l| &mut l.entities),
        )
        .chain(&mut writer.drawing.model.entities)
    {
        let IfccadEntity::Native(entity) = entity else {
            continue;
        };
        let source = ids
            .iter()
            .find_map(|(handle, id)| (*id == entity.id).then_some(handle));
        match &mut entity.kind {
            IfccadEntityKind::Leader(leader) => {
                if let Some(target) = source.and_then(|h| targets.get(h)) {
                    leader.annotation_entity = Some(*ids.get(target).ok_or_else(|| {
                        annotation::invalid("LEADER annotation target was not converted")
                    })?);
                }
            }
            IfccadEntityKind::MultiLeader(value) => {
                let Some(EntityType::MultiLeader(original)) =
                    source.and_then(|h| writer.document.get_entity(*h))
                else {
                    return Err(annotation::invalid(
                        "missing MLeader source for label binding",
                    ));
                };
                for (label, original) in value
                    .block_labels
                    .iter_mut()
                    .zip(&original.block_attributes)
                {
                    label.definition_id = original
                        .attribute_definition_handle
                        .filter(|h| !h.is_null())
                        .map(|h| {
                            ids.get(&h).copied().ok_or_else(|| {
                                annotation::invalid("MLeader ATTDEF target was not converted")
                            })
                        })
                        .transpose()?;
                    if label.definition_id.is_none() {
                        writer.losses.note(format!(
                            "MLeader {} label {} has no direct ATTDEF binding",
                            entity.id, label.key
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}
pub(super) fn link_import(
    drawing: &IfccadDocument,
    document: &mut Document,
    handles: &HashMap<u64, Handle>,
) -> Result<(), Error> {
    for entity in drawing
        .blocks
        .iter()
        .flat_map(|b| &b.entities)
        .chain(drawing.paper_layouts.iter().flat_map(|l| &l.entities))
        .chain(&drawing.model.entities)
    {
        let Some(entity) = entity.as_native() else {
            continue;
        };
        if let IfccadEntityKind::Leader(value) = &entity.kind {
            let target = value
                .annotation_entity
                .map(|id| {
                    handles
                        .get(&id)
                        .copied()
                        .ok_or_else(|| annotation::invalid("missing imported LEADER target"))
                })
                .transpose()?
                .unwrap_or(Handle::NULL);
            if let Some(EntityType::Leader(leader)) = handles
                .get(&entity.id)
                .and_then(|h| document.get_entity_mut(*h))
            {
                leader.annotation_handle = target;
            }
        }
        if let IfccadEntityKind::MultiLeader(value) = &entity.kind {
            let targets = value
                .block_labels
                .iter()
                .map(|label| {
                    label
                        .definition_id
                        .map(|id| {
                            handles.get(&id).copied().ok_or_else(|| {
                                annotation::invalid("missing imported MLeader ATTDEF")
                            })
                        })
                        .transpose()
                })
                .collect::<Result<Vec<_>, Error>>()?;
            if let Some(EntityType::MultiLeader(target)) = handles
                .get(&entity.id)
                .and_then(|h| document.get_entity_mut(*h))
            {
                for (label, handle) in target.block_attributes.iter_mut().zip(targets) {
                    label.attribute_definition_handle = handle;
                }
            }
        }
    }
    Ok(())
}
