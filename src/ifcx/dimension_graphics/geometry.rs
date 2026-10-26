//! Hard, absolute picture-coordinate qualification. No relative epsilon and no
//! generic approximate comparison of entity records: unlisted state stays exact.
use super::*;

pub(super) const POSITION: f64 = 1e-7;
const DIRECTION: f64 = 1e-12;

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1]).hypot(a[2] - b[2])
}
fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    a.into_iter().chain(b).all(f64::is_finite) && distance(a, b) <= POSITION
}
fn axes(p: &IfccadPlacement) -> Prepared<[Vector3; 3]> {
    let x = vector(p.x_axis);
    let y = vector(p.y_axis);
    for v in p.origin.into_iter().chain(p.x_axis).chain(p.y_axis) {
        finite(v)?;
    }
    if (x.dot(&x) - 1.).abs() > DIRECTION
        || (y.dot(&y) - 1.).abs() > DIRECTION
        || x.dot(&y).abs() > DIRECTION
    {
        return Err(unsupported("picture frame is not orthonormal"));
    }
    Ok([x, y, x.cross(&y)])
}
pub(super) fn validate_frame(p: &IfccadPlacement) -> Prepared<()> {
    axes(p).map(|_| ())
}

fn sum(a: f64, b: f64) -> Prepared<(f64, f64)> {
    let value = finite(a + b)?;
    let bv = finite(value - a)?;
    let av = finite(value - bv)?;
    Ok((value, finite((a - av) + (b - bv))?.abs()))
}

fn translate(v: [f64; 3], offset: [f64; 3]) -> Prepared<([f64; 3], f64)> {
    let mut out = [0.; 3];
    let mut errors = [0.; 3];
    for i in 0..3 {
        (out[i], errors[i]) = sum(v[i], offset[i])?;
    }
    Ok((out, 4. * errors[0].hypot(errors[1]).hypot(errors[2])))
}

/// Error-free product residuals (FMA) and TwoSum residuals bound each matrix
/// evaluation itself. An inverse-only test can hide large forward rounding.
fn linear(v: [f64; 3], basis: [Vector3; 3], inverse: bool) -> Prepared<([f64; 3], f64)> {
    let mut out = [0.; 3];
    let mut errors = [0.; 3];
    let basis = basis.map(array);
    for row in 0..3 {
        let mut value = 0.;
        let mut error = 0.;
        for col in 0..3 {
            let coefficient = if inverse {
                basis[row][col]
            } else {
                basis[col][row]
            };
            let product = finite(coefficient * v[col])?;
            error += finite(coefficient.mul_add(v[col], -product))?.abs();
            let (added, residual) = sum(value, product)?;
            value = added;
            error += residual;
        }
        out[row] = value;
        errors[row] = error;
    }
    // Conservative margin for accumulation of positive residual bounds.
    Ok((
        out,
        finite(4. * errors[0].hypot(errors[1]).hypot(errors[2]))?,
    ))
}

