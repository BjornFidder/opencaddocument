//! Layouts: paper layouts and their viewports, every layout's sheet and plot
//! settings, and the editor state around them (model windows, paper
//! canvases, viewport drawing aids, the current layer and layout).
//!
//! Field mappings follow the ifccad-convert and cad-workspace-convert crates
//! of ifccad (MPL-2.0).

use acadrust::entities::{Block, BlockEnd, GridFlags, Viewport, ViewportRenderMode};
use acadrust::objects::{Layout, ObjectType};
use acadrust::tables::VPort;
use ifccad::geometry_kernel::{CoordinateFrame3, CoordinateLengthUnit, Point2, Point3};
use ifccad::plot_kernel::{
    validate_layout_output, LayoutMedia, LayoutOutputKind, MediaUnit, PlotArea, PlotMapping,
    PlotOffsetReference, PlotOptions, PlotOutput, PlotPage, PlotPlacement, PlotRect, PlotRotation,
    PlotScale, PlotSettings, PlotUnit, ShadedPlot, ShadedPlotMode, ShadedPlotQuality,
    ShadedPlotQualityMode,
};
use ifccad::workspace_kernel::{
    validate_canvas_frame, validate_grid, validate_snap, validate_view, WorkspaceCanvasFrame,
    WorkspaceClip, WorkspaceClipMode, WorkspaceGrid, WorkspaceGridStyle, WorkspaceIsometricPlane,
    WorkspaceProjection, WorkspaceRenderMode, WorkspaceSnap, WorkspaceSnapStyle, WorkspaceView,
    WorkspaceViewKind,
};
use ifccad::{
    validate_ifccad_viewport_parameters, IfccadDrawingWorkspaceState, IfccadLayoutSettings,
    IfccadModelViewState, IfccadModelWindow, IfccadPaperCanvas, IfccadPaperLayout, IfccadViewport,
    IfccadViewportClipMode, IfccadViewportDepthClip, IfccadViewportFrame,
    IfccadViewportLayerOverride, IfccadViewportPaperClip, IfccadViewportProjection,
    IfccadViewportRenderMode, IfccadViewportView, IfccadViewportWorkspace,
};

use super::*;

// ---------------------------------------------------------------- writing

/// Every layout's settings, the paper layouts with their entities, model
/// windows and the drawing's workspace state.
pub(super) fn write(writer: &mut Writer) -> Result<(), Error> {
    let document = writer.document;
    let model_record = document.header.model_space_block_handle;
    let mut layouts: Vec<&Layout> = document
        .objects
        .values()
        .filter_map(|object| match object {
            ObjectType::Layout(layout) => Some(layout),
            _ => None,
        })
        .collect();
    layouts.sort_by_key(|layout| (layout.tab_order, layout.name.clone()));
    let mut tab_index = 0;
    for layout in layouts {
        if layout.block_record == model_record {
            writer.drawing.model.settings = settings_out(layout, true, &mut writer.losses);
            continue;
        }
        let Some(record) = document
            .block_records
            .iter()
            .find(|record| record.handle == layout.block_record)
        else {
            continue;
        };
        let id = writer.drawing.id_counters.allocate_layout_id()?;
        writer.layout_ids.insert(layout.handle, id);
        let overall = overall_viewport(document, layout);
        let mut canvas = None;
        let mut entities = Vec::new();
        for entity in document.entities_in_block(&record.name) {
            if let EntityType::Viewport(viewport) = entity {
                if Some(viewport.common.handle) == overall {
                    canvas = canvas_out(viewport, writer);
                    continue;
                }
            }
            if let Some(entity) = writer.entity(entity)? {
                entities.push(entity);
            }
        }
        if overall.is_none() {
            // Reading gives a sheet without one a default sheet viewport.
            let mut sheet = Viewport::new();
            sheet.id = 1;
            canvas = canvas_out(&sheet, writer);
        }
        tab_index += 1;
        writer.drawing.paper_layouts.push(IfccadPaperLayout {
            bounds_quality: None,
            canvas,
            id,
            name: layout.name.clone(),
            tab_index,
            settings: settings_out(layout, false, &mut writer.losses),
            bounds: None,
            entities,
        });
    }

    for vport in document
        .vports
        .iter()
        .filter(|vport| vport.name.eq_ignore_ascii_case("*Active"))
    {
        let Some(window) = model_window_out(vport, writer) else {
            writer.losses.note("a model window");
            continue;
        };
        writer.drawing.model_windows.push(window);
    }
    if let [window] = writer.drawing.model_windows.as_slice() {
        let id = window.id;
        writer
            .drawing
            .model_view_state
            .get_or_insert_with(IfccadModelViewState::default)
            .active_model_window_id = Some(id);
    }

    let header = &document.header;
    let active_layout_id = match header.show_model_space {
        true => Some(writer.drawing.model.id),
        false => document.objects.values().find_map(|object| match object {
            ObjectType::Layout(layout)
                if layout.block_record == header.paper_space_block_handle =>
            {
                writer.layout_ids.get(&layout.handle).copied()
            }
            _ => None,
        }),
    };
    writer.drawing.workspace_state = Some(IfccadDrawingWorkspaceState {
        current_layer_id: writer
            .layers
            .get(&header.current_layer_name.to_ascii_lowercase())
            .copied(),
        active_layout_id,
    });
    Ok(())
}

/// The paper-space viewport that is the sheet itself (number 1).
fn overall_viewport(document: &Document, layout: &Layout) -> Option<Handle> {
    let is_overall = |handle: &Handle| matches!(document.get_entity(*handle), Some(EntityType::Viewport(v)) if v.id == 1);
    if is_overall(&layout.viewport) {
        return Some(layout.viewport);
    }
    let record = document
        .block_records
        .iter()
        .find(|record| record.handle == layout.block_record)?;
    record.entity_handles.iter().copied().find(is_overall)
}

