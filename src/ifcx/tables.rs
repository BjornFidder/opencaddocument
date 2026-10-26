//! Drawing-wide tables: layers, text styles, the SHX files complex line
//! patterns draw symbols from, line patterns, UCSs and the point display.

use acadrust::tables::{
    Layer, LineType, LineTypeComplexContent, LineTypeComplexData, LineTypeElement, TextStyle, Ucs,
};
use ifccad::geometry_kernel::{CoordinateFrame3, Point3, Vector3 as KernelVector};
use ifccad::line_pattern::{
    validate_line_pattern_transform, validate_line_pattern_values, LinePatternElement,
    LinePatternRotation, LinePatternTransform, SymbolSource,
};
use ifccad::text::{FontRequest, TextStyleProperties};
use ifccad::{
    IfccadLayer, IfccadLayerAppearance, IfccadLinePattern, IfccadLinePatternId,
    IfccadLinePatternMarker, IfccadModelViewState, IfccadPointDisplay, IfccadPointGlyph,
    IfccadPointSize, IfccadSymbolSource, IfccadTextStyle, IfccadUcsDefinition, IfccadUcsSelection,
};

use super::*;

pub(super) fn read(
    drawing: &IfccadDocument,
    document: &mut Document,
    names: &mut Names,
    losses: &mut Losses,
) -> Result<(), Error> {
    document.header.linetype_scale = drawing.line_pattern_scale;
    if let Some(display) = drawing.point_display {
        let (mode, size) = point_display_in(display);
        document.header.point_display_mode = mode;
        document.header.point_display_size = size;
    }

    for source in &drawing.symbol_sources {
        let SymbolSource::Shx { file_name } = &source.source;
        let existing = document
            .text_styles
            .iter()
            .find(|style| style.is_shape_file && style.font_file.eq_ignore_ascii_case(file_name))
            .map(|style| style.handle);
        let handle = match existing {
            Some(handle) => handle,
            None => {
                let mut style = TextStyle::new("");
                style.handle = document.allocate_handle();
                style.is_shape_file = true;
                style.font_file = file_name.clone();
                let handle = style.handle;
                document.text_styles.add_allow_duplicate(style);
                handle
            }
        };
        names.symbols.insert(source.id, handle);
    }

    for style in &drawing.text_styles {
        names.styles.insert(style.id, style.name.clone());
        let properties = &style.properties;
        let mut converted = document
            .text_styles
            .get(&style.name)
            .cloned()
            .unwrap_or_else(|| TextStyle::new(style.name.clone()));
        converted.font_file = properties.font.cad_font_name.clone().unwrap_or_default();
        converted.big_font_file = properties.font.big_font_name.clone().unwrap_or_default();
        converted.true_type_font = properties.font.family.clone().unwrap_or_default();
        converted.width_factor = properties.width_factor;
        converted.oblique_angle = properties.oblique_angle;
        converted.is_vertical = properties.vertical;
        converted.height = properties.creation_height.unwrap_or(0.0);
        if let Some(height) = properties.last_used_height {
            converted.last_height = height;
        }
        converted.flags.backward = properties.creation_backward;
        converted.flags.upside_down = properties.creation_upside_down;
        let handle = match document.text_styles.get_mut(&style.name) {
            Some(existing) => {
                converted.handle = existing.handle;
                *existing = converted;
                existing.handle
            }
            None => {
                converted.handle = document.allocate_handle();
                let handle = converted.handle;
                document.text_styles.add(converted)?;
                handle
            }
        };
        names.style_handles.insert(style.id, handle);
    }

    for pattern in &drawing.line_patterns {
        names.line_patterns.insert(pattern.id, pattern.name.clone());
        let mut converted = LineType::new(pattern.name.clone());
        converted.description = pattern.description.clone().unwrap_or_default();
        for element in &pattern.pattern {
            let complex = match &element.marker {
                None => None,
                Some(marker) => match marker_in(marker, names) {
                    Some(complex) => Some(complex),
                    // A pattern without one of its symbols is not that
                    // pattern; draw it continuous rather than wrong.
                    None => {
                        losses.note(format!("the pattern of line pattern {}", pattern.name));
                        converted.elements.clear();
                        break;
                    }
                },
            };
            converted.elements.push(LineTypeElement {
                length: element.length,
                complex,
            });
        }
        converted.pattern_length = converted.elements.iter().map(|e| e.length.abs()).sum();
        converted.handle = match document.line_types.get(&pattern.name) {
            Some(existing) => existing.handle,
            None => document.allocate_handle(),
        };
        document.line_types.add_or_replace(converted);
    }

    for layer in &drawing.layers {
        names.layers.insert(layer.id, layer.name.clone());
        let mut converted = document
            .layers
            .get(&layer.name)
            .cloned()
            .unwrap_or_else(|| Layer::new(layer.name.clone()));
        converted.description = layer.description.clone().unwrap_or_default();
        converted.color = color_in(&layer.appearance.color);
        converted.line_type = names
            .line_patterns
            .get(&layer.appearance.line_pattern)
            .cloned()
            .unwrap_or_else(|| "Continuous".to_string());
        converted.line_weight = weight_in(layer.appearance.line_weight);
        converted.transparency = transparency_in(layer.appearance.opacity);
        converted.flags.off = !layer.visible;
        converted.flags.frozen = layer.frozen;
        converted.flags.locked = layer.locked;
        converted.flags.frozen_in_new_viewport = layer.frozen_in_new_viewports;
        converted.is_plottable = layer.plottable;
        let handle = match document.layers.get_mut(&layer.name) {
            Some(existing) => {
                converted.handle = existing.handle;
                *existing = converted;
                existing.handle
            }
            None => {
                converted.handle = document.allocate_handle();
                let handle = converted.handle;
                document.layers.add(converted)?;
                handle
            }
        };
        names.layer_handles.insert(layer.id, handle);
    }

    let mut ucs_names = HashMap::new();
    for ucs in &drawing.ucs_definitions {
        ucs_names.insert(ucs.id, ucs.name.clone());
        let mut converted = Ucs::new(ucs.name.clone());
        converted.origin = vector(ucs.frame.origin().components());
        converted.x_axis = vector(ucs.frame.x_axis().components());
        converted.y_axis = vector(ucs.frame.y_axis().components());
        converted.elevation = ucs.elevation;
        converted.handle = match document.ucss.get(&ucs.name) {
            Some(existing) => existing.handle,
            None => document.allocate_handle(),
        };
        document.ucss.add_or_replace(converted);
    }
    let current = drawing
        .model_view_state
        .as_ref()
        .and_then(|state| state.current_model_ucs);
    let (name, frame) = match current {
        Some(IfccadUcsSelection::Named(id)) => (
            ucs_names.get(&id).cloned().unwrap_or_default(),
            drawing
                .ucs_definitions
                .iter()
                .find(|ucs| ucs.id == id)
                .map(|ucs| ucs.frame),
        ),
        Some(IfccadUcsSelection::Unnamed(frame)) => (String::new(), Some(frame)),
        Some(IfccadUcsSelection::World) | None => (String::new(), None),
    };
    if let Some(frame) = frame {
        let header = &mut document.header;
        header.model_space_ucs_name = name;
        header.model_space_ucs_origin = vector(frame.origin().components());
        header.model_space_ucs_x_axis = vector(frame.x_axis().components());
        header.model_space_ucs_y_axis = vector(frame.y_axis().components());
    }
    Ok(())
}

