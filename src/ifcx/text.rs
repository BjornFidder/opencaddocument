//! Text content: TEXT values and MTEXT markup to and from IFCX's runs and
//! paragraphs, and the MTEXT entity around them.
//!
//! The markup grammar follows the cad-text crate of ifccad (MPL-2.0), read
//! leniently: a code this reader does not know stays in the text instead of
//! failing the drawing.

use acadrust::entities::{AttachmentPoint, DrawingDirection, LineSpacingStyle, MText};
use ifccad::text::{
    validate_background, validate_font_request, validate_mtext_columns,
    validate_mtext_effective_formats, validate_paragraph_format, CharacterFormat, MTextAttachment,
    MTextBackground, MTextColumnHeight, MTextColumns, MTextFill, MTextFlow, MTextInline,
    MTextParagraph, MTextValueContext, ParagraphFormat, TextColor, TextHeight, TextLineSpacing,
    TextPadding, TextParagraphAlignment, TextPosition, TextStack, TextStackKind, TextTabAlignment,
    TextTabStop,
};
use ifccad::IfccadMText;

use super::*;

type Format = CharacterFormat<IfccadColor>;
type Paragraph = MTextParagraph<IfccadColor>;

/// MTEXT line spacing is a factor of this many text heights.
const LINE_PITCH: f64 = 5.0 / 3.0;

// ---------------------------------------------------------------- TEXT

/// A TEXT value as runs: `%%u`, `%%o` and `%%k` toggle decorations and
/// `%%d`, `%%p`, `%%c`, `%%%` and `%%nnn` are characters.
pub(super) fn text_runs(value: &str) -> Vec<TextRun> {
    let chars: Vec<char> = value.chars().collect();
    let mut runs: Vec<TextRun> = Vec::new();
    let mut decorations = [false; 3];
    let mut i = 0;
    while i < chars.len() {
        let mut c = chars[i];
        i += 1;
        if c == '%' && chars.get(i) == Some(&'%') {
            match percent_code(&chars, i + 1) {
                Some((Percent::Toggle(index), next)) => {
                    decorations[index] = !decorations[index];
                    i = next;
                    continue;
                }
                Some((Percent::Char(value), next)) => {
                    c = value;
                    i = next;
                }
                None => {}
            }
        }
        if c.is_control() {
            continue;
        }
        let [underline, overline, strike_through] = decorations;
        match runs.last_mut() {
            Some(run) if [run.underline, run.overline, run.strike_through] == decorations => {
                run.text.push(c)
            }
            _ => runs.push(TextRun {
                text: c.to_string(),
                underline,
                overline,
                strike_through,
            }),
        }
    }
    runs
}

/// Runs as a TEXT value.
pub(super) fn text_value(runs: &[TextRun]) -> String {
    let mut out = String::new();
    let mut decorations = [false; 3];
    for run in runs {
        let wanted = [run.underline, run.overline, run.strike_through];
        for (index, code) in ["%%u", "%%o", "%%k"].into_iter().enumerate() {
            if decorations[index] != wanted[index] {
                out.push_str(code);
                decorations[index] = wanted[index];
            }
        }
        push_percent_safe(&mut out, &run.text, |out, c| out.push(c));
    }
    out
}

enum Percent {
    Toggle(usize),
    Char(char),
}

/// The `%%` code whose letter is at `at`, and the index after it.
fn percent_code(chars: &[char], at: usize) -> Option<(Percent, usize)> {
    let code = match chars.get(at)? {
        'u' | 'U' => Percent::Toggle(0),
        'o' | 'O' => Percent::Toggle(1),
        'k' | 'K' => Percent::Toggle(2),
        'd' | 'D' => Percent::Char('°'),
        'p' | 'P' => Percent::Char('±'),
        'c' | 'C' => Percent::Char('Ø'),
        '%' => Percent::Char('%'),
        digit if digit.is_ascii_digit() => {
            let digits: String = chars.get(at..at + 3)?.iter().collect();
            let value = digits.parse::<u32>().ok()?;
            return Some((Percent::Char(char::from_u32(value)?), at + 3));
        }
        _ => return None,
    };
    Some((code, at + 1))
}

/// `text` with every `%` that could start a `%%` code written as `%%%`.
fn push_percent_safe(out: &mut String, text: &str, mut push: impl FnMut(&mut String, char)) {
    let chars: Vec<char> = text.chars().collect();
    for (index, c) in chars.iter().enumerate() {
        match c {
            '%' if matches!(chars.get(index + 1), Some('%') | None) => out.push_str("%%%"),
            c if c.is_control() => {}
            c => push(out, *c),
        }
    }
}

// ---------------------------------------------------------------- MTEXT markup