pub(super) fn viewport_out(writer: &mut Writer, viewport: &Viewport) -> Option<IfccadEntityKind> {
    let document = writer.document;
    // Viewports look into the model from a sheet; elsewhere they mean nothing.
    let on_sheet = document
        .block_records
        .iter()
        .any(|record| record.handle == viewport.common.owner_handle && record.is_paper_space());
    if !on_sheet || viewport.center.z != 0.0 || viewport.view_center.z != 0.0 {
        return None;
    }
    let layer_overrides = viewport
        .frozen_layers
        .iter()
        .filter_map(|handle| {
            let layer = document
                .layers
                .iter()
                .find(|layer| layer.handle == *handle)?;
            writer.layers.get(&layer.name.to_ascii_lowercase()).copied()
        })
        .map(|layer_id| IfccadViewportLayerOverride {
            layer_id,
            frozen: true,
            color: None,
            opacity: None,
            line_pattern_id: None,
            line_weight: None,
        })
        .collect();
    let status = &viewport.status;
    let converted = IfccadViewport {
        workspace: aids_out(viewport, writer).map(|(grid, snap, stored_ucs)| {
            Box::new(IfccadViewportWorkspace {
                grid,
                snap,
                stored_ucs,
                use_stored_ucs: viewport.ucs_per_viewport,
            })
        }),
        model_id: writer.drawing.model.id,
        frame: IfccadViewportFrame {
            center: [viewport.center.x, viewport.center.y],
            width: viewport.width,
            height: viewport.height,
        },
        view: IfccadViewportView {
            center: [viewport.view_center.x, viewport.view_center.y],
            target: array(viewport.view_target),
            direction: array(viewport.view_direction),
            height: viewport.view_height,
            twist: viewport.twist_angle,
            projection: match status.perspective {
                true => IfccadViewportProjection::Perspective,
                false => IfccadViewportProjection::Orthographic,
            },
            lens_length_mm: Some(viewport.lens_length),
            front_clip: IfccadViewportDepthClip {
                mode: match (status.front_clipping, status.front_clip_not_at_eye) {
                    (false, _) => IfccadViewportClipMode::Disabled,
                    (true, true) => IfccadViewportClipMode::AtDistance,
                    (true, false) => IfccadViewportClipMode::AtCamera,
                },
                distance: Some(viewport.front_clip_z),
            },
            back_clip: IfccadViewportDepthClip {
                mode: match status.back_clipping {
                    true => IfccadViewportClipMode::AtDistance,
                    false => IfccadViewportClipMode::Disabled,
                },
                distance: Some(viewport.back_clip_z),
            },
        },
        render_mode: render_mode_out(viewport.render_mode),
        view_enabled: status.is_on && !status.is_off,
        view_locked: status.locked,
        paper_clip: IfccadViewportPaperClip {
            enabled: status.non_rectangular_clipping,
            boundary_entity_id: None,
        },
        layer_overrides,
        plot_shading_override: match viewport.shade_plot_mode {
            1 => Some(ShadedPlotMode::Wireframe),
            2 => Some(ShadedPlotMode::Hidden),
            3 => Some(ShadedPlotMode::Rendered),
            _ => None,
        },
    };
    validate_ifccad_viewport_parameters(&converted).ok()?;
    if !viewport.clip_boundary_handle.is_null() {
        writer
            .viewport_clips
            .insert(viewport.common.handle, viewport.clip_boundary_handle);
    }
    Some(IfccadEntityKind::Viewport(converted))
}

/// Point each viewport at the entity that clips it, when it is in the same
/// layout.
pub(super) fn link_clips(writer: &mut Writer) {
    let ids = &writer.entity_ids;
    for layout in &mut writer.drawing.paper_layouts {
        let here: std::collections::HashSet<u64> =
            layout.entities.iter().map(IfccadEntity::id).collect();
        for (handle, boundary) in &writer.viewport_clips {
            let (Some(id), Some(boundary)) = (ids.get(handle), ids.get(boundary)) else {
                continue;
            };
            if !here.contains(boundary) {
                continue;
            }
            for entity in &mut layout.entities {
                if let IfccadEntity::Native(IfccadNativeEntity {
                    id: entity_id,
                    kind: IfccadEntityKind::Viewport(viewport),
                    ..
                }) = entity
                {
                    if entity_id == id {
                        viewport.paper_clip.boundary_entity_id = Some(*boundary);
                    }
                }
            }
        }
    }
}

fn canvas_out(viewport: &Viewport, writer: &Writer) -> Option<IfccadPaperCanvas> {
    let (grid, snap, stored_ucs) = aids_out(viewport, writer)?;
    let view = view_out(viewport);
    validate_view(&view, WorkspaceViewKind::PaperCanvas).ok()?;
    let frame = WorkspaceCanvasFrame {
        center: Point3::new(viewport.center.x, viewport.center.y, viewport.center.z),
        width: viewport.width,
        height: viewport.height,
    };
    Some(IfccadPaperCanvas {
        view,
        frame: validate_canvas_frame(&frame).is_ok().then_some(frame),
        grid,
        snap,
        stored_ucs,
        use_stored_ucs: viewport.ucs_per_viewport,
        current_ucs: None,
        active_context: None,
    })
}

fn view_out(viewport: &Viewport) -> WorkspaceView {
    let status = &viewport.status;
    WorkspaceView {
        center: Point2::new(viewport.view_center.x, viewport.view_center.y),
        target: point3(viewport.view_target),
        direction: kernel_vector(viewport.view_direction),
        height: viewport.view_height,
        twist: viewport.twist_angle,
        projection: match status.perspective {
            true => WorkspaceProjection::Perspective,
            false => WorkspaceProjection::Orthographic,
        },
        lens_length: Some(viewport.lens_length),
        front_clip: WorkspaceClip {
            mode: match (status.front_clipping, status.front_clip_not_at_eye) {
                (false, _) => WorkspaceClipMode::Disabled,
                (true, true) => WorkspaceClipMode::AtDistance,
                (true, false) => WorkspaceClipMode::AtCamera,
            },
            distance: Some(viewport.front_clip_z),
        },
        back_clip: WorkspaceClip {
            mode: match status.back_clipping {
                true => WorkspaceClipMode::AtDistance,
                false => WorkspaceClipMode::Disabled,
            },
            distance: Some(viewport.back_clip_z),
        },
    }
}

/// A viewport's grid, snap and stored UCS, if IFCX accepts them.
fn aids_out(
    viewport: &Viewport,
    writer: &Writer,
) -> Option<(WorkspaceGrid, WorkspaceSnap, IfccadUcsSelection)> {
    let status = &viewport.status;
    let grid = grid_out(
        status.grid_on,
        [viewport.grid_spacing.x, viewport.grid_spacing.y],
        viewport.grid_major,
        &viewport.grid_flags,
    )?;
    let snap = WorkspaceSnap {
        enabled: status.snap_on,
        base: Point2::new(viewport.snap_base.x, viewport.snap_base.y),
        spacing: Point2::new(viewport.snap_spacing.x, viewport.snap_spacing.y),
        angle: viewport.snap_angle,
        style: match status.isometric_snap {
            true => WorkspaceSnapStyle::Isometric,
            false => WorkspaceSnapStyle::Rectangular,
        },
        isometric_plane: match (status.iso_pair_top, status.iso_pair_right) {
            (true, false) => WorkspaceIsometricPlane::Top,
            (false, true) => WorkspaceIsometricPlane::Right,
            _ => WorkspaceIsometricPlane::Left,
        },
    };
    validate_snap(&snap).ok()?;
    let ucs = stored_ucs_out(
        viewport.ucs_handle,
        viewport.ucs_origin,
        viewport.ucs_x_axis,
        viewport.ucs_y_axis,
        writer,
    )?;
    Some((grid, snap, ucs))
}