pub(super) fn write(writer: &mut Writer) -> Result<(), Error> {
    let document = writer.document;
    let header = &document.header;
    if header.linetype_scale.is_finite() && header.linetype_scale > 0.0 {
        writer.drawing.line_pattern_scale = header.linetype_scale;
    }
    match point_display_out(header.point_display_mode, header.point_display_size) {
        Some(display) => writer.drawing.point_display = Some(display),
        None => writer.losses.note("the point display mode"),
    }

    // SHX files complex linetypes draw their shapes from.
    for style in document.text_styles.iter().filter(|style| style.is_shape_file) {
        let file_name = style.font_file.rsplit(['/', '\\']).next().unwrap_or("");
        let source = SymbolSource::Shx {
            file_name: file_name.to_string(),
        };
        if ifccad::line_pattern::validate_symbol_source(&source).is_err() {
            continue;
        }
        let id = writer.drawing.id_counters.allocate_symbol_source_id()?;
        writer.symbols.insert(style.handle, id);
        writer.drawing.symbol_sources.push(IfccadSymbolSource { id, source });
    }

    for style in document.text_styles.iter() {
        if style.is_shape_file || style.name.is_empty() {
            continue;
        }
        let optional = |name: &String| (!name.is_empty()).then(|| name.clone());
        let mut font = FontRequest {
            family: optional(&style.true_type_font),
            cad_font_name: optional(&style.font_file),
            big_font_name: optional(&style.big_font_file),
            ..Default::default()
        };
        if font.cad_font_name.is_none() && (font.family.is_none() || font.big_font_name.is_some())
        {
            font.cad_font_name = Some("txt".to_string());
        }
        let mut properties = TextStyleProperties::new(font);
        if style.width_factor > 0.0 {
            properties.width_factor = style.width_factor;
        }
        properties.oblique_angle = style.oblique_angle;
        properties.vertical = style.is_vertical;
        properties.creation_height = (style.height > 0.0).then_some(style.height);
        properties.last_used_height = (style.last_height >= 0.0).then_some(style.last_height);
        properties.creation_backward = style.flags.backward;
        properties.creation_upside_down = style.flags.upside_down;
        let id = writer.drawing.id_counters.allocate_text_style_id()?;
        writer.styles.insert(style.name.to_ascii_lowercase(), id);
        writer.style_handles.insert(style.handle, id);
        writer.drawing.text_styles.push(IfccadTextStyle {
            id,
            name: style.name.clone(),
            properties,
        });
    }

    // The template's continuous pattern (id 0) stands for the document's.
    for linetype in document.line_types.iter() {
        let key = linetype.name.to_ascii_lowercase();
        if key == "bylayer" || key == "byblock" {
            continue;
        }
        let mut pattern = Vec::new();
        for element in &linetype.elements {
            let marker = match &element.complex {
                None => None,
                Some(complex) => match marker_out(complex, writer) {
                    Some(marker) => Some(marker),
                    None => {
                        pattern.clear();
                        break;
                    }
                },
            };
            pattern.push(LinePatternElement {
                length: element.length,
                marker,
            });
        }
        if pattern.len() != linetype.elements.len()
            || validate_line_pattern_values(&pattern).is_err()
        {
            writer
                .losses
                .note(format!("the pattern of linetype {}", linetype.name));
            pattern.clear();
        }
        let description = (!linetype.description.is_empty()).then(|| linetype.description.clone());
        if key == "continuous" && pattern.is_empty() {
            if let Some(template) = writer
                .drawing
                .line_patterns
                .iter_mut()
                .find(|pattern| pattern.id == IfccadLinePatternId(0))
            {
                template.name = linetype.name.clone();
                template.description = description;
            }
            writer.line_patterns.insert(key, IfccadLinePatternId(0));
            continue;
        }
        let id = writer.drawing.id_counters.allocate_line_pattern_id()?;
        writer.line_patterns.insert(key, id);
        writer.drawing.line_patterns.push(IfccadLinePattern {
            id,
            name: linetype.name.clone(),
            description,
            pattern,
        });
    }

    for layer in document.layers.iter() {
        let line_pattern = match writer.line_patterns.get(&layer.line_type.to_ascii_lowercase()) {
            Some(id) => *id,
            None => {
                writer.losses.note(format!("the linetype of layer {}", layer.name));
                IfccadLinePatternId(0)
            }
        };
        let id = writer.drawing.id_counters.allocate_layer_id()?;
        writer.layers.insert(layer.name.to_ascii_lowercase(), id);
        writer.drawing.layers.push(IfccadLayer {
            id,
            name: layer.name.clone(),
            description: (!layer.description.is_empty()).then(|| layer.description.clone()),
            visible: !layer.is_off(),
            frozen: layer.is_frozen(),
            locked: layer.is_locked(),
            plottable: layer.is_plottable,
            frozen_in_new_viewports: layer.flags.frozen_in_new_viewport,
            appearance: IfccadLayerAppearance {
                color: color_out(layer.color).unwrap_or(IfccadColor::Foreground),
                opacity: match layer.transparency {
                    Transparency::Explicit(amount) => 1.0 - f64::from(amount) / 255.0,
                    _ => 1.0,
                },
                line_pattern,
                line_weight: weight_out(layer.line_weight).unwrap_or(0.25),
            },
        });
    }

    for ucs in document.ucss.iter() {
        let Some(frame) = frame_out(ucs.origin, ucs.x_axis, ucs.y_axis) else {
            writer.losses.note(format!("UCS {}", ucs.name));
            continue;
        };
        let id = writer.drawing.id_counters.allocate_ucs_id()?;
        writer.ucs_ids.insert(ucs.name.to_ascii_lowercase(), id);
        writer.drawing.ucs_definitions.push(IfccadUcsDefinition {
            id,
            name: ucs.name.clone(),
            frame,
            elevation: ucs.elevation,
        });
    }
    let current = match writer.ucs_ids.get(&header.model_space_ucs_name.to_ascii_lowercase()) {
        Some(id) => Some(IfccadUcsSelection::Named(*id)),
        None => frame_out(
            header.model_space_ucs_origin,
            header.model_space_ucs_x_axis,
            header.model_space_ucs_y_axis,
        )
        .map(|frame| match frame == CoordinateFrame3::default() {
            true => IfccadUcsSelection::World,
            false => IfccadUcsSelection::Unnamed(frame),
        }),
    };
    if let Some(current) = current.filter(|ucs| *ucs != IfccadUcsSelection::World) {
        writer
            .drawing
            .model_view_state
            .get_or_insert_with(IfccadModelViewState::default)
            .current_model_ucs = Some(current);
    }
    Ok(())
}

