//! Complete saved-picture transfer. No renderer, source-index guessing or partial capture.
use crate::Error;
use acadrust::io::dxf::code_page::LegacyEncoding;
use acadrust::{CheckedProxyGraphicRecord, CheckedProxyGraphics};
use ifccad::*;
use std::collections::BTreeMap;

mod commands;

/// Resource indices belong to the physical CAD tables, never to native IDs.
pub(super) fn capture(
    writer: &super::Writer,
    data: Option<&[u8]>,
) -> Result<Option<IfccadLeaderGraphics>, Error> {
    let Some(data) = data else { return Ok(None) };
    let layers = writer
        .document
        .layers
        .iter()
        .enumerate()
        .filter_map(|(index, layer)| {
            let id = writer
                .layers
                .get(&layer.name.to_ascii_lowercase())
                .copied()?;
            Some(
                u32::try_from(index)
                    .map(|index| (index, id))
                    .map_err(|_| Error::from("graphics layer index overflow")),
            )
        })
        .collect::<Result<_, Error>>()?;
    let patterns = writer
        .document
        .line_types
        .iter()
        .filter(|p| {
            !p.name.eq_ignore_ascii_case("BYLAYER") && !p.name.eq_ignore_ascii_case("BYBLOCK")
        })
        .enumerate()
        .filter_map(|(index, pattern)| {
            let id = writer
                .line_patterns
                .get(&pattern.name.to_ascii_lowercase())
                .copied()?;
            Some(
                u32::try_from(index)
                    .map(|index| (index, id))
                    .map_err(|_| Error::from("graphics linetype index overflow")),
            )
        })
        .collect::<Result<_, Error>>()?;
    let encoding = acadrust::io::dxf::code_page::try_encoding_from_code_page(
        &writer.document.header.code_page,
    )
    .filter(|e| e.is_faithful());
    decode_snapshot(
        data,
        &LeaderGraphicBindings::new(&writer.drawing, layers, patterns, encoding)?,
    )
    .map(Some)
}
pub(super) fn restore(
    drawing: &IfccadDocument,
    document: &super::Document,
    names: &super::Names,
    snapshot: &IfccadLeaderGraphics,
    losses: &mut super::Losses,
) -> Result<Vec<u8>, Error> {
    let layers = document
        .layers
        .iter()
        .enumerate()
        .filter_map(|(index, layer)| {
            let id = names
                .layers
                .iter()
                .find_map(|(id, name)| name.eq_ignore_ascii_case(&layer.name).then_some(*id))?;
            Some(
                u32::try_from(index)
                    .map(|index| (index, id))
                    .map_err(|_| Error::from("graphics layer index overflow")),
            )
        })
        .collect::<Result<_, Error>>()?;
    let patterns = document
        .line_types
        .iter()
        .filter(|p| {
            !p.name.eq_ignore_ascii_case("BYLAYER") && !p.name.eq_ignore_ascii_case("BYBLOCK")
        })
        .enumerate()
        .filter_map(|(index, pattern)| {
            let id = names
                .line_patterns
                .iter()
                .find_map(|(id, name)| name.eq_ignore_ascii_case(&pattern.name).then_some(*id))?;
            Some(
                u32::try_from(index)
                    .map(|index| (index, id))
                    .map_err(|_| Error::from("graphics linetype index overflow")),
            )
        })
        .collect::<Result<_, Error>>()?;
    let encoding =
        acadrust::io::dxf::code_page::try_encoding_from_code_page(&document.header.code_page)
            .filter(|e| e.is_faithful());
    let result = encode_snapshot(
        snapshot,
        &LeaderGraphicBindings::new(drawing, layers, patterns, encoding)?,
    )?;
    if snapshot.origin == IfccadLeaderGraphicsOrigin::Generated {
        losses
            .note("generated leader picture provenance becomes SourceSnapshot after CAD readback");
    }
    Ok(result)
}

pub(super) struct LeaderGraphicBindings<'a> {
    drawing: &'a IfccadDocument,
    layers: BTreeMap<u32, u64>,
    line_patterns: BTreeMap<u32, IfccadLinePatternId>,
    layer_indices: BTreeMap<u64, u32>,
    line_pattern_indices: BTreeMap<IfccadLinePatternId, u32>,
    encoding: Option<LegacyEncoding>,
}

fn inverse<T: Copy + Ord>(values: &BTreeMap<u32, T>) -> Result<BTreeMap<T, u32>, Error> {
    let mut result = BTreeMap::new();
    for (index, value) in values {
        if result.insert(*value, *index).is_some() {
            return Err("ambiguous inverse graphics resource binding".into());
        }
    }
    Ok(result)
}

