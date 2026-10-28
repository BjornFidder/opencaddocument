//! Definition conversion for the additional Dimension kinds.
//! Unsupported parameter/sector states are never replaced by another measure.
use super::*;
use acadrust::entities::{
    Dimension, DimensionAngular2Ln, DimensionAngular3Pt, DimensionArc, DimensionDiameter,
    DimensionLargeRadial, DimensionOrdinate, DimensionRadius,
};
use ifccad::dimension::*;

pub(super) fn into_cad(
    source: &DimensionDefinition,
    world: &impl Fn([f64; 3]) -> Vector3,
    frame: f64,
) -> Option<Dimension> {
    let plane = |p: [f64; 2]| world([p[0], p[1], 0.]);
    // The native measure uses projected XY anchors. These CAD radial helpers
    // measure a spatial distance, so unequal reference heights are unsupported.
    if !radial_heights_match(source) {
        return None;
    }
    let mut result = match source {
        DimensionDefinition::Radius {
            center,
            curve_point,
            leader_length,
        } => {
            let mut d = DimensionRadius::new(world(*center), world(*curve_point));
            d.leader_length = *leader_length;
            Dimension::Radius(d)
        }
        DimensionDefinition::Diameter {
            first_diameter_point,
            second_diameter_point,
            leader_length,
        } => {
            let mut d =
                DimensionDiameter::new(world(*first_diameter_point), world(*second_diameter_point));
            d.leader_length = *leader_length;
            Dimension::Diameter(d)
        }
        DimensionDefinition::AngularTwoLines {
            line1_start,
            line1_end,
            line2_start,
            line2_end,
            dimension_arc_point,
            ..
        } => {
            let d = DimensionAngular2Ln {
                first_point: world(*line1_start),
                second_point: world(*line1_end),
                angle_vertex: world(*line2_start),
                definition_point: world(*line2_end),
                dimension_arc: plane(*dimension_arc_point),
                ..Default::default()
            };
            Dimension::Angular2Ln(d)
        }
        DimensionDefinition::AngularThreePoints {
            vertex,
            first_point,
            second_point,
            dimension_arc_point,
            ..
        } => {
            let mut d =
                DimensionAngular3Pt::new(world(*vertex), world(*first_point), world(*second_point));
            d.definition_point = plane(*dimension_arc_point);
            Dimension::Angular3Pt(d)
        }
        DimensionDefinition::Ordinate {
            origin,
            feature_point,
            axis_angle,
            leader_end,
            value_mode,
        } => {
            if *value_mode != DimensionOrdinateMode::Absolute {
                return None;
            }
            let mut d = DimensionOrdinate::new(world(*feature_point), plane(*leader_end), true);
            d.definition_point = world(*origin);
            d.base.horizontal_direction = -(*axis_angle + frame);
            Dimension::Ordinate(d)
        }
        DimensionDefinition::ArcLength {
            center,
            radius,
            start_parameter,
            sweep,
            dimension_arc_point,
            leader,
        } => {
            let span = sweep.radians();
            if span <= 0. {
                return None;
            }
            let point = |angle: f64| {
                let (s, c) = angle.sin_cos();
                world([center[0] + radius * c, center[1] + radius * s, center[2]])
            };
            let mut d = DimensionArc {
                center_point: world(*center),
                first_extension_point: point(*start_parameter),
                second_extension_point: point(*start_parameter + span),
                definition_point: plane(*dimension_arc_point),
                arc_start_parameter: *start_parameter + frame,
                arc_end_parameter: *start_parameter + frame + span,
                ..Default::default()
            };
            if let Some(l) = leader {
                d.has_leader = true;
                d.first_leader_point = plane(l.start);
                d.second_leader_point = plane(l.end);
            }
            Dimension::Arc(d)
        }
        DimensionDefinition::JoggedRadius {
            center,
            curve_point,
            display_center,
            jog_point,
            jog_angle,
        } => {
            let d = DimensionLargeRadial {
                definition_point: world(*center),
                chord_point: world(*curve_point),
                override_center: plane(*display_center),
                jog_point: plane(*jog_point),
                jog_angle: *jog_angle,
                ..Default::default()
            };
            Dimension::LargeRadial(d)
        }
        _ => return None,
    };
    let origin = world([0.; 3]);
    result.base_mut().normal = (world([1., 0., 0.]) - origin)
        .cross(&(world([0., 1., 0.]) - origin))
        .normalize();
    // Codec 501b694 stores this subtype's arc point as raw XY in DWG.
    // Other planes need a separate codec/model qualification, not a WCS/OCS guess.
    if matches!(source, DimensionDefinition::AngularTwoLines { .. })
        && (result.base().normal != Vector3::new(0., 0., 1.) || origin.z != 0.)
    {
        return None;
    }
    let measurement = evaluate_dimension(source).ok()?;
    if matches!(measurement, DimensionMeasurement::Angle(_)) && !arc_inside_sector(source) {
        return None;
    }
    let expected = match measurement {
        DimensionMeasurement::Angle(x) | DimensionMeasurement::Distance(x) => x,
    };
    if !within_two_ulps(result.stored_measurement(), expected) {
        return None;
    }
    Some(result)
}

