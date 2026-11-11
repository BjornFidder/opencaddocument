//! Independent saved Leader/FCF/MLeader scale presentations.
use super::*;
use acadrust::objects::{LeaderContext, ObjectContextKind};
use ifccad::*;

pub(super) fn export(
    writer: &mut Writer,
    entity: &EntityType,
    kind: &mut IfccadEntityKind,
) -> Result<(), Error> {
    match kind {
        IfccadEntityKind::Leader(value) => {
            value.annotation = annotation::export(writer, entity, |p| {
                let ObjectContextKind::Leader(p) = p else {
                    return Err(annotation::invalid("Leader scale context kind mismatch"));
                };
                Ok(IfccadLeaderAnnotationContext {
                    points: p
                        .points
                        .iter()
                        .map(|p| annotation::local(*p, &value.placement, true))
                        .collect(),
                    horizontal_direction: annotation::local(p.x_direction, &value.placement, false),
                    annotation_enabled: p.annotation_enabled,
                    insertion_offset: annotation::local(
                        p.insertion_offset,
                        &value.placement,
                        false,
                    ),
                    endpoint_projection: annotation::local(
                        p.endpoint_projection,
                        &value.placement,
                        false,
                    ),
                })
            })?
            .map(|p| *p);
        }
        IfccadEntityKind::Tolerance(value) => {
            value.annotation = annotation::export(writer, entity, |p| {
                let ObjectContextKind::Fcf {
                    location,
                    horizontal_direction,
                } = p
                else {
                    return Err(annotation::invalid("FCF scale context kind mismatch"));
                };
                Ok(IfccadToleranceAnnotationContext {
                    location: annotation::local(*location, &value.placement, true),
                    horizontal_direction: annotation::local(
                        *horizontal_direction,
                        &value.placement,
                        false,
                    ),
                })
            })?
            .map(|p| *p);
        }
        IfccadEntityKind::MultiLeader(value) => {
            let raw = annotation::export(writer, entity, |p| {
                let ObjectContextKind::MLeader(p) = p else {
                    return Err(annotation::invalid("MLeader scale context kind mismatch"));
                };
                Ok(p.clone())
            })?;
            if let Some(raw) = raw {
                let mut contexts = Vec::new();
                for c in raw.contexts {
                    if let Some(p) = &c.presentation {
                        if let Some(scale) = p.scale_handle.filter(|h| !h.is_null()) {
                            if writer.annotation_scales.get(&scale) != Some(&c.key.scale_id) {
                                return Err(annotation::invalid(
                                    "MLeader context scale aliases conflict",
                                ));
                            }
                        }
                    }
                    contexts.push(IfccadAnnotationContext {
                        key: c.key,
                        presentation: c
                            .presentation
                            .as_ref()
                            .map(|p| {
                                multi_leader::presentation_out(
                                    writer,
                                    p,
                                    value.settings.common.content_kind,
                                )
                            })
                            .transpose()?,
                    });
                }
                value.annotation = Some(IfccadAnnotation {
                    mode: raw.mode,
                    orientation: raw.orientation,
                    reference: raw.reference,
                    contexts,
                });
            }
            let EntityType::MultiLeader(source) = entity else {
                unreachable!()
            };
            if source.enable_annotation_scale {
                if value
                    .annotation
                    .as_ref()
                    .is_some_and(|a| a.mode == IfccadAnnotationMode::Disabled)
                {
                    return Err(annotation::invalid("conflicting MLeader annotative flags"));
                }
                value
                    .annotation
                    .get_or_insert(IfccadAnnotation {
                        mode: IfccadAnnotationMode::Enabled,
                        orientation: None,
                        reference: None,
                        contexts: vec![],
                    })
                    .mode = IfccadAnnotationMode::Enabled;
            }
        }
        _ => {}
    }
    Ok(())
}
pub(super) fn import(
    drawing: &IfccadDocument,
    document: &mut Document,
    names: &Names,
    handles: &HashMap<u64, Handle>,
    losses: &mut Losses,
) -> Result<(), Error> {
    for entity in drawing
        .blocks
        .iter()
        .flat_map(|b| &b.entities)
        .chain(drawing.paper_layouts.iter().flat_map(|p| &p.entities))
        .chain(&drawing.model.entities)
    {
        let Some(entity) = entity.as_native() else {
            continue;
        };
        let Some(handle) = handles.get(&entity.id).copied() else {
            continue;
        };
        match &entity.kind {
            IfccadEntityKind::Leader(value) => {
                if let Some(a) = &value.annotation {
                    annotation::import(a, handle, document, names, |p| {
                        Ok(ObjectContextKind::Leader(LeaderContext {
                            points: p
                                .points
                                .iter()
                                .map(|p| annotation::world(*p, &value.placement, true))
                                .collect(),
                            x_direction: annotation::world(
                                p.horizontal_direction,
                                &value.placement,
                                false,
                            ),
                            annotation_enabled: p.annotation_enabled,
                            insertion_offset: annotation::world(
                                p.insertion_offset,
                                &value.placement,
                                false,
                            ),
                            endpoint_projection: annotation::world(
                                p.endpoint_projection,
                                &value.placement,
                                false,
                            ),
                        }))
                    })?;
                }
            }
            IfccadEntityKind::Tolerance(value) => {
                if let Some(a) = &value.annotation {
                    annotation::import(a, handle, document, names, |p| {
                        Ok(ObjectContextKind::Fcf {
                            location: annotation::world(p.location, &value.placement, true),
                            horizontal_direction: annotation::world(
                                p.horizontal_direction,
                                &value.placement,
                                false,
                            ),
                        })
                    })?;
                }
            }
            IfccadEntityKind::MultiLeader(value) => {
                if let Some(a) = &value.annotation {
                    let mut a = a.clone();
                    if a.mode == IfccadAnnotationMode::ByDefinition {
                        let enabled = value
                            .style_id
                            .and_then(|id| drawing.multi_leader_styles.iter().find(|s| s.id == id))
                            .ok_or_else(|| {
                                annotation::invalid("inherited MLeader annotation lacks style")
                            })?
                            .annotative;
                        a.mode = if enabled {
                            IfccadAnnotationMode::Enabled
                        } else {
                            IfccadAnnotationMode::Disabled
                        };
                        losses.note(format!("MLeader {} annotation inheritance materialized to the current style flag in CAD",entity.id));
                    }
                    let mut contexts = Vec::new();
                    for c in &a.contexts {
                        let mut p = c
                            .presentation
                            .as_ref()
                            .map(|p| multi_leader::presentation_in(p, names, document, losses))
                            .transpose()?;
                        if let Some(p) = &mut p {
                            p.scale_handle =
                                Some(*names.annotation_scales.get(&c.key.scale_id).ok_or_else(
                                    || annotation::invalid("missing MLeader context scale"),
                                )?);
                        }
                        contexts.push(IfccadAnnotationContext {
                            key: c.key,
                            presentation: p,
                        });
                    }
                    if let Some(EntityType::MultiLeader(target)) = document.get_entity_mut(handle) {
                        target.enable_annotation_scale = a.mode == IfccadAnnotationMode::Enabled;
                    }
                    annotation::import(
                        &IfccadAnnotation {
                            mode: a.mode,
                            orientation: a.orientation,
                            reference: a.reference,
                            contexts,
                        },
                        handle,
                        document,
                        names,
                        |p| Ok(ObjectContextKind::MLeader(p.clone())),
                    )?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}
