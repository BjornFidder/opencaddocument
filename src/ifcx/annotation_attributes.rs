use super::*;
use acadrust::objects::{
    EmbeddedMTextContext, MTextAttributeContext, MTextColumns, MTextContext, ObjectContextKind,
};
use ifccad::{
    IfccadAnnotation, IfccadAnnotationColumnKind, IfccadAttributeAnnotationContext as A,
    IfccadAttributeEmbeddedContext, IfccadAttributeMText, IfccadMTextAnnotationColumns,
    IfccadMTextAnnotationContext, IfccadTextAnnotationContext,
};

fn text_out(
    horizontal: i16,
    rotation: f64,
    insertion: Vector2,
    alignment: Vector2,
    text: &IfccadText,
) -> Result<IfccadTextAnnotationContext, Error> {
    let (origin, _, r) = placement_in(&text.placement);
    let (s, c) = r.sin_cos();
    let point = |p: Vector2| {
        let x = p.x - origin.x;
        let y = p.y - origin.y;
        [x * c + y * s, -x * s + y * c]
    };
    Ok(IfccadTextAnnotationContext {
        insertion: point(insertion),
        alignment: point(alignment),
        rotation: rotation - r,
        horizontal: annotation::horizontal(horizontal)?,
    })
}
fn text_in(p: &IfccadTextAnnotationContext, text: &IfccadText) -> (i16, f64, Vector2, Vector2) {
    let (origin, _, r) = placement_in(&text.placement);
    let (s, c) = r.sin_cos();
    let point = |p: [f64; 2]| {
        Vector2::new(
            origin.x + p[0] * c - p[1] * s,
            origin.y + p[0] * s + p[1] * c,
        )
    };
    (
        annotation::horizontal_code(p.horizontal),
        p.rotation + r,
        point(p.insertion),
        point(p.alignment),
    )
}
fn mtext_out(
    p: &MTextContext,
    t: &ifccad::IfccadMText,
) -> Result<IfccadMTextAnnotationContext, Error> {
    let columns = match (p.column_type, &p.columns) {
        (0, None) => None,
        (kind @ 1..=2, Some(c)) => Some(IfccadMTextAnnotationColumns {
            kind: if kind == 1 {
                IfccadAnnotationColumnKind::Static
            } else {
                IfccadAnnotationColumnKind::Dynamic
            },
            count: u32::try_from(c.num_heights)
                .map_err(|_| annotation::invalid("negative attribute context columns"))?,
            width: c.width,
            gutter: c.gutter,
            auto_height: c.auto_height,
            flow_reversed: c.flow_reversed,
            heights: c.heights.clone(),
        }),
        _ => {
            return Err(annotation::invalid(
                "inconsistent attribute context columns",
            ))
        }
    };
    Ok(IfccadMTextAnnotationContext {
        insertion: annotation::local(p.insertion, &t.placement, true),
        x_direction: annotation::local(p.x_axis_dir, &t.placement, false),
        attachment: annotation::attachment(p.attachment)?,
        rectangle_width: p.rect_width,
        rectangle_height: p.rect_height,
        columns,
    })
}
fn mtext_in(
    p: &IfccadMTextAnnotationContext,
    t: &ifccad::IfccadMText,
) -> Result<MTextContext, Error> {
    let columns = p
        .columns
        .as_ref()
        .map(|c| {
            Ok::<_, Error>(MTextColumns {
                num_heights: i32::try_from(c.count).map_err(|_| {
                    annotation::invalid("attribute context column count exceeds CAD")
                })?,
                width: c.width,
                gutter: c.gutter,
                auto_height: c.auto_height,
                flow_reversed: c.flow_reversed,
                heights: c.heights.clone(),
            })
        })
        .transpose()?;
    Ok(MTextContext {
        attachment: annotation::attachment_code(p.attachment),
        x_axis_dir: annotation::world(p.x_direction, &t.placement, false),
        insertion: annotation::world(p.insertion, &t.placement, true),
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
    })
}
pub(super) fn export(
    w: &mut Writer<'_>,
    entity: &EntityType,
    text: &IfccadText,
    mtext: Option<&IfccadAttributeMText>,
) -> Result<Option<Box<IfccadAnnotation<A>>>, Error> {
    let mut omitted_extents = false;
    let out = annotation::export(w, entity, |p| match p {
        ObjectContextKind::Text {
            horizontal_mode,
            rotation,
            insertion,
            alignment,
        } if mtext.is_none() => Ok(A::Text {
            text: text_out(*horizontal_mode, *rotation, *insertion, *alignment, text)?,
        }),
        ObjectContextKind::MTextAttribute(p) if mtext.is_some() => {
            let embedded = p
                .context
                .as_ref()
                .map(|e| -> Result<_, Error> {
                    if !e.owner_handle.is_null()
                        || !e.reactors.is_empty()
                        || e.xdictionary_handle.is_some()
                        || e.has_binary_data
                    {
                        return Err(annotation::invalid(
                            "embedded attribute context has unsupported object relationships",
                        ));
                    }
                    omitted_extents |= e.mtext.extents_width != 0. || e.mtext.extents_height != 0.;
                    Ok(IfccadAttributeEmbeddedContext {
                        scale_id: *w.annotation_scales.get(&e.scale).ok_or_else(|| {
                            annotation::invalid("unresolved embedded attribute scale")
                        })?,
                        is_default: e.is_default,
                        presentation: mtext_out(&e.mtext, &mtext.unwrap().text)?,
                    })
                })
                .transpose()?;
            Ok(A::MText {
                text: text_out(
                    p.horizontal_mode,
                    p.rotation,
                    p.insertion,
                    p.alignment,
                    text,
                )?,
                m_text_enabled: p.enable_context,
                embedded,
            })
        }
        _ => Err(annotation::invalid(
            "annotation context does not match attribute content",
        )),
    })?;
    if omitted_extents {
        w.losses.note("attribute MText context layout-extents cache omitted; saved rectangle/columns retained");
    }
    Ok(out)
}
pub(super) fn import(
    doc: &mut Document,
    handle: Handle,
    a: Option<&IfccadAnnotation<A>>,
    text: &IfccadText,
    mtext: Option<&IfccadAttributeMText>,
    names: &Names,
) -> Result<(), Error> {
    let Some(a) = a else { return Ok(()) };
    annotation::import(a, handle, doc, names, |p| match p {
        A::Text { text: p } => {
            let (h, r, i, a) = text_in(p, text);
            Ok(ObjectContextKind::Text {
                horizontal_mode: h,
                rotation: r,
                insertion: i,
                alignment: a,
            })
        }
        A::MText {
            text: p,
            m_text_enabled,
            embedded,
        } => {
            let (h, r, i, a) = text_in(p, text);
            let context = embedded
                .as_ref()
                .map(|e| -> Result<_, Error> {
                    Ok(EmbeddedMTextContext {
                        owner_handle: Handle::NULL,
                        reactors: vec![],
                        xdictionary_handle: None,
                        has_binary_data: false,
                        class_version: 4,
                        has_file_to_extension_dictionary: true,
                        is_default: e.is_default,
                        scale: *names.annotation_scales.get(&e.scale_id).ok_or_else(|| {
                            annotation::invalid("missing embedded attribute scale")
                        })?,
                        mtext: mtext_in(
                            &e.presentation,
                            &mtext
                                .ok_or_else(|| {
                                    annotation::invalid("MText attribute context without MText")
                                })?
                                .text,
                        )?,
                    })
                })
                .transpose()?;
            Ok(ObjectContextKind::MTextAttribute(MTextAttributeContext {
                horizontal_mode: h,
                rotation: r,
                insertion: i,
                alignment: a,
                enable_context: *m_text_enabled,
                context,
            }))
        }
    })
}