pub(super) fn from_cad(
    d: &Dimension,
    local: &impl Fn(Vector3) -> [f64; 3],
) -> Option<DimensionDefinition> {
    if let Dimension::Angular2Ln(v) = d {
        if v.base.normal != Vector3::new(0., 0., 1.)
            || v.dimension_arc.z != 0.
            || v.base.text_middle_point.z != 0.
        {
            return None;
        }
    }
    let xy = |p: Vector3| {
        let q = local(p);
        [q[0], q[1]]
    };
    let definition = match d {
        Dimension::Radius(v) => DimensionDefinition::Radius {
            center: local(v.angle_vertex),
            curve_point: local(v.definition_point),
            leader_length: v.leader_length,
        },
        Dimension::Diameter(v) => DimensionDefinition::Diameter {
            first_diameter_point: local(v.angle_vertex),
            second_diameter_point: local(v.definition_point),
            leader_length: v.leader_length,
        },
        Dimension::Angular2Ln(v) => {
            let mut definition = DimensionDefinition::AngularTwoLines {
                line1_start: local(v.first_point),
                line1_end: local(v.second_point),
                line2_start: local(v.angle_vertex),
                line2_end: local(v.definition_point),
                ray1_sign: DimensionRaySign::Positive,
                ray2_sign: DimensionRaySign::Positive,
                traversal: DimensionTraversal::Counterclockwise,
                dimension_arc_point: xy(v.dimension_arc),
            };
            reconcile_sector(&mut definition, v.measurement_radians())?;
            definition
        }
        Dimension::Angular3Pt(v) => {
            let mut definition = DimensionDefinition::AngularThreePoints {
                vertex: local(v.angle_vertex),
                first_point: local(v.first_point),
                second_point: local(v.second_point),
                traversal: DimensionTraversal::Counterclockwise,
                dimension_arc_point: xy(v.definition_point),
            };
            reconcile_sector(&mut definition, v.measurement_radians())?;
            definition
        }
        Dimension::Ordinate(v) => DimensionDefinition::Ordinate {
            origin: local(v.definition_point),
            feature_point: local(v.feature_location),
            axis_angle: -v.base.horizontal_direction
                + if v.is_ordinate_type_x {
                    0.
                } else {
                    std::f64::consts::FRAC_PI_2
                },
            leader_end: xy(v.leader_endpoint),
            value_mode: DimensionOrdinateMode::Absolute,
        },
        Dimension::Arc(v) => {
            let center = local(v.center_point);
            let first = local(v.first_extension_point);
            let second = local(v.second_extension_point);
            if first[2] != center[2] || second[2] != center[2] {
                return None;
            }
            if (v.has_leader
                && (local(v.first_leader_point)[2] != 0. || local(v.second_leader_point)[2] != 0.))
                || (!v.has_leader
                    && (v.first_leader_point != Vector3::ZERO
                        || v.second_leader_point != Vector3::ZERO))
            {
                return None;
            }
            let raw = v.arc_end_parameter - v.arc_start_parameter;
            if !raw.is_finite() || raw <= 0. || raw > std::f64::consts::TAU {
                return None;
            }
            let sweep = if raw == std::f64::consts::TAU {
                DimensionArcSpan::Full {
                    traversal: DimensionTraversal::Counterclockwise,
                }
            } else {
                DimensionArcSpan::Partial { radians: raw }
            };
            let radius = (first[0] - center[0]).hypot(first[1] - center[1]);
            let end_radius = (second[0] - center[0]).hypot(second[1] - center[1]);
            if !within_two_ulps(radius, end_radius) || v.is_partial {
                return None;
            }
            if !arc_endpoint_matches(center, radius, v.arc_start_parameter, first)
                || !arc_endpoint_matches(center, radius, v.arc_end_parameter, second)
            {
                return None;
            }
            DimensionDefinition::ArcLength {
                center,
                radius,
                start_parameter: v.arc_start_parameter,
                sweep,
                dimension_arc_point: xy(v.definition_point),
                leader: v.has_leader.then(|| DimensionArcLeader {
                    start: xy(v.first_leader_point),
                    end: xy(v.second_leader_point),
                }),
            }
        }
        Dimension::LargeRadial(v) => {
            if local(v.jog_point)[2] != 0. {
                return None;
            }
            DimensionDefinition::JoggedRadius {
                center: local(v.definition_point),
                curve_point: local(v.chord_point),
                display_center: xy(v.override_center),
                jog_point: xy(v.jog_point),
                jog_angle: v.jog_angle,
            }
        }
        _ => return None,
    };
    if !radial_heights_match(&definition) {
        return None;
    }
    let native = match evaluate_dimension(&definition).ok()? {
        DimensionMeasurement::Distance(v) | DimensionMeasurement::Angle(v) => v,
    };
    // Axis/sector re-expression must not introduce a different measure. In
    // particular, a CAD Y ordinate at zero cannot acquire a large X residual.
    within_two_ulps(native, d.stored_measurement()).then_some(definition)
}

