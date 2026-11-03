#![cfg(feature = "ifcx")]
use ifccad::text::*;
use ifccad::*;
use opencaddocument::Ifcx;

fn requested(font: FontRequest) -> IfccadDocument {
    let mut drawing = load_ifccad_bytes(
        include_bytes!("fixtures/text-rich.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    let mtext = drawing
        .model
        .entities
        .iter_mut()
        .find_map(|e| match e {
            IfccadEntity::Native(n) => match &mut n.kind {
                IfccadEntityKind::MText(m) => Some(m),
                _ => None,
            },
            _ => None,
        })
        .expect("fixture has ordinary MText");
    mtext.character_format = CharacterFormat {
        font: Some(font),
        ..Default::default()
    };
    mtext.content = vec![MTextParagraph {
        inlines: vec![MTextInline::Run {
            text: "Review".into(),
            character_format: Default::default(),
        }],
        ..Default::default()
    }];
    drawing
}

#[test]
fn mtext_unwritable_font_request_is_not_sanitized() {
    for name in [r"project\font.shx", "A;B", "A|B", "{Font}"] {
        let drawing = requested(FontRequest::family(name));
        let result = Ifcx.import_ifccad(&drawing);
        assert!(result.is_err(), "must not silently change {name:?}");
    }
}

#[test]
fn mtext_extra_selector_has_located_loss() {
    let drawing = requested(FontRequest {
        family: Some("Requested Family".into()),
        cad_font_name: Some("requested.ttf".into()),
        ..Default::default()
    });
    let converted = Ifcx.import_ifccad(&drawing).unwrap();
    assert!(converted
        .losses
        .iter()
        .any(|loss| loss.message.contains("cadFontName") && loss.message.contains("/")));
}

#[test]
fn paragraph_stack_and_dormant_requests_are_audited() {
    let mut drawing = requested(FontRequest::family(r"dormant\font.shx"));
    let text = drawing
        .model
        .entities
        .iter_mut()
        .find_map(|e| match &mut e.as_native_mut()?.kind {
            IfccadEntityKind::MText(t) => Some(t.as_mut()),
            _ => None,
        })
        .unwrap();
    text.content[0].character_format.font = Some(FontRequest::family("A|B"));
    assert!(Ifcx
        .import_ifccad(&drawing)
        .unwrap_err()
        .to_string()
        .contains("/content/0/characterFormat/font/family"));
    let text = drawing
        .model
        .entities
        .iter_mut()
        .find_map(|e| match &mut e.as_native_mut()?.kind {
            IfccadEntityKind::MText(t) => Some(t.as_mut()),
            _ => None,
        })
        .unwrap();
    text.content[0].character_format.font = Some(FontRequest::family("Één Family"));
    text.content[0].inlines = vec![MTextInline::Stack(TextStack {
        upper: "1".into(),
        lower: "2".into(),
        character_format: CharacterFormat {
            font: Some(FontRequest::family("A;B")),
            ..Default::default()
        },
        ..Default::default()
    })];
    assert!(Ifcx
        .import_ifccad(&drawing)
        .unwrap_err()
        .to_string()
        .contains("/inlines/0/characterFormat/font/family"));
    let text = drawing
        .model
        .entities
        .iter_mut()
        .find_map(|e| match &mut e.as_native_mut()?.kind {
            IfccadEntityKind::MText(t) => Some(t.as_mut()),
            _ => None,
        })
        .unwrap();
    let MTextInline::Stack(stack) = &mut text.content[0].inlines[0] else {
        panic!()
    };
    stack.character_format.font = None;
    let converted = Ifcx.import_ifccad(&drawing).unwrap();
    assert!(converted
        .losses
        .iter()
        .any(|l| l.message.contains("dormant font override")));
    let names: Vec<_> = converted
        .document
        .model_space_entities()
        .filter_map(|e| match e {
            opencaddocument::EntityType::MText(t) => Some(t.value.as_str()),
            _ => None,
        })
        .collect();
    assert!(names.iter().any(|name| name.contains("Één Family")));
}