/// MTEXT markup as paragraphs. Every run carries the whole formatting in
/// force where it stands; paragraph properties carry over to the next
/// paragraph until changed.
pub(super) fn parse_mtext(value: &str) -> Vec<Paragraph> {
    let s: Vec<char> = value.chars().collect();
    let mut paragraphs = vec![Paragraph::default()];
    let mut format = Format::default();
    let mut scopes: Vec<Format> = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        i += 1;
        match c {
            '{' => scopes.push(format.clone()),
            '}' => {
                if let Some(outer) = scopes.pop() {
                    format = outer;
                }
            }
            '\\' if i < s.len() => {
                let code = s[i];
                i += 1;
                match code {
                    '\\' | '{' | '}' => literal(&mut paragraphs, &format, code),
                    '~' => literal(&mut paragraphs, &format, '\u{a0}'),
                    'P' => new_paragraph(&mut paragraphs),
                    'N' => current(&mut paragraphs)
                        .inlines
                        .push(MTextInline::ColumnBreak),
                    'L' | 'l' => format.underline = Some(code == 'L'),
                    'O' | 'o' => format.overline = Some(code == 'O'),
                    'K' | 'k' => format.strike_through = Some(code == 'K'),
                    'U' if s.get(i) == Some(&'+') => match unicode(&s, i + 1) {
                        Some((value, next)) => {
                            literal(&mut paragraphs, &format, value);
                            i = next;
                        }
                        None => literal(&mut paragraphs, &format, 'U'),
                    },
                    'H' | 'W' | 'w' | 'T' | 'Q' | 'C' | 'c' | 'A' | 'f' | 'F' | 'p' | 'S' => {
                        let Some((body, next)) = code_body(&s, i) else {
                            literal(&mut paragraphs, &format, '\\');
                            literal(&mut paragraphs, &format, code);
                            continue;
                        };
                        i = next;
                        match code {
                            'p' => parse_paragraph(
                                &body,
                                &mut current(&mut paragraphs).paragraph_format,
                            ),
                            'S' => stack(&body, &format, current(&mut paragraphs)),
                            _ => apply_code(code, &body, &mut format),
                        }
                    }
                    other => {
                        literal(&mut paragraphs, &format, '\\');
                        literal(&mut paragraphs, &format, other);
                    }
                }
            }
            '%' if s.get(i) == Some(&'%') => match percent_code(&s, i + 1) {
                Some((Percent::Toggle(index), next)) => {
                    let decoration = match index {
                        0 => &mut format.underline,
                        1 => &mut format.overline,
                        _ => &mut format.strike_through,
                    };
                    *decoration = Some(!decoration.unwrap_or(false));
                    i = next;
                }
                Some((Percent::Char(value), next)) => {
                    literal(&mut paragraphs, &format, value);
                    i = next;
                }
                None => literal(&mut paragraphs, &format, '%'),
            },
            '^' if matches!(s.get(i), Some('I' | 'J' | 'M')) => {
                match s[i] {
                    'I' => current(&mut paragraphs).inlines.push(MTextInline::Tab),
                    'J' => new_paragraph(&mut paragraphs),
                    _ => {}
                }
                i += 1;
            }
            '\t' => current(&mut paragraphs).inlines.push(MTextInline::Tab),
            '\n' => new_paragraph(&mut paragraphs),
            '\r' => {
                if s.get(i) == Some(&'\n') {
                    i += 1;
                }
                new_paragraph(&mut paragraphs);
            }
            c if c.is_control() => {}
            c => literal(&mut paragraphs, &format, c),
        }
    }
    paragraphs
}

fn current(paragraphs: &mut [Paragraph]) -> &mut Paragraph {
    paragraphs.last_mut().expect("there is always a paragraph")
}

fn new_paragraph(paragraphs: &mut Vec<Paragraph>) {
    let paragraph_format = current(paragraphs).paragraph_format.clone();
    paragraphs.push(Paragraph {
        paragraph_format,
        ..Default::default()
    });
}

fn literal(paragraphs: &mut [Paragraph], format: &Format, c: char) {
    let inlines = &mut current(paragraphs).inlines;
    if let Some(MTextInline::Run {
        text,
        character_format,
    }) = inlines.last_mut()
    {
        if character_format == format {
            text.push(c);
            return;
        }
    }
    inlines.push(MTextInline::Run {
        text: c.to_string(),
        character_format: format.clone(),
    });
}

/// The body of a `\X...;` code starting at `at` (escapes resolved), and the
/// index after its `;`.
fn code_body(s: &[char], at: usize) -> Option<(String, usize)> {
    let mut body = String::new();
    let mut i = at;
    while i < s.len() {
        match s[i] {
            ';' => return Some((body, i + 1)),
            '\\' if matches!(s.get(i + 1), Some(';' | '\\' | '{' | '}')) => {
                body.push(s[i + 1]);
                i += 2;
            }
            c if c.is_control() => return None,
            c => {
                body.push(c);
                i += 1;
            }
        }
    }
    None
}

