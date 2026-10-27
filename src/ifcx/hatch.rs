//! Hatches: boundary loops, solid, line-pattern and gradient fills, and the
//! settings a CAD editor keeps to re-create them.
//!
//! The pattern mapping follows the cad-geometry-convert crate of ifccad
//! (MPL-2.0). A DWG stores pattern lines already turned and scaled in the
//! hatch's object coordinate system; IFCX stores the pattern's own families
//! plus its rotation and scale.

use std::f64::consts::TAU;

use acadrust::entities::hatch::{
    BoundaryEdge, BoundaryPath, BoundaryPathFlags, CircularArcEdge, EllipticArcEdge,
    GradientColorEntry, Hatch, HatchGradientPattern, HatchPattern, HatchPatternLine,
    HatchPatternType, HatchStyleType, LineEdge, PolylineEdge,
};
use ifccad::geometry_kernel::hatch::{
    validate_hatch_boundaries, validate_hatch_boundary_construction,
    validate_hatch_classic_settings, validate_hatch_fill, validate_hatch_gradient, HatchAreaRule,
    HatchBoundary2, HatchBoundaryConstruction, HatchClassicFill, HatchClassicSettings,
    HatchCreationType, HatchDash, HatchEdge2, HatchFill, HatchGradient, HatchGradientColorMode,
    HatchGradientType, HatchLineFamily, HatchLinePattern, HatchLoopProvenance,
    HatchPatternDefinitionLine, DEFAULT_HATCH_JOIN_TOLERANCE,
};
use ifccad::{IfccadHatch, IfccadHatchLoop};

use super::*;

const GRADIENTS: [(HatchGradientType, &str); 9] = [
    (HatchGradientType::Linear, "LINEAR"),
    (HatchGradientType::Cylinder, "CYLINDER"),
    (HatchGradientType::InverseCylinder, "INVCYLINDER"),
    (HatchGradientType::Spherical, "SPHERICAL"),
    (HatchGradientType::InverseSpherical, "INVSPHERICAL"),
    (HatchGradientType::Hemispherical, "HEMISPHERICAL"),
    (HatchGradientType::InverseHemispherical, "INVHEMISPHERICAL"),
    (HatchGradientType::Curved, "CURVED"),
    (HatchGradientType::InverseCurved, "INVCURVED"),
];

pub(super) fn hatch_out(hatch: &Hatch, losses: &mut Losses) -> Option<IfccadEntityKind> {
    let mut loops = Vec::new();
    for path in &hatch.paths {
        let bits = path.flags.bits();
        let provenance = HatchLoopProvenance {
            from_source_objects: bits & 1 != 0,
            derived_from_pick_point: bits & 4 != 0,
            text_box: bits & 8 != 0,
            outermost_hint: bits & 16 != 0,
        };
        loops.push(IfccadHatchLoop {
            boundary: boundary_out(path)?,
            source_entity_ids: Vec::new(),
            provenance: (!provenance.is_empty()).then_some(provenance),
            text_island: bits & 128 != 0,
        });
    }

    let gradient = gradient_out(&hatch.gradient_color);
    let fill = if hatch.gradient_color.enabled && gradient.is_some() {
        HatchFill::Gradient
    } else if hatch.is_solid || hatch.gradient_color.enabled {
        HatchFill::Solid
    } else {
        HatchFill::LinePattern(pattern_out(hatch)?)
    };
    let gradient =
        match validate_hatch_gradient(fill == HatchFill::Gradient, gradient.as_ref(), concrete) {
            Ok(()) => gradient,
            Err(_) => {
                losses.note("a hatch gradient");
                None
            }
        };
    let fill = match (fill, &gradient) {
        (HatchFill::Gradient, None) => HatchFill::Solid,
        (fill, _) => fill,
    };

    let classic = classic_out(hatch, &fill);
    let construction = HatchBoundaryConstruction {
        seed_points: hatch.seed_points.iter().map(|p| [p.x, p.y]).collect(),
        pixel_size: hatch.pixel_size,
    };
    let converted = IfccadHatch {
        classic_settings: classic.filter(|c| validate_hatch_classic_settings(c).is_ok()),
        boundary_construction: (!construction.is_empty()
            && validate_hatch_boundary_construction(&construction).is_ok())
        .then_some(construction),
        association_enabled: hatch.is_associative,
        gradient,
        placement: placement_out(Vector3::new(0.0, 0.0, hatch.elevation), hatch.normal, 0.0),
        loops,
        area_rule: match hatch.style {
            HatchStyleType::Normal => HatchAreaRule::Normal,
            HatchStyleType::Outer => HatchAreaRule::Outer,
            HatchStyleType::Ignore => HatchAreaRule::Ignore,
        },
        join_tolerance: DEFAULT_HATCH_JOIN_TOLERANCE,
        fill,
    };
    let valid = validate_hatch_fill(&converted.fill).is_ok()
        && validate_hatch_boundaries(
            converted.loops.iter().map(|l| &l.boundary),
            converted.join_tolerance,
        )
        .is_ok();
    valid.then_some(IfccadEntityKind::Hatch(converted))
}

