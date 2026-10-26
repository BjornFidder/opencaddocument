//! IFCX through the ifccad library. ifccad reads and writes the files; the
//! mapping between its drawing model and a [`Document`] lives here, so the two
//! libraries never depend on each other.
//!
//! Layers and model-space lines, circles, arcs, points and lightweight
//! polylines map both ways. Everything else is reported as a [`Loss`].

use std::collections::{BTreeMap, HashMap};
use std::f64::consts::TAU;

use acadrust::entities::{Arc, Circle, EntityCommon, Line, LwPolyline, LwVertex, Point};
use acadrust::tables::Layer;
use acadrust::types::{Color, LineWeight, Transparency, Vector2, Vector3};
use acadrust::EntityType;
use ifccad::{
    encode_ifccad_document, load_ifccad_bytes, IfccadColor, IfccadDocument, IfccadEntity,
    IfccadEntityAppearance, IfccadEntityKind, IfccadHeader, IfccadLayer, IfccadLayerAppearance,
    IfccadLinePatternGeneration, IfccadLinePatternId, IfccadMode, IfccadNativeEntity,
    IfccadPlacement, IfccadPlanarPolylineWidths,
};

use crate::{Document, Error, Format, Loss, Placement};

// Custom document properties that carry an IFCX file's identity through
// editing, so a drawing read from IFCX is written back as the same drawing.
const HEADER_ID: &str = "IFCX id";
const DATA_VERSION: &str = "IFCX data version";
const DRAWING_ID: &str = "IFCX drawing id";

/// An empty drawing: a model layout and the continuous line pattern (id 0)
/// that layers and entities refer to.
const TEMPLATE: &str = r#"{
  "header": {
    "id": "opencad-drawing",
    "ifcxVersion": "ifcx_alpha",
    "dataVersion": "0.1.0",
    "author": "OpenCAD",
    "timestamp": "1970-01-01T00:00:00Z"
  },
  "imports": [{ "uri": "urn:example:ifccad:0.1.0" }],
  "schemas": {},
  "data": [
    {
      "path": "/cad/d1",
      "children": {
        "linePattern0": "/cad/d1/linePattern/0",
        "model": "/cad/d1/layout/1"
      },
      "attributes": {
        "ifccad::drawing": {
          "lengthUnit": "mm",
          "nextBlockId": 1,
          "nextEntityId": 1,
          "nextLayerId": 1,
          "nextLayoutId": 2,
          "nextLinePatternId": 1,
          "nextSymbolSourceId": 1,
          "profileVersion": "0.1.0",
          "nextUcsId": 1,
          "nextModelWindowId": 1
        }
      }
    },
    {
      "path": "/cad/d1/linePattern/0",
      "attributes": { "ifccad::linePattern": { "name": "Continuous", "pattern": [] } }
    },
    {
      "path": "/cad/d1/layout/1",
      "attributes": { "ifccad::layout": { "kind": "Model", "tabIndex": 0 } }
    }
  ]
}"#;

/// INSUNITS codes 0-24 in order, as IFCX length-unit tokens.
const UNITS: [&str; 25] = [
    "unitless", "in", "ft", "mi", "mm", "cm", "m", "km", "microin", "mil", "yd", "angstrom",
    "nm", "um", "dm", "dam", "hm", "Gm", "au", "ly", "pc", "usSurveyFoot", "usSurveyInch",
    "usSurveyYard", "usSurveyMile",
];

/// Placements that are not plain plan-view frames convert through
/// trigonometry; they are written snapped so that reading a file and writing
/// it again produces the same bytes (see [`Placement::snapped`]).
const POSITION_STEP: f64 = 1e-8;
const ANGLE_STEP: f64 = 1e-12;

pub struct Ifcx;

impl Format for Ifcx {
    fn name(&self) -> &str {
        "IFCX"
    }

    fn extensions(&self) -> &[&str] {
        &["ifcx"]
    }

