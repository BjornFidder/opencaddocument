//! Rigid picture arithmetic uses shared frame validity. Residuals are
//! observations only; finite valid geometry has no separate accuracy budget.
use super::*;

fn axes(p: &IfccadPlacement) -> Prepared<[Vector3; 3]> {
    p.coordinate_frame()
        .map_err(|_| invalid("invalid picture coordinate frame"))?;
    let x = vector(p.x_axis);
    let y = vector(p.y_axis);
    Ok([x, y, x.cross(&y)])
}
pub(super) fn validate_frame(p: &IfccadPlacement) -> Prepared<()> {
    axes(p).map(|_| ())
}
fn sum(a: f64, b: f64) -> Prepared<(f64, bool)> {
    finite(a)?;
    finite(b)?;
    let value = finite(a + b)?;
    let bv = value - a;
    let av = value - bv;
    let residual = (a - av) + (b - bv);
    Ok((value, residual.is_finite() && residual != 0.))
}
fn translate(v: [f64; 3], offset: [f64; 3]) -> Prepared<([f64; 3], bool)> {
    let mut out = [0.; 3];
    let mut rounded = false;
    for i in 0..3 {
        let (value, changed) = sum(v[i], offset[i])?;
        out[i] = value;
        rounded |= changed;
    }
    Ok((out, rounded))
}
fn linear(v: [f64; 3], basis: [Vector3; 3], inverse: bool) -> Prepared<([f64; 3], bool)> {
    let basis = basis.map(array);
    let mut out = [0.; 3];
    let mut rounded = false;
    for row in 0..3 {
        let mut value = 0.;
        for col in 0..3 {
            let coefficient = if inverse {
                basis[row][col]
            } else {
                basis[col][row]
            };
            let product = finite(coefficient * v[col])?;
            let residual = coefficient.mul_add(v[col], -product);
            rounded |= residual.is_finite() && residual != 0.;
            let (next, changed) = sum(value, product)?;
            value = next;
            rounded |= changed;
        }
        out[row] = value;
    }
    Ok((out, rounded))
}
pub(super) fn transform(
    mut kind: IfccadEntityKind,
    p: &IfccadPlacement,
    inverse: bool,
    path: &str,
    numerics: &mut PictureNumerics,
) -> Prepared<IfccadEntityKind> {
    let basis = axes(p)?;
    let direction = |v| linear(v, basis, inverse);
    let at = |v| -> Prepared<([f64; 3], bool)> {
        if inverse {
            let (local, a) = translate(v, p.origin.map(|x| -x))?;
            let (out, b) = direction(local)?;
            Ok((out, a || b))
        } else {
            let (local, a) = direction(v)?;
            let (out, b) = translate(local, p.origin)?;
            Ok((out, a || b))
        }
    };
    let mut placed = |f: &mut IfccadPlacement| -> Prepared<()> {
        validate_frame(f)?;
        let (origin, position) = at(f.origin)?;
        let (x, dx) = direction(f.x_axis)?;
        let (y, dy) = direction(f.y_axis)?;
        f.origin = origin;
        f.x_axis = x;
        f.y_axis = y;
        validate_frame(f)?;
        if position {
            numerics.record(path, NumericCategory::Position);
        }
        if dx || dy {
            numerics.record(path, NumericCategory::Direction);
        }
        Ok(())
    };
    match &mut kind {
        IfccadEntityKind::LineSegment {
            start,
            end,
            extrusion_direction,
            ..
        } => {
            let (normal, rounded) = direction(*extrusion_direction)?;
            *extrusion_direction = normal;
            if rounded {
                numerics.record(path, NumericCategory::Direction);
            }
            let (a, ra) = at(*start)?;
            let (b, rb) = at(*end)?;
            *start = a;
            *end = b;
            if ra || rb {
                numerics.record(path, NumericCategory::Position);
            }
        }
        IfccadEntityKind::SpatialPolyline { vertices, .. } => {
            for v in vertices {
                let (out, rounded) = at(*v)?;
                *v = out;
                if rounded {
                    numerics.record(path, NumericCategory::Position);
                }
            }
        }
        IfccadEntityKind::Point { placement, .. }
        | IfccadEntityKind::Circle { placement, .. }
        | IfccadEntityKind::Arc { placement, .. }
        | IfccadEntityKind::Ellipse { placement, .. }
        | IfccadEntityKind::EllipseArc { placement, .. }
        | IfccadEntityKind::PlanarPolyline { placement, .. } => placed(placement)?,
        IfccadEntityKind::Solid(q) | IfccadEntityKind::Trace(q) => placed(&mut q.placement)?,
        IfccadEntityKind::Text(t) => placed(&mut t.placement)?,
        IfccadEntityKind::MText(t) => placed(&mut t.placement)?,
        IfccadEntityKind::BlockInstance { transform, .. } => placed(&mut transform.placement)?,
        _ => return Err(unsupported("unsupported picture transform kind")),
    }
    canonical(kind, [0.; 3], path, numerics)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_matrix_rounding_is_observed_without_rejecting_the_line() {
        let (sin, cos) = 0.37_f64.sin_cos();
        let frame = IfccadPlacement {
            origin: [0.; 3],
            x_axis: [cos, sin, 0.],
            y_axis: [-sin, cos, 0.],
        };
        let kind = IfccadEntityKind::LineSegment {
            start: [1e12, 0., 0.],
            end: [0.; 3],
            thickness: 0.,
            extrusion_direction: [0., 0., 1.],
        };
        let mut numerics = PictureNumerics::default();
        let out = transform(kind, &frame, false, "p0", &mut numerics)
            .unwrap_or_else(|_| panic!("valid finite line"));
        let IfccadEntityKind::LineSegment { start, .. } = out else {
            panic!()
        };
        assert_eq!(start, [1e12 * cos, 1e12 * sin, 0.]);
        assert!(numerics.categories.contains(&NumericCategory::Position));
    }

    #[test]
    fn valid_rotated_frame_rounding_is_observed_at_any_radius() {
        let (sin, cos) = 0.37_f64.sin_cos();
        let frame = IfccadPlacement {
            origin: [0.; 3],
            x_axis: [cos, sin, 0.],
            y_axis: [-sin, cos, 0.],
        };
        let (sin, cos) = 0.23_f64.sin_cos();
        let kind = IfccadEntityKind::Circle {
            radius: 1e12,
            placement: IfccadPlacement {
                origin: [0.; 3],
                x_axis: [cos, sin, 0.],
                y_axis: [-sin, cos, 0.],
            },
            thickness: 0.,
        };
        let mut numerics = PictureNumerics::default();
        let out = transform(kind, &frame, false, "p0", &mut numerics)
            .unwrap_or_else(|_| panic!("valid finite circle"));
        let IfccadEntityKind::Circle {
            radius, placement, ..
        } = out
        else {
            panic!()
        };
        assert_eq!(radius, 1e12);
        assert!(placement.coordinate_frame().is_ok());
        assert!(numerics.categories.contains(&NumericCategory::Direction));
    }
}
