//! Authored font selectors and qualified STYLE descriptor fields; no file lookup.
use super::*;
use acadrust::tables::TextStyle;

const BOLD: u32 = 0x0200_0000;
const ITALIC: u32 = 0x0100_0000;
const PITCH_FAMILY: u32 = 0xf3;
const KNOWN: u32 = BOLD | ITALIC | 0xff00 | PITCH_FAMILY;

pub(super) fn preflight_font_requests(
    drawing: &IfccadDocument,
    losses: &mut Losses,
) -> Result<(), Error> {
    use ifccad::text::MTextInline;
    use ifccad::{IfccadDimensionGraphics, IfccadDimensionGraphicsKind, IfccadMText};
    fn mtext(text: &IfccadMText, path: &str, losses: &mut Losses) -> Result<(), Error> {
        let mut authored = Vec::new();
        let mut emitted = Vec::new();
        let global = text.character_format.font.as_ref();
        if let Some(font) = global {
            authored.push((font, format!("{path}/characterFormat/font")));
        }
        for (p, paragraph) in text.content.iter().enumerate() {
            let parent = paragraph.character_format.font.as_ref().or(global);
            if let Some(font) = paragraph.character_format.font.as_ref() {
                authored.push((font, format!("{path}/content/{p}/characterFormat/font")));
            }
            for (i, inline) in paragraph.inlines.iter().enumerate() {
                let format = match inline {
                    MTextInline::Run {
                        character_format, ..
                    } => character_format,
                    MTextInline::Stack(stack) => &stack.character_format,
                    _ => continue,
                };
                if let Some(font) = format.font.as_ref() {
                    authored.push((
                        font,
                        format!("{path}/content/{p}/inlines/{i}/characterFormat/font"),
                    ));
                }
                if let Some(font) = format.font.as_ref().or(parent) {
                    emitted.push(font);
                }
            }
        }
        for (font, location) in authored {
            let active = emitted.iter().any(|other| std::ptr::eq(*other, font));
            let (field, selected) = if let Some(family) = &font.family {
                ("family", family)
            } else if let Some(name) = &font.cad_font_name {
                ("cadFontName", name)
            } else {
                continue;
            };
            if active && selected.contains(['|', ';', '\\', '{', '}']) {
                return Err(format!("{location}/{field}: unsupported MText font request cannot be written without changing its name").into());
            }
            if !active {
                losses.note(format!(
                    "{location}: authored dormant font override has no emitted MText scope"
                ));
            }
            if font.family.is_some() && font.cad_font_name.is_some() {
                losses.note(format!("{location}/cadFontName: additional selector has no MText grammar field; family retained"));
            }
            if font.big_font_name.is_some() {
                losses.note(format!(
                    "{location}/bigFontName: selector has no MText grammar field"
                ));
            }
        }
        Ok(())
    }
    fn graphics(
        picture: &IfccadDimensionGraphics,
        path: &str,
        losses: &mut Losses,
    ) -> Result<(), Error> {
        for (index, part) in picture.parts.iter().enumerate() {
            if let IfccadDimensionGraphicsKind::MText(text) = &part.kind {
                mtext(text, &format!("{path}/parts/{index}/mText"), losses)?;
            }
        }
        Ok(())
    }
    for entity in drawing
        .model
        .entities
        .iter()
        .chain(
            drawing
                .paper_layouts
                .iter()
                .flat_map(|layout| &layout.entities),
        )
        .chain(drawing.blocks.iter().flat_map(|block| &block.entities))
    {
        let IfccadEntity::Native(entity) = entity else {
            continue;
        };
        let path = format!("/cad/d{}/e{}", drawing.drawing_id, entity.id);
        match &entity.kind {
            IfccadEntityKind::MText(text) => mtext(text, &format!("{path}/ifccad::mText"), losses)?,
            IfccadEntityKind::Dimension(dimension) => {
                if let Some(picture) = &dimension.graphics {
                    graphics(picture, &format!("{path}/graphics"), losses)?;
                }
                if let Some(annotation) = &dimension.annotation {
                    for (index, context) in annotation.contexts.iter().enumerate() {
                        if let Some(picture) = context
                            .presentation
                            .as_ref()
                            .and_then(|p| p.graphics.as_ref())
                        {
                            graphics(
                                picture,
                                &format!("{path}/annotation/contexts/{index}/graphics"),
                                losses,
                            )?;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn font_from_cad_style(
    style: &TextStyle,
    location: &str,
    losses: &mut Losses,
) -> Result<FontRequest, Error> {
    let optional = |value: &String| (!value.is_empty()).then(|| value.clone());
    let mut font = FontRequest {
        family: optional(&style.true_type_font),
        cad_font_name: optional(&style.font_file),
        big_font_name: optional(&style.big_font_file),
        ..Default::default()
    };
    let flags = style.true_type_font_flags as u32;
    if font.family.is_some() {
        font.bold = Some(flags & BOLD != 0);
        font.italic = Some(flags & ITALIC != 0);
        font.charset = Some(((flags >> 8) & 0xff) as u16);
        font.pitch = Some((flags & PITCH_FAMILY) as u8);
        if flags & !KNOWN != 0 {
            losses.note(format!(
                "{location}: unknown TrueType flag bits {:#x}",
                flags & !KNOWN
            ));
        }
    } else if style.true_type_font_flags != 34 {
        losses.note(format!("{location}: TrueType descriptor without a family"));
    }
    Ok(font)
}

pub(super) fn font_to_cad_style(
    font: &FontRequest,
    style: &mut TextStyle,
    location: &str,
    losses: &mut Losses,
) -> Result<(), Error> {
    if font.charset.is_some_and(|value| value > 255) {
        return Err(format!("{location}/charset: unsupported STYLE charset outside 0..255").into());
    }
    if font
        .pitch
        .is_some_and(|value| u32::from(value) & !PITCH_FAMILY != 0)
    {
        return Err(format!("{location}/pitch: unsupported STYLE pitch/family bits").into());
    }
    if font.family.as_ref().is_some_and(|name| name.trim() != name) {
        return Err(
            format!("{location}/family: codec cannot retain surrounding whitespace").into(),
        );
    }
    if font.family.is_none()
        && (font.bold.is_some()
            || font.italic.is_some()
            || font.charset.is_some()
            || font.pitch.is_some())
    {
        return Err(format!(
            "{location}: STYLE descriptor requires a family for physical retention"
        )
        .into());
    }
    style.font_file = font.cad_font_name.clone().unwrap_or_default();
    style.big_font_file = font.big_font_name.clone().unwrap_or_default();
    style.true_type_font = font.family.clone().unwrap_or_default();
    style.true_type_font_flags = ((if font.bold.unwrap_or(false) { BOLD } else { 0 })
        | (if font.italic.unwrap_or(false) {
            ITALIC
        } else {
            0
        })
        | (u32::from(font.charset.unwrap_or(0)) << 8)
        | u32::from(font.pitch.unwrap_or(34))) as i32;
    if font.family.is_some()
        && (font.bold.is_none()
            || font.italic.is_none()
            || font.charset.is_none()
            || font.pitch.is_none())
    {
        losses.retained_note(Loss { handle: None, message:format!("{location}: unspecified STYLE face fields materialized as bold=false, italic=false, charset=0, pitch=34 where absent; request names retained") });
    }
    Ok(())
}