    fn read(&self, bytes: &[u8]) -> Result<(Document, Vec<Loss>), Error> {
        let drawing = load_ifccad_bytes(bytes, Default::default())?.into_document();
        let mut losses = Losses::default();
        let mut document = Document::new();
        document.header.insertion_units = UNITS
            .iter()
            .position(|unit| *unit == drawing.length_unit)
            .unwrap_or(0) as i16;
        let info = &mut document.summary_info;
        set(&mut info.custom_properties, HEADER_ID, drawing.header.id.clone());
        set(&mut info.custom_properties, DATA_VERSION, drawing.header.data_version.clone());
        set(&mut info.custom_properties, DRAWING_ID, drawing.drawing_id.to_string());
        if info.author.is_empty() {
            info.author = drawing.header.author.clone();
        }

        let mut layer_names = HashMap::new();
        for layer in &drawing.layers {
            layer_names.insert(layer.id, layer.name.clone());
            if layer.appearance.line_pattern != continuous(&drawing) {
                losses.note(format!("the line pattern of layer {}", layer.name));
            }
            let mut converted = document
                .layers
                .get(&layer.name)
                .cloned()
                .unwrap_or_else(|| Layer::new(layer.name.clone()));
            converted.color = color_in(&layer.appearance.color);
            converted.line_weight = weight_in(layer.appearance.line_weight);
            converted.flags.off = !layer.visible;
            converted.flags.frozen = layer.frozen;
            converted.flags.locked = layer.locked;
            converted.is_plottable = layer.plottable;
            match document.layers.get_mut(&layer.name) {
                Some(existing) => *existing = converted,
                None => {
                    document.layers.add(converted)?;
                }
            }
        }

        for entity in &drawing.model.entities {
            let IfccadEntity::Native(entity) = entity else {
                losses.note("an entity ifccad keeps only as opaque data");
                continue;
            };
            let Some(mut converted) = entity_in(&entity.kind) else {
                losses.note(kind_name(&entity.kind));
                continue;
            };
            let common = converted.common_mut();
            common.layer = layer_names
                .get(&entity.layer_id)
                .cloned()
                .unwrap_or_else(|| "0".to_string());
            appearance_in(&entity.appearance, &drawing, common, &mut losses);
            common.invisible = !entity.visible;
            common.linetype_scale = entity.line_pattern_scale;
            document.add_entity(converted)?;
        }
        if !drawing.paper_layouts.is_empty() {
            losses.note("paper layouts");
        }
        if !drawing.blocks.is_empty() {
            losses.note("block definitions");
        }
        Ok((document, losses.into_vec()))
    }

