//! Independent world-space observation for the placement regressions. No
//! Placement, Matrix3::arbitrary_axis or CAD INSERT transform calls are used.
use std::collections::BTreeMap;
use std::f64::consts::TAU;
use std::io::Cursor;

use ifccad::dimension::DimensionDefinition;
use ifccad::text::*;
use ifccad::*;
use opencaddocument::entities::Dimension;
use opencaddocument::{
    Document, DwgReader, DwgWriter, DxfReader, DxfWriter, EntityType, Format, Ifcx,
};
use serde_json::Value;

type V = [f64; 3];
const EYE: [V; 3] = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
fn add(a: V, b: V) -> V {
    std::array::from_fn(|i| a[i] + b[i])
}
fn sub(a: V, b: V) -> V {
    std::array::from_fn(|i| a[i] - b[i])
}
fn mul(a: V, s: f64) -> V {
    a.map(|x| x * s)
}
fn dot(a: V, b: V) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn norm(a: V) -> f64 {
    dot(a, a).sqrt()
}
fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: V) -> V {
    let length = norm(a);
    a.map(|x| x / length)
}
fn linear(cols: [V; 3], p: V) -> V {
    std::array::from_fn(|i| (0..3).map(|j| cols[j][i] * p[j]).sum())
}
fn v(value: &Value) -> V {
    std::array::from_fn(|i| value[i].as_f64().unwrap())
}
fn cv(value: opencaddocument::types::Vector3) -> V {
    [value.x, value.y, value.z]
}
fn vector(value: V) -> opencaddocument::types::Vector3 {
    opencaddocument::types::Vector3::new(value[0], value[1], value[2])
}

#[derive(Clone, Copy)]
struct Affine {
    cols: [V; 3],
    origin: V,
}
impl Affine {
    const ID: Self = Self {
        cols: EYE,
        origin: [0.; 3],
    };
    fn at(self, p: V) -> V {
        add(self.origin, linear(self.cols, p))
    }
    fn then(self, child: Self) -> Self {
        Self {
            cols: child.cols.map(|c| linear(self.cols, c)),
            origin: self.at(child.origin),
        }
    }
    fn block(self, rotation: f64, scale: V, base: V) -> Self {
        let (s, c) = rotation.sin_cos();
        let local = [
            [c * scale[0], s * scale[0], 0.],
            [-s * scale[1], c * scale[1], 0.],
            [0., 0., scale[2]],
        ];
        let cols = local.map(|p| linear(self.cols, p));
        Self {
            cols,
            origin: sub(self.origin, linear(cols, base)),
        }
    }
}
fn frame(p: &IfccadPlacement) -> Affine {
    Affine {
        cols: [p.x_axis, p.y_axis, cross(p.x_axis, p.y_axis)],
        origin: p.origin,
    }
}
fn aaa(normal: V) -> [V; 3] {
    let n = unit(normal);
    let reference = if n[0].abs() < 1. / 64. && n[1].abs() < 1. / 64. {
        EYE[1]
    } else {
        EYE[2]
    };
    let x = unit(cross(reference, n));
    [x, cross(n, x), n]
}
fn cad_frame(origin: V, normal: V) -> Affine {
    let cols = aaa(normal);
    Affine {
        cols,
        origin: linear(cols, origin),
    }
}
fn identity(origin: V) -> IfccadPlacement {
    IfccadPlacement {
        origin,
        x_axis: EYE[0],
        y_axis: EYE[1],
    }
}

#[derive(Clone, Debug)]
pub struct Observation(BTreeMap<String, Value>);
impl Observation {
    fn new() -> Self {
        Self(BTreeMap::new())
    }
    fn insert(&mut self, key: &str, value: Value) {
        assert!(
            self.0.insert(key.into(), value).is_none(),
            "ambiguous geometry: {key}"
        );
    }
    fn point(&mut self, key: &str, t: Affine, p: V) {
        self.insert(key, serde_json::json!({"point":t.at(p)}));
    }
    fn conic(&mut self, key: &str, t: Affine, r: f64, start: f64, sweep: f64, circle: bool) {
        let (s, c) = start.sin_cos();
        self.insert(key,serde_json::json!({"center":t.origin,"a":linear(t.cols,[r*c,r*s,0.]),"b":linear(t.cols,[-r*s,r*c,0.]),"sweep":sweep,"circle":circle}));
    }
    pub fn assert_matches(&self, expected: &Self, mm: f64, budget: f64, context: &str) {
        assert_eq!(
            self.0.keys().collect::<Vec<_>>(),
            expected.0.keys().collect::<Vec<_>>(),
            "{context}: missing/extra geometry"
        );
        for (key, b) in &self.0 {
            let a = &expected.0[key];
            if a.get("text").is_some() {
                assert_eq!(a, b, "{context}: {key}");
                continue;
            }
            let error = descriptor_error(a, b) * mm;
            assert!(
                error <= budget,
                "{context}: {key}: {error:.12e} mm > {budget:.3e} mm\nexpected {a}\nactual {b}"
            );
        }
    }
    fn equivalent_picture(&self, mm: f64, budget: f64) {
        let select = |prefix: &str| {
            Self(
                self.0
                    .iter()
                    .filter_map(|(k, v)| k.strip_prefix(prefix).map(|k| (k.into(), v.clone())))
                    .collect(),
            )
        };
        select("picture/").assert_matches(
            &select("ordinary/"),
            mm,
            budget,
            "picture versus equivalent ordinary geometry",
        );
    }
    pub fn ordinary_in_mm(&self, mm: f64) -> Self {
        Self(
            self.0
                .iter()
                .filter(|(key, _)| !key.starts_with("picture/"))
                .map(|(key, value)| {
                    let mut value = value.clone();
                    for field in ["point", "center", "a", "b"] {
                        if let Some(coords) = value.get_mut(field) {
                            *coords = serde_json::json!(mul(v(coords), mm));
                        }
                    }
                    (key.clone(), value)
                })
                .collect(),
        )
    }
}
fn descriptor_error(a: &Value, b: &Value) -> f64 {
    if a.get("point").is_some() {
        return norm(sub(v(&a["point"]), v(&b["point"])));
    }
    assert_eq!(a["circle"], b["circle"]);
    let (xa, xb) = (v(&a["a"]), v(&a["b"]));
    let (mut ya, mut yb) = (v(&b["a"]), v(&b["b"]));
    if a["circle"] == true {
        let x = dot(xa, ya) + dot(xb, yb);
        let y = dot(xa, yb) - dot(xb, ya);
        let den = x.hypot(y);
        if den > 0. {
            let (c, s) = (x / den, y / den);
            (ya, yb) = (add(mul(ya, c), mul(yb, s)), add(mul(ya, -s), mul(yb, c)));
        }
    }
    let da = sub(xa, ya);
    let db = sub(xb, yb);
    let mut error =
        norm(sub(v(&a["center"]), v(&b["center"]))) + (dot(da, da) + dot(db, db)).sqrt();
    if a["circle"] != true {
        error += (norm(ya) + norm(yb))
            * (a["sweep"].as_f64().unwrap() - b["sweep"].as_f64().unwrap()).abs();
    }
    error
}