/// A `\U+XXXX` escape whose hex digits start at `at` (surrogate pairs
/// joined), and the index after it.
fn unicode(s: &[char], at: usize) -> Option<(char, usize)> {
    let unit = |at: usize| -> Option<u32> {
        let digits: String = s.get(at..at + 4)?.iter().collect();
        u32::from_str_radix(&digits, 16).ok()
    };
    let high = unit(at)?;
    if (0xd800..=0xdbff).contains(&high) {
        let rest: String = s.get(at + 4..at + 7)?.iter().collect();
        let low = unit(at + 7).filter(|low| rest == "\\U+" && (0xdc00..=0xdfff).contains(low))?;
        let value = 0x10000 + ((high - 0xd800) << 10) + (low - 0xdc00);
        return Some((char::from_u32(value)?, at + 11));
    }
    Some((char::from_u32(high)?, at + 4))
}

fn apply_code(code: char, body: &str, format: &mut Format) {
    let number = |body: &str| body.parse::<f64>().ok().filter(|v| v.is_finite());
    let scalar = |body: &str| -> Option<(f64, bool)> {
        match body.strip_suffix('x') {
            Some(factor) => number(factor).map(|v| (v, true)),
            None => number(body).map(|v| (v, false)),
        }
    };
    match code {
        'H' => {
            if let Some((value, relative)) = scalar(body).filter(|(v, _)| *v > 0.0) {
                format.height = Some(match (relative, format.height) {
                    (false, _) => TextHeight::Absolute { distance: value },
                    (true, Some(TextHeight::Absolute { distance })) => TextHeight::Absolute {
                        distance: distance * value,
                    },
                    (true, Some(TextHeight::Relative { factor })) => TextHeight::Relative {
                        factor: factor * value,
                    },
                    (true, None) => TextHeight::Relative { factor: value },
                });
            }
        }
        'W' | 'w' | 'T' => {
            if let Some((value, relative)) = scalar(body).filter(|(v, _)| *v > 0.0) {
                let field = match code {
                    'T' => &mut format.tracking,
                    _ => &mut format.width_factor,
                };
                *field = Some(match relative {
                    true => field.unwrap_or(1.0) * value,
                    false => value,
                });
            }
        }
        'Q' => {
            if let Some(degrees) = number(body).filter(|v| v.abs() < 90.0) {
                format.oblique_angle = Some(degrees.to_radians());
            }
        }
        'C' => {
            if let Ok(index) = body.parse::<u16>() {
                format.color = match index {
                    0 => Some(TextColor::ByBlock),
                    256 => Some(TextColor::ByLayer),
                    1..=255 => color_out(Color::Index(index as u8)).map(TextColor::Explicit),
                    _ => return,
                };
            }
        }
        'c' => {
            if let Ok(value) = body.parse::<u32>().map(|v| v & 0xff_ffff) {
                format.color = Some(TextColor::Explicit(IfccadColor::Concrete {
                    rgb: [value as u8, (value >> 8) as u8, (value >> 16) as u8],
                    indexed: None,
                    named: None,
                }));
            }
        }
        'A' => {
            format.position = match body {
                "0" => Some(TextPosition::Bottom),
                "1" => Some(TextPosition::Center),
                "2" => Some(TextPosition::Top),
                _ => format.position,
            };
        }
        'f' | 'F' => {
            let mut parts = body.split('|');
            let name = parts.next().unwrap_or("").to_string();
            let mut font = match code {
                'F' => FontRequest {
                    cad_font_name: Some(name),
                    ..Default::default()
                },
                _ => FontRequest::family(name),
            };
            for part in parts {
                let (flag, value) = part.split_at(part.len().min(1));
                match flag {
                    "b" => font.bold = Some(value == "1"),
                    "i" => font.italic = Some(value == "1"),
                    "c" => font.charset = value.parse().ok(),
                    "p" => font.pitch = value.parse().ok(),
                    _ => {}
                }
            }
            if validate_font_request(&font).is_ok() {
                format.font = Some(font);
            }
        }
        _ => {}
    }
}

/// `\S` stacked text: upper and lower parts around `/`, `#` or `^`.
fn stack(body: &str, format: &Format, paragraph: &mut Paragraph) {
    let Some(index) = body.find(['/', '#', '^']) else {
        return;
    };
    let (upper, lower) = (&body[..index], &body[index + 1..]);
    if upper.is_empty() && lower.is_empty() {
        return;
    }
    paragraph.inlines.push(MTextInline::Stack(TextStack {
        stack_kind: match &body[index..index + 1] {
            "/" => TextStackKind::Fraction,
            "#" => TextStackKind::DiagonalFraction,
            _ => TextStackKind::Tolerance,
        },
        upper: upper.to_string(),
        lower: lower.to_string(),
        character_format: format.clone(),
        ..Default::default()
    }));
}

