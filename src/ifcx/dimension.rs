//! Dimensions: dimension styles, linear and aligned dimensions with their
//! own style overrides (the `ACAD`/`DSTYLE` extended data) and point
//! associations.
//!
//! The style mapping follows the cad-dimension crate of ifccad (MPL-2.0). IFCX
//! defines linear and aligned dimensions only; other kinds stay losses. Stored
//! dimension pictures (anonymous `*D` blocks) are not carried. On import,
//! present IFCCAD graphics are reported as a loss independently of the retained
//! definition; equivalent regenerated graphics are not guaranteed.

use std::collections::BTreeMap;

use acadrust::entities::{
    AttachmentPointType, Dimension, DimensionAligned, DimensionLinear, DimensionType,
};
use acadrust::objects::{
    AssocDimensionAssociation, AssocDimensionReference, AssociativeData, AssociativeObject,
    ObjectType,
};
use acadrust::tables::DimStyle;
use acadrust::xdata::{ExtendedDataRecord, XDataValue};
use ifccad::dimension::{
    evaluate_dimension, resolve_dimension_settings, validate_dimension_definition,
    validate_dimension_settings, validate_dimension_text_placement,
    DimensionAlternateChoice, DimensionAlternateNumberSettings, DimensionAngleFormat,
    DimensionAppearanceChoice, DimensionArcSymbol, DimensionBackground, DimensionCenterMark,
    DimensionDefinition, DimensionDistanceFormat, DimensionExtensionLength,
    DimensionFactorApplication, DimensionFitPlacement, DimensionFractionLayout,
    DimensionLabel, DimensionLabelPart, DimensionLineSpacing, DimensionMarker,
    DimensionMeasurement, DimensionPadding, DimensionPointSelector, DimensionRole,
    DimensionStyleSettings, DimensionTextMovement, DimensionTextOrientation,
    DimensionTextPlacement, DimensionToleranceSettings, DimensionZeroSuppression,
};
use ifccad::text::MTextAttachment;
use ifccad::{
    IfccadDimension, IfccadDimensionAssociation, IfccadDimensionOverrides,
    IfccadDimensionSettings, IfccadDimensionStyle,
};

use super::*;

// ---------------------------------------------------------------- DSTYLE

/// Dimension variables by DXF group code, as the per-dimension overrides in
/// a dimension's `ACAD`/`DSTYLE` extended data name them.
macro_rules! variables {
    ($($code:literal $kind:ident $field:ident),* $(,)?) => {
        const CODES: &[i16] = &[$($code),*];

        fn variable(style: &DimStyle, code: i16) -> Option<XDataValue> {
            Some(match code {
                $($code => variables!(@get $kind style.$field),)*
                _ => return None,
            })
        }

        fn set_variable(style: &mut DimStyle, code: i16, value: &XDataValue) {
            match code {
                $($code => variables!(@set $kind style.$field, value),)*
                _ => {}
            }
        }
    };
    (@get real $value:expr) => { XDataValue::Real($value) };
    (@get int $value:expr) => { XDataValue::Integer16($value) };
    (@get bool $value:expr) => { XDataValue::Integer16(i16::from($value)) };
    (@get text $value:expr) => { XDataValue::String($value.clone()) };
    (@get handle $value:expr) => { XDataValue::Handle($value) };
    (@set real $target:expr, $value:expr) => {
        if let XDataValue::Real(v) | XDataValue::Distance(v) | XDataValue::ScaleFactor(v) = $value {
            $target = *v;
        }
    };
    (@set int $target:expr, $value:expr) => {
        if let XDataValue::Integer16(v) = $value { $target = *v; }
    };
    (@set bool $target:expr, $value:expr) => {
        if let XDataValue::Integer16(v) = $value { $target = *v != 0; }
    };
    (@set text $target:expr, $value:expr) => {
        if let XDataValue::String(v) = $value { $target = v.clone(); }
    };
    (@set handle $target:expr, $value:expr) => {
        if let XDataValue::Handle(v) = $value { $target = *v; }
    };
}

variables! {
    3 text dimpost, 4 text dimapost, 40 real dimscale, 41 real dimasz, 42 real dimexo,
    43 real dimdli, 44 real dimexe, 45 real dimrnd, 46 real dimdle, 47 real dimtp,
    48 real dimtm, 49 real dimfxl, 50 real dimjogang, 69 int dimtfill, 70 int dimtfillclr,
    71 bool dimtol, 72 bool dimlim, 73 bool dimtih, 74 bool dimtoh, 75 bool dimse1,
    76 bool dimse2, 77 int dimtad, 78 int dimzin, 79 int dimazin, 90 int dimarcsym,
    140 real dimtxt, 141 real dimcen, 142 real dimtsz, 143 real dimaltf, 144 real dimlfac,
    145 real dimtvp, 146 real dimtfac, 147 real dimgap, 148 real dimaltrnd, 170 bool dimalt,
    171 int dimaltd, 172 bool dimtofl, 173 bool dimsah, 174 bool dimtix, 175 bool dimsoxd,
    176 int dimclrd, 177 int dimclre, 178 int dimclrt, 179 int dimadec, 271 int dimdec,
    272 int dimtdec, 273 int dimaltu, 274 int dimalttd, 275 int dimaunit, 276 int dimfrac,
    277 int dimlunit, 278 int dimdsep, 279 int dimtmove, 280 int dimjust, 281 bool dimsd1,
    282 bool dimsd2, 283 int dimtolj, 284 int dimtzin, 285 int dimaltz, 286 int dimalttz,
    288 bool dimupt, 289 int dimatfit, 290 bool dimfxlon, 294 bool dimtxtdirection,
    295 real dimaltmzf, 296 text dimaltmzs, 297 real dimmzf, 298 text dimmzs,
    340 handle dimtxsty_handle, 341 handle dimldrblk, 342 handle dimblk, 343 handle dimblk1,
    344 handle dimblk2, 345 handle dimltex_handle, 346 handle dimltex1_handle,
    347 handle dimltex2_handle, 371 int dimlwd, 372 int dimlwe,
}

