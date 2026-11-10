use super::*;
use acadrust::{
    Color, ProxyGraphicCommand as Cad, ProxyLegacyText, ProxyLegacyTextStyle,
    ProxyStyledUnicodeText, ProxyUnicodeText, Vector3,
};
use IfccadLeaderGraphicArcClosure as Closure;
use IfccadLeaderGraphicCommand as Native;
use IfccadLeaderGraphicInheritance as Inheritance;
use IfccadLeaderGraphicLineWeight as Weight;
use IfccadLeaderGraphicPrimitive as Primitive;
use IfccadLeaderGraphicTextProcessing as Processing;

fn array(value: Vector3) -> [f64; 3] {
    [value.x, value.y, value.z]
}
fn vector(value: [f64; 3]) -> Vector3 {
    Vector3::new(value[0], value[1], value[2])
}
fn transpose(value: [f64; 16]) -> [f64; 16] {
    std::array::from_fn(|index| value[(index % 4) * 4 + index / 4])
}
fn closure(value: i32) -> Result<Closure, Error> {
    match value {
        0 => Ok(Closure::Simple),
        1 => Ok(Closure::Sector),
        2 => Ok(Closure::Chord),
        _ => Err(format!("unsupported arc closure {value}").into()),
    }
}
fn closure_code(value: Closure) -> i32 {
    match value {
        Closure::Simple => 0,
        Closure::Sector => 1,
        Closure::Chord => 2,
    }
}

fn index_color(index: u32) -> Result<Inheritance<IfccadColor>, Error> {
    match index {
        0 => Ok(Inheritance::ByBlock),
        256 => Ok(Inheritance::ByLayer),
        1..=255 => Ok(Inheritance::Explicit(
            super::super::color_out(Color::Index(index as u8)).ok_or("unmapped ACI color")?,
        )),
        _ => Err(format!("unsupported graphics color index {index}").into()),
    }
}
fn encoded_color(value: u32) -> Result<Inheritance<IfccadColor>, Error> {
    let payload = value & 0x00ff_ffff;
    match value >> 24 {
        0xc0 if payload == 0 => Ok(Inheritance::ByLayer),
        0xc1 if payload == 0 => Ok(Inheritance::ByBlock),
        0xc2 => Ok(Inheritance::Explicit(IfccadColor::rgb(
            (payload >> 16) as u8,
            (payload >> 8) as u8,
            payload as u8,
        ))),
        0xc3 => index_color(payload),
        0xc5 if payload == 0 => Ok(Inheritance::Explicit(IfccadColor::Foreground)),
        _ => Err(format!("unsupported encoded graphics color {value:#x}").into()),
    }
}
fn color_code(value: &Inheritance<IfccadColor>) -> Result<u32, Error> {
    match value {
        Inheritance::Owner => {
            Err("authored owner-color reset has no qualified CAD encoding".into())
        }
        Inheritance::ByLayer => Ok(0xc000_0000),
        Inheritance::ByBlock => Ok(0xc100_0000),
        Inheritance::Explicit(IfccadColor::Foreground) => Ok(0xc300_0007),
        Inheritance::Explicit(IfccadColor::Concrete {
            rgb,
            indexed,
            named,
        }) => {
            if named.is_some() {
                return Err("named graphics color cannot be reduced to RGB".into());
            }
            if let Some((system, index)) = indexed {
                if system.eq_ignore_ascii_case("ACI")
                    && (1..=255).contains(index)
                    && Color::Index(*index as u8).rgb() == Some((rgb[0], rgb[1], rgb[2]))
                {
                    return Ok(0xc300_0000 | *index as u32);
                }
                return Err("graphics color index identity has no exact CAD encoding".into());
            }
            Ok(0xc200_0000 | u32::from(rgb[0]) << 16 | u32::from(rgb[1]) << 8 | u32::from(rgb[2]))
        }
    }
}
fn weight(value: i32) -> Result<Weight, Error> {
    match value {
        -1 => Ok(Weight::ByLayer),
        -2 => Ok(Weight::ByBlock),
        -3 => Ok(Weight::DrawingDefault),
        0.. => Ok(Weight::Explicit(f64::from(value) / 100.)),
        _ => Err(format!("unsupported graphics lineweight {value}").into()),
    }
}
fn weight_code(value: Weight) -> Result<i32, Error> {
    match value {
        Weight::Owner => {
            Err("authored owner-lineweight reset has no qualified CAD encoding".into())
        }
        Weight::ByLayer => Ok(-1),
        Weight::ByBlock => Ok(-2),
        Weight::DrawingDefault => Ok(-3),
        Weight::Explicit(value) => {
            let integer = (value * 100.).round();
            if integer.is_finite()
                && integer >= 0.
                && integer <= f64::from(i32::MAX)
                && integer / 100. == value
            {
                Ok(integer as i32)
            } else {
                Err("graphics lineweight is not exactly representable in hundredths of a millimetre".into())
            }
        }
    }
}