fn grid_out(
    enabled: bool,
    spacing: [f64; 2],
    major: i16,
    flags: &GridFlags,
) -> Option<WorkspaceGrid> {
    let grid = WorkspaceGrid {
        enabled,
        spacing: Point2::new(spacing[0], spacing[1]),
        style: WorkspaceGridStyle::Lines,
        major_line_frequency: u32::try_from(major).ok().filter(|f| *f > 0).unwrap_or(5),
        beyond_limits: flags.beyond_limits,
        adaptive: flags.adaptive,
        subdivision: flags.subdivision,
        follows_workplane: flags.follow_dynamic,
    };
    validate_grid(&grid).ok().map(|_| grid)
}

/// A stored UCS: the named one its handle refers to, else its frame.
fn stored_ucs_out(
    handle: Handle,
    origin: Vector3,
    x: Vector3,
    y: Vector3,
    writer: &Writer,
) -> Option<IfccadUcsSelection> {
    let named = writer
        .document
        .ucss
        .iter()
        .find(|ucs| !handle.is_null() && ucs.handle == handle)
        .and_then(|ucs| writer.ucs_ids.get(&ucs.name.to_ascii_lowercase()));
    if let Some(id) = named {
        return Some(IfccadUcsSelection::Named(*id));
    }
    let frame =
        CoordinateFrame3::try_new(point3(origin), kernel_vector(x), kernel_vector(y)).ok()?;
    Some(match frame == CoordinateFrame3::default() {
        true => IfccadUcsSelection::World,
        false => IfccadUcsSelection::Unnamed(frame),
    })
}

fn model_window_out(vport: &VPort, writer: &mut Writer) -> Option<IfccadModelWindow> {
    let grid = grid_out(
        vport.grid_on,
        [vport.grid_spacing.x, vport.grid_spacing.y],
        vport.grid_major,
        &vport.grid_flags,
    )?;
    let snap = WorkspaceSnap {
        enabled: vport.snap_on,
        base: Point2::new(vport.snap_base.x, vport.snap_base.y),
        spacing: Point2::new(vport.snap_spacing.x, vport.snap_spacing.y),
        angle: vport.snap_rotation,
        style: match vport.snap_style {
            true => WorkspaceSnapStyle::Isometric,
            false => WorkspaceSnapStyle::Rectangular,
        },
        isometric_plane: match vport.snap_isopair {
            1 => WorkspaceIsometricPlane::Top,
            2 => WorkspaceIsometricPlane::Right,
            _ => WorkspaceIsometricPlane::Left,
        },
    };
    validate_snap(&snap).ok()?;
    let view = WorkspaceView {
        center: Point2::new(vport.view_center.x, vport.view_center.y),
        target: point3(vport.view_target),
        direction: kernel_vector(vport.view_direction),
        height: vport.view_height,
        twist: vport.view_twist,
        projection: match vport.perspective {
            true => WorkspaceProjection::Perspective,
            false => WorkspaceProjection::Orthographic,
        },
        lens_length: Some(vport.lens_length),
        front_clip: WorkspaceClip {
            mode: match (vport.front_clipping, vport.front_clip_at_eye) {
                (false, _) => WorkspaceClipMode::Disabled,
                (true, true) => WorkspaceClipMode::AtCamera,
                (true, false) => WorkspaceClipMode::AtDistance,
            },
            distance: Some(vport.front_clip),
        },
        back_clip: WorkspaceClip {
            mode: match vport.back_clipping {
                true => WorkspaceClipMode::AtDistance,
                false => WorkspaceClipMode::Disabled,
            },
            distance: Some(vport.back_clip),
        },
    };
    validate_view(&view, WorkspaceViewKind::Model).ok()?;
    let rectangle = [
        vport.lower_left.x,
        vport.lower_left.y,
        vport.upper_right.x,
        vport.upper_right.y,
    ];
    let valid_rectangle = rectangle.iter().all(|v| (0.0..=1.0).contains(v))
        && rectangle[0] < rectangle[2]
        && rectangle[1] < rectangle[3]
        && vport.aspect_ratio > 0.0
        && vport.aspect_ratio.is_finite();
    if !valid_rectangle {
        return None;
    }
    let stored_ucs = stored_ucs_out(
        vport.named_ucs_handle,
        vport.ucs_origin,
        vport.ucs_x_axis,
        vport.ucs_y_axis,
        writer,
    )?;
    Some(IfccadModelWindow {
        id: writer.drawing.id_counters.allocate_model_window_id().ok()?,
        rectangle,
        view,
        aspect_ratio: vport.aspect_ratio,
        render_mode: workspace_render_mode(render_mode_out(vport.render_mode)),
        grid,
        snap,
        stored_ucs,
        use_stored_ucs: vport.ucs_per_viewport,
    })
}

fn render_mode_out(mode: ViewportRenderMode) -> IfccadViewportRenderMode {
    match mode {
        ViewportRenderMode::Wireframe2D => IfccadViewportRenderMode::TwoDimensional,
        ViewportRenderMode::Wireframe3D => IfccadViewportRenderMode::Wireframe,
        ViewportRenderMode::HiddenLine => IfccadViewportRenderMode::HiddenLine,
        ViewportRenderMode::FlatShaded => IfccadViewportRenderMode::FlatShadedWithoutEdges,
        ViewportRenderMode::FlatShadedWithEdges => IfccadViewportRenderMode::FlatShadedWithEdges,
        ViewportRenderMode::GouraudShaded => IfccadViewportRenderMode::SmoothShadedWithoutEdges,
        ViewportRenderMode::GouraudShadedWithEdges => {
            IfccadViewportRenderMode::SmoothShadedWithEdges
        }
    }
}

fn render_mode_in(mode: IfccadViewportRenderMode) -> ViewportRenderMode {
    match mode {
        IfccadViewportRenderMode::TwoDimensional => ViewportRenderMode::Wireframe2D,
        IfccadViewportRenderMode::Wireframe => ViewportRenderMode::Wireframe3D,
        IfccadViewportRenderMode::HiddenLine => ViewportRenderMode::HiddenLine,
        IfccadViewportRenderMode::FlatShadedWithoutEdges => ViewportRenderMode::FlatShaded,
        IfccadViewportRenderMode::FlatShadedWithEdges => ViewportRenderMode::FlatShadedWithEdges,
        IfccadViewportRenderMode::SmoothShadedWithoutEdges => ViewportRenderMode::GouraudShaded,
        IfccadViewportRenderMode::SmoothShadedWithEdges => {
            ViewportRenderMode::GouraudShadedWithEdges
        }
    }
}

fn workspace_render_mode(mode: IfccadViewportRenderMode) -> WorkspaceRenderMode {
    match mode {
        IfccadViewportRenderMode::TwoDimensional => WorkspaceRenderMode::TwoDimensional,
        IfccadViewportRenderMode::Wireframe => WorkspaceRenderMode::Wireframe,
        IfccadViewportRenderMode::HiddenLine => WorkspaceRenderMode::HiddenLine,
        IfccadViewportRenderMode::FlatShadedWithoutEdges => {
            WorkspaceRenderMode::FlatShadedWithoutEdges
        }
        IfccadViewportRenderMode::FlatShadedWithEdges => WorkspaceRenderMode::FlatShadedWithEdges,
        IfccadViewportRenderMode::SmoothShadedWithoutEdges => {
            WorkspaceRenderMode::SmoothShadedWithoutEdges
        }
        IfccadViewportRenderMode::SmoothShadedWithEdges => {
            WorkspaceRenderMode::SmoothShadedWithEdges
        }
    }
}