/// `\p` paragraph properties: alignment, indents and tab stops.
fn parse_paragraph(body: &str, out: &mut ParagraphFormat) {
    let mut parsed = out.clone();
    let body = body.strip_prefix('x').unwrap_or(body);
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let key = bytes[i];
        i += 1;
        let start = i;
        let number_end = |mut i: usize| {
            while bytes.get(i).is_some_and(|c| {
                c.is_ascii_digit() || matches!(c, b'.' | b'-' | b'+' | b'e' | b'E')
            }) {
                i += 1;
            }
            i
        };
        match key {
            b',' => continue,
            b'q' => {
                parsed.alignment = match bytes.get(i) {
                    Some(b'l') => Some(TextParagraphAlignment::Left),
                    Some(b'c') => Some(TextParagraphAlignment::Center),
                    Some(b'r') => Some(TextParagraphAlignment::Right),
                    Some(b'j' | b'*') => Some(TextParagraphAlignment::Justified),
                    Some(b'd') => Some(TextParagraphAlignment::Distributed),
                    _ => parsed.alignment,
                };
                i += 1;
            }
            b'i' | b'l' | b'r' => {
                i = number_end(i);
                if let Ok(value) = body[start..i].parse::<f64>() {
                    *match key {
                        b'i' => &mut parsed.first_line_indent_factor,
                        b'l' => &mut parsed.left_indent_factor,
                        _ => &mut parsed.right_indent_factor,
                    } = Some(value);
                }
            }
            b't' => {
                let stops = body[i..]
                    .split(',')
                    .filter(|part| !part.is_empty())
                    .filter_map(|part| {
                        let (alignment, number) = match part.as_bytes()[0] {
                            b'c' | b'C' => (TextTabAlignment::Center, &part[1..]),
                            b'r' | b'R' => (TextTabAlignment::Right, &part[1..]),
                            _ => (TextTabAlignment::Left, part),
                        };
                        let position_factor = number.parse::<f64>().ok()?;
                        Some(TextTabStop {
                            position_factor,
                            alignment,
                        })
                    })
                    .collect();
                parsed.tab_stops = Some(stops);
                i = bytes.len();
            }
            // Spacing keys and anything unknown: skip the value.
            _ => {
                while bytes.get(i).is_some_and(|c| *c != b',') {
                    i += 1;
                }
            }
        }
    }
    if validate_paragraph_format(&parsed).is_ok() {
        *out = parsed;
    }
}

/// Paragraphs as MTEXT markup, formatting folded from `global` (the whole
/// text's) and each paragraph's into every run.
pub(super) fn emit_mtext(
    global: &Format,
    global_paragraph: &ParagraphFormat,
    paragraphs: &[Paragraph],
) -> String {
    let mut out = String::new();
    let mut previous = ParagraphFormat::default();
    for (index, paragraph) in paragraphs.iter().enumerate() {
        if index > 0 {
            out.push_str("\\P");
        }
        let mut paragraph_format = merge_paragraph(global_paragraph, &paragraph.paragraph_format);
        paragraph_format.line_spacing = None;
        if paragraph_format != previous {
            out.push_str(&paragraph_code(&paragraph_format));
            previous = paragraph_format;
        }
        let base = merge_format(global, &paragraph.character_format);
        // Neighbours with the same formatting share one scope.
        let mut scope = Scope::default();
        for inline in &paragraph.inlines {
            match inline {
                MTextInline::Run {
                    text,
                    character_format,
                } => {
                    let mut body = String::new();
                    push_percent_safe(&mut body, text, |out, c| match c {
                        '\\' => out.push_str("\\\\"),
                        '{' => out.push_str("\\{"),
                        '}' => out.push_str("\\}"),
                        '\u{a0}' => out.push_str("\\~"),
                        // A caret could read as a control with what follows.
                        '^' => out.push_str("{^}"),
                        c => out.push(c),
                    });
                    scope.enter(&mut out, merge_format(&base, character_format));
                    out.push_str(&body);
                }
                MTextInline::Tab => out.push_str("^I"),
                MTextInline::ColumnBreak => out.push_str("\\N"),
                MTextInline::LineBreak => out.push_str("\\P"),
                MTextInline::Stack(stack) => {
                    let separator = match stack.stack_kind {
                        TextStackKind::Fraction => '/',
                        TextStackKind::DiagonalFraction => '#',
                        TextStackKind::Tolerance | TextStackKind::DecimalTolerance => '^',
                    };
                    let part = |text: &str| {
                        text.chars()
                            .filter(|c| !c.is_control())
                            .flat_map(|c| match c {
                                ';' | '\\' | '{' | '}' => vec!['\\', c],
                                c => vec![c],
                            })
                            .collect::<String>()
                    };
                    let body = format!(
                        "\\S{}{separator}{};",
                        part(&stack.upper),
                        part(&stack.lower)
                    );
                    scope.enter(&mut out, merge_format(&base, &stack.character_format));
                    out.push_str(&body);
                }
            }
        }
        scope.enter(&mut out, Format::default());
    }
    out
}