fn radial_heights_match(d: &DimensionDefinition) -> bool {
    match d {
        DimensionDefinition::Radius {
            center,
            curve_point,
            ..
        }
        | DimensionDefinition::JoggedRadius {
            center,
            curve_point,
            ..
        } => center[2] == curve_point[2],
        DimensionDefinition::Diameter {
            first_diameter_point: a,
            second_diameter_point: b,
            ..
        } => a[2] == b[2],
        _ => true,
    }
}

fn arc_endpoint_matches(center: [f64; 3], radius: f64, parameter: f64, point: [f64; 3]) -> bool {
    // Named input-coherence bound: two ULPs at the local coordinate/radius
    // extent for the multiply/add reconstruction of each planar component.
    // Unqualified or contradictory source geometry is diagnosed, never used
    // to infer a different start parameter or selected sweep.
    let extent = center[0].abs().max(center[1].abs()).max(radius);
    if !extent.is_finite() || extent <= 0. {
        return false;
    }
    let ulp = f64::from_bits(extent.to_bits() + 1) - extent;
    if !ulp.is_finite() {
        return false;
    }
    let (s, c) = parameter.sin_cos();
    [c, s].into_iter().enumerate().all(|(i, direction)| {
        let expected = center[i] + radius * direction;
        expected.is_finite() && (expected - point[i]).abs() <= 2. * ulp
    })
}

