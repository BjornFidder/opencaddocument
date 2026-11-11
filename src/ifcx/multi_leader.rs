//! Typed MLeader settings and independently stored presentations.
use super::*;
use super::{leader, text};
use acadrust::entities::{multileader as cad, MText, MultiLeader};
use ifccad::*;
use std::collections::BTreeSet;
mod styles;
pub(super) use styles::{export as export_styles, import as import_styles};

macro_rules! codes {
    ($out:ident,$input:ident,$ty:ident,{$($n:literal=>$v:ident),+$(,)?}) => {
        fn $out(value:i16)->Result<$ty,Error>{Ok(match value{$($n=>$ty::$v,)+_=>return Err(annotation::invalid(concat!("unsupported MLeader ",stringify!($ty))))})}
        fn $input(value:$ty)->i16{match value{$($ty::$v=>$n,)+}}
    }
}
codes!(path_out,path_in,IfccadMultiLeaderPathKind,{0=>Invisible,1=>StraightSegments,2=>SplineThroughVertices});
codes!(content_out,content_in,IfccadMultiLeaderContentKind,{0=>None,1=>Block,2=>MText,3=>Tolerance});
codes!(attachment_out,attachment_in,IfccadMultiLeaderTextAttachment,{0=>TopOfTopLine,1=>MiddleOfTopLine,2=>MiddleOfText,3=>MiddleOfBottomLine,4=>BottomOfBottomLine,5=>BottomLine,6=>BottomOfTopLineUnderlineBottomLine,7=>BottomOfTopLineUnderlineTopLine,8=>BottomOfTopLineUnderlineAll,9=>CenterOfText,10=>CenterOfTextOverline});
codes!(point_out,point_in,IfccadMultiLeaderTextAttachmentPoint,{1=>Left,2=>Center,3=>Right});
codes!(align_out,align_in,IfccadMultiLeaderTextAlignment,{0=>Left,1=>Center,2=>Right});
codes!(direction_out,direction_in,IfccadMultiLeaderAttachmentDirection,{0=>Horizontal,1=>Vertical});
codes!(angle_out,angle_in,IfccadMultiLeaderTextAngle,{0=>ParallelToLastSegment,1=>Horizontal,2=>ReadableParallel});
codes!(connection_out,connection_in,IfccadMultiLeaderBlockConnection,{0=>Extents,1=>BasePoint});

