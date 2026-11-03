//! Dimension context snapshots; never modify canonical definitions or associations.
use super::annotation::invalid;
use super::*;
use acadrust::objects::{DimContext, DimSubtype, ObjectContextKind};
use ifccad::dimension::DimensionKind;
use ifccad::*;
fn layout(p: &IfccadDimensionContextLayout) -> (u8, [bool; 5]) {
    let values = [
        p.line_outside,
        p.suppress_arrows_outside,
        p.fit_switch,
        p.force_text_inside,
        p.text_movement_switch,
    ];
    (
        values
            .iter()
            .enumerate()
            .fold(0, |bits, (i, v)| bits | if v.active { 1 << i } else { 0 }),
        values.map(|v| v.value),
    )
}
fn subtype_in(p: &IfccadDimensionContextGeometry, frame: &IfccadPlacement) -> DimSubtype {
    use IfccadDimensionContextGeometry::*;
    let world = |p| annotation::world(p, frame, true);
    match p {
        Linear {
            dimension_line_point,
        }
        | Aligned {
            dimension_line_point,
        } => DimSubtype::Aligned {
            dimline_pt: world(*dimension_line_point),
        },
        AngularTwoLines {
            dimension_arc_point,
        }
        | AngularThreePoints {
            dimension_arc_point,
        }
        | ArcLength {
            dimension_arc_point,
        } => DimSubtype::Angular {
            arc_pt: world(*dimension_arc_point),
        },
        Radius { curve_point } => DimSubtype::Radial {
            first_arc_pt: world(*curve_point),
        },
        Diameter {
            first_diameter_point,
            second_diameter_point,
        } => DimSubtype::Diametric {
            first_arc_pt: world(*first_diameter_point),
            def_pt: world(*second_diameter_point),
        },
        Ordinate {
            feature_point,
            leader_end,
        } => DimSubtype::Ordinate {
            feature_location_pt: world(*feature_point),
            leader_endpt: world(*leader_end),
        },
        JoggedRadius {
            display_center,
            jog_point,
        } => DimSubtype::RadialLarge {
            ovr_center: world(*display_center),
            jog_point: world(*jog_point),
        },
    }
}
fn subtype_out(
    p: &DimSubtype,
    kind: DimensionKind,
    frame: &IfccadPlacement,
) -> Result<IfccadDimensionContextGeometry, Error> {
    use IfccadDimensionContextGeometry as G;
    let local = |p| annotation::local(p, frame, true);
    Ok(match (kind, p) {
        (DimensionKind::Linear, DimSubtype::Aligned { dimline_pt }) => G::Linear {
            dimension_line_point: local(*dimline_pt),
        },
        (DimensionKind::Aligned, DimSubtype::Aligned { dimline_pt }) => G::Aligned {
            dimension_line_point: local(*dimline_pt),
        },
        (DimensionKind::AngularTwoLines, DimSubtype::Angular { arc_pt }) => G::AngularTwoLines {
            dimension_arc_point: local(*arc_pt),
        },
        (DimensionKind::AngularThreePoints, DimSubtype::Angular { arc_pt }) => {
            G::AngularThreePoints {
                dimension_arc_point: local(*arc_pt),
            }
        }
        (DimensionKind::ArcLength, DimSubtype::Angular { arc_pt }) => G::ArcLength {
            dimension_arc_point: local(*arc_pt),
        },
        (DimensionKind::Radius, DimSubtype::Radial { first_arc_pt }) => G::Radius {
            curve_point: local(*first_arc_pt),
        },
        (
            DimensionKind::Diameter,
            DimSubtype::Diametric {
                first_arc_pt,
                def_pt,
            },
        ) => G::Diameter {
            first_diameter_point: local(*first_arc_pt),
            second_diameter_point: local(*def_pt),
        },
        (
            DimensionKind::Ordinate,
            DimSubtype::Ordinate {
                feature_location_pt,
                leader_endpt,
            },
        ) => G::Ordinate {
            feature_point: local(*feature_location_pt),
            leader_end: local(*leader_endpt),
        },
        (
            DimensionKind::JoggedRadius,
            DimSubtype::RadialLarge {
                ovr_center,
                jog_point,
            },
        ) => G::JoggedRadius {
            display_center: local(*ovr_center),
            jog_point: local(*jog_point),
        },
        _ => {
            return Err(invalid(
                "Dimension annotation subtype does not match canonical owner kind",
            ))
        }
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
            layer_id,
            kind: IfccadEntityKind::Dimension(source),
            ..
        }) = entity.as_native()
        else {
            continue;
        };
        let Some(a) = &source.annotation else {
            continue;
        };
        let Some(handle) = handles.get(id).copied() else {
            continue;
        };
        let owner = document
            .get_entity(handle)
            .ok_or_else(|| invalid("missing Dimension context owner"))?
            .common()
            .owner_handle;
        let mut out = IfccadAnnotation {
            orientation: a.orientation,
            mode: a.mode,
            reference: a.reference,
            contexts: vec![],
        };
        if a.mode == IfccadAnnotationMode::ByDefinition && !a.contexts.is_empty() {
            let enabled = drawing
                .dimension_styles
                .iter()
                .find(|s| s.id == source.style_id)
                .ok_or_else(|| invalid("missing Dimension context style"))?
                .annotative;
            out.mode = if enabled {
                IfccadAnnotationMode::Enabled
            } else {
                IfccadAnnotationMode::Disabled
            };
            losses.note(format!("Dimension {id} annotation inheritance with memberships materialized to its current effective CAD object flag"));
        }
        let (origin, _, rotation) = placement_in(&source.placement);
        for c in &a.contexts {
            let presentation = if let Some(p) = &c.presentation {
                let block = if p.graphics.is_some() {
                    let mut picture_source = source.as_ref().clone();
                    picture_source.annotation = None;
                    picture_source.graphics = p.graphics.clone();
                    let before = losses.content_snapshot();
                    let name = dimension_graphics::import(
                        &picture_source,
                        *id,
                        *layer_id,
                        owner,
                        drawing,
                        document,
                        names,
                        losses,
                    )?;
                    losses.locate_new_content(
                        &before,
                        &format!("Dimension {id} annotation scale{}", c.key.scale_id.0),
                    );
                    name.and_then(|n| document.block_records.get(&n).map(|r| r.handle))
                        .unwrap_or(Handle::NULL)
                } else {
                    Handle::NULL
                };
                let (override_code, [dimtofl, dimosxd, dimatfit, dimtix, dimtmove]) =
                    layout(&p.layout);
                Some(DimContext {
                    def_pt: frame_to_ocs(p.text_location[0], p.text_location[1], origin, rotation),
                    is_def_textloc: p.user_text_location,
                    text_rotation: p.text_rotation + rotation,
                    block,
                    b293: false,
                    dimtofl,
                    dimosxd,
                    dimatfit,
                    dimtix,
                    dimtmove,
                    override_code,
                    has_arrow2: p.layout.second_arrow_present,
                    flip_arrow1: p.layout.flip_first,
                    flip_arrow2: p.layout.flip_second,
                    subtype: subtype_in(&p.geometry, &source.placement),
                })
            } else {
                None
            };
            out.contexts.push(IfccadAnnotationContext {
                key: c.key,
                presentation,
            });
        }
        annotation::import(&out, handle, document, names, |p| {
            Ok(ObjectContextKind::Dim(p.clone()))
        })?;
    }
    Ok(())
}
pub(super) fn export_entity(
    writer: &mut Writer,
    entity: &EntityType,
    kind: &mut IfccadEntityKind,
) -> Result<(), Error> {
    let (EntityType::Dimension(source), IfccadEntityKind::Dimension(target)) = (entity, kind)
    else {
        return Ok(());
    };
    let (origin, _, rotation) = placement_in(&target.placement);
    let local_text = |p: Vector2| {
        let (s, c) = rotation.sin_cos();
        let x = p.x - origin.x;
        let y = p.y - origin.y;
        [x * c + y * s, -x * s + y * c]
    };
    let mut blocks = vec![];
    target.annotation = annotation::export(writer, entity, |p| {
        let ObjectContextKind::Dim(p) = p else {
            return Err(invalid("annotation context does not match Dimension owner"));
        };
        if p.b293 || p.override_code & !0x1f != 0 {
            return Err(invalid(
                "unsupported reserved Dimension annotation layout fields",
            ));
        }
        let hint = |i: u32, value: bool| IfccadStoredBoolOverride {
            active: p.override_code & (1u8 << i) != 0u8,
            value,
        };
        blocks.push(p.block);
        Ok(IfccadDimensionAnnotationContext {
            text_location: local_text(p.def_pt),
            user_text_location: p.is_def_textloc,
            text_rotation: p.text_rotation - rotation,
            layout: IfccadDimensionContextLayout {
                line_outside: hint(0, p.dimtofl),
                suppress_arrows_outside: hint(1, p.dimosxd),
                fit_switch: hint(2, p.dimatfit),
                force_text_inside: hint(3, p.dimtix),
                text_movement_switch: hint(4, p.dimtmove),
                second_arrow_present: p.has_arrow2,
                flip_first: p.flip_arrow1,
                flip_second: p.flip_arrow2,
            },
            geometry: subtype_out(&p.subtype, target.definition.kind(), &target.placement)?,
            graphics: None,
        })
    })?;
    if let Some(a) = &mut target.annotation {
        for (c, block) in a
            .contexts
            .iter_mut()
            .filter(|c| c.presentation.is_some())
            .zip(blocks)
        {
            if block.is_null() {
                continue;
            }
            let name = writer
                .document
                .block_records
                .iter()
                .find(|r| r.handle == block)
                .ok_or_else(|| invalid("dangling Dimension annotation picture block"))?
                .name
                .clone();
            let mut picture_source = source.as_ref().clone();
            picture_source.base_mut().block_name = name;
            let before = writer.losses.content_snapshot();
            c.presentation.as_mut().unwrap().graphics =
                dimension_graphics::capture(writer, &picture_source, &target.placement)?;
            writer.losses.locate_new_content(
                &before,
                &format!(
                    "Dimension {} annotation scale{}",
                    source.base().common.handle,
                    c.key.scale_id.0
                ),
            );
        }
    }
    Ok(())
}