// Qualification bound for nonnegative scalar measurements, not a geometric
// tolerance or permission to merge nearby definition points.
fn within_two_ulps(a: f64, b: f64) -> bool {
    a.is_finite() && b.is_finite() && a >= 0. && b >= 0. && a.to_bits().abs_diff(b.to_bits()) <= 2
}

fn reconcile_sector(definition: &mut DimensionDefinition, expected: f64) -> Option<()> {
    for (first, second) in [
        (DimensionRaySign::Positive, DimensionRaySign::Positive),
        (DimensionRaySign::Positive, DimensionRaySign::Negative),
        (DimensionRaySign::Negative, DimensionRaySign::Positive),
        (DimensionRaySign::Negative, DimensionRaySign::Negative),
    ] {
        if let DimensionDefinition::AngularTwoLines {
            ray1_sign,
            ray2_sign,
            ..
        } = definition
        {
            *ray1_sign = first;
            *ray2_sign = second;
        }
        for traversal in [
            DimensionTraversal::Counterclockwise,
            DimensionTraversal::Clockwise,
        ] {
            match definition {
                DimensionDefinition::AngularTwoLines { traversal: t, .. }
                | DimensionDefinition::AngularThreePoints { traversal: t, .. } => *t = traversal,
                _ => return None,
            }
            if let Ok(DimensionMeasurement::Angle(value)) = evaluate_dimension(definition) {
                if within_two_ulps(value, expected) && arc_inside_sector(definition) {
                    return Some(());
                }
            }
        }
    }
    None
}