fn decode_legacy(bytes: &[u8], bindings: &LeaderGraphicBindings<'_>) -> Result<String, Error> {
    let encoding = bindings
        .encoding
        .ok_or("graphics text requires an explicit faithful legacy encoding")?;
    let (text, errors) = encoding.decode_without_bom_handling(bytes);
    if errors {
        Err("invalid byte sequence in declared graphics text codepage".into())
    } else {
        Ok(text.into_owned())
    }
}
fn encode_legacy(text: &str, bindings: &LeaderGraphicBindings<'_>) -> Result<Vec<u8>, Error> {
    let encoding = bindings
        .encoding
        .ok_or("graphics text requires an explicit faithful legacy encoding")?;
    let (bytes, _, errors) = encoding.encode(text);
    let (read, read_errors) = encoding.decode_without_bom_handling(&bytes);
    if errors || read_errors || read != text {
        return Err("graphics text is not exact in the target codepage".into());
    }
    Ok(bytes.into_owned())
}
fn text_base(
    position: Vector3,
    normal: Vector3,
    direction: Vector3,
    text: String,
    height: f64,
    width: f64,
    oblique: f64,
) -> IfccadLeaderGraphicText {
    IfccadLeaderGraphicText {
        position: array(position),
        normal: array(normal),
        direction: array(direction),
        text,
        processing: Processing::Interpreted,
        height,
        width_factor: width,
        oblique_angle: oblique,
        tracking: 1.,
        backwards: false,
        upside_down: false,
        vertical: false,
        underline: false,
        overline: false,
        font: None,
    }
}
fn set_flags(text: &mut IfccadLeaderGraphicText, flags: [bool; 5]) {
    [
        text.backwards,
        text.upside_down,
        text.vertical,
        text.underline,
        text.overline,
    ] = flags;
}
fn flags(text: &IfccadLeaderGraphicText) -> [bool; 5] {
    [
        text.backwards,
        text.upside_down,
        text.vertical,
        text.underline,
        text.overline,
    ]
}
fn processing(value: i32) -> Result<Processing, Error> {
    match value {
        0 => Ok(Processing::Interpreted),
        1 => Ok(Processing::Raw),
        _ => Err("unsupported graphics text processing mode".into()),
    }
}
fn processing_code(value: Processing) -> i32 {
    match value {
        Processing::Interpreted => 0,
        Processing::Raw => 1,
    }
}