fn color_out_exact(value: Color) -> Result<IfccadMode<IfccadColor>, Error> {
    Ok(match value {
        Color::ByLayer => IfccadMode::ByLayer,
        Color::ByBlock => IfccadMode::ByBlock,
        value => IfccadMode::Explicit(
            color_out(value).ok_or_else(|| annotation::invalid("unsupported MLeader color"))?,
        ),
    })
}
fn color_in_exact(value: &IfccadMode<IfccadColor>) -> Result<Color, Error> {
    Ok(match value {
        IfccadMode::ByLayer => Color::ByLayer,
        IfccadMode::ByBlock => Color::ByBlock,
        IfccadMode::Explicit(value) => {
            let result = color_in(value);
            if color_out(result).as_ref() != Some(value) {
                return Err(annotation::invalid(
                    "MLeader color metadata has no exact CAD mapping",
                ));
            }
            result
        }
    })
}
fn weight_out_exact(value: LineWeight) -> IfccadLeaderLineWeight {
    match value {
        LineWeight::ByLayer => IfccadLeaderLineWeight::ByLayer,
        LineWeight::ByBlock => IfccadLeaderLineWeight::ByBlock,
        LineWeight::Default => IfccadLeaderLineWeight::DrawingDefault,
        LineWeight::Value(n) => IfccadLeaderLineWeight::Explicit(f64::from(n) / 100.),
    }
}
fn weight_in_exact(value: IfccadLeaderLineWeight) -> Result<LineWeight, Error> {
    Ok(match value {
        IfccadLeaderLineWeight::ByLayer => LineWeight::ByLayer,
        IfccadLeaderLineWeight::ByBlock => LineWeight::ByBlock,
        IfccadLeaderLineWeight::DrawingDefault => LineWeight::Default,
        IfccadLeaderLineWeight::Explicit(n) => {
            let raw = (n * 100.).round();
            if !(0. ..=211.).contains(&raw) || raw / 100. != n {
                return Err(annotation::invalid(
                    "MLeader line weight is not exactly representable in CAD",
                ));
            }
            LineWeight::Value(raw as i16)
        }
    })
}
fn block_out(writer: &mut Writer, handle: Option<Handle>) -> Result<Option<u64>, Error> {
    let Some(handle) = handle.filter(|h| !h.is_null()) else {
        return Ok(None);
    };
    let name = writer
        .document
        .block_records
        .iter()
        .find(|b| b.handle == handle)
        .map(|b| b.name.clone())
        .ok_or_else(|| annotation::invalid("missing MLeader block"))?;
    writer
        .block(&name)?
        .map(Some)
        .ok_or_else(|| annotation::invalid("unrepresentable MLeader block"))
}
fn block_in(names: &Names, document: &Document, id: Option<u64>) -> Result<Option<Handle>, Error> {
    id.map(|id| {
        names
            .blocks
            .get(&id)
            .and_then(|n| document.block_records.get(n))
            .map(|b| b.handle)
            .ok_or_else(|| annotation::invalid("unresolved MLeader block"))
    })
    .transpose()
}
fn pattern_out(
    writer: &Writer,
    handle: Option<Handle>,
) -> Result<IfccadMode<IfccadLinePatternId>, Error> {
    let pattern = match handle.filter(|h| !h.is_null()) {
        Some(h) => writer.document.line_types.iter().find(|p| p.handle == h),
        None => writer.document.line_types.get("CONTINUOUS"),
    }
    .ok_or_else(|| annotation::invalid("unresolved MLeader linetype"))?;
    Ok(if pattern.name.eq_ignore_ascii_case("BYLAYER") {
        IfccadMode::ByLayer
    } else if pattern.name.eq_ignore_ascii_case("BYBLOCK") {
        IfccadMode::ByBlock
    } else {
        IfccadMode::Explicit(
            *writer
                .line_patterns
                .get(&pattern.name.to_ascii_lowercase())
                .ok_or_else(|| annotation::invalid("missing native MLeader line pattern"))?,
        )
    })
}
fn pattern_in(
    names: &Names,
    document: &Document,
    value: &IfccadMode<IfccadLinePatternId>,
) -> Result<Option<Handle>, Error> {
    let name = match value {
        IfccadMode::ByLayer => "BYLAYER",
        IfccadMode::ByBlock => "BYBLOCK",
        IfccadMode::Explicit(id) => names
            .line_patterns
            .get(id)
            .map(String::as_str)
            .ok_or_else(|| annotation::invalid("unresolved native MLeader linetype"))?,
    };
    document
        .line_types
        .get(name)
        .map(|p| Some(p.handle))
        .ok_or_else(|| annotation::invalid("missing CAD MLeader linetype"))
}
fn style_out(writer: &Writer, handle: Option<Handle>) -> Result<Option<IfccadTextStyleId>, Error> {
    handle
        .filter(|h| !h.is_null())
        .map(|h| {
            writer
                .style_handles
                .get(&h)
                .copied()
                .ok_or_else(|| annotation::invalid("missing MLeader text style"))
        })
        .transpose()
}
fn style_in(names: &Names, value: Option<IfccadTextStyleId>) -> Result<Option<Handle>, Error> {
    value
        .map(|id| {
            names
                .style_handles
                .get(&id)
                .copied()
                .ok_or_else(|| annotation::invalid("unresolved MLeader text style"))
        })
        .transpose()
}
fn line_out(
    writer: &mut Writer,
    path: i16,
    color: Color,
    pattern: Option<Handle>,
    weight: LineWeight,
    arrow: Option<Handle>,
    size: f64,
) -> Result<IfccadMultiLeaderLineSettings, Error> {
    Ok(IfccadMultiLeaderLineSettings {
        path_kind: path_out(path)?,
        color: color_out_exact(color)?,
        line_pattern: pattern_out(writer, pattern)?,
        line_weight: weight_out_exact(weight),
        arrow: leader::arrow_out(arrow.unwrap_or(Handle::NULL), writer)?,
        arrow_size: size,
    })
}
fn common_out(
    writer: &mut Writer,
    value: &MultiLeader,
) -> Result<IfccadMultiLeaderCommonSettings, Error> {
    Ok(IfccadMultiLeaderCommonSettings {
        line: line_out(
            writer,
            value.path_type as i16,
            value.line_color,
            value.line_type_handle,
            value.line_weight,
            value.arrowhead_handle,
            value.arrowhead_size,
        )?,
        landing_enabled: value.enable_landing,
        dogleg_enabled: value.enable_dogleg,
        dogleg_length: value.dogleg_length,
        content_kind: content_out(value.content_type as i16)?,
        text: IfccadMultiLeaderTextSettings {
            style_id: style_out(writer, value.text_style_handle)?,
            color: color_out_exact(value.text_color)?,
            height: value.text_height,
            frame: value.text_frame,
            attachments: IfccadMultiLeaderTextAttachments {
                left: attachment_out(value.text_left_attachment as i16)?,
                right: attachment_out(value.text_right_attachment as i16)?,
                top: attachment_out(value.text_top_attachment as i16)?,
                bottom: attachment_out(value.text_bottom_attachment as i16)?,
            },
            attachment_direction: direction_out(value.text_attachment_direction as i16)?,
            angle: angle_out(value.text_angle_type as i16)?,
            alignment: align_out(value.text_alignment as i16)?,
        },
        block: IfccadMultiLeaderBlockSettings {
            definition_id: block_out(writer, value.block_content_handle)?,
            color: color_out_exact(value.block_content_color)?,
            connection: connection_out(value.block_connection_type as i16)?,
            rotation: value.block_rotation,
            scale: array(value.block_scale),
        },
        scale_factor: value.scale_factor,
    })
}
fn common_in(
    value: &IfccadMultiLeaderCommonSettings,
    target: &mut MultiLeader,
    names: &Names,
    document: &Document,
) -> Result<(), Error> {
    target.path_type = path_in(value.line.path_kind).into();
    target.line_color = color_in_exact(&value.line.color)?;
    target.line_type_handle = pattern_in(names, document, &value.line.line_pattern)?;
    target.line_weight = weight_in_exact(value.line.line_weight)?;
    target.arrowhead_handle =
        Some(leader::arrow_in(value.line.arrow, names, document)?).filter(|h| !h.is_null());
    target.arrowhead_size = value.line.arrow_size;
    target.enable_landing = value.landing_enabled;
    target.enable_dogleg = value.dogleg_enabled;
    target.dogleg_length = value.dogleg_length;
    target.content_type = content_in(value.content_kind).into();
    target.text_style_handle = style_in(names, value.text.style_id)?;
    target.text_color = color_in_exact(&value.text.color)?;
    target.text_height = value.text.height;
    target.text_frame = value.text.frame;
    target.text_left_attachment = attachment_in(value.text.attachments.left).into();
    target.text_right_attachment = attachment_in(value.text.attachments.right).into();
    target.text_top_attachment = attachment_in(value.text.attachments.top).into();
    target.text_bottom_attachment = attachment_in(value.text.attachments.bottom).into();
    target.text_attachment_direction = direction_in(value.text.attachment_direction).into();
    target.text_angle_type = angle_in(value.text.angle).into();
    target.text_alignment = align_in(value.text.alignment).into();
    target.block_content_handle = block_in(names, document, value.block.definition_id)?;
    target.block_content_color = color_in_exact(&value.block.color)?;
    target.block_connection_type = connection_in(value.block.connection).into();
    target.block_rotation = value.block.rotation;
    target.block_scale = vector(value.block.scale);
    target.scale_factor = value.scale_factor;
    Ok(())
}

