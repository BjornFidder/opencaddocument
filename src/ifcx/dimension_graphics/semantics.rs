//! Closed comparison: only listed derived fields may differ. Other state stays
//! exact after the existing resource and equivalent-representation factoring.
use super::*;
use ifccad::text::{CharacterFormat, MTextInline, MTextParagraph, TextLineSpacing};

fn scalar(
    a: &mut f64,
    b: f64,
    category: NumericCategory,
    path: &str,
    n: &mut PictureNumerics,
) -> Prepared<()> {
    finite(*a)?;
    finite(b)?;
    if *a != b {
        n.record(path, category);
    }
    *a = b;
    Ok(())
}
fn coords<const N: usize>(
    a: &mut [f64; N],
    b: &[f64; N],
    category: NumericCategory,
    path: &str,
    n: &mut PictureNumerics,
) -> Prepared<()> {
    for (a, b) in a.iter_mut().zip(b) {
        scalar(a, *b, category, path, n)?;
    }
    Ok(())
}
fn vector_field(
    a: &mut Vector3,
    b: Vector3,
    category: NumericCategory,
    path: &str,
    n: &mut PictureNumerics,
) -> Prepared<()> {
    let mut value = array(*a);
    coords(&mut value, &array(b), category, path, n)?;
    *a = vector(value);
    Ok(())
}
fn placed(
    a: &mut IfccadPlacement,
    b: &IfccadPlacement,
    path: &str,
    n: &mut PictureNumerics,
) -> Prepared<()> {
    geometry::validate_frame(a)?;
    geometry::validate_frame(b)?;
    coords(&mut a.origin, &b.origin, NumericCategory::Position, path, n)?;
    coords(
        &mut a.x_axis,
        &b.x_axis,
        NumericCategory::Direction,
        path,
        n,
    )?;
    coords(
        &mut a.y_axis,
        &b.y_axis,
        NumericCategory::Direction,
        path,
        n,
    )
}
fn ordered<T: PartialEq>(a: &[T], b: &[T]) -> Prepared<()> {
    if a.len() != b.len() {
        return Err(unsupported("picture vertex count differs"));
    }
    // A recognizable permutation is a topology change, not arithmetic. The
    // converter itself carries each vertex through its original ordered slot.
    if a != b && a.iter().all(|p| b.contains(p)) && b.iter().all(|p| a.contains(p)) {
        return Err(unsupported("picture vertex order differs"));
    }
    Ok(())
}
fn sweep(a: f64, b: f64) -> Prepared<()> {
    finite(a)?;
    finite(b)?;
    if a == 0. || b == 0. || a.signum() != b.signum() || (a.abs() == TAU) != (b.abs() == TAU) {
        return Err(unsupported(
            "picture arc direction or full/partial meaning differs",
        ));
    }
    Ok(())
}
fn cad_sweep(start: f64, end: f64) -> f64 {
    match (end - start).rem_euclid(TAU) {
        0. => TAU,
        x => x,
    }
}
fn oblique(
    a: &mut CharacterFormat<IfccadColor>,
    b: &CharacterFormat<IfccadColor>,
    path: &str,
    n: &mut PictureNumerics,
) -> Prepared<()> {
    match (&mut a.oblique_angle, b.oblique_angle) {
        (Some(a), Some(b)) => scalar(a, b, NumericCategory::Angle, path, n)?,
        (None, None) => {}
        _ => return Err(unsupported("MText oblique presence differs")),
    }
    Ok(())
}
fn content(
    a: Vec<MTextParagraph<IfccadColor>>,
    b: Vec<MTextParagraph<IfccadColor>>,
    path: &str,
    n: &mut PictureNumerics,
) -> Prepared<()> {
    fn normalize(
        mut paragraphs: Vec<MTextParagraph<IfccadColor>>,
    ) -> Prepared<Vec<MTextParagraph<IfccadColor>>> {
        fn defaults(f: &mut CharacterFormat<IfccadColor>) {
            f.underline.get_or_insert(false);
            f.overline.get_or_insert(false);
            f.strike_through.get_or_insert(false);
            f.tracking.get_or_insert(1.);
            f.position.get_or_insert(ifccad::text::TextPosition::Bottom);
        }
        for p in &mut paragraphs {
            let resolved =
                ifccad::text::resolve_paragraph_format(&Default::default(), &p.paragraph_format)
                    .map_err(|_| invalid("invalid MText paragraph format"))?;
            p.paragraph_format = ifccad::text::ParagraphFormat {
                alignment: Some(resolved.alignment),
                left_indent_factor: Some(resolved.left_indent_factor),
                right_indent_factor: Some(resolved.right_indent_factor),
                first_line_indent_factor: Some(resolved.first_line_indent_factor),
                space_before: Some(resolved.space_before),
                space_after: Some(resolved.space_after),
                line_spacing: None,
                tab_stops: Some(resolved.tab_stops),
            };
            defaults(&mut p.character_format);
            for i in &mut p.inlines {
                match i {
                    MTextInline::Run {
                        character_format, ..
                    } => defaults(character_format),
                    MTextInline::Stack(s) => defaults(&mut s.character_format),
                    _ => {}
                }
            }
            let mut inlines: Vec<MTextInline<IfccadColor>> = Vec::new();
            for i in std::mem::take(&mut p.inlines) {
                match (inlines.last_mut(), &i) {
                    (
                        Some(MTextInline::Run {
                            text: a,
                            character_format: fa,
                        }),
                        MTextInline::Run {
                            text: b,
                            character_format: fb,
                        },
                    ) if fa == fb => a.push_str(b),
                    _ => inlines.push(i),
                }
            }
            p.inlines = inlines;
        }
        Ok(paragraphs)
    }
    let mut a = normalize(a)?;
    let b = normalize(b)?;
    if a.len() != b.len() {
        return Err(unsupported("MText paragraphs differ"));
    }
    for (a, b) in a.iter_mut().zip(&b) {
        oblique(&mut a.character_format, &b.character_format, path, n)?;
        if a.inlines.len() != b.inlines.len() {
            return Err(unsupported("MText inline structure differs"));
        }
        for (a, b) in a.inlines.iter_mut().zip(&b.inlines) {
            match (a, b) {
                (
                    MTextInline::Run {
                        character_format: a,
                        ..
                    },
                    MTextInline::Run {
                        character_format: b,
                        ..
                    },
                ) => oblique(a, b, path, n)?,
                (MTextInline::Stack(a), MTextInline::Stack(b)) => {
                    oblique(&mut a.character_format, &b.character_format, path, n)?
                }
                _ => {}
            }
        }
    }
    if a == b {
        Ok(())
    } else {
        Err(unsupported("MText text or formatting differs"))
    }
}
fn parsed(value: &str) -> Prepared<Vec<MTextParagraph<IfccadColor>>> {
    text::picture_mtext_content(value)
        .ok_or_else(|| unsupported("MText markup has an unsupported or malformed code"))
}
fn native_spacing(t: &ifccad::IfccadMText) -> (bool, f64) {
    match t.paragraph_format.line_spacing {
        Some(TextLineSpacing::Exact { distance }) => (true, distance),
        Some(TextLineSpacing::AtLeast { distance }) => (false, distance),
        Some(TextLineSpacing::Multiple { factor }) => (false, factor * (5. / 3.) * t.height),
        None => (false, (5. / 3.) * t.height),
    }
}
fn native_mtext(
    a: &mut ifccad::IfccadMText,
    b: &ifccad::IfccadMText,
    path: &str,
    n: &mut PictureNumerics,
) -> Prepared<()> {
    // Paragraph-local spacing is not represented by the CAD grammar.
    let local_spacing = |t: &ifccad::IfccadMText| {
        t.content
            .iter()
            .map(|p| p.paragraph_format.line_spacing)
            .collect::<Vec<_>>()
    };
    if local_spacing(a) != local_spacing(b) {
        return Err(unsupported("MText paragraph-local spacing differs"));
    }
    content(
        text::picture_native_content(a),
        text::picture_native_content(b),
        path,
        n,
    )?;
    let (kind, mut distance) = native_spacing(a);
    let (expected, d) = native_spacing(b);
    if kind != expected {
        return Err(unsupported("MText line spacing mode differs"));
    }
    scalar(&mut distance, d, NumericCategory::Spacing, path, n)?;
    a.character_format = b.character_format.clone();
    a.paragraph_format = b.paragraph_format.clone();
    a.content = b.content.clone();
    placed(&mut a.placement, &b.placement, path, n)?;
    scalar(&mut a.rotation, b.rotation, NumericCategory::Angle, path, n)
}
pub(super) fn native(
    actual: &IfccadEntityKind,
    expected: &IfccadEntityKind,
    path: &str,
    n: &mut PictureNumerics,
) -> Prepared<()> {
    let mut a = actual.clone();
    match (&mut a, expected) {
        (
            IfccadEntityKind::LineSegment {
                start: a,
                end: c,
                extrusion_direction: na,
                ..
            },
            IfccadEntityKind::LineSegment {
                start: b,
                end: d,
                extrusion_direction: nb,
                ..
            },
        ) => {
            coords(a, b, NumericCategory::Position, path, n)?;
            coords(c, d, NumericCategory::Position, path, n)?;
            coords(na, nb, NumericCategory::Direction, path, n)?;
        }
        (
            IfccadEntityKind::Point { placement: a, .. },
            IfccadEntityKind::Point { placement: b, .. },
        )
        | (
            IfccadEntityKind::Circle { placement: a, .. },
            IfccadEntityKind::Circle { placement: b, .. },
        ) => placed(a, b, path, n)?,
        (
            IfccadEntityKind::Arc {
                placement: a,
                start_parameter: sa,
                sweep_parameter: wa,
                ..
            },
            IfccadEntityKind::Arc {
                placement: b,
                start_parameter: sb,
                sweep_parameter: wb,
                ..
            },
        ) => {
            sweep(*wa, *wb)?;
            placed(a, b, path, n)?;
            scalar(sa, *sb, NumericCategory::Angle, path, n)?;
            scalar(wa, *wb, NumericCategory::Angle, path, n)?;
        }
        (
            IfccadEntityKind::Ellipse {
                placement: a,
                semi_major_radius: ma,
                semi_minor_radius: mi,
            },
            IfccadEntityKind::Ellipse {
                placement: b,
                semi_major_radius: mb,
                semi_minor_radius: mj,
            },
        ) => {
            placed(a, b, path, n)?;
            scalar(ma, *mb, NumericCategory::Length, path, n)?;
            scalar(mi, *mj, NumericCategory::Length, path, n)?;
        }
        (
            IfccadEntityKind::EllipseArc {
                placement: a,
                semi_major_radius: ma,
                semi_minor_radius: mi,
                start_parameter: sa,
                sweep_parameter: wa,
            },
            IfccadEntityKind::EllipseArc {
                placement: b,
                semi_major_radius: mb,
                semi_minor_radius: mj,
                start_parameter: sb,
                sweep_parameter: wb,
            },
        ) => {
            sweep(*wa, *wb)?;
            placed(a, b, path, n)?;
            scalar(ma, *mb, NumericCategory::Length, path, n)?;
            scalar(mi, *mj, NumericCategory::Length, path, n)?;
            scalar(sa, *sb, NumericCategory::Angle, path, n)?;
            scalar(wa, *wb, NumericCategory::Angle, path, n)?;
        }
        (
            IfccadEntityKind::PlanarPolyline {
                placement: a,
                vertices: va,
                ..
            },
            IfccadEntityKind::PlanarPolyline {
                placement: b,
                vertices: vb,
                ..
            },
        ) => {
            ordered(va, vb)?;
            placed(a, b, path, n)?;
            for (a, b) in va.iter_mut().zip(vb) {
                coords(a, b, NumericCategory::Position, path, n)?;
            }
        }
        (
            IfccadEntityKind::SpatialPolyline { vertices: a, .. },
            IfccadEntityKind::SpatialPolyline { vertices: b, .. },
        ) => {
            ordered(a, b)?;
            for (a, b) in a.iter_mut().zip(b) {
                coords(a, b, NumericCategory::Position, path, n)?;
            }
        }
        (IfccadEntityKind::Solid(a), IfccadEntityKind::Solid(b))
        | (IfccadEntityKind::Trace(a), IfccadEntityKind::Trace(b)) => {
            ordered(&a.corners, &b.corners)?;
            placed(&mut a.placement, &b.placement, path, n)?;
            for (a, b) in a.corners.iter_mut().zip(&b.corners) {
                coords(a, b, NumericCategory::Position, path, n)?;
            }
        }
        (IfccadEntityKind::Text(a), IfccadEntityKind::Text(b)) => {
            placed(&mut a.placement, &b.placement, path, n)?;
            scalar(&mut a.rotation, b.rotation, NumericCategory::Angle, path, n)?;
            if text::text_runs(&text::text_value(&a.content))
                == text::text_runs(&text::text_value(&b.content))
            {
                a.content = b.content.clone();
            }
        }
        (IfccadEntityKind::MText(a), IfccadEntityKind::MText(b)) => native_mtext(a, b, path, n)?,
        (
            IfccadEntityKind::BlockInstance { transform: a, .. },
            IfccadEntityKind::BlockInstance { transform: b, .. },
        ) => {
            placed(&mut a.placement, &b.placement, path, n)?;
            scalar(&mut a.rotation, b.rotation, NumericCategory::Angle, path, n)?;
        }
        _ => {}
    }
    if a == *expected {
        Ok(())
    } else {
        Err(unsupported(
            "native picture kind, content or direct parameters differ",
        ))
    }
}
pub(super) fn cad(
    actual: &EntityType,
    expected: &EntityType,
    path: &str,
    n: &mut PictureNumerics,
) -> Prepared<()> {
    let mut a = actual.clone();
    match (&mut a, expected) {
        (EntityType::Line(a), EntityType::Line(b)) => {
            vector_field(&mut a.start, b.start, NumericCategory::Position, path, n)?;
            vector_field(&mut a.end, b.end, NumericCategory::Position, path, n)?;
            vector_field(&mut a.normal, b.normal, NumericCategory::Direction, path, n)?;
        }
        (EntityType::Point(a), EntityType::Point(b)) => vector_field(
            &mut a.location,
            b.location,
            NumericCategory::Position,
            path,
            n,
        )?,
        (EntityType::Circle(a), EntityType::Circle(b)) => {
            vector_field(&mut a.center, b.center, NumericCategory::Position, path, n)?;
            vector_field(&mut a.normal, b.normal, NumericCategory::Direction, path, n)?;
        }
        (EntityType::Arc(a), EntityType::Arc(b)) => {
            sweep(
                cad_sweep(a.start_angle, a.end_angle),
                cad_sweep(b.start_angle, b.end_angle),
            )?;
            vector_field(&mut a.center, b.center, NumericCategory::Position, path, n)?;
            vector_field(&mut a.normal, b.normal, NumericCategory::Direction, path, n)?;
            scalar(
                &mut a.start_angle,
                b.start_angle,
                NumericCategory::Angle,
                path,
                n,
            )?;
            scalar(
                &mut a.end_angle,
                b.end_angle,
                NumericCategory::Angle,
                path,
                n,
            )?;
        }
        (EntityType::Ellipse(a), EntityType::Ellipse(b)) => {
            if ((a.end_parameter - a.start_parameter).rem_euclid(TAU) == 0.)
                != ((b.end_parameter - b.start_parameter).rem_euclid(TAU) == 0.)
            {
                return Err(unsupported("ellipse full/partial meaning differs"));
            }
            vector_field(&mut a.center, b.center, NumericCategory::Position, path, n)?;
            vector_field(&mut a.normal, b.normal, NumericCategory::Direction, path, n)?;
            vector_field(
                &mut a.major_axis,
                b.major_axis,
                NumericCategory::Length,
                path,
                n,
            )?;
            scalar(
                &mut a.minor_axis_ratio,
                b.minor_axis_ratio,
                NumericCategory::Length,
                path,
                n,
            )?;
            scalar(
                &mut a.start_parameter,
                b.start_parameter,
                NumericCategory::Angle,
                path,
                n,
            )?;
            scalar(
                &mut a.end_parameter,
                b.end_parameter,
                NumericCategory::Angle,
                path,
                n,
            )?;
        }
        (EntityType::LwPolyline(a), EntityType::LwPolyline(b)) => {
            ordered(
                &a.vertices.iter().map(|v| v.location).collect::<Vec<_>>(),
                &b.vertices.iter().map(|v| v.location).collect::<Vec<_>>(),
            )?;
            scalar(
                &mut a.elevation,
                b.elevation,
                NumericCategory::Position,
                path,
                n,
            )?;
            vector_field(&mut a.normal, b.normal, NumericCategory::Direction, path, n)?;
            for (a, b) in a.vertices.iter_mut().zip(&b.vertices) {
                scalar(
                    &mut a.location.x,
                    b.location.x,
                    NumericCategory::Position,
                    path,
                    n,
                )?;
                scalar(
                    &mut a.location.y,
                    b.location.y,
                    NumericCategory::Position,
                    path,
                    n,
                )?;
            }
        }
        (EntityType::Polyline3D(a), EntityType::Polyline3D(b)) => {
            ordered(
                &a.vertices.iter().map(|v| v.position).collect::<Vec<_>>(),
                &b.vertices.iter().map(|v| v.position).collect::<Vec<_>>(),
            )?;
            for (a, b) in a.vertices.iter_mut().zip(&b.vertices) {
                vector_field(
                    &mut a.position,
                    b.position,
                    NumericCategory::Position,
                    path,
                    n,
                )?;
            }
        }
        (EntityType::Solid(a), EntityType::Solid(b)) => {
            ordered(
                &[
                    a.first_corner,
                    a.second_corner,
                    a.third_corner,
                    a.fourth_corner,
                ],
                &[
                    b.first_corner,
                    b.second_corner,
                    b.third_corner,
                    b.fourth_corner,
                ],
            )?;
            vector_field(
                &mut a.first_corner,
                b.first_corner,
                NumericCategory::Position,
                path,
                n,
            )?;
            vector_field(
                &mut a.second_corner,
                b.second_corner,
                NumericCategory::Position,
                path,
                n,
            )?;
            vector_field(
                &mut a.third_corner,
                b.third_corner,
                NumericCategory::Position,
                path,
                n,
            )?;
            vector_field(
                &mut a.fourth_corner,
                b.fourth_corner,
                NumericCategory::Position,
                path,
                n,
            )?;
            vector_field(&mut a.normal, b.normal, NumericCategory::Direction, path, n)?;
        }
        (EntityType::Text(a), EntityType::Text(b)) => {
            vector_field(
                &mut a.insertion_point,
                b.insertion_point,
                NumericCategory::Position,
                path,
                n,
            )?;
            vector_field(&mut a.normal, b.normal, NumericCategory::Direction, path, n)?;
            scalar(&mut a.rotation, b.rotation, NumericCategory::Angle, path, n)?;
            match (&mut a.alignment_point, b.alignment_point) {
                (Some(a), Some(b)) => vector_field(a, b, NumericCategory::Position, path, n)?,
                (None, None) => {}
                _ => return Err(unsupported("Text alignment presence differs")),
            }
            if text::text_runs(&a.value) == text::text_runs(&b.value) {
                a.value = b.value.clone();
            }
        }
        (EntityType::MText(a), EntityType::MText(b)) => {
            content(parsed(&a.value)?, parsed(&b.value)?, path, n)?;
            a.value = b.value.clone();
            vector_field(
                &mut a.insertion_point,
                b.insertion_point,
                NumericCategory::Position,
                path,
                n,
            )?;
            let direction = |t: &acadrust::entities::MText| {
                t.dwg_x_direction
                    .unwrap_or_else(|| Vector3::new(t.rotation.cos(), t.rotation.sin(), 0.))
            };
            let mut aa = direction(a);
            let bb = direction(b);
            vector_field(&mut aa, bb, NumericCategory::Direction, path, n)?;
            if b.dwg_x_direction.is_none() {
                scalar(&mut a.rotation, b.rotation, NumericCategory::Angle, path, n)?;
            } else {
                a.rotation = b.rotation;
            }
            a.dwg_x_direction = b.dwg_x_direction;
            scalar(
                &mut a.line_spacing_factor,
                b.line_spacing_factor,
                NumericCategory::Spacing,
                path,
                n,
            )?;
            if a.background_fill_flags & 2 != 0 && b.background_fill_flags & 2 != 0 {
                a.background_color = b.background_color;
            }
            a.extents_width = b.extents_width;
            a.extents_height = b.extents_height;
        }
        (EntityType::Insert(a), EntityType::Insert(b)) => {
            vector_field(
                &mut a.insert_point,
                b.insert_point,
                NumericCategory::Position,
                path,
                n,
            )?;
            vector_field(&mut a.normal, b.normal, NumericCategory::Direction, path, n)?;
            scalar(&mut a.rotation, b.rotation, NumericCategory::Angle, path, n)?;
        }
        _ => {}
    }
    if a == *expected {
        Ok(())
    } else {
        Err(unsupported(
            "CAD picture kind, content or direct parameters differ",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equivalent_markup_and_finite_derived_positions_are_not_content_loss() {
        let mut expected = acadrust::entities::MText::new();
        expected.value = r"\A1;240".into();
        let mut actual = expected.clone();
        actual.value = r"{\A1;240}".into();
        assert!(cad(
            &EntityType::MText(Box::new(actual)),
            &EntityType::MText(Box::new(expected)),
            "p0",
            &mut PictureNumerics::default()
        )
        .is_ok());
        let expected = IfccadEntityKind::LineSegment {
            start: [1e16, 0., 0.],
            end: [1e16 + 2., 0., 0.],
            thickness: 0.,
            extrusion_direction: [0., 0., 1.],
        };
        let actual = IfccadEntityKind::LineSegment {
            start: [1e16, 0., 0.],
            end: [1e16, 0., 0.],
            thickness: 0.,
            extrusion_direction: [0., 0., 1.],
        };
        assert!(native(&actual, &expected, "p0", &mut PictureNumerics::default()).is_ok());
    }
    #[test]
    fn direct_radius_and_scale_sign_are_protected() {
        let p = IfccadPlacement {
            origin: [0.; 3],
            x_axis: [1., 0., 0.],
            y_axis: [0., 1., 0.],
        };
        let a = IfccadEntityKind::Circle {
            radius: 1.,
            placement: p.clone(),
            thickness: 0.,
        };
        let b = IfccadEntityKind::Circle {
            radius: 2.,
            placement: p.clone(),
            thickness: 0.,
        };
        assert!(native(&a, &b, "p0", &mut PictureNumerics::default()).is_err());
        let a = IfccadEntityKind::BlockInstance {
            definition_id: 1,
            transform: IfccadBlockTransform {
                placement: p.clone(),
                rotation: 0.,
                scale: [-1., 1., 1.],
            },
        };
        let b = IfccadEntityKind::BlockInstance {
            definition_id: 1,
            transform: IfccadBlockTransform {
                placement: p,
                rotation: 0.,
                scale: [1., 1., 1.],
            },
        };
        assert!(native(&a, &b, "p0", &mut PictureNumerics::default()).is_err());
    }

    #[test]
    fn mtext_words_full_formatting_columns_style_and_layer_stay_protected() {
        let mut expected = acadrust::entities::MText::new();
        expected.value = "plain".into();
        for value in [
            "different",
            r"\C1;plain",
            r"\fArial;plain",
            r"\Lplain",
            r"\S1/2;",
            r"\pxqc;plain",
        ] {
            let mut actual = expected.clone();
            actual.value = value.into();
            assert!(
                cad(
                    &EntityType::MText(Box::new(actual)),
                    &EntityType::MText(Box::new(expected.clone())),
                    "p0",
                    &mut PictureNumerics::default()
                )
                .is_err(),
                "{value}"
            );
        }
        for which in 0..5 {
            let mut actual = expected.clone();
            match which {
                0 => actual.column_data.width = 2.,
                1 => actual.style = "Different".into(),
                2 => actual.common.layer = "Different".into(),
                3 => actual.height = 2.,
                _ => actual.rectangle_width = 3.,
            }
            assert!(
                cad(
                    &EntityType::MText(Box::new(actual)),
                    &EntityType::MText(Box::new(expected.clone())),
                    "p0",
                    &mut PictureNumerics::default()
                )
                .is_err(),
                "field {which}"
            );
        }
    }

    #[test]
    fn ordered_vertices_closure_bulges_and_widths_stay_protected() {
        let p = IfccadPlacement {
            origin: [0.; 3],
            x_axis: [1., 0., 0.],
            y_axis: [0., 1., 0.],
        };
        let expected = IfccadEntityKind::PlanarPolyline {
            placement: p,
            vertices: vec![[0., 0.], [2., 1.], [3., 4.]],
            bulges: vec![0., 0.5, 0.],
            closed: false,
            line_pattern_generation: IfccadLinePatternGeneration::PerSegment,
            widths: IfccadPlanarPolylineWidths::Constant { width: 0.25 },
            thickness: 0.,
        };
        for which in 0..4 {
            let mut actual = expected.clone();
            let IfccadEntityKind::PlanarPolyline {
                vertices,
                bulges,
                closed,
                widths,
                ..
            } = &mut actual
            else {
                panic!()
            };
            match which {
                0 => vertices.swap(0, 1),
                1 => bulges[1] = -0.5,
                2 => *closed = true,
                _ => *widths = IfccadPlanarPolylineWidths::Constant { width: 0.5 },
            }
            assert!(
                native(&actual, &expected, "p0", &mut PictureNumerics::default()).is_err(),
                "field {which}"
            );
        }
    }
    #[test]
    fn paragraph_spacing_font_metadata_and_decimal_tabs_are_not_erased() {
        let expected = parsed("plain").unwrap_or_else(|_| panic!("plain text must parse"));
        for which in 0..3 {
            let mut actual = expected.clone();
            match which {
                0 => actual[0].paragraph_format.space_after = Some(2.),
                1 => {
                    actual[0].paragraph_format.tab_stops = Some(vec![ifccad::text::TextTabStop {
                        position_factor: 3.,
                        alignment: ifccad::text::TextTabAlignment::Decimal { separator: ',' },
                    }])
                }
                _ => {
                    actual[0].character_format.font = Some(ifccad::text::FontRequest {
                        big_font_name: Some("bigfont.shx".into()),
                        ..Default::default()
                    })
                }
            }
            assert!(
                content(
                    actual,
                    expected.clone(),
                    "p0",
                    &mut PictureNumerics::default()
                )
                .is_err(),
                "field {which}"
            );
        }
    }

    #[test]
    fn arc_discrete_meaning_and_oblique_presence_stay_protected() {
        for (a, b) in [(0., 1.), (-1., 1.), (TAU, TAU - 1e-12)] {
            assert!(sweep(a, b).is_err());
        }
        let mut actual = CharacterFormat::default();
        let expected = CharacterFormat {
            oblique_angle: Some(0.),
            ..Default::default()
        };
        assert!(oblique(
            &mut actual,
            &expected,
            "p0",
            &mut PictureNumerics::default()
        )
        .is_err());
    }
}