pub fn assert_reference_sanity() {
    let t = Affine {
        origin: [10., 20., 30.],
        ..Affine::ID
    }
    .block(0., [2., -3., 1.], [4., 5., 6.]);
    assert_eq!(t.at([7., 8., 9.]), [16., 11., 33.]);
    let a = serde_json::json!({"center":[0.,0.,0.],"a":[2.,0.,0.],"b":[0.,3.,0.],"circle":true,"sweep":TAU});
    let b = serde_json::json!({"center":[0.,0.,0.],"a":[0.,3.,0.],"b":[-2.,0.,0.],"circle":true,"sweep":TAU});
    assert_eq!(descriptor_error(&a, &b), 0.);
}

pub struct SpatialCase {
    pub data: Value,
    oracle: Value,
}
pub fn cases() -> Vec<SpatialCase> {
    let data: Vec<Value> =
        serde_json::from_str(include_str!("../fixtures/numerics-3d-cases.json")).unwrap();
    let oracle: Value =
        serde_json::from_str(include_str!("../fixtures/numerics-3d-oracle.json")).unwrap();
    assert_eq!(oracle["precision"], 100);
    data.into_iter()
        .map(|data| SpatialCase {
            oracle: oracle["cases"][data["name"].as_str().unwrap()].clone(),
            data,
        })
        .collect()
}
pub fn spatial_case(name: &str) -> SpatialCase {
    cases().into_iter().find(|c| c.name() == name).unwrap()
}
impl SpatialCase {
    pub fn name(&self) -> &str {
        self.data["name"].as_str().unwrap()
    }
    pub fn mm(&self) -> f64 {
        self.data["mm_per_unit"].as_f64().unwrap()
    }
    pub fn budget(&self) -> f64 {
        self.data["budget_mm"].as_f64().unwrap()
    }
    pub fn assert_world(&self, actual: &Observation, direction: &str, context: &str) {
        let expected = Observation(serde_json::from_value(self.oracle[direction].clone()).unwrap());
        actual.assert_matches(
            &expected,
            self.mm(),
            self.budget(),
            &format!("{} {direction} {context}", self.name()),
        );
        if self.data["wide_picture"] == true {
            actual.equivalent_picture(self.mm(), self.budget());
        }
    }
}
pub fn strict_native(bytes: &[u8]) -> IfccadDocument {
    let d = load_ifccad_bytes(
        bytes,
        IfccadReadOptions {
            composition_policy: IfccadCompositionPolicy::RejectConflicts,
        },
    )
    .unwrap()
    .into_document();
    validate_ifccad_document(&d).unwrap();
    d
}
fn part(kind: IfccadDimensionGraphicsKind) -> IfccadDimensionGraphicsPart {
    IfccadDimensionGraphicsPart {
        kind,
        layer_id: None,
        visible: true,
        line_pattern_scale: 1.,
        purpose: IfccadDimensionGraphicsPurpose::Display,
        appearance: IfccadEntityAppearance {
            color: IfccadMode::ByLayer,
            opacity: IfccadMode::ByLayer,
            line_pattern: IfccadMode::ByLayer,
            line_weight: IfccadMode::ByLayer,
        },
    }
}
fn fresh(
    d: &mut IfccadDocument,
    template: &IfccadNativeEntity,
    kind: IfccadEntityKind,
) -> IfccadEntity {
    let mut e = template.clone();
    e.id = d.id_counters.allocate_entity_id().unwrap();
    e.kind = kind;
    IfccadEntity::Native(e)
}
fn block(
    id: u64,
    name: String,
    base_point: V,
    entities: Vec<IfccadEntity>,
    unit: &str,
) -> IfccadBlockDefinition {
    IfccadBlockDefinition {
        id,
        name,
        description: String::new(),
        anonymous: false,
        explodable: true,
        uniform_scaling: false,
        base_point,
        insertion_unit: unit.into(),
        bounds: None,
        bounds_quality: None,
        entities,
    }
}
pub fn native_source(case: &SpatialCase) -> IfccadDocument {
    let mut d = strict_native(include_bytes!("../fixtures/dimension-graphics.ifcx"));
    let f = if case.data["physical_equivalent"] == true {
        10. / case.mm()
    } else {
        1.
    };
    d.length_unit = case.data["unit"].as_str().unwrap().into();
    d.blocks.clear();
    let line = d
        .model
        .entities
        .iter()
        .find(|e| {
            matches!(
                e.as_native().unwrap().kind,
                IfccadEntityKind::LineSegment { .. }
            )
        })
        .unwrap()
        .clone();
    let dimension=d.model.entities.iter().find(|e|matches!(&e.as_native().unwrap().kind,IfccadEntityKind::Dimension(dim) if matches!(dim.definition,DimensionDefinition::Linear {..}))).unwrap().clone();
    let template = line.as_native().unwrap().clone();
    d.model.entities = vec![line, dimension];
    let IfccadEntityKind::LineSegment { start, end, .. } =
        &mut d.model.entities[0].as_native_mut().unwrap().kind
    else {
        panic!()
    };
    *start = [0.; 3];
    *end = [4000. * f, 0., 0.];
    let style_id = d.text_styles[0].id;
    let IfccadEntityKind::Dimension(dim) = &mut d.model.entities[1].as_native_mut().unwrap().kind
    else {
        panic!()
    };
    dim.placement = identity([16. * f, 8. * f, 0.]);
    dim.definition = DimensionDefinition::Linear {
        first_point: [0.; 3],
        second_point: [4000. * f, 0., 0.],
        direction_angle: 0.,
        dimension_line_point: [0., 300. * f],
        extension_direction_angle: None,
    };
    dim.graphics = Some(IfccadDimensionGraphics {
        origin: IfccadDimensionGraphicsOrigin::SourceSnapshot,
        parts: vec![
            part(IfccadDimensionGraphicsKind::LineSegment {
                start: [0., 10. * f, 0.],
                end: [40. * f, 10. * f, 0.],
                thickness: 0.,
                extrusion_direction: [0., 0., 1.],
            }),
            part(IfccadDimensionGraphicsKind::Circle {
                radius: 1.5 * f,
                placement: identity([2. * f, 4. * f, 0.]),
                thickness: 0.,
            }),
            part(IfccadDimensionGraphicsKind::Point {
                placement: identity([3. * f, 5. * f, f]),
                thickness: 0.,
            }),
            part(IfccadDimensionGraphicsKind::BlockInstance {
                definition_id: 10,
                transform: IfccadBlockTransform {
                    placement: identity([9. * f, 11. * f, 0.]),
                    rotation: 0.,
                    scale: [-1., 2., 1.],
                },
            }),
        ],
    });
    if case.data["picture"] == false {
        dim.graphics = None;
    }
    if case.data["wide_picture"] == true {
        let p = identity([2. * f, 4. * f, 0.]);
        dim.graphics.as_mut().unwrap().parts.extend([
            part(IfccadDimensionGraphicsKind::Arc {
                placement: p.clone(),
                radius: 2. * f,
                start_parameter: 0.,
                sweep_parameter: 1.,
                thickness: 0.,
            }),
            part(IfccadDimensionGraphicsKind::Ellipse {
                placement: p.clone(),
                semi_major_radius: 2. * f,
                semi_minor_radius: f,
            }),
            part(IfccadDimensionGraphicsKind::PlanarPolyline {
                placement: p.clone(),
                vertices: vec![[0., 0.], [2. * f, f]],
                bulges: vec![0.5, 0.],
                closed: false,
                line_pattern_generation: IfccadLinePatternGeneration::Continuous,
                widths: IfccadPlanarPolylineWidths::Constant { width: 0.25 * f },
                thickness: 0.,
            }),
            part(IfccadDimensionGraphicsKind::SpatialPolyline {
                vertices: vec![[0., 0., f], [2. * f, f, 2. * f]],
                closed: false,
                line_pattern_generation: IfccadLinePatternGeneration::Continuous,
            }),
            part(IfccadDimensionGraphicsKind::Solid(
                IfccadFilledQuadrilateral {
                    placement: p.clone(),
                    corners: [[0., 0.], [2. * f, 0.], [0., f], [0., f]],
                    thickness: 0.,
                },
            )),
            part(IfccadDimensionGraphicsKind::Text(IfccadText {
                style_id,
                placement: p.clone(),
                rotation: 0.,
                backward: false,
                upside_down: false,
                layout: TextLayout::Anchored {
                    horizontal: TextHorizontalAlignment::Left,
                    vertical: TextVerticalAlignment::Baseline,
                    height: 2. * f,
                    width_factor: 1.,
                },
                oblique_angle: 0.,
                thickness: 0.,
                content: vec![TextRun {
                    text: "1200 mm".into(),
                    underline: false,
                    overline: false,
                    strike_through: false,
                }],
            })),
            part(IfccadDimensionGraphicsKind::MText(Box::new(IfccadMText {
                style_id,
                placement: p,
                rotation: 0.,
                backward: false,
                upside_down: false,
                height: 2. * f,
                attachment: MTextAttachment::TopLeft,
                flow: MTextFlow::ByStyle,
                wrap_width: None,
                columns: None,
                background: None,
                character_format: Default::default(),
                paragraph_format: Default::default(),
                content: vec![MTextParagraph {
                    inlines: vec![MTextInline::Run {
                        text: if case.data["unbraced"] == true {
                            "240"
                        } else {
                            "1200 mm"
                        }
                        .into(),
                        character_format: CharacterFormat {
                            position: if case.data["unbraced"] == true {
                                Some(TextPosition::Center)
                            } else {
                                None
                            },
                            ..Default::default()
                        },
                    }],
                    ..Default::default()
                }],
            }))),
        ]);
    }
    let counterparts = dim
        .graphics
        .as_ref()
        .map(|g| g.parts.clone())
        .unwrap_or_default();
    let marker = fresh(
        &mut d,
        &template,
        IfccadEntityKind::LineSegment {
            start: [0.; 3],
            end: [2. * f, f, 0.],
            thickness: 0.,
            extrusion_direction: [0., 0., 1.],
        },
    );
    d.blocks.push(block(
        10,
        "Marker".into(),
        [f, 2. * f, 0.],
        vec![marker],
        &d.length_unit,
    ));
    let mut members = std::mem::take(&mut d.model.entities);
    members.push(fresh(
        &mut d,
        &template,
        IfccadEntityKind::Circle {
            radius: 3. * f,
            placement: identity([30. * f, 5. * f, 2. * f]),
            thickness: 0.,
        },
    ));
    if case.data["wide_picture"] == true {
        let mut ordinary = Vec::new();
        for part in counterparts {
            let mut kind: IfccadEntityKind = part.kind.into();
            let offset = [16. * f, 8. * f, 0.];
            let shift = |p: &mut IfccadPlacement| p.origin = add(p.origin, offset);
            match &mut kind {
                IfccadEntityKind::LineSegment { start, end, .. } => {
                    *start = add(*start, offset);
                    *end = add(*end, offset);
                }
                IfccadEntityKind::SpatialPolyline { vertices, .. } => {
                    for v in vertices {
                        *v = add(*v, offset);
                    }
                }
                IfccadEntityKind::Point { placement, .. }
                | IfccadEntityKind::Circle { placement, .. }
                | IfccadEntityKind::Arc { placement, .. }
                | IfccadEntityKind::Ellipse { placement, .. }
                | IfccadEntityKind::PlanarPolyline { placement, .. } => shift(placement),
                IfccadEntityKind::Solid(q) => shift(&mut q.placement),
                IfccadEntityKind::Text(t) => shift(&mut t.placement),
                IfccadEntityKind::MText(t) => shift(&mut t.placement),
                IfccadEntityKind::BlockInstance { transform, .. } => {
                    shift(&mut transform.placement)
                }
                _ => panic!(),
            }
            ordinary.push(fresh(&mut d, &template, kind));
        }
        d.blocks.push(block(
            20,
            "Picture counterparts".into(),
            [0.; 3],
            ordinary,
            &d.length_unit,
        ));
        members.push(fresh(
            &mut d,
            &template,
            IfccadEntityKind::BlockInstance {
                definition_id: 20,
                transform: IfccadBlockTransform {
                    placement: identity([0.; 3]),
                    rotation: 0.,
                    scale: [1.; 3],
                },
            },
        ));
    }
    members.push(fresh(
        &mut d,
        &template,
        IfccadEntityKind::Arc {
            radius: 7.5 * f,
            start_parameter: 0.5,
            sweep_parameter: 1.5,
            placement: identity([-10. * f, 20. * f, 0.]),
            thickness: 0.,
        },
    ));
    for (i, p) in case.data["poses"].as_array().unwrap().iter().enumerate() {
        let id = i as u64 + 1;
        d.blocks.push(block(
            id,
            format!("Level{i}"),
            v(&p["base"]),
            members,
            &d.length_unit,
        ));
        let n = &p["native"];
        members = vec![fresh(
            &mut d,
            &template,
            IfccadEntityKind::BlockInstance {
                definition_id: id,
                transform: IfccadBlockTransform {
                    placement: IfccadPlacement {
                        origin: v(&n["origin"]),
                        x_axis: v(&n["x"]),
                        y_axis: v(&n["y"]),
                    },
                    rotation: p["extra_rotation"].as_f64().unwrap_or(0.),
                    scale: v(&p["scale"]),
                },
            },
        )];
    }
    d.model.entities = members;
    d.id_counters.next_block_id = if case.data["wide_picture"] == true {
        21
    } else {
        11
    };
    validate_ifccad_document(&d).unwrap();
    d
}
pub fn read_native(d: &IfccadDocument) -> Document {
    let bytes = encode_ifccad_document(d).unwrap();
    strict_native(bytes.bytes());
    let (cad, losses) = Ifcx.read(bytes.bytes()).unwrap();
    assert_retained_or_empty(&losses);
    cad
}
pub fn cad_source(case: &SpatialCase) -> Document {
    // Build the unrelated resources through the production importer, then set
    // every ordinary INSERT from the original independent CAD OCS inputs.
    let mut cad = read_native(&native_source(case));
    let handles: Vec<_> = cad
        .entities()
        .filter_map(|e| match e {
            EntityType::Insert(i) if i.block_name.starts_with("Level") => Some(i.common.handle),
            _ => None,
        })
        .collect();
    assert_eq!(handles.len(), case.data["poses"].as_array().unwrap().len());
    for handle in handles {
        let EntityType::Insert(i) = cad.get_entity_mut(handle).unwrap() else {
            panic!()
        };
        let index: usize = i.block_name.strip_prefix("Level").unwrap().parse().unwrap();
        let p = &case.data["poses"][index];
        i.insert_point = vector(v(&p["origin"]));
        i.normal = vector(v(&p["normal"]));
        i.rotation = p["rotation"].as_f64().unwrap() + p["extra_rotation"].as_f64().unwrap_or(0.);
        let scale = v(&p["scale"]);
        i.set_x_scale(scale[0]);
        i.set_y_scale(scale[1]);
        i.set_z_scale(scale[2]);
    }
    if case.data["unbraced"] == true {
        let handles: Vec<_> = cad
            .entities()
            .filter_map(|e| matches!(e, EntityType::MText(_)).then_some(e.common().handle))
            .collect();
        for h in handles {
            let EntityType::MText(m) = cad.get_entity_mut(h).unwrap() else {
                panic!()
            };
            m.value = r"\A1;240".into();
        }
    }
    cad
}

