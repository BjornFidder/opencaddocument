//! Saved Hatch scale and view collections; no cache evaluation or selector merging.
use super::annotation::{child_dictionary, invalid};
use super::*;
use acadrust::entities::hatch::HatchPatternLine;
use acadrust::objects::{
    HatchLoopContext, HatchScaleContext, HatchViewContext, ObjectContextData, ObjectContextKind,
    ObjectType,
};
use ifccad::geometry_kernel::hatch::{HatchDash, HatchLineFamily};
use ifccad::*;

fn flags(c: IfccadHatchLoopClassification) -> i32 {
    [
        c.external,
        c.polyline,
        c.derived,
        c.text_box,
        c.outermost,
        c.not_closed,
        c.self_intersecting,
        c.text_island,
        c.duplicate,
        c.is_annotative,
        c.does_not_support_scale,
        c.force_annotation_visible,
        c.orient_to_paper,
        c.is_annotative_block,
    ]
    .into_iter()
    .enumerate()
    .fold(0, |bits, (i, on)| bits | if on { 1 << i } else { 0 })
}
fn classification(bits: i32) -> Result<IfccadHatchLoopClassification, Error> {
    if bits & !0x3fff != 0 {
        return Err(invalid("unknown Hatch annotation loop classification"));
    }
    let b = |i| bits & (1 << i) != 0;
    Ok(IfccadHatchLoopClassification {
        external: b(0),
        polyline: b(1),
        derived: b(2),
        text_box: b(3),
        outermost: b(4),
        not_closed: b(5),
        self_intersecting: b(6),
        text_island: b(7),
        duplicate: b(8),
        is_annotative: b(9),
        does_not_support_scale: b(10),
        force_annotation_visible: b(11),
        orient_to_paper: b(12),
        is_annotative_block: b(13),
    })
}
fn presentation_in(
    p: &IfccadHatchAnnotationContext,
    placement: &IfccadPlacement,
    handles: &HashMap<u64, Handle>,
) -> Result<HatchScaleContext, Error> {
    let (origin, _, rotation) = placement_in(placement);
    let frame = hatch::Frame2 {
        origin: [origin.x, origin.y],
        rotation,
    };
    let pattern_lines = p
        .pattern
        .families
        .iter()
        .map(|f| {
            let (s, c) = f.angle.sin_cos();
            let [along, across] = f.offset;
            HatchPatternLine {
                angle: f.angle + rotation,
                base_point: frame.point(f.base_point),
                offset: frame.vector([along * c - across * s, along * s + across * c]),
                dash_lengths: f
                    .dashes
                    .iter()
                    .map(|d| match *d {
                        HatchDash::Dot => 0.,
                        HatchDash::Dash { length } => length,
                        HatchDash::Gap { length } => -length,
                    })
                    .collect(),
            }
        })
        .collect();
    let loops =
        p.loops
            .iter()
            .enumerate()
            .map(|(index, l)| match l {
                IfccadHatchContextLoop::Shared {
                    base_loop_index,
                    classification,
                } => {
                    if *base_loop_index as usize != index {
                        return Err(invalid(
                        "CAD Hatch shared context loop must reference its corresponding base loop",
                    ));
                    }
                    Ok(HatchLoopContext {
                        loop_type: flags(*classification),
                        supports_context: true,
                        boundary: None,
                    })
                }
                IfccadHatchContextLoop::Local {
                    classification,
                    boundary,
                } => {
                    let mut path = hatch::boundary_in(&boundary.geometry, &frame);
                    path.boundary_handles = boundary
                        .source_entity_ids
                        .iter()
                        .map(|id| {
                            handles.get(id).copied().ok_or_else(|| {
                                invalid("unresolved Hatch annotation boundary source")
                            })
                        })
                        .collect::<Result<_, _>>()?;
                    Ok(HatchLoopContext {
                        loop_type: flags(*classification),
                        supports_context: false,
                        boundary: Some(path),
                    })
                }
            })
            .collect::<Result<_, _>>()?;
    Ok(HatchScaleContext {
        pattern_lines,
        pattern_scale: p.pattern.scale,
        pattern_base: annotation::world(p.pattern.orientation_reference, placement, false),
        loops,
    })
}
fn presentation_out(
    p: &HatchScaleContext,
    placement: &IfccadPlacement,
    ids: &HashMap<Handle, u64>,
) -> Result<IfccadHatchAnnotationContext, Error> {
    // Exported base Hatch frames are square to OCS, so context OCS XY is local XY.
    let families = p
        .pattern_lines
        .iter()
        .map(|l| {
            let (s, c) = l.angle.sin_cos();
            HatchLineFamily {
                angle: l.angle,
                base_point: [l.base_point.x, l.base_point.y],
                offset: [
                    l.offset.x * c + l.offset.y * s,
                    l.offset.y * c - l.offset.x * s,
                ],
                dashes: l
                    .dash_lengths
                    .iter()
                    .map(|d| {
                        if *d == 0. {
                            HatchDash::Dot
                        } else if *d > 0. {
                            HatchDash::Dash { length: *d }
                        } else {
                            HatchDash::Gap { length: -*d }
                        }
                    })
                    .collect(),
            }
        })
        .collect();
    let loops = p
        .loops
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let classification = classification(l.loop_type)?;
            match (l.supports_context, l.boundary.as_ref()) {
                (true, None) => Ok(IfccadHatchContextLoop::Shared {
                    base_loop_index: u32::try_from(i)
                        .map_err(|_| invalid("Hatch annotation loop index exceeds native range"))?,
                    classification,
                }),
                (false, Some(path)) => Ok(IfccadHatchContextLoop::Local {
                    classification,
                    boundary: IfccadHatchContextBoundary {
                        geometry: hatch::boundary_out(path)
                            .ok_or_else(|| invalid("unsupported Hatch annotation boundary"))?,
                        source_entity_ids: path
                            .boundary_handles
                            .iter()
                            .map(|h| {
                                ids.get(h).copied().ok_or_else(|| {
                                    invalid("unresolved Hatch annotation boundary source")
                                })
                            })
                            .collect::<Result<_, _>>()?,
                    },
                }),
                _ => Err(invalid("inconsistent Hatch annotation loop context")),
            }
        })
        .collect::<Result<_, _>>()?;
    Ok(IfccadHatchAnnotationContext {
        pattern: IfccadHatchContextPattern {
            families,
            scale: p.pattern_scale,
            orientation_reference: annotation::local(p.pattern_base, placement, false),
        },
        loops,
    })
}

