#![cfg(feature = "ifcx")]
use ifccad::line_pattern::SymbolSource;
use ifccad::text::{FontRequest, TextStyleProperties};
use ifccad::*;
use opencaddocument::Ifcx;

#[test]
fn absent_cad_font_selector_is_not_replaced_with_txt() {
    let mut cad = opencaddocument::Document::new();
    let style = cad.text_styles.get_mut("Standard").unwrap();
    style.font_file.clear();
    style.true_type_font.clear();
    assert!(Ifcx.export_ifccad(&cad).is_err());
}

#[test]
fn unsupported_style_descriptor_fields_are_located() {
    let mut drawing = load_ifccad_bytes(
        include_bytes!("fixtures/font-symbol-base.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    let id = drawing.id_counters.allocate_text_style_id().unwrap();
    drawing.text_styles.push(IfccadTextStyle {
        id,
        name: "Review".into(),
        annotation: Default::default(),
        properties: TextStyleProperties::new(FontRequest::family("Review")),
    });
    drawing.text_styles[0].properties.font = FontRequest {
        family: Some("Review".into()),
        charset: Some(256),
        ..Default::default()
    };
    assert!(Ifcx
        .import_ifccad(&drawing)
        .err()
        .unwrap()
        .to_string()
        .contains("/charset"));
    drawing.text_styles[0].properties.font.charset = Some(0);
    drawing.text_styles[0].properties.font.pitch = Some(4);
    assert!(Ifcx
        .import_ifccad(&drawing)
        .err()
        .unwrap()
        .to_string()
        .contains("/pitch"));
    let mut cad = opencaddocument::Document::new();
    let style = cad.text_styles.get_mut("Standard").unwrap();
    style.true_type_font = "Review".into();
    style.true_type_font_flags = 0x04000022;
    let result = Ifcx.export_ifccad(&cad).unwrap();
    assert!(result
        .losses
        .iter()
        .any(|l| l.message.contains("unknown TrueType flag bits 0x4000000")));
}

fn physical(document: &opencaddocument::Document, format: &str) -> opencaddocument::Document {
    use opencaddocument::{DwgReader, DwgWriter, DxfReader, DxfWriter};
    let bytes = match format {
        "dwg" => DwgWriter::write_to_vec(document).unwrap(),
        "binary" => DxfWriter::new_binary(document).write_to_vec().unwrap(),
        _ => DxfWriter::new(document).write_to_vec().unwrap(),
    };
    let read = if format == "dwg" {
        DwgReader::from_stream(std::io::Cursor::new(bytes))
            .read_with_stats()
            .unwrap()
    } else {
        DxfReader::from_reader(std::io::Cursor::new(bytes))
            .unwrap()
            .read_with_stats()
            .unwrap()
    };
    assert_eq!(read.stats.recovered_errors, 0);
    assert_eq!(read.stats.skipped_source_records, 0);
    read.document
}

#[test]
fn broken_marker_style_is_not_a_missing_external_file() {
    use opencaddocument::tables::{
        LineType, LineTypeComplexContent, LineTypeComplexData, LineTypeElement,
    };
    use opencaddocument::types::Handle;
    let mut cad = opencaddocument::Document::new();
    let mut pattern = LineType::new("BrokenShape");
    pattern.handle = cad.allocate_handle();
    pattern.elements.push(LineTypeElement {
        length: 10.,
        complex: Some(LineTypeComplexData {
            content: LineTypeComplexContent::Shape { shape_number: 42 },
            style_handle: Handle::NULL,
            scale: 1.,
            rotation: 0.,
            absolute_rotation: false,
            upright: false,
            source_flags: 0,
            offset: [0., 0.],
        }),
    });
    pattern.pattern_length = 10.;
    cad.line_types.add_or_replace(pattern);
    assert!(Ifcx.export_ifccad(&cad).is_err());
}

#[test]
fn style_font_names_and_face_fields_survive() {
    // Literal flags from ODA DWG specification 5.4.1, section20.4.56.
    for (bold, italic, charset, pitch, flags) in [
        (false, false, 0, 34, 34),
        (false, true, 0, 34, 16777250),
        (true, false, 0, 34, 33554466),
        (true, true, 0, 34, 50331682),
        (false, true, 128, 49, 16810033),
        (false, false, 255, 0, 65280),
    ] {
        let mut drawing = load_ifccad_bytes(
            include_bytes!("fixtures/font-symbol-base.ifcx"),
            Default::default(),
        )
        .unwrap()
        .into_document();
        let font = FontRequest {
            family: Some("Requested Family".into()),
            cad_font_name: Some("requested.ttf".into()),
            big_font_name: Some("requested-big.shx".into()),
            bold: Some(bold),
            italic: Some(italic),
            charset: Some(charset),
            pitch: Some(pitch),
        };
        let id = drawing.id_counters.allocate_text_style_id().unwrap();
        drawing.text_styles.push(IfccadTextStyle {
            id,
            name: "Requested".into(),
            annotation: Default::default(),
            properties: TextStyleProperties::new(font.clone()),
        });
        let cad = Ifcx.import_ifccad(&drawing).unwrap();
        let style = cad.document.text_styles.get("Requested").unwrap();
        assert_eq!(style.true_type_font_flags, flags);
        let native = Ifcx.export_ifccad(&cad.document).unwrap();
        assert_eq!(
            native
                .drawing
                .text_styles
                .iter()
                .find(|s| s.name == "Requested")
                .unwrap()
                .properties
                .font,
            font
        );
        for format in ["dwg", "ascii", "binary"] {
            let reopened = physical(&cad.document, format);
            let native = Ifcx.export_ifccad(&reopened).unwrap();
            assert_eq!(
                native
                    .drawing
                    .text_styles
                    .iter()
                    .find(|s| s.name == "Requested")
                    .unwrap()
                    .properties
                    .font,
                font,
                "{format}"
            );
            let wire = encode_ifccad_document(&native.drawing).unwrap();
            assert!(load_ifccad_bytes(wire.bytes(), Default::default()).is_ok());
        }
    }
}

#[test]
fn equal_named_shx_sources_keep_distinct_style_records() {
    let mut drawing = load_ifccad_bytes(
        include_bytes!("fixtures/font-symbol-base.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    drawing.id_counters.next_symbol_source_id = 3;
    for id in [1, 2] {
        drawing.symbol_sources.push(IfccadSymbolSource {
            id: IfccadSymbolSourceId(id),
            source: SymbolSource::Shx {
                file_name: "missing.shx".into(),
                cad_file_name: None,
            },
        });
    }
    drawing.id_counters.next_line_pattern_id = 2;
    drawing.line_patterns.push(IfccadLinePattern {
        id: IfccadLinePatternId(1),
        name: "SharedSource".into(),
        description: None,
        pattern: vec![0, 1]
            .into_iter()
            .map(|_| ifccad::line_pattern::LinePatternElement {
                length: 10.,
                marker: Some(IfccadLinePatternMarker::Symbol {
                    symbol_source_id: IfccadSymbolSourceId(1),
                    shape_number: 42,
                    transform: Default::default(),
                }),
            })
            .collect(),
    });
    let converted = Ifcx.import_ifccad(&drawing).unwrap();
    let styles: Vec<_> = converted
        .document
        .text_styles
        .iter()
        .filter(|s| s.is_shape_file)
        .collect();
    assert_eq!(styles.len(), 2);
    assert_ne!(styles[0].handle, styles[1].handle);
    for format in ["dwg", "ascii", "binary"] {
        let reopened = physical(&converted.document, format);
        let restored = Ifcx.export_ifccad(&reopened).unwrap().drawing;
        assert_eq!(restored.symbol_sources.len(), 2, "{format}");
        let pattern = restored
            .line_patterns
            .iter()
            .find(|p| p.name == "SharedSource")
            .unwrap();
        let refs: Vec<_> = pattern
            .pattern
            .iter()
            .map(|e| match e.marker.as_ref().unwrap() {
                IfccadLinePatternMarker::Symbol {
                    symbol_source_id, ..
                } => *symbol_source_id,
                _ => panic!("symbol role"),
            })
            .collect();
        assert_eq!(refs[0], refs[1], "{format}: shared source");
        assert_eq!(
            restored
                .symbol_sources
                .iter()
                .filter(|s| s.id != refs[0])
                .count(),
            1,
            "{format}: independent unused source"
        );
    }
}

#[test]
fn original_shx_path_is_retained_in_both_directions() {
    let mut drawing = load_ifccad_bytes(
        include_bytes!("fixtures/font-symbol-base.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    drawing.id_counters.next_symbol_source_id = 2;
    drawing.symbol_sources.push(IfccadSymbolSource {
        id: IfccadSymbolSourceId(1),
        source: SymbolSource::Shx {
            file_name: "missing.shx".into(),
            cad_file_name: Some(r"C:\project\missing.shx".into()),
        },
    });
    let converted = Ifcx.import_ifccad(&drawing).unwrap();
    let style = converted
        .document
        .text_styles
        .iter()
        .find(|s| s.is_shape_file)
        .unwrap();
    assert_eq!(style.font_file, r"C:\project\missing.shx");
    let restored = Ifcx.export_ifccad(&converted.document).unwrap();
    assert_eq!(
        restored.drawing.symbol_sources[0].source,
        drawing.symbol_sources[0].source
    );
    for format in ["dwg", "ascii", "binary"] {
        let reopened = physical(&converted.document, format);
        let restored = Ifcx.export_ifccad(&reopened).unwrap();
        assert_eq!(
            restored.drawing.symbol_sources[0].source, drawing.symbol_sources[0].source,
            "{format}"
        );
    }
}

#[test]
fn literal_font_descriptor_is_read_without_using_our_writer() {
    let cad = opencaddocument::DxfReader::from_reader(std::io::Cursor::new(include_bytes!(
        "fixtures/font-source-flags.dxf"
    )))
    .unwrap()
    .read()
    .unwrap();
    let drawing = Ifcx.export_ifccad(&cad).unwrap().drawing;
    let font = &drawing
        .text_styles
        .iter()
        .find(|s| s.name == "Charset128")
        .unwrap()
        .properties
        .font;
    assert_eq!(font.charset, Some(128));
    assert_eq!(font.pitch, Some(49));
    assert_eq!(font.bold, Some(false));
    assert_eq!(font.italic, Some(true));
}
