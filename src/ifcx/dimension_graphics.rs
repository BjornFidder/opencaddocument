//! Complete, kind-independent picture transfer. No regeneration or side store.
use super::*;
use acadrust::entities::{Dimension, EntityCommon};
use ifccad::{
    IfccadDimension, IfccadDimensionGraphics, IfccadDimensionGraphicsKind as G,
    IfccadDimensionGraphicsOrigin as Origin, IfccadDimensionGraphicsPart as Part,
    IfccadDimensionGraphicsPurpose as Purpose, IfccadLayout,
};
use std::collections::BTreeSet;
mod geometry;
mod semantics;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum NumericCategory {
    Position,
    Direction,
    Length,
    Angle,
    Spacing,
}
#[derive(Default)]
struct PictureNumerics {
    categories: BTreeSet<NumericCategory>,
    first_path: Option<String>,
}
impl PictureNumerics {
    fn record(&mut self, path: &str, category: NumericCategory) {
        self.first_path.get_or_insert_with(|| path.into());
        self.categories.insert(category);
    }
    fn summary(&self, label: &str, handle: Option<Handle>) -> Option<Loss> {
        let path = self.first_path.as_ref()?;
        let categories = self
            .categories
            .iter()
            .map(|c| match c {
                NumericCategory::Position => "position",
                NumericCategory::Direction => "direction",
                NumericCategory::Length => "derived length",
                NumericCategory::Angle => "derived angle",
                NumericCategory::Spacing => "spacing",
            })
            .collect::<Vec<_>>()
            .join(", ");
        Some(Loss {
            handle,
            message: format!(
                "{label} graphics retained with numerical rounding ({categories}); first {path}"
            ),
        })
    }
}

enum Problem {
    Unsupported(String),
    Invalid(Error),
}
type Prepared<T> = Result<T, Problem>;
impl From<Error> for Problem {
    fn from(error: Error) -> Self {
        Self::Invalid(error)
    }
}
fn unsupported(reason: &str) -> Problem {
    Problem::Unsupported(reason.into())
}
fn invalid(reason: &str) -> Problem {
    Problem::Invalid(reason.into())
}

fn finite(value: f64) -> Prepared<f64> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(invalid("nonfinite picture geometry"))
    }
}

/// Observe a TwoSum residual without making it an acceptance criterion.
fn add(a: f64, b: f64, path: &str, numerics: &mut PictureNumerics) -> Prepared<f64> {
    finite(a)?;
    finite(b)?;
    let sum = finite(a + b)?;
    let bv = sum - a;
    let av = sum - bv;
    let error = (a - av) + (b - bv);
    if error.is_finite() && error != 0. {
        numerics.record(path, NumericCategory::Position);
    }
    Ok(sum)
}
fn point(
    p: [f64; 3],
    offset: [f64; 3],
    path: &str,
    numerics: &mut PictureNumerics,
) -> Prepared<[f64; 3]> {
    Ok([
        add(p[0], offset[0], path, numerics)?,
        add(p[1], offset[1], path, numerics)?,
        add(p[2], offset[2], path, numerics)?,
    ])
}
fn frame(
    p: &mut IfccadPlacement,
    offset: [f64; 3],
    path: &str,
    numerics: &mut PictureNumerics,
) -> Prepared<()> {
    for v in p.origin.into_iter().chain(p.x_axis).chain(p.y_axis) {
        finite(v)?;
    }
    geometry::validate_frame(p)?;
    p.origin = point(p.origin, offset, path, numerics)?;
    Ok(())
}

