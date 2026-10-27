//! Independent world-space observation for the placement regressions. No
//! Placement, Matrix3::arbitrary_axis or CAD INSERT transform calls are used.
use std::collections::BTreeMap;
use std::f64::consts::TAU;
use std::io::Cursor;

use ifccad::dimension::DimensionDefinition;
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
            let error = descriptor_error(a, b) * mm;
            assert!(
                error <= budget,
                "{context}: {key}: {error:.12e} mm > {budget:.3e} mm\nexpected {a}\nactual {b}"
            );
        }
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
    let IfccadEntityKind::LineSegment { start, end } =
        &mut d.model.entities[0].as_native_mut().unwrap().kind
    else {
        panic!()
    };
    *start = [0.; 3];
    *end = [4000. * f, 0., 0.];
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
            }),
            part(IfccadDimensionGraphicsKind::Circle {
                radius: 1.5 * f,
                placement: identity([2. * f, 4. * f, 0.]),
            }),
            part(IfccadDimensionGraphicsKind::Point {
                placement: identity([3. * f, 5. * f, f]),
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
    let marker = fresh(
        &mut d,
        &template,
        IfccadEntityKind::LineSegment {
            start: [0.; 3],
            end: [2. * f, f, 0.],
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
        },
    ));
    members.push(fresh(
        &mut d,
        &template,
        IfccadEntityKind::Arc {
            radius: 7.5 * f,
            start_parameter: 0.5,
            sweep_parameter: 1.5,
            placement: identity([-10. * f, 20. * f, 0.]),
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
    d.id_counters.next_block_id = 11;
    validate_ifccad_document(&d).unwrap();
    d
}
pub fn read_native(d: &IfccadDocument) -> Document {
    let bytes = encode_ifccad_document(d).unwrap();
    strict_native(bytes.bytes());
    let (cad, losses) = Ifcx.read(bytes.bytes()).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
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
    cad
}

pub fn native_world(d: &IfccadDocument) -> Observation {
    fn walk(
        d: &IfccadDocument,
        entities: &[IfccadEntity],
        t: Affine,
        marker: bool,
        out: &mut Observation,
        depth: usize,
    ) {
        assert!(depth < 8);
        for entity in entities {
            match &entity.as_native().unwrap().kind {
                IfccadEntityKind::BlockInstance {
                    definition_id,
                    transform,
                } => {
                    let b = d.blocks.iter().find(|b| b.id == *definition_id).unwrap();
                    walk(
                        d,
                        &b.entities,
                        t.then(frame(&transform.placement).block(
                            transform.rotation,
                            transform.scale,
                            b.base_point,
                        )),
                        marker,
                        out,
                        depth + 1,
                    );
                }
                IfccadEntityKind::LineSegment { start, end } => {
                    let prefix = if marker { "picture/marker" } else { "line" };
                    out.point(&format!("{prefix}/start"), t, *start);
                    out.point(&format!("{prefix}/end"), t, *end);
                }
                IfccadEntityKind::Circle { radius, placement } => {
                    out.conic("circle", t.then(frame(placement)), *radius, 0., TAU, true)
                }
                IfccadEntityKind::Arc {
                    radius,
                    start_parameter,
                    sweep_parameter,
                    placement,
                } => out.conic(
                    "arc",
                    t.then(frame(placement)),
                    *radius,
                    *start_parameter,
                    *sweep_parameter,
                    false,
                ),
                IfccadEntityKind::Dimension(dim) => {
                    let dt = t.then(frame(&dim.placement));
                    let (a, b) = dim.definition.reference_points();
                    out.point("dimension/first", dt, a);
                    out.point("dimension/second", dt, b);
                    let line = match &dim.definition {
                        DimensionDefinition::Linear {
                            dimension_line_point,
                            ..
                        } => *dimension_line_point,
                        _ => panic!(),
                    };
                    out.point("dimension/line", dt, [line[0], line[1], 0.]);
                    assert_eq!(
                        d.dimension_styles
                            .iter()
                            .find(|s| s.id == dim.style_id)
                            .unwrap()
                            .name,
                        "Building"
                    );
                    let Some(picture) = dim.graphics.as_ref() else {
                        continue;
                    };
                    assert_eq!(picture.parts.len(), 4);
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
                        assert_eq!(
                            p.layer_id.unwrap_or(entity.as_native().unwrap().layer_id),
                            entity.as_native().unwrap().layer_id
                        );
                        assert_eq!(p.appearance, part(p.kind.clone()).appearance);
                        match &p.kind {
                            IfccadDimensionGraphicsKind::LineSegment { start, end } => {
                                out.point("picture/line/start", dt, *start);
                                out.point("picture/line/end", dt, *end);
                            }
                            IfccadDimensionGraphicsKind::Circle { radius, placement } => out.conic(
                                "picture/circle",
                                dt.then(frame(placement)),
                                *radius,
                                0.,
                                TAU,
                                true,
                            ),
                            IfccadDimensionGraphicsKind::Point { placement } => {
                                out.point("picture/point", dt, placement.origin)
                            }
                            IfccadDimensionGraphicsKind::BlockInstance {
                                definition_id,
                                transform,
                            } => {
                                let b = d.blocks.iter().find(|b| b.id == *definition_id).unwrap();
                                walk(
                                    d,
                                    &b.entities,
                                    dt.then(frame(&transform.placement).block(
                                        transform.rotation,
                                        transform.scale,
                                        b.base_point,
                                    )),
                                    true,
                                    out,
                                    depth + 1,
                                );
                            }
                            kind => panic!("unexpected picture kind {kind:?}"),
                        }
                    }
                }
                kind => panic!("unexpected native kind {kind:?}"),
            }
        }
    }
    let mut out = Observation::new();
    walk(d, &d.model.entities, Affine::ID, false, &mut out, 0);
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
        picture: bool,
        marker: bool,
        out: &mut Observation,
        depth: usize,
    ) {
        assert!(depth < 8);
        for e in entities {
            match e {
                EntityType::Insert(i) => {
                    let b = d.block_records.get(&i.block_name).unwrap();
                    let child = cad_frame(cv(i.insert_point), cv(i.normal)).block(
                        i.rotation,
                        [i.x_scale(), i.y_scale(), i.z_scale()],
                        cv(b.base_point),
                    );
                    walk(
                        d,
                        d.entities_in_block(&i.block_name)
                            .filter(|e| e.common().owner_handle == b.handle)
                            .collect(),
                        t.then(child),
                        picture,
                        picture,
                        out,
                        depth + 1,
                    );
                }
                EntityType::Line(l) => {
                    let prefix = if marker {
                        "picture/marker"
                    } else if picture {
                        "picture/line"
                    } else {
                        "line"
                    };
                    out.point(&format!("{prefix}/start"), t, cv(l.start));
                    out.point(&format!("{prefix}/end"), t, cv(l.end));
                }
                EntityType::Circle(c) => out.conic(
                    if picture { "picture/circle" } else { "circle" },
                    t.then(cad_frame(cv(c.center), cv(c.normal))),
                    c.radius,
                    0.,
                    TAU,
                    true,
                ),
                EntityType::Arc(a) => out.conic(
                    "arc",
                    t.then(cad_frame(cv(a.center), cv(a.normal))),
                    a.radius,
                    a.start_angle,
                    (a.end_angle - a.start_angle).rem_euclid(TAU),
                    false,
                ),
                EntityType::Point(p) if picture => out.point("picture/point", t, cv(p.location)),
                EntityType::Dimension(dim) => {
                    let (a, b) = match dim.as_ref() {
                        Dimension::Linear(l) => (l.first_point, l.second_point),
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
                    let record = d.block_records.get(name).unwrap();
                    let members: Vec<_> = d
                        .entities_in_block(name)
                        .filter(|e| {
                            e.common().owner_handle == record.handle
                                && !matches!(e, EntityType::Block(_) | EntityType::BlockEnd(_))
                        })
                        .collect();
                    assert_eq!(members.len(), 4);
                    assert!(matches!(members[0], EntityType::Line(_)));
                    assert!(matches!(members[1], EntityType::Circle(_)));
                    assert!(matches!(members[2], EntityType::Point(_)));
                    assert!(matches!(members[3], EntityType::Insert(_)));
                    walk(d, members, t, true, false, out, depth + 1);
                }
                EntityType::Block(_) | EntityType::BlockEnd(_) => {}
                // The paper layout's overall viewport is sheet state, not
                // authored geometry; ownership is checked separately.
                EntityType::Viewport(v) if !picture && v.id == 1 => {}
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
    walk(d, entities, Affine::ID, false, false, &mut out, 0);
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
    match t {
        Transport::Direct => d.clone(),
        Transport::AsciiDxf => {
            DxfReader::from_reader(Cursor::new(DxfWriter::new(d).write_to_vec().unwrap()))
                .unwrap()
                .read()
                .unwrap()
        }
        Transport::BinaryDxf => DxfReader::from_reader(Cursor::new(
            DxfWriter::new_binary(d).write_to_vec().unwrap(),
        ))
        .unwrap()
        .read()
        .unwrap(),
        Transport::Dwg => {
            let bytes = DwgWriter::write_to_vec(d).unwrap();
            assert!(bytes.starts_with(b"AC1032"));
            DwgReader::from_stream(Cursor::new(bytes)).read().unwrap()
        }
    }
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
        assert!(
            losses.is_empty(),
            "{} {direction} {route:?} cycle {cycle}: {losses:?}",
            case.name()
        );
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
        assert!(losses.is_empty(), "{losses:?}");
        cad = back;
        case.assert_world(
            &cad_world(&cad),
            direction,
            &format!("{route:?} CAD {cycle}"),
        );
    }
}