pub(super) fn import_all(
    drawing: &IfccadDocument,
    document: &mut Document,
    names: &Names,
    handles: &HashMap<u64, Handle>,
    losses: &mut Losses,
) -> Result<(), Error> {
    for entity in drawing
        .blocks
        .iter()
        .map(|b| &b.entities)
        .chain(drawing.paper_layouts.iter().map(|p| &p.entities))
        .chain([&drawing.model.entities])
        .flatten()
    {
        let Some(IfccadNativeEntity {
            id,
            kind: IfccadEntityKind::Hatch(h),
            ..
        }) = entity.as_native()
        else {
            continue;
        };
        let Some(owner) = handles.get(id).copied() else {
            continue;
        };
        let has_local_sources = |p: &IfccadHatchAnnotationContext| {
            p.loops.iter().any(|l| matches!(l, IfccadHatchContextLoop::Local{boundary,..} if !boundary.source_entity_ids.is_empty()))
        };
        if h.annotation.as_ref().is_some_and(|a| {
            a.contexts
                .iter()
                .filter_map(|c| c.presentation.as_ref())
                .any(has_local_sources)
        }) || h.annotation_view_contexts.as_ref().is_some_and(|v| {
            v.contexts
                .iter()
                .any(|c| has_local_sources(&c.presentation))
        }) {
            losses.note(format!("Hatch {id} local annotation boundary source references have no CAD context file field"));
        }
        if let Some(a) = &h.annotation {
            annotation::import(a, owner, document, names, |p| {
                Ok(ObjectContextKind::HatchScale(presentation_in(
                    p,
                    &h.placement,
                    handles,
                )?))
            })?;
        }
        let Some(views) = &h.annotation_view_contexts else {
            continue;
        };
        let extension = document.ensure_extension_dictionary(owner);
        let manager = child_dictionary(document, extension, "AcDbContextDataManager");
        let collection =
            child_dictionary(document, manager, "ACDB_ANNOTATIONSCALE_VIEW_COLLECTION");
        for (index, c) in views.contexts.iter().enumerate() {
            let scale = *names
                .annotation_scales
                .get(&c.scale_id)
                .ok_or_else(|| invalid("unresolved Hatch view scale"))?;
            let view = match c.target {
                None => Handle::NULL,
                Some(IfccadAnnotationViewTarget::PaperViewport { entity_id }) => *handles
                    .get(&entity_id)
                    .ok_or_else(|| invalid("unresolved Hatch Paper viewport target"))?,
                Some(IfccadAnnotationViewTarget::StoredView { id }) => *names
                    .annotation_views
                    .get(&id)
                    .ok_or_else(|| invalid("unresolved Hatch stored-view target"))?,
                Some(IfccadAnnotationViewTarget::ModelWindow { id }) => *names
                    .model_window_handles
                    .get(&id)
                    .ok_or_else(|| invalid("unresolved Hatch model-window target"))?,
            };
            let handle = document.allocate_handle();
            let context = ObjectContextData {
                handle,
                owner_handle: collection,
                reactors: vec![collection],
                xdictionary_handle: None,
                class_version: 4,
                has_file_to_extension_dictionary: true,
                uses_legacy_mleader_context_base: false,
                is_default: views.reference == Some(c.id),
                scale,
                kind: ObjectContextKind::HatchView(HatchViewContext {
                    hatch: presentation_in(&c.presentation, &h.placement, handles)?,
                    view,
                    view_normal: vector(c.view_state.normal),
                    view_rotation: c.view_state.rotation,
                    evaluate_hatch: c.view_state.evaluate,
                }),
            };
            document.register_object_context_class(context.class_name());
            document.context_scales.insert(handle, scale);
            document
                .objects
                .insert(handle, ObjectType::ObjectContextData(context));
            let Some(ObjectType::Dictionary(d)) = document.objects.get_mut(&collection) else {
                unreachable!()
            };
            d.insert(format!("*A{index}"), handle);
        }
    }
    Ok(())
}
fn view_collection(document: &Document, entity: &EntityType) -> Result<Option<Handle>, Error> {
    let mut current = entity.common().xdictionary_handle;
    for key in [
        "AcDbContextDataManager",
        "ACDB_ANNOTATIONSCALE_VIEW_COLLECTION",
    ] {
        let Some(handle) = current else {
            return Ok(None);
        };
        let Some(ObjectType::Dictionary(d)) = document.objects.get(&handle) else {
            return Err(invalid("unresolved Hatch annotation dictionary"));
        };
        current = d.get(key);
    }
    Ok(current)
}