fn arc_inside_sector(definition: &DimensionDefinition) -> bool {
    let delta = |a: [f64; 3], b: [f64; 3]| [b[0] - a[0], b[1] - a[1]];
    let (first, second, arc, traversal) = match definition {
        DimensionDefinition::AngularThreePoints {
            vertex,
            first_point,
            second_point,
            dimension_arc_point,
            traversal,
        } => (
            delta(*vertex, *first_point),
            delta(*vertex, *second_point),
            [
                dimension_arc_point[0] - vertex[0],
                dimension_arc_point[1] - vertex[1],
            ],
            *traversal,
        ),
        DimensionDefinition::AngularTwoLines {
            line1_start,
            line1_end,
            line2_start,
            line2_end,
            ray1_sign,
            ray2_sign,
            dimension_arc_point,
            traversal,
        } => {
            let a = delta(*line1_start, *line1_end);
            let b = delta(*line2_start, *line2_end);
            let separation = delta(*line1_start, *line2_start);
            let t = (separation[0] * b[1] - separation[1] * b[0]) / (a[0] * b[1] - a[1] * b[0]);
            let vertex = [line1_start[0] + t * a[0], line1_start[1] + t * a[1]];
            let sign = |s: DimensionRaySign| {
                if s == DimensionRaySign::Positive {
                    1.
                } else {
                    -1.
                }
            };
            (
                [a[0] * sign(*ray1_sign), a[1] * sign(*ray1_sign)],
                [b[0] * sign(*ray2_sign), b[1] * sign(*ray2_sign)],
                [
                    dimension_arc_point[0] - vertex[0],
                    dimension_arc_point[1] - vertex[1],
                ],
                *traversal,
            )
        }
        _ => return false,
    };
    if [first, second, arc]
        .into_iter()
        .any(|p| !p[0].is_finite() || !p[1].is_finite() || p == [0.; 2])
    {
        return false;
    }
    let angle = |p: [f64; 2]| p[1].atan2(p[0]);
    let directed = |b: f64| {
        if traversal == DimensionTraversal::Counterclockwise {
            (b - angle(first)).rem_euclid(std::f64::consts::TAU)
        } else {
            (angle(first) - b).rem_euclid(std::f64::consts::TAU)
        }
    };
    let selected = directed(angle(arc));
    selected > 0. && selected < directed(angle(second))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(p: [f64; 3]) -> Vector3 {
        Vector3::new(p[0], p[1], p[2])
    }

    #[test]
    fn different_reference_heights_are_diagnosed_instead_of_changing_radial_measure() {
        let source = DimensionDefinition::Radius {
            center: [0., 0., 7.],
            curve_point: [3., 4., 9.],
            leader_length: 0.,
        };
        assert!(into_cad(&source, &identity, 0.).is_none());
        let mut cad = DimensionRadius::new(identity([0., 0., 7.]), identity([3., 4., 9.]));
        cad.leader_length = 0.;
        assert!(from_cad(&Dimension::Radius(cad), &|v| [v.x, v.y, v.z]).is_none());
    }

    #[test]
    fn unsupported_angular_sector_is_not_silently_replaced_by_minor_angle() {
        let source = DimensionDefinition::AngularThreePoints {
            vertex: [0.; 3],
            first_point: [1., 0., 0.],
            second_point: [0., 1., 0.],
            traversal: DimensionTraversal::Clockwise,
            dimension_arc_point: [-5., -5.],
        };
        assert!(into_cad(&source, &identity, 0.).is_none());
    }

    #[test]
    fn inconsistent_arc_endpoint_is_diagnosed() {
        let cad = DimensionArc {
            center_point: identity([0.; 3]),
            first_extension_point: identity([5., 0., 0.]),
            second_extension_point: identity([0., 8., 0.]),
            arc_end_parameter: std::f64::consts::FRAC_PI_2,
            ..Default::default()
        };
        assert!(from_cad(&Dimension::Arc(cad), &|v| [v.x, v.y, v.z]).is_none());
    }

    #[test]
    fn equal_radii_do_not_make_contradictory_arc_parameters_consistent() {
        let cad = DimensionArc {
            first_extension_point: identity([5., 0., 0.]),
            second_extension_point: identity([-5., 0., 0.]),
            arc_end_parameter: std::f64::consts::FRAC_PI_2,
            ..Default::default()
        };
        assert!(from_cad(&Dimension::Arc(cad), &|v| [v.x, v.y, v.z]).is_none());
    }

    #[test]
    fn opposite_two_line_sector_keeps_both_ray_signs() {
        let cad = DimensionAngular2Ln {
            second_point: identity([1., 0., 0.]),
            definition_point: identity([0., 1., 0.]),
            dimension_arc: identity([-5., -5., 0.]),
            ..Default::default()
        };
        let definition = from_cad(&Dimension::Angular2Ln(cad), &|v| [v.x, v.y, v.z]).unwrap();
        assert!(matches!(
            definition,
            DimensionDefinition::AngularTwoLines {
                ray1_sign: DimensionRaySign::Negative,
                ray2_sign: DimensionRaySign::Negative,
                ..
            }
        ));
    }

    #[test]
    fn unqualified_two_line_plane_is_not_written_as_an_unrelated_raw_xy_point() {
        let source = DimensionDefinition::AngularTwoLines {
            line1_start: [0.; 3],
            line1_end: [1., 0., 0.],
            line2_start: [0.; 3],
            line2_end: [0., 1., 0.],
            ray1_sign: DimensionRaySign::Positive,
            ray2_sign: DimensionRaySign::Positive,
            traversal: DimensionTraversal::Counterclockwise,
            dimension_arc_point: [5., 5.],
        };
        assert!(into_cad(&source, &|p| identity([p[0], p[1], p[2] + 7.]), 0.).is_none());
    }

    #[test]
    fn unrepresentable_y_ordinate_does_not_become_a_huge_nonzero_measure() {
        let cad =
            DimensionOrdinate::new(identity([1e300, 0., 0.]), identity([1e300, 1., 0.]), false);
        assert_eq!(cad.measurement(), 0.);
        assert!(from_cad(&Dimension::Ordinate(cad), &|v| [v.x, v.y, v.z]).is_none());
    }
}