    fn write(&self, document: &Document) -> Result<(Vec<u8>, Vec<Loss>), Error> {
        let mut drawing = load_ifccad_bytes(TEMPLATE.as_bytes(), Default::default())?
            .into_document();
        let mut losses = Losses::default();
        let info = &document.summary_info;
        let get = |name: &str| {
            info.custom_properties
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        };
        drawing.header = IfccadHeader {
            id: get(HEADER_ID).unwrap_or_else(|| "opencad-drawing".to_string()),
            data_version: get(DATA_VERSION).unwrap_or_else(|| "0.1.0".to_string()),
            author: match info.author.as_str() {
                "" => "OpenCAD".to_string(),
                author => author.to_string(),
            },
            timestamp: rfc3339(now_seconds()),
        };
        drawing.drawing_id = get(DRAWING_ID).and_then(|id| id.parse().ok()).unwrap_or(1);
        drawing.length_unit = UNITS
            .get(document.header.insertion_units as usize)
            .unwrap_or(&"unitless")
            .to_string();

        let mut layer_ids = HashMap::new();
        for layer in document.layers.iter() {
            if !is_continuous(&layer.line_type) {
                losses.note(format!("the linetype of layer {}", layer.name));
            }
            let id = drawing.id_counters.allocate_layer_id()?;
            layer_ids.insert(layer.name.to_ascii_lowercase(), id);
            drawing.layers.push(IfccadLayer {
                id,
                name: layer.name.clone(),
                description: None,
                visible: !layer.is_off(),
                frozen: layer.is_frozen(),
                locked: layer.is_locked(),
                plottable: layer.is_plottable,
                frozen_in_new_viewports: false,
                appearance: IfccadLayerAppearance {
                    color: color_out(layer.color).unwrap_or(IfccadColor::Foreground),
                    opacity: 1.0,
                    line_pattern: IfccadLinePatternId(0),
                    line_weight: weight_out(layer.line_weight).unwrap_or(0.25),
                },
            });
        }
        let layer_zero = layer_ids.get("0").copied().unwrap_or(0);

        let mut model = 0;
        for entity in document.model_space_entities() {
            model += 1;
            let Some(kind) = entity_out(entity, &mut losses) else {
                losses.note(entity.as_entity().entity_type());
                continue;
            };
            let common = entity.common();
            let layer_id = layer_ids
                .get(&common.layer.to_ascii_lowercase())
                .copied()
                .unwrap_or(layer_zero);
            drawing.model.entities.push(IfccadEntity::Native(IfccadNativeEntity {
                id: drawing.id_counters.allocate_entity_id()?,
                visible: !common.invisible,
                layer_id,
                appearance: appearance_out(common, &mut losses),
                line_pattern_scale: common.linetype_scale,
                kind,
            }));
        }
        let elsewhere = document.entities().count() - model;
        if elsewhere > 0 {
            losses.note(format!(
                "{elsewhere} entities outside model space (paper layouts, block definitions)"
            ));
        }

        let encoded = encode_ifccad_document(&drawing)?;
        Ok((encoded.bytes().to_vec(), losses.into_vec()))
    }
}

/// What a conversion leaves behind, counted per kind.
#[derive(Default)]
struct Losses(BTreeMap<String, usize>);

impl Losses {
    fn note(&mut self, what: impl Into<String>) {
        *self.0.entry(what.into()).or_default() += 1;
    }

    fn into_vec(self) -> Vec<Loss> {
        self.0
            .into_iter()
            .map(|(what, count)| Loss {
                handle: None,
                message: match count {
                    1 => format!("{what} is not converted"),
                    n => format!("{what} is not converted ({n}×)"),
                },
            })
            .collect()
    }
}

