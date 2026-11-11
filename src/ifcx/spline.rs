use acadrust::entities::{BoundaryEdge, Spline, SplineEdge};
use acadrust::types::{DxfVersion, Vector2, Vector3};
use acadrust::EntityType;
use ifccad::geometry_kernel::{SplineControl, SplineKnots};
use ifccad::{
    IfccadSpline, IfccadSplineAuthoringMode, IfccadSplineDefinition, IfccadSplineFitData,
    IfccadSplineParameterization, IfccadSplineTopology,
};

use super::{hatch::Frame2, Error, IfccadImportOutcome, Loss};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplineExchangeTarget {
    Dwg,
    Dxf,
}

impl IfccadImportOutcome {
    /// Prepare spline records for the selected physical writer. A strict
    /// rejection leaves the outgoing Document untouched.
    pub fn prepare_spline_exchange(
        &mut self,
        target: SplineExchangeTarget,
        strict: bool,
    ) -> Result<(), Vec<Loss>> {
        let version = self.document.version;
        let mut losses = Vec::new();
        let mut discard_fit = Vec::new();
        let mut expand = Vec::new();
        let mut unusable = false;
        for entity in self.document.entities() {
            let handle = entity.common().handle;
            let mut note = |message: String| {
                losses.push(Loss {
                    handle: Some(handle),
                    message,
                })
            };
            match entity {
                EntityType::Spline(spline) => {
                    let compact = spline.flags.periodic
                        && spline.knots.len() == spline.control_points.len() + 1;
                    if target == SplineExchangeTarget::Dxf {
                        let max_count = i16::MAX as usize;
                        let fits = usize::try_from(spline.degree).is_ok_and(|degree| {
                            if degree > max_count {
                                return false;
                            }
                            let extra = if compact { degree } else { 0 };
                            spline.control_points.len().saturating_add(extra) <= max_count
                                && spline.knots.len().saturating_add(2 * extra) <= max_count
                                && spline.fit_points.len() <= max_count
                        });
                        if !fits {
                            note("Standalone spline degree or count exceeds the signed 16-bit DXF record field".into());
                            unusable = true;
                            continue;
                        }
                    }
                    if target == SplineExchangeTarget::Dxf && compact {
                        let expanded = usize::try_from(spline.degree).ok().and_then(|degree| {
                            basis(
                                degree,
                                true,
                                spline.control_points.iter().copied().map(xyz).collect(),
                                spline.knots.clone(),
                                (!spline.weights.is_empty()).then(|| spline.weights.clone()),
                            )
                            .exact_periodic_expansion()
                            .ok()
                            .flatten()
                        });
                        if let Some(expanded) = expanded {
                            if version != DxfVersion::AC1032 {
                                note("Exact expanded periodic spline is only qualified for DXF AC1032".into());
                            }
                            expand.push((handle, expanded));
                        } else {
                            note("Compact periodic spline cannot be expanded into an exact binary64 DXF basis".into());
                            unusable = true;
                        }
                    }
                    if target == SplineExchangeTarget::Dwg {
                        if !spline.fit_points.is_empty() && !spline.control_points.is_empty() {
                            note("Standalone DWG spline retains its control definition and omits the fit data".into());
                            discard_fit.push(handle);
                        } else if !spline.fit_points.is_empty()
                            && version >= DxfVersion::AC1027
                            && spline.knot_parameterization == 15
                        {
                            note("Standalone DWG cannot encode this fit-only spline with custom knot parameterization".into());
                            unusable = true;
                        }
                        if spline.flags.planar || spline.flags.linear {
                            note(
                                "Standalone DWG omits the explicit spline plane or linear flag"
                                    .into(),
                            );
                        }
                        if version < DxfVersion::AC1027 && spline.cv_frame_visible {
                            note(
                                "Standalone DWG omits the spline control frame flag before R2013"
                                    .into(),
                            );
                        }
                        if spline.control_points.is_empty() && !spline.fit_points.is_empty() {
                            let repeated_end = spline.fit_points.len() >= 3
                                && spline.fit_points.first() == spline.fit_points.last();
                            if spline.flags.periodic || spline.flags.closed != repeated_end {
                                note("Standalone DWG fit record changes the spline topology flag or its endpoint sequence".into());
                            }
                            if version < DxfVersion::AC1027 && spline.dxf_flags & 0x1e0 != 0 {
                                note("Standalone DWG omits explicit fit parameterization before R2013".into());
                            }
                        }
                    }
                }
                EntityType::Hatch(hatch) if version < DxfVersion::AC1024 => {
                    for (loop_index, path) in hatch.paths.iter().enumerate() {
                        for (edge_index, edge) in path.edges.iter().enumerate() {
                            if matches!(edge, BoundaryEdge::Spline(edge) if !edge.fit_points.is_empty())
                            {
                                note(format!("Hatch loop {loop_index}, edge {edge_index} omits spline fit data before R2010"));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if unusable || (strict && !losses.is_empty()) {
            return Err(losses);
        }
        for handle in discard_fit {
            if let Some(EntityType::Spline(spline)) = self.document.get_entity_mut(handle) {
                spline.fit_points.clear();
                spline.begin_tangent = Vector3::ZERO;
                spline.end_tangent = Vector3::ZERO;
                spline.knot_parameterization = 15;
                spline.dwg_flags1 &= !(1 | 8);
                spline.dwg_scenario = Some(1);
            }
        }
        for (handle, expanded) in expand {
            if let Some(EntityType::Spline(spline)) = self.document.get_entity_mut(handle) {
                spline.control_points = expanded.points.into_iter().map(v3).collect();
                spline.knots = knot_values(&expanded.knots);
                spline.weights = expanded.weights.unwrap_or_default();
            }
        }
        self.losses.extend(losses);
        Ok(())
    }
}

fn xyz(p: Vector3) -> [f64; 3] {
    [p.x, p.y, p.z]
}
fn v3([x, y, z]: [f64; 3]) -> Vector3 {
    Vector3::new(x, y, z)
}
fn xy(p: Vector2) -> [f64; 2] {
    [p.x, p.y]
}

fn basis<const D: usize>(
    degree: usize,
    periodic: bool,
    mut points: Vec<[f64; D]>,
    knots: Vec<f64>,
    mut weights: Option<Vec<f64>>,
) -> SplineControl<D> {
    if periodic && knots.len() == points.len() && points.len() >= 2 {
        let repeated_point = points
            .first()
            .zip(points.last())
            .is_some_and(|(a, b)| a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits()));
        let repeated_weight = weights
            .as_ref()
            .is_none_or(|w| w.len() == points.len() && w[0].to_bits() == w[w.len() - 1].to_bits());
        if repeated_point && repeated_weight {
            points.pop();
            if let Some(weights) = &mut weights {
                weights.pop();
            }
        }
    }
    if periodic
        && degree > 0
        && points.len() >= 2 * degree
        && knots.len() == points.len() + degree + 1
    {
        let cycle_len = points.len() - degree;
        if weights.as_ref().is_none_or(|w| w.len() == points.len()) {
            let compact = SplineControl {
                degree,
                knots: SplineKnots::PeriodicCycle(knots[degree..=points.len()].to_vec()),
                points: points[..cycle_len].to_vec(),
                weights: weights.as_ref().map(|w| w[..cycle_len].to_vec()),
            };
            if let Ok(Some(expanded)) = compact.exact_periodic_expansion() {
                let SplineKnots::Full(derived) = &expanded.knots else {
                    unreachable!()
                };
                let exact =
                    |a: &[f64], b: &[f64]| a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits());
                if exact(derived, &knots)
                    && expanded
                        .points
                        .iter()
                        .zip(&points)
                        .all(|(x, y)| exact(x, y))
                    && expanded
                        .weights
                        .as_ref()
                        .zip(weights.as_ref())
                        .is_none_or(|(x, y)| exact(x, y))
                {
                    return compact;
                }
            }
        }
    }
    let knots = if periodic && knots.len() == points.len() + 1 {
        SplineKnots::PeriodicCycle(knots)
    } else {
        SplineKnots::Full(knots)
    };
    SplineControl {
        degree,
        knots,
        points,
        weights,
    }
}

fn knot_values(knots: &SplineKnots) -> Vec<f64> {
    match knots {
        SplineKnots::Full(values) | SplineKnots::PeriodicCycle(values) => values.clone(),
    }
}

fn parameterization(value: i32) -> Option<IfccadSplineParameterization> {
    use IfccadSplineParameterization as P;
    match value {
        0 => Some(P::Chord),
        1 => Some(P::Centripetal),
        2 => Some(P::Uniform),
        15 => Some(P::Custom),
        _ => None,
    }
}

pub(super) fn out(source: &Spline) -> Result<IfccadSpline, Error> {
    let degree = u32::try_from(source.degree)?;
    let has_parameterization = source.dxf_flags & 0x1e0 != 0
        || source.dwg_flags1 & 8 != 0
        || source.knot_parameterization != 0;
    let fit = (!source.fit_points.is_empty()).then(|| IfccadSplineFitData {
        points: source.fit_points.iter().copied().map(xyz).collect(),
        start_tangent: (source.begin_tangent != Vector3::ZERO).then(|| xyz(source.begin_tangent)),
        end_tangent: (source.end_tangent != Vector3::ZERO).then(|| xyz(source.end_tangent)),
        tolerance: Some(source.fit_tolerance),
        parameterization: has_parameterization
            .then(|| parameterization(source.knot_parameterization))
            .flatten(),
    });
    let fit_only = source.control_points.is_empty() && source.knots.is_empty();
    let definition = if fit_only {
        IfccadSplineDefinition::FitOnly {
            degree: Some(degree),
            fit: fit.ok_or("spline has neither control nor fit data")?,
        }
    } else {
        IfccadSplineDefinition::Control {
            basis: basis(
                degree as usize,
                source.flags.periodic,
                source.control_points.iter().copied().map(xyz).collect(),
                source.knots.clone(),
                (!source.weights.is_empty()).then(|| source.weights.clone()),
            ),
            fit,
        }
    };
    let topology = if source.flags.periodic {
        IfccadSplineTopology::Periodic
    } else if source.flags.closed {
        IfccadSplineTopology::Closed
    } else {
        IfccadSplineTopology::Open
    };
    let mut native = IfccadSpline::new(topology, definition);
    if source.dwg_scenario.is_none() {
        native.planar = Some(source.flags.planar);
        native.linear = Some(source.flags.linear);
        native.normal = source.flags.planar.then(|| xyz(source.normal));
    }
    if source.dwg_scenario != Some(2) {
        native.rational = Some(source.flags.rational);
        native.knot_tolerance = Some(source.knot_tolerance);
        native.control_tolerance = Some(source.control_tolerance);
    }
    native.authoring_mode = Some(if fit_only || source.dwg_flags1 & 1 != 0 {
        IfccadSplineAuthoringMode::Fit
    } else {
        IfccadSplineAuthoringMode::Control
    });
    native.control_frame_visible = Some(source.cv_frame_visible);
    native.validate()?;
    Ok(native)
}

pub(super) fn to_cad(source: &IfccadSpline) -> Result<Spline, Error> {
    use IfccadSplineParameterization as P;
    let mut target = Spline::new();
    target.flags.closed = source.topology != IfccadSplineTopology::Open;
    target.flags.periodic = source.topology == IfccadSplineTopology::Periodic;
    target.flags.planar = source.planar.unwrap_or(source.normal.is_some());
    target.flags.linear = source.linear.unwrap_or(false);
    target.cv_frame_visible = source.control_frame_visible.unwrap_or(false);
    if let Some(normal) = source.normal {
        target.normal = v3(normal);
    }
    if let Some(value) = source.knot_tolerance {
        target.knot_tolerance = value;
    }
    if let Some(value) = source.control_tolerance {
        target.control_tolerance = value;
    }
    let fit = match &source.definition {
        IfccadSplineDefinition::Control { basis, fit } => {
            target.degree = i32::try_from(basis.degree)?;
            target.knots = knot_values(&basis.knots);
            target.control_points = basis.points.iter().copied().map(v3).collect();
            target.weights = basis.weights.clone().unwrap_or_default();
            target.flags.rational = source.rational.unwrap_or(basis.weights.is_some());
            fit.as_ref()
        }
        IfccadSplineDefinition::FitOnly { degree, fit } => {
            if let Some(degree) = degree {
                target.degree = i32::try_from(*degree)?;
            }
            Some(fit)
        }
    };
    if let Some(fit) = fit {
        target.fit_points = fit.points.iter().copied().map(v3).collect();
        target.begin_tangent = fit.start_tangent.map_or(Vector3::ZERO, v3);
        target.end_tangent = fit.end_tangent.map_or(Vector3::ZERO, v3);
        if let Some(value) = fit.tolerance {
            target.fit_tolerance = value;
        }
        if let Some(param) = fit.parameterization {
            let (method, flags) = match param {
                P::Chord => (0, 32),
                P::Centripetal => (1, 64),
                P::Uniform => (2, 128),
                P::Custom => (15, 256),
            };
            target.knot_parameterization = method;
            target.dxf_flags |= flags;
        }
    }
    if source.control_basis().is_none()
        || source.authoring_mode == Some(IfccadSplineAuthoringMode::Fit)
    {
        target.dwg_flags1 |= 1 | 8;
    }
    Ok(target)
}

pub(super) fn edge_out(
    source: &SplineEdge,
) -> Option<(SplineControl<2>, Option<IfccadSplineFitData<2>>)> {
    let control = basis(
        usize::try_from(source.degree).ok()?,
        source.periodic,
        source.control_points.iter().map(|p| [p.x, p.y]).collect(),
        source.knots.clone(),
        source
            .rational
            .then(|| source.control_points.iter().map(|p| p.z).collect()),
    );
    let fit = (!source.fit_points.is_empty()).then(|| IfccadSplineFitData {
        points: source.fit_points.iter().copied().map(xy).collect(),
        start_tangent: Some(xy(source.start_tangent)),
        end_tangent: Some(xy(source.end_tangent)),
        tolerance: None,
        parameterization: None,
    });
    Some((control, fit))
}

pub(super) fn edge_in(
    control: &SplineControl<2>,
    fit: Option<&IfccadSplineFitData<2>>,
    periodic: bool,
    frame: &Frame2,
) -> Result<SplineEdge, Error> {
    Ok(SplineEdge {
        degree: i32::try_from(control.degree)?,
        rational: control.weights.is_some(),
        periodic,
        knots: knot_values(&control.knots),
        control_points: control
            .points
            .iter()
            .enumerate()
            .map(|(i, &point)| {
                let p = frame.point(point);
                let w = control.weights.as_ref().map_or(1., |w| w[i]);
                Vector3::new(p.x, p.y, w)
            })
            .collect(),
        fit_points: fit.map_or_else(Vec::new, |f| {
            f.points.iter().map(|&p| frame.point(p)).collect()
        }),
        start_tangent: fit
            .and_then(|f| f.start_tangent)
            .map_or(Vector2::ZERO, |v| frame.vector(v)),
        end_tangent: fit
            .and_then(|f| f.end_tangent)
            .map_or(Vector2::ZERO, |v| frame.vector(v)),
    })
}