impl<'a> LeaderGraphicBindings<'a> {
    pub(super) fn new(
        drawing: &'a IfccadDocument,
        layers: BTreeMap<u32, u64>,
        line_patterns: BTreeMap<u32, IfccadLinePatternId>,
        encoding: Option<LegacyEncoding>,
    ) -> Result<Self, Error> {
        if encoding.is_some_and(|encoding| !encoding.is_faithful()) {
            return Err("graphics require a faithful declared codepage, not a stand-in".into());
        }
        if line_patterns
            .keys()
            .any(|index| matches!(*index, 32766 | 32767))
        {
            return Err("reserved graphics linetype modes cannot bind an explicit resource".into());
        }
        for id in layers.values() {
            if !drawing.layers.iter().any(|layer| layer.id == *id) {
                return Err("graphics binding references a missing native layer".into());
            }
        }
        for id in line_patterns.values() {
            if !drawing
                .line_patterns
                .iter()
                .any(|pattern| pattern.id == *id)
            {
                return Err("graphics binding references a missing native line pattern".into());
            }
        }
        let layer_indices = inverse(&layers)?;
        let line_pattern_indices = inverse(&line_patterns)?;
        Ok(Self {
            drawing,
            layers,
            line_patterns,
            layer_indices,
            line_pattern_indices,
            encoding,
        })
    }
}

fn placement() -> IfccadPlacement {
    IfccadPlacement {
        origin: [0.; 3],
        x_axis: [1., 0., 0.],
        y_axis: [0., 1., 0.],
    }
}
fn located(index: usize, error: Error) -> Error {
    format!("leader graphics record {index}: {error}").into()
}