fn intervals_out(
    frame: &IfccadPlacement,
    values: &[cad::StartEndPointPair],
) -> Vec<IfccadLeaderBreakInterval> {
    values
        .iter()
        .map(|p| IfccadLeaderBreakInterval {
            start: leader::local_point(frame, p.start_point),
            end: leader::local_point(frame, p.end_point),
        })
        .collect()
}
fn intervals_in(
    frame: &IfccadPlacement,
    values: &[IfccadLeaderBreakInterval],
) -> Vec<cad::StartEndPointPair> {
    values
        .iter()
        .map(|p| {
            cad::StartEndPointPair::new(
                leader::world_point(frame, p.start),
                leader::world_point(frame, p.end),
            )
        })
        .collect()
}
pub(super) fn presentation_out(
    writer: &mut Writer,
    value: &cad::MultiLeaderAnnotContext,
    kind: IfccadMultiLeaderContentKind,
) -> Result<IfccadMultiLeaderPresentation, Error> {
    if kind == IfccadMultiLeaderContentKind::Tolerance {
        return Err(annotation::invalid("MLeader ToleranceContent physical payload is unqualified; use explicit experimental probe"));
    }
    let plane = IfccadPlacement {
        origin: array(value.base_point),
        x_axis: array(value.base_direction),
        y_axis: array(value.base_vertical),
    };
    plane.coordinate_frame()?;
    let mut next_part_id = 1;
    let mut groups = Vec::new();
    for root in &value.leader_roots {
        let key = next_part_id;
        next_part_id += 1;
        let mut lines = Vec::new();
        for line in &root.lines {
            let line_key = next_part_id;
            next_part_id += 1;
            let mut breaks = Vec::new();
            if let Some(first) = line.break_infos.first() {
                if !line.break_points.is_empty()
                    && (line.segment_index != first.segment_index
                        || line.break_points != first.break_points)
                {
                    return Err(annotation::invalid(
                        "conflicting MLeader legacy and complete break aliases",
                    ));
                }
            }
            if line.break_infos.is_empty() && !line.break_points.is_empty() {
                breaks.push(IfccadMultiLeaderSegmentBreaks {
                    segment_index: u32::try_from(line.segment_index)
                        .map_err(|_| annotation::invalid("negative MLeader break segment"))?,
                    intervals: intervals_out(&plane, &line.break_points),
                });
            }
            for b in &line.break_infos {
                breaks.push(IfccadMultiLeaderSegmentBreaks {
                    segment_index: u32::try_from(b.segment_index)
                        .map_err(|_| annotation::invalid("negative MLeader break segment"))?,
                    intervals: intervals_out(&plane, &b.break_points),
                });
            }
            lines.push(IfccadMultiLeaderLine {
                key: line_key,
                source_index: Some(line.index),
                settings: line_out(
                    writer,
                    line.path_type as i16,
                    line.line_color,
                    line.line_type_handle,
                    line.line_weight,
                    line.arrowhead_handle,
                    line.arrowhead_size,
                )?,
                overridden_properties: line_flags_out(line.override_flags.bits())?,
                vertices: line
                    .points
                    .iter()
                    .map(|p| leader::local_point(&plane, *p))
                    .collect(),
                connection: if root.content_valid {
                    IfccadMultiLeaderConnection::GroupConnection
                } else {
                    IfccadMultiLeaderConnection::ExplicitLastVertex
                },
                breaks,
            });
        }
        groups.push(IfccadMultiLeaderGroup {
            key,
            source_index: Some(root.leader_index),
            has_connection_point: root.content_valid,
            connection_point: leader::local_point(&plane, root.connection_point),
            has_dogleg_direction: root.unknown,
            dogleg_direction: leader::local_direction(&plane, root.direction),
            dogleg_length: root.landing_distance,
            text_attachment_direction: direction_out(root.text_attachment_direction as i16)?,
            lines,
            breaks: intervals_out(&plane, &root.break_points),
        });
    }
    let mut content = IfccadMultiLeaderStoredContent::default();
    if value.has_text_contents {
        let mut text = text_out(writer, value)?;
        text.text.placement = leader::local_frame(&plane, &text.text.placement);
        content.m_text = Some(Box::new(text));
    }
    if value.has_block_contents {
        let definition_id = block_out(writer, value.block_content_handle)?
            .ok_or_else(|| annotation::invalid("MLeader block content lacks definition"))?;
        content.block = Some(IfccadMultiLeaderBlockContent {
            definition_id,
            transform: IfccadBlockTransform {
                placement: leader::local_frame(
                    &plane,
                    &placement_out(
                        to_ocs(value.block_content_location, value.block_content_normal),
                        value.block_content_normal,
                        0.,
                    ),
                ),
                rotation: value.block_rotation,
                scale: array(value.block_content_scale),
            },
            color: color_out_exact(value.block_content_color)?,
            connection: connection_out(value.block_connection_type as i16)?,
            stored_transform: Some(value.transform_matrix),
        });
    }
    if value.dwg_unknown_text_bit || value.standalone_flags != 0 || value.standalone_uses_root_flags
    {
        writer
            .losses
            .note("MLeader presentation has nondefault unqualified DWG flags");
    }
    let content_base_point = leader::local_point(&plane, value.content_base_point);
    Ok(IfccadMultiLeaderPresentation {
        plane,
        normal_reversed: value.normal_reversed,
        content_base_point,
        scale_factor: value.scale_factor,
        text_height: value.text_height,
        arrow_size: value.arrowhead_size,
        landing_gap: value.landing_gap,
        next_part_id,
        groups,
        content,
    })
}
pub(super) fn presentation_in(
    value: &IfccadMultiLeaderPresentation,
    names: &Names,
    document: &Document,
    losses: &mut Losses,
) -> Result<cad::MultiLeaderAnnotContext, Error> {
    if value.content.tolerance.is_some() {
        return Err(annotation::invalid(
            "MLeader tolerance payload mapping is unqualified",
        ));
    }
    if value.content.m_text.is_some() && value.content.block.is_some() {
        return Err(annotation::invalid(
            "CAD MLeader writer cannot retain simultaneous text and block payloads",
        ));
    }
    let mut target = cad::MultiLeaderAnnotContext::new();
    let frame = &value.plane;
    target.base_point = vector(frame.origin);
    target.base_direction = vector(frame.x_axis);
    target.base_vertical = vector(frame.y_axis);
    target.normal_reversed = value.normal_reversed;
    target.content_base_point = leader::world_point(frame, value.content_base_point);
    target.scale_factor = value.scale_factor;
    target.text_height = value.text_height;
    target.arrowhead_size = value.arrow_size;
    target.landing_gap = value.landing_gap;
    for (index, group) in value.groups.iter().enumerate() {
        let mut root = cad::LeaderRoot::new(group.source_index.unwrap_or(
            i32::try_from(index).map_err(|_| annotation::invalid("too many MLeader groups"))?,
        ));
        root.content_valid = group.has_connection_point;
        root.connection_point = leader::world_point(frame, group.connection_point);
        root.unknown = group.has_dogleg_direction;
        root.direction = leader::world_direction(frame, group.dogleg_direction);
        root.landing_distance = group.dogleg_length;
        root.text_attachment_direction = direction_in(group.text_attachment_direction).into();
        root.break_points = intervals_in(frame, &group.breaks);
        for (index, line) in group.lines.iter().enumerate() {
            if (line.connection == IfccadMultiLeaderConnection::GroupConnection)
                != group.has_connection_point
            {
                return Err(annotation::invalid(
                    "mixed MLeader endpoint modes have no exact CAD root mapping",
                ));
            }
            let mut converted = cad::LeaderLine::new(line.source_index.unwrap_or(
                i32::try_from(index).map_err(|_| annotation::invalid("too many MLeader lines"))?,
            ));
            converted.points = line
                .vertices
                .iter()
                .map(|p| leader::world_point(frame, *p))
                .collect();
            converted.path_type = path_in(line.settings.path_kind).into();
            converted.line_color = color_in_exact(&line.settings.color)?;
            converted.line_type_handle = pattern_in(names, document, &line.settings.line_pattern)?;
            converted.line_weight = weight_in_exact(line.settings.line_weight)?;
            converted.arrowhead_handle =
                Some(leader::arrow_in(line.settings.arrow, names, document)?)
                    .filter(|h| !h.is_null());
            converted.arrowhead_size = line.settings.arrow_size;
            converted.override_flags = cad::LeaderLinePropertyOverrideFlags::from_bits_retain(
                line_flags_in(&line.overridden_properties),
            );
            converted.break_infos = line
                .breaks
                .iter()
                .map(|b| {
                    Ok(cad::LeaderLineBreakInfo {
                        segment_index: i32::try_from(b.segment_index).map_err(|_| {
                            annotation::invalid("MLeader segment index exceeds CAD range")
                        })?,
                        break_points: intervals_in(frame, &b.intervals),
                    })
                })
                .collect::<Result<_, Error>>()?;
            converted.break_info_count = i32::try_from(converted.break_infos.len())
                .map_err(|_| annotation::invalid("too many MLeader breaks"))?;
            if let Some(first) = converted.break_infos.first() {
                converted.segment_index = first.segment_index;
                converted.break_points = first.break_points.clone();
            }
            root.lines.push(converted);
        }
        target.leader_roots.push(root);
    }
    if let Some(text) = &value.content.m_text {
        if text.text.height != value.text_height {
            return Err(annotation::invalid(
                "independent MLeader context and owned text heights cannot both be encoded in CAD",
            ));
        }
        let mut text = text.as_ref().clone();
        text.text.placement = leader::world_frame(frame, &text.text.placement);
        text_in(&text, &mut target, names, losses)?;
    }
    if let Some(block) = &value.content.block {
        target.has_block_contents = true;
        target.block_content_handle = block_in(names, document, Some(block.definition_id))?;
        let placement = leader::world_frame(frame, &block.transform.placement);
        target.block_content_location = vector(placement.origin);
        let (_, normal, rotation) = placement_in(&placement);
        target.block_content_normal = normal;
        target.block_rotation = rotation + block.transform.rotation;
        target.block_content_scale = vector(block.transform.scale);
        target.block_content_color = color_in_exact(&block.color)?;
        target.block_connection_type = connection_in(block.connection).into();
        if let Some(matrix) = block.stored_transform {
            target.transform_matrix = matrix;
        }
    }
    Ok(target)
}