fn marker_out(complex: &LineTypeComplexData, writer: &Writer) -> Option<IfccadLinePatternMarker> {
    let transform = LinePatternTransform {
        scale: complex.scale,
        offset: complex.offset,
        rotation: match (complex.upright, complex.absolute_rotation) {
            (true, _) => LinePatternRotation::Upright(complex.rotation),
            (false, true) => LinePatternRotation::Absolute(complex.rotation),
            (false, false) => LinePatternRotation::Relative(complex.rotation),
        },
    };
    validate_line_pattern_transform(&transform).ok()?;
    Some(match &complex.content {
        LineTypeComplexContent::Text { text } => IfccadLinePatternMarker::Text {
            text: text.clone(),
            text_style_id: *writer.style_handles.get(&complex.style_handle)?,
            transform,
        },
        LineTypeComplexContent::Shape { shape_number } => IfccadLinePatternMarker::Symbol {
            symbol_source_id: *writer.symbols.get(&complex.style_handle)?,
            shape_number: u32::try_from(*shape_number).ok()?,
            transform,
        },
    })
}

fn marker_in(marker: &IfccadLinePatternMarker, names: &Names) -> Option<LineTypeComplexData> {
    let (content, style_handle, transform) = match marker {
        IfccadLinePatternMarker::Text {
            text,
            text_style_id,
            transform,
        } => (
            LineTypeComplexContent::Text { text: text.clone() },
            *names.style_handles.get(text_style_id)?,
            transform,
        ),
        IfccadLinePatternMarker::Symbol {
            symbol_source_id,
            shape_number,
            transform,
        } => (
            LineTypeComplexContent::Shape {
                shape_number: i16::try_from(*shape_number).ok()?,
            },
            *names.symbols.get(symbol_source_id)?,
            transform,
        ),
    };
    let (rotation, absolute_rotation, upright) = match transform.rotation {
        LinePatternRotation::Relative(angle) => (angle, false, false),
        LinePatternRotation::Absolute(angle) => (angle, true, false),
        LinePatternRotation::Upright(angle) => (angle, false, true),
    };
    Some(LineTypeComplexData {
        content,
        style_handle,
        scale: transform.scale,
        rotation,
        absolute_rotation,
        upright,
        source_flags: 0,
        offset: transform.offset,
    })
}