/// `style` with a dimension's own overrides applied.
fn with_overrides(style: &DimStyle, common: &EntityCommon) -> DimStyle {
    let mut style = style.clone();
    let values = common
        .extended_data
        .get_record("ACAD")
        .and_then(|record| match record.values.first() {
            Some(XDataValue::String(name)) if name == "DSTYLE" => Some(&record.values[1..]),
            _ => None,
        })
        .unwrap_or(&[]);
    let mut values = values.iter();
    while let Some(value) = values.next() {
        if let (XDataValue::Integer16(code), Some(value)) = (value, values.clone().next()) {
            set_variable(&mut style, *code, value);
            values.next();
        }
    }
    style
}

/// The overrides that turn `base` into `effective`, as a `DSTYLE` record.
fn overrides_record(base: &DimStyle, effective: &DimStyle) -> Option<ExtendedDataRecord> {
    let changed: Vec<i16> = CODES
        .iter()
        .copied()
        .filter(|code| variable(base, *code) != variable(effective, *code))
        .collect();
    if changed.is_empty() {
        return None;
    }
    let mut record = ExtendedDataRecord::new("ACAD");
    record.add_value(XDataValue::String("DSTYLE".to_string()));
    record.add_value(XDataValue::ControlString("{".to_string()));
    for code in changed {
        record.add_value(XDataValue::Integer16(code));
        record.add_value(variable(effective, code).expect("listed code"));
    }
    record.add_value(XDataValue::ControlString("}".to_string()));
    Some(record)
}

// ---------------------------------------------------------------- styles

/// Built-in dimension arrowheads by their block names.
const MARKERS: [(&str, DimensionMarker<u64>); 5] = [
    ("_CLOSEDBLANK", DimensionMarker::ClosedOpen),
    ("_OPEN", DimensionMarker::Open),
    ("_DOT", DimensionMarker::Dot),
    ("_OBLIQUE", DimensionMarker::Tick),
    ("_NONE", DimensionMarker::None),
];

/// A dimension style as IFCX settings; what IFCX cannot hold falls back to
/// its default and is reported.
fn settings_out(style: &DimStyle, writer: &mut Writer) -> Option<IfccadDimensionSettings> {
    let document = writer.document;
    let text_style = document
        .text_styles
        .iter()
        .find(|t| !style.dimtxsty_handle.is_null() && t.handle == style.dimtxsty_handle)
        .or_else(|| document.text_styles.get(&style.dimtxsty))
        .or_else(|| document.text_styles.get("Standard"))?;
    let text_style_id = *writer.style_handles.get(&text_style.handle)?;
    let mut lost = Vec::new();
    let mut v = DimensionStyleSettings::new(text_style_id);
    v.presentation_scale = match style.dimscale {
        scale if scale > 0.0 => scale,
        _ => {
            lost.push("scale to viewport (DIMSCALE 0)");
            1.0
        }
    };
    v.dimension_line.extension = style.dimdle;
    v.dimension_line.suppress_first = style.dimsd1;
    v.dimension_line.suppress_second = style.dimsd2;
    v.dimension_line.appearance.color = color_choice_out(style.dimclrd, style.dimclrd_true_color);
    v.dimension_line.appearance.line_weight = weight_choice_out(style.dimlwd);
    v.dimension_line.appearance.line_pattern = pattern_choice_out(style.dimltex_handle, writer);
    for (first, extension) in [
        (true, &mut v.extension_lines.first),
        (false, &mut v.extension_lines.second),
    ] {
        extension.offset = style.dimexo;
        extension.extension = style.dimexe;
        extension.suppressed = if first { style.dimse1 } else { style.dimse2 };
        extension.length = match style.dimfxlon {
            true => DimensionExtensionLength::Fixed {
                length: style.dimfxl,
            },
            false => DimensionExtensionLength::Automatic,
        };
        extension.appearance.color = color_choice_out(style.dimclre, style.dimclre_true_color);
        extension.appearance.line_weight = weight_choice_out(style.dimlwe);
        let pattern = if first {
            style.dimltex1_handle
        } else {
            style.dimltex2_handle
        };
        extension.appearance.line_pattern = pattern_choice_out(pattern, writer);
    }
    let (first, second) = match style.dimsah {
        true => (
            (style.dimblk1, &style.dimblk1_name),
            (style.dimblk2, &style.dimblk2_name),
        ),
        false => ((style.dimblk, &style.dimblk_name), (style.dimblk, &style.dimblk_name)),
    };
    v.arrows.first = marker_out(first.0, first.1, writer);
    v.arrows.second = marker_out(second.0, second.1, writer);
    v.arrows.size = style.dimasz;
    v.arrows.tick_size = style.dimtsz;
    v.arrows.center_mark = match style.dimcen {
        0.0 => DimensionCenterMark::None,
        size if size > 0.0 => DimensionCenterMark::Mark { size },
        size => DimensionCenterMark::Lines { size: -size },
    };
    v.text.height = match text_style.height {
        height if height > 0.0 => height,
        _ => style.dimtxt,
    };
    v.text.gap = style.dimgap;
    v.text.attachment = match style.dimtad {
        0 => MTextAttachment::MiddleCenter,
        1 => MTextAttachment::BottomCenter,
        _ => {
            lost.push("text placement (DIMTAD)");
            MTextAttachment::BottomCenter
        }
    };
    if style.dimjust != 0 || style.dimtxtdirection || (style.dimtad == 0 && style.dimtvp != 0.0) {
        lost.push("text justification (DIMJUST, DIMTVP, DIMTXTDIRECTION)");
    }
    v.text.inside_orientation = orientation(style.dimtih);
    v.text.outside_orientation = orientation(style.dimtoh);
    v.text.movement = match style.dimtmove {
        0 => DimensionTextMovement::MoveDimensionLine,
        2 => DimensionTextMovement::Free,
        _ => DimensionTextMovement::AddLeader,
    };
    v.text.appearance.color = color_choice_out(style.dimclrt, style.dimclrt_true_color);
    let padding = DimensionPadding::Relative { factor: 0.0 };
    v.text.background = match style.dimtfill {
        1 => DimensionBackground::Canvas {
            opacity: 1.0,
            padding,
        },
        2 => match color_choice_out(style.dimtfillclr, style.dimtfillclr_true_color) {
            DimensionAppearanceChoice::Explicit(color) => DimensionBackground::Color {
                color,
                opacity: 1.0,
                padding,
            },
            _ => DimensionBackground::Canvas {
                opacity: 1.0,
                padding,
            },
        },
        _ => DimensionBackground::None,
    };
    v.fit.preferred_placement = match style.dimatfit {
        1 => DimensionFitPlacement::ArrowsInside,
        2 => DimensionFitPlacement::TextInside,
        _ => DimensionFitPlacement::BestFit,
    };
    if style.dimsoxd {
        lost.push("outside arrow suppression (DIMSOXD)");
    }
    v.fit.force_text_inside = style.dimtix;
    v.fit.keep_dimension_line_outside = style.dimtofl;
    let primary = &mut v.numbers.primary;
    primary.measurement_factor = match style.dimlfac.abs() {
        0.0 => 1.0,
        factor => factor,
    };
    primary.application = match style.dimlfac < 0.0 {
        true => DimensionFactorApplication::PaperOnly,
        false => DimensionFactorApplication::AllOwners,
    };
    primary.format = distance_format(style.dimlunit);
    primary.fraction_layout = match style.dimfrac {
        0 => DimensionFractionLayout::Horizontal,
        1 => DimensionFractionLayout::Diagonal,
        _ => DimensionFractionLayout::Inline,
    };
    primary.fraction_scale = match style.dimtfac {
        scale if scale > 0.0 => scale,
        _ => 1.0,
    };
    primary.precision = precision(style.dimdec);
    primary.decimal_separator = match style.dimdsep {
        0 => '.',
        code => char::from_u32(code as u32).unwrap_or('.'),
    };
    primary.rounding_increment = style.dimrnd.max(0.0);
    primary.zero_suppression = zero_suppression(style.dimzin);
    (primary.prefix, primary.suffix) = prefix_suffix(&style.dimpost);
    primary.angle_format = match style.dimaunit {
        1 => DimensionAngleFormat::DegreesMinutesSeconds,
        2 => DimensionAngleFormat::Grads,
        3 => DimensionAngleFormat::Radians,
        _ => DimensionAngleFormat::DecimalDegrees,
    };
    primary.arc_symbol = match style.dimarcsym {
        1 => DimensionArcSymbol::Above,
        2 => DimensionArcSymbol::None,
        _ => DimensionArcSymbol::Before,
    };
    let separator = primary.decimal_separator;
    let fraction_scale = primary.fraction_scale;
    if style.dimalt {
        let (prefix, suffix) = prefix_suffix(&style.dimapost);
        let (format, fraction_layout) = match style.dimaltu {
            4 => (DimensionDistanceFormat::Architectural, DimensionFractionLayout::Horizontal),
            5 => (DimensionDistanceFormat::Fractional, DimensionFractionLayout::Horizontal),
            6 => (DimensionDistanceFormat::Architectural, DimensionFractionLayout::Inline),
            7 => (DimensionDistanceFormat::Fractional, DimensionFractionLayout::Inline),
            code => (distance_format(code), DimensionFractionLayout::Inline),
        };
        v.numbers.alternate = Some(DimensionAlternateNumberSettings {
            fraction_layout,
            fraction_scale,
            factor: style.dimaltf,
            format,
            precision: precision(style.dimaltd),
            decimal_separator: separator,
            rounding_increment: style.dimaltrnd.max(0.0),
            zero_suppression: zero_suppression(style.dimaltz),
            prefix,
            suffix,
        });
    }
    v.numbers.tolerance = match (style.dimlim, style.dimtol) {
        (true, _) => DimensionToleranceSettings::Limits {
            upper: style.dimtp,
            lower: style.dimtm,
            precision: precision(style.dimtdec),
        },
        (false, true) => DimensionToleranceSettings::Deviations {
            upper: style.dimtp,
            lower: style.dimtm,
            precision: precision(style.dimtdec),
        },
        (false, false) => DimensionToleranceSettings::None,
    };
    if style.annotative {
        lost.push("annotative scaling");
    }
    for what in lost {
        writer
            .losses
            .note(format!("dimension style {}: {what}", style.name));
    }
    match validate_dimension_settings(&v) {
        Ok(()) => Some(v),
        Err(_) => {
            writer
                .losses
                .note(format!("the settings of dimension style {}", style.name));
            Some(DimensionStyleSettings::new(text_style_id))
        }
    }
}