fn text_out(
    writer: &mut Writer,
    value: &cad::MultiLeaderAnnotContext,
) -> Result<IfccadMultiLeaderTextContent, Error> {
    let mut m = MText::new();
    m.value = value.text_string.clone();
    m.insertion_point = value.text_location;
    m.normal = value.text_normal;
    m.dwg_x_direction = Some(value.text_direction);
    m.rotation = value.text_rotation;
    m.height = value.text_height;
    m.rectangle_width = value.text_width;
    m.line_spacing_factor = value.line_spacing_factor;
    m.line_spacing_style = value.line_spacing_style;
    m.style = match value.text_style_handle.filter(|h| !h.is_null()) {
        Some(h) => writer.document.text_styles.iter().find(|s| s.handle == h),
        None => writer.document.text_styles.get("Standard"),
    }
    .ok_or_else(|| annotation::invalid("MLeader content text style is unresolved"))?
    .name
    .clone();
    m.attachment_point = match value.text_attachment_point {
        cad::TextAttachmentPointType::Left => acadrust::entities::AttachmentPoint::TopLeft,
        cad::TextAttachmentPointType::Center => acadrust::entities::AttachmentPoint::TopCenter,
        cad::TextAttachmentPointType::Right => acadrust::entities::AttachmentPoint::TopRight,
    };
    m.drawing_direction = match value.text_flow_direction {
        cad::FlowDirectionType::Horizontal => acadrust::entities::DrawingDirection::LeftToRight,
        cad::FlowDirectionType::Vertical => acadrust::entities::DrawingDirection::TopToBottom,
        cad::FlowDirectionType::ByStyle => acadrust::entities::DrawingDirection::ByStyle,
    };
    m.column_data.column_type = value.column_type;
    m.column_data.width = value.column_width;
    m.column_data.gutter = value.column_gutter;
    m.column_data.flow_reversed = value.column_flow_reversed;
    m.column_data.heights = value.column_sizes.clone();
    m.column_data.column_count = i32::try_from(value.column_sizes.len())
        .map_err(|_| annotation::invalid("too many text columns"))?;
    m.column_data.auto_height = value.text_height_automatic;
    m.rectangle_height = Some(value.text_boundary_height);
    let Some(IfccadEntityKind::MText(text)) = text::mtext_out(writer, &m) else {
        return Err(annotation::invalid("unrepresentable owned MLeader text"));
    };
    if !matches!((value.background_transparency as u32) >> 24, 0..=3) {
        return Err(annotation::invalid(
            "unknown MLeader background transparency method",
        ));
    }
    let transparency = Transparency::from_alpha_value(value.background_transparency as u32);
    let opacity = match transparency {
        Transparency::ByLayer => IfccadMode::ByLayer,
        Transparency::ByBlock => IfccadMode::ByBlock,
        Transparency::Explicit(alpha) => IfccadMode::Explicit(f64::from(255 - alpha) / 255.),
    };
    Ok(IfccadMultiLeaderTextContent {
        text: *text,
        state: IfccadMultiLeaderTextState {
            stored_rotation: Some(value.text_rotation),
            boundary_height: value.text_boundary_height,
            automatic_height: value.text_height_automatic,
            word_break: value.word_break,
            color: color_out_exact(value.text_color)?,
            attachments: IfccadMultiLeaderTextAttachments {
                left: attachment_out(value.text_left_attachment as i16)?,
                right: attachment_out(value.text_right_attachment as i16)?,
                top: attachment_out(value.text_top_attachment as i16)?,
                bottom: attachment_out(value.text_bottom_attachment as i16)?,
            },
            attachment_point: point_out(value.text_attachment_point as i16)?,
            alignment: align_out(value.text_alignment as i16)?,
            background: IfccadMultiLeaderTextBackground {
                enabled: value.background_fill_enabled,
                mask_enabled: value.background_mask_fill_on,
                color: color_out_exact(value.background_fill_color)?,
                scale_factor: value.background_scale_factor,
                opacity,
            },
        },
    })
}
fn text_in(
    value: &IfccadMultiLeaderTextContent,
    target: &mut cad::MultiLeaderAnnotContext,
    names: &Names,
    losses: &mut Losses,
) -> Result<(), Error> {
    let Some(EntityType::MText(m)) = text::mtext_in(&value.text, names, losses) else {
        return Err(annotation::invalid(
            "unrepresentable native owned MLeader text",
        ));
    };
    target.has_text_contents = true;
    target.text_string = m.value;
    target.text_location = m.insertion_point;
    target.text_normal = m.normal;
    target.text_direction = vector(value.text.placement.x_axis);
    target.text_rotation = value.state.stored_rotation.unwrap_or(m.rotation);
    target.text_height = m.height;
    target.text_width = m.rectangle_width;
    target.text_style_handle = Some(
        *names
            .style_handles
            .get(&value.text.style_id)
            .ok_or_else(|| annotation::invalid("missing MLeader text style"))?,
    );
    target.line_spacing_factor = m.line_spacing_factor;
    target.line_spacing_style = m.line_spacing_style;
    target.text_flow_direction = match m.drawing_direction {
        acadrust::entities::DrawingDirection::LeftToRight => cad::FlowDirectionType::Horizontal,
        acadrust::entities::DrawingDirection::TopToBottom => cad::FlowDirectionType::Vertical,
        acadrust::entities::DrawingDirection::ByStyle => cad::FlowDirectionType::ByStyle,
    };
    target.column_type = m.column_data.column_type;
    target.column_width = m.column_data.width;
    target.column_gutter = m.column_data.gutter;
    target.column_flow_reversed = m.column_data.flow_reversed;
    target.column_sizes = m.column_data.heights;
    target.text_boundary_height = value.state.boundary_height;
    target.text_height_automatic = value.state.automatic_height;
    target.word_break = value.state.word_break;
    target.text_color = color_in_exact(&value.state.color)?;
    target.text_attachment_point = point_in(value.state.attachment_point).into();
    target.text_alignment = align_in(value.state.alignment).into();
    target.text_left_attachment = attachment_in(value.state.attachments.left).into();
    target.text_right_attachment = attachment_in(value.state.attachments.right).into();
    target.text_top_attachment = attachment_in(value.state.attachments.top).into();
    target.text_bottom_attachment = attachment_in(value.state.attachments.bottom).into();
    target.background_fill_enabled = value.state.background.enabled;
    target.background_mask_fill_on = value.state.background.mask_enabled;
    target.background_fill_color = color_in_exact(&value.state.background.color)?;
    target.background_scale_factor = value.state.background.scale_factor;
    target.background_transparency = match value.state.background.opacity {
        IfccadMode::ByLayer => Transparency::ByLayer,
        IfccadMode::ByBlock => Transparency::ByBlock,
        IfccadMode::Explicit(alpha) => {
            let raw = (alpha * 255.).round() as u8;
            if f64::from(raw) / 255. != alpha {
                return Err(annotation::invalid(
                    "MLeader background opacity is not exactly representable",
                ));
            }
            Transparency::Explicit(255 - raw)
        }
    }
    .to_alpha_value();
    Ok(())
}

