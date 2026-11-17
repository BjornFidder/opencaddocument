#![cfg(feature = "ifcx")]

use ifccad::{encode_ifccad_document, load_ifccad_bytes, IfccadPlotStyle, IfccadPlotStyleMode};
use opencaddocument::{
    entities::Line,
    objects::{Layout, ObjectType, PlaceHolder},
    Color, Document, Dwg, Dxf, DxfVersion, EntityType, Format, Ifcx,
};

fn paper_layout(document: &mut Document) -> &mut Layout {
    document
        .objects
        .values_mut()
        .find_map(|object| match object {
            ObjectType::Layout(layout) if layout.name == "Layout1" => Some(layout),
            _ => None,
        })
        .expect("default paper layout")
}

#[test]
fn plot_intent_survives_dxf_and_dwg_with_external_tables() {
    let named = IfccadPlotStyle::Named {
        name: "Review".into(),
    };
    for (mode, table_name) in [
        (IfccadPlotStyleMode::ColorDependent, "proof.ctb"),
        (IfccadPlotStyleMode::Named, "proof.stb"),
    ] {
        let mut drawing = Document::with_version(DxfVersion::AC1032);
        let color_dependent = mode == IfccadPlotStyleMode::ColorDependent;
        drawing.header.plotstyle_mode = color_dependent;
        let layout = paper_layout(&mut drawing);
        layout.plot_style_sheet = table_name.into();
        layout.plot_flags.plot_plot_styles = false;

        let mut line = Line::from_coords(10.0, 10.0, 0.0, 30.0, 10.0, 0.0);
        line.common.color = if color_dependent {
            Color::Index(251)
        } else {
            let dictionary = drawing.header.acad_plotstylename_dict_handle;
            let mut style = PlaceHolder::new();
            style.handle = drawing.allocate_handle();
            style.owner = dictionary;
            let style_handle = style.handle;
            drawing
                .objects
                .insert(style_handle, ObjectType::PlaceHolder(style));
            let Some(ObjectType::DictionaryWithDefault(styles)) =
                drawing.objects.get_mut(&dictionary)
            else {
                panic!("missing plot-style dictionary");
            };
            styles.add_entry("Review", style_handle);
            line.common.plotstyle_flags = 3;
            line.common.plotstyle_handle = Some(style_handle);
            drawing.layers.get_mut("0").unwrap().plotstyle_handle = style_handle;
            Color::Rgb {
                r: 26,
                g: 85,
                b: 132,
            }
        };
        drawing
            .add_paper_space_entity(EntityType::Line(line))
            .unwrap();

        if !color_dependent {
            for (flag, y) in [(1, 20.0), (2, 30.0)] {
                let mut line = Line::from_coords(10.0, y, 0.0, 30.0, y, 0.0);
                line.common.plotstyle_flags = flag;
                drawing
                    .add_paper_space_entity(EntityType::Line(line))
                    .unwrap();
            }
        }

        for codec in [&Dxf as &dyn Format, &Dwg] {
            let (bytes, _) = codec.write(&drawing).expect("write initial CAD");
            let (cad, _) = codec.read(&bytes).expect("read initial CAD");
            let mut native = Ifcx.export_ifccad(&cad).expect("convert to IFCCAD").drawing;
            assert_eq!(native.plot_style_mode, mode, "{}", codec.name());
            let paper = &native.paper_layouts[0];
            let binding = native.plot_styles.layout_sources[&paper.id];
            let styles = &mut native.plot_styles;
            let source = styles.sources.get_mut(&binding).expect("layout table");
            assert_eq!(source.file_name, table_name);
            source.sha256 = Some("0123456789abcdef".repeat(4));
            if !color_dependent {
                assert_eq!(styles.entity_assignments[&paper.entities[0].id()], named);
                assert_eq!(
                    styles.entity_assignments[&paper.entities[1].id()],
                    IfccadPlotStyle::ByBlock
                );
                assert_eq!(
                    styles.entity_assignments[&paper.entities[2].id()],
                    IfccadPlotStyle::Normal
                );
                assert_eq!(styles.layer_assignments[&native.layers[0].id], named);
            }
            let bytes = encode_ifccad_document(&native).expect("encode IFCCAD");
            let native = load_ifccad_bytes(bytes.bytes(), Default::default())
                .expect("strictly read IFCCAD")
                .into_document();
            let converted = Ifcx.import_ifccad(&native).expect("convert back to CAD");
            assert!(
                converted.losses.iter().any(|loss| {
                    loss.handle.is_some() && loss.message.contains("without its SHA-256")
                }),
                "{:?}",
                converted.losses
            );

            let (output, _) = codec.write(&converted.document).expect("write final CAD");
            let (mut returned, _) = codec.read(&output).expect("read final CAD");
            assert_eq!(returned.header.plotstyle_mode, color_dependent);
            let layout = paper_layout(&mut returned);
            assert_eq!(layout.plot_style_sheet, table_name);
            assert!(!layout.plot_flags.plot_plot_styles);
            let first_line_color = returned.entities().find_map(|entity| match entity {
                EntityType::Line(line) if line.start.y == 10.0 => Some(line.common.color),
                _ => None,
            });
            let expected_color = if color_dependent {
                Color::Index(251)
            } else {
                Color::Rgb {
                    r: 26,
                    g: 85,
                    b: 132,
                }
            };
            assert_eq!(first_line_color, Some(expected_color));
            let back = Ifcx
                .export_ifccad(&returned)
                .expect("inspect final CAD")
                .drawing;
            assert_eq!(
                back.plot_styles.entity_assignments,
                native.plot_styles.entity_assignments
            );
            assert_eq!(
                back.plot_styles.layer_assignments,
                native.plot_styles.layer_assignments
            );
            assert!(back
                .plot_styles
                .sources
                .values()
                .all(|source| source.sha256.is_none()));
        }
    }
}