pub(super) fn decode(
    record: CheckedProxyGraphicRecord,
    bindings: &LeaderGraphicBindings<'_>,
) -> Result<Native, Error> {
    let command = match record {
        CheckedProxyGraphicRecord::Unknown { record_type, .. } => {
            return Err(format!(
                "unsupported proxy record {record_type}; complete snapshot unavailable"
            )
            .into())
        }
        CheckedProxyGraphicRecord::FillOff => {
            return Err(
                "proxy record 21 has unqualified semantics; complete snapshot unavailable".into(),
            )
        }
        CheckedProxyGraphicRecord::UnicodeText(t) => {
            return Ok(Native::Draw(Primitive::Text(Box::new(text_base(
                t.position,
                t.normal,
                t.direction,
                t.text,
                t.height,
                t.width_factor,
                t.oblique_angle,
            )))))
        }
        CheckedProxyGraphicRecord::Command(command) => command,
    };
    Ok(match command {
        Cad::Extents([min, max]) => Native::StoredExtents {
            min: array(min),
            max: array(max),
        },
        Cad::Polyline(vertices) => Native::Draw(Primitive::Polyline {
            vertices: vertices.into_iter().map(array).collect(),
            normal: None,
        }),
        Cad::Polygon(vertices) => Native::Draw(Primitive::Polygon {
            vertices: vertices.into_iter().map(array).collect(),
        }),
        Cad::PolylineWithNormal { vertices, normal } => Native::Draw(Primitive::Polyline {
            vertices: vertices.into_iter().map(array).collect(),
            normal: Some(array(normal)),
        }),
        Cad::Circle {
            center,
            radius,
            normal,
        } => Native::Draw(Primitive::Circle {
            center: array(center),
            radius,
            normal: array(normal),
        }),
        Cad::Circle3(points) => Native::Draw(Primitive::Circle3 {
            points: points.map(array),
        }),
        Cad::CircularArc {
            center,
            radius,
            normal,
            start_direction,
            sweep_angle,
            arc_type,
        } => Native::Draw(Primitive::Arc {
            center: array(center),
            radius,
            normal: array(normal),
            start_direction: array(start_direction),
            sweep_angle,
            closure: closure(arc_type)?,
        }),
        Cad::CircularArc3 { points, arc_type } => Native::Draw(Primitive::Arc3 {
            points: points.map(array),
            closure: closure(arc_type)?,
        }),
        Cad::ColorIndex(value) => Native::SetColor(index_color(value)?),
        Cad::EncodedColor(value) => Native::SetColor(encoded_color(value)?),
        Cad::LayerIndex(index) => Native::SetLayer(Some(
            *bindings
                .layers
                .get(&index)
                .ok_or_else(|| format!("unbound source layer index {index}"))?,
        )),
        Cad::LineTypeIndex(index) => Native::SetLinePattern(match index {
            32766 => Inheritance::ByBlock,
            32767 => Inheritance::ByLayer,
            _ => Inheritance::Explicit(
                *bindings
                    .line_patterns
                    .get(&index)
                    .ok_or_else(|| format!("unbound source linetype index {index}"))?,
            ),
        }),
        Cad::SelectionMarker(value) => Native::SetSelectionMarker(value),
        Cad::Fill(value) => Native::SetFill(value),
        Cad::LineWeight(value) => Native::SetLineWeight(weight(value)?),
        Cad::LineTypeScale(value) => Native::SetLinePatternScale(value),
        Cad::Thickness(value) => Native::SetThickness(value),
        Cad::PushTransform(value) => Native::PushTransform(transpose(value)),
        Cad::PopTransform => Native::PopTransform,
        Cad::Compatibility51Zero => Native::CadCompatibility51Zero,
        Cad::StyledUnicodeText(t) => {
            let t = *t;
            let mut text = text_base(
                t.position,
                t.normal,
                t.direction,
                t.text,
                t.height,
                t.width_factor,
                t.oblique_angle,
            );
            text.processing = processing(t.processing_mode)?;
            text.tracking = t.tracking;
            set_flags(&mut text, t.text_flags);
            text.font = Some(IfccadLeaderGraphicFont {
                family: t.family,
                font_file: t.font_file,
                big_font_file: t.big_font_file,
                bold: Some(t.bold),
                italic: Some(t.italic),
                charset: Some(t.charset),
                pitch_family: Some(t.pitch_family),
            });
            Native::Draw(Primitive::Text(Box::new(text)))
        }
        Cad::LegacyText(t) => {
            let t = *t;
            let mut text = text_base(
                t.position,
                t.normal,
                t.direction,
                decode_legacy(&t.text, bindings)?,
                t.height,
                t.width_factor,
                t.oblique_angle,
            );
            if let Some(style) = t.styling {
                text.processing = processing(style.processing_mode)?;
                text.tracking = style.tracking;
                set_flags(&mut text, style.text_flags);
                text.font = Some(IfccadLeaderGraphicFont {
                    family: String::new(),
                    font_file: decode_legacy(&style.font_file, bindings)?,
                    big_font_file: decode_legacy(&style.big_font_file, bindings)?,
                    bold: None,
                    italic: None,
                    charset: None,
                    pitch_family: None,
                });
            }
            Native::Draw(Primitive::Text(Box::new(text)))
        }
    })
}

