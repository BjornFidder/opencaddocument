//! Proven drawing-variable, Layout XDATA and saved-view/viewport scale backings.
use super::annotation::invalid;
use super::*;
use acadrust::objects::{DictionaryCloningFlags, ObjectType, XRecordEntry};
use acadrust::xdata::{ExtendedDataRecord, XDataValue};
use ifccad::*;
const VIEW_SCALE: &str = "ADSK_XREC_VTR_ANNOSCALE_DATA";
const VIEWPORT_SCALE: &str = "ASDK_XREC_ANNOTATION_SCALE_INFO";
fn variable<'a>(document: &'a Document, key: &str) -> Result<Option<&'a str>, Error> {
    let root = document.header.named_objects_dict_handle;
    if root.is_null() {
        return Ok(None);
    }
    let Some(ObjectType::Dictionary(root)) = document.objects.get(&root) else {
        return Err(invalid("invalid annotation variable root"));
    };
    let Some(handle) = root.get("AcDbVariableDictionary") else {
        return Ok(None);
    };
    let Some(ObjectType::Dictionary(vars)) = document.objects.get(&handle) else {
        return Err(invalid("invalid annotation variable dictionary"));
    };
    let Some(handle) = vars.get(key) else {
        return Ok(None);
    };
    let Some(ObjectType::DictionaryVariable(v)) = document.objects.get(&handle) else {
        return Err(invalid("annotation variable is not a DICTIONARYVAR"));
    };
    Ok(Some(&v.value))
}
fn flag(document: &Document, key: &str) -> Result<Option<bool>, Error> {
    variable(document, key)?
        .map(|v| match v.trim() {
            "0" => Ok(false),
            "1" => Ok(true),
            _ => Err(invalid("unsupported annotation drawing variable value")),
        })
        .transpose()
}
fn set_variable(document: &mut Document, key: &str, value: &str) -> Result<(), Error> {
    if document.set_drawing_variable(key, value) {
        Ok(())
    } else {
        Err(invalid("cannot store annotation drawing variable"))
    }
}
fn visibility(
    document: &Document,
    layout: Handle,
) -> Result<Option<IfccadAnnotationVisibility>, Error> {
    let Some(data) = document.object_xdata.get(&layout) else {
        return Ok(None);
    };
    let Some(record) = data
        .records()
        .iter()
        .find(|r| r.application_name.eq_ignore_ascii_case("AcadAnnoAV"))
    else {
        return Ok(None);
    };
    match record.values.as_slice() {
        [XDataValue::Integer16(0)] => Ok(Some(IfccadAnnotationVisibility::CurrentScaleOnly)),
        [XDataValue::Integer16(1)] => Ok(Some(IfccadAnnotationVisibility::All)),
        _ => Err(invalid("unsupported Layout AcadAnnoAV payload")),
    }
}
fn set_visibility(
    document: &mut Document,
    layout: Handle,
    value: IfccadAnnotationVisibility,
) -> Result<(), Error> {
    if document.app_ids.get("AcadAnnoAV").is_none() {
        let mut app = acadrust::tables::AppId::new("AcadAnnoAV");
        app.handle = document.allocate_handle();
        document.app_ids.add(app)?;
    }
    let mut record = ExtendedDataRecord::new("AcadAnnoAV");
    record.values.push(XDataValue::Integer16(i16::from(
        value == IfccadAnnotationVisibility::All,
    )));
    document
        .object_xdata
        .entry(layout)
        .or_default()
        .upsert_record(record);
    Ok(())
}
fn overall(document: &Document, owner: Handle) -> Result<Option<Handle>, Error> {
    let mut matches = document.entities().filter_map(|e| match e {
        EntityType::Viewport(v) if v.common.owner_handle == owner && v.id == 1 => {
            Some(v.common.handle)
        }
        _ => None,
    });
    let result = matches.next();
    if matches.next().is_some() {
        return Err(invalid(
            "multiple Paper overall viewports for annotation selection",
        ));
    }
    Ok(result)
}
fn set_scale(document: &mut Document, owner: Handle, scale: Handle, view: bool) {
    let key = if view { VIEW_SCALE } else { VIEWPORT_SCALE };
    let record = document.ensure_xrecord(owner, key);
    let Some(ObjectType::XRecord(record)) = document.objects.get_mut(&record) else {
        unreachable!()
    };
    record.cloning_flags = DictionaryCloningFlags::KeepExisting;
    if !view && !record.entries.iter().any(|e| e.code == 90) {
        record.entries.insert(0, XRecordEntry::int32(90, 1));
    }
    record.set_annotation_scale_handle(scale);
}
fn scale(
    document: &Document,
    owner: Handle,
    view: bool,
    losses: &mut Losses,
) -> Result<Option<Handle>, Error> {
    let Some(dictionary) = document.extension_dictionary_handle(owner) else {
        return Ok(None);
    };
    let Some(ObjectType::Dictionary(dictionary)) = document.objects.get(&dictionary) else {
        return Err(invalid("invalid annotation-scale extension dictionary"));
    };
    let keys = if view {
        &[VIEW_SCALE][..]
    } else {
        &[VIEWPORT_SCALE, "ASDK_XREC_ANNO_SCALE_INFO"][..]
    };
    let mut result = None;
    let mut found = false;
    for key in keys {
        let Some(handle) = dictionary.get(key) else {
            continue;
        };
        let Some(ObjectType::XRecord(record)) = document.objects.get(&handle) else {
            return Err(invalid("annotation-scale selection is not an XRECORD"));
        };
        let mut values = record.entries.iter().filter(|e| e.code == 340);
        let value = values
            .next()
            .and_then(|e| e.value.as_handle())
            .ok_or_else(|| invalid("missing annotation-scale selection pointer"))?;
        if values.next().is_some() {
            return Err(invalid("multiple annotation-scale selection pointers"));
        }
        if record
            .entries
            .iter()
            .filter(|e| e.code == 90)
            .any(|e| e.value.as_i32() != Some(1))
        {
            return Err(invalid(
                "unsupported annotation-scale selection record version",
            ));
        }
        if record.entries.iter().any(|e| !matches!(e.code, 90 | 340))
            || record.cloning_flags != DictionaryCloningFlags::KeepExisting
        {
            losses.note(format!(
                "annotation-scale selection {owner} additional XRecord metadata"
            ));
        }
        let value = (!value.is_null()).then_some(value);
        if found && result != value {
            return Err(invalid("conflicting annotation-scale selection backings"));
        };
        found = true;
        result = value;
    }
    Ok(result)
}
fn native_scale(
    writer: &Writer,
    handle: Option<Handle>,
) -> Result<Option<IfccadAnnotationScaleId>, Error> {
    handle
        .map(|h| {
            writer
                .annotation_scales
                .get(&h)
                .copied()
                .ok_or_else(|| invalid("unresolved saved annotation-scale selection"))
        })
        .transpose()
}
fn cad_scale(names: &Names, id: IfccadAnnotationScaleId) -> Result<Handle, Error> {
    names
        .annotation_scales
        .get(&id)
        .copied()
        .ok_or_else(|| invalid("unresolved native annotation-scale selection"))
}
fn state(
    current_scale_id: Option<IfccadAnnotationScaleId>,
    visibility: Option<IfccadAnnotationVisibility>,
) -> Option<IfccadLayoutAnnotationState> {
    (current_scale_id.is_some() || visibility.is_some()).then_some(IfccadLayoutAnnotationState {
        current_scale_id,
        visibility,
    })
}