/// Factor equivalent XY origins; retain other qualified orthonormal frames.
fn canonical(
    mut kind: IfccadEntityKind,
    offset: [f64; 3],
    path: &str,
    numerics: &mut PictureNumerics,
) -> Prepared<IfccadEntityKind> {
    if let Some(g) = kind
        .as_shared_geometry()
        .map_err(|_| invalid("invalid picture primitive"))?
    {
        ifccad::geometry_kernel::validate_geometry(g)
            .map_err(|_| invalid("invalid picture primitive"))?;
    }
    match &mut kind {
        IfccadEntityKind::LineSegment { start, end, .. } => {
            *start = point(*start, offset, path, numerics)?;
            *end = point(*end, offset, path, numerics)?;
        }
        IfccadEntityKind::SpatialPolyline { vertices, .. } => {
            for p in vertices {
                *p = point(*p, offset, path, numerics)?;
            }
        }
        IfccadEntityKind::Point { placement, .. } => frame(placement, offset, path, numerics)?,
        IfccadEntityKind::Circle {
            placement, radius, ..
        }
        | IfccadEntityKind::Arc {
            placement, radius, ..
        } => {
            frame(placement, offset, path, numerics)?;
            if finite(*radius)? <= 0.0 {
                return Err(invalid("invalid picture radius"));
            }
        }
        IfccadEntityKind::Ellipse {
            placement,
            semi_major_radius,
            semi_minor_radius,
        }
        | IfccadEntityKind::EllipseArc {
            placement,
            semi_major_radius,
            semi_minor_radius,
            ..
        } => {
            frame(placement, offset, path, numerics)?;
            finite(*semi_major_radius)?;
            finite(*semi_minor_radius)?;
        }
        IfccadEntityKind::PlanarPolyline {
            placement,
            vertices,
            bulges,
            widths,
            ..
        } => {
            frame(placement, offset, path, numerics)?;
            if placement.x_axis == [1., 0., 0.] && placement.y_axis == [0., 1., 0.] {
                for v in vertices.iter_mut() {
                    v[0] = add(v[0], placement.origin[0], path, numerics)?;
                    v[1] = add(v[1], placement.origin[1], path, numerics)?;
                }
                placement.origin[0] = 0.0;
                placement.origin[1] = 0.0;
            }
            if bulges.is_empty() {
                bulges.resize(vertices.len(), 0.0);
            }
            for b in bulges {
                finite(*b)?;
            }
            if let IfccadPlanarPolylineWidths::Defaulted {
                default_start_width,
                default_end_width,
                overrides,
            } = widths
            {
                *widths = IfccadPlanarPolylineWidths::PerVertex {
                    widths: overrides
                        .iter()
                        .map(|w| {
                            [
                                w.start.unwrap_or(*default_start_width),
                                w.end.unwrap_or(*default_end_width),
                            ]
                        })
                        .collect(),
                };
            }
            if let IfccadPlanarPolylineWidths::PerVertex { widths: pairs } = widths {
                for pair in pairs.iter() {
                    finite(pair[0])?;
                    finite(pair[1])?;
                }
                if pairs.iter().all(|p| *p == [0., 0.]) {
                    *widths = IfccadPlanarPolylineWidths::Constant { width: 0. };
                }
            }
        }
        IfccadEntityKind::Solid(q) | IfccadEntityKind::Trace(q) => {
            frame(&mut q.placement, offset, path, numerics)?;
            finite(q.thickness)?;
            if q.placement.x_axis == [1., 0., 0.] && q.placement.y_axis == [0., 1., 0.] {
                for v in &mut q.corners {
                    v[0] = add(v[0], q.placement.origin[0], path, numerics)?;
                    v[1] = add(v[1], q.placement.origin[1], path, numerics)?;
                }
                q.placement.origin[0] = 0.;
                q.placement.origin[1] = 0.;
            }
        }
        IfccadEntityKind::Text(t) => {
            frame(&mut t.placement, offset, path, numerics)?;
            if matches!(
                t.layout,
                TextLayout::Aligned { .. } | TextLayout::Fit { .. }
            ) {
                return Err(unsupported(
                    "fitted Text is outside the exact picture profile",
                ));
            }
        }
        IfccadEntityKind::MText(t) => {
            frame(&mut t.placement, offset, path, numerics)?;
            if t.wrap_width == Some(0.) {
                t.wrap_width = None;
            }
        }
        IfccadEntityKind::BlockInstance { transform, .. } => {
            frame(&mut transform.placement, offset, path, numerics)?;
            finite(transform.rotation)?;
            for v in transform.scale {
                finite(v)?;
            }
        }
        _ => {
            return Err(unsupported(
                "part kind is outside the exact picture profile",
            ))
        }
    }
    if let IfccadEntityKind::Arc {
        start_parameter,
        sweep_parameter,
        ..
    }
    | IfccadEntityKind::EllipseArc {
        start_parameter,
        sweep_parameter,
        ..
    } = &kind
    {
        finite(*start_parameter)?;
        finite(*sweep_parameter)?;
    }
    Ok(kind)
}

