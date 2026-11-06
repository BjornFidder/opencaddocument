//! Style resource registration shares the entity's common setting mapping.
use super::*;
use acadrust::objects::{MultiLeaderStyle, ObjectType};

fn source_common(source: &MultiLeaderStyle) -> MultiLeader {
    let mut v = MultiLeader::new();
    v.path_type = (source.path_type as i16).into();
    v.line_color = source.line_color;
    v.line_type_handle = source.line_type_handle;
    v.line_weight = source.line_weight;
    v.enable_landing = source.enable_landing;
    v.enable_dogleg = source.enable_dogleg;
    v.dogleg_length = source.landing_distance;
    v.arrowhead_handle = source.arrowhead_handle;
    v.arrowhead_size = source.arrowhead_size;
    v.content_type = (source.content_type as i16).into();
    v.text_style_handle = source.text_style_handle;
    v.text_color = source.text_color;
    v.text_height = source.text_height;
    v.text_frame = source.text_frame;
    v.text_left_attachment = (source.text_left_attachment as i16).into();
    v.text_right_attachment = (source.text_right_attachment as i16).into();
    v.text_top_attachment = (source.text_top_attachment as i16).into();
    v.text_bottom_attachment = (source.text_bottom_attachment as i16).into();
    v.text_attachment_direction = (source.text_attachment_direction as i16).into();
    v.text_angle_type = (source.text_angle_type as i16).into();
    v.text_alignment = (source.text_alignment as i16).into();
    v.block_content_handle = source.block_content_handle;
    v.block_content_color = source.block_content_color;
    v.block_connection_type = (source.block_content_connection as i16).into();
    v.block_rotation = source.block_content_rotation;
    v.block_scale = Vector3::new(
        source.block_content_scale_x,
        source.block_content_scale_y,
        source.block_content_scale_z,
    );
    v.scale_factor = source.scale_factor;
    v
}
pub(in crate::ifcx) fn export(writer: &mut Writer) -> Result<(), Error> {
    let mut sources: Vec<_> = writer
        .document
        .objects
        .values()
        .filter_map(|o| match o {
            ObjectType::MultiLeaderStyle(s) => Some(s),
            _ => None,
        })
        .collect();
    sources.sort_by(|a, b| a.name.cmp(&b.name).then(a.handle.cmp(&b.handle)));
    for source in sources {
        let id = writer
            .drawing
            .id_counters
            .allocate_multi_leader_style_id()?;
        writer.multi_leader_styles.insert(source.handle, id);
        let mut style = IfccadMultiLeaderStyle::new(id, source.name.clone());
        style.common = common_out(writer, &source_common(source))?;
        style.description = source.description.clone();
        style.landing_gap = source.landing_gap;
        style.default_m_text.content = text::picture_mtext_content(&source.default_text)
            .ok_or_else(|| {
                annotation::invalid("MLeader style default text contains unsupported formatting")
            })?;
        style.text_always_left = source.text_always_left;
        style.block_scale_enabled = source.enable_block_scale;
        style.block_rotation_enabled = source.enable_block_rotation;
        style.align_space = source.align_space;
        style.break_gap_size = source.break_gap_size;
        style.max_leader_points = u32::try_from(source.max_leader_points)
            .map_err(|_| annotation::invalid("negative MLeader style point limit"))?;
        style.first_segment_angle = source.first_segment_angle;
        style.second_segment_angle = source.second_segment_angle;
        style.leader_draw_order = match source.leader_draw_order as i16 {
            0 => IfccadMultiLeaderDrawOrder::HeadFirst,
            1 => IfccadMultiLeaderDrawOrder::TailFirst,
            _ => {
                return Err(annotation::invalid(
                    "unknown MLeader style leader draw order",
                ))
            }
        };
        style.content_draw_order = match source.multileader_draw_order as i16 {
            0 => IfccadMultiLeaderContentDrawOrder::ContentFirst,
            1 => IfccadMultiLeaderContentDrawOrder::LeadersFirst,
            _ => {
                return Err(annotation::invalid(
                    "unknown MLeader style content draw order",
                ))
            }
        };
        style.annotative = source.is_annotative;
        if source.property_changed || source.unknown_flag_298 {
            writer.losses.note(format!(
                "MLeader style {} unqualified CAD bookkeeping flags",
                source.name
            ));
        }
        writer.drawing.multi_leader_styles.push(style);
    }
    Ok(())
}
pub(in crate::ifcx) fn import(
    drawing: &IfccadDocument,
    document: &mut Document,
    names: &mut Names,
) -> Result<(), Error> {
    let owner = document.ensure_named_objects_dictionary();
    let dictionary = annotation::child_dictionary(document, owner, "ACAD_MLEADERSTYLE");
    for source in &drawing.multi_leader_styles {
        let mut common = MultiLeader::new();
        common_in(&source.common, &mut common, names, document)?;
        let existing = match document.objects.get(&dictionary) {
            Some(ObjectType::Dictionary(d)) => d.get(&source.name),
            _ => None,
        };
        let handle = existing.unwrap_or_else(|| document.allocate_handle());
        let mut style = MultiLeaderStyle::new(&source.name);
        style.handle = handle;
        style.owner_handle = dictionary;
        style.description = source.description.clone();
        style.path_type = (common.path_type as i16).into();
        style.line_color = common.line_color;
        style.line_type_handle = common.line_type_handle;
        style.line_weight = common.line_weight;
        style.enable_landing = common.enable_landing;
        style.enable_dogleg = common.enable_dogleg;
        style.landing_distance = common.dogleg_length;
        style.landing_gap = source.landing_gap;
        style.arrowhead_handle = common.arrowhead_handle;
        style.arrowhead_size = common.arrowhead_size;
        style.content_type = (common.content_type as i16).into();
        style.text_style_handle = common.text_style_handle;
        style.text_color = common.text_color;
        style.text_height = common.text_height;
        style.text_frame = common.text_frame;
        style.text_left_attachment = (common.text_left_attachment as i16).into();
        style.text_right_attachment = (common.text_right_attachment as i16).into();
        style.text_top_attachment = (common.text_top_attachment as i16).into();
        style.text_bottom_attachment = (common.text_bottom_attachment as i16).into();
        style.text_attachment_direction = (common.text_attachment_direction as i16).into();
        style.text_angle_type = (common.text_angle_type as i16).into();
        style.text_alignment = (common.text_alignment as i16).into();
        style.block_content_handle = common.block_content_handle;
        style.block_content_color = common.block_content_color;
        style.block_content_connection = (common.block_connection_type as i16).into();
        style.block_content_rotation = common.block_rotation;
        style.block_content_scale_x = common.block_scale.x;
        style.block_content_scale_y = common.block_scale.y;
        style.block_content_scale_z = common.block_scale.z;
        style.scale_factor = common.scale_factor;
        style.default_text = text::emit_mtext(
            &source.default_m_text.character_format,
            &source.default_m_text.paragraph_format,
            &source.default_m_text.content,
        );
        style.text_always_left = source.text_always_left;
        style.enable_block_scale = source.block_scale_enabled;
        style.enable_block_rotation = source.block_rotation_enabled;
        style.align_space = source.align_space;
        style.break_gap_size = source.break_gap_size;
        style.max_leader_points = i32::try_from(source.max_leader_points)
            .map_err(|_| annotation::invalid("MLeader style point limit exceeds CAD range"))?;
        style.first_segment_angle = source.first_segment_angle;
        style.second_segment_angle = source.second_segment_angle;
        style.leader_draw_order = match source.leader_draw_order {
            IfccadMultiLeaderDrawOrder::HeadFirst => 0,
            IfccadMultiLeaderDrawOrder::TailFirst => 1,
        }
        .into();
        style.multileader_draw_order = match source.content_draw_order {
            IfccadMultiLeaderContentDrawOrder::ContentFirst => 0,
            IfccadMultiLeaderContentDrawOrder::LeadersFirst => 1,
        }
        .into();
        style.is_annotative = source.annotative;
        document
            .objects
            .insert(handle, ObjectType::MultiLeaderStyle(style));
        if let Some(ObjectType::Dictionary(d)) = document.objects.get_mut(&dictionary) {
            d.insert(&source.name, handle);
        }
        names.multi_leader_styles.insert(source.id, handle);
    }
    Ok(())
}