macro_rules! flags {
    ($out:ident,$input:ident,$ty:ident,[$($v:ident),+$(,)?])=>{
        fn $out(bits:u32)->Result<BTreeSet<$ty>,Error>{let variants=[$($ty::$v),+];if bits>>variants.len()!=0{return Err(annotation::invalid("unknown MLeader override bits"));}Ok(variants.into_iter().enumerate().filter_map(|(i,v)|(bits&(1<<i)!=0).then_some(v)).collect())}
        fn $input(values:&BTreeSet<$ty>)->u32{let variants=[$($ty::$v),+];variants.into_iter().enumerate().fold(0,|bits,(i,v)|bits|if values.contains(&v){1<<i}else{0})}
    }
}
flags!(
    line_flags_out,
    line_flags_in,
    IfccadMultiLeaderLineProperty,
    [PathKind, Color, LinePattern, LineWeight, ArrowSize, Arrow]
);
flags!(
    flags_out,
    flags_in,
    IfccadMultiLeaderProperty,
    [
        PathKind,
        LineColor,
        LinePattern,
        LineWeight,
        LandingEnabled,
        LandingGap,
        DoglegEnabled,
        DoglegLength,
        Arrow,
        ArrowSize,
        ContentKind,
        TextStyle,
        TextLeftAttachment,
        TextAngle,
        TextAlignment,
        TextColor,
        TextHeight,
        TextFrame,
        UseDefaultMText,
        BlockDefinition,
        BlockColor,
        BlockScale,
        BlockRotation,
        BlockConnection,
        ScaleFactor,
        TextRightAttachment,
        TextSwitchAlignment,
        TextAttachmentDirection,
        TextTopAttachment,
        TextBottomAttachment
    ]
);