/// IFCX settings as a dimension style named `name`.
fn style_in(name: &str, v: &IfccadDimensionSettings, names: &Names, document: &Document) -> DimStyle {
    let mut style = document
        .dim_styles
        .get(name)
        .cloned()
        .unwrap_or_else(|| DimStyle::new(name));
    style.dimscale = v.presentation_scale;
    style.dimdle = v.dimension_line.extension;
    style.dimsd1 = v.dimension_line.suppress_first;
    style.dimsd2 = v.dimension_line.suppress_second;
    (style.dimclrd, style.dimclrd_true_color) = color_choice_in(&v.dimension_line.appearance.color);
    style.dimlwd = weight_choice_in(&v.dimension_line.appearance.line_weight);
    style.dimltex_handle = pattern_choice_in(&v.dimension_line.appearance.line_pattern, names, document);
    let first = &v.extension_lines.first;
    let second = &v.extension_lines.second;
    style.dimexo = first.offset;
    style.dimexe = first.extension;
    style.dimse1 = first.suppressed;
    style.dimse2 = second.suppressed;
    match first.length {
        DimensionExtensionLength::Automatic => style.dimfxlon = false,
        DimensionExtensionLength::Fixed { length } => {
            style.dimfxlon = true;
            style.dimfxl = length;
        }
    }
    (style.dimclre, style.dimclre_true_color) = color_choice_in(&first.appearance.color);
    style.dimlwe = weight_choice_in(&first.appearance.line_weight);
    style.dimltex1_handle = pattern_choice_in(&first.appearance.line_pattern, names, document);
    style.dimltex2_handle = pattern_choice_in(&second.appearance.line_pattern, names, document);
    style.dimasz = v.arrows.size;
    style.dimtsz = v.arrows.tick_size;
    style.dimsah = v.arrows.first != v.arrows.second;
    let (first_block, first_name) = marker_in(&v.arrows.first, names, document);
    let (second_block, second_name) = marker_in(&v.arrows.second, names, document);
    match style.dimsah {
        true => {
            (style.dimblk1, style.dimblk1_name) = (first_block, first_name);
            (style.dimblk2, style.dimblk2_name) = (second_block, second_name);
        }
        false => (style.dimblk, style.dimblk_name) = (first_block, first_name),
    }
    style.dimcen = match v.arrows.center_mark {
        DimensionCenterMark::None => 0.0,
        DimensionCenterMark::Mark { size } => size,
        DimensionCenterMark::Lines { size } => -size,
    };
    if let Some((style_name, handle)) = names
        .styles
        .get(&v.text.style)
        .zip(names.style_handles.get(&v.text.style))
    {
        style.dimtxsty = style_name.clone();
        style.dimtxsty_handle = *handle;
    }
    style.dimtxt = v.text.height;
    style.dimgap = v.text.gap;
    style.dimtad = match v.text.attachment {
        MTextAttachment::MiddleCenter | MTextAttachment::MiddleLeft | MTextAttachment::MiddleRight => 0,
        _ => 1,
    };
    style.dimtih = v.text.inside_orientation == DimensionTextOrientation::Horizontal;
    style.dimtoh = v.text.outside_orientation == DimensionTextOrientation::Horizontal;
    style.dimtmove = match v.text.movement {
        DimensionTextMovement::MoveDimensionLine => 0,
        DimensionTextMovement::AddLeader => 1,
        DimensionTextMovement::Free => 2,
    };
    (style.dimclrt, style.dimclrt_true_color) = color_choice_in(&v.text.appearance.color);
    match &v.text.background {
        DimensionBackground::None => style.dimtfill = 0,
        DimensionBackground::Canvas { .. } => style.dimtfill = 1,
        DimensionBackground::Color { color, .. } => {
            style.dimtfill = 2;
            (style.dimtfillclr, style.dimtfillclr_true_color) =
                color_choice_in(&DimensionAppearanceChoice::Explicit(color.clone()));
        }
    }
    style.dimatfit = match v.fit.preferred_placement {
        DimensionFitPlacement::ArrowsInside => 1,
        DimensionFitPlacement::TextInside => 2,
        DimensionFitPlacement::BestFit => 3,
    };
    style.dimtix = v.fit.force_text_inside;
    style.dimtofl = v.fit.keep_dimension_line_outside;
    let primary = &v.numbers.primary;
    style.dimlfac = match primary.application {
        DimensionFactorApplication::PaperOnly => -primary.measurement_factor,
        DimensionFactorApplication::AllOwners => primary.measurement_factor,
    };
    style.dimlunit = format_code(primary.format);
    style.dimdec = i16::from(primary.precision);
    style.dimdsep = match primary.decimal_separator {
        '.' => 0,
        separator => i16::try_from(separator as u32).unwrap_or(0),
    };
    style.dimrnd = primary.rounding_increment;
    style.dimzin = zero_code(primary.zero_suppression);
    style.dimpost = post(&primary.prefix, &primary.suffix);
    style.dimaunit = match primary.angle_format {
        DimensionAngleFormat::DecimalDegrees => 0,
        DimensionAngleFormat::DegreesMinutesSeconds => 1,
        DimensionAngleFormat::Grads => 2,
        DimensionAngleFormat::Radians => 3,
    };
    style.dimfrac = match primary.fraction_layout {
        DimensionFractionLayout::Horizontal => 0,
        DimensionFractionLayout::Diagonal => 1,
        DimensionFractionLayout::Inline => 2,
    };
    style.dimtfac = primary.fraction_scale;
    style.dimarcsym = match primary.arc_symbol {
        DimensionArcSymbol::Before => 0,
        DimensionArcSymbol::Above => 1,
        DimensionArcSymbol::None => 2,
    };
    match &v.numbers.alternate {
        Some(alternate) => {
            style.dimalt = true;
            style.dimaltf = alternate.factor;
            style.dimaltu = match (alternate.format, alternate.fraction_layout) {
                (DimensionDistanceFormat::Architectural, DimensionFractionLayout::Horizontal) => 4,
                (DimensionDistanceFormat::Fractional, DimensionFractionLayout::Horizontal) => 5,
                (DimensionDistanceFormat::Architectural, _) => 6,
                (DimensionDistanceFormat::Fractional, _) => 7,
                (format, _) => format_code(format),
            };
            style.dimaltd = i16::from(alternate.precision);
            style.dimaltrnd = alternate.rounding_increment;
            style.dimaltz = zero_code(alternate.zero_suppression);
            style.dimapost = post(&alternate.prefix, &alternate.suffix);
        }
        None => style.dimalt = false,
    }
    (style.dimtol, style.dimlim) = match v.numbers.tolerance {
        DimensionToleranceSettings::None => (false, false),
        DimensionToleranceSettings::Deviations {
            upper,
            lower,
            precision,
        } => {
            (style.dimtp, style.dimtm, style.dimtdec) = (upper, lower, i16::from(precision));
            (true, false)
        }
        DimensionToleranceSettings::Limits {
            upper,
            lower,
            precision,
        } => {
            (style.dimtp, style.dimtm, style.dimtdec) = (upper, lower, i16::from(precision));
            (false, true)
        }
    };
    style
}

