use ifccad::{dimension::*, *};
use serde_json::json;

pub fn definitions() -> Vec<DimensionDefinition> {
    [

        json!({"kind":"radius","center":[0.,0.,0.],"curvePoint":[3.,4.,0.],"leaderLength":0.}),
        json!({"kind":"diameter","firstDiameterPoint":[-3.,-4.,0.],"secondDiameterPoint":[3.,4.,0.],"leaderLength":0.}),
        json!({"kind":"angularTwoLines","line1Start":[0.,0.,0.],"line1End":[1.,0.,0.],"line2Start":[0.,0.,0.],"line2End":[0.,1.,0.],"ray1Sign":"positive","ray2Sign":"positive","traversal":"counterclockwise","dimensionArcPoint":[5.,5.]}),
        json!({"kind":"angularThreePoints","vertex":[0.,0.,0.],"firstPoint":[1.,0.,0.],"secondPoint":[0.,1.,0.],"traversal":"counterclockwise","dimensionArcPoint":[5.,5.]}),
        json!({"kind":"ordinate","origin":[0.,0.,0.],"featurePoint":[3.,4.,0.],"axisAngle":0.,"leaderEnd":[10.,10.],"valueMode":"absolute"}),
        json!({"kind":"arcLength","center":[0.,0.,0.],"radius":5.,"startParameter":0.,"sweep":{"kind":"partial","radians":std::f64::consts::FRAC_PI_2},"dimensionArcPoint":[6.,6.]}),
        json!({"kind":"joggedRadius","center":[0.,0.,0.],"curvePoint":[3.,4.,0.],"displayCenter":[1.,1.],"jogPoint":[2.,2.],"jogAngle":0.5}),
    ].into_iter().map(|value| serde_json::from_value(value).unwrap()).collect()
}