pub(super) fn decode_snapshot(
    data: &[u8],
    bindings: &LeaderGraphicBindings<'_>,
) -> Result<IfccadLeaderGraphics, Error> {
    let graphics = CheckedProxyGraphics::decode_checked(data)?;
    let commands = graphics
        .records
        .into_iter()
        .enumerate()
        .map(|(index, record)| {
            commands::decode(record, bindings).map_err(|error| located(index, error))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let snapshot = IfccadLeaderGraphics {
        origin: IfccadLeaderGraphicsOrigin::SourceSnapshot,
        placement: placement(),
        commands,
    };
    snapshot.validate(bindings.drawing)?;
    Ok(snapshot)
}

/// The CAD metafile has no provenance field: subsequent CAD import labels any
/// saved picture SourceSnapshot. The public converter reports that normalization
/// when exporting a native Generated picture.
pub(super) fn encode_snapshot(
    snapshot: &IfccadLeaderGraphics,
    bindings: &LeaderGraphicBindings<'_>,
) -> Result<Vec<u8>, Error> {
    snapshot.validate(bindings.drawing)?;
    if snapshot.placement != placement() {
        return Err(
            "nonidentity native picture placement has no qualified exact CAD mapping".into(),
        );
    }
    let records = snapshot
        .commands
        .iter()
        .enumerate()
        .map(|(index, command)| {
            commands::encode(command, bindings).map_err(|error| located(index, error))
        })
        .collect::<Result<Vec<CheckedProxyGraphicRecord>, Error>>()?;
    Ok(CheckedProxyGraphics { records }.encode_checked()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn drawing() -> IfccadDocument {
        let mut drawing = load_ifccad_bytes(super::super::TEMPLATE.as_bytes(), Default::default())
            .unwrap()
            .into_document();
        drawing.layers.push(IfccadLayer {
            id: 37,
            name: "0".into(),
            description: None,
            visible: true,
            frozen: false,
            locked: false,
            plottable: true,
            frozen_in_new_viewports: false,
            appearance: IfccadLayerAppearance {
                color: IfccadColor::Foreground,
                opacity: 1.,
                line_pattern: drawing.line_patterns[0].id,
                line_weight: 0.25,
            },
        });
        drawing.id_counters.next_layer_id = 38;
        validate_ifccad_document(&drawing).unwrap();
        drawing
    }

    fn bindings(drawing: &IfccadDocument) -> LeaderGraphicBindings<'_> {
        LeaderGraphicBindings::new(
            drawing,
            BTreeMap::from([(0, 37)]),
            BTreeMap::new(),
            acadrust::io::dxf::code_page::try_encoding_from_code_page("ANSI_1252"),
        )
        .unwrap()
    }

    #[test]
    fn complete_source_commands_rebind_resources_and_roundtrip() {
        let drawing = drawing();
        let bindings = bindings(&drawing);
        let bytes = include_bytes!("../../tests/fixtures/leaders/sample_ac1032_mleader_b1e.bin");
        let snapshot = decode_snapshot(bytes, &bindings).unwrap();
        snapshot.validate(&drawing).unwrap();
        assert!(snapshot
            .commands
            .iter()
            .any(|command| matches!(command, IfccadLeaderGraphicCommand::SetLayer(Some(37)))));
        assert!(snapshot
            .commands
            .iter()
            .any(|command| matches!(command, IfccadLeaderGraphicCommand::CadCompatibility51Zero)));
        let encoded = encode_snapshot(&snapshot, &bindings).unwrap();
        assert_eq!(decode_snapshot(&encoded, &bindings).unwrap(), snapshot);
    }

    #[test]
    fn unknown_record_never_returns_partial_picture() {
        let drawing = drawing();
        let bindings = bindings(&drawing);
        let bytes = [16u32, 1, 8, 9001]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        let error = decode_snapshot(&bytes, &bindings).unwrap_err().to_string();
        assert!(error.contains("9001"), "{error}");
        assert!(error.contains("record 0"), "{error}");
    }

    #[test]
    fn source_resource_index_is_not_a_native_id() {
        let drawing = drawing();
        let bindings =
            LeaderGraphicBindings::new(&drawing, BTreeMap::new(), BTreeMap::new(), None).unwrap();
        let bytes = [20u32, 1, 12, 16, 0]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        assert!(decode_snapshot(&bytes, &bindings).is_err());
    }

    #[test]
    fn matrix_layout_transposes_only_at_the_boundary() {
        let drawing = drawing();
        let bindings = bindings(&drawing);
        let source = [
            1., 0., 0., 12., 0., 1., 0., 13., 0., 0., 1., 14., 0., 0., 0., 1.,
        ];
        let graphics = acadrust::CheckedProxyGraphics {
            records: vec![
                acadrust::CheckedProxyGraphicRecord::Command(
                    acadrust::ProxyGraphicCommand::PushTransform(source),
                ),
                acadrust::CheckedProxyGraphicRecord::Command(
                    acadrust::ProxyGraphicCommand::PopTransform,
                ),
            ],
        };
        let snapshot = decode_snapshot(&graphics.encode_checked().unwrap(), &bindings).unwrap();
        let IfccadLeaderGraphicCommand::PushTransform(matrix) = snapshot.commands[0] else {
            panic!("missing transform")
        };
        assert_eq!([matrix[12], matrix[13], matrix[14]], [12., 13., 14.]);
        assert_eq!(
            decode_snapshot(&encode_snapshot(&snapshot, &bindings).unwrap(), &bindings).unwrap(),
            snapshot
        );
    }

    #[test]
    fn legacy_requests_do_not_invent_font_defaults_or_replacement_text() {
        use acadrust::{ProxyGraphicCommand as C, ProxyLegacyText, ProxyLegacyTextStyle, Vector3};
        let drawing = drawing();
        let bindings = bindings(&drawing);
        let raw = CheckedProxyGraphics {
            records: vec![CheckedProxyGraphicRecord::Command(C::LegacyText(Box::new(
                ProxyLegacyText {
                    position: Vector3::ZERO,
                    normal: Vector3::new(0., 0., 1.),
                    direction: Vector3::new(1., 0., 0.),
                    text: vec![0x80],
                    height: 2.,
                    width_factor: 1.,
                    oblique_angle: 0.,
                    styling: Some(ProxyLegacyTextStyle {
                        declared_length: 1,
                        processing_mode: 1,
                        tracking: 1.,
                        text_flags: [false, false, true, false, false],
                        font_file: vec![],
                        big_font_file: vec![],
                    }),
                },
            )))],
        };
        let mut snapshot = decode_snapshot(&raw.encode_checked().unwrap(), &bindings).unwrap();
        let IfccadLeaderGraphicCommand::Draw(IfccadLeaderGraphicPrimitive::Text(text)) =
            &snapshot.commands[0]
        else {
            panic!("missing graphic text")
        };
        assert_eq!(text.text, "€");
        let font = text.font.as_ref().unwrap();
        assert!(font.font_file.is_empty());
        assert_eq!(font.bold, None);
        assert_eq!(font.charset, None);
        assert_eq!(
            decode_snapshot(&encode_snapshot(&snapshot, &bindings).unwrap(), &bindings).unwrap(),
            snapshot
        );
        let IfccadLeaderGraphicCommand::Draw(IfccadLeaderGraphicPrimitive::Text(text)) =
            &mut snapshot.commands[0]
        else {
            unreachable!()
        };
        text.text = "🙂".into();
        assert!(
            encode_snapshot(&snapshot, &bindings).is_err(),
            "unmappable text must not become replacement chars or an escape literal"
        );
    }

    #[test]
    fn unsupported_native_owner_reset_fails_before_emitting_a_picture() {
        let drawing = drawing();
        let bindings = bindings(&drawing);
        let snapshot = IfccadLeaderGraphics {
            origin: IfccadLeaderGraphicsOrigin::Generated,
            placement: placement(),
            commands: vec![IfccadLeaderGraphicCommand::SetColor(
                IfccadLeaderGraphicInheritance::Owner,
            )],
        };
        let error = encode_snapshot(&snapshot, &bindings)
            .unwrap_err()
            .to_string();
        assert!(error.contains("record 0"), "{error}");
        assert!(error.contains("owner"), "{error}");
    }
}