pub(super) fn hatch_in(hatch: &IfccadHatch) -> EntityType {
    let (origin, normal, rotation) = placement_in(&hatch.placement);
    let frame = Frame2 {
        origin: [origin.x, origin.y],
        rotation,
    };
    let mut converted = Hatch::new();
    converted.normal = normal;
    converted.elevation = origin.z;
    converted.is_associative = hatch.association_enabled;
    converted.style = match hatch.area_rule {
        HatchAreaRule::Normal => HatchStyleType::Normal,
        HatchAreaRule::Outer => HatchStyleType::Outer,
        HatchAreaRule::Ignore => HatchStyleType::Ignore,
    };
    for hatch_loop in &hatch.loops {
        let mut path = boundary_in(&hatch_loop.boundary, &frame);
        let mut bits = path.flags.bits() & BoundaryPathFlags::POLYLINE.bits();
        if let Some(p) = &hatch_loop.provenance {
            bits |= u32::from(p.from_source_objects)
                | (u32::from(p.derived_from_pick_point) << 2)
                | (u32::from(p.text_box) << 3)
                | (u32::from(p.outermost_hint) << 4);
        }
        bits |= u32::from(hatch_loop.text_island) << 7;
        path.flags = BoundaryPathFlags::from_bits(bits);
        converted.paths.push(path);
    }
    if let Some(construction) = &hatch.boundary_construction {
        converted.seed_points = construction
            .seed_points
            .iter()
            .map(|p| frame.point(*p))
            .collect();
        converted.pixel_size = construction.pixel_size;
    }

    // Editing settings first; the active fill overrides what it decides.
    if let Some(classic) = &hatch.classic_settings {
        converted.pattern_type = match classic.creation_type {
            Some(HatchCreationType::UserDefined) => HatchPatternType::UserDefined,
            Some(HatchCreationType::Custom) => HatchPatternType::Custom,
            Some(HatchCreationType::Predefined) | None => HatchPatternType::Predefined,
        };
        converted.pattern.name = classic.name.clone().unwrap_or_default();
        converted.pattern.description = classic.description.clone().unwrap_or_default();
        converted.pattern_angle = classic.rotation + frame.rotation;
        converted.pattern_scale = classic.scale;
        converted.is_double = classic.double;
        if let Some(lines) = &classic.definition_lines {
            converted.pattern.lines = lines
                .iter()
                .map(|line| HatchPatternLine {
                    angle: line.angle + frame.rotation,
                    base_point: frame.point(line.base_point),
                    offset: frame.vector(line.offset_vector),
                    dash_lengths: line.dash_lengths.clone(),
                })
                .collect();
        }
    }
    match &hatch.fill {
        HatchFill::Solid | HatchFill::Gradient => {
            converted.is_solid = true;
            if hatch.classic_settings.is_none() {
                converted.pattern = HatchPattern::solid();
                converted.pattern_type = HatchPatternType::Predefined;
            }
        }
        HatchFill::LinePattern(pattern) => {
            converted.is_solid = false;
            pattern_in(pattern, &frame, &mut converted);
        }
    }
    if let Some(gradient) = &hatch.gradient {
        converted.gradient_color = HatchGradientPattern {
            enabled: hatch.fill == HatchFill::Gradient,
            reserved: 0,
            angle: gradient.rotation,
            shift: gradient.shift,
            is_single_color: gradient.color_mode == HatchGradientColorMode::OneColor,
            color_tint: gradient.tint,
            name: gradient
                .gradient_type
                .and_then(|kind| GRADIENTS.iter().find(|(k, _)| *k == kind))
                .map_or("", |(_, name)| name)
                .to_string(),
            colors: gradient
                .colors
                .iter()
                .flatten()
                .zip([0.0, 1.0])
                .map(|(color, value)| GradientColorEntry {
                    value,
                    color: color_in(color),
                    color_name: None,
                    book_name: None,
                })
                .collect(),
        };
    }
    EntityType::Hatch(Box::new(converted))
}