pub fn assert_retained_or_empty(losses: &[opencaddocument::Loss]) {
    assert!(
        losses.iter().all(|l| l
            .message
            .contains("graphics retained with numerical rounding")),
        "{losses:?}"
    );
}

fn curve(out: &mut Observation, key: &str, t: Affine, a: V, b: V, sweep: f64, full: bool) {
    out.insert(key,serde_json::json!({"center":t.origin,"a":linear(t.cols,a),"b":linear(t.cols,b),"sweep":sweep,"circle":full}));
}
fn polyline(out: &mut Observation, prefix: &str, t: Affine, vertices: &[[f64; 2]], bulges: &[f64]) {
    assert_eq!(vertices.len(), 2);
    assert_eq!(bulges, &[0.5, 0.]);
    for (i, p) in vertices.iter().enumerate() {
        out.point(&format!("{prefix}polyline/{i}"), t, [p[0], p[1], 0.]);
    }
    let (a, b) = (vertices[0], vertices[1]);
    let bulge = bulges[0];
    let f = (1. - bulge * bulge) / (4. * bulge);
    let center = [
        (a[0] + b[0]) / 2. - (b[1] - a[1]) * f,
        (a[1] + b[1]) / 2. + (b[0] - a[0]) * f,
        0.,
    ];
    let aa = [a[0] - center[0], a[1] - center[1], 0.];
    let bb = [-aa[1], aa[0], 0.];
    curve(
        out,
        &format!("{prefix}polyline/bulge"),
        t.then(Affine {
            origin: center,
            ..Affine::ID
        }),
        aa,
        bb,
        4. * bulge.atan(),
        false,
    );
}
fn text_observation(
    out: &mut Observation,
    key: &str,
    t: Affine,
    height: f64,
    value: &str,
    center: bool,
) {
    out.point(&format!("{key}/anchor"), t, [0.; 3]);
    out.point(&format!("{key}/direction"), t, [1., 0., 0.]);
    out.point(&format!("{key}/height"), t, [0., height, 0.]);
    out.insert(
        &format!("{key}/content"),
        serde_json::json!({"text":value,"format":if center {"center"}else{"plain"}}),
    );
}
fn native_text(value: &IfccadMText) -> (&str, bool) {
    assert_eq!(value.content.len(), 1);
    assert_eq!(value.content[0].inlines.len(), 1);
    let MTextInline::Run {
        text,
        character_format,
    } = &value.content[0].inlines[0]
    else {
        panic!()
    };
    assert!(!character_format.underline.unwrap_or(false));
    assert!(character_format.font.is_none());
    let position = character_format
        .position
        .or(value.character_format.position)
        .unwrap_or(TextPosition::Bottom);
    (text, position == TextPosition::Center)
}
pub fn native_world(d: &IfccadDocument) -> Observation {
    fn walk(
        d: &IfccadDocument,
        entities: &[IfccadEntity],
        t: Affine,
        prefix: &str,
        out: &mut Observation,
        depth: usize,
    ) {
        assert!(depth < 8);
        for entity in entities {
            let e = entity.as_native().unwrap();
            visit(d, &e.kind, t, prefix, e.layer_id, out, depth);
        }
    }
    fn visit(
        d: &IfccadDocument,
        kind: &IfccadEntityKind,
        t: Affine,
        prefix: &str,
        layer: u64,
        out: &mut Observation,
        depth: usize,
    ) {
        match kind {
            IfccadEntityKind::BlockInstance {
                definition_id,
                transform,
            } => {
                let b = d.blocks.iter().find(|b| b.id == *definition_id).unwrap();
                let child = t.then(frame(&transform.placement).block(
                    transform.rotation,
                    transform.scale,
                    b.base_point,
                ));
                let next = if b.name == "Picture counterparts" {
                    "ordinary/".into()
                } else if b.name == "Marker" {
                    format!("{prefix}marker/")
                } else {
                    prefix.into()
                };
                walk(d, &b.entities, child, &next, out, depth + 1);
            }
            IfccadEntityKind::LineSegment { start, end, .. } => {
                let key = if prefix.ends_with("marker/") {
                    prefix.to_owned()
                } else {
                    format!("{prefix}line/")
                };
                out.point(&format!("{key}start"), t, *start);
                out.point(&format!("{key}end"), t, *end);
            }
            IfccadEntityKind::Point { placement, .. } => {
                out.point(&format!("{prefix}point"), t, placement.origin)
            }
            IfccadEntityKind::Circle {
                radius, placement, ..
            } => out.conic(
                &format!("{prefix}circle"),
                t.then(frame(placement)),
                *radius,
                0.,
                TAU,
                true,
            ),
            IfccadEntityKind::Arc {
                radius,
                start_parameter,
                sweep_parameter,
                placement,
                ..
            } => out.conic(
                &format!("{prefix}arc"),
                t.then(frame(placement)),
                *radius,
                *start_parameter,
                *sweep_parameter,
                false,
            ),
            IfccadEntityKind::Ellipse {
                semi_major_radius,
                semi_minor_radius,
                placement,
            } => curve(
                out,
                &format!("{prefix}ellipse"),
                t.then(frame(placement)),
                [*semi_major_radius, 0., 0.],
                [0., *semi_minor_radius, 0.],
                TAU,
                true,
            ),
            IfccadEntityKind::PlanarPolyline {
                placement,
                vertices,
                bulges,
                closed,
                widths,
                ..
            } => {
                assert!(!closed);
                assert_eq!(
                    *widths,
                    IfccadPlanarPolylineWidths::Constant { width: 0.25 }
                );
                polyline(out, prefix, t.then(frame(placement)), vertices, bulges);
            }
            IfccadEntityKind::SpatialPolyline {
                vertices, closed, ..
            } => {
                assert!(!closed);
                assert_eq!(vertices.len(), 2);
                for (i, p) in vertices.iter().enumerate() {
                    out.point(&format!("{prefix}spatial/{i}"), t, *p);
                }
            }
            IfccadEntityKind::Solid(q) => {
                assert_eq!(q.thickness, 0.);
                let tt = t.then(frame(&q.placement));
                for (i, p) in q.corners.iter().enumerate() {
                    out.point(&format!("{prefix}solid/{i}"), tt, [p[0], p[1], 0.]);
                }
            }
            IfccadEntityKind::Text(v) => {
                assert_eq!(v.content.len(), 1);
                let TextLayout::Anchored { height, .. } = v.layout else {
                    panic!()
                };
                assert_eq!(
                    d.text_styles
                        .iter()
                        .find(|s| s.id == v.style_id)
                        .unwrap()
                        .name,
                    "Dimension text"
                );
                text_observation(
                    out,
                    &format!("{prefix}text"),
                    t.then(frame(&v.placement).block(v.rotation, [1.; 3], [0.; 3])),
                    height,
                    &v.content[0].text,
                    false,
                );
            }
            IfccadEntityKind::MText(v) => {
                assert_eq!(
                    d.text_styles
                        .iter()
                        .find(|s| s.id == v.style_id)
                        .unwrap()
                        .name,
                    "Dimension text"
                );
                let (text, center) = native_text(v);
                text_observation(
                    out,
                    &format!("{prefix}mtext"),
                    t.then(frame(&v.placement).block(v.rotation, [1.; 3], [0.; 3])),
                    v.height,
                    text,
                    center,
                );
            }
            IfccadEntityKind::Dimension(dim) => {
                let dt = t.then(frame(&dim.placement));
                let (a, b) = dim.definition.reference_points();
                out.point("dimension/first", dt, a);
                out.point("dimension/second", dt, b);
                let DimensionDefinition::Linear {
                    dimension_line_point,
                    ..
                } = &dim.definition
                else {
                    panic!()
                };
                out.point(
                    "dimension/line",
                    dt,
                    [dimension_line_point[0], dimension_line_point[1], 0.],
                );
                assert_eq!(
                    d.dimension_styles
                        .iter()
                        .find(|s| s.id == dim.style_id)
                        .unwrap()
                        .name,
                    "Building"
                );
                if let Some(picture) = &dim.graphics {
                    assert!(matches!(picture.parts.len(), 4 | 11));
                    assert!(matches!(
                        picture.parts[0].kind,
                        IfccadDimensionGraphicsKind::LineSegment { .. }
                    ));
                    assert!(matches!(
                        picture.parts[1].kind,
                        IfccadDimensionGraphicsKind::Circle { .. }
                    ));
                    assert!(matches!(
                        picture.parts[2].kind,
                        IfccadDimensionGraphicsKind::Point { .. }
                    ));
                    assert!(matches!(
                        picture.parts[3].kind,
                        IfccadDimensionGraphicsKind::BlockInstance { .. }
                    ));
                    for p in &picture.parts {
                        assert!(p.visible);
                        assert_eq!(p.line_pattern_scale, 1.);
                        assert_eq!(p.purpose, IfccadDimensionGraphicsPurpose::Display);
                        assert_eq!(p.layer_id.unwrap_or(layer), layer);
                        assert_eq!(p.appearance, part(p.kind.clone()).appearance);
                        visit(
                            d,
                            &p.kind.clone().into(),
                            dt,
                            "picture/",
                            layer,
                            out,
                            depth + 1,
                        );
                    }
                }
            }
            kind => panic!("unexpected native kind {kind:?}"),
        }
    }
    let mut out = Observation::new();
    walk(d, &d.model.entities, Affine::ID, "", &mut out, 0);
    out
}

