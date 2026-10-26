use std::f64::consts::FRAC_PI_2;

use acadrust::types::{Matrix3, Vector3};

use crate::snap;

/// Where a planar object sits in space: an origin, a plane and a rotation in
/// that plane.
///
/// Stored the way DWG stores it (origin in the object coordinate system,
/// extrusion normal, rotation about the normal), so DWG values pass through
/// unchanged. Other forms convert on the way in and out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    origin: Vector3,
    normal: Vector3,
    rotation: f64,
}

impl Placement {
    /// DWG form: `origin` in the object coordinate system of `normal`
    /// (arbitrary axis algorithm), `rotation` in radians about `normal`.
    pub fn from_ocs(origin: Vector3, normal: Vector3, rotation: f64) -> Self {
        Self {
            origin,
            normal,
            rotation,
        }
    }

    pub fn to_ocs(&self) -> (Vector3, Vector3, f64) {
        (self.origin, self.normal, self.rotation)
    }

    /// World origin and two perpendicular unit axes (the IFC form).
    pub fn from_axes(origin: Vector3, x: Vector3, y: Vector3) -> Self {
        let normal = x.cross(&y).normalize();
        let ocs = Matrix3::arbitrary_axis(normal);
        let rotation = x.dot(&column(&ocs, 1)).atan2(x.dot(&column(&ocs, 0)));
        Self {
            origin: ocs.transpose().transform_point(origin),
            normal,
            rotation,
        }
    }

    /// This placement rounded in its own parameters: the origin to multiples
    /// of `position` and the normal's direction and the rotation to multiples
    /// of `angle` radians.
    ///
    /// A converter writing a format that does not store this form writes
    /// `snapped(..)` converted, not the placement itself. A conversion and its
    /// inverse agree only to a few ulps, so otherwise every save would change
    /// the file's last digits. Read back, a snapped placement lies a few ulps
    /// from the grid and rounds to the same point again, so after the first
    /// save the file stays the same. Rounding the converted values instead
    /// does not work: rounded axes are no longer unit length or
    /// perpendicular, and reading them back corrects that by about one step.
    pub fn snapped(&self, position: f64, angle: f64) -> Self {
        let n = self.normal.normalize();
        let elevation = snap(n.z.atan2(n.x.hypot(n.y)), angle);
        // At a pole every azimuth is the same direction, and the snapped
        // elevation is not exactly a right angle: take the pole itself, or
        // a plan-view normal would come out tilted by the rounding.
        let normal = if elevation.abs() == snap(FRAC_PI_2, angle) {
            Vector3::new(0.0, 0.0, elevation.signum())
        } else {
            let azimuth = snap(n.y.atan2(n.x), angle);
            let (sin_e, cos_e) = elevation.sin_cos();
            let (sin_a, cos_a) = azimuth.sin_cos();
            Vector3::new(cos_e * cos_a, cos_e * sin_a, sin_e)
        };
        Self {
            origin: Vector3::new(
                snap(self.origin.x, position),
                snap(self.origin.y, position),
                snap(self.origin.z, position),
            ),
            normal,
            rotation: snap(self.rotation.sin().atan2(self.rotation.cos()), angle),
        }
    }

    /// World origin, X axis and Y axis.
    pub fn to_axes(&self) -> (Vector3, Vector3, Vector3) {
        let ocs = Matrix3::arbitrary_axis(self.normal);
        let (sin, cos) = self.rotation.sin_cos();
        let x = column(&ocs, 0) * cos + column(&ocs, 1) * sin;
        let y = column(&ocs, 2).cross(&x);
        (ocs.transform_point(self.origin), x, y)
    }
}

fn column(m: &Matrix3, i: usize) -> Vector3 {
    Vector3::new(m.m[0][i], m.m[1][i], m.m[2][i])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic placements spread over every direction, including normals
    /// close to Z, where the arbitrary axis algorithm switches its reference.
    fn samples() -> Vec<Placement> {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
        };
        let mut out = Vec::new();
        for i in 0..2000 {
            let normal = if i % 10 == 0 {
                Vector3::new(next() / 50.0, next() / 50.0, 1.0).normalize()
            } else {
                Vector3::new(next(), next(), next()).normalize()
            };
            let origin = Vector3::new(next() * 1e6, next() * 1e6, next() * 1e3);
            out.push(Placement::from_ocs(
                origin,
                normal,
                next() * std::f64::consts::PI,
            ));
        }
        out
    }

    #[test]
    fn dwg_form_passes_through_unchanged() {
        for p in samples() {
            let (origin, normal, rotation) = p.to_ocs();
            assert_eq!(Placement::from_ocs(origin, normal, rotation), p);
        }
    }

    /// Axes are not the stored form, so a round trip through them drifts by a
    /// few ulps every time and never settles. Writing the snapped placement
    /// stops that: after the first save, saving again writes the same values.
    #[test]
    fn snapped_axes_are_stable_after_the_first_save() {
        let write = |p: Placement| p.snapped(1e-6, 1e-12).to_axes();
        for p in samples() {
            let first = write(p);
            let mut saved = first;
            for _ in 0..20 {
                saved = write(Placement::from_axes(saved.0, saved.1, saved.2));
                assert_eq!(saved, first);
            }
        }
    }
}