fn viewport_render_mode(mode: WorkspaceRenderMode) -> ViewportRenderMode {
    match mode {
        WorkspaceRenderMode::TwoDimensional => ViewportRenderMode::Wireframe2D,
        WorkspaceRenderMode::Wireframe => ViewportRenderMode::Wireframe3D,
        WorkspaceRenderMode::HiddenLine => ViewportRenderMode::HiddenLine,
        WorkspaceRenderMode::FlatShadedWithoutEdges => ViewportRenderMode::FlatShaded,
        WorkspaceRenderMode::FlatShadedWithEdges => ViewportRenderMode::FlatShadedWithEdges,
        WorkspaceRenderMode::SmoothShadedWithoutEdges => ViewportRenderMode::GouraudShaded,
        WorkspaceRenderMode::SmoothShadedWithEdges => ViewportRenderMode::GouraudShadedWithEdges,
    }
}

// ---------------------------------------------------------------- reading

/// Creates the paper layouts and applies every layout's settings; returns
/// each paper layout's block record, to own its entities.
pub(super) fn read(
    drawing: &IfccadDocument,
    document: &mut Document,
    losses: &mut Losses,
) -> Result<Vec<Handle>, Error> {
    if let Some(model) = layout_handle(document, document.header.model_space_block_handle) {
        if let Some(ObjectType::Layout(layout)) = document.objects.get_mut(&model) {
            settings_in(layout, &drawing.model.settings, losses);
        }
    }
    let mut papers: Vec<&IfccadPaperLayout> = drawing.paper_layouts.iter().collect();
    papers.sort_by_key(|paper| paper.tab_index);
    let mut owners = Vec::new();
    for (index, paper) in papers.into_iter().enumerate() {
        // The first sheet takes over the empty one every new drawing has.
        let handle = match (
            index,
            layout_handle(document, document.header.paper_space_block_handle),
        ) {
            (0, Some(handle)) => {
                rename_layout(document, handle, &paper.name);
                handle
            }
            _ => document.add_layout(&paper.name)?,
        };
        let Some(ObjectType::Layout(layout)) = document.objects.get_mut(&handle) else {
            continue;
        };
        layout.tab_order = i16::try_from(paper.tab_index).unwrap_or(i16::MAX);
        settings_in(layout, &paper.settings, losses);
        owners.push(layout.block_record);
    }
    // Every sheet, the untouched one of a new drawing too, gets its markers
    // and its sheet viewport, as a written and re-read drawing has them.
    let sheets: Vec<(Handle, Handle)> = document
        .objects
        .iter()
        .filter_map(|(handle, object)| match object {
            ObjectType::Layout(layout)
                if layout.block_record != document.header.model_space_block_handle =>
            {
                Some((*handle, layout.block_record))
            }
            _ => None,
        })
        .collect();
    for (handle, record) in sheets {
        ensure_markers(document, record)?;
        if overall_viewport_in(document, handle).is_none() {
            let mut viewport = Viewport::new();
            viewport.id = 1;
            viewport.common.owner_handle = record;
            let viewport = document.add_entity(EntityType::Viewport(Box::new(viewport)))?;
            if let Some(ObjectType::Layout(layout)) = document.objects.get_mut(&handle) {
                layout.viewport = viewport;
                layout.viewports.insert(0, viewport);
            }
        }
    }
    Ok(owners)
}

/// After the entities: the paper canvases and viewport aids, viewport
/// numbers and lists, model windows and the workspace state.
pub(super) fn finish(
    drawing: &IfccadDocument,
    document: &mut Document,
    names: &Names,
    owners: &[Handle],
    handles: &HashMap<u64, Handle>,
) -> Result<(), Error> {
    let mut papers: Vec<&IfccadPaperLayout> = drawing.paper_layouts.iter().collect();
    papers.sort_by_key(|paper| paper.tab_index);
    for (paper, record) in papers.into_iter().zip(owners) {
        let Some(layout) = layout_handle(document, *record) else {
            continue;
        };
        let overall = overall_viewport_in(document, layout);
        if let (Some(canvas), Some(overall)) = (&paper.canvas, overall) {
            let ucs = stored_ucs_in(canvas.stored_ucs, drawing, document);
            if let Some(EntityType::Viewport(viewport)) = document.get_entity_mut(overall) {
                canvas_in(canvas, ucs, viewport);
            }
        }
        // Viewports: their aids and clip boundaries, numbered after the sheet.
        let mut number = 1;
        let mut viewports = overall.into_iter().collect::<Vec<_>>();
        for entity in paper.entities.iter().filter_map(IfccadEntity::as_native) {
            let IfccadEntityKind::Viewport(source) = &entity.kind else {
                continue;
            };
            let Some(handle) = handles.get(&entity.id).copied() else {
                continue;
            };
            let ucs = source
                .workspace
                .as_ref()
                .map(|workspace| stored_ucs_in(workspace.stored_ucs, drawing, document));
            let boundary = source
                .paper_clip
                .boundary_entity_id
                .and_then(|id| handles.get(&id).copied());
            if let Some(EntityType::Viewport(viewport)) = document.get_entity_mut(handle) {
                number += 1;
                viewport.id = number;
                if let (Some(workspace), Some(ucs)) = (&source.workspace, ucs) {
                    aids_in(
                        &workspace.grid,
                        &workspace.snap,
                        workspace.use_stored_ucs,
                        ucs,
                        viewport,
                    );
                }
                if let Some(boundary) = boundary {
                    viewport.clip_boundary_handle = boundary;
                }
                viewports.push(handle);
            }
        }
        if let Some(ObjectType::Layout(layout)) = document.objects.get_mut(&layout) {
            if let Some(overall) = overall {
                layout.viewport = overall;
            }
            layout.viewports = viewports;
        }
    }

    if !drawing.model_windows.is_empty() {
        let active = drawing
            .model_view_state
            .as_ref()
            .and_then(|state| state.active_model_window_id);
        let mut windows: Vec<&IfccadModelWindow> = drawing.model_windows.iter().collect();
        windows.sort_by_key(|window| Some(window.id) != active);
        let mut vports = Vec::new();
        for window in windows {
            let ucs = stored_ucs_in(window.stored_ucs, drawing, document);
            let mut vport = VPort::active();
            vport.handle = document.allocate_handle();
            model_window_in(window, ucs, &mut vport);
            vports.push(vport);
        }
        document.vports.clear();
        for vport in vports {
            document.vports.add_allow_duplicate(vport);
        }
    }

    if let Some(state) = &drawing.workspace_state {
        if let Some(name) = state.current_layer_id.and_then(|id| names.layers.get(&id)) {
            if let Some(layer) = document.layers.get(name) {
                let handle = layer.handle;
                document.header.current_layer_name = name.clone();
                document.header.current_layer_handle = handle;
            }
        }
        if let Some(id) = state.active_layout_id {
            document.header.show_model_space = id == drawing.model.id;
            let active = drawing
                .paper_layouts
                .iter()
                .find(|paper| paper.id == id)
                .and_then(|paper| owners.get(position_by_tab(drawing, paper)));
            if let Some(record) = active {
                activate_paper(document, *record)?;
            }
        }
    }
    Ok(())
}