fn entity_in(kind: &IfccadEntityKind) -> Option<EntityType> {
    Some(match kind {
        IfccadEntityKind::LineSegment { start, end } => {
            EntityType::Line(Line::from_points(vector(*start), vector(*end)))
        }
        IfccadEntityKind::Point { placement } => {
            let mut point = Point::new();
            point.location = vector(placement.origin);
            EntityType::Point(point)
        }
        IfccadEntityKind::Circle { radius, placement } => {
            let (center, normal, _) = placement_in(placement);
            let mut circle = Circle::new();
            circle.center = center;
            circle.normal = normal;
            circle.radius = *radius;
            EntityType::Circle(circle)
        }
        IfccadEntityKind::Arc {
            radius,
            start_parameter,
            sweep_parameter,
            placement,
        } => {
            let (center, normal, rotation) = placement_in(placement);
            // A clockwise sweep is the same arc drawn from its other end.
            let start = start_parameter + rotation + sweep_parameter.min(0.0);
            let mut arc = Arc::new();
            arc.center = center;
            arc.normal = normal;
            arc.radius = *radius;
            arc.start_angle = start.rem_euclid(TAU);
            arc.end_angle = (start + sweep_parameter.abs()).rem_euclid(TAU);
            EntityType::Arc(arc)
        }
        IfccadEntityKind::PlanarPolyline {
            widths,
            vertices,
            bulges,
            closed,
            placement,
            line_pattern_generation,
        } => {
            let (origin, normal, rotation) = placement_in(placement);
            let (sin, cos) = rotation.sin_cos();
            let mut polyline = LwPolyline::new();
            polyline.normal = normal;
            polyline.elevation = origin.z;
            polyline.is_closed = *closed;
            polyline.plinegen = *line_pattern_generation == IfccadLinePatternGeneration::Continuous;
            for (index, [u, v]) in vertices.iter().enumerate() {
                // Local frame coordinates, rotated and moved into the OCS.
                let (x, y) = match rotation {
                    0.0 => (*u, *v),
                    _ => (u * cos - v * sin, u * sin + v * cos),
                };
                let mut vertex = LwVertex::new(Vector2::new(x + origin.x, y + origin.y));
                vertex.bulge = bulges.get(index).copied().unwrap_or(0.0);
                polyline.vertices.push(vertex);
            }
            match widths {
                IfccadPlanarPolylineWidths::Constant { width } => polyline.constant_width = *width,
                IfccadPlanarPolylineWidths::PerVertex { widths } => {
                    for (vertex, [start, end]) in polyline.vertices.iter_mut().zip(widths) {
                        vertex.start_width = *start;
                        vertex.end_width = *end;
                    }
                }
                IfccadPlanarPolylineWidths::Defaulted {
                    default_start_width,
                    default_end_width,
                    overrides,
                } => {
                    for (vertex, width) in polyline.vertices.iter_mut().zip(overrides) {
                        vertex.start_width = width.start.unwrap_or(*default_start_width);
                        vertex.end_width = width.end.unwrap_or(*default_end_width);
                    }
                }
            }
            EntityType::LwPolyline(polyline)
        }
        _ => return None,
    })
}

fn entity_out(entity: &EntityType, losses: &mut Losses) -> Option<IfccadEntityKind> {
    Some(match entity {
        EntityType::Line(line) => {
            if line.thickness != 0.0 {
                losses.note("line thickness");
            }
            IfccadEntityKind::LineSegment {
                start: array(line.start),
                end: array(line.end),
            }
        }
        EntityType::Point(point) => IfccadEntityKind::Point {
            placement: placement_out(point.location, Vector3::new(0.0, 0.0, 1.0)),
        },
        EntityType::Circle(circle) => IfccadEntityKind::Circle {
            radius: circle.radius,
            placement: placement_out(circle.center, circle.normal),
        },
        EntityType::Arc(arc) => IfccadEntityKind::Arc {
            radius: arc.radius,
            start_parameter: arc.start_angle,
            sweep_parameter: match (arc.end_angle - arc.start_angle).rem_euclid(TAU) {
                0.0 => TAU,
                sweep => sweep,
            },
            placement: placement_out(arc.center, arc.normal),
        },
        EntityType::LwPolyline(polyline) => {
            let vertex_widths = polyline
                .vertices
                .iter()
                .any(|vertex| vertex.start_width != 0.0 || vertex.end_width != 0.0);
            IfccadEntityKind::PlanarPolyline {
                widths: match vertex_widths {
                    true => IfccadPlanarPolylineWidths::PerVertex {
                        widths: polyline
                            .vertices
                            .iter()
                            .map(|vertex| [vertex.start_width, vertex.end_width])
                            .collect(),
                    },
                    false => IfccadPlanarPolylineWidths::Constant {
                        width: polyline.constant_width,
                    },
                },
                vertices: polyline
                    .vertices
                    .iter()
                    .map(|vertex| [vertex.location.x, vertex.location.y])
                    .collect(),
                bulges: polyline.vertices.iter().map(|vertex| vertex.bulge).collect(),
                closed: polyline.is_closed,
                placement: placement_out(
                    Vector3::new(0.0, 0.0, polyline.elevation),
                    polyline.normal,
                ),
                line_pattern_generation: match polyline.plinegen {
                    true => IfccadLinePatternGeneration::Continuous,
                    false => IfccadLinePatternGeneration::PerSegment,
                },
            }
        }
        _ => return None,
    })
}