pub(super) fn export_all(writer: &mut Writer) -> Result<(), Error> {
    let mut converted = HashMap::new();
    for (handle, id) in &writer.entity_ids {
        let Some(entity @ EntityType::Hatch(_)) = writer.document.get_entity(*handle) else {
            continue;
        };
        let placement = if let EntityType::Hatch(h) = entity {
            placement_out(Vector3::new(0., 0., h.elevation), h.normal, 0.)
        } else {
            unreachable!()
        };
        let mut annotation = annotation::export(writer, entity, |p| match p {
            ObjectContextKind::HatchScale(p) => presentation_out(p, &placement, &writer.entity_ids),
            _ => Err(invalid("annotation context does not match Hatch owner")),
        })?;
        let views = if let Some(collection) = view_collection(writer.document, entity)? {
            let Some(ObjectType::Dictionary(d)) = writer.document.objects.get(&collection) else {
                return Err(invalid("unresolved Hatch view collection"));
            };
            let mut out = IfccadHatchAnnotationViewContexts {
                next_context_id: 1,
                reference: None,
                contexts: vec![],
            };
            for (_, handle) in &d.entries {
                let Some(ObjectType::ObjectContextData(c)) = writer.document.objects.get(handle)
                else {
                    return Err(invalid("unresolved Hatch view leaf"));
                };
                let ObjectContextKind::HatchView(p) = &c.kind else {
                    return Err(invalid("unexpected Hatch view context class"));
                };
                let id = out.allocate_context_id()?;
                if c.is_default && out.reference.replace(id).is_some() {
                    return Err(invalid("multiple default Hatch view contexts"));
                }
                let target = if p.view.is_null() {
                    None
                } else if let Some(EntityType::Viewport(_)) = writer.document.get_entity(p.view) {
                    Some(IfccadAnnotationViewTarget::PaperViewport {
                        entity_id: *writer
                            .entity_ids
                            .get(&p.view)
                            .ok_or_else(|| invalid("unmapped Hatch viewport target"))?,
                    })
                } else if let Some(id) = writer.annotation_views.get(&p.view) {
                    Some(IfccadAnnotationViewTarget::StoredView { id: *id })
                } else if let Some(id) = writer.model_window_handles.get(&p.view) {
                    Some(IfccadAnnotationViewTarget::ModelWindow { id: *id })
                } else {
                    return Err(invalid("unmapped Hatch view target"));
                };
                out.contexts.push(IfccadHatchAnnotationViewContext {
                    id,
                    scale_id: *writer
                        .annotation_scales
                        .get(&c.scale)
                        .ok_or_else(|| invalid("unresolved Hatch view scale"))?,
                    target,
                    presentation: presentation_out(&p.hatch, &placement, &writer.entity_ids)?,
                    view_state: IfccadHatchContextViewState {
                        normal: [p.view_normal.x, p.view_normal.y, p.view_normal.z],
                        rotation: p.view_rotation,
                        evaluate: p.evaluate_hatch,
                    },
                });
            }
            Some(Box::new(out))
        } else {
            None
        };
        if views.as_ref().is_some_and(|v| !v.contexts.is_empty())
            && annotation::data_flag(&entity.common().extended_data)?.is_none()
        {
            let a = annotation.get_or_insert_with(|| {
                Box::new(IfccadAnnotation {
                    orientation: None,
                    mode: IfccadAnnotationMode::Enabled,
                    reference: None,
                    contexts: vec![],
                })
            });
            a.mode = IfccadAnnotationMode::Enabled;
        }
        converted.insert(*id, (annotation, views));
    }
    let drawing = &mut writer.drawing;
    for entity in drawing
        .blocks
        .iter_mut()
        .map(|b| &mut b.entities)
        .chain(drawing.paper_layouts.iter_mut().map(|p| &mut p.entities))
        .chain([&mut drawing.model.entities])
        .flatten()
    {
        if let IfccadEntity::Native(IfccadNativeEntity {
            id,
            kind: IfccadEntityKind::Hatch(h),
            ..
        }) = entity
        {
            if let Some((annotation, views)) = converted.remove(id) {
                h.annotation = annotation;
                h.annotation_view_contexts = views;
            }
        }
    }
    Ok(())
}