/// The formatting scope open in the markup being written.
#[derive(Default)]
struct Scope {
    format: Format,
    open: bool,
}

impl Scope {
    /// Continue in `format`: close the open scope and open one with its
    /// codes, unless it is already in force.
    fn enter(&mut self, out: &mut String, format: Format) {
        if format == self.format {
            return;
        }
        if self.open {
            out.push('}');
        }
        let codes = format_codes(&format);
        self.open = !codes.is_empty();
        if self.open {
            out.push('{');
            out.push_str(&codes);
        }
        self.format = format;
    }
}

fn format_codes(format: &Format) -> String {
    let mut out = String::new();
    if let Some(font) = &format.font {
        let (code, name) = match (&font.family, &font.cad_font_name) {
            (Some(family), _) => ("\\f", family),
            (None, Some(name)) => ("\\F", name),
            (None, None) => ("", &String::new()),
        };
        if !code.is_empty() {
            out.push_str(code);
            out.push_str(&name.replace(['|', ';', '\\', '{', '}'], ""));
            for (flag, value) in [("b", font.bold), ("i", font.italic)] {
                if let Some(value) = value {
                    out.push_str(&format!("|{flag}{}", u8::from(value)));
                }
            }
            if let Some(charset) = font.charset {
                out.push_str(&format!("|c{charset}"));
            }
            if let Some(pitch) = font.pitch {
                out.push_str(&format!("|p{pitch}"));
            }
            out.push(';');
        }
    }
    match &format.color {
        Some(TextColor::ByLayer) => out.push_str("\\C256;"),
        Some(TextColor::ByBlock) => out.push_str("\\C0;"),
        Some(TextColor::Explicit(color)) => match color_in(color) {
            Color::Index(index) => out.push_str(&format!("\\C{index};")),
            Color::Rgb { r, g, b } => out.push_str(&format!(
                "\\c{};",
                u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)
            )),
            _ => {}
        },
        Some(TextColor::Entity) | None => {}
    }
    match format.height {
        Some(TextHeight::Absolute { distance }) => out.push_str(&format!("\\H{distance};")),
        Some(TextHeight::Relative { factor }) => out.push_str(&format!("\\H{factor}x;")),
        None => {}
    }
    if let Some(width) = format.width_factor {
        out.push_str(&format!("\\W{width};"));
    }
    if let Some(tracking) = format.tracking {
        out.push_str(&format!("\\T{tracking};"));
    }
    if let Some(angle) = format.oblique_angle {
        // Degrees and radians do not convert exactly; snapped, a read and
        // a write give back the same value.
        out.push_str(&format!("\\Q{};", snap(angle.to_degrees(), 1e-9)));
    }
    for (value, on, off) in [
        (format.underline, "\\L", "\\l"),
        (format.overline, "\\O", "\\o"),
        (format.strike_through, "\\K", "\\k"),
    ] {
        if let Some(value) = value {
            out.push_str(if value { on } else { off });
        }
    }
    match format.position {
        Some(TextPosition::Bottom) => out.push_str("\\A0;"),
        Some(TextPosition::Center) => out.push_str("\\A1;"),
        Some(TextPosition::Top) => out.push_str("\\A2;"),
        None => {}
    }
    out
}

/// `\p` code setting every paragraph property, so that it replaces what an
/// earlier paragraph set.
fn paragraph_code(format: &ParagraphFormat) -> String {
    let alignment = match format.alignment.unwrap_or_default() {
        TextParagraphAlignment::Left => 'l',
        TextParagraphAlignment::Center => 'c',
        TextParagraphAlignment::Right => 'r',
        TextParagraphAlignment::Justified => 'j',
        TextParagraphAlignment::Distributed => 'd',
    };
    let stops: Vec<String> = format
        .tab_stops
        .iter()
        .flatten()
        .map(|stop| {
            let prefix = match stop.alignment {
                TextTabAlignment::Center => "c",
                TextTabAlignment::Right => "r",
                TextTabAlignment::Left | TextTabAlignment::Decimal { .. } => "",
            };
            format!("{prefix}{}", stop.position_factor)
        })
        .collect();
    format!(
        "\\pxq{alignment},i{},l{},r{},t{};",
        format.first_line_indent_factor.unwrap_or(0.0),
        format.left_indent_factor.unwrap_or(0.0),
        format.right_indent_factor.unwrap_or(0.0),
        stops.join(",")
    )
}