/// A frame with origin `origin` in the object coordinate system of `normal`.
fn placement_out(origin: Vector3, normal: Vector3) -> IfccadPlacement {
    let placement = Placement::from_ocs(origin, normal, 0.0);
    // A plan-view frame converts exactly; leave it untouched.
    let placement = match normal == Vector3::new(0.0, 0.0, 1.0) {
        true => placement,
        false => placement.snapped(POSITION_STEP, ANGLE_STEP),
    };
    let (origin, x, y) = placement.to_axes();
    IfccadPlacement {
        origin: array(origin),
        x_axis: array(x),
        y_axis: array(y),
    }
}

/// OCS origin, normal and rotation of an IFCX frame.
fn placement_in(placement: &IfccadPlacement) -> (Vector3, Vector3, f64) {
    Placement::from_axes(
        vector(placement.origin),
        vector(placement.x_axis),
        vector(placement.y_axis),
    )
    .to_ocs()
}

fn appearance_out(common: &EntityCommon, losses: &mut Losses) -> IfccadEntityAppearance {
    let color = match common.color {
        Color::ByLayer => IfccadMode::ByLayer,
        Color::ByBlock => IfccadMode::ByBlock,
        color => match color_out(color) {
            Some(color) => IfccadMode::Explicit(color),
            None => {
                losses.note("an entity colour");
                IfccadMode::ByLayer
            }
        },
    };
    let line_pattern = match common.linetype.to_ascii_lowercase().as_str() {
        "" | "bylayer" => IfccadMode::ByLayer,
        "byblock" => IfccadMode::ByBlock,
        "continuous" => IfccadMode::Explicit(IfccadLinePatternId(0)),
        _ => {
            losses.note("an entity linetype");
            IfccadMode::ByLayer
        }
    };
    let line_weight = match common.line_weight {
        LineWeight::ByBlock => IfccadMode::ByBlock,
        weight => weight_out(weight).map_or(IfccadMode::ByLayer, IfccadMode::Explicit),
    };
    let opacity = match common.transparency {
        Transparency::ByLayer => IfccadMode::ByLayer,
        Transparency::ByBlock => IfccadMode::ByBlock,
        Transparency::Explicit(amount) => IfccadMode::Explicit(1.0 - f64::from(amount) / 255.0),
    };
    IfccadEntityAppearance {
        color,
        opacity,
        line_pattern,
        line_weight,
    }
}

fn appearance_in(
    appearance: &IfccadEntityAppearance,
    drawing: &IfccadDocument,
    common: &mut EntityCommon,
    losses: &mut Losses,
) {
    common.color = match &appearance.color {
        IfccadMode::ByLayer => Color::ByLayer,
        IfccadMode::ByBlock => Color::ByBlock,
        IfccadMode::Explicit(color) => color_in(color),
    };
    common.linetype = match appearance.line_pattern {
        IfccadMode::ByLayer => "ByLayer".to_string(),
        IfccadMode::ByBlock => "ByBlock".to_string(),
        IfccadMode::Explicit(pattern) if pattern == continuous(drawing) => {
            "Continuous".to_string()
        }
        IfccadMode::Explicit(_) => {
            losses.note("an entity line pattern");
            "ByLayer".to_string()
        }
    };
    common.line_weight = match appearance.line_weight {
        IfccadMode::ByLayer => LineWeight::ByLayer,
        IfccadMode::ByBlock => LineWeight::ByBlock,
        IfccadMode::Explicit(millimetres) => weight_in(millimetres),
    };
    common.transparency = match appearance.opacity {
        IfccadMode::ByLayer => Transparency::ByLayer,
        IfccadMode::ByBlock => Transparency::ByBlock,
        IfccadMode::Explicit(opacity) => {
            Transparency::Explicit(((1.0 - opacity.clamp(0.0, 1.0)) * 255.0).round() as u8)
        }
    };
}