pub fn cad_world(d: &Document) -> Observation {
    cad_owner_world(d, None)
}
pub fn cad_owner_world(d: &Document, owner: Option<&str>) -> Observation {
    fn walk(
        d: &Document,
        entities: Vec<&EntityType>,
        t: Affine,
        prefix: &str,
        out: &mut Observation,
        depth: usize,
    ) {
        assert!(depth < 8);
        for e in entities {
            match e {
                EntityType::Insert(i) => {
                    let b = d.block_records.get(&i.block_name).unwrap();
                    let next = if b.name == "Picture counterparts" {
                        "ordinary/".into()
                    } else if b.name == "Marker" {
                        format!("{prefix}marker/")
                    } else {
                        prefix.into()
                    };
                    let child = t.then(cad_frame(cv(i.insert_point), cv(i.normal)).block(
                        i.rotation,
                        [i.x_scale(), i.y_scale(), i.z_scale()],
                        cv(b.base_point),
                    ));
                    walk(
                        d,
                        d.entities_in_block(&i.block_name)
                            .filter(|e| e.common().owner_handle == b.handle)
                            .collect(),
                        child,
                        &next,
                        out,
                        depth + 1,
                    );
                }
                EntityType::Line(v) => {
                    let key = if prefix.ends_with("marker/") {
                        prefix.to_owned()
                    } else {
                        format!("{prefix}line/")
                    };
                    out.point(&format!("{key}start"), t, cv(v.start));
                    out.point(&format!("{key}end"), t, cv(v.end));
                }
                EntityType::Point(v) => out.point(&format!("{prefix}point"), t, cv(v.location)),
                EntityType::Circle(v) => out.conic(
                    &format!("{prefix}circle"),
                    t.then(cad_frame(cv(v.center), cv(v.normal))),
                    v.radius,
                    0.,
                    TAU,
                    true,
                ),
                EntityType::Arc(v) => out.conic(
                    &format!("{prefix}arc"),
                    t.then(cad_frame(cv(v.center), cv(v.normal))),
                    v.radius,
                    v.start_angle,
                    (v.end_angle - v.start_angle).rem_euclid(TAU),
                    false,
                ),
                EntityType::Ellipse(v) => {
                    assert_eq!(v.start_parameter, 0.);
                    assert_eq!(v.end_parameter, TAU);
                    let aa = cv(v.major_axis);
                    let bb = mul(cross(unit(cv(v.normal)), aa), v.minor_axis_ratio);
                    curve(
                        out,
                        &format!("{prefix}ellipse"),
                        t.then(Affine {
                            origin: cv(v.center),
                            ..Affine::ID
                        }),
                        aa,
                        bb,
                        TAU,
                        true,
                    );
                }
                EntityType::LwPolyline(v) => {
                    assert!(!v.is_closed);
                    for vertex in &v.vertices {
                        for width in [vertex.start_width, vertex.end_width] {
                            assert_eq!(if width == 0. { v.constant_width } else { width }, 0.25);
                        }
                    }
                    let local = cad_frame([0., 0., v.elevation], cv(v.normal));
                    polyline(
                        out,
                        prefix,
                        t.then(local),
                        &v.vertices
                            .iter()
                            .map(|p| [p.location.x, p.location.y])
                            .collect::<Vec<_>>(),
                        &v.vertices.iter().map(|p| p.bulge).collect::<Vec<_>>(),
                    );
                }
                EntityType::Polyline3D(v) => {
                    assert!(!v.flags.closed);
                    assert_eq!(v.vertices.len(), 2);
                    for (i, p) in v.vertices.iter().enumerate() {
                        out.point(&format!("{prefix}spatial/{i}"), t, cv(p.position));
                    }
                }
                EntityType::Solid(v) => {
                    assert!(!v.is_trace);
                    assert_eq!(v.thickness, 0.);
                    let tt = t.then(cad_frame([0.; 3], cv(v.normal)));
                    for (i, p) in [
                        v.first_corner,
                        v.second_corner,
                        v.third_corner,
                        v.fourth_corner,
                    ]
                    .iter()
                    .enumerate()
                    {
                        out.point(&format!("{prefix}solid/{i}"), tt, cv(*p));
                    }
                }
                EntityType::Text(v) => {
                    assert_eq!(v.generation_flags, 0);
                    assert!(v.style.eq_ignore_ascii_case("Dimension text"));
                    text_observation(
                        out,
                        &format!("{prefix}text"),
                        t.then(
                            cad_frame(cv(v.insertion_point), cv(v.normal))
                                .block(v.rotation, [1.; 3], [0.; 3]),
                        ),
                        v.height,
                        &v.value,
                        false,
                    );
                }
                EntityType::MText(v) => {
                    assert!(v.style.eq_ignore_ascii_case("Dimension text"));
                    let (value, center) = match v.value.as_str() {
                        "1200 mm" | "{1200 mm}" => ("1200 mm", false),
                        r"\A1;240" | r"{\A1;240}" => ("240", true),
                        value => panic!("unexpected fixture text {value}"),
                    };
                    let x = unit(v.dwg_x_direction.map(cv).unwrap_or([
                        v.rotation.cos(),
                        v.rotation.sin(),
                        0.,
                    ]));
                    let z = unit(cv(v.normal));
                    text_observation(
                        out,
                        &format!("{prefix}mtext"),
                        t.then(Affine {
                            cols: [x, cross(z, x), z],
                            origin: cv(v.insertion_point),
                        }),
                        v.height,
                        value,
                        center,
                    );
                }
                EntityType::Dimension(dim) => {
                    let (a, b) = match dim.as_ref() {
                        Dimension::Linear(v) => (v.first_point, v.second_point),
                        _ => panic!(),
                    };
                    out.point("dimension/first", t, cv(a));
                    out.point("dimension/second", t, cv(b));
                    out.point("dimension/line", t, cv(dim.base().definition_point));
                    assert_eq!(dim.base().style_name, "Building");
                    let name = &dim.base().block_name;
                    if name.is_empty() {
                        continue;
                    }
                    let b = d.block_records.get(name).unwrap();
                    let members: Vec<_> = d
                        .entities_in_block(name)
                        .filter(|e| {
                            e.common().owner_handle == b.handle
                                && !matches!(e, EntityType::Block(_) | EntityType::BlockEnd(_))
                        })
                        .collect();
                    assert!(matches!(members.len(), 4 | 11));
                    assert!(matches!(members[0], EntityType::Line(_)));
                    assert!(matches!(members[1], EntityType::Circle(_)));
                    assert!(matches!(members[2], EntityType::Point(_)));
                    assert!(matches!(members[3], EntityType::Insert(_)));
                    walk(d, members, t, "picture/", out, depth + 1);
                }
                EntityType::Block(_) | EntityType::BlockEnd(_) => {}
                EntityType::Viewport(v) if prefix.is_empty() && v.id == 1 => {}
                kind => panic!("unexpected CAD kind {kind:?}"),
            }
        }
    }
    let entities = match owner {
        None => d.model_space_entities().collect(),
        Some(name) => {
            let record = d.block_records.get(name).unwrap();
            let entities: Vec<_> = d
                .entities_in_block(name)
                .filter(|e| e.common().owner_handle == record.handle)
                .collect();
            assert_eq!(
                entities
                    .iter()
                    .filter(|e| matches!(e,EntityType::Viewport(v) if v.id==1))
                    .count(),
                1
            );
            assert_eq!(
                entities
                    .iter()
                    .filter(|e| matches!(e, EntityType::Insert(_)))
                    .count(),
                1
            );
            entities
        }
    };
    let mut out = Observation::new();
    walk(d, entities, Affine::ID, "", &mut out, 0);
    out
}