/// Where `paper` stands among the paper layouts in tab order.
fn position_by_tab(drawing: &IfccadDocument, paper: &IfccadPaperLayout) -> usize {
    drawing
        .paper_layouts
        .iter()
        .filter(|other| other.tab_index < paper.tab_index)
        .count()
}

fn layout_handle(document: &Document, record: Handle) -> Option<Handle> {
    document
        .objects
        .iter()
        .find_map(|(handle, object)| match object {
            ObjectType::Layout(layout) if layout.block_record == record => Some(*handle),
            _ => None,
        })
}

fn overall_viewport_in(document: &Document, layout: Handle) -> Option<Handle> {
    let Some(ObjectType::Layout(layout)) = document.objects.get(&layout) else {
        return None;
    };
    overall_viewport(document, layout)
}

fn rename_layout(document: &mut Document, handle: Handle, name: &str) {
    let dictionary = document.header.acad_layout_dict_handle;
    let Some(ObjectType::Layout(layout)) = document.objects.get_mut(&handle) else {
        return;
    };
    let old = std::mem::replace(&mut layout.name, name.to_string());
    if let Some(ObjectType::Dictionary(dictionary)) = document.objects.get_mut(&dictionary) {
        for (entry, target) in &mut dictionary.entries {
            if *target == handle && *entry == old {
                *entry = name.to_string();
            }
        }
    }
}

/// BLOCK and ENDBLK markers owned by a paper layout's record, so that a
/// DWG reader can tell its sheets apart.
fn ensure_markers(document: &mut Document, record: Handle) -> Result<(), Error> {
    let Some(block) = document.block_records.iter().find(|r| r.handle == record) else {
        return Ok(());
    };
    let (name, base, begin, end) = (
        block.name.clone(),
        block.base_point,
        block.block_entity_handle,
        block.block_end_handle,
    );
    if !begin.is_null() && document.get_entity(begin).is_none() {
        let mut marker = Block::new(name, base);
        marker.common.handle = begin;
        marker.common.owner_handle = record;
        marker.common.entity_mode = Some(0);
        document.add_entity(EntityType::Block(marker))?;
    }
    if !end.is_null() && document.get_entity(end).is_none() {
        let mut marker = BlockEnd::new();
        marker.common.handle = end;
        marker.common.owner_handle = record;
        marker.common.entity_mode = Some(0);
        document.add_entity(EntityType::BlockEnd(marker))?;
    }
    Ok(())
}

/// Make `record`'s layout the active paper space: it takes the reserved
/// record name, keeping every handle and owner.
fn activate_paper(document: &mut Document, record: Handle) -> Result<(), Error> {
    let Some(active) = document.block_records.get("*Paper_Space").map(|r| r.handle) else {
        return Ok(());
    };
    if active != record {
        let Some(selected) = document
            .block_records
            .iter()
            .find(|r| r.handle == record)
            .map(|r| r.name.clone())
        else {
            return Ok(());
        };
        let mut temporary = "__active_paper_swap".to_string();
        while document.block_records.contains(&temporary) {
            temporary.push('_');
        }
        let rename = |document: &mut Document, from: &str, to: &str| {
            document
                .block_records
                .rename(from, to.to_string())
                .map_err(Error::from)
        };
        rename(document, "*Paper_Space", &temporary)?;
        rename(document, &selected, "*Paper_Space")?;
        rename(document, &temporary, &selected)?;
        let markers: Vec<_> = document
            .block_records
            .iter()
            .filter(|r| r.is_paper_space())
            .map(|r| (r.block_entity_handle, r.name.clone()))
            .collect();
        for (handle, name) in markers {
            if let Some(EntityType::Block(marker)) = document.get_entity_mut(handle) {
                marker.name = name;
            }
        }
    }
    document.header.paper_space_block_handle = record;
    Ok(())
}

pub(super) fn viewport_in(source: &IfccadViewport, names: &Names) -> EntityType {
    let mut viewport = Viewport::new();
    viewport.center = Vector3::new(source.frame.center[0], source.frame.center[1], 0.0);
    viewport.width = source.frame.width;
    viewport.height = source.frame.height;
    let view = &source.view;
    viewport.view_center = Vector3::new(view.center[0], view.center[1], 0.0);
    viewport.view_target = vector(view.target);
    viewport.view_direction = vector(view.direction);
    viewport.view_height = view.height;
    viewport.twist_angle = view.twist;
    if let Some(lens) = view.lens_length_mm {
        viewport.lens_length = lens;
    }
    if let Some(distance) = view.front_clip.distance {
        viewport.front_clip_z = distance;
    }
    if let Some(distance) = view.back_clip.distance {
        viewport.back_clip_z = distance;
    }
    let status = &mut viewport.status;
    status.perspective = view.projection == IfccadViewportProjection::Perspective;
    status.front_clipping = view.front_clip.mode != IfccadViewportClipMode::Disabled;
    status.front_clip_not_at_eye = view.front_clip.mode == IfccadViewportClipMode::AtDistance;
    status.back_clipping = view.back_clip.mode != IfccadViewportClipMode::Disabled;
    status.locked = source.view_locked;
    status.is_on = true;
    status.is_off = !source.view_enabled;
    status.non_rectangular_clipping = source.paper_clip.enabled;
    viewport.render_mode = render_mode_in(source.render_mode);
    viewport.shade_plot_mode = match source.plot_shading_override {
        None | Some(ShadedPlotMode::AsDisplayed) => 0,
        Some(ShadedPlotMode::Wireframe) => 1,
        Some(ShadedPlotMode::Hidden) => 2,
        Some(ShadedPlotMode::Rendered) => 3,
    };
    // Frozen layers by name now; their handles once the layers exist.
    viewport.frozen_layers = source
        .layer_overrides
        .iter()
        .filter(|row| row.frozen)
        .filter_map(|row| names.layer_handles.get(&row.layer_id).copied())
        .collect();
    EntityType::Viewport(Box::new(viewport))
}

/// Whether a viewport carries layer overrides beyond freezing.
pub(super) fn has_property_overrides(source: &IfccadViewport) -> bool {
    source.layer_overrides.iter().any(|row| {
        row.color.is_some()
            || row.opacity.is_some()
            || row.line_pattern_id.is_some()
            || row.line_weight.is_some()
    })
}