/// A frame turned in the hatch plane, for IFCX hatches whose placement is
/// not square to the OCS; written ones always are.
struct Frame2 {
    origin: [f64; 2],
    rotation: f64,
}

impl Frame2 {
    fn vector(&self, [u, v]: [f64; 2]) -> Vector2 {
        match self.rotation {
            0.0 => Vector2::new(u, v),
            rotation => {
                let (sin, cos) = rotation.sin_cos();
                Vector2::new(u * cos - v * sin, u * sin + v * cos)
            }
        }
    }

    fn point(&self, p: [f64; 2]) -> Vector2 {
        let v = self.vector(p);
        match self.origin {
            [0.0, 0.0] => v,
            [x, y] => Vector2::new(v.x + x, v.y + y),
        }
    }
}

fn boundary_out(path: &BoundaryPath) -> Option<HatchBoundary2> {
    let xy = |p: Vector2| [p.x, p.y];
    match path.edges.as_slice() {
        [BoundaryEdge::Polyline(polyline)] => {
            return Some(HatchBoundary2::Polyline {
                vertices: polyline.vertices.iter().map(|v| [v.x, v.y]).collect(),
                bulges: polyline.vertices.iter().map(|v| v.z).collect(),
            })
        }
        [BoundaryEdge::CircularArc(arc)]
            if sweep(arc.start_angle, arc.end_angle, arc.counter_clockwise).abs() == TAU =>
        {
            return Some(HatchBoundary2::Circle {
                center: xy(arc.center),
                radius: arc.radius,
            })
        }
        [BoundaryEdge::EllipticArc(arc)]
            if sweep(arc.start_angle, arc.end_angle, arc.counter_clockwise).abs() == TAU =>
        {
            let (x_axis, major, minor) = ellipse_axes(arc)?;
            return Some(HatchBoundary2::Ellipse {
                center: xy(arc.center),
                x_axis,
                semi_major_radius: major,
                semi_minor_radius: minor,
            });
        }
        _ => {}
    }
    let mut edges = Vec::new();
    for edge in &path.edges {
        edges.push(match edge {
            BoundaryEdge::Line(line) => HatchEdge2::Line {
                start: xy(line.start),
                end: xy(line.end),
            },
            BoundaryEdge::CircularArc(arc) => HatchEdge2::CircularArc {
                center: xy(arc.center),
                radius: arc.radius,
                start_parameter: signed(arc.start_angle, arc.counter_clockwise),
                sweep_parameter: sweep(arc.start_angle, arc.end_angle, arc.counter_clockwise),
            },
            BoundaryEdge::EllipticArc(arc) => {
                let (x_axis, major, minor) = ellipse_axes(arc)?;
                HatchEdge2::EllipticArc {
                    center: xy(arc.center),
                    x_axis,
                    semi_major_radius: major,
                    semi_minor_radius: minor,
                    start_parameter: signed(arc.start_angle, arc.counter_clockwise),
                    sweep_parameter: sweep(arc.start_angle, arc.end_angle, arc.counter_clockwise),
                }
            }
            // Spline boundaries have no IFCX form; nor does a polyline among
            // other edges.
            BoundaryEdge::Spline(_) | BoundaryEdge::Polyline(_) => return None,
        });
    }
    Some(HatchBoundary2::Edges(edges))
}

