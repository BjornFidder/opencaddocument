//! ATTDEF and INSERT children share ordinary text conversion and independent identity.
use super::*;
use acadrust::entities::{
    AttributeDefinition, AttributeEntity, AttributeFlags, HorizontalAlignment as AH, MTextFlag,
    VerticalAlignment as AV,
};
use ifccad::{
    IfccadAttributeDefinition, IfccadAttributeFlags, IfccadAttributeMText, IfccadAttributeValue,
    IfccadBlockAttribute,
};

fn h_out(h: AH) -> TextHorizontalAlignment {
    match h {
        AH::Left => TextHorizontalAlignment::Left,
        AH::Center => TextHorizontalAlignment::Center,
        AH::Right => TextHorizontalAlignment::Right,
        AH::Aligned => TextHorizontalAlignment::Aligned,
        AH::Middle => TextHorizontalAlignment::Middle,
        AH::Fit => TextHorizontalAlignment::Fit,
    }
}
fn v_out(v: AV) -> TextVerticalAlignment {
    match v {
        AV::Baseline => TextVerticalAlignment::Baseline,
        AV::Bottom => TextVerticalAlignment::Bottom,
        AV::Middle => TextVerticalAlignment::Middle,
        AV::Top => TextVerticalAlignment::Top,
    }
}
fn h_in(h: TextHorizontalAlignment) -> AH {
    match h {
        TextHorizontalAlignment::Left => AH::Left,
        TextHorizontalAlignment::Center => AH::Center,
        TextHorizontalAlignment::Right => AH::Right,
        TextHorizontalAlignment::Aligned => AH::Aligned,
        TextHorizontalAlignment::Middle => AH::Middle,
        TextHorizontalAlignment::Fit => AH::Fit,
    }
}
fn v_in(v: TextVerticalAlignment) -> AV {
    match v {
        TextVerticalAlignment::Baseline => AV::Baseline,
        TextVerticalAlignment::Bottom => AV::Bottom,
        TextVerticalAlignment::Middle => AV::Middle,
        TextVerticalAlignment::Top => AV::Top,
    }
}
fn flags_out(f: AttributeFlags) -> IfccadAttributeFlags {
    IfccadAttributeFlags {
        invisible: f.invisible,
        constant: f.constant,
        verify: f.verify,
        preset: f.preset,
    }
}
fn flags_in(f: IfccadAttributeFlags) -> AttributeFlags {
    AttributeFlags {
        invisible: f.invisible,
        constant: f.constant,
        verify: f.verify,
        preset: f.preset,
        ..Default::default()
    }
}