fn orientation(horizontal: bool) -> DimensionTextOrientation {
    match horizontal {
        true => DimensionTextOrientation::Horizontal,
        false => DimensionTextOrientation::Aligned,
    }
}

fn distance_format(code: i16) -> DimensionDistanceFormat {
    match code {
        1 => DimensionDistanceFormat::Scientific,
        3 => DimensionDistanceFormat::Engineering,
        4 => DimensionDistanceFormat::Architectural,
        5 => DimensionDistanceFormat::Fractional,
        _ => DimensionDistanceFormat::Decimal,
    }
}

fn format_code(format: DimensionDistanceFormat) -> i16 {
    match format {
        DimensionDistanceFormat::Scientific => 1,
        DimensionDistanceFormat::Decimal => 2,
        DimensionDistanceFormat::Engineering => 3,
        DimensionDistanceFormat::Architectural => 4,
        DimensionDistanceFormat::Fractional => 5,
    }
}

fn precision(value: i16) -> u8 {
    value.clamp(0, 8) as u8
}

fn zero_suppression(code: i16) -> DimensionZeroSuppression {
    DimensionZeroSuppression {
        leading: code & 4 != 0,
        trailing: code & 8 != 0,
        zero_feet: code & 1 == 0,
        zero_inches: code & 2 == 0,
    }
}

