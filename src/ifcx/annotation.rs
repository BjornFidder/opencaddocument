//! Drawing-local annotation identities mapped to CAD SCALE objects.
use super::*;
use acadrust::objects::{ObjectType, Scale};
use ifccad::IfccadAnnotationScale;

pub(super) fn read_scales(drawing: &IfccadDocument, document: &mut Document, names: &mut Names) {
    if drawing.annotation_scales.is_empty() {
        return;
    }
    let dictionary = document.ensure_subdictionary("ACAD_SCALELIST");
    for scale in &drawing.annotation_scales {
        let handle = document.allocate_handle();
        let mut converted = Scale::new(&scale.name, scale.paper_units, scale.drawing_units);
        converted.handle = handle;
        converted.owner_handle = dictionary;
        converted.is_unit_scale = scale.unit_scale;
        document
            .objects
            .insert(handle, ObjectType::Scale(converted));
        let Some(ObjectType::Dictionary(d)) = document.objects.get_mut(&dictionary) else {
            unreachable!("ensured scale dictionary")
        };
        d.insert(format!("*A{}", scale.id.0), handle);
        names.annotation_scales.insert(scale.id, handle);
    }
}

pub(super) fn write_scales(writer: &mut Writer) -> Result<(), Error> {
    let mut scales = writer
        .document
        .objects
        .iter()
        .filter_map(|(handle, object)| {
            if let ObjectType::Scale(scale) = object {
                Some((*handle, scale))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    scales.sort_by_key(|(handle, _)| handle.value());
    for (handle, scale) in scales {
        let id = writer.drawing.id_counters.allocate_annotation_scale_id()?;
        writer.annotation_scales.insert(handle, id);
        writer
            .drawing
            .annotation_scales
            .push(IfccadAnnotationScale {
                id,
                name: scale.name.clone(),
                paper_units: scale.paper_units,
                drawing_units: scale.drawing_units,
                unit_scale: scale.is_unit_scale,
            });
        if scale.is_temporary {
            writer.losses.note(format!(
                "annotation scale {} temporary designation",
                scale.name
            ));
        }
    }
    Ok(())
}

use acadrust::objects::{Dictionary, ObjectContextData, ObjectContextKind};
use acadrust::xdata::{ExtendedDataRecord, XDataValue};
use ifccad::{
    IfccadAnnotation, IfccadAnnotationContext, IfccadAnnotationContextKey, IfccadAnnotationMode,
    IfccadTextAnnotationContext, IfccadTextAnnotationHorizontal,
};

pub(super) fn invalid(message: &str) -> Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message).into()
}
pub(super) fn child_dictionary(document: &mut Document, owner: Handle, key: &str) -> Handle {
    if let Some(ObjectType::Dictionary(parent)) = document.objects.get(&owner) {
        if let Some(existing) = parent.get(key) {
            return existing;
        }
    }
    let handle = document.allocate_handle();
    let mut d = Dictionary::new();
    d.handle = handle;
    d.owner = owner;
    d.reactors.push(owner);
    d.hard_owner = true;
    document.objects.insert(handle, ObjectType::Dictionary(d));
    let Some(ObjectType::Dictionary(parent)) = document.objects.get_mut(&owner) else {
        unreachable!("existing parent dictionary")
    };
    parent.insert(key, handle);
    handle
}
pub(super) fn import<P>(
    a: &IfccadAnnotation<P>,
    owner: Handle,
    document: &mut Document,
    names: &Names,
    convert: impl Fn(&P) -> Result<ObjectContextKind, Error>,
) -> Result<(), Error> {
    let enabled = match a.mode {
        IfccadAnnotationMode::Enabled => Some(true),
        IfccadAnnotationMode::Disabled => Some(false),
        IfccadAnnotationMode::ByDefinition => None,
    };
    let flag = enabled.map(definition_flag);
    if a.orientation.is_some() {
        ensure_paper_app(document)?;
    }
    let entity = document
        .get_entity_mut(owner)
        .ok_or_else(|| invalid("missing annotation owner"))?;
    if let Some(flag) = flag {
        entity.common_mut().extended_data.upsert_record(flag);
    }
    if let Some(orientation) = a.orientation {
        entity
            .common_mut()
            .extended_data
            .upsert_record(paper_record(orientation));
    }
    if a.contexts.is_empty() {
        return Ok(());
    }
    let extension = document.ensure_extension_dictionary(owner);
    let manager = child_dictionary(document, extension, "AcDbContextDataManager");
    let collection = child_dictionary(document, manager, "ACDB_ANNOTATIONSCALES");
    for (index, c) in a.contexts.iter().enumerate() {
        let scale = *names
            .annotation_scales
            .get(&c.key.scale_id)
            .ok_or_else(|| invalid("unresolved annotation scale"))?;
        let handle = document.allocate_handle();
        let context = ObjectContextData {
            handle,
            owner_handle: collection,
            reactors: vec![collection],
            xdictionary_handle: None,
            class_version: 4,
            has_file_to_extension_dictionary: true,
            uses_legacy_mleader_context_base: false,
            is_default: a.reference == Some(c.key),
            scale,
            kind: c
                .presentation
                .as_ref()
                .map(&convert)
                .transpose()?
                .unwrap_or(ObjectContextKind::AnnotScale),
        };
        document.register_object_context_class(context.class_name());
        document.context_scales.insert(handle, scale);
        document
            .objects
            .insert(handle, ObjectType::ObjectContextData(context));
        let Some(ObjectType::Dictionary(d)) = document.objects.get_mut(&collection) else {
            unreachable!("new collection")
        };
        d.insert(format!("*A{index}"), handle);
    }
    Ok(())
}
pub(super) fn data_flag(data: &acadrust::xdata::ExtendedData) -> Result<Option<bool>, Error> {
    let Some(record) = data
        .records()
        .iter()
        .find(|r| r.application_name.eq_ignore_ascii_case("AcadAnnotative"))
    else {
        return Ok(None);
    };
    match record.values.as_slice() {
        [XDataValue::String(tag), XDataValue::ControlString(open), XDataValue::Integer16(1), XDataValue::Integer16(value), XDataValue::ControlString(close)]
            if tag == "AnnotativeData" && open == "{" && close == "}" && [0, 1].contains(value) =>
        {
            Ok(Some(*value == 1))
        }
        _ => Err(invalid("unsupported AcadAnnotative flag payload")),
    }
}
pub(super) fn export<P>(
    writer: &Writer,
    entity: &EntityType,
    mut convert: impl FnMut(&ObjectContextKind) -> Result<P, Error>,
) -> Result<Option<Box<IfccadAnnotation<P>>>, Error> {
    let orientation = data_orientation(&entity.common().extended_data)?;
    let flag = data_flag(&entity.common().extended_data)?;
    let flag = match entity {
        EntityType::MText(t) if t.is_annotative => {
            if flag == Some(false) {
                return Err(invalid("conflicting MText annotation flags"));
            }
            Some(true)
        }
        _ => flag,
    };
    let child = |owner: Handle, key: &str| -> Result<Option<Handle>, Error> {
        let Some(ObjectType::Dictionary(d)) = writer.document.objects.get(&owner) else {
            return Err(invalid("unresolved annotation dictionary"));
        };
        Ok(d.get(key))
    };
    let collection = if let Some(extension) = entity.common().xdictionary_handle {
        if let Some(manager) = child(extension, "AcDbContextDataManager")? {
            child(manager, "ACDB_ANNOTATIONSCALES")?
        } else {
            None
        }
    } else {
        None
    };
    if flag.is_none() && collection.is_none() && orientation.is_none() {
        return Ok(None);
    }
    let mut out = IfccadAnnotation {
        orientation,
        mode: if flag.is_none()
            && matches!(entity, EntityType::Insert(_) | EntityType::Dimension(_))
        {
            IfccadAnnotationMode::ByDefinition
        } else if flag.unwrap_or(false) {
            IfccadAnnotationMode::Enabled
        } else {
            IfccadAnnotationMode::Disabled
        },
        reference: None,
        contexts: vec![],
    };
    if let Some(collection) = collection {
        let Some(ObjectType::Dictionary(d)) = writer.document.objects.get(&collection) else {
            return Err(invalid("unresolved annotation collection"));
        };
        for (_, handle) in &d.entries {
            let Some(ObjectType::ObjectContextData(c)) = writer.document.objects.get(handle) else {
                return Err(invalid("unresolved annotation context"));
            };
            let scale_id = *writer
                .annotation_scales
                .get(&c.scale)
                .ok_or_else(|| invalid("unresolved annotation context scale"))?;
            let key = IfccadAnnotationContextKey { scale_id };
            if c.is_default && out.reference.replace(key).is_some() {
                return Err(invalid("multiple default annotation contexts"));
            }
            let presentation = if matches!(c.kind, ObjectContextKind::AnnotScale) {
                None
            } else {
                Some(convert(&c.kind)?)
            };
            out.contexts
                .push(IfccadAnnotationContext { key, presentation });
        }
    }
    if flag.is_none() {
        if out.contexts.is_empty() {
            if orientation.is_none() {
                return Ok(None);
            }
        } else {
            out.mode = IfccadAnnotationMode::Enabled;
        }
    }
    Ok(Some(Box::new(out)))
}
fn horizontal(value: i16) -> Result<IfccadTextAnnotationHorizontal, Error> {
    use IfccadTextAnnotationHorizontal::*;
    match value {
        0 => Ok(Left),
        1 => Ok(Center),
        2 => Ok(Right),
        3 => Ok(Aligned),
        4 => Ok(Middle),
        5 => Ok(Fit),
        _ => Err(invalid("invalid Text annotation horizontal mode")),
    }
}
fn horizontal_code(value: IfccadTextAnnotationHorizontal) -> i16 {
    use IfccadTextAnnotationHorizontal::*;
    match value {
        Left => 0,
        Center => 1,
        Right => 2,
        Aligned => 3,
        Middle => 4,
        Fit => 5,
    }
}

use acadrust::objects::{MTextColumns, MTextContext};
use ifccad::text::MTextAttachment;
use ifccad::{
    IfccadAnnotationColumnKind, IfccadMTextAnnotationColumns, IfccadMTextAnnotationContext,
};
pub(super) fn world(p: [f64; 3], frame: &IfccadPlacement, point: bool) -> Vector3 {
    let x = vector(frame.x_axis);
    let y = vector(frame.y_axis);
    let z = x.cross(&y);
    x * p[0]
        + y * p[1]
        + z * p[2]
        + if point {
            vector(frame.origin)
        } else {
            Vector3::ZERO
        }
}
pub(super) fn local(p: Vector3, frame: &IfccadPlacement, point: bool) -> [f64; 3] {
    let delta = p - if point {
        vector(frame.origin)
    } else {
        Vector3::ZERO
    };
    let x = vector(frame.x_axis);
    let y = vector(frame.y_axis);
    let z = x.cross(&y);
    [delta.dot(&x), delta.dot(&y), delta.dot(&z)]
}
fn attachment(value: i32) -> Result<MTextAttachment, Error> {
    use MTextAttachment::*;
    match value {
        1 => Ok(TopLeft),
        2 => Ok(TopCenter),
        3 => Ok(TopRight),
        4 => Ok(MiddleLeft),
        5 => Ok(MiddleCenter),
        6 => Ok(MiddleRight),
        7 => Ok(BottomLeft),
        8 => Ok(BottomCenter),
        9 => Ok(BottomRight),
        _ => Err(invalid("invalid MText annotation attachment")),
    }
}
fn attachment_code(value: MTextAttachment) -> i32 {
    use MTextAttachment::*;
    match value {
        TopLeft => 1,
        TopCenter => 2,
        TopRight => 3,
        MiddleLeft => 4,
        MiddleCenter => 5,
        MiddleRight => 6,
        BottomLeft => 7,
        BottomCenter => 8,
        BottomRight => 9,
    }
}
pub(super) fn import_entity(
    kind: &IfccadEntityKind,
    handle: Handle,
    document: &mut Document,
    names: &Names,
    losses: &mut Losses,
) -> Result<(), Error> {
    match kind {
        IfccadEntityKind::BlockInstance {
            definition_id,
            transform,
            annotation: Some(a),
        } => {
            let mut a = a.as_ref().clone();
            if a.mode == IfccadAnnotationMode::ByDefinition && !a.contexts.is_empty() {
                let name = names
                    .blocks
                    .get(definition_id)
                    .ok_or_else(|| invalid("missing annotation block definition"))?;
                let record = document
                    .block_records
                    .get(name)
                    .ok_or_else(|| invalid("missing annotation block record"))?;
                let (record_handle, legacy) = (record.handle, record.block_entity_handle);
                let enabled =
                    export_definition(document, record_handle, Some(legacy), None, losses)?.enabled;
                a.mode = if enabled {
                    IfccadAnnotationMode::Enabled
                } else {
                    IfccadAnnotationMode::Disabled
                };
                losses.note("Block annotation inheritance with memberships materialized to its current effective CAD object flag");
            }
            let (_, base_normal, _) = placement_in(&transform.placement);
            import(&a, handle, document, names, |p| {
                let (_, normal, frame) = placement_in(&p.transform.placement);
                if normal != base_normal {
                    return Err(invalid(
                        "CAD block annotation context cannot store a separate extrusion plane",
                    ));
                }
                Ok(ObjectContextKind::BlkRef {
                    rotation: p.transform.rotation + frame,
                    insertion: vector(p.transform.placement.origin),
                    scale_factor: vector(p.transform.scale),
                })
            })?;
        }
        IfccadEntityKind::Text(t) => {
            if let Some(a) = &t.annotation {
                let (origin, _, rotation) = placement_in(&t.placement);
                import(a, handle, document, names, |p| {
                    Ok(ObjectContextKind::Text {
                        horizontal_mode: horizontal_code(p.horizontal),
                        rotation: p.rotation + rotation,
                        insertion: frame_to_ocs(p.insertion[0], p.insertion[1], origin, rotation),
                        alignment: frame_to_ocs(p.alignment[0], p.alignment[1], origin, rotation),
                    })
                })?;
            }
        }
        IfccadEntityKind::MText(t) => {
            if let Some(a) = &t.annotation {
                for c in &a.contexts {
                    if let Some(p) = &c.presentation {
                        if let Some(columns) = &p.columns {
                            if (columns.kind != IfccadAnnotationColumnKind::Dynamic
                                || columns.auto_height)
                                && !columns.heights.is_empty()
                            {
                                losses.note("dormant MText annotation column heights have no CAD file field");
                            }
                        }
                    }
                }
                import(a, handle, document, names, |p| {
                    let columns = p
                        .columns
                        .as_ref()
                        .map(|c| {
                            Ok::<_, Error>(MTextColumns {
                                num_heights: i32::try_from(c.count).map_err(|_| {
                                    invalid("MText annotation column count exceeds CAD range")
                                })?,
                                width: c.width,
                                gutter: c.gutter,
                                auto_height: c.auto_height,
                                flow_reversed: c.flow_reversed,
                                heights: c.heights.clone(),
                            })
                        })
                        .transpose()?;
                    Ok(ObjectContextKind::MText(MTextContext {
                        attachment: attachment_code(p.attachment),
                        x_axis_dir: world(p.x_direction, &t.placement, false),
                        insertion: world(p.insertion, &t.placement, true),
                        rect_width: p.rectangle_width,
                        rect_height: p.rectangle_height,
                        extents_width: 0.,
                        extents_height: 0.,
                        column_type: match p.columns.as_ref().map(|c| c.kind) {
                            None => 0,
                            Some(IfccadAnnotationColumnKind::Static) => 1,
                            Some(IfccadAnnotationColumnKind::Dynamic) => 2,
                        },
                        columns,
                    }))
                })?;
                if let Some(EntityType::MText(target)) = document.get_entity_mut(handle) {
                    target.is_annotative = a.mode == IfccadAnnotationMode::Enabled;
                }
            }
        }
        _ => (),
    }
    Ok(())
}
pub(super) fn export_entity(
    writer: &mut Writer,
    entity: &EntityType,
    kind: &mut IfccadEntityKind,
) -> Result<(), Error> {
    match kind {
        IfccadEntityKind::BlockInstance { annotation, .. } => {
            let EntityType::Insert(base) = entity else {
                return Err(invalid("annotation block owner must be INSERT"));
            };
            *annotation = export(writer, entity, |p| {
                let ObjectContextKind::BlkRef {
                    rotation,
                    insertion,
                    scale_factor,
                } = p
                else {
                    return Err(invalid("annotation context does not match block owner"));
                };
                Ok(ifccad::IfccadBlockAnnotationContext {
                    transform: IfccadBlockTransform {
                        placement: placement_out(to_ocs(*insertion, base.normal), base.normal, 0.),
                        rotation: *rotation,
                        scale: [scale_factor.x, scale_factor.y, scale_factor.z],
                    },
                })
            })?;
        }
        IfccadEntityKind::Text(t) => {
            let (origin, _, rotation) = placement_in(&t.placement);
            let local = |point: Vector2| {
                let x = point.x - origin.x;
                let y = point.y - origin.y;
                let (s, c) = rotation.sin_cos();
                [x * c + y * s, -x * s + y * c]
            };
            t.annotation = export(writer, entity, |p| {
                if let ObjectContextKind::Text {
                    horizontal_mode,
                    rotation: r,
                    insertion,
                    alignment,
                } = p
                {
                    Ok(IfccadTextAnnotationContext {
                        insertion: local(*insertion),
                        alignment: local(*alignment),
                        rotation: r - rotation,
                        horizontal: horizontal(*horizontal_mode)?,
                    })
                } else {
                    Err(invalid("annotation context does not match Text owner"))
                }
            })?;
        }
        IfccadEntityKind::MText(t) => {
            let mut omitted_extents = false;
            t.annotation = export(writer, entity, |p| {
                let ObjectContextKind::MText(p) = p else {
                    return Err(invalid("annotation context does not match MText owner"));
                };
                omitted_extents |= p.extents_width != 0. || p.extents_height != 0.;
                let columns = match (p.column_type, p.columns.as_ref()) {
                    (0, None) => None,
                    (kind @ 1..=2, Some(c)) => Some(IfccadMTextAnnotationColumns {
                        kind: if kind == 1 {
                            IfccadAnnotationColumnKind::Static
                        } else {
                            IfccadAnnotationColumnKind::Dynamic
                        },
                        count: u32::try_from(c.num_heights)
                            .map_err(|_| invalid("negative MText annotation column count"))?,
                        width: c.width,
                        gutter: c.gutter,
                        auto_height: c.auto_height,
                        flow_reversed: c.flow_reversed,
                        heights: c.heights.clone(),
                    }),
                    _ => return Err(invalid("inconsistent MText annotation columns")),
                };
                Ok(IfccadMTextAnnotationContext {
                    insertion: local(p.insertion, &t.placement, true),
                    x_direction: local(p.x_axis_dir, &t.placement, false),
                    attachment: attachment(p.attachment)?,
                    rectangle_width: p.rect_width,
                    rectangle_height: p.rect_height,
                    columns,
                })
            })?;
            if omitted_extents {
                writer.losses.note("MText annotation layout extents cache omitted; saved rectangle and column semantics retained");
            }
        }
        _ => (),
    }
    Ok(())
}

fn definition_flag(enabled: bool) -> ExtendedDataRecord {
    let mut r = ExtendedDataRecord::new("AcadAnnotative");
    r.values = vec![
        XDataValue::String("AnnotativeData".into()),
        XDataValue::ControlString("{".into()),
        XDataValue::Integer16(1),
        XDataValue::Integer16(i16::from(enabled)),
        XDataValue::ControlString("}".into()),
    ];
    r
}

fn data_orientation(
    data: &acadrust::xdata::ExtendedData,
) -> Result<Option<ifccad::IfccadAnnotationOrientation>, Error> {
    let Some(r) = data
        .records()
        .iter()
        .find(|r| r.application_name.eq_ignore_ascii_case("AcadAnnoPO"))
    else {
        return Ok(None);
    };
    match r.values.as_slice() {
        [XDataValue::Integer16(0)] => Ok(Some(ifccad::IfccadAnnotationOrientation::Fixed)),
        [XDataValue::Integer16(1)] => Ok(Some(ifccad::IfccadAnnotationOrientation::MatchLayout)),
        _ => Err(invalid("unsupported AcadAnnoPO orientation payload")),
    }
}
fn paper_record(orientation: ifccad::IfccadAnnotationOrientation) -> ExtendedDataRecord {
    let mut r = ExtendedDataRecord::new("AcadAnnoPO");
    r.values.push(XDataValue::Integer16(i16::from(
        orientation == ifccad::IfccadAnnotationOrientation::MatchLayout,
    )));
    r
}
fn ensure_paper_app(document: &mut Document) -> Result<(), Error> {
    if document.app_ids.get("AcadAnnoPO").is_none() {
        let mut app = acadrust::tables::AppId::new("AcadAnnoPO");
        app.handle = document.allocate_handle();
        document.app_ids.add(app)?;
    }
    Ok(())
}
pub(super) fn import_orientation(
    orientation: ifccad::IfccadAnnotationOrientation,
    handle: Handle,
    document: &mut Document,
) -> Result<(), Error> {
    ensure_paper_app(document)?;
    document.set_object_xdata_record(handle, paper_record(orientation));
    Ok(())
}
pub(super) fn import_definition(
    policy: ifccad::IfccadAnnotationDefinition,
    handle: Handle,
    document: &mut Document,
    _losses: &mut Losses,
) -> Result<(), Error> {
    if policy.enabled {
        document.set_object_xdata_record(handle, definition_flag(true));
    }
    if policy.orientation == ifccad::IfccadAnnotationOrientation::MatchLayout {
        import_orientation(policy.orientation, handle, document)?;
    }
    Ok(())
}
pub(super) fn export_definition(
    document: &Document,
    handle: Handle,
    legacy: Option<Handle>,
    enabled: Option<bool>,
    _losses: &mut Losses,
) -> Result<ifccad::IfccadAnnotationDefinition, Error> {
    let object = document.object_xdata(handle);
    let legacy_data = legacy
        .and_then(|h| document.get_entity(h))
        .map(|e| &e.common().extended_data);
    let explicit = object.map(data_flag).transpose()?.flatten();
    let legacy = legacy_data.map(data_flag).transpose()?.flatten();
    if explicit.zip(legacy).is_some_and(|(a, b)| a != b) {
        return Err(invalid("conflicting block definition annotation flags"));
    }
    let orientation = object.map(data_orientation).transpose()?.flatten();
    let legacy_orientation = legacy_data.map(data_orientation).transpose()?.flatten();
    if orientation
        .zip(legacy_orientation)
        .is_some_and(|(a, b)| a != b)
    {
        return Err(invalid("conflicting block definition paper orientation"));
    }
    Ok(ifccad::IfccadAnnotationDefinition {
        enabled: enabled.or(explicit).or(legacy).unwrap_or(false),
        orientation: orientation
            .or(legacy_orientation)
            .unwrap_or(ifccad::IfccadAnnotationOrientation::Fixed),
    })
}