fn boundary_in(boundary: &HatchBoundary2, frame: &Frame2) -> BoundaryPath {
    let mut path = BoundaryPath::new();
    match boundary {
        HatchBoundary2::Polyline { vertices, bulges } => {
            path.flags = BoundaryPathFlags::POLYLINE;
            let vertices = vertices
                .iter()
                .enumerate()
                .map(|(index, p)| {
                    let p = frame.point(*p);
                    Vector3::new(p.x, p.y, bulges.get(index).copied().unwrap_or(0.0))
                })
                .collect();
            path.edges.push(BoundaryEdge::Polyline(PolylineEdge {
                vertices,
                is_closed: true,
            }));
        }
        HatchBoundary2::Circle { center, radius } => {
            path.edges.push(BoundaryEdge::CircularArc(CircularArcEdge {
                center: frame.point(*center),
                radius: *radius,
                start_angle: 0.0,
                end_angle: TAU,
                counter_clockwise: true,
            }));
        }
        HatchBoundary2::Ellipse {
            center,
            x_axis,
            semi_major_radius,
            semi_minor_radius,
        } => path.edges.push(BoundaryEdge::EllipticArc(elliptic_in(
            frame,
            *center,
            *x_axis,
            *semi_major_radius,
            *semi_minor_radius,
            0.0,
            TAU,
        ))),
        HatchBoundary2::Edges(edges) => {
            for edge in edges {
                path.edges.push(match edge {
                    HatchEdge2::Line { start, end } => BoundaryEdge::Line(LineEdge {
                        start: frame.point(*start),
                        end: frame.point(*end),
                    }),
                    HatchEdge2::CircularArc {
                        center,
                        radius,
                        start_parameter,
                        sweep_parameter,
                    } => {
                        let (start, end, counter_clockwise) =
                            stored_angles(*start_parameter + frame.rotation, *sweep_parameter);
                        BoundaryEdge::CircularArc(CircularArcEdge {
                            center: frame.point(*center),
                            radius: *radius,
                            start_angle: start,
                            end_angle: end,
                            counter_clockwise,
                        })
                    }
                    HatchEdge2::EllipticArc {
                        center,
                        x_axis,
                        semi_major_radius,
                        semi_minor_radius,
                        start_parameter,
                        sweep_parameter,
                    } => BoundaryEdge::EllipticArc(elliptic_in(
                        frame,
                        *center,
                        *x_axis,
                        *semi_major_radius,
                        *semi_minor_radius,
                        *start_parameter,
                        *sweep_parameter,
                    )),
                });
            }
        }
    }
    path
}

fn elliptic_in(
    frame: &Frame2,
    center: [f64; 2],
    x_axis: [f64; 2],
    major: f64,
    minor: f64,
    start_parameter: f64,
    sweep_parameter: f64,
) -> EllipticArcEdge {
    let axis = frame.vector(x_axis);
    let (start, end, counter_clockwise) = stored_angles(start_parameter, sweep_parameter);
    EllipticArcEdge {
        center: frame.point(center),
        major_axis_endpoint: Vector2::new(axis.x * major, axis.y * major),
        minor_axis_ratio: minor / major,
        start_angle: start,
        end_angle: end,
        counter_clockwise,
    }
}

/// Hatch arc angles run in the arc's stored direction: a clockwise arc's
/// angles are mathematical angles negated.
fn signed(angle: f64, counter_clockwise: bool) -> f64 {
    match counter_clockwise {
        true => angle,
        false => -angle,
    }
}

/// The signed sweep of a stored arc; a full turn for coincident ends.
fn sweep(start: f64, end: f64, counter_clockwise: bool) -> f64 {
    let turn = (end - start).rem_euclid(TAU);
    let turn = match turn {
        0.0 => TAU,
        turn => turn,
    };
    match counter_clockwise {
        true => turn,
        false => -turn,
    }
}