fn zero_code(value: DimensionZeroSuppression) -> i16 {
    i16::from(!value.zero_feet)
        + 2 * i16::from(!value.zero_inches)
        + 4 * i16::from(value.leading)
        + 8 * i16::from(value.trailing)
}

/// DIMPOST: text around the measurement `<>`; without one, a suffix.
fn prefix_suffix(value: &str) -> (String, String) {
    match value.split_once("<>") {
        Some((prefix, suffix)) => (prefix.to_string(), suffix.to_string()),
        None => (String::new(), value.to_string()),
    }
}

fn post(prefix: &str, suffix: &str) -> String {
    match (prefix.is_empty(), suffix.is_empty()) {
        (true, true) => String::new(),
        _ => format!("{prefix}<>{suffix}"),
    }
}

fn color_choice_out(index: i16, true_color: Option<Color>) -> DimensionAppearanceChoice<IfccadColor> {
    if let Some(color) = true_color.and_then(color_out) {
        return DimensionAppearanceChoice::Explicit(color);
    }
    match index {
        256 => DimensionAppearanceChoice::ByLayer,
        1..=255 => color_out(Color::Index(index as u8))
            .map_or(DimensionAppearanceChoice::InheritEntity, DimensionAppearanceChoice::Explicit),
        _ => DimensionAppearanceChoice::InheritEntity,
    }
}

fn color_choice_in(choice: &DimensionAppearanceChoice<IfccadColor>) -> (i16, Option<Color>) {
    match choice {
        DimensionAppearanceChoice::InheritEntity | DimensionAppearanceChoice::ByBlock => (0, None),
        DimensionAppearanceChoice::ByLayer => (256, None),
        DimensionAppearanceChoice::Explicit(color) => match color_in(color) {
            Color::Index(index) => (i16::from(index), None),
            color => (0, Some(color)),
        },
    }
}

fn weight_choice_out(weight: i16) -> DimensionAppearanceChoice<f64> {
    match weight {
        -1 => DimensionAppearanceChoice::ByLayer,
        0..=211 => DimensionAppearanceChoice::Explicit(f64::from(weight) / 100.0),
        _ => DimensionAppearanceChoice::InheritEntity,
    }
}

fn weight_choice_in(choice: &DimensionAppearanceChoice<f64>) -> i16 {
    match choice {
        DimensionAppearanceChoice::ByLayer => -1,
        DimensionAppearanceChoice::Explicit(millimetres) => (millimetres * 100.0).round() as i16,
        _ => -2,
    }
}

fn pattern_choice_out(
    handle: Handle,
    writer: &Writer,
) -> DimensionAppearanceChoice<IfccadLinePatternId> {
    writer
        .document
        .line_types
        .iter()
        .find(|linetype| !handle.is_null() && linetype.handle == handle)
        .and_then(|linetype| writer.line_patterns.get(&linetype.name.to_ascii_lowercase()))
        .map_or(DimensionAppearanceChoice::InheritEntity, |id| {
            DimensionAppearanceChoice::Explicit(*id)
        })
}

fn pattern_choice_in(
    choice: &DimensionAppearanceChoice<IfccadLinePatternId>,
    names: &Names,
    document: &Document,
) -> Handle {
    match choice {
        DimensionAppearanceChoice::Explicit(id) => names
            .line_patterns
            .get(id)
            .and_then(|name| document.line_types.get(name))
            .map_or(Handle::NULL, |linetype| linetype.handle),
        _ => Handle::NULL,
    }
}

fn marker_out(handle: Handle, name: &str, writer: &mut Writer) -> DimensionMarker<u64> {
    let document = writer.document;
    let block = document
        .block_records
        .iter()
        .find(|record| !handle.is_null() && record.handle == handle)
        .map(|record| record.name.clone())
        .unwrap_or_else(|| name.to_string());
    if block.is_empty() || block.eq_ignore_ascii_case("_CLOSEDFILLED") {
        return DimensionMarker::ClosedFilled;
    }
    if let Some((_, marker)) = MARKERS.iter().find(|(n, _)| block.eq_ignore_ascii_case(n)) {
        return marker.clone();
    }
    match writer.block(&block) {
        Ok(Some(id)) => DimensionMarker::LocalBlock { block: id },
        _ => {
            writer.losses.note(format!("the dimension arrowhead {block}"));
            DimensionMarker::ClosedFilled
        }
    }
}

/// A marker as an arrowhead block handle and name; built-in ones by name,
/// for the CAD to supply.
fn marker_in(marker: &DimensionMarker<u64>, names: &Names, document: &Document) -> (Handle, String) {
    let name = match marker {
        DimensionMarker::ClosedFilled => return (Handle::NULL, String::new()),
        DimensionMarker::LocalBlock { block } => names.blocks.get(block).cloned().unwrap_or_default(),
        other => MARKERS
            .iter()
            .find(|(_, m)| m == other)
            .map(|(name, _)| name.to_string())
            .unwrap_or_default(),
    };
    let handle = document.block_records.get(&name).map_or(Handle::NULL, |r| r.handle);
    (handle, name)
}

/// The dimension styles; fills the writer's style ids and base settings.
pub(super) fn write_styles(writer: &mut Writer) -> Result<(), Error> {
    let document = writer.document;
    for style in document.dim_styles.iter() {
        let Some(settings) = settings_out(style, writer) else {
            writer.losses.note(format!("dimension style {}", style.name));
            continue;
        };
        let id = writer.drawing.id_counters.allocate_dimension_style_id()?;
        let key = style.name.to_ascii_lowercase();
        writer.dim_styles.insert(key.clone(), (id, settings.clone()));
        writer.drawing.dimension_styles.push(IfccadDimensionStyle {
            id,
            name: style.name.clone(),
            settings,
            type_overrides: BTreeMap::new(),
        });
    }
    Ok(())
}

/// The dimension styles, after the blocks their arrowheads may use.
pub(super) fn read_styles(
    drawing: &IfccadDocument,
    document: &mut Document,
    names: &mut Names,
    losses: &mut Losses,
) -> Result<(), Error> {
    for style in &drawing.dimension_styles {
        if !style.type_overrides.is_empty() {
            losses.note(format!("per-kind settings of dimension style {}", style.name));
        }
        let mut converted = style_in(&style.name, &style.settings, names, document);
        match document.dim_styles.get_mut(&style.name) {
            Some(existing) => {
                converted.handle = existing.handle;
                *existing = converted;
            }
            None => {
                converted.handle = document.allocate_handle();
                document.dim_styles.add(converted)?;
            }
        }
        names.dim_styles.insert(style.id, (style.name.clone(), style.settings.clone(), style.type_overrides.clone()));
    }
    Ok(())
}