/// Resource-only conversion context: never clone the drawing's entity graph.
fn resources(d: &IfccadDocument) -> IfccadDocument {
    IfccadDocument {
        dimension_styles: d.dimension_styles.clone(),
        symbol_sources: d.symbol_sources.clone(),
        point_display: d.point_display,
        text_styles: d.text_styles.clone(),
        ucs_definitions: vec![],
        model_windows: vec![],
        workspace_state: None,
        model_view_state: None,
        header: d.header.clone(),
        preservation: None,
        drawing_id: d.drawing_id,
        id_counters: d.id_counters,
        length_unit: d.length_unit.clone(),
        plot_style_mode: d.plot_style_mode,
        line_patterns: d.line_patterns.clone(),
        line_pattern_scale: d.line_pattern_scale,
        layers: d.layers.clone(),
        model: IfccadLayout {
            id: d.model.id,
            tab_index: d.model.tab_index,
            settings: d.model.settings.clone(),
            bounds: None,
            bounds_quality: None,
            entities: vec![],
        },
        paper_layouts: vec![],
        blocks: vec![],
    }
}
fn context<'a>(document: &'a Document, drawing: &IfccadDocument) -> Writer<'a> {
    Writer {
        document,
        drawing: resources(drawing),
        losses: Losses::default(),
        layers: HashMap::new(),
        line_patterns: HashMap::new(),
        styles: HashMap::new(),
        style_handles: HashMap::new(),
        symbols: HashMap::new(),
        blocks: HashMap::new(),
        entity_ids: HashMap::new(),
        hatch_sources: HashMap::new(),
        viewport_clips: HashMap::new(),
        layout_ids: HashMap::new(),
        ucs_ids: HashMap::new(),
        dim_styles: HashMap::new(),
        dimension_sources: HashMap::new(),
    }
}
fn inverse<'a>(document: &'a Document, drawing: &IfccadDocument, names: &Names) -> Writer<'a> {
    let mut w = context(document, drawing);
    w.layers = names
        .layers
        .iter()
        .map(|(id, n)| (n.to_ascii_lowercase(), *id))
        .collect();
    w.line_patterns = names
        .line_patterns
        .iter()
        .map(|(id, n)| (n.to_ascii_lowercase(), *id))
        .collect();
    w.styles = names
        .styles
        .iter()
        .map(|(id, n)| (n.to_ascii_lowercase(), *id))
        .collect();
    w.style_handles = names
        .style_handles
        .iter()
        .map(|(id, h)| (*h, *id))
        .collect();
    w.symbols = names.symbols.iter().map(|(id, h)| (*h, *id)).collect();
    w.blocks = names
        .blocks
        .iter()
        .map(|(id, n)| (n.to_ascii_lowercase(), Some(*id)))
        .collect();
    w
}
fn names(w: &Writer) -> Names {
    Names {
        layers: w.layers.iter().map(|(n, id)| (*id, n.clone())).collect(),
        line_patterns: w
            .line_patterns
            .iter()
            .map(|(n, id)| (*id, n.clone()))
            .collect(),
        styles: w.styles.iter().map(|(n, id)| (*id, n.clone())).collect(),
        style_handles: w.style_handles.iter().map(|(h, id)| (*id, *h)).collect(),
        symbols: w.symbols.iter().map(|(h, id)| (*id, *h)).collect(),
        blocks: w
            .blocks
            .iter()
            .filter_map(|(n, id)| id.map(|id| (id, n.clone())))
            .collect(),
        ..Default::default()
    }
}
fn stage<'a>(w: &Writer<'a>) -> Writer<'a> {
    let mut staged = context(w.document, &w.drawing);
    staged.layers = w.layers.clone();
    staged.line_patterns = w.line_patterns.clone();
    staged.styles = w.styles.clone();
    staged.style_handles = w.style_handles.clone();
    staged.symbols = w.symbols.clone();
    staged.blocks = w.blocks.clone();
    staged
}