fn merge_format(parent: &Format, child: &Format) -> Format {
    let mut out = parent.clone();
    macro_rules! replace {
        ($($field:ident),+) => { $(if child.$field.is_some() { out.$field = child.$field.clone(); })+ };
    }
    replace!(
        font,
        color,
        height,
        width_factor,
        tracking,
        oblique_angle,
        underline,
        overline,
        strike_through,
        position
    );
    out
}

fn merge_paragraph(parent: &ParagraphFormat, child: &ParagraphFormat) -> ParagraphFormat {
    let mut out = parent.clone();
    macro_rules! replace {
        ($($field:ident),+) => { $(if child.$field.is_some() { out.$field = child.$field.clone(); })+ };
    }
    replace!(
        alignment,
        left_indent_factor,
        right_indent_factor,
        first_line_indent_factor,
        space_before,
        space_after,
        line_spacing,
        tab_stops
    );
    out
}

// ---------------------------------------------------------------- MTEXT entity

pub(super) fn mtext_out(writer: &mut Writer, mtext: &MText) -> Option<IfccadEntityKind> {
    let style_id = *writer.styles.get(&mtext.style.to_ascii_lowercase())?;
    if mtext.height.is_nan() || mtext.height <= 0.0 {
        return None;
    }
    let direction = mtext
        .dwg_x_direction
        .unwrap_or_else(|| Vector3::new(mtext.rotation.cos(), mtext.rotation.sin(), 0.0));
    let direction = to_ocs(direction, mtext.normal);
    let placement = placement_out(
        to_ocs(mtext.insertion_point, mtext.normal),
        mtext.normal,
        direction.y.atan2(direction.x),
    );
    let columns = match mtext.column_data.column_type {
        0 => None,
        _ => match columns_out(mtext) {
            Some(columns) => Some(columns),
            None => {
                writer.losses.note("MText columns");
                None
            }
        },
    };
    let mut content = parse_mtext(&mtext.value);
    if columns.is_none() {
        for paragraph in &mut content {
            paragraph
                .inlines
                .retain(|inline| !matches!(inline, MTextInline::ColumnBreak));
        }
    }
    let paragraph_format = ParagraphFormat {
        line_spacing: spacing_out(mtext),
        ..Default::default()
    };
    let background = background_out(mtext, &mut writer.losses);
    let mut converted = IfccadMText {
        style_id,
        placement,
        rotation: 0.0,
        backward: false,
        upside_down: false,
        height: mtext.height,
        attachment: match mtext.attachment_point {
            AttachmentPoint::TopLeft => MTextAttachment::TopLeft,
            AttachmentPoint::TopCenter => MTextAttachment::TopCenter,
            AttachmentPoint::TopRight => MTextAttachment::TopRight,
            AttachmentPoint::MiddleLeft => MTextAttachment::MiddleLeft,
            AttachmentPoint::MiddleCenter => MTextAttachment::MiddleCenter,
            AttachmentPoint::MiddleRight => MTextAttachment::MiddleRight,
            AttachmentPoint::BottomLeft => MTextAttachment::BottomLeft,
            AttachmentPoint::BottomCenter => MTextAttachment::BottomCenter,
            AttachmentPoint::BottomRight => MTextAttachment::BottomRight,
        },
        flow: match mtext.drawing_direction {
            DrawingDirection::LeftToRight => MTextFlow::Horizontal,
            DrawingDirection::TopToBottom => MTextFlow::Vertical,
            DrawingDirection::ByStyle => MTextFlow::ByStyle,
        },
        wrap_width: (columns.is_none() && mtext.rectangle_width > 0.0)
            .then_some(mtext.rectangle_width),
        columns,
        background,
        character_format: Format::default(),
        paragraph_format,
        content,
    };
    if mtext_is_valid(writer, &converted) {
        return Some(IfccadEntityKind::MText(Box::new(converted)));
    }
    // Formatting IFCX rejects: keep the words.
    writer.losses.note("MText formatting");
    for paragraph in &mut converted.content {
        paragraph.paragraph_format = ParagraphFormat::default();
        paragraph.character_format = Format::default();
        for inline in &mut paragraph.inlines {
            match inline {
                MTextInline::Run {
                    character_format, ..
                } => *character_format = Format::default(),
                MTextInline::Stack(stack) => stack.character_format = Format::default(),
                _ => {}
            }
        }
    }
    mtext_is_valid(writer, &converted).then(|| IfccadEntityKind::MText(Box::new(converted)))
}