fn canvas_in(canvas: &IfccadPaperCanvas, ucs: StoredUcs, viewport: &mut Viewport) {
    aids_in(
        &canvas.grid,
        &canvas.snap,
        canvas.use_stored_ucs,
        ucs,
        viewport,
    );
    if let Some(frame) = canvas.frame {
        viewport.center = vector(frame.center.components());
        viewport.width = frame.width;
        viewport.height = frame.height;
    }
    let view = &canvas.view;
    viewport.view_center = Vector3::new(view.center.x(), view.center.y(), 0.0);
    viewport.view_target = vector(view.target.components());
    viewport.view_direction = vector(view.direction.components());
    viewport.view_height = view.height;
    viewport.twist_angle = view.twist;
    viewport.lens_length = view.lens_length.unwrap_or(50.0);
    viewport.status.perspective = view.projection == WorkspaceProjection::Perspective;
    viewport.status.front_clipping = view.front_clip.mode != WorkspaceClipMode::Disabled;
    viewport.status.front_clip_not_at_eye = view.front_clip.mode == WorkspaceClipMode::AtDistance;
    viewport.front_clip_z = view.front_clip.distance.unwrap_or(0.0);
    viewport.status.back_clipping = view.back_clip.mode != WorkspaceClipMode::Disabled;
    viewport.back_clip_z = view.back_clip.distance.unwrap_or(0.0);
}

/// A stored UCS as the handle of a named one plus its frame and elevation.
struct StoredUcs {
    handle: Handle,
    origin: Vector3,
    x: Vector3,
    y: Vector3,
    elevation: f64,
}

fn stored_ucs_in(
    selection: IfccadUcsSelection,
    drawing: &IfccadDocument,
    document: &Document,
) -> StoredUcs {
    let frame = |frame: CoordinateFrame3| {
        (
            vector(frame.origin().components()),
            vector(frame.x_axis().components()),
            vector(frame.y_axis().components()),
        )
    };
    let (handle, (origin, x, y), elevation) = match selection {
        IfccadUcsSelection::World => (Handle::NULL, frame(CoordinateFrame3::default()), 0.0),
        IfccadUcsSelection::Unnamed(value) => (Handle::NULL, frame(value), 0.0),
        IfccadUcsSelection::Named(id) => {
            match drawing.ucs_definitions.iter().find(|u| u.id == id) {
                Some(ucs) => (
                    document
                        .ucss
                        .get(&ucs.name)
                        .map_or(Handle::NULL, |u| u.handle),
                    frame(ucs.frame),
                    ucs.elevation,
                ),
                None => (Handle::NULL, frame(CoordinateFrame3::default()), 0.0),
            }
        }
    };
    StoredUcs {
        handle,
        origin,
        x,
        y,
        elevation,
    }
}

fn aids_in(
    grid: &WorkspaceGrid,
    snap: &WorkspaceSnap,
    use_stored_ucs: bool,
    ucs: StoredUcs,
    viewport: &mut Viewport,
) {
    viewport.status.grid_on = grid.enabled;
    viewport.grid_spacing = Vector3::new(grid.spacing.x(), grid.spacing.y(), 0.0);
    viewport.grid_major = i16::try_from(grid.major_line_frequency).unwrap_or(5);
    viewport.grid_flags = grid_flags(grid);
    viewport.status.snap_on = snap.enabled;
    viewport.snap_base = Vector3::new(snap.base.x(), snap.base.y(), 0.0);
    viewport.snap_spacing = Vector3::new(snap.spacing.x(), snap.spacing.y(), 0.0);
    viewport.snap_angle = snap.angle;
    viewport.status.isometric_snap = snap.style == WorkspaceSnapStyle::Isometric;
    viewport.status.iso_pair_top = snap.isometric_plane == WorkspaceIsometricPlane::Top;
    viewport.status.iso_pair_right = snap.isometric_plane == WorkspaceIsometricPlane::Right;
    viewport.ucs_handle = ucs.handle;
    viewport.ucs_origin = ucs.origin;
    viewport.ucs_x_axis = ucs.x;
    viewport.ucs_y_axis = ucs.y;
    viewport.elevation = ucs.elevation;
    viewport.ucs_per_viewport = use_stored_ucs;
}

fn model_window_in(window: &IfccadModelWindow, ucs: StoredUcs, vport: &mut VPort) {
    let [left, bottom, right, top] = window.rectangle;
    vport.lower_left = Vector2::new(left, bottom);
    vport.upper_right = Vector2::new(right, top);
    let view = &window.view;
    vport.view_center = Vector2::new(view.center.x(), view.center.y());
    vport.view_target = vector(view.target.components());
    vport.view_direction = vector(view.direction.components());
    vport.view_height = view.height;
    vport.view_twist = view.twist;
    vport.aspect_ratio = window.aspect_ratio;
    vport.perspective = view.projection == WorkspaceProjection::Perspective;
    vport.lens_length = view.lens_length.unwrap_or(50.0);
    vport.front_clipping = view.front_clip.mode != WorkspaceClipMode::Disabled;
    vport.front_clip_at_eye = view.front_clip.mode == WorkspaceClipMode::AtCamera;
    vport.front_clip = view.front_clip.distance.unwrap_or(0.0);
    vport.back_clipping = view.back_clip.mode != WorkspaceClipMode::Disabled;
    vport.back_clip = view.back_clip.distance.unwrap_or(0.0);
    vport.render_mode = viewport_render_mode(window.render_mode);
    vport.grid_on = window.grid.enabled;
    vport.grid_spacing = Vector2::new(window.grid.spacing.x(), window.grid.spacing.y());
    vport.grid_major = i16::try_from(window.grid.major_line_frequency).unwrap_or(5);
    vport.grid_flags = grid_flags(&window.grid);
    vport.snap_on = window.snap.enabled;
    vport.snap_rotation = window.snap.angle;
    vport.snap_base = Vector2::new(window.snap.base.x(), window.snap.base.y());
    vport.snap_spacing = Vector2::new(window.snap.spacing.x(), window.snap.spacing.y());
    vport.snap_style = window.snap.style == WorkspaceSnapStyle::Isometric;
    vport.snap_isopair = match window.snap.isometric_plane {
        WorkspaceIsometricPlane::Left => 0,
        WorkspaceIsometricPlane::Top => 1,
        WorkspaceIsometricPlane::Right => 2,
    };
    vport.named_ucs_handle = ucs.handle;
    vport.ucs_origin = ucs.origin;
    vport.ucs_x_axis = ucs.x;
    vport.ucs_y_axis = ucs.y;
    vport.ucs_elevation = ucs.elevation;
    vport.ucs_per_viewport = window.use_stored_ucs;
}

fn grid_flags(grid: &WorkspaceGrid) -> GridFlags {
    GridFlags {
        beyond_limits: grid.beyond_limits,
        adaptive: grid.adaptive,
        subdivision: grid.subdivision,
        follow_dynamic: grid.follows_workplane,
    }
}

fn point3(v: Vector3) -> Point3 {
    Point3::new(v.x, v.y, v.z)
}