fn outer_text(a: &AttributeEntity) -> Text {
    Text {
        common: a.common.clone(),
        value: a.value.clone(),
        insertion_point: a.insertion_point,
        alignment_point: Some(a.alignment_point),
        height: a.height,
        rotation: a.rotation,
        width_factor: a.width_factor,
        oblique_angle: a.oblique_angle,
        style: a.text_style.clone(),
        horizontal_alignment: h_out(a.horizontal_alignment),
        vertical_alignment: v_out(a.vertical_alignment),
        normal: a.normal,
        // The drawing model keeps no thickness on attributes.
        thickness: 0.0,
        generation_flags: a.text_generation_flags,
    }
}
fn value_out(
    w: &mut Writer<'_>,
    a: &AttributeEntity,
    definition: bool,
) -> Result<Option<IfccadAttributeValue>, Error> {
    let Some(IfccadEntityKind::Text(t)) = w.kind(&EntityType::Text(outer_text(a)))? else {
        return Ok(None);
    };
    let m_text = if let Some(m) = &a.embedded_mtext {
        let before = w.losses.content_snapshot();
        if text::picture_mtext_content(&m.value).is_none() {
            w.losses
                .note("attribute MText uses unsupported formatting; entire attribute omitted");
            return Ok(None);
        }
        let Some(IfccadEntityKind::MText(converted)) = text::mtext_out(w, m) else {
            return Ok(None);
        };
        if before != w.losses.content_snapshot() {
            w.losses.note("attribute MText cannot be represented without changing active content; entire attribute omitted");
            return Ok(None);
        }
        let Some(layer_id) = w.layers.get(&m.common.layer.to_ascii_lowercase()).copied() else {
            return Err(annotation::invalid("unresolved embedded attribute layer"));
        };
        Some(Box::new(IfccadAttributeMText {
            visible: !m.common.invisible,
            layer_id,
            appearance: appearance_out(&m.common, &w.line_patterns, &mut w.losses),
            line_pattern_scale: m.common.linetype_scale,
            text: *converted,
        }))
    } else if a.is_multiline || a.mtext_flag != MTextFlag::SingleLine {
        w.losses
            .note("multiline attribute without embedded MText; entire attribute omitted");
        return Ok(None);
    } else {
        None
    };
    if a.flags.locked_position {
        w.losses
            .note("nonstandard attribute bit-16 lock omitted; dedicated position lock retained");
    }
    if a.flags.annotative {
        w.losses
            .note("attribute bit-128 flag is not a saved annotation context; bit omitted");
    }
    let annotation = super::annotation_attributes::export(
        w,
        &EntityType::AttributeEntity(a.clone()),
        &t,
        m_text.as_deref(),
    )?;
    let fields = super::attribute_fields::capture(w, a.common.handle, definition)?;
    Ok(Some(IfccadAttributeValue {
        tag: a.tag.clone(),
        definition_id: None,
        flags: flags_out(a.flags),
        lock_position: a.lock_position,
        field_length: i32::from(a.field_length),
        text: t,
        m_text,
        annotation,
        secondary_attributes: vec![],
        fields,
    }))
}
pub(super) fn definition_out(
    w: &mut Writer<'_>,
    d: &AttributeDefinition,
) -> Result<Option<IfccadAttributeDefinition>, Error> {
    let mut a = AttributeEntity::from_definition(d, None);
    a.common = d.common.clone();
    // from_definition intentionally changes some authoring defaults; conversion retains the source fields.
    a.value = d.default_value.clone();
    a.attdef_handle = Handle::NULL;
    let Some(v) = value_out(w, &a, true)? else {
        return Ok(None);
    };
    Ok(Some(IfccadAttributeDefinition {
        tag: v.tag,
        prompt: d.prompt.clone(),
        flags: v.flags,
        lock_position: v.lock_position,
        field_length: v.field_length,
        text: v.text,
        m_text: v.m_text,
        annotation: v.annotation,
        secondary_attributes: v.secondary_attributes,
        fields: v.fields,
    }))
}
pub(super) fn children_out(
    w: &mut Writer<'_>,
    i: &Insert,
) -> Result<Vec<IfccadBlockAttribute>, Error> {
    let mut result = vec![];
    for a in &i.attributes {
        let before = w.losses.content_snapshot();
        let Some(value) = value_out(w, a, false)? else {
            w.losses.note(format!(
                "INSERT {} attribute {} omitted",
                i.common.handle, a.tag
            ));
            continue;
        };
        let layer_id = *w
            .layers
            .get(&a.common.layer.to_ascii_lowercase())
            .ok_or_else(|| annotation::invalid("unresolved attribute layer"))?;
        let id = w.drawing.id_counters.allocate_entity_id()?;
        if !a.common.handle.is_null() {
            w.entity_ids.insert(a.common.handle, id);
        }
        note_entity_plot_style_loss(
            w.document,
            &mut w.losses,
            &a.common,
            format_args!("INSERT {} attribute {}", i.common.handle, a.tag),
        );
        result.push(IfccadBlockAttribute {
            id,
            visible: !a.common.invisible,
            layer_id,
            appearance: appearance_out(&a.common, &w.line_patterns, &mut w.losses),
            line_pattern_scale: a.common.linetype_scale,
            value,
        });
        w.losses.locate_new_content(
            &before,
            &format!("INSERT {} attribute {}", i.common.handle, a.tag),
        );
    }
    Ok(result)
}
fn value_in(v: &IfccadAttributeValue, n: &Names, l: &mut Losses) -> Option<AttributeEntity> {
    let EntityType::Text(t) = entity_in(&IfccadEntityKind::Text(v.text.clone()), n, l)? else {
        return None;
    };
    let field_length = match i16::try_from(v.field_length) {
        Ok(v) => v,
        Err(_) => {
            l.note("attribute field length exceeds CAD signed-16-bit range; attribute omitted");
            return None;
        }
    };
    let mut a = AttributeEntity::simple(&v.tag, t.value);
    a.insertion_point = t.insertion_point;
    a.alignment_point = t.alignment_point.unwrap_or(t.insertion_point);
    a.height = t.height;
    if t.thickness != 0.0 {
        l.note(format!(
            "attribute {} thickness has no place in the drawing model; dropped",
            v.tag
        ));
    }
    a.rotation = t.rotation;
    a.width_factor = t.width_factor;
    a.oblique_angle = t.oblique_angle;
    a.text_style = t.style;
    a.text_generation_flags = t.generation_flags;
    a.horizontal_alignment = h_in(t.horizontal_alignment);
    a.vertical_alignment = v_in(t.vertical_alignment);
    a.normal = t.normal;
    a.flags = flags_in(v.flags);
    a.field_length = field_length;
    a.lock_position = v.lock_position;
    if let Some(m) = &v.m_text {
        let EntityType::MText(mut t) = text::mtext_in(&m.text, n, l)? else {
            return None;
        };
        t.common.layer = n.layers.get(&m.layer_id)?.clone();
        appearance_in(&m.appearance, n, &mut t.common, l);
        t.common.invisible = !m.visible;
        t.common.linetype_scale = m.line_pattern_scale;
        if a.value != t.value {
            l.note(format!("attribute {} outer fallback will be normalized to active embedded MText by physical CAD storage; native keeps both",v.tag));
        }
        a.is_multiline = true;
        a.mtext_flag = MTextFlag::MultiLine;
        a.line_count = t.value.matches("\\P").count() as i16 + 1;
        a.embedded_mtext = Some(t);
    }
    Some(a)
}
pub(super) fn definition_in(
    d: &IfccadAttributeDefinition,
    n: &Names,
    l: &mut Losses,
) -> Option<EntityType> {
    let v = IfccadAttributeValue {
        tag: d.tag.clone(),
        definition_id: None,
        flags: d.flags,
        lock_position: d.lock_position,
        field_length: d.field_length,
        text: d.text.clone(),
        m_text: d.m_text.clone(),
        annotation: d.annotation.clone(),
        secondary_attributes: d.secondary_attributes.clone(),
        fields: d.fields.clone(),
    };
    let a = value_in(&v, n, l)?;
    let mut d = AttributeDefinition::new(a.tag, d.prompt.clone(), a.value);
    d.insertion_point = a.insertion_point;
    d.alignment_point = a.alignment_point;
    d.height = a.height;
    d.rotation = a.rotation;
    d.width_factor = a.width_factor;
    d.oblique_angle = a.oblique_angle;
    d.text_style = a.text_style;
    d.text_generation_flags = a.text_generation_flags;
    d.horizontal_alignment = a.horizontal_alignment;
    d.vertical_alignment = a.vertical_alignment;
    d.normal = a.normal;
    d.flags = a.flags;
    d.field_length = a.field_length;
    d.lock_position = a.lock_position;
    d.is_multiline = a.is_multiline;
    d.mtext_flag = if a.is_multiline {
        MTextFlag::ConstantMultiLine
    } else {
        MTextFlag::SingleLine
    };
    d.line_count = a.line_count;
    d.embedded_mtext = a.embedded_mtext;
    Some(EntityType::AttributeDefinition(d))
}
pub(super) fn children_in(
    doc: &mut Document,
    handle: Handle,
    items: &[IfccadBlockAttribute],
    names: &Names,
    losses: &mut Losses,
    handles: &mut HashMap<u64, Handle>,
) -> Result<(), Error> {
    let mut result = vec![];
    for a in items {
        let Some(mut v) = value_in(&a.value, names, losses) else {
            losses.note(format!("attribute {} omitted", a.id));
            continue;
        };
        v.common.handle = doc.allocate_handle();
        v.common.owner_handle = handle;
        v.common.layer = names
            .layers
            .get(&a.layer_id)
            .cloned()
            .ok_or_else(|| annotation::invalid("attribute layer unavailable"))?;
        appearance_in(&a.appearance, names, &mut v.common, losses);
        if let IfccadMode::Explicit(color) = &a.appearance.color {
            note_aci_fallback(
                losses,
                color,
                v.common.color,
                v.common.handle,
                format_args!("IFCCAD attribute {}", a.id),
            );
        }
        v.common.invisible = !a.visible;
        v.common.linetype_scale = a.line_pattern_scale;
        handles.insert(a.id, v.common.handle);
        result.push(v);
    }
    let Some(EntityType::Insert(i)) = doc.get_entity_mut(handle) else {
        return Err(annotation::invalid("attribute owner is not INSERT"));
    };
    i.attributes = result;
    Ok(())
}
pub(super) fn common_mut(doc: &mut Document, handle: Handle) -> Option<&mut EntityCommon> {
    if doc.get_entity(handle).is_some() {
        return doc.get_entity_mut(handle).map(|e| e.common_mut());
    }
    let parent = doc.entities().find_map(|e| match e {
        EntityType::Insert(i) if i.attributes.iter().any(|a| a.common.handle == handle) => {
            Some(i.common.handle)
        }
        _ => None,
    })?;
    let EntityType::Insert(i) = doc.get_entity_mut(parent)? else {
        return None;
    };
    i.attributes
        .iter_mut()
        .find(|a| a.common.handle == handle)
        .map(|a| &mut a.common)
}
fn field_transport_note(
    fields: Option<&ifccad::IfccadAttributeFieldGraph>,
    id: u64,
    losses: &mut Losses,
) {
    if fields.is_some_and(|g| g.records.iter().any(|r| !r.format.is_empty())) {
        losses.note(format!("attribute {id} field format scalar has no separate AC1032 slot; field code and typed cache formatting are retained"));
    }
}
pub(super) fn finish_import(
    d: &IfccadDocument,
    doc: &mut Document,
    names: &Names,
    handles: &HashMap<u64, Handle>,
    losses: &mut Losses,
) -> Result<(), Error> {
    for e in d
        .model
        .entities
        .iter()
        .chain(d.paper_layouts.iter().flat_map(|p| &p.entities))
        .chain(d.blocks.iter().flat_map(|b| &b.entities))
        .filter_map(|e| e.as_native())
    {
        match &e.kind {
            IfccadEntityKind::AttributeDefinition(a) => {
                field_transport_note(a.fields.as_ref(), e.id, losses);
                if let Some(h) = handles.get(&e.id) {
                    super::annotation_attributes::import(
                        doc,
                        *h,
                        a.annotation.as_deref(),
                        &a.text,
                        a.m_text.as_deref(),
                        names,
                    )?;
                    super::attribute_fields::reconstruct(doc, *h, a.fields.as_ref(), handles)?;
                    if !a.secondary_attributes.is_empty() {
                        losses.note(format!("attribute {} secondary native links have no selected CAD representation",e.id));
                    }
                }
            }
            IfccadEntityKind::BlockInstance { attributes, .. } => {
                for a in attributes {
                    field_transport_note(a.value.fields.as_ref(), a.id, losses);
                    let Some(h) = handles.get(&a.id) else {
                        continue;
                    };
                    if let Some(def) = a.value.definition_id {
                        losses.note(format!("attribute {} explicit definition binding is retained in memory but has no physical DWG/DXF representation",a.id));
                        let target = *handles.get(&def).ok_or_else(|| {
                            annotation::invalid("definition link target missing in CAD output")
                        })?;
                        let parent = handles[&e.id];
                        if let Some(EntityType::Insert(i)) = doc.get_entity_mut(parent) {
                            if let Some(v) = i.attributes.iter_mut().find(|v| v.common.handle == *h)
                            {
                                v.attdef_handle = target;
                            }
                        }
                    }
                    super::annotation_attributes::import(
                        doc,
                        *h,
                        a.value.annotation.as_deref(),
                        &a.value.text,
                        a.value.m_text.as_deref(),
                        names,
                    )?;
                    super::attribute_fields::reconstruct(
                        doc,
                        *h,
                        a.value.fields.as_ref(),
                        handles,
                    )?;
                    if !a.value.secondary_attributes.is_empty() {
                        losses.note(format!("attribute {} secondary native links have no selected CAD representation",a.id));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}
pub(super) fn finish_export(w: &mut Writer<'_>) -> Result<(), Error> {
    let mut links = HashMap::new();
    for e in w.document.entities() {
        if let EntityType::Insert(i) = e {
            for a in &i.attributes {
                if !a.attdef_handle.is_null() {
                    if let Some(id) = w.entity_ids.get(&a.common.handle) {
                        links.insert(*id, a.attdef_handle);
                    }
                }
            }
        }
    }
    let ids = &w.entity_ids;
    for e in w
        .drawing
        .model
        .entities
        .iter_mut()
        .chain(
            w.drawing
                .paper_layouts
                .iter_mut()
                .flat_map(|p| &mut p.entities),
        )
        .chain(w.drawing.blocks.iter_mut().flat_map(|b| &mut b.entities))
        .filter_map(|e| e.as_native_mut())
    {
        match &mut e.kind {
            IfccadEntityKind::AttributeDefinition(a) => {
                super::attribute_fields::bind(&mut a.fields, ids)?
            }
            IfccadEntityKind::BlockInstance { attributes, .. } => {
                for a in attributes {
                    if let Some(h) = links.get(&a.id) {
                        a.value.definition_id = ids.get(h).copied();
                        if a.value.definition_id.is_none() {
                            return Err(annotation::invalid(
                                "explicit attribute definition target not mapped",
                            ));
                        }
                    }
                    super::attribute_fields::bind(&mut a.value.fields, ids)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}