fn mtext_is_valid(writer: &Writer, mtext: &IfccadMText) -> bool {
    let Some(style) = writer
        .drawing
        .text_styles
        .iter()
        .find(|style| style.id == mtext.style_id)
    else {
        return false;
    };
    let context = MTextValueContext {
        nominal_height: mtext.height,
        wrap_width: mtext.wrap_width,
        columns: mtext.columns.as_ref(),
    };
    validate_paragraph_format(&mtext.paragraph_format).is_ok()
        && validate_mtext_effective_formats(
            &mtext.content,
            context,
            &style.properties,
            &mtext.character_format,
            &mtext.paragraph_format,
            |_| true,
        )
        .is_ok()
        && mtext
            .background
            .as_ref()
            .is_none_or(|background| validate_background(background, |_| true).is_ok())
}

pub(super) fn mtext_in(
    mtext: &IfccadMText,
    names: &Names,
    losses: &mut Losses,
) -> Option<EntityType> {
    let style = names.styles.get(&mtext.style_id)?;
    if mtext.backward || mtext.upside_down {
        losses.note("MText mirroring");
    }
    let (origin, normal, frame) = placement_in(&mtext.placement);
    let rotation = mtext.rotation + snap(frame, ANGLE_STEP);
    let mut converted = MText::new();
    converted.style = style.clone();
    converted.height = mtext.height;
    converted.insertion_point = vector(mtext.placement.origin);
    converted.normal = normal;
    converted.rotation = rotation;
    if normal != Vector3::new(0.0, 0.0, 1.0) {
        let (_, x, _) = Placement::from_ocs(origin, normal, rotation).to_axes();
        converted.dwg_x_direction = Some(x);
    }
    converted.attachment_point = match mtext.attachment {
        MTextAttachment::TopLeft => AttachmentPoint::TopLeft,
        MTextAttachment::TopCenter => AttachmentPoint::TopCenter,
        MTextAttachment::TopRight => AttachmentPoint::TopRight,
        MTextAttachment::MiddleLeft => AttachmentPoint::MiddleLeft,
        MTextAttachment::MiddleCenter => AttachmentPoint::MiddleCenter,
        MTextAttachment::MiddleRight => AttachmentPoint::MiddleRight,
        MTextAttachment::BottomLeft => AttachmentPoint::BottomLeft,
        MTextAttachment::BottomCenter => AttachmentPoint::BottomCenter,
        MTextAttachment::BottomRight => AttachmentPoint::BottomRight,
    };
    converted.drawing_direction = match mtext.flow {
        MTextFlow::Horizontal => DrawingDirection::LeftToRight,
        MTextFlow::Vertical => DrawingDirection::TopToBottom,
        MTextFlow::ByStyle => DrawingDirection::ByStyle,
    };
    converted.rectangle_width = mtext.wrap_width.unwrap_or(0.0);
    if let Some(columns) = &mtext.columns {
        columns_in(columns, &mut converted);
    }
    if let Some(background) = &mtext.background {
        background_in(background, mtext.height, &mut converted);
    }
    let mut paragraph_format = mtext.paragraph_format.clone();
    if let Some(spacing) = paragraph_format.line_spacing.take() {
        spacing_in(spacing, &mut converted);
    }
    converted.value = emit_mtext(&mtext.character_format, &paragraph_format, &mtext.content);
    Some(EntityType::MText(Box::new(converted)))
}

fn spacing_out(mtext: &MText) -> Option<TextLineSpacing> {
    if mtext.line_spacing_factor.is_nan()
        || mtext.line_spacing_factor <= 0.0
        || (mtext.line_spacing_style == LineSpacingStyle::AtLeast
            && mtext.line_spacing_factor == 1.0)
    {
        return None;
    }
    let distance = settle(mtext.line_spacing_factor * LINE_PITCH * mtext.height);
    Some(match mtext.line_spacing_style {
        LineSpacingStyle::AtLeast => TextLineSpacing::AtLeast { distance },
        LineSpacingStyle::Exactly => TextLineSpacing::Exact { distance },
    })
}

fn spacing_in(spacing: TextLineSpacing, mtext: &mut MText) {
    let (factor, style) = match spacing {
        TextLineSpacing::Exact { distance } => (
            distance / (LINE_PITCH * mtext.height),
            LineSpacingStyle::Exactly,
        ),
        TextLineSpacing::AtLeast { distance } => (
            distance / (LINE_PITCH * mtext.height),
            LineSpacingStyle::AtLeast,
        ),
        TextLineSpacing::Multiple { factor } => (factor, LineSpacingStyle::AtLeast),
    };
    if factor.is_finite() && factor > 0.0 {
        mtext.line_spacing_factor = factor;
        mtext.line_spacing_style = style;
    }
}