/// A UCS frame, if its axes are the unit, perpendicular pair IFCX requires.
fn frame_out(origin: Vector3, x: Vector3, y: Vector3) -> Option<CoordinateFrame3> {
    let kernel = |v: Vector3| KernelVector::new(v.x, v.y, v.z);
    CoordinateFrame3::try_new(Point3::new(origin.x, origin.y, origin.z), kernel(x), kernel(y)).ok()
}

/// PDMODE and PDSIZE.
fn point_display_out(mode: i16, size: f64) -> Option<IfccadPointDisplay> {
    if mode < 0 || mode & !0x67 != 0 || !size.is_finite() {
        return None;
    }
    Some(IfccadPointDisplay {
        glyph: match mode & 7 {
            0 => IfccadPointGlyph::Dot,
            1 => IfccadPointGlyph::Hidden,
            2 => IfccadPointGlyph::Plus,
            3 => IfccadPointGlyph::Cross,
            4 => IfccadPointGlyph::ShortLine,
            _ => return None,
        },
        circle: mode & 32 != 0,
        square: mode & 64 != 0,
        size: match size {
            0.0 => IfccadPointSize::DefaultFivePercent,
            size if size > 0.0 => IfccadPointSize::Absolute(size),
            size => IfccadPointSize::ViewportPercent(-size),
        },
    })
}

fn point_display_in(display: IfccadPointDisplay) -> (i16, f64) {
    let glyph = match display.glyph {
        IfccadPointGlyph::Dot => 0,
        IfccadPointGlyph::Hidden => 1,
        IfccadPointGlyph::Plus => 2,
        IfccadPointGlyph::Cross => 3,
        IfccadPointGlyph::ShortLine => 4,
    };
    let mode = glyph + i16::from(display.circle) * 32 + i16::from(display.square) * 64;
    let size = match display.size {
        IfccadPointSize::DefaultFivePercent => 0.0,
        IfccadPointSize::Absolute(size) => size,
        IfccadPointSize::ViewportPercent(percent) => -percent,
    };
    (mode, size)
}
