//! Bounded named VIEW cameras referenced by annotation snapshots.
use super::*;
use acadrust::tables::View;
use ifccad::*;

pub(super) fn import(
    drawing: &IfccadDocument,
    document: &mut Document,
    names: &mut Names,
) -> Result<(), Error> {
    for source in &drawing.annotation_views {
        let mut v = View::new(&source.name);
        v.handle = document.allocate_handle();
        let c = &source.view;
        v.center = Vector3::new(c.center[0], c.center[1], 0.);
        v.target = vector(c.target);
        v.direction = vector(c.direction);
        v.height = c.height;
        v.width = source.width;
        v.twist_angle = c.twist;
        v.perspective = c.projection == IfccadViewportProjection::Perspective;
        v.lens_length = c.lens_length_mm.unwrap_or(50.);
        v.front_clipping = c.front_clip.mode != IfccadViewportClipMode::Disabled;
        v.front_clip_at_eye = c.front_clip.mode == IfccadViewportClipMode::AtCamera;
        v.front_clip = c.front_clip.distance.unwrap_or(0.);
        v.back_clipping = c.back_clip.mode != IfccadViewportClipMode::Disabled;
        v.back_clip = c.back_clip.distance.unwrap_or(0.);
        v.paper_space = source.space == IfccadAnnotationViewSpace::Paper;
        names.annotation_views.insert(source.id, v.handle);
        document.views.add(v)?;
    }
    Ok(())
}
pub(super) fn export(writer: &mut Writer) -> Result<(), Error> {
    for v in writer.document.views.iter() {
        let id = writer.drawing.id_counters.allocate_annotation_view_id()?;
        let view = IfccadViewportView {
            center: [v.center.x, v.center.y],
            target: [v.target.x, v.target.y, v.target.z],
            direction: [v.direction.x, v.direction.y, v.direction.z],
            height: v.height,
            twist: v.twist_angle,
            projection: if v.perspective {
                IfccadViewportProjection::Perspective
            } else {
                IfccadViewportProjection::Orthographic
            },
            lens_length_mm: Some(v.lens_length),
            front_clip: IfccadViewportDepthClip {
                mode: if !v.front_clipping {
                    IfccadViewportClipMode::Disabled
                } else if v.front_clip_at_eye {
                    IfccadViewportClipMode::AtCamera
                } else {
                    IfccadViewportClipMode::AtDistance
                },
                distance: Some(v.front_clip),
            },
            back_clip: IfccadViewportDepthClip {
                mode: if v.back_clipping {
                    IfccadViewportClipMode::AtDistance
                } else {
                    IfccadViewportClipMode::Disabled
                },
                distance: Some(v.back_clip),
            },
        };
        writer.annotation_views.insert(v.handle, id);
        writer
            .drawing
            .annotation_views
            .push(IfccadAnnotationStoredView {
                annotation_scale_id: None,
                id,
                name: v.name.clone(),
                space: if v.paper_space {
                    IfccadAnnotationViewSpace::Paper
                } else {
                    IfccadAnnotationViewSpace::Model
                },
                width: v.width,
                view,
            });
        // Account for VIEW data outside this bounded camera contract.
        let mut remainder = v.clone();
        let default = View::new(&v.name);
        remainder.handle = default.handle;
        remainder.center = Vector3::new(default.center.x, default.center.y, v.center.z);
        remainder.target = default.target;
        remainder.direction = default.direction;
        remainder.height = default.height;
        remainder.width = default.width;
        remainder.twist_angle = default.twist_angle;
        remainder.perspective = default.perspective;
        remainder.lens_length = default.lens_length;
        remainder.front_clipping = default.front_clipping;
        remainder.back_clipping = default.back_clipping;
        remainder.front_clip = default.front_clip;
        remainder.back_clip = default.back_clip;
        remainder.paper_space = default.paper_space;
        if v.front_clipping {
            remainder.front_clip_at_eye = default.front_clip_at_eye;
        }
        if remainder != default {
            writer
                .losses
                .note(format!("saved VIEW {} non-camera metadata", v.name));
        }
    }
    Ok(())
}