/// Stored start and end angles and direction of a signed sweep.
fn stored_angles(start_parameter: f64, sweep_parameter: f64) -> (f64, f64, bool) {
    let counter_clockwise = sweep_parameter >= 0.0;
    let start = signed(start_parameter, counter_clockwise);
    (start, start + sweep_parameter.abs(), counter_clockwise)
}

/// Unit major direction and the two radii.
fn ellipse_axes(arc: &EllipticArcEdge) -> Option<([f64; 2], f64, f64)> {
    let axis = arc.major_axis_endpoint;
    let major = axis.x.hypot(axis.y);
    let minor = major * arc.minor_axis_ratio;
    if !(major > 0.0 && minor > 0.0 && minor <= major) {
        return None;
    }
    let angle = axis.y.atan2(axis.x);
    let x_axis = match angle {
        0.0 => [1.0, 0.0],
        angle => [angle.cos(), angle.sin()],
    };
    Some((x_axis, major, minor))
}

/// The pattern families with the hatch's rotation and scale taken out.
fn pattern_out(hatch: &Hatch) -> Option<HatchLinePattern> {
    let rotation = hatch.pattern_angle;
    let scale = hatch.pattern_scale;
    if hatch.pattern.lines.is_empty() || scale.is_nan() || scale <= 0.0 || !rotation.is_finite() {
        return None;
    }
    let (sin, cos) = (-rotation).sin_cos();
    let unturn = |v: Vector2| {
        (
            (v.x * cos - v.y * sin) / scale,
            (v.x * sin + v.y * cos) / scale,
        )
    };
    let mut families = Vec::new();
    for line in &hatch.pattern.lines {
        let angle = line.angle - rotation;
        let base = unturn(line.base_point);
        let offset = unturn(line.offset);
        let (d_sin, d_cos) = angle.sin_cos();
        let along = offset.0 * d_cos + offset.1 * d_sin;
        let across = d_cos * offset.1 - d_sin * offset.0;
        let dashes = line
            .dash_lengths
            .iter()
            .map(|length| match *length {
                0.0 => HatchDash::Dot,
                length if length > 0.0 => HatchDash::Dash {
                    length: length / scale,
                },
                length => HatchDash::Gap {
                    length: -length / scale,
                },
            })
            .collect();
        families.push(HatchLineFamily {
            angle,
            base_point: [base.0, base.1],
            offset: [along, across],
            dashes,
        });
    }
    Some(HatchLinePattern {
        name: (!hatch.pattern.name.is_empty()).then(|| hatch.pattern.name.clone()),
        description: (!hatch.pattern.description.is_empty())
            .then(|| hatch.pattern.description.clone()),
        origin: [0.0, 0.0],
        rotation,
        scale,
        families,
    })
}

fn pattern_in(pattern: &HatchLinePattern, frame: &Frame2, hatch: &mut Hatch) {
    let rotation = pattern.rotation + frame.rotation;
    let scale = pattern.scale;
    let (sin, cos) = pattern.rotation.sin_cos();
    let turn = |(u, v): (f64, f64)| [(u * cos - v * sin) * scale, (u * sin + v * cos) * scale];
    let mut lines = Vec::new();
    for family in &pattern.families {
        let (d_sin, d_cos) = family.angle.sin_cos();
        let [along, across] = family.offset;
        let base = turn((family.base_point[0], family.base_point[1]));
        let offset = turn((
            along * d_cos - across * d_sin,
            along * d_sin + across * d_cos,
        ));
        lines.push(HatchPatternLine {
            angle: family.angle + rotation,
            base_point: frame.point([base[0] + pattern.origin[0], base[1] + pattern.origin[1]]),
            offset: frame.vector(offset),
            dash_lengths: family
                .dashes
                .iter()
                .map(|dash| match *dash {
                    HatchDash::Dot => 0.0,
                    HatchDash::Dash { length } => length * scale,
                    HatchDash::Gap { length } => -length * scale,
                })
                .collect(),
        });
    }
    hatch.pattern.lines = lines;
    if let Some(name) = &pattern.name {
        hatch.pattern.name = name.clone();
    }
    if let Some(description) = &pattern.description {
        hatch.pattern.description = description.clone();
    }
    if hatch.pattern.name.is_empty() {
        hatch.pattern.name = "_USER".to_string();
    }
    hatch.pattern_angle = rotation;
    hatch.pattern_scale = scale;
}