fn kernel_vector(v: Vector3) -> ifccad::geometry_kernel::Vector3 {
    ifccad::geometry_kernel::Vector3::new(v.x, v.y, v.z)
}

// ---------------------------------------------------------------- sheet and plot

/// A layout's sheet, limits and plot settings.
fn settings_out(layout: &Layout, model: bool, losses: &mut Losses) -> IfccadLayoutSettings {
    let media = (layout.paper_width > 0.0 && layout.paper_height > 0.0).then_some(LayoutMedia {
        unit: millimetres(),
        width: layout.paper_width,
        height: layout.paper_height,
    });
    let limits = (layout.min_limits != layout.max_limits)
        .then(|| {
            rect(
                layout.min_limits.0,
                layout.min_limits.1,
                layout.max_limits.0,
                layout.max_limits.1,
            )
        })
        .flatten();
    let plot_settings = media
        .as_ref()
        .and_then(|media| plot_out(layout, media, model));
    let has_media = media.is_some();
    let mut settings = IfccadLayoutSettings {
        media,
        limits,
        limits_checking: layout.flags & 2 != 0,
        paper_space_linetype_scaling: layout.flags & 1 != 0,
        plot_settings,
    };
    let kind = match model {
        true => LayoutOutputKind::Model,
        false => LayoutOutputKind::Paper,
    };
    if settings.plot_settings.is_some() && validate_layout_output(&settings, kind).is_err() {
        settings.plot_settings = None;
    }
    if settings.plot_settings.is_none() && has_media && !plot_is_default(layout) {
        losses.note(format!("the plot settings of layout {}", layout.name));
    }
    if validate_layout_output(&settings, kind).is_err() {
        settings.media = None;
        settings.limits = None;
    }
    settings
}

fn settings_in(layout: &mut Layout, settings: &IfccadLayoutSettings, losses: &mut Losses) {
    layout.flags = (layout.flags & !3)
        | i16::from(settings.paper_space_linetype_scaling)
        | (i16::from(settings.limits_checking) << 1);
    if let Some(limits) = settings.limits {
        layout.min_limits = (limits.min_x, limits.min_y);
        layout.max_limits = (limits.max_x, limits.max_y);
    }
    let Some(media) = &settings.media else {
        return;
    };
    let Some(to_mm) = millimetres_per(media.unit) else {
        losses.note(format!("the sheet of layout {}", layout.name));
        return;
    };
    layout.paper_width = settle(media.width * to_mm);
    layout.paper_height = settle(media.height * to_mm);
    if let Some(plot) = &settings.plot_settings {
        plot_in(layout, media.width, media.height, to_mm, plot);
    }
}

fn plot_out(layout: &Layout, media: &LayoutMedia, model: bool) -> Option<PlotSettings> {
    if plot_is_default(layout) {
        return None;
    }
    let unit = match layout.plot_paper_units {
        0 => PlotUnit::Inch,
        1 => PlotUnit::Millimetre,
        _ => return None,
    };
    let area = match layout.plot_type {
        1 => PlotArea::Extents,
        2 if model => PlotArea::Limits,
        4 => PlotArea::Window(rect(
            layout.plot_window_min_x,
            layout.plot_window_min_y,
            layout.plot_window_max_x,
            layout.plot_window_max_y,
        )?),
        5 if !model => PlotArea::Layout,
        _ => return None,
    };
    let scale = plot_scale_out(layout)?;
    let output_mm = match unit {
        PlotUnit::Inch => 25.4,
        _ => 1.0,
    };
    let placement = match layout.plot_flags.plot_centered {
        true => PlotPlacement::Centered,
        false => PlotPlacement::Offset {
            reference: PlotOffsetReference::Media,
            x: settle(layout.plot_origin_x / output_mm),
            y: settle(layout.plot_origin_y / output_mm),
        },
    };
    let quality = match layout.shade_plot_resolution {
        0 => ShadedPlotQualityMode::Draft,
        1 => ShadedPlotQualityMode::Preview,
        2 => ShadedPlotQualityMode::Normal,
        3 => ShadedPlotQualityMode::Presentation,
        4 => ShadedPlotQualityMode::Maximum,
        5 => ShadedPlotQualityMode::Custom,
        _ => return None,
    };
    let flags = &layout.plot_flags;
    Some(PlotSettings {
        plot_unit: unit,
        page: PlotPage {
            printable_area: rect(
                layout.plot_margin_left,
                layout.plot_margin_bottom,
                settle(media.width - layout.plot_margin_right),
                settle(media.height - layout.plot_margin_top),
            )?,
            rotation: match layout.plot_rotation {
                1 => PlotRotation::CounterClockwise90,
                2 => PlotRotation::UpsideDown,
                3 => PlotRotation::Clockwise90,
                _ => PlotRotation::None,
            },
            device_name: nonempty(&layout.plot_printer_name),
            media_name: nonempty(&layout.paper_size),
        },
        area,
        mapping: PlotMapping { scale, placement },
        output: PlotOutput {
            shaded_plot: ShadedPlot {
                mode: match layout.shade_plot_mode {
                    1 => ShadedPlotMode::Wireframe,
                    2 => ShadedPlotMode::Hidden,
                    3 => ShadedPlotMode::Rendered,
                    _ => ShadedPlotMode::AsDisplayed,
                },
                quality: ShadedPlotQuality {
                    mode: quality,
                    dpi: (quality == ShadedPlotQualityMode::Custom)
                        .then(|| u32::try_from(layout.shade_plot_dpi).ok())
                        .flatten()
                        .filter(|dpi| (100..=32767).contains(dpi)),
                },
            },
            apply_plot_styles: flags.plot_plot_styles,
            plot_style_table_name: nonempty(&layout.plot_style_sheet),
        },
        options: PlotOptions {
            plot_viewport_borders: flags.plot_viewport_borders,
            plot_paper_space_last: flags.draw_viewports_first,
            hide_paper_space_objects: flags.plot_hidden,
            plot_line_weights: flags.print_lineweights,
            scale_line_weights: flags.scale_lineweights,
            plot_transparency: false,
        },
    })
}