pub(super) fn export(writer: &mut Writer, value: &MultiLeader) -> Result<IfccadEntityKind, Error> {
    let common = common_out(writer, value)?;
    let presentation = presentation_out(writer, &value.context, common.content_kind)?;
    if value.text_align_in_ipe != 0 {
        writer.losses.note(format!(
            "MLeader {} unqualified IPE alignment",
            value.common.handle
        ));
    }
    let mut block_labels = Vec::new();
    for (i, label) in value.block_attributes.iter().enumerate() {
        block_labels.push(IfccadMultiLeaderBlockLabel {
            key: i as u64 + 1,
            definition_id: None,
            source_index: Some(i32::from(label.index)),
            width: label.width,
            text: label.text.clone(),
        });
    }
    let mut legacy_arrow_overrides = Vec::new();
    for arrow in &value.arrowhead_overrides {
        let mut lines = presentation
            .groups
            .iter()
            .flat_map(|g| &g.lines)
            .filter(|l| l.source_index == Some(arrow.index));
        let line_key = lines
            .next()
            .ok_or_else(|| annotation::invalid("unbound legacy MLeader arrow index"))?
            .key;
        if lines.next().is_some() {
            return Err(annotation::invalid(
                "ambiguous legacy MLeader arrow index across groups",
            ));
        }
        legacy_arrow_overrides.push(IfccadMultiLeaderLegacyArrowOverride {
            line_key,
            is_default: arrow.is_default,
            arrow: leader::arrow_out(arrow.arrowhead_handle.unwrap_or(Handle::NULL), writer)?,
        });
    }
    Ok(IfccadEntityKind::MultiLeader(Box::new(IfccadMultiLeader {
        style_id: value
            .style_handle
            .filter(|h| !h.is_null())
            .map(|h| {
                writer
                    .multi_leader_styles
                    .get(&h)
                    .copied()
                    .ok_or_else(|| annotation::invalid("unresolved source MLeader style"))
            })
            .transpose()?,
        settings: IfccadMultiLeaderSettings {
            common,
            text_attachment_point: point_out(value.text_attachment_point as i16)?,
            text_direction_negative: value.text_direction_negative,
            extend_leader_to_text: value.extend_leader_to_text,
        },
        overridden_properties: flags_out(value.property_override_flags.bits())?,
        next_label_id: block_labels.len() as u64 + 1,
        block_labels,
        legacy_arrow_overrides,
        presentation,
        annotation: None,
        graphics: None,
    })))
}
pub(super) fn import(
    value: &IfccadMultiLeader,
    names: &Names,
    document: &Document,
    losses: &mut Losses,
) -> Result<EntityType, Error> {
    let mut target = MultiLeader::new();
    target.style_handle = value
        .style_id
        .map(|id| {
            names
                .multi_leader_styles
                .get(&id)
                .copied()
                .ok_or_else(|| annotation::invalid("unresolved native MLeader style"))
        })
        .transpose()?;
    common_in(&value.settings.common, &mut target, names, document)?;
    target.context = presentation_in(&value.presentation, names, document, losses)?;
    target.text_attachment_point = point_in(value.settings.text_attachment_point).into();
    target.text_direction_negative = value.settings.text_direction_negative;
    target.extend_leader_to_text = value.settings.extend_leader_to_text;
    target.property_override_flags = cad::MultiLeaderPropertyOverrideFlags::from_bits_retain(
        flags_in(&value.overridden_properties),
    );
    for arrow in &value.legacy_arrow_overrides {
        let (root_index, line_index) = value
            .presentation
            .groups
            .iter()
            .enumerate()
            .find_map(|(i, g)| {
                g.lines
                    .iter()
                    .position(|l| l.key == arrow.line_key)
                    .map(|j| (i, j))
            })
            .ok_or_else(|| annotation::invalid("missing native legacy arrow line"))?;
        target
            .arrowhead_overrides
            .push(cad::MultiLeaderArrowheadOverride {
                index: target.context.leader_roots[root_index].lines[line_index].index,
                is_default: arrow.is_default,
                arrowhead_handle: Some(leader::arrow_in(arrow.arrow, names, document)?)
                    .filter(|h| !h.is_null()),
            });
    }
    for label in &value.block_labels {
        target.block_attributes.push(cad::BlockAttribute {
            attribute_definition_handle: None,
            index: i16::try_from(label.source_index.unwrap_or(0))
                .map_err(|_| annotation::invalid("MLeader attribute index exceeds CAD range"))?,
            width: label.width,
            text: label.text.clone(),
        });
    }
    Ok(EntityType::MultiLeader(Box::new(target)))
}