// ---------------------------------------------------------------- dimensions

pub(super) fn dimension_out(writer: &mut Writer, dimension: &Dimension) -> Option<IfccadEntityKind> {
    let base = dimension.base();
    let (first, second, line, rotation, oblique) = match dimension {
        Dimension::Linear(d) => (
            d.first_point,
            d.second_point,
            d.definition_point,
            Some(d.rotation),
            d.ext_line_rotation,
        ),
        Dimension::Aligned(d) => (d.first_point, d.second_point, d.definition_point, None, d.ext_line_rotation),
        _ => return None,
    };
    let key = base.style_name.to_ascii_lowercase();
    let key = match writer.dim_styles.contains_key(&key) {
        true => key,
        false => "standard".to_string(),
    };
    let (style_id, base_settings) = writer.dim_styles.get(&key)?.clone();
    let document = writer.document;
    let style = document.dim_styles.iter().find(|s| s.name.eq_ignore_ascii_case(&key))?;

    let normal = base.normal;
    let origin = to_ocs(line, normal);
    let local = |point: Vector3| {
        let p = to_ocs(point, normal);
        [p.x - origin.x, p.y - origin.y, p.z - origin.z]
    };
    let definition = match rotation {
        Some(angle) => DimensionDefinition::Linear {
            first_point: local(first),
            second_point: local(second),
            direction_angle: angle,
            dimension_line_point: [0.0, 0.0],
            extension_direction_angle: (oblique != 0.0).then_some(angle + oblique),
        },
        None => {
            if oblique != 0.0 {
                writer.losses.note("oblique extension lines of aligned dimensions");
            }
            DimensionDefinition::Aligned {
                first_point: local(first),
                second_point: local(second),
                dimension_line_point: [0.0, 0.0],
                extension_direction_angle: None,
            }
        }
    };
    validate_dimension_definition(&definition).ok()?;
    let text_placement = match base.text_user_positioned {
        true => DimensionTextPlacement::Explicit {
            position: [
                base.text_middle_point.x - origin.x,
                base.text_middle_point.y - origin.y,
            ],
            rotation: base.text_rotation,
            attachment: attachment_out(base.attachment_point),
        },
        false => DimensionTextPlacement::StyleDriven,
    };
    if validate_dimension_text_placement(&text_placement).is_err() {
        return None;
    }

    // Own overrides: the style's settings against those with the DSTYLE
    // record applied, plus what CAD keeps on the entity.
    let effective_style = with_overrides(style, &base.common);
    let mut effective = match overrides_record(style, &effective_style) {
        None => base_settings.clone(),
        Some(_) => {
            // The style's own losses were reported with the style.
            let losses = std::mem::take(&mut writer.losses);
            let settings = settings_out(&effective_style, writer);
            writer.losses = losses;
            settings?
        }
    };
    effective.arrows.flip_first = base.flip_arrow1;
    effective.arrows.flip_second = base.flip_arrow2;
    effective.text.line_spacing = match base.line_spacing_style {
        2 => DimensionLineSpacing::Exact {
            factor: base.line_spacing_factor,
        },
        _ => DimensionLineSpacing::AtLeast {
            factor: base.line_spacing_factor,
        },
    };
    let overrides = overrides_between(&base_settings, &effective);
    if base.horizontal_direction != 0.0 {
        writer.losses.note("dimension text direction");
    }
    if base.dimension_type != DimensionType::Linear && base.dimension_type != DimensionType::Aligned {
        return None;
    }
    let associations = associations_out(document, base.common.handle);
    if !associations.is_empty() {
        writer.dimension_sources.insert(base.common.handle, associations);
    }
    Some(IfccadEntityKind::Dimension(Box::new(IfccadDimension {
        placement: placement_out(origin, normal, 0.0),
        style_id,
        definition,
        text_placement,
        label: label_out(base.text_override()),
        overrides,
        associations: Vec::new(),
        graphics: None,
    })))
}

pub(super) fn dimension_in(
    source: &IfccadDimension,
    document: &Document,
    names: &Names,
    losses: &mut Losses,
) -> Option<EntityType> {
    if source.graphics.is_some() {
        losses.note("stored dimension graphics");
    }
    let (style_name, base_settings, kinds) = names.dim_styles.get(&source.style_id)?;
    let (origin, normal, frame) = placement_in(&source.placement);
    let frame = snap(frame, ANGLE_STEP);
    let (sin, cos) = frame.sin_cos();
    // Frame coordinates to the OCS of the normal, then to the world.
    let ocs = |[u, v, w]: [f64; 3]| {
        Vector3::new(origin.x + u * cos - v * sin, origin.y + u * sin + v * cos, origin.z + w)
    };
    let world = |p: [f64; 3]| ocs_to_world(ocs(p), normal);
    let (first, second) = source.definition.reference_points();
    let (line, _) = match &source.definition {
        DimensionDefinition::Linear {
            dimension_line_point,
            extension_direction_angle,
            ..
        }
        | DimensionDefinition::Aligned {
            dimension_line_point,
            extension_direction_angle,
            ..
        } => (*dimension_line_point, *extension_direction_angle),
    };
    let line = world([line[0], line[1], 0.0]);
    let mut dimension = match &source.definition {
        DimensionDefinition::Linear {
            direction_angle,
            extension_direction_angle,
            ..
        } => {
            let mut d = DimensionLinear::new(world(first), world(second));
            d.definition_point = line;
            d.rotation = direction_angle + frame;
            d.ext_line_rotation = extension_direction_angle.map_or(0.0, |angle| angle - direction_angle);
            Dimension::Linear(d)
        }
        DimensionDefinition::Aligned { .. } => {
            let mut d = DimensionAligned::new(world(first), world(second));
            d.definition_point = line;
            Dimension::Aligned(d)
        }
    };
    let kind = source.definition.kind();
    let effective = resolve_dimension_settings(base_settings, kind, kinds, &source.overrides);
    let base = dimension.base_mut();
    base.style_name = style_name.clone();
    base.normal = normal;
    base.definition_point = line;
    if let Ok(DimensionMeasurement::Distance(distance)) = evaluate_dimension(&source.definition) {
        base.actual_measurement = distance;
    }
    base.set_text_override(label_in(&source.label));
    if let DimensionTextPlacement::Explicit {
        position,
        rotation,
        attachment,
    } = &source.text_placement
    {
        base.text_middle_point = ocs([position[0], position[1], 0.0]);
        base.text_user_positioned = true;
        base.text_rotation = *rotation;
        base.attachment_point = attachment_in(*attachment);
    }
    base.flip_arrow1 = effective.arrows.flip_first;
    base.flip_arrow2 = effective.arrows.flip_second;
    (base.line_spacing_style, base.line_spacing_factor) = match effective.text.line_spacing {
        DimensionLineSpacing::AtLeast { factor } => (1, factor),
        DimensionLineSpacing::Exact { factor } => (2, factor),
    };
    let style = document.dim_styles.get(style_name)?;
    let effective_style = style_in(style_name, &effective, names, document);
    if let Some(record) = overrides_record(style, &effective_style) {
        base.common.extended_data.upsert_record(record);
    }
    Some(EntityType::Dimension(Box::new(dimension)))
}

