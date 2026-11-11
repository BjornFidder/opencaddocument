use super::*;
use crate::{
    entities::{
        EntityCommon, Line, MLine, MLineFlags, MLineJustification, MLineSegment, MLineVertex,
    },
    objects::{MLineStyle, MLineStyleElement, MLineStyleFlags},
    tables::{Layer, LayerFlags, LineType, LineTypeElement},
    types::{Color, LineWeight, Transparency, Vector3},
};

fn invalid(message: &str) -> super::super::SelectionStorageError {
    error(Kind::InvalidSelection, message)
}

fn vector_bits(v: Vector3) -> [u64; 3] {
    [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]
}

fn vector_from_bits([x, y, z]: [u64; 3]) -> Vector3 {
    Vector3::new(f64::from_bits(x), f64::from_bits(y), f64::from_bits(z))
}

fn common_flags(material: u8, plotstyle: u8, mode: Option<u8>) -> Result<()> {
    if material > 1 || plotstyle > 1 || mode.is_some_and(|v| v != 0 && v != 2) {
        return Err(invalid("unsupported entity resource flags or mode"));
    }
    Ok(())
}

text_enum!(Justification(MLineJustification) {
    "top" => Top, "zero" => Zero, "bottom" => Bottom,
});
tagged!(WireColor {
    "by_layer" => ByLayer(), "none" => None(), "by_block" => ByBlock(),
    "index" => Index(value: u8), "rgb" => Rgb(r: u8, g: u8, b: u8),
});
tagged!(WireWeight {
    "by_layer" => ByLayer(), "by_block" => ByBlock(), "default" => Default(),
    "value" => Value(value: i16),
});
tagged!(WireTransparency {
    "by_layer" => ByLayer(), "by_block" => ByBlock(), "explicit" => Explicit(value: u8),
});
impl From<Color> for WireColor {
    fn from(value: Color) -> Self {
        match value {
            Color::ByLayer => Self::ByLayer(),
            Color::None => Self::None(),
            Color::ByBlock => Self::ByBlock(),
            Color::Index(v) => Self::Index(v),
            Color::Rgb { r, g, b } => Self::Rgb(r, g, b),
        }
    }
}
impl From<WireColor> for Color {
    fn from(value: WireColor) -> Self {
        match value {
            WireColor::ByLayer() => Self::ByLayer,
            WireColor::None() => Self::None,
            WireColor::ByBlock() => Self::ByBlock,
            WireColor::Index(v) => Self::Index(v),
            WireColor::Rgb(r, g, b) => Self::Rgb { r, g, b },
        }
    }
}
impl From<LineWeight> for WireWeight {
    fn from(value: LineWeight) -> Self {
        match value {
            LineWeight::ByLayer => Self::ByLayer(),
            LineWeight::ByBlock => Self::ByBlock(),
            LineWeight::Default => Self::Default(),
            LineWeight::Value(v) => Self::Value(v),
        }
    }
}
impl From<WireWeight> for LineWeight {
    fn from(value: WireWeight) -> Self {
        match value {
            WireWeight::ByLayer() => Self::ByLayer,
            WireWeight::ByBlock() => Self::ByBlock,
            WireWeight::Default() => Self::Default,
            WireWeight::Value(v) => Self::Value(v),
        }
    }
}
impl From<Transparency> for WireTransparency {
    fn from(value: Transparency) -> Self {
        match value {
            Transparency::ByLayer => Self::ByLayer(),
            Transparency::ByBlock => Self::ByBlock(),
            Transparency::Explicit(v) => Self::Explicit(v),
        }
    }
}
impl From<WireTransparency> for Transparency {
    fn from(value: WireTransparency) -> Self {
        match value {
            WireTransparency::ByLayer() => Self::ByLayer,
            WireTransparency::ByBlock() => Self::ByBlock,
            WireTransparency::Explicit(v) => Self::Explicit(v),
        }
    }
}
named_map!(Common {
    handle: u64,
    layer: String,
    layer_handle: Option<u64>,
    color: WireColor,
    line_weight: WireWeight,
    linetype: String,
    linetype_handle: Option<u64>,
    linetype_scale: u64,
    transparency: WireTransparency,
    color_name: Option<String>,
    invisible: bool,
    owner_handle: u64,
    material_flags: u8,
    shadow_flags: u8,
    plotstyle_flags: u8,
    entity_mode: Option<u8>,
});
impl Common {
    fn from_model(p: &mut Projection, value: &EntityCommon) -> Result<Self> {
        let EntityCommon {
            handle,
            layer,
            layer_handle,
            color,
            line_weight,
            linetype,
            linetype_handle,
            linetype_scale,
            transparency,
            color_name,
            invisible,
            owner_handle,
            material_flags,
            shadow_flags,
            plotstyle_flags,
            entity_mode,
            extended_data,
            graphic_data,
            reactors,
            xdictionary_handle,
            color_book_handle,
            full_visual_style_handle,
            face_visual_style_handle,
            edge_visual_style_handle,
            material_handle,
            plotstyle_handle,
            r12_metadata,
            has_ds_data,
            raw_record,
        } = value;
        common_flags(*material_flags, *plotstyle_flags, *entity_mode)?;
        if !extended_data.records().is_empty()
            || !extended_data.raw_dwg_eed.is_empty()
            || !reactors.is_empty()
            || *has_ds_data
            || [
                *xdictionary_handle,
                *color_book_handle,
                *full_visual_style_handle,
                *face_visual_style_handle,
                *edge_visual_style_handle,
                *material_handle,
                *plotstyle_handle,
            ]
            .iter()
            .any(Option::is_some)
            || graphic_data.is_some()
            || r12_metadata.is_some()
            || raw_record.is_some()
        {
            return Err(invalid("unsupported common entity content"));
        }
        Ok(Self {
            handle: handle.value(),
            layer: p.text(layer)?,
            layer_handle: layer_handle.map(|h| h.value()),
            color: (*color).into(),
            line_weight: (*line_weight).into(),
            linetype: p.text(linetype)?,
            linetype_handle: linetype_handle.map(|h| h.value()),
            linetype_scale: linetype_scale.to_bits(),
            transparency: (*transparency).into(),
            color_name: p.optional_text(color_name)?,
            invisible: *invisible,
            owner_handle: owner_handle.value(),
            material_flags: *material_flags,
            shadow_flags: *shadow_flags,
            plotstyle_flags: *plotstyle_flags,
            entity_mode: *entity_mode,
        })
    }
    fn into_model(self) -> Result<EntityCommon> {
        let Self {
            handle,
            layer,
            layer_handle,
            color,
            line_weight,
            linetype,
            linetype_handle,
            linetype_scale,
            transparency,
            color_name,
            invisible,
            owner_handle,
            material_flags,
            shadow_flags,
            plotstyle_flags,
            entity_mode,
        } = self;
        common_flags(material_flags, plotstyle_flags, entity_mode)?;
        Ok(EntityCommon {
            handle: Handle::new(handle),
            layer,
            layer_handle: layer_handle.map(Handle::new),
            color: color.into(),
            line_weight: line_weight.into(),
            linetype,
            linetype_handle: linetype_handle.map(Handle::new),
            linetype_scale: f64::from_bits(linetype_scale),
            transparency: transparency.into(),
            color_name,
            invisible,
            owner_handle: Handle::new(owner_handle),
            material_flags,
            shadow_flags,
            plotstyle_flags,
            entity_mode,
            extended_data: Default::default(),
            graphic_data: None,
            reactors: Vec::new(),
            xdictionary_handle: None,
            color_book_handle: None,
            full_visual_style_handle: None,
            face_visual_style_handle: None,
            edge_visual_style_handle: None,
            material_handle: None,
            plotstyle_handle: None,
            r12_metadata: None,
            has_ds_data: false,
            raw_record: None,
        })
    }
}
named_map!(pub(super) LineData {
    common: Common,
    start: [u64; 3],
    end: [u64; 3],
    thickness: u64,
    normal: [u64; 3],
});
impl LineData {
    fn from_model(p: &mut Projection, value: &Line) -> Result<Self> {
        let Line {
            common,
            start,
            end,
            thickness,
            normal,
        } = value;
        Ok(Self {
            common: Common::from_model(p, common)?,
            start: vector_bits(*start),
            end: vector_bits(*end),
            thickness: thickness.to_bits(),
            normal: vector_bits(*normal),
        })
    }
    fn into_model(self) -> Result<Line> {
        let Self {
            common,
            start,
            end,
            thickness,
            normal,
        } = self;
        Ok(Line {
            common: common.into_model()?,
            start: vector_from_bits(start),
            end: vector_from_bits(end),
            thickness: f64::from_bits(thickness),
            normal: vector_from_bits(normal),
        })
    }
}
named_map!(Segment {
    parameters: Vec<u64>,
    area_fill_parameters: Vec<u64>,
});
impl Segment {
    fn from_model(p: &mut Projection, value: &MLineSegment) -> Result<Self> {
        let MLineSegment {
            parameters,
            area_fill_parameters,
        } = value;
        Ok(Self {
            parameters: p.list(parameters, |_, x| Ok(x.to_bits()))?,
            area_fill_parameters: p.list(area_fill_parameters, |_, x| Ok(x.to_bits()))?,
        })
    }
    fn into_model(self) -> Result<MLineSegment> {
        let Self {
            parameters,
            area_fill_parameters,
        } = self;
        Ok(MLineSegment {
            parameters: parameters.into_iter().map(f64::from_bits).collect(),
            area_fill_parameters: area_fill_parameters
                .into_iter()
                .map(f64::from_bits)
                .collect(),
        })
    }
}
named_map!(Vertex {
    position: [u64; 3],
    direction: [u64; 3],
    miter: [u64; 3],
    segments: Vec<Segment>,
});
impl Vertex {
    fn from_model(p: &mut Projection, value: &MLineVertex) -> Result<Self> {
        let MLineVertex {
            position,
            direction,
            miter,
            segments,
        } = value;
        Ok(Self {
            position: vector_bits(*position),
            direction: vector_bits(*direction),
            miter: vector_bits(*miter),
            segments: p.list(segments, Segment::from_model)?,
        })
    }
    fn into_model(self) -> Result<MLineVertex> {
        let Self {
            position,
            direction,
            miter,
            segments,
        } = self;
        Ok(MLineVertex {
            position: vector_from_bits(position),
            direction: vector_from_bits(direction),
            miter: vector_from_bits(miter),
            segments: segments
                .into_iter()
                .map(Segment::into_model)
                .collect::<Result<_>>()?,
        })
    }
}
named_map!(pub(super) MLineData {
    common: Common,
    flags: i16,
    justification: Justification,
    normal: [u64; 3],
    scale_factor: u64,
    start_point: [u64; 3],
    style_handle: Option<u64>,
    style_name: String,
    style_element_count: u32,
    vertices: Vec<Vertex>,
});
impl MLineData {
    fn from_model(p: &mut Projection, value: &MLine) -> Result<Self> {
        let MLine {
            common,
            flags,
            justification,
            normal,
            scale_factor,
            start_point,
            style_handle,
            style_name,
            style_element_count,
            vertices,
        } = value;
        Ok(Self {
            common: Common::from_model(p, common)?,
            flags: flags.bits(),
            justification: Justification(*justification),
            normal: vector_bits(*normal),
            scale_factor: scale_factor.to_bits(),
            start_point: vector_bits(*start_point),
            style_handle: style_handle.map(|h| h.value()),
            style_name: p.text(style_name)?,
            style_element_count: count32(*style_element_count)?,
            vertices: p.list(vertices, Vertex::from_model)?,
        })
    }
    fn into_model(self) -> Result<MLine> {
        let Self {
            common,
            flags,
            justification,
            normal,
            scale_factor,
            start_point,
            style_handle,
            style_name,
            style_element_count,
            vertices,
        } = self;
        Ok(MLine {
            common: common.into_model()?,
            flags: MLineFlags::from_bits_retain(flags),
            justification: justification.0,
            normal: vector_from_bits(normal),
            scale_factor: f64::from_bits(scale_factor),
            start_point: vector_from_bits(start_point),
            style_handle: style_handle.map(Handle::new),
            style_name,
            style_element_count: style_element_count as usize,
            vertices: vertices
                .into_iter()
                .map(Vertex::into_model)
                .collect::<Result<_>>()?,
        })
    }
}
named_map!(StyleFlags {
    fill_on: bool,
    display_joints: bool,
    start_square_cap: bool,
    start_inner_arcs_cap: bool,
    start_round_cap: bool,
    end_square_cap: bool,
    end_inner_arcs_cap: bool,
    end_round_cap: bool,
});
impl StyleFlags {
    fn from_model(_p: &mut Projection, value: &MLineStyleFlags) -> Result<Self> {
        let MLineStyleFlags {
            fill_on,
            display_joints,
            start_square_cap,
            start_inner_arcs_cap,
            start_round_cap,
            end_square_cap,
            end_inner_arcs_cap,
            end_round_cap,
        } = value;
        Ok(Self {
            fill_on: *fill_on,
            display_joints: *display_joints,
            start_square_cap: *start_square_cap,
            start_inner_arcs_cap: *start_inner_arcs_cap,
            start_round_cap: *start_round_cap,
            end_square_cap: *end_square_cap,
            end_inner_arcs_cap: *end_inner_arcs_cap,
            end_round_cap: *end_round_cap,
        })
    }
    fn into_model(self) -> Result<MLineStyleFlags> {
        let Self {
            fill_on,
            display_joints,
            start_square_cap,
            start_inner_arcs_cap,
            start_round_cap,
            end_square_cap,
            end_inner_arcs_cap,
            end_round_cap,
        } = self;
        Ok(MLineStyleFlags {
            fill_on,
            display_joints,
            start_square_cap,
            start_inner_arcs_cap,
            start_round_cap,
            end_square_cap,
            end_inner_arcs_cap,
            end_round_cap,
        })
    }
}
named_map!(StyleElement {
    offset: u64,
    color: WireColor,
    linetype: String,
});
impl StyleElement {
    fn from_model(p: &mut Projection, value: &MLineStyleElement) -> Result<Self> {
        let MLineStyleElement {
            offset,
            color,
            linetype,
        } = value;
        Ok(Self {
            offset: offset.to_bits(),
            color: (*color).into(),
            linetype: p.text(linetype)?,
        })
    }
    fn into_model(self) -> Result<MLineStyleElement> {
        let Self {
            offset,
            color,
            linetype,
        } = self;
        Ok(MLineStyleElement {
            offset: f64::from_bits(offset),
            color: color.into(),
            linetype,
        })
    }
}
named_map!(pub(super) StyleData {
    handle: u64,
    owner: u64,
    name: String,
    description: String,
    elements: Vec<StyleElement>,
    start_angle: u64,
    end_angle: u64,
    fill_color: WireColor,
    flags: StyleFlags,
});
impl StyleData {
    fn from_model(p: &mut Projection, value: &MLineStyle) -> Result<Self> {
        let MLineStyle {
            handle,
            owner,
            name,
            description,
            elements,
            start_angle,
            end_angle,
            fill_color,
            flags,
        } = value;
        Ok(Self {
            handle: handle.value(),
            owner: owner.value(),
            name: p.text(name)?,
            description: p.text(description)?,
            elements: p.list(elements, StyleElement::from_model)?,
            start_angle: start_angle.to_bits(),
            end_angle: end_angle.to_bits(),
            fill_color: (*fill_color).into(),
            flags: StyleFlags::from_model(p, flags)?,
        })
    }
    fn into_model(self) -> Result<MLineStyle> {
        let Self {
            handle,
            owner,
            name,
            description,
            elements,
            start_angle,
            end_angle,
            fill_color,
            flags,
        } = self;
        Ok(MLineStyle {
            handle: Handle::new(handle),
            owner: Handle::new(owner),
            name,
            description,
            elements: elements
                .into_iter()
                .map(StyleElement::into_model)
                .collect::<Result<_>>()?,
            start_angle: f64::from_bits(start_angle),
            end_angle: f64::from_bits(end_angle),
            fill_color: fill_color.into(),
            flags: flags.into_model()?,
        })
    }
}
named_map!(LayerFlagsData {
    frozen: bool,
    locked: bool,
    frozen_in_new_viewport: bool,
    off: bool,
});
impl LayerFlagsData {
    fn from_model(_p: &mut Projection, value: &LayerFlags) -> Result<Self> {
        let LayerFlags {
            frozen,
            locked,
            frozen_in_new_viewport,
            off,
            xref_dependent,
        } = value;
        if *xref_dependent {
            return Err(invalid("external reference layer flag"));
        }
        Ok(Self {
            frozen: *frozen,
            locked: *locked,
            frozen_in_new_viewport: *frozen_in_new_viewport,
            off: *off,
        })
    }
    fn into_model(self) -> Result<LayerFlags> {
        let Self {
            frozen,
            locked,
            frozen_in_new_viewport,
            off,
        } = self;
        Ok(LayerFlags {
            frozen,
            locked,
            frozen_in_new_viewport,
            off,
            xref_dependent: false,
        })
    }
}
named_map!(pub(super) LayerData {
    handle: u64,
    name: String,
    flags: LayerFlagsData,
    color: WireColor,
    color_name: Option<String>,
    book_name: Option<String>,
    line_type: String,
    line_weight: WireWeight,
    is_plottable: bool,
    transparency: WireTransparency,
    description: String,
});
impl LayerData {
    fn from_model(p: &mut Projection, value: &Layer) -> Result<Self> {
        let Layer {
            handle,
            name,
            flags,
            color,
            color_name,
            book_name,
            line_type,
            line_weight,
            is_plottable,
            transparency,
            description,
            plot_style,
            material,
            plotstyle_handle,
            xref_block_record_handle,
        } = value;
        if name.contains('|')
            || !plot_style.is_empty()
            || !material.is_null()
            || !plotstyle_handle.is_null()
            || !xref_block_record_handle.is_null()
        {
            return Err(invalid("unsupported layer dependency"));
        }
        Ok(Self {
            handle: handle.value(),
            name: p.text(name)?,
            flags: LayerFlagsData::from_model(p, flags)?,
            color: (*color).into(),
            color_name: p.optional_text(color_name)?,
            book_name: p.optional_text(book_name)?,
            line_type: p.text(line_type)?,
            line_weight: (*line_weight).into(),
            is_plottable: *is_plottable,
            transparency: (*transparency).into(),
            description: p.text(description)?,
        })
    }
    fn into_model(self) -> Result<Layer> {
        let Self {
            handle,
            name,
            flags,
            color,
            color_name,
            book_name,
            line_type,
            line_weight,
            is_plottable,
            transparency,
            description,
        } = self;
        if name.contains('|') {
            return Err(invalid("external reference layer name"));
        }
        Ok(Layer {
            handle: Handle::new(handle),
            name,
            flags: flags.into_model()?,
            color: color.into(),
            color_name,
            book_name,
            line_type,
            line_weight: line_weight.into(),
            is_plottable,
            transparency: transparency.into(),
            description,
            plot_style: String::new(),
            material: Handle::NULL,
            plotstyle_handle: Handle::NULL,
            xref_block_record_handle: Handle::NULL,
        })
    }
}
named_map!(PatternElement { length: u64 });
impl PatternElement {
    fn from_model(_p: &mut Projection, value: &LineTypeElement) -> Result<Self> {
        let LineTypeElement { length, complex } = value;
        if complex.is_some() {
            return Err(invalid("complex linetype element"));
        }
        Ok(Self {
            length: length.to_bits(),
        })
    }
    fn into_model(self) -> Result<LineTypeElement> {
        let Self { length } = self;
        Ok(LineTypeElement {
            length: f64::from_bits(length),
            complex: None,
        })
    }
}
named_map!(pub(super) LineTypeData {
    handle: u64,
    name: String,
    description: String,
    elements: Vec<PatternElement>,
    pattern_length: u64,
    alignment: u32,
});
impl LineTypeData {
    fn from_model(p: &mut Projection, value: &LineType) -> Result<Self> {
        let LineType {
            handle,
            name,
            description,
            elements,
            pattern_length,
            alignment,
            xref_dependent,
            xref_block_record_handle,
        } = value;
        if name.contains('|') || *xref_dependent || !xref_block_record_handle.is_null() {
            return Err(invalid("external reference linetype"));
        }
        Ok(Self {
            handle: handle.value(),
            name: p.text(name)?,
            description: p.text(description)?,
            elements: p.list(elements, PatternElement::from_model)?,
            pattern_length: pattern_length.to_bits(),
            alignment: u32::from(*alignment),
        })
    }
    fn into_model(self) -> Result<LineType> {
        let Self {
            handle,
            name,
            description,
            elements,
            pattern_length,
            alignment,
        } = self;
        if name.contains('|') {
            return Err(invalid("external reference linetype name"));
        }
        Ok(LineType {
            handle: Handle::new(handle),
            name,
            description,
            elements: elements
                .into_iter()
                .map(PatternElement::into_model)
                .collect::<Result<_>>()?,
            pattern_length: f64::from_bits(pattern_length),
            alignment: char::from_u32(alignment)
                .ok_or_else(|| error(Kind::InvalidBody, "invalid Unicode scalar"))?,
            xref_dependent: false,
            xref_block_record_handle: Handle::NULL,
        })
    }
}
tagged!(pub(super) Payload {
    "line" => Line(data: Box<LineData>),
    "mline" => MLine(data: Box<MLineData>),
    "mline_style" => MLineStyle(data: Box<StyleData>),
    "layer" => Layer(data: Box<LayerData>),
    "linetype" => LineType(data: Box<LineTypeData>),
});
impl Payload {
    pub(super) fn from_model(p: &mut Projection, value: &SelectionPayload) -> Result<Self> {
        Ok(match value {
            SelectionPayload::Line(v) => Self::Line(Box::new(LineData::from_model(p, v)?)),
            SelectionPayload::MLine(v) => Self::MLine(Box::new(MLineData::from_model(p, v)?)),
            SelectionPayload::MLineStyle(v) => {
                Self::MLineStyle(Box::new(StyleData::from_model(p, v)?))
            }
            SelectionPayload::Layer(v) => Self::Layer(Box::new(LayerData::from_model(p, v)?)),
            SelectionPayload::LineType(v) => {
                Self::LineType(Box::new(LineTypeData::from_model(p, v)?))
            }
        })
    }
    pub(super) fn into_model(self) -> Result<SelectionPayload> {
        Ok(match self {
            Self::Line(v) => SelectionPayload::Line(Box::new(v.into_model()?)),
            Self::MLine(v) => SelectionPayload::MLine(Box::new(v.into_model()?)),
            Self::MLineStyle(v) => SelectionPayload::MLineStyle(Box::new(v.into_model()?)),
            Self::Layer(v) => SelectionPayload::Layer(Box::new(v.into_model()?)),
            Self::LineType(v) => SelectionPayload::LineType(Box::new(v.into_model()?)),
        })
    }
}