pub fn drawing(definition: DimensionDefinition) -> IfccadDocument {
    let mut drawing = load_ifccad_bytes(
        include_bytes!("../../tests/fixtures/dimension-linear-aligned.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    let mut found = false;
    drawing.model.entities.retain_mut(|entity| {
        let Some(native) = entity.as_native_mut() else {
            return true;
        };
        let IfccadEntityKind::Dimension(d) = &mut native.kind else {
            return true;
        };
        if found {
            return false;
        }
        found = true;
        d.definition = definition.clone();
        d.associations.clear();
        d.graphics = None;
        d.placement = IfccadPlacement {
            origin: [0.; 3],
            x_axis: [1., 0., 0.],
            y_axis: [0., 1., 0.],
        };
        true
    });
    validate_ifccad_document(&drawing).unwrap();
    drawing
}

pub fn dimension(drawing: &IfccadDocument) -> &IfccadDimension {
    drawing
        .model
        .entities
        .iter()
        .filter_map(IfccadEntity::as_native)
        .find_map(|e| match &e.kind {
            IfccadEntityKind::Dimension(d) => Some(d.as_ref()),
            _ => None,
        })
        .unwrap()
}

pub fn gallery() -> IfccadDocument {
    let mut definitions = definitions();
    definitions.insert(
        0,
        DimensionDefinition::Aligned {
            first_point: [0.; 3],
            second_point: [6., 4., 0.],
            dimension_line_point: [0., 5.],
            extension_direction_angle: None,
        },
    );
    definitions.insert(
        0,
        DimensionDefinition::Linear {
            first_point: [0.; 3],
            second_point: [8., 0., 0.],
            direction_angle: 0.,
            dimension_line_point: [0., 3.],
            extension_direction_angle: None,
        },
    );
    let mut result = drawing(definitions[0].clone());
    let template = result
        .model
        .entities
        .iter()
        .filter_map(IfccadEntity::as_native)
        .find(|e| matches!(e.kind, IfccadEntityKind::Dimension(_)))
        .unwrap()
        .clone();
    let style_id = dimension(&result).style_id;
    let style = result
        .dimension_styles
        .iter_mut()
        .find(|s| s.id == style_id)
        .unwrap();
    let text_style = style.settings.text.style;
    let mut settings = DimensionStyleSettings::new(text_style);
    settings.text.height = 0.8;
    settings.arrows.size = 0.3;
    style.settings = settings.clone();
    style.type_overrides.clear();
    result.model.entities.clear();
    for (i, definition) in definitions.into_iter().enumerate() {
        let mut entity = template.clone();
        entity.id = (i + 1) as u64;
        let IfccadEntityKind::Dimension(d) = &mut entity.kind else {
            unreachable!()
        };
        d.definition = definition;
        d.overrides = Default::default();
        d.text_placement = Default::default();
        d.label = DimensionLabel::Generated;
        d.placement.origin = [(i % 3) as f64 * 30., -((i / 3) as f64) * 25., 0.];
        d.graphics = Some(picture(&d.definition, &settings));
        result.model.entities.push(IfccadEntity::Native(entity));
    }
    result.model.bounds = None;
    result.model.bounds_quality = None;
    recompute_ifccad_document_bounds(&mut result).unwrap();
    validate_ifccad_document(&result).unwrap();
    result
}

fn plane(p: [f64; 3]) -> IfccadPlacement {
    IfccadPlacement {
        origin: p,
        x_axis: [1., 0., 0.],
        y_axis: [0., 1., 0.],
    }
}

fn text(
    value: String,
    at: [f64; 3],
    style_id: IfccadTextStyleId,
    height: f64,
) -> IfccadDimensionGraphicsKind {
    use ifccad::text::*;
    IfccadDimensionGraphicsKind::Text(IfccadText {
        style_id,
        placement: plane(at),
        rotation: 0.,
        backward: false,
        upside_down: false,
        layout: TextLayout::Anchored {
            horizontal: TextHorizontalAlignment::Left,
            vertical: TextVerticalAlignment::Baseline,
            height,
            width_factor: 1.,
        },
        oblique_angle: 0.,
        thickness: 0.,
        content: vec![TextRun {
            text: value,
            ..Default::default()
        }],
    })
}

fn picture(
    definition: &DimensionDefinition,
    settings: &IfccadDimensionSettings,
) -> IfccadDimensionGraphics {
    use IfccadDimensionGraphicsKind as G;
    let line = |a: [f64; 3], b: [f64; 3]| G::LineSegment {
        start: a,
        end: b,
        thickness: 0.,
        extrusion_direction: [0., 0., 1.],
    };
    let mut shapes = vec![];
    let arrow = |tip: [f64; 3], towards: [f64; 3]| {
        let dx = towards[0] - tip[0];
        let dy = towards[1] - tip[1];
        let length = dx.hypot(dy);
        let ux = dx / length;
        let uy = dy / length;
        let a = [
            tip[0] + 0.35 * ux - 0.12 * uy,
            tip[1] + 0.35 * uy + 0.12 * ux,
        ];
        let b = [
            tip[0] + 0.35 * ux + 0.12 * uy,
            tip[1] + 0.35 * uy - 0.12 * ux,
        ];
        G::Solid(IfccadFilledQuadrilateral {
            placement: plane([0.; 3]),
            corners: [[tip[0], tip[1]], a, b, b],
            thickness: 0.,
        })
    };
    let label = match definition {
        DimensionDefinition::Radius {
            center,
            curve_point,
            ..
        }
        | DimensionDefinition::JoggedRadius {
            center,
            curve_point,
            ..
        } => {
            shapes.push(G::Circle {
                placement: plane(*center),
                radius: 5.,
                thickness: 0.,
            });
            if let DimensionDefinition::JoggedRadius {
                display_center,
                jog_point,
                ..
            } = definition
            {
                shapes.push(line(
                    [display_center[0], display_center[1], 0.],
                    [jog_point[0] - 0.3, jog_point[1] + 0.3, 0.],
                ));
                shapes.push(line(
                    [jog_point[0] - 0.3, jog_point[1] + 0.3, 0.],
                    [jog_point[0] + 0.3, jog_point[1] - 0.3, 0.],
                ));
                shapes.push(line(
                    [jog_point[0] + 0.3, jog_point[1] - 0.3, 0.],
                    *curve_point,
                ));
            } else {
                shapes.push(line(*center, *curve_point));
            }
            shapes.push(arrow(*curve_point, *center));
            [4., 5., 0.]
        }
        DimensionDefinition::Diameter {
            first_diameter_point,
            second_diameter_point,
            ..
        } => {
            shapes.push(G::Circle {
                placement: plane([0.; 3]),
                radius: 5.,
                thickness: 0.,
            });
            shapes.push(line(*first_diameter_point, *second_diameter_point));
            shapes.push(arrow(*first_diameter_point, *second_diameter_point));
            shapes.push(arrow(*second_diameter_point, *first_diameter_point));
            [4., 5., 0.]
        }
        DimensionDefinition::AngularTwoLines { .. }
        | DimensionDefinition::AngularThreePoints { .. } => {
            shapes.push(line([0.; 3], [9., 0., 0.]));
            shapes.push(line([0.; 3], [0., 9., 0.]));
            shapes.push(G::Arc {
                placement: plane([0.; 3]),
                radius: 50_f64.sqrt(),
                start_parameter: 0.,
                sweep_parameter: std::f64::consts::FRAC_PI_2,
                thickness: 0.,
            });
            let r = 50_f64.sqrt();
            shapes.push(arrow([r, 0., 0.], [r, 1., 0.]));
            shapes.push(arrow([0., r, 0.], [1., r, 0.]));
            [5.5, 5.5, 0.]
        }
        DimensionDefinition::Ordinate {
            feature_point,
            leader_end,
            ..
        } => {
            shapes.push(line([0.; 3], *feature_point));
            shapes.push(line(*feature_point, [leader_end[0], feature_point[1], 0.]));
            shapes.push(line(
                [leader_end[0], feature_point[1], 0.],
                [leader_end[0], leader_end[1], 0.],
            ));
            [10.5, 10., 0.]
        }
        DimensionDefinition::ArcLength {
            radius,
            start_parameter,
            sweep,
            ..
        } => {
            shapes.push(G::Arc {
                placement: plane([0.; 3]),
                radius: *radius,
                start_parameter: *start_parameter,
                sweep_parameter: sweep.radians(),
                thickness: 0.,
            });
            shapes.push(G::Arc {
                placement: plane([0.; 3]),
                radius: 72_f64.sqrt(),
                start_parameter: *start_parameter,
                sweep_parameter: sweep.radians(),
                thickness: 0.,
            });
            shapes.push(line([5., 0., 0.], [9., 0., 0.]));
            shapes.push(line([0., 5., 0.], [0., 9., 0.]));
            let r = 72_f64.sqrt();
            shapes.push(arrow([r, 0., 0.], [r, 1., 0.]));
            shapes.push(arrow([0., r, 0.], [1., r, 0.]));
            [6.5, 6.5, 0.]
        }
        DimensionDefinition::Linear {
            first_point,
            second_point,
            ..
        }
        | DimensionDefinition::Aligned {
            first_point,
            second_point,
            ..
        } => {
            let offset = if definition.kind() == DimensionKind::Linear {
                [0., 3., 0.]
            } else {
                [-2., 3., 0.]
            };
            let a = std::array::from_fn(|i| first_point[i] + offset[i]);
            let b = std::array::from_fn(|i| second_point[i] + offset[i]);
            shapes.extend([
                line(*first_point, *second_point),
                line(*first_point, a),
                line(*second_point, b),
                line(a, b),
            ]);
            shapes.push(arrow(a, b));
            shapes.push(arrow(b, a));
            [1., 8., 0.]
        }
    };
    let value = format_dimension_kind_label(
        definition.kind(),
        evaluate_dimension(definition).unwrap(),
        &settings.numbers,
        DimensionFormatContext { is_paper: false },
        &DimensionLabel::Generated,
    )
    .unwrap();
    let mut label_text = value.runs.into_iter().map(|r| r.text).collect::<String>();
    if matches!(
        definition.kind(),
        DimensionKind::AngularTwoLines | DimensionKind::AngularThreePoints
    ) {
        label_text.push('°');
    }
    shapes.push(text(label_text, label, settings.text.style, 0.8));
    shapes.push(text(
        format!("{:?}", definition.kind()),
        [-6., -8., 0.],
        settings.text.style,
        0.8,
    ));
    IfccadDimensionGraphics {
        origin: IfccadDimensionGraphicsOrigin::Generated,
        parts: shapes
            .into_iter()
            .map(|kind| IfccadDimensionGraphicsPart {
                kind,
                layer_id: None,
                appearance: IfccadEntityAppearance {
                    color: IfccadMode::ByLayer,
                    opacity: IfccadMode::ByLayer,
                    line_pattern: IfccadMode::ByLayer,
                    line_weight: IfccadMode::ByLayer,
                },
                visible: true,
                line_pattern_scale: 1.,
                purpose: IfccadDimensionGraphicsPurpose::Display,
            })
            .collect(),
    }
}