/// Rigid dimension-local <-> owner mapping. Inverse residuals are checked in
/// owner coordinates, including cancellation at large global origins.
pub(super) fn transform(
    mut kind: IfccadEntityKind,
    p: &IfccadPlacement,
    inverse: bool,
) -> Prepared<IfccadEntityKind> {
    let [x, y, z] = axes(p)?;
    let direction = |v: [f64; 3]| linear(v, [x, y, z], inverse);
    let at = |v: [f64; 3]| -> Prepared<([f64; 3], f64)> {
        let (out, error) = if inverse {
            let (local, subtraction_error) = translate(v, p.origin.map(|x| -x))?;
            let (out, rotation_error) = direction(local)?;
            // The qualified near-orthonormal matrix has norm below 2.
            (out, rotation_error + 2. * subtraction_error)
        } else {
            let (rotated, rotation_error) = direction(v)?;
            let (out, addition_error) = translate(rotated, p.origin)?;
            (out, rotation_error + addition_error)
        };
        if !error.is_finite() || error > POSITION {
            return Err(unsupported(
                "picture transform arithmetic exceeds the absolute coordinate budget",
            ));
        }
        let back = if inverse {
            point(array(x * out[0] + y * out[1] + z * out[2]), p.origin)?
        } else {
            let v = vector(out) - vector(p.origin);
            [v.dot(&x), v.dot(&y), v.dot(&z)]
        };
        if !close(v, back) {
            return Err(unsupported(
                "picture transform exceeds the absolute coordinate budget",
            ));
        }
        Ok((out, error))
    };
    let reach = match &kind {
        IfccadEntityKind::Point { .. } => Some(0.),
        IfccadEntityKind::Circle { radius, .. } | IfccadEntityKind::Arc { radius, .. } => {
            Some(radius.abs())
        }
        IfccadEntityKind::Ellipse {
            semi_major_radius,
            semi_minor_radius,
            ..
        }
        | IfccadEntityKind::EllipseArc {
            semi_major_radius,
            semi_minor_radius,
            ..
        } => Some(semi_major_radius.abs().max(semi_minor_radius.abs())),
        IfccadEntityKind::Solid(q) | IfccadEntityKind::Trace(q) => Some(
            q.corners
                .iter()
                .map(|v| v[0].hypot(v[1]))
                .fold(0., f64::max)
                + q.thickness.abs(),
        ),
        IfccadEntityKind::MText(t) => Some(t.wrap_width.unwrap_or(0.).max(t.height)),
        // A rounded orientation of arbitrary text, bulged/width-bearing paths
        // or a resource closure needs its own extent proof. Exact arithmetic
        // remains eligible, including the existing identity/translation path.
        _ => None,
    };
    let placed = |f: &mut IfccadPlacement| -> Prepared<()> {
        validate_frame(f)?;
        let (origin, origin_error) = at(f.origin)?;
        let (x, x_error) = direction(f.x_axis)?;
        let (y, y_error) = direction(f.y_axis)?;
        if x_error + y_error != 0. {
            let reach = reach.ok_or_else(|| {
                unsupported("rounded picture orientation needs an extent qualification")
            })?;
            if !reach.is_finite() || origin_error + 4. * (x_error + y_error) * reach > POSITION {
                return Err(unsupported(
                    "picture frame arithmetic exceeds the absolute coordinate budget",
                ));
            }
        }
        f.origin = origin;
        f.x_axis = x;
        f.y_axis = y;
        validate_frame(f)
    };
    match &mut kind {
        IfccadEntityKind::LineSegment { start, end } => {
            *start = at(*start)?.0;
            *end = at(*end)?.0;
        }
        IfccadEntityKind::SpatialPolyline { vertices, .. } => {
            for v in vertices {
                *v = at(*v)?.0;
            }
        }
        IfccadEntityKind::Point { placement }
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
    canonical(kind, [0.; 3])
}

fn placement(a: &mut IfccadPlacement, b: &IfccadPlacement, reach: f64) -> bool {
    if !close(a.origin, b.origin) || !reach.is_finite() {
        return false;
    }
    let dx = distance(a.x_axis, b.x_axis);
    let dy = distance(a.y_axis, b.y_axis);
    let dz = distance(
        array(vector(a.x_axis).cross(&vector(a.y_axis))),
        array(vector(b.x_axis).cross(&vector(b.y_axis))),
    );
    // A sum of column errors bounds any unit direction, including the normal
    // used by nonzero extrusion thickness. Never scale the budget by origin.
    let delta = dx + dy + dz;
    if dx.max(dy).max(dz) > DIRECTION || distance(a.origin, b.origin) + delta * reach > POSITION {
        return false;
    }
    *a = b.clone();
    true
}

/// Only the geometric fields qualified here may differ. All styles, content,
/// flags, widths, topology and semantic settings still compare exactly.
pub(super) fn native(a: &IfccadEntityKind, b: &IfccadEntityKind) -> bool {
    if a == b {
        return true;
    }
    let mut a = a.clone();
    let ok = match (&mut a, b) {
        (
            IfccadEntityKind::LineSegment { start: a, end: c },
            IfccadEntityKind::LineSegment { start: b, end: d },
        ) => {
            let ok = close(*a, *b) && close(*c, *d);
            *a = *b;
            *c = *d;
            ok
        }
        (IfccadEntityKind::Point { placement: a }, IfccadEntityKind::Point { placement: b }) => {
            placement(a, b, 0.)
        }
        (
            IfccadEntityKind::Circle {
                placement: a,
                radius,
            },
            IfccadEntityKind::Circle { placement: b, .. },
        )
        | (
            IfccadEntityKind::Arc {
                placement: a,
                radius,
                ..
            },
            IfccadEntityKind::Arc { placement: b, .. },
        ) => placement(a, b, *radius),
        (
            IfccadEntityKind::Ellipse {
                placement: a,
                semi_major_radius,
                ..
            },
            IfccadEntityKind::Ellipse { placement: b, .. },
        )
        | (
            IfccadEntityKind::EllipseArc {
                placement: a,
                semi_major_radius,
                ..
            },
            IfccadEntityKind::EllipseArc { placement: b, .. },
        ) => placement(a, b, *semi_major_radius),
        (
            IfccadEntityKind::PlanarPolyline {
                placement: a,
                vertices,
                ..
            },
            IfccadEntityKind::PlanarPolyline { placement: b, .. },
        ) => placement(
            a,
            b,
            vertices.iter().map(|v| v[0].hypot(v[1])).fold(0., f64::max),
        ),
        (IfccadEntityKind::Solid(a), IfccadEntityKind::Solid(b))
        | (IfccadEntityKind::Trace(a), IfccadEntityKind::Trace(b)) => placement(
            &mut a.placement,
            &b.placement,
            a.corners
                .iter()
                .map(|v| v[0].hypot(v[1]))
                .fold(0., f64::max)
                + a.thickness.abs(),
        ),
        (IfccadEntityKind::MText(a), IfccadEntityKind::MText(b)) => {
            let characters = 1.
                + a.content
                    .iter()
                    .flat_map(|p| &p.inlines)
                    .map(|inline| match inline {
                        ifccad::text::MTextInline::Run { text, .. } => text.chars().count() as f64,
                        ifccad::text::MTextInline::Stack(s) => {
                            (s.upper.chars().count() + s.lower.chars().count()) as f64
                        }
                        _ => 1.,
                    })
                    .sum::<f64>();
            placement(
                &mut a.placement,
                &b.placement,
                a.wrap_width.unwrap_or(0.).max(a.height * characters),
            )
        }
        _ => false,
    };
    ok && a == *b
}

pub(super) fn cad(a: &EntityType, b: &EntityType) -> bool {
    let mut a = a.clone();
    match (&mut a, b) {
        (EntityType::Line(a), EntityType::Line(b)) => {
            if !close(array(a.start), array(b.start)) || !close(array(a.end), array(b.end)) {
                return false;
            }
            a.start = b.start;
            a.end = b.end;
        }
        (EntityType::Circle(a), EntityType::Circle(b)) => {
            if a.normal != b.normal || !close(array(a.center), array(b.center)) {
                return false;
            }
            a.center = b.center;
        }
        (EntityType::Point(a), EntityType::Point(b)) => {
            if !close(array(a.location), array(b.location)) {
                return false;
            }
            a.location = b.location;
        }
        (EntityType::MText(a), EntityType::MText(b)) => {
            if !close(array(a.insertion_point), array(b.insertion_point)) {
                return false;
            }
            let dir = |t: &acadrust::entities::MText| {
                t.dwg_x_direction
                    .unwrap_or_else(|| Vector3::new(t.rotation.cos(), t.rotation.sin(), 0.))
            };
            let da = dir(a);
            let db = dir(b);
            if !da.dot(&da).is_finite()
                || !db.dot(&db).is_finite()
                || da.dot(&da) == 0.
                || db.dot(&db) == 0.
            {
                return false;
            }
            let delta = distance(array(da.normalize()), array(db.normalize()));
            let reach = a
                .rectangle_width
                .max(a.height * (a.value.chars().count() + 1) as f64);
            if !reach.is_finite()
                || !delta.is_finite()
                || delta > DIRECTION
                || distance(array(a.insertion_point), array(b.insertion_point)) + delta * reach
                    > POSITION
            {
                return false;
            }
            let line_error = (a.line_spacing_factor - b.line_spacing_factor).abs()
                * a.height
                * (5. / 3.)
                * (a.value.chars().count() + 1) as f64;
            if !line_error.is_finite() || line_error > POSITION {
                return false;
            }
            a.insertion_point = b.insertion_point;
            a.rotation = b.rotation;
            a.dwg_x_direction = b.dwg_x_direction;
            a.line_spacing_factor = b.line_spacing_factor;
            // Canvas selects the effective colour. The dormant authored colour
            // and renderer caches are diagnosed by the caller, never preserved.
            if a.background_fill_flags & 2 != 0 && b.background_fill_flags & 2 != 0 {
                a.background_color = b.background_color;
            }
            a.extents_width = b.extents_width;
            a.extents_height = b.extents_height;
        }
        _ => {}
    }
    a == *b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_cancellation_cannot_hide_large_forward_rounding() {
        let (sin, cos) = 0.37_f64.sin_cos();
        let frame = IfccadPlacement {
            origin: [0.; 3],
            x_axis: [cos, sin, 0.],
            y_axis: [-sin, cos, 0.],
        };
        let kind = IfccadEntityKind::LineSegment {
            start: [1e12, 0., 0.],
            end: [0.; 3],
        };
        assert!(transform(kind, &frame, false).is_err());
    }

    #[test]
    fn rotated_frame_rounding_is_bounded_at_the_curve_radius() {
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
        };
        assert!(transform(kind, &frame, false).is_err());
    }
}