/// The id of the drawing's continuous (solid) line pattern.
fn continuous(drawing: &IfccadDocument) -> IfccadLinePatternId {
    drawing
        .line_patterns
        .iter()
        .find(|pattern| pattern.pattern.is_empty())
        .map_or(IfccadLinePatternId(0), |pattern| pattern.id)
}

fn is_continuous(linetype: &str) -> bool {
    linetype.is_empty() || linetype.eq_ignore_ascii_case("continuous")
}

/// ACI 7 is the foreground colour; other indices keep their index identity.
fn color_out(color: Color) -> Option<IfccadColor> {
    match color {
        Color::Index(7) => Some(IfccadColor::Foreground),
        Color::Index(index) => {
            let (r, g, b) = color.rgb()?;
            Some(IfccadColor::Concrete {
                rgb: [r, g, b],
                indexed: Some(("ACI".to_string(), u64::from(index))),
                named: None,
            })
        }
        Color::Rgb { r, g, b } => Some(IfccadColor::Concrete {
            rgb: [r, g, b],
            indexed: None,
            named: None,
        }),
        _ => None,
    }
}

fn color_in(color: &IfccadColor) -> Color {
    match color {
        IfccadColor::Foreground => Color::Index(7),
        IfccadColor::Concrete { rgb, indexed, .. } => {
            let [r, g, b] = *rgb;
            match indexed {
                Some((system, index))
                    if system.eq_ignore_ascii_case("ACI")
                        && (1..=255).contains(index)
                        && Color::Index(*index as u8).rgb() == Some((r, g, b)) =>
                {
                    Color::Index(*index as u8)
                }
                _ => Color::Rgb { r, g, b },
            }
        }
    }
}

/// A line weight in millimetres; `None` for the inherited and default weights.
fn weight_out(weight: LineWeight) -> Option<f64> {
    match weight {
        LineWeight::Value(hundredths) => Some(f64::from(hundredths) / 100.0),
        _ => None,
    }
}

fn weight_in(millimetres: f64) -> LineWeight {
    LineWeight::Value((millimetres * 100.0).round() as i16)
}

fn vector([x, y, z]: [f64; 3]) -> Vector3 {
    Vector3::new(x, y, z)
}

fn array(v: Vector3) -> [f64; 3] {
    [v.x, v.y, v.z]
}

fn kind_name(kind: &IfccadEntityKind) -> &'static str {
    match kind {
        IfccadEntityKind::Solid(_) => "Solid",
        IfccadEntityKind::Trace(_) => "Trace",
        IfccadEntityKind::Dimension(_) => "Dimension",
        IfccadEntityKind::Hatch(_) => "Hatch",
        IfccadEntityKind::Text(_) => "Text",
        IfccadEntityKind::MText(_) => "MText",
        IfccadEntityKind::Viewport(_) => "Viewport",
        IfccadEntityKind::Ellipse { .. } => "Ellipse",
        IfccadEntityKind::EllipseArc { .. } => "EllipseArc",
        IfccadEntityKind::SpatialPolyline { .. } => "SpatialPolyline",
        IfccadEntityKind::BlockInstance { .. } => "BlockInstance",
        _ => "an entity",
    }
}

fn set(properties: &mut Vec<(String, String)>, name: &str, value: String) {
    match properties.iter_mut().find(|(key, _)| key == name) {
        Some(entry) => entry.1 = value,
        None => properties.push((name.to_string(), value)),
    }
}

fn now_seconds() -> u64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// `seconds` since the Unix epoch as an RFC 3339 UTC timestamp.
fn rfc3339(seconds: u64) -> String {
    // Civil date from a day count (Howard Hinnant's algorithm).
    let days = (seconds / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let time = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time / 3_600,
        time % 3_600 / 60,
        time % 60
    )
}

#[cfg(test)]
mod tests {
    use super::rfc3339;

    #[test]
    fn timestamps_are_utc_civil_dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_793_000_000), "2026-10-26T07:33:20Z");
    }
}