fn plot_in(layout: &mut Layout, width: f64, height: f64, to_mm: f64, plot: &PlotSettings) {
    let output_mm = match plot.plot_unit {
        PlotUnit::Inch => 25.4,
        _ => 1.0,
    };
    layout.plot_paper_units = match plot.plot_unit {
        PlotUnit::Inch => 0,
        PlotUnit::Millimetre => 1,
        PlotUnit::Pixel => 2,
    };
    let page = &plot.page;
    layout.plot_rotation = match page.rotation {
        PlotRotation::None => 0,
        PlotRotation::CounterClockwise90 => 1,
        PlotRotation::UpsideDown => 2,
        PlotRotation::Clockwise90 => 3,
    };
    layout.plot_printer_name = page.device_name.clone().unwrap_or_default();
    layout.paper_size = page.media_name.clone().unwrap_or_default();
    let area = page.printable_area;
    layout.plot_margin_left = area.min_x * to_mm;
    layout.plot_margin_bottom = area.min_y * to_mm;
    layout.plot_margin_right = (width - area.max_x) * to_mm;
    layout.plot_margin_top = (height - area.max_y) * to_mm;
    layout.plot_type = match plot.area {
        PlotArea::Extents => 1,
        PlotArea::Limits => 2,
        PlotArea::Window(window) => {
            layout.plot_window_min_x = window.min_x;
            layout.plot_window_min_y = window.min_y;
            layout.plot_window_max_x = window.max_x;
            layout.plot_window_max_y = window.max_y;
            4
        }
        PlotArea::Layout => 5,
    };
    match plot.mapping.scale {
        PlotScale::FitToArea => {
            layout.plot_scale_type = 0;
            layout.plot_flags.use_standard_scale = true;
        }
        PlotScale::Fixed {
            output_length,
            scope_length,
        } => {
            layout.plot_scale_type = 0;
            layout.plot_flags.use_standard_scale = false;
            layout.plot_scale_numerator = output_length;
            layout.plot_scale_denominator = scope_length;
            layout.plot_scale_factor = output_length / scope_length;
        }
    }
    layout.plot_flags.plot_centered = plot.mapping.placement == PlotPlacement::Centered;
    if let PlotPlacement::Offset { x, y, .. } = plot.mapping.placement {
        layout.plot_origin_x = x * output_mm;
        layout.plot_origin_y = y * output_mm;
    }
    let shaded = &plot.output.shaded_plot;
    layout.shade_plot_mode = match shaded.mode {
        ShadedPlotMode::AsDisplayed => 0,
        ShadedPlotMode::Wireframe => 1,
        ShadedPlotMode::Hidden => 2,
        ShadedPlotMode::Rendered => 3,
    };
    layout.shade_plot_resolution = match shaded.quality.mode {
        ShadedPlotQualityMode::Draft => 0,
        ShadedPlotQualityMode::Preview => 1,
        ShadedPlotQualityMode::Normal => 2,
        ShadedPlotQualityMode::Presentation => 3,
        ShadedPlotQualityMode::Maximum => 4,
        ShadedPlotQualityMode::Custom => 5,
    };
    if let Some(dpi) = shaded.quality.dpi {
        layout.shade_plot_dpi = i16::try_from(dpi).unwrap_or(i16::MAX);
    }
    layout.plot_flags.plot_plot_styles = plot.output.apply_plot_styles;
    layout.plot_style_sheet = plot
        .output
        .plot_style_table_name
        .clone()
        .unwrap_or_default();
    let options = plot.options;
    layout.plot_flags.plot_viewport_borders = options.plot_viewport_borders;
    layout.plot_flags.draw_viewports_first = options.plot_paper_space_last;
    layout.plot_flags.plot_hidden = options.hide_paper_space_objects;
    layout.plot_flags.print_lineweights = options.plot_line_weights;
    layout.plot_flags.scale_lineweights = options.scale_line_weights;
}

/// Plot scale: a fixed ratio, or fit to the plot area.
fn plot_scale_out(layout: &Layout) -> Option<PlotScale> {
    if layout.plot_flags.use_standard_scale {
        // Standard scales by their index, as output : drawing lengths.
        const STANDARD: [(f64, f64); 32] = [
            (1., 1536.),
            (1., 768.),
            (1., 384.),
            (1., 192.),
            (1., 128.),
            (1., 96.),
            (1., 64.),
            (1., 48.),
            (1., 32.),
            (1., 24.),
            (1., 16.),
            (1., 12.),
            (1., 4.),
            (1., 2.),
            (1., 1.),
            (1., 1.),
            (1., 2.),
            (1., 4.),
            (1., 8.),
            (1., 10.),
            (1., 16.),
            (1., 20.),
            (1., 30.),
            (1., 40.),
            (1., 50.),
            (1., 100.),
            (2., 1.),
            (4., 1.),
            (8., 1.),
            (10., 1.),
            (100., 1.),
            (1000., 1.),
        ];
        return match layout.plot_scale_type {
            0 => Some(PlotScale::FitToArea),
            index => {
                let (output_length, scope_length) =
                    *STANDARD.get(usize::try_from(index - 1).ok()?)?;
                Some(PlotScale::Fixed {
                    output_length,
                    scope_length,
                })
            }
        };
    }
    let (output_length, scope_length) =
        (layout.plot_scale_numerator, layout.plot_scale_denominator);
    (output_length > 0.0
        && scope_length > 0.0
        && output_length.is_finite()
        && scope_length.is_finite())
    .then_some(PlotScale::Fixed {
        output_length,
        scope_length,
    })
}

/// Whether a layout's plot settings are those of a fresh layout.
fn plot_is_default(layout: &Layout) -> bool {
    let fresh = Layout::new(&layout.name);
    let mut flags = layout.plot_flags;
    flags.model_type = fresh.plot_flags.model_type;
    flags == fresh.plot_flags
        && layout.plot_type == fresh.plot_type
        && layout.plot_rotation == fresh.plot_rotation
        && layout.plot_paper_units == fresh.plot_paper_units
        && layout.plot_scale_type == fresh.plot_scale_type
        && layout.plot_scale_numerator == fresh.plot_scale_numerator
        && layout.plot_scale_denominator == fresh.plot_scale_denominator
        && layout.plot_origin_x == 0.0
        && layout.plot_origin_y == 0.0
        && layout.plot_margin_left == 0.0
        && layout.plot_margin_bottom == 0.0
        && layout.plot_margin_right == 0.0
        && layout.plot_margin_top == 0.0
        && layout.plot_window_min_x == 0.0
        && layout.plot_window_min_y == 0.0
        && layout.plot_window_max_x == 0.0
        && layout.plot_window_max_y == 0.0
        && layout.shade_plot_mode == fresh.shade_plot_mode
        && layout.shade_plot_resolution == fresh.shade_plot_resolution
        && layout.shade_plot_dpi == fresh.shade_plot_dpi
        && layout.plot_printer_name.is_empty()
        && layout.paper_size.is_empty()
        && layout.plot_style_sheet.is_empty()
}

fn rect(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Option<PlotRect> {
    let valid = [min_x, min_y, max_x, max_y].iter().all(|v| v.is_finite())
        && min_x <= max_x
        && min_y <= max_y;
    valid.then_some(PlotRect {
        min_x,
        min_y,
        max_x,
        max_y,
    })
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn millimetres() -> MediaUnit {
    MediaUnit::Physical(CoordinateLengthUnit::Millimetre)
}

/// Millimetres per sheet unit; none for raster units.
fn millimetres_per(unit: MediaUnit) -> Option<f64> {
    let MediaUnit::Physical(unit) = unit else {
        return None;
    };
    Some(match unit.as_str() {
        "mm" => 1.0,
        "cm" => 10.0,
        "dm" => 100.0,
        "m" => 1000.0,
        "in" => 25.4,
        "ft" => 304.8,
        "um" => 0.001,
        "mil" => 0.0254,
        _ => return None,
    })
}