/// The settings that differ between `base` and `effective`, as overrides.
fn overrides_between(
    base: &IfccadDimensionSettings,
    effective: &IfccadDimensionSettings,
) -> IfccadDimensionOverrides {
    let mut o = IfccadDimensionOverrides::default();
    macro_rules! differ {
        ($($($field:ident).+),+ $(,)?) => { $(
            if base.$($field).+ != effective.$($field).+ {
                o.$($field).+ = Some(effective.$($field).+.clone());
            }
        )+ };
    }
    differ!(
        dimension_line.appearance.color,
        dimension_line.appearance.opacity,
        dimension_line.appearance.line_pattern,
        dimension_line.appearance.line_weight,
        dimension_line.extension,
        dimension_line.suppress_first,
        dimension_line.suppress_second,
        extension_lines.first.appearance.color,
        extension_lines.first.appearance.opacity,
        extension_lines.first.appearance.line_pattern,
        extension_lines.first.appearance.line_weight,
        extension_lines.first.offset,
        extension_lines.first.extension,
        extension_lines.first.suppressed,
        extension_lines.first.length,
        extension_lines.second.appearance.color,
        extension_lines.second.appearance.opacity,
        extension_lines.second.appearance.line_pattern,
        extension_lines.second.appearance.line_weight,
        extension_lines.second.offset,
        extension_lines.second.extension,
        extension_lines.second.suppressed,
        extension_lines.second.length,
        arrows.first,
        arrows.second,
        arrows.size,
        arrows.tick_size,
        arrows.flip_first,
        arrows.flip_second,
        arrows.center_mark,
        text.appearance.color,
        text.appearance.opacity,
        text.appearance.line_pattern,
        text.appearance.line_weight,
        text.style,
        text.height,
        text.attachment,
        text.inside_orientation,
        text.outside_orientation,
        text.gap,
        text.movement,
        text.line_spacing,
        text.background,
        fit.preferred_placement,
        fit.force_text_inside,
        fit.keep_dimension_line_outside,
        numbers.primary.measurement_factor,
        numbers.primary.application,
        numbers.primary.format,
        numbers.primary.angle_format,
        numbers.primary.precision,
        numbers.primary.decimal_separator,
        numbers.primary.rounding_increment,
        numbers.primary.zero_suppression,
        numbers.primary.prefix,
        numbers.primary.suffix,
        numbers.primary.arc_symbol,
        numbers.primary.fraction_layout,
        numbers.primary.fraction_scale,
    );
    if base.numbers.alternate != effective.numbers.alternate {
        o.numbers.alternate = Some(match &effective.numbers.alternate {
            Some(alternate) => DimensionAlternateChoice::Enabled(alternate.clone()),
            None => DimensionAlternateChoice::None,
        });
    }
    if base.numbers.tolerance != effective.numbers.tolerance {
        o.numbers.tolerance = Some(effective.numbers.tolerance.clone());
    }
    if base.presentation_scale != effective.presentation_scale {
        o.presentation_scale = Some(effective.presentation_scale);
    }
    o
}

fn label_out(text: Option<&str>) -> DimensionLabel {
    let text = match text {
        None | Some("") | Some("<>") => return DimensionLabel::Generated,
        Some(" ") => return DimensionLabel::Suppressed,
        Some(text) => text,
    };
    let literal = |text: &str| DimensionLabelPart::Literal(vec![TextRun {
        text: text.to_string(),
        underline: false,
        overline: false,
        strike_through: false,
    }]);
    let mut parts = Vec::new();
    let mut rest = text;
    while let Some((before, after)) = rest.split_once("<>") {
        if !before.is_empty() {
            parts.push(literal(before));
        }
        parts.push(DimensionLabelPart::Measurement);
        rest = after;
    }
    if !rest.is_empty() {
        parts.push(literal(rest));
    }
    DimensionLabel::Template(parts)
}

fn label_in(label: &DimensionLabel) -> Option<String> {
    match label {
        DimensionLabel::Generated => None,
        DimensionLabel::Suppressed => Some(" ".to_string()),
        DimensionLabel::Template(parts) => Some(
            parts
                .iter()
                .map(|part| match part {
                    DimensionLabelPart::Measurement => "<>".to_string(),
                    DimensionLabelPart::Literal(runs) => {
                        runs.iter().map(|run| run.text.as_str()).collect()
                    }
                })
                .collect(),
        ),
    }
}

fn attachment_out(point: AttachmentPointType) -> MTextAttachment {
    match point {
        AttachmentPointType::TopLeft => MTextAttachment::TopLeft,
        AttachmentPointType::TopCenter => MTextAttachment::TopCenter,
        AttachmentPointType::TopRight => MTextAttachment::TopRight,
        AttachmentPointType::MiddleLeft => MTextAttachment::MiddleLeft,
        AttachmentPointType::MiddleCenter => MTextAttachment::MiddleCenter,
        AttachmentPointType::MiddleRight => MTextAttachment::MiddleRight,
        AttachmentPointType::BottomLeft => MTextAttachment::BottomLeft,
        AttachmentPointType::BottomCenter => MTextAttachment::BottomCenter,
        AttachmentPointType::BottomRight => MTextAttachment::BottomRight,
    }
}