#[derive(Clone, Copy, Debug)]
pub enum Transport {
    Direct,
    AsciiDxf,
    BinaryDxf,
    Dwg,
}
pub fn transport(d: &Document, t: Transport) -> Document {
    let outcome = match t {
        Transport::Direct => return d.clone(),
        Transport::AsciiDxf => {
            DxfReader::from_reader(Cursor::new(DxfWriter::new(d).write_to_vec().unwrap()))
                .unwrap()
                .read_with_stats()
                .unwrap()
        }
        Transport::BinaryDxf => DxfReader::from_reader(Cursor::new(
            DxfWriter::new_binary(d).write_to_vec().unwrap(),
        ))
        .unwrap()
        .read_with_stats()
        .unwrap(),
        Transport::Dwg => {
            let bytes = DwgWriter::write_to_vec(d).unwrap();
            assert!(bytes.starts_with(b"AC1032"));
            DwgReader::from_stream(Cursor::new(bytes))
                .read_with_stats()
                .unwrap()
        }
    };
    assert!(outcome.stats.record_stream_read && outcome.stats.stream_completed);
    assert!(!outcome.stats.recovery_mode);
    assert_eq!(outcome.stats.recovered_errors, 0);
    assert_eq!(outcome.stats.skipped_source_records, 0);
    assert!(
        outcome.stats.diagnostics.is_empty(),
        "{:?}",
        outcome.stats.diagnostics
    );
    assert_eq!(
        outcome.stats.output_entities,
        outcome.document.entities().count()
    );
    outcome.document
}
pub fn assert_three_cycles(case: &SpatialCase, route: Transport, cad_first: bool) {
    let direction = if cad_first { "cad" } else { "native" };
    let mut cad = if cad_first {
        cad_source(case)
    } else {
        let source = native_source(case);
        case.assert_world(&native_world(&source), direction, "original");
        read_native(&source)
    };
    case.assert_world(&cad_world(&cad), direction, "first CAD");
    let mut first = None;
    for cycle in 1..=3 {
        cad = transport(&cad, route);
        case.assert_world(
            &cad_world(&cad),
            direction,
            &format!("{route:?} transport {cycle}"),
        );
        let (bytes, losses) = Ifcx.write(&cad).unwrap();
        assert_retained_or_empty(&losses);
        let native = strict_native(&bytes);
        let observed = native_world(&native);
        case.assert_world(&observed, direction, &format!("{route:?} native {cycle}"));
        if let Some(ref initial) = first {
            observed.assert_matches(
                initial,
                case.mm(),
                case.budget(),
                "additional drift since first output",
            );
        } else {
            first = Some(observed);
        }
        let (back, losses) = Ifcx.read(&bytes).unwrap();
        assert_retained_or_empty(&losses);
        cad = back;
        case.assert_world(
            &cad_world(&cad),
            direction,
            &format!("{route:?} CAD {cycle}"),
        );
    }
}
