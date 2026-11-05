//! One grouped block-attribute review gallery, constructed through the selected pair.
use ifccad::*;
use opencaddocument::entities::{AttributeDefinition, AttributeEntity, Insert, Line, MText, Text};
use opencaddocument::fields::NewField;
use opencaddocument::tables::BlockRecord;
use opencaddocument::{Document, EntityType, Ifcx, Vector3};
pub fn gallery() -> Result<IfccadDocument, Box<dyn std::error::Error + Send + Sync>> {
    let mut d = Document::new();
    let mut b = BlockRecord::new("ATTR_REVIEW_DOOR");
    b.handle = d.allocate_handle();
    b.block_entity_handle = d.allocate_handle();
    b.block_end_handle = d.allocate_handle();
    b.base_point = Vector3::new(7., 9., 0.);
    let owner = b.handle;
    d.block_records.add(b)?;
    let tags = [
        "CODE",
        "ROOM",
        "CONSTANT",
        "HIDDEN",
        "PRESET",
        "VERIFY",
        "EMPTY_MTEXT",
        "FILENAME",
        "LINE_LENGTH",
    ];
    let mut defs = Vec::new();
    for (i, tag) in tags.iter().enumerate() {
        let mut a = AttributeDefinition::new(
            (*tag).into(),
            format!("Prompt {tag}"),
            format!("Default {tag}"),
        );
        a.common.owner_handle = owner;
        a.insertion_point = Vector3::new(7., 9. - i as f64 * 18., 0.);
        a.alignment_point = a.insertion_point;
        a.height = 2.5;
        a.flags.constant = i == 2;
        a.flags.invisible = i == 3;
        a.flags.preset = i == 4;
        a.flags.verify = i == 5;
        a.lock_position = i == 5;
        a.field_length = 16;
        if i == 1 || i == 6 {
            a.is_multiline = true;
            a.vertical_alignment =
                opencaddocument::entities::attribute_definition::VerticalAlignment::Top;
            a.embedded_mtext = Some(Box::new(MText::with_value(
                if i == 6 {
                    ""
                } else {
                    "Room name\\PSecond line"
                },
                a.insertion_point,
            )));
            a.embedded_mtext.as_mut().unwrap().height = 2.5;
            a.embedded_mtext.as_mut().unwrap().rectangle_width = 80.;
        }
        defs.push((d.add_entity(EntityType::AttributeDefinition(a))?, i));
    }
    for (column, origin) in [
        (0, Vector3::new(0., 0., 0.)),
        (1, Vector3::new(130., 0., 0.)),
    ] {
        let mut ins = Insert::new("ATTR_REVIEW_DOOR", origin);
        if column == 1 {
            ins.rotation = 0.15;
            ins.set_x_scale(-1.2);
            ins.set_y_scale(1.1);
        }
        for (h, i) in &defs {
            if *i == 2 {
                continue;
            }
            let EntityType::AttributeDefinition(def) = d.get_entity(*h).unwrap() else {
                unreachable!()
            };
            let mut a = AttributeEntity::from_definition(def, None);
            a.common.handle = d.allocate_handle();
            a.attdef_handle = *h;
            a.value = match *i {
                0 => format!("D-{}", 101 + column),
                1 => format!("Room {}", column + 1),
                6 => "STALE OUTER FALLBACK".into(),
                7 => "Review gallery.dwg".into(),
                8 => "12.50".into(),
                _ => format!("Value {}", tags[*i]),
            };
            a.insertion_point = Vector3::new(origin.x, origin.y - *i as f64 * 18., 0.);
            a.alignment_point = a.insertion_point;
            if column == 1 && *i == 0 {
                a.tag = "RENAMED_CODE".into();
            }
            if let Some(m) = &mut a.embedded_mtext {
                m.insertion_point = a.insertion_point;
                m.value = if *i == 6 {
                    "".into()
                } else {
                    format!("Room {}\\P{{\\C1;Red detail}}", column + 1)
                };
            }
            ins.attributes.push(a);
        }
        d.add_entity(EntityType::Insert(ins))?;
    }
    let line = d.add_entity(EntityType::Line(Line::from_points(
        Vector3::new(0., -175., 0.),
        Vector3::new(12.5, -175., 0.),
    )))?;
    for (h, i) in &defs {
        if *i == 7 {
            d.set_text_field(
                *h,
                "File: %<\\_FldIdx 0>%",
                vec![NewField::new(
                    "\\AcVar Filename \\f \"%fn6\"",
                    "Review gallery.dwg",
                )],
            )
            .ok_or("field host")?;
        }
        if *i == 8 {
            let mut f = NewField::new(
                "\\AcObjProp Object(%<\\_ObjIdx 0>%).Length \\f \"%lu2%pr2\"",
                "12.50",
            );
            f.objects = vec![line];
            d.set_text_field(*h, "Length: %<\\_FldIdx 0>%", vec![f])
                .ok_or("field host")?;
        }
    }
    for (text, p) in [
        ("BLOCK ATTRIBUTES / normal", Vector3::new(0., 15., 0.)),
        (
            "renamed tag / mirrored nonuniform INSERT",
            Vector3::new(130., 15., 0.),
        ),
        (
            "Constant default has no ATTRIB. Hidden row is intentionally invisible.",
            Vector3::new(0., -190., 0.),
        ),
        (
            "Empty active MText keeps an old outer fallback in native storage.",
            Vector3::new(0., -205., 0.),
        ),
    ] {
        d.add_entity(EntityType::Text(Text::with_value(text, p).with_height(2.5)))?;
    }
    let exported = Ifcx.export_ifccad(&d)?;
    let mut n = exported.drawing;
    n.header.id = "block-attributes-gallery".into();
    n.annotation_scales = vec![
        IfccadAnnotationScale {
            id: IfccadAnnotationScaleId(1),
            name: "1:1 review".into(),
            paper_units: 1.,
            drawing_units: 1.,
            unit_scale: true,
        },
        IfccadAnnotationScale {
            id: IfccadAnnotationScaleId(2),
            name: "1:2 embedded".into(),
            paper_units: 1.,
            drawing_units: 2.,
            unit_scale: false,
        },
    ];
    n.id_counters.next_annotation_scale_id = 3;
    let fields: std::collections::HashMap<_, _> = n
        .blocks
        .iter()
        .flat_map(|b| &b.entities)
        .filter_map(|e| e.as_native())
        .filter_map(|e| {
            if let IfccadEntityKind::AttributeDefinition(a) = &e.kind {
                a.fields.clone().map(|f| (a.tag.clone(), f))
            } else {
                None
            }
        })
        .collect();
    for e in n
        .model
        .entities
        .iter_mut()
        .filter_map(|e| e.as_native_mut())
    {
        if let IfccadEntityKind::BlockInstance { attributes, .. } = &mut e.kind {
            for a in attributes.iter_mut() {
                a.value.fields = fields.get(&a.value.tag).cloned();
            }
            for a in attributes.iter_mut().filter(|a| a.value.tag == "ROOM") {
                let m = &a.value.m_text.as_ref().unwrap().text;
                let context = IfccadAttributeAnnotationContext::MText {
                    text: IfccadTextAnnotationContext {
                        insertion: [
                            a.value.text.placement.origin[0],
                            a.value.text.placement.origin[1],
                        ],
                        alignment: [
                            a.value.text.placement.origin[0],
                            a.value.text.placement.origin[1],
                        ],
                        rotation: 0.,
                        horizontal: IfccadTextAnnotationHorizontal::Left,
                    },
                    m_text_enabled: true,
                    embedded: Some(IfccadAttributeEmbeddedContext {
                        scale_id: IfccadAnnotationScaleId(2),
                        is_default: true,
                        presentation: IfccadMTextAnnotationContext {
                            insertion: m.placement.origin,
                            x_direction: [1., 0., 0.],
                            attachment: m.attachment,
                            rectangle_width: 80.,
                            rectangle_height: 15.,
                            columns: None,
                        },
                    }),
                };
                a.value.annotation = Some(Box::new(IfccadAnnotation {
                    mode: IfccadAnnotationMode::Enabled,
                    orientation: None,
                    reference: Some(IfccadAnnotationContextKey {
                        scale_id: IfccadAnnotationScaleId(1),
                    }),
                    contexts: vec![IfccadAnnotationContext {
                        key: IfccadAnnotationContextKey {
                            scale_id: IfccadAnnotationScaleId(1),
                        },
                        presentation: Some(context),
                    }],
                }));
            }
        }
    }
    let mut title = n
        .model
        .entities
        .iter()
        .find(|e| {
            matches!(
                e.as_native().map(|e| &e.kind),
                Some(IfccadEntityKind::BlockInstance { .. })
            )
        })
        .unwrap()
        .clone();
    {
        let e = title.as_native_mut().unwrap();
        e.id = n.id_counters.allocate_entity_id()?;
        if let IfccadEntityKind::BlockInstance {
            attributes,
            transform,
            ..
        } = &mut e.kind
        {
            transform.placement.origin = [40., 180., 0.];
            for a in attributes {
                a.id = n.id_counters.allocate_entity_id()?;
                a.value.text.placement.origin[0] += 40.;
                a.value.text.placement.origin[1] += 180.;
                if let Some(m) = &mut a.value.m_text {
                    m.text.placement.origin[0] += 40.;
                    m.text.placement.origin[1] += 180.;
                }
                a.value.annotation = None;
            }
        }
    }
    let mut nested = n.blocks[0].clone();
    nested.id = n.id_counters.next_block_id;
    n.id_counters.next_block_id += 1;
    nested.name = "NESTED_ATTRIBUTE_REVIEW".into();
    nested.base_point = [0.; 3];
    nested.bounds = None;
    nested.bounds_quality = None;
    nested.entities = vec![title.clone()];
    // The nested Model occurrence has its own local origin, independent of Paper.
    if let Some(e) = nested.entities[0].as_native_mut() {
        if let IfccadEntityKind::BlockInstance {
            transform,
            attributes,
            ..
        } = &mut e.kind
        {
            transform.placement.origin = [0.; 3];
            for a in attributes {
                a.value.text.placement.origin[0] -= 40.;
                a.value.text.placement.origin[1] -= 180.;
                if let Some(m) = &mut a.value.m_text {
                    m.text.placement.origin[0] -= 40.;
                    m.text.placement.origin[1] -= 180.;
                }
            }
        }
    }

    {
        let e = title.as_native_mut().unwrap();
        e.id = n.id_counters.allocate_entity_id()?;
        if let IfccadEntityKind::BlockInstance { attributes, .. } = &mut e.kind {
            for a in attributes {
                a.id = n.id_counters.allocate_entity_id()?;
            }
        }
    }
    let mut canvas = n
        .paper_layouts
        .iter()
        .find_map(|p| p.canvas.clone())
        .ok_or("missing default Paper canvas")?;
    canvas.frame = Some(ifccad::workspace_kernel::WorkspaceCanvasFrame {
        center: ifccad::geometry_kernel::Point3::new(148.5, 105., 0.),
        width: 297.,
        height: 210.,
    });
    canvas.view.center = ifccad::geometry_kernel::Point2::new(148.5, 105.);
    canvas.view.height = 230.;
    canvas.active_context = Some(IfccadPaperContext::Canvas);
    let mut paper_entities = vec![title];
    let label_template = n
        .model
        .entities
        .iter()
        .find(|e| {
            matches!(
                e.as_native().map(|e| &e.kind),
                Some(IfccadEntityKind::Text(_))
            )
        })
        .unwrap()
        .clone();
    for (label, x, y) in [
        ("ATTRIBUTE TITLEBLOCK / PAPER ONLY", 40., 198.),
        ("CODE: D-101", 160., 180.),
        ("ROOM: two lines, red detail", 160., 162.),
        ("CONSTANT: definition default", 160., 144.),
        ("HIDDEN: intentionally invisible", 160., 126.),
        ("PRESET", 160., 108.),
        ("VERIFY: position locked", 160., 90.),
        ("EMPTY_MTEXT: intentionally empty", 160., 72.),
        ("FILENAME: stored field", 160., 54.),
        ("LINE_LENGTH: cached 12.50", 160., 36.),
    ] {
        let mut label_entity = label_template.clone();
        let e = label_entity.as_native_mut().unwrap();
        e.id = n.id_counters.allocate_entity_id()?;
        if let IfccadEntityKind::Text(t) = &mut e.kind {
            t.placement.origin = [x, y, 0.];
            t.content[0].text = label.into();
        }
        paper_entities.push(label_entity);
    }
    n.paper_layouts.push(IfccadPaperLayout {
        annotation_state: None,
        bounds_quality: None,
        canvas: Some(canvas),
        id: n.id_counters.next_layout_id,
        name: "Attribute titleblock".into(),
        tab_index: n
            .paper_layouts
            .iter()
            .map(|l| l.tab_index)
            .max()
            .unwrap_or(0)
            + 1,
        settings: IfccadLayoutSettings {
            media: Some(IfccadLayoutMedia {
                unit: IfccadMediaUnit::Physical(
                    ifccad::geometry_kernel::CoordinateLengthUnit::Millimetre,
                ),
                width: 297.,
                height: 210.,
            }),
            // Media dimensions are physical millimetres; Paper coordinates are also
            // millimetres. A missing plot mapping inherits the CAD default inch page.
            plot_settings: Some(IfccadPlotSettings {
                plot_unit: ifccad::plot_kernel::PlotUnit::Millimetre,
                page: ifccad::plot_kernel::PlotPage {
                    printable_area: ifccad::plot_kernel::PlotRect {
                        min_x: 0.,
                        min_y: 0.,
                        max_x: 297.,
                        max_y: 210.,
                    },
                    rotation: ifccad::plot_kernel::PlotRotation::None,
                    device_name: None,
                    media_name: None,
                },
                area: ifccad::plot_kernel::PlotArea::Layout,
                mapping: ifccad::plot_kernel::PlotMapping {
                    scale: ifccad::plot_kernel::PlotScale::Fixed {
                        output_length: 1.,
                        scope_length: 1.,
                    },
                    placement: ifccad::plot_kernel::PlotPlacement::Offset {
                        reference: ifccad::plot_kernel::PlotOffsetReference::PrintableArea,
                        x: 0.,
                        y: 0.,
                    },
                },
                output: ifccad::plot_kernel::PlotOutput {
                    shaded_plot: ifccad::plot_kernel::ShadedPlot {
                        mode: ifccad::plot_kernel::ShadedPlotMode::AsDisplayed,
                        quality: ifccad::plot_kernel::ShadedPlotQuality {
                            mode: ifccad::plot_kernel::ShadedPlotQualityMode::Normal,
                            dpi: None,
                        },
                    },
                    apply_plot_styles: false,
                    plot_style_table_name: None,
                },
                options: ifccad::plot_kernel::PlotOptions {
                    plot_viewport_borders: false,
                    plot_paper_space_last: false,
                    hide_paper_space_objects: false,
                    plot_line_weights: true,
                    scale_line_weights: false,
                    plot_transparency: false,
                },
            }),
            limits: Some(ifccad::plot_kernel::PlotRect {
                min_x: 0.,
                min_y: 0.,
                max_x: 297.,
                max_y: 210.,
            }),
            ..Default::default()
        },
        bounds: None,
        entities: paper_entities,
    });
    n.id_counters.next_layout_id += 1;
    let mut occurrence = n
        .model
        .entities
        .iter()
        .find(|e| {
            matches!(
                e.as_native().map(|e| &e.kind),
                Some(IfccadEntityKind::BlockInstance { .. })
            )
        })
        .unwrap()
        .clone();
    {
        let e = occurrence.as_native_mut().unwrap();
        e.id = n.id_counters.allocate_entity_id()?;
        if let IfccadEntityKind::BlockInstance {
            definition_id,
            attributes,
            transform,
            annotation,
        } = &mut e.kind
        {
            *definition_id = nested.id;
            attributes.clear();
            *annotation = None;
            transform.placement.origin = [270., 0., 0.];
            transform.scale = [1.; 3];
        }
    }
    n.blocks.push(nested);
    n.model.entities.push(occurrence);
    let wire = encode_ifccad_document(&n)?;
    load_ifccad_bytes(wire.bytes(), Default::default())?;
    Ok(n)
}
#[cfg(not(test))]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let out = std::env::args().nth(1).ok_or("output IFCX path required")?;
    std::fs::write(out, encode_ifccad_document(&gallery()?)?.bytes())?;
    Ok(())
}