fn attachment_in(attachment: MTextAttachment) -> AttachmentPointType {
    match attachment {
        MTextAttachment::TopLeft => AttachmentPointType::TopLeft,
        MTextAttachment::TopCenter => AttachmentPointType::TopCenter,
        MTextAttachment::TopRight => AttachmentPointType::TopRight,
        MTextAttachment::MiddleLeft => AttachmentPointType::MiddleLeft,
        MTextAttachment::MiddleCenter => AttachmentPointType::MiddleCenter,
        MTextAttachment::MiddleRight => AttachmentPointType::MiddleRight,
        MTextAttachment::BottomLeft => AttachmentPointType::BottomLeft,
        MTextAttachment::BottomCenter => AttachmentPointType::BottomCenter,
        MTextAttachment::BottomRight => AttachmentPointType::BottomRight,
    }
}

/// An OCS point in world coordinates.
fn ocs_to_world(point: Vector3, normal: Vector3) -> Vector3 {
    Matrix3::arbitrary_axis(normal).transform_point(point)
}

// ---------------------------------------------------------------- associations

/// A dimension's point associations: role, source entity and the point of
/// it, from the dimension's DIMASSOC object.
fn associations_out(
    document: &Document,
    dimension: Handle,
) -> Vec<(DimensionRole, Handle, DimensionPointSelector)> {
    let Some(association) = document.objects.values().find_map(|object| match object {
        ObjectType::Associative(object) => match &object.data {
            AssociativeData::DimensionAssociation(a) if a.dimension == dimension => Some(a),
            _ => None,
        },
        _ => None,
    }) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (slot, references) in association.references.iter().take(2).enumerate() {
        let [reference] = references.as_slice() else {
            continue;
        };
        let [source] = reference.xrefs.as_slice() else {
            continue;
        };
        let selector = match (reference.osnap_type, document.get_entity(*source)) {
            (13, Some(EntityType::Line(_))) => DimensionPointSelector::Start,
            (1, Some(EntityType::Line(line))) if reference.osnap_point == line.start => {
                DimensionPointSelector::Start
            }
            (1, Some(EntityType::Line(line))) if reference.osnap_point == line.end => {
                DimensionPointSelector::End
            }
            (3, Some(EntityType::Circle(_) | EntityType::Arc(_))) => DimensionPointSelector::Center,
            (4, Some(EntityType::Point(_))) => DimensionPointSelector::Position,
            _ => continue,
        };
        let role = match slot {
            0 => DimensionRole::FirstPoint,
            _ => DimensionRole::SecondPoint,
        };
        out.push((role, *source, selector));
    }
    out
}

/// Point each dimension at its source entities, the ones in its scope.
pub(super) fn link_associations(
    entities: &mut [IfccadEntity],
    ids: &HashMap<Handle, u64>,
    sources: &HashMap<Handle, Vec<(DimensionRole, Handle, DimensionPointSelector)>>,
) {
    let here: std::collections::HashMap<Handle, u64> = ids
        .iter()
        .filter(|(_, id)| entities.iter().any(|entity| entity.id() == **id))
        .map(|(handle, id)| (*handle, *id))
        .collect();
    let by_id: HashMap<u64, Handle> = here.iter().map(|(handle, id)| (*id, *handle)).collect();
    for entity in entities {
        let IfccadEntity::Native(IfccadNativeEntity {
            id,
            kind: IfccadEntityKind::Dimension(dimension),
            ..
        }) = entity
        else {
            continue;
        };
        let Some(links) = by_id.get(id).and_then(|handle| sources.get(handle)) else {
            continue;
        };
        dimension.associations = links
            .iter()
            .filter_map(|(role, source, selector)| {
                Some(IfccadDimensionAssociation {
                    role: *role,
                    source_entity_id: *here.get(source)?,
                    selector: *selector,
                })
            })
            .collect();
    }
}

/// DIMASSOC objects for the dimensions read with point associations.
pub(super) fn associations_in(
    drawing: &IfccadDocument,
    document: &mut Document,
    handles: &HashMap<u64, Handle>,
) {
    let scopes = drawing.blocks.iter().map(|block| &block.entities);
    let papers = drawing.paper_layouts.iter().map(|paper| &paper.entities);
    for entity in scopes.chain(papers).chain([&drawing.model.entities]).flatten() {
        let Some(IfccadNativeEntity {
            id,
            kind: IfccadEntityKind::Dimension(dimension),
            ..
        }) = entity.as_native()
        else {
            continue;
        };
        let Some(handle) = handles.get(id).copied() else {
            continue;
        };
        let mut association = AssocDimensionAssociation {
            dimension: handle,
            ..Default::default()
        };
        for link in &dimension.associations {
            let Some(source) = handles.get(&link.source_entity_id).copied() else {
                continue;
            };
            let (osnap_type, osnap_point) = match (link.selector, document.get_entity(source)) {
                (DimensionPointSelector::Start, Some(EntityType::Line(line))) => (13, line.start),
                (DimensionPointSelector::End, Some(EntityType::Line(line))) => (1, line.end),
                (DimensionPointSelector::Center, Some(EntityType::Circle(circle))) => {
                    (3, ocs_to_world(circle.center, circle.normal))
                }
                (DimensionPointSelector::Position, Some(EntityType::Point(point))) => {
                    (4, point.location)
                }
                _ => continue,
            };
            let slot = match link.role {
                DimensionRole::FirstPoint => 0,
                DimensionRole::SecondPoint => 1,
            };
            association.associativity |= 1 << slot;
            association.references[slot].push(AssocDimensionReference {
                class_name: "AcDbOsnapPointRef".to_string(),
                osnap_type,
                xrefs: vec![source],
                osnap_point,
                ..Default::default()
            });
        }
        if association.associativity == 0 {
            continue;
        }
        let mut object = AssociativeObject::new("DIMASSOC", "AcDbDimAssoc");
        object.handle = document.allocate_handle();
        object.owner = handle;
        object.data = AssociativeData::DimensionAssociation(association);
        document.objects.insert(object.handle, ObjectType::Associative(object));
    }
}