/// What a CAD editor shows for re-creating the hatch: absent for the plain
/// solid fill every CAD writes by default.
fn classic_out(hatch: &Hatch, fill: &HatchFill) -> Option<HatchClassicSettings> {
    let default_solid = hatch.is_solid
        && hatch.pattern.name.eq_ignore_ascii_case("SOLID")
        && hatch.pattern.description.is_empty()
        && hatch.pattern.lines.is_empty()
        && hatch.pattern_type == HatchPatternType::Predefined
        && hatch.pattern_angle == 0.0
        && hatch.pattern_scale == 1.0
        && !hatch.is_double;
    if default_solid {
        return None;
    }
    let classic = HatchClassicSettings {
        preferred_fill: Some(match fill {
            HatchFill::LinePattern(_) => HatchClassicFill::LinePattern,
            _ => HatchClassicFill::Solid,
        }),
        creation_type: Some(match hatch.pattern_type {
            HatchPatternType::UserDefined => HatchCreationType::UserDefined,
            HatchPatternType::Predefined => HatchCreationType::Predefined,
            HatchPatternType::Custom => HatchCreationType::Custom,
        }),
        name: Some(hatch.pattern.name.clone()),
        description: Some(hatch.pattern.description.clone()),
        origin: None,
        rotation: hatch.pattern_angle,
        scale: hatch.pattern_scale,
        double: hatch.is_double,
        // A line-pattern fill carries its own families.
        definition_lines: match fill {
            HatchFill::LinePattern(_) => None,
            _ => Some(
                hatch
                    .pattern
                    .lines
                    .iter()
                    .map(|line| HatchPatternDefinitionLine {
                        angle: line.angle,
                        base_point: [line.base_point.x, line.base_point.y],
                        offset_vector: [line.offset.x, line.offset.y],
                        dash_lengths: line.dash_lengths.clone(),
                    })
                    .collect(),
            ),
        },
    };
    Some(classic)
}

fn gradient_out(gradient: &HatchGradientPattern) -> Option<HatchGradient<IfccadColor>> {
    let colors = match gradient.colors.as_slice() {
        [first, second] => Some([concrete_out(first.color)?, concrete_out(second.color)?]),
        [] => None,
        _ => return None,
    };
    let dormant = !gradient.enabled
        && gradient.name.is_empty()
        && colors.is_none()
        && gradient.angle == 0.0
        && gradient.shift == 0.0
        && gradient.color_tint == 0.0
        && !gradient.is_single_color;
    if dormant {
        return None;
    }
    Some(HatchGradient {
        gradient_type: GRADIENTS
            .iter()
            .find(|(_, name)| gradient.name.eq_ignore_ascii_case(name))
            .map(|(kind, _)| *kind),
        colors,
        rotation: gradient.angle,
        shift: gradient.shift,
        color_mode: match gradient.is_single_color {
            true => HatchGradientColorMode::OneColor,
            false => HatchGradientColorMode::TwoColor,
        },
        tint: gradient.color_tint,
    })
}

/// A gradient end colour: a concrete one, so ACI 7 is its white.
fn concrete_out(color: Color) -> Option<IfccadColor> {
    match color {
        Color::Index(7) => Some(IfccadColor::Concrete {
            rgb: [255, 255, 255],
            indexed: Some(("ACI".to_string(), 7)),
            named: None,
        }),
        color => color_out(color),
    }
}

fn concrete(color: &IfccadColor) -> bool {
    color.concrete_rgb().is_some()
}