fn width_loss(kind: &IfccadEntityKind) -> Option<&'static str> {
    match kind {
        IfccadEntityKind::PlanarPolyline {
            widths: IfccadPlanarPolylineWidths::Defaulted { .. },
            ..
        } => Some("width inheritance materialized"),
        IfccadEntityKind::PlanarPolyline {
            widths: IfccadPlanarPolylineWidths::PerVertex { widths },
            ..
        } if widths.iter().all(|p| *p == [0., 0.]) => Some("zero-width authored mode factored"),
        IfccadEntityKind::MText(t) if t.wrap_width == Some(0.) => {
            Some("MText zero wrap-width presence")
        }
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn native_entity(
    kind: IfccadEntityKind,
    appearance: &IfccadEntityAppearance,
    layer: &str,
    visible: bool,
    scale: f64,
    back: &mut Writer<'_>,
    names: &Names,
    path: &str,
    numerics: &mut PictureNumerics,
) -> Prepared<EntityType> {
    let expected = canonical(kind, [0.; 3], path, numerics)?;
    let mut losses = Losses::default();
    let mut entity = entity_in(&expected, names, &mut losses)
        .ok_or_else(|| unsupported("unsupported picture part or resource"))?;
    let common = entity.common_mut();
    common.layer = layer.into();
    common.invisible = !visible;
    common.linetype_scale = scale;
    appearance_in(appearance, names, common, &mut losses);
    let round_appearance = appearance_out(common, &back.line_patterns, &mut losses);
    let before = back.losses.content_snapshot();
    let actual = back
        .kind(&entity)?
        .ok_or_else(|| unsupported("part cannot be recaptured"))?;
    if losses.has_content_loss()
        || before != back.losses.content_snapshot()
        || round_appearance != *appearance
    {
        return Err(unsupported("part appearance or content would change"));
    }
    semantics::native(
        &canonical(actual, [0.; 3], path, numerics)?,
        &expected,
        path,
        numerics,
    )?;
    Ok(entity)
}

#[allow(clippy::too_many_arguments)]
fn native_block(
    id: u64,
    source: &IfccadDocument,
    back: &mut Writer<'_>,
    names: &Names,
    active: &mut BTreeSet<u64>,
    done: &mut BTreeSet<u64>,
    metadata: &mut Vec<String>,
    path: &str,
    numerics: &mut PictureNumerics,
) -> Prepared<()> {
    if done.contains(&id) {
        return Ok(());
    }
    if !active.insert(id) {
        return Err(invalid("cyclic picture marker resources"));
    }
    let block = source
        .blocks
        .iter()
        .find(|b| b.id == id)
        .ok_or_else(|| invalid("missing picture marker resource"))?;
    for e in &block.entities {
        let e = e
            .as_native()
            .ok_or_else(|| unsupported("opaque picture marker member"))?;
        let member_path = format!("{path}/marker {id}/member {}", e.id);
        if let IfccadEntityKind::BlockInstance { definition_id, .. } = &e.kind {
            native_block(
                *definition_id,
                source,
                back,
                names,
                active,
                done,
                metadata,
                &member_path,
                numerics,
            )?;
        }
        if let Some(reason) = width_loss(&e.kind) {
            metadata.push(format!("graphics marker {id} member {}: {reason}", e.id));
        }
        let layer = names
            .layers
            .get(&e.layer_id)
            .ok_or_else(|| invalid("missing marker layer"))?;
        native_entity(
            e.kind.clone(),
            &e.appearance,
            layer,
            e.visible,
            e.line_pattern_scale,
            back,
            names,
            &member_path,
            numerics,
        )?;
    }
    active.remove(&id);
    done.insert(id);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn import(
    source: &IfccadDimension,
    id: u64,
    layer_id: u64,
    owner: Handle,
    drawing: &IfccadDocument,
    document: &mut Document,
    names: &Names,
    losses: &mut Losses,
) -> Result<Option<String>, Error> {
    let Some(graphics) = &source.graphics else {
        return Ok(None);
    };
    let mut metadata = Vec::new();
    let mut numerics = PictureNumerics::default();
    let mut prepare = || -> Prepared<Vec<EntityType>> {
        let mut placement = source.placement.clone();
        frame(&mut placement, [0.; 3], "dimension", &mut numerics)?;
        if placement_in(&placement).1 != Vector3::new(0., 0., 1.) {
            return Err(unsupported(
                "picture dimension plane is outside the qualified +Z profile",
            ));
        }
        let root_layer = names
            .layers
            .get(&layer_id)
            .ok_or_else(|| invalid("missing Dimension layer"))?;
        let in_block = document
            .block_records
            .iter()
            .any(|r| r.handle == owner && !r.is_layout());
        let mut back = inverse(document, drawing, names);
        let mut parts = vec![];
        let mut done = BTreeSet::new();
        for (index, p) in graphics.parts.iter().enumerate() {
            let path = format!("p{index}");
            let layer = match p.layer_id {
                Some(id) => names
                    .layers
                    .get(&id)
                    .ok_or_else(|| invalid("missing picture layer"))?
                    .as_str(),
                None => "0",
            };
            if p.layer_id.is_some() && layer == "0" && (root_layer != "0" || in_block) {
                return Err(unsupported(
                    "explicit layer 0 would acquire CAD inheritance",
                ));
            }
            if p.layer_id.is_some() && layer == "0" {
                metadata.push(format!("dimension {id} graphics p{index}: explicit layer binding factored to inheritance"));
            }
            if p.purpose == Purpose::ReferencePoint
                && (p.layer_id.is_none() || !layer.eq_ignore_ascii_case("Defpoints"))
            {
                return Err(unsupported(
                    "ReferencePoint requires an explicit Defpoints layer",
                ));
            }
            if p.purpose == Purpose::Display
                && matches!(p.kind, G::Point { .. })
                && layer.eq_ignore_ascii_case("Defpoints")
            {
                return Err(unsupported("Display Point would become a Defpoints helper"));
            }
            let original: IfccadEntityKind = p.kind.clone().into();
            if let Some(reason) = width_loss(&original) {
                metadata.push(format!("dimension {id} graphics p{index}: {reason}"));
            }
            if let IfccadEntityKind::BlockInstance { definition_id, .. } = &original {
                native_block(
                    *definition_id,
                    drawing,
                    &mut back,
                    names,
                    &mut BTreeSet::new(),
                    &mut done,
                    &mut metadata,
                    &path,
                    &mut numerics,
                )?;
            }
            let kind = geometry::transform(original, &placement, false, &path, &mut numerics)?;
            parts.push(native_entity(
                kind,
                &p.appearance,
                layer,
                p.visible,
                p.line_pattern_scale,
                &mut back,
                names,
                &path,
                &mut numerics,
            )?);
        }
        Ok(parts)
    };
    // A picture that cannot be taken over, malformed or not, is left out
    // with its reason; the drawing around it still reads.
    let parts = match prepare() {
        Ok(parts) => parts,
        Err(Problem::Unsupported(reason)) => {
            losses.note(format!("dimension {id} graphics: {reason}"));
            return Ok(None);
        }
        Err(Problem::Invalid(error)) => {
            losses.note(format!("dimension {id} graphics: {error}"));
            return Ok(None);
        }
    };
    let mut number = 1;
    let name = loop {
        let name = format!("*D{number}");
        if document.block_records.get(&name).is_none() {
            break name;
        }
        number += 1;
    };
    let mut record = BlockRecord::new(name.clone());
    record.flags.anonymous = true;
    record.handle = document.allocate_handle();
    record.block_entity_handle = document.allocate_handle();
    record.block_end_handle = document.allocate_handle();
    let mut start = Block::new(name.clone(), Vector3::new(0., 0., 0.));
    start.common.handle = record.block_entity_handle;
    start.common.owner_handle = record.handle;
    let mut end = BlockEnd::new();
    end.common.handle = record.block_end_handle;
    end.common.owner_handle = record.handle;
    document.block_records.add(record.clone())?;
    document.add_entity(EntityType::Block(start))?;
    document.add_entity(EntityType::BlockEnd(end))?;
    for mut part in parts {
        part.common_mut().owner_handle = record.handle;
        document.add_entity(part)?;
    }
    for message in metadata {
        losses.note(message);
    }
    if graphics.origin == Origin::Generated {
        losses.note(format!("dimension {id} graphics provenance (Generated)"));
    }
    if let Some(note) = numerics.summary(&format!("dimension {id}"), None) {
        losses.retained_note(note);
    }
    Ok(Some(name))
}

fn common_supported(source: &EntityCommon) -> Prepared<()> {
    let mut extra = source.clone();
    let default = EntityCommon::default();
    extra.handle = default.handle;
    extra.owner_handle = default.owner_handle;
    extra.layer = default.layer.clone();
    extra.layer_handle = None;
    extra.color = default.color;
    extra.line_weight = default.line_weight;
    extra.linetype = default.linetype.clone();
    extra.linetype_handle = None;
    extra.linetype_scale = default.linetype_scale;
    extra.transparency = default.transparency;
    extra.invisible = default.invisible;
    extra.entity_mode = None;
    extra.raw_record = None;
    extra.extended_data = Default::default();
    if extra != default {
        return Err(unsupported(
            "picture member has unsupported common metadata",
        ));
    }
    Ok(())
}

// Check every reachable member before an unsupported sibling can short-circuit
// qualification. No source graph is changed or repaired by this pass.
fn cad_numbers(entity: &EntityType) -> Prepared<()> {
    let vector = |v: Vector3| -> Prepared<()> {
        finite(v.x)?;
        finite(v.y)?;
        finite(v.z)?;
        Ok(())
    };
    finite(entity.common().linetype_scale)?;
    match entity {
        EntityType::Line(v) => {
            vector(v.start)?;
            vector(v.end)?;
            vector(v.normal)?;
            finite(v.thickness)?;
        }
        EntityType::Point(v) => {
            finite(v.thickness)?;
            vector(v.location)?;
        }
        EntityType::Circle(v) => {
            finite(v.thickness)?;
            vector(v.center)?;
            vector(v.normal)?;
            finite(v.radius)?;
        }
        EntityType::Arc(v) => {
            finite(v.thickness)?;
            vector(v.center)?;
            vector(v.normal)?;
            finite(v.radius)?;
            finite(v.start_angle)?;
            finite(v.end_angle)?;
        }
        EntityType::Ellipse(v) => {
            vector(v.center)?;
            vector(v.normal)?;
            vector(v.major_axis)?;
            finite(v.minor_axis_ratio)?;
            finite(v.start_parameter)?;
            finite(v.end_parameter)?;
        }
        EntityType::LwPolyline(v) => {
            vector(v.normal)?;
            finite(v.elevation)?;
            finite(v.thickness)?;
            finite(v.constant_width)?;
            for p in &v.vertices {
                finite(p.location.x)?;
                finite(p.location.y)?;
                finite(p.bulge)?;
                finite(p.start_width)?;
                finite(p.end_width)?;
            }
        }
        EntityType::Polyline3D(v) => {
            for p in &v.vertices {
                vector(p.position)?;
            }
        }
        EntityType::Solid(v) => {
            for p in [
                v.first_corner,
                v.second_corner,
                v.third_corner,
                v.fourth_corner,
                v.normal,
            ] {
                vector(p)?;
            }
            finite(v.thickness)?;
        }
        EntityType::Text(v) => {
            vector(v.insertion_point)?;
            vector(v.normal)?;
            if let Some(p) = v.alignment_point {
                vector(p)?;
            }
            for x in [
                v.height,
                v.rotation,
                v.width_factor,
                v.oblique_angle,
                v.thickness,
            ] {
                finite(x)?;
            }
        }
        EntityType::MText(v) => {
            vector(v.insertion_point)?;
            vector(v.normal)?;
            if let Some(p) = v.dwg_x_direction {
                vector(p)?;
            }
            for x in [
                v.height,
                v.rotation,
                v.rectangle_width,
                v.line_spacing_factor,
            ] {
                finite(x)?;
            }
            if let Some(x) = v.rectangle_height {
                finite(x)?;
            }
        }
        EntityType::Insert(v) => {
            vector(v.insert_point)?;
            vector(v.normal)?;
            for x in [v.rotation, v.x_scale(), v.y_scale(), v.z_scale()] {
                finite(x)?;
            }
        }
        _ => {}
    }
    Ok(())
}
// CAD records reserve structural marker identities even when the reader does
// not materialize their entities (notably DXF ENDBLK). They are not member links.
fn headers<'a>(
    document: &'a Document,
    record: &BlockRecord,
) -> Prepared<(Option<&'a Block>, Option<&'a BlockEnd>)> {
    let start = match document.get_entity(record.block_entity_handle) {
        Some(EntityType::Block(start)) => Some(start),
        None => None,
        _ => return Err(invalid("mistyped picture BLOCK identity")),
    };
    let end = match document.get_entity(record.block_end_handle) {
        Some(EntityType::BlockEnd(end)) => Some(end),
        None => None,
        _ => return Err(invalid("mistyped picture ENDBLK identity")),
    };
    if start.is_some_and(|s| {
        s.common.owner_handle != record.handle || !s.name.eq_ignore_ascii_case(&record.name)
    }) || end.is_some_and(|e| e.common.owner_handle != record.handle)
    {
        return Err(invalid("conflicting picture block header ownership/name"));
    }
    Ok((start, end))
}

fn numeric_tree(
    document: &Document,
    name: &str,
    active: &mut BTreeSet<String>,
    done: &mut BTreeSet<String>,
) -> Prepared<()> {
    let key = name.to_ascii_lowercase();
    if done.contains(&key) {
        return Ok(());
    }
    if !active.insert(key.clone()) {
        return Err(invalid("cyclic picture block references"));
    }
    let record = document
        .block_records
        .get(name)
        .ok_or_else(|| invalid("dangling picture block reference"))?;
    let (start, _) = headers(document, record)?;
    if let Some(start) = start {
        for x in [start.base_point.x, start.base_point.y, start.base_point.z] {
            finite(x)?;
        }
    }
    let mut members = BTreeSet::new();
    for handle in &record.entity_handles {
        let entity = document
            .get_entity(*handle)
            .ok_or_else(|| invalid("dangling picture member handle"))?;
        if !members.insert(*handle)
            || matches!(entity, EntityType::Block(_) | EntityType::BlockEnd(_))
            || entity.common().owner_handle != record.handle
        {
            return Err(invalid("conflicting picture member ownership/membership"));
        }
    }
    for x in [
        record.base_point.x,
        record.base_point.y,
        record.base_point.z,
    ] {
        finite(x)?;
    }
    for e in document.entities_in_block(name) {
        cad_numbers(e)?;
        let common = e.common();
        if document.layers.get(&common.layer).is_none() {
            return Err(invalid("unresolved picture layer name"));
        }
        if let Some(handle) = common.layer_handle {
            if !document
                .layers
                .iter()
                .any(|l| l.handle == handle && l.name.eq_ignore_ascii_case(&common.layer))
            {
                return Err(invalid("unresolved or conflicting picture layer handle"));
            }
        }
        if let EntityType::Insert(v) = e {
            numeric_tree(document, &v.block_name, active, done)?;
        }
    }
    active.remove(&key);
    done.insert(key);
    Ok(())
}

fn record_metadata(document: &Document, record: &BlockRecord, marker: bool) -> Prepared<()> {
    // Header content is separate from BLOCK_RECORD in the CAD model. It must
    // not disappear merely because the record itself is representable.
    let (start, end) = headers(document, record)?;
    if start.is_some_and(|start| {
        start.base_point != record.base_point
            || start.description != record.description
            || !start.xref_path.is_empty()
    }) {
        return Err(unsupported("picture block has unsupported header metadata"));
    }
    for common in start
        .map(|s| &s.common)
        .into_iter()
        .chain(end.map(|e| &e.common))
    {
        let mut actual = common.clone();
        let baseline = EntityCommon::default();
        actual.handle = baseline.handle;
        actual.owner_handle = baseline.owner_handle;
        actual.layer_handle = None;
        actual.linetype_handle = None;
        actual.entity_mode = None;
        actual.raw_record = None;
        if actual != baseline {
            return Err(unsupported("picture block has unsupported header metadata"));
        }
    }
    if document.objects.values().any(|object| matches!(object,
        acadrust::objects::ObjectType::SortEntitiesTable(order) if order.block_owner_handle == record.handle
    )) {
        return Err(unsupported("external picture draw order is outside the qualified profile"));
    }
    let mut actual = record.clone();
    let baseline = BlockRecord::new(record.name.clone());
    actual.handle = baseline.handle;
    actual.block_entity_handle = baseline.block_entity_handle;
    actual.block_end_handle = baseline.block_end_handle;
    actual.flags.anonymous = baseline.flags.anonymous;
    if marker {
        if !(0..UNITS.len() as i16).contains(&actual.units) {
            return Err(unsupported("marker block has unsupported units"));
        }
        actual.units = baseline.units;
        actual.description.clone_from(&baseline.description);
        actual.explodable = baseline.explodable;
        actual.scale_uniformly = baseline.scale_uniformly;
        actual.base_point = baseline.base_point;
    }
    actual.entity_handles.clear();
    actual.insert_handles.clear();
    if actual.insert_count_bytes.iter().all(|b| *b == 1) {
        actual.insert_count_bytes.clear();
    }
    if actual != baseline {
        return Err(unsupported("picture block has unsupported record metadata"));
    }
    Ok(())
}

fn cad_part(
    w: &mut Writer<'_>,
    entity: &EntityType,
    placement: &IfccadPlacement,
    active: &mut BTreeSet<String>,
    done: &mut BTreeSet<String>,
    path: &str,
    numerics: &mut PictureNumerics,
) -> Prepared<Part> {
    common_supported(entity.common())?;
    if let EntityType::Insert(insert) = entity {
        cad_block(w, &insert.block_name, active, done, path, numerics)?;
    }
    let before = w.losses.content_snapshot();
    let native = w
        .kind(entity)?
        .ok_or_else(|| unsupported("unsupported picture member"))?;
    if w.losses.content_snapshot() != before {
        return Err(unsupported("picture member conversion loses content"));
    }
    let kind = geometry::transform(native, placement, true, path, numerics)?;
    let common = entity.common();
    let layer = *w
        .layers
        .get(&common.layer.to_ascii_lowercase())
        .ok_or_else(|| invalid("missing CAD picture layer"))?;
    let appearance = appearance_out(common, &w.line_patterns, &mut w.losses);
    let lookup = names(w);
    let owner_kind = geometry::transform(kind.clone(), placement, false, path, numerics)?;
    let mut issues = Losses::default();
    let mut back = entity_in(&owner_kind, &lookup, &mut issues)
        .ok_or_else(|| unsupported("picture part cannot be reconstructed"))?;
    appearance_in(&appearance, &lookup, back.common_mut(), &mut issues);
    let mut actual_common = back.common().clone();
    // These fields are transport identity, not part of the retained geometry.
    actual_common.handle = common.handle;
    actual_common.owner_handle = common.owner_handle;
    actual_common.layer = common.layer.clone();
    actual_common.layer_handle = common.layer_handle;
    actual_common.linetype_handle = common.linetype_handle;
    actual_common.entity_mode = common.entity_mode;
    actual_common.raw_record = common.raw_record.clone();
    actual_common.extended_data = common.extended_data.clone();
    actual_common.linetype_scale = common.linetype_scale;
    actual_common.invisible = common.invisible;
    if common.linetype.is_empty()
        || actual_common
            .linetype
            .eq_ignore_ascii_case(&common.linetype)
    {
        actual_common.linetype = common.linetype.clone();
    }
    if actual_common != *common
        || issues.has_content_loss()
        || before != w.losses.content_snapshot()
    {
        return Err(unsupported("picture member appearance cannot be retained"));
    }
    *back.common_mut() = common.clone();
    // CAD table references are case-insensitive; compare the resolved binding.
    match (&mut back, entity) {
        (EntityType::Text(a), EntityType::Text(b)) if a.style.eq_ignore_ascii_case(&b.style) => {
            a.style.clone_from(&b.style)
        }
        (EntityType::MText(a), EntityType::MText(b)) => {
            if a.style.eq_ignore_ascii_case(&b.style) {
                a.style.clone_from(&b.style);
            }
            // A stored +X direction exactly duplicates the qualified zero angle.
            if a.rotation == 0.
                && b.rotation == 0.
                && b.dwg_x_direction == Some(Vector3::new(1., 0., 0.))
            {
                a.dwg_x_direction = b.dwg_x_direction;
            }
        }
        (EntityType::Insert(a), EntityType::Insert(b))
            if a.block_name.eq_ignore_ascii_case(&b.block_name) =>
        {
            a.block_name.clone_from(&b.block_name)
        }
        (EntityType::Polyline3D(a), EntityType::Polyline3D(b)) => {
            for (a, b) in a.vertices.iter_mut().zip(&b.vertices) {
                a.handle = b.handle;
                // DWG has no independent vertex layer; its reader leaves this
                // DXF-only slot empty instead of the constructor's layer 0.
                if b.layer.is_empty() {
                    a.layer.clear();
                }
            }
        }
        _ => {}
    }
    semantics::cad(&back, entity, path, numerics)?;
    for record in common.extended_data.records() {
        w.losses.note(format!(
            "picture member {} XDATA application {} omitted",
            common.handle, record.application_name
        ));
    }
    if common.extended_data.records().is_empty() && common.extended_data != Default::default() {
        w.losses.note(format!(
            "picture member {} undecoded XDATA omitted",
            common.handle
        ));
    }
    if let (EntityType::MText(a), EntityType::MText(b)) = (&back, entity) {
        if b.dwg_x_direction.is_some()
            && (a.rotation - b.rotation)
                .sin()
                .atan2((a.rotation - b.rotation).cos())
                .abs()
                > 1e-12
        {
            w.losses.note(format!(
                "picture member {} inactive MText angle omitted (explicit direction selected)",
                common.handle
            ));
        }
        if a.background_color != b.background_color {
            w.losses.note(format!(
                "picture member {} inactive MText background colour omitted (canvas selected)",
                common.handle
            ));
        }
        if a.extents_width != b.extents_width || a.extents_height != b.extents_height {
            w.losses.note(format!(
                "picture member {} cached MText extents omitted",
                common.handle
            ));
        }
    }
    Ok(Part {
        kind: G::try_from(kind).map_err(|e| Problem::Invalid(Box::new(e)))?,
        layer_id: (common.layer != "0").then_some(layer),
        appearance,
        visible: !common.invisible,
        line_pattern_scale: common.linetype_scale,
        purpose: if matches!(entity, EntityType::Point(_))
            && common.layer.eq_ignore_ascii_case("Defpoints")
        {
            Purpose::ReferencePoint
        } else {
            Purpose::Display
        },
    })
}
fn cad_block(
    w: &mut Writer<'_>,
    name: &str,
    active: &mut BTreeSet<String>,
    done: &mut BTreeSet<String>,
    path: &str,
    numerics: &mut PictureNumerics,
) -> Prepared<()> {
    let key = name.to_ascii_lowercase();
    if done.contains(&key) {
        return Ok(());
    }
    if !active.insert(key.clone()) {
        return Err(invalid("cyclic CAD picture marker resources"));
    }
    let record = w
        .document
        .block_records
        .get(name)
        .ok_or_else(|| invalid("missing CAD picture marker block"))?;
    if record.is_layout() || record.flags.is_xref || record.flags.is_xref_overlay {
        return Err(unsupported("external/layout picture marker resource"));
    }
    record_metadata(w.document, record, true)?;
    let entities: Vec<_> = w.document.entities_in_block(name).cloned().collect();
    for e in entities {
        let member_path = format!("{path}/marker {name}/member {}", e.common().handle);
        cad_part(
            w,
            &e,
            &IfccadPlacement {
                origin: [0.; 3],
                x_axis: [1., 0., 0.],
                y_axis: [0., 1., 0.],
            },
            active,
            done,
            &member_path,
            numerics,
        )?;
    }
    active.remove(&key);
    done.insert(key);
    Ok(())
}

pub(super) fn capture(
    w: &mut Writer<'_>,
    dimension: &Dimension,
    placement: &IfccadPlacement,
) -> Result<Option<IfccadDimensionGraphics>, Error> {
    let base = dimension.base();
    if base.block_name.is_empty() {
        return Ok(None);
    }
    let mut staged = stage(w);
    let mut numerics = PictureNumerics::default();
    let mut prepare = || -> Prepared<Vec<Part>> {
        let record = staged
            .document
            .block_records
            .get(&base.block_name)
            .ok_or_else(|| invalid("dangling Dimension picture block link"))?;
        if record.is_layout() {
            return Err(invalid("Dimension picture links a layout block"));
        }
        let mut p = placement.clone();
        frame(&mut p, [0.; 3], "dimension", &mut numerics)?;
        if base.normal != Vector3::new(0., 0., 1.)
            || base.insertion_point != Vector3::new(0., 0., 0.)
            || base.insertion_scale != Vector3::new(1., 1., 1.)
            || base.insertion_rotation != 0.0
            || record.base_point != Vector3::new(0., 0., 0.)
        {
            return Err(unsupported(
                "picture clone/base/normal is outside the qualified profile",
            ));
        }
        let entities: Vec<_> = staged
            .document
            .entities_in_block(&base.block_name)
            .cloned()
            .collect();
        let mut parts = Vec::new();
        let mut active = BTreeSet::from([base.block_name.to_ascii_lowercase()]);
        let mut done = BTreeSet::new();
        numeric_tree(
            staged.document,
            &base.block_name,
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
        )?;
        record_metadata(staged.document, record, false)?;
        for (index, e) in entities.into_iter().enumerate() {
            let losses_before = staged.losses.content_snapshot();
            match cad_part(
                &mut staged,
                &e,
                &p,
                &mut active,
                &mut done,
                &format!("p{index}"),
                &mut numerics,
            ) {
                Ok(part) => {
                    parts.push(part);
                    staged.losses.locate_new_content(
                        &losses_before,
                        &format!("dimension {} graphics p{index}", base.common.handle),
                    );
                }
                Err(Problem::Unsupported(reason)) => {
                    return Err(Problem::Unsupported(format!("p{index}: {reason}")))
                }
                Err(error) => return Err(error),
            }
        }
        Ok(parts)
    };
    // A picture that cannot be written, malformed or not, is left out with
    // its reason; the drawing around it still saves.
    let reason = match prepare() {
        Ok(parts) => {
            if let Some(note) = numerics.summary(
                &format!("dimension {}", base.common.handle),
                Some(base.common.handle),
            ) {
                staged.losses.retained_note(note);
            }
            w.losses.merge(staged.losses);
            w.drawing.blocks.extend(staged.drawing.blocks);
            w.blocks = staged.blocks;
            w.drawing.id_counters = staged.drawing.id_counters;
            w.entity_ids.extend(staged.entity_ids);
            return Ok(Some(IfccadDimensionGraphics {
                origin: Origin::SourceSnapshot,
                parts,
            }));
        }
        Err(Problem::Unsupported(reason)) => reason,
        Err(Problem::Invalid(error)) => error.to_string(),
    };
    w.losses.note(format!(
        "dimension {} graphics: {reason}",
        base.common.handle
    ));
    Ok(None)
}