pub(super) fn import(
    drawing: &IfccadDocument,
    document: &mut Document,
    names: &Names,
    handles: &HashMap<u64, Handle>,
) -> Result<(), Error> {
    if let Some(s) = drawing.annotation_settings {
        if let Some(value) = s.insert_as_annotative {
            set_variable(document, "ANNOTATIVEDWG", if value { "1" } else { "0" })?;
        }
        if let Some(value) = s.model_linetype_scaling {
            set_variable(
                document,
                "MSLTSCALE",
                if value == IfccadModelLinetypeAnnotationMode::ByAnnotationScale {
                    "1"
                } else {
                    "0"
                },
            )?;
        }
    }
    if let Some(s) = drawing.model.annotation_state {
        if let Some(id) = s.current_scale_id {
            let source = drawing
                .annotation_scales
                .iter()
                .find(|s| s.id == id)
                .ok_or_else(|| invalid("unresolved Model annotation scale"))?;
            set_variable(document, "CANNOSCALE", &source.name)?;
            document.header.current_annotation_scale = source.name.clone();
            let ratio = source.paper_units / source.drawing_units;
            if ratio.is_finite() && ratio > 0. {
                document.header.annotation_scale_value = ratio;
            }
        }
        if let Some(value) = s.visibility {
            set_variable(
                document,
                "ANNOALLVISIBLE",
                if value == IfccadAnnotationVisibility::All {
                    "1"
                } else {
                    "0"
                },
            )?;
        }
    }
    for paper in &drawing.paper_layouts {
        if let Some(s) = paper.annotation_state {
            let layout = document
                .objects
                .values()
                .find_map(|o| match o {
                    ObjectType::Layout(l) if l.name == paper.name => {
                        Some((l.handle, l.block_record))
                    }
                    _ => None,
                })
                .ok_or_else(|| invalid("missing Paper annotation layout"))?;
            if let Some(value) = s.visibility {
                set_visibility(document, layout.0, value)?;
            }
            if let Some(id) = s.current_scale_id {
                let viewport = overall(document, layout.1)?.ok_or_else(|| {
                    invalid("missing Paper overall viewport for annotation selection")
                })?;
                set_scale(document, viewport, cad_scale(names, id)?, false);
            }
        }
    }
    for e in drawing
        .paper_layouts
        .iter()
        .flat_map(|p| &p.entities)
        .filter_map(IfccadEntity::as_native)
    {
        if let IfccadEntityKind::Viewport(v) = &e.kind {
            if let Some(id) = v.annotation_scale_id {
                let owner = *handles
                    .get(&e.id)
                    .ok_or_else(|| invalid("missing annotation viewport"))?;
                set_scale(document, owner, cad_scale(names, id)?, false);
            }
        }
    }
    for v in &drawing.annotation_views {
        if let Some(id) = v.annotation_scale_id {
            let owner = *names
                .annotation_views
                .get(&v.id)
                .ok_or_else(|| invalid("missing saved annotation view"))?;
            set_scale(document, owner, cad_scale(names, id)?, true);
        }
    }
    Ok(())
}
pub(super) fn export(writer: &mut Writer) -> Result<(), Error> {
    let document = writer.document;
    writer.drawing.annotation_settings = IfccadDrawingAnnotationSettings {
        insert_as_annotative: flag(document, "ANNOTATIVEDWG")?,
        model_linetype_scaling: flag(document, "MSLTSCALE")?.map(|v| {
            if v {
                IfccadModelLinetypeAnnotationMode::ByAnnotationScale
            } else {
                IfccadModelLinetypeAnnotationMode::Unscaled
            }
        }),
    }
    .into_option();
    // A drawing that names a current scale its scale list lacks (one made
    // without a SCALE object for it) has no current scale to send.
    let current = variable(document, "CANNOSCALE")?.and_then(|name| {
        let id = writer
            .drawing
            .annotation_scales
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.id);
        if id.is_none() {
            writer
                .losses
                .note("a current annotation scale missing from the scale list");
        }
        id
    });
    let global_visibility = flag(document, "ANNOALLVISIBLE")?.map(|v| {
        if v {
            IfccadAnnotationVisibility::All
        } else {
            IfccadAnnotationVisibility::CurrentScaleOnly
        }
    });
    let model_layout = document.objects.values().find_map(|o| match o {
        ObjectType::Layout(l) if l.block_record == document.header.model_space_block_handle => {
            Some(l.handle)
        }
        _ => None,
    });
    let model_visibility = model_layout
        .map(|h| visibility(document, h))
        .transpose()?
        .flatten();
    if global_visibility.is_some()
        && model_visibility.is_some()
        && global_visibility != model_visibility
    {
        return Err(invalid("conflicting Model annotation visibility backings"));
    };
    writer.drawing.model.annotation_state = state(current, model_visibility.or(global_visibility));
    let mut paper_states = HashMap::new();
    for (handle, id) in &writer.layout_ids {
        let Some(ObjectType::Layout(layout)) = document.objects.get(handle) else {
            return Err(invalid("missing Paper Layout selection owner"));
        };
        let selection = overall(document, layout.block_record)?
            .map(|h| scale(document, h, false, &mut writer.losses))
            .transpose()?
            .flatten();
        paper_states.insert(
            *id,
            state(
                native_scale(writer, selection)?,
                visibility(document, *handle)?,
            ),
        );
    }
    for p in &mut writer.drawing.paper_layouts {
        p.annotation_state = paper_states.remove(&p.id).flatten();
    }
    let mut viewport_scales = HashMap::new();
    for (handle, id) in &writer.entity_ids {
        if matches!(document.get_entity(*handle), Some(EntityType::Viewport(_))) {
            let selection = scale(document, *handle, false, &mut writer.losses)?;
            viewport_scales.insert(*id, native_scale(writer, selection)?);
        }
    }
    for e in writer
        .drawing
        .paper_layouts
        .iter_mut()
        .flat_map(|p| &mut p.entities)
        .filter_map(IfccadEntity::as_native_mut)
    {
        if let IfccadEntityKind::Viewport(v) = &mut e.kind {
            v.annotation_scale_id = viewport_scales.remove(&e.id).flatten();
        }
    }
    let mut view_scales = HashMap::new();
    for (handle, id) in &writer.annotation_views {
        let selection = scale(document, *handle, true, &mut writer.losses)?;
        view_scales.insert(*id, native_scale(writer, selection)?);
    }
    for v in &mut writer.drawing.annotation_views {
        v.annotation_scale_id = view_scales.remove(&v.id).flatten();
    }
    Ok(())
}