fn encode_text(
    text: &IfccadLeaderGraphicText,
    bindings: &LeaderGraphicBindings<'_>,
) -> Result<CheckedProxyGraphicRecord, Error> {
    let position = vector(text.position);
    let normal = vector(text.normal);
    let direction = vector(text.direction);
    let Some(font) = &text.font else {
        if text.processing != Processing::Interpreted
            || flags(text).iter().any(|flag| *flag)
            || text.tracking != 1.
        {
            return Err(
                "styled text without an explicit font request has no qualified CAD representation"
                    .into(),
            );
        }
        return Ok(CheckedProxyGraphicRecord::UnicodeText(ProxyUnicodeText {
            position,
            normal,
            direction,
            text: text.text.clone(),
            height: text.height,
            width_factor: text.width_factor,
            oblique_angle: text.oblique_angle,
        }));
    };
    match (font.bold, font.italic, font.charset, font.pitch_family) {
        (Some(bold), Some(italic), Some(charset), Some(pitch_family)) => {
            Ok(CheckedProxyGraphicRecord::Command(Cad::StyledUnicodeText(
                Box::new(ProxyStyledUnicodeText {
                    position,
                    normal,
                    direction,
                    text: text.text.clone(),
                    declared_length: i32::try_from(text.text.encode_utf16().count())?,
                    processing_mode: processing_code(text.processing),
                    height: text.height,
                    width_factor: text.width_factor,
                    oblique_angle: text.oblique_angle,
                    tracking: text.tracking,
                    text_flags: flags(text),
                    bold,
                    italic,
                    charset,
                    pitch_family,
                    family: font.family.clone(),
                    font_file: font.font_file.clone(),
                    big_font_file: font.big_font_file.clone(),
                }),
            )))
        }
        (None, None, None, None) if font.family.is_empty() => Ok(
            CheckedProxyGraphicRecord::Command(Cad::LegacyText(Box::new(ProxyLegacyText {
                position,
                normal,
                direction,
                text: encode_legacy(&text.text, bindings)?,
                height: text.height,
                width_factor: text.width_factor,
                oblique_angle: text.oblique_angle,
                styling: Some(ProxyLegacyTextStyle {
                    declared_length: -1,
                    processing_mode: processing_code(text.processing),
                    tracking: text.tracking,
                    text_flags: flags(text),
                    font_file: encode_legacy(&font.font_file, bindings)?,
                    big_font_file: encode_legacy(&font.big_font_file, bindings)?,
                }),
            }))),
        ),
        _ => Err("partial graphics font descriptor has no exact CAD representation".into()),
    }
}

pub(super) fn encode(
    command: &Native,
    bindings: &LeaderGraphicBindings<'_>,
) -> Result<CheckedProxyGraphicRecord, Error> {
    let value = match command {
        Native::StoredExtents { min, max } => Cad::Extents([vector(*min), vector(*max)]),
        Native::SetColor(value) => Cad::EncodedColor(color_code(value)?),
        Native::SetLayer(Some(id)) => Cad::LayerIndex(
            *bindings
                .layer_indices
                .get(id)
                .ok_or("native layer has no CAD storage index")?,
        ),
        Native::SetLayer(None) => {
            return Err("authored owner-layer reset has no qualified CAD encoding".into())
        }
        Native::SetLinePattern(value) => Cad::LineTypeIndex(match value {
            Inheritance::Owner => {
                return Err("authored owner-linetype reset has no qualified CAD encoding".into())
            }
            Inheritance::ByLayer => 32767,
            Inheritance::ByBlock => 32766,
            Inheritance::Explicit(id) => *bindings
                .line_pattern_indices
                .get(id)
                .ok_or("native line pattern has no CAD storage index")?,
        }),
        Native::SetSelectionMarker(value) => Cad::SelectionMarker(*value),
        Native::SetFill(value) => Cad::Fill(*value),
        Native::SetLineWeight(value) => Cad::LineWeight(weight_code(*value)?),
        Native::SetLinePatternScale(value) => Cad::LineTypeScale(*value),
        Native::SetThickness(value) => Cad::Thickness(*value),
        Native::PushTransform(value) => Cad::PushTransform(transpose(*value)),
        Native::PopTransform => Cad::PopTransform,
        Native::CadCompatibility51Zero => Cad::Compatibility51Zero,
        Native::Draw(value) => match value {
            Primitive::Point(_) => {
                return Err("native point picture has no qualified CAD point opcode".into())
            }
            Primitive::Polyline {
                vertices,
                normal: None,
            } => Cad::Polyline(vertices.iter().copied().map(vector).collect()),
            Primitive::Polyline {
                vertices,
                normal: Some(normal),
            } => Cad::PolylineWithNormal {
                vertices: vertices.iter().copied().map(vector).collect(),
                normal: vector(*normal),
            },
            Primitive::Polygon { vertices } => {
                Cad::Polygon(vertices.iter().copied().map(vector).collect())
            }
            Primitive::Circle {
                center,
                radius,
                normal,
            } => Cad::Circle {
                center: vector(*center),
                radius: *radius,
                normal: vector(*normal),
            },
            Primitive::Circle3 { points } => Cad::Circle3(points.map(vector)),
            Primitive::Arc {
                center,
                radius,
                normal,
                start_direction,
                sweep_angle,
                closure,
            } => Cad::CircularArc {
                center: vector(*center),
                radius: *radius,
                normal: vector(*normal),
                start_direction: vector(*start_direction),
                sweep_angle: *sweep_angle,
                arc_type: closure_code(*closure),
            },
            Primitive::Arc3 { points, closure } => Cad::CircularArc3 {
                points: points.map(vector),
                arc_type: closure_code(*closure),
            },
            Primitive::Text(text) => return encode_text(text, bindings),
        },
    };
    Ok(CheckedProxyGraphicRecord::Command(value))
}