fn columns_out(mtext: &MText) -> Option<MTextColumns> {
    let c = &mtext.column_data;
    let count = u32::try_from(c.column_count)
        .ok()
        .filter(|count| *count > 0)?;
    let columns = match c.column_type {
        1 => MTextColumns::Static {
            count,
            column_width: c.width,
            gutter: c.gutter,
            column_height: mtext.rectangle_height?,
            flow_reversed: c.flow_reversed,
        },
        2 if c.auto_height => MTextColumns::DynamicAutoHeight {
            column_width: c.width,
            gutter: c.gutter,
            column_height: mtext.rectangle_height?,
            current_column_count: count,
            flow_reversed: c.flow_reversed,
        },
        2 => {
            // The last manual column takes whatever text remains.
            let last = c.heights.len().checked_sub(1)?;
            MTextColumns::DynamicManualHeight {
                column_width: c.width,
                gutter: c.gutter,
                column_heights: c
                    .heights
                    .iter()
                    .enumerate()
                    .map(|(index, height)| match index == last {
                        true => MTextColumnHeight::Auto,
                        false => MTextColumnHeight::Fixed { distance: *height },
                    })
                    .collect(),
                flow_reversed: c.flow_reversed,
            }
        }
        _ => return None,
    };
    validate_mtext_columns(&columns).ok().map(|_| columns)
}

fn columns_in(columns: &MTextColumns, mtext: &mut MText) {
    let c = &mut mtext.column_data;
    let (count, width, gutter, reversed) = match columns {
        MTextColumns::Static {
            count,
            column_width,
            gutter,
            column_height,
            flow_reversed,
        } => {
            c.column_type = 1;
            mtext.rectangle_height = Some(*column_height);
            (*count, *column_width, *gutter, *flow_reversed)
        }
        MTextColumns::DynamicAutoHeight {
            column_width,
            gutter,
            column_height,
            current_column_count,
            flow_reversed,
        } => {
            c.column_type = 2;
            c.auto_height = true;
            mtext.rectangle_height = Some(*column_height);
            (
                *current_column_count,
                *column_width,
                *gutter,
                *flow_reversed,
            )
        }
        MTextColumns::DynamicManualHeight {
            column_width,
            gutter,
            column_heights,
            flow_reversed,
        } => {
            c.column_type = 2;
            c.heights = column_heights
                .iter()
                .map(|height| match height {
                    MTextColumnHeight::Fixed { distance } => *distance,
                    MTextColumnHeight::Auto => 0.0,
                })
                .collect();
            (
                column_heights.len() as u32,
                *column_width,
                *gutter,
                *flow_reversed,
            )
        }
    };
    c.column_count = count as i32;
    c.width = width;
    c.gutter = gutter;
    c.flow_reversed = reversed;
    mtext.rectangle_width = f64::from(count) * width + f64::from(count.saturating_sub(1)) * gutter;
}

fn background_out(mtext: &MText, losses: &mut Losses) -> Option<MTextBackground<IfccadColor>> {
    let flags = mtext.background_fill_flags;
    if flags & 0x13 == 0 {
        return None;
    }
    // Packed like other DWG transparencies: 0x02 in the top byte marks an
    // explicit alpha in the low byte.
    let transparency = mtext.background_transparency;
    let opacity = match transparency >> 24 {
        0 => 1.0,
        2 => f64::from(transparency & 0xff) / 255.0,
        _ => {
            losses.note("MText background transparency");
            1.0
        }
    };
    let fill = if flags & 2 != 0 {
        MTextFill::Canvas
    } else if flags & 1 != 0 {
        match color_out(mtext.background_color) {
            Some(color) => MTextFill::Color(color),
            None => MTextFill::Canvas,
        }
    } else {
        MTextFill::None
    };
    Some(MTextBackground {
        fill,
        padding: TextPadding::Relative {
            factor: (mtext.background_scale - 1.0).max(0.0),
        },
        opacity,
        frame: flags & 0x10 != 0,
    })
}

fn background_in(background: &MTextBackground<IfccadColor>, height: f64, mtext: &mut MText) {
    let padding = match background.padding {
        TextPadding::Relative { factor } => factor,
        TextPadding::Absolute { distance } => distance / height,
    };
    mtext.background_scale = 1.0 + padding;
    mtext.background_fill_flags = if background.frame { 0x10 } else { 0 };
    match &background.fill {
        MTextFill::None => {}
        MTextFill::Canvas => mtext.background_fill_flags |= 3,
        MTextFill::Color(color) => {
            mtext.background_fill_flags |= 1;
            mtext.background_color = color_in(color);
        }
    }
    if background.opacity < 1.0 {
        mtext.background_transparency =
            0x0200_0000 | (background.opacity.clamp(0.0, 1.0) * 255.0).round() as i32;
    }
}
