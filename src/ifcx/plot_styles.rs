use acadrust::objects::{Layout, ObjectType, PlaceHolder};
use ifccad::{IfccadPlotStyle as Style, IfccadPlotStyleSource, IfccadPlotStyleTableKind as Kind};

use super::*;

pub(super) fn validate_cad_bindings(drawing: &IfccadDocument) -> Result<(), Error> {
    let styles = &drawing.plot_styles;
    let layouts = [(drawing.model.id, "Model")].into_iter().chain(
        drawing
            .paper_layouts
            .iter()
            .map(|layout| (layout.id, layout.name.as_str())),
    );
    validate_cad_sources(layouts.filter_map(|(id, layout)| {
        let source = styles.sources.get(styles.layout_sources.get(&id)?)?;
        Some((layout, source))
    }))
}

pub(super) fn validate_cad_sources<'a>(
    layouts: impl IntoIterator<Item = (&'a str, &'a IfccadPlotStyleSource)>,
) -> Result<(), Error> {
    let mut names = BTreeMap::new();
    for (layout, source) in layouts {
        let Some(hash) = source.sha256.as_deref() else {
            continue;
        };
        let (first_layout, first_hash) = names
            .entry(source.file_name.to_ascii_lowercase())
            .or_insert((layout, hash));
        if *first_hash != hash {
            return Err(annotation::invalid(&format!(
                "Layouts {first_layout:?} and {layout:?} bind different plot style tables named {:?}. CAD requires distinct file names for these tables.",
                source.file_name,
            )));
        }
    }
    Ok(())
}

pub(super) fn source_out(writer: &mut Writer<'_>, layout: &Layout, id: u64) {
    let name = &layout.plot_style_sheet;
    if name.is_empty() || name.eq_ignore_ascii_case("None") {
        return;
    }
    let (kind, extension) = if writer.document.header.plotstyle_mode {
        (Kind::Ctb, "ctb")
    } else {
        (Kind::Stb, "stb")
    };
    let valid_name = !name
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '/' | '\\' | ':'))
        && name.rsplit_once('.').is_some_and(|(stem, ext)| {
            !stem.trim().is_empty()
                && !matches!(stem, "." | "..")
                && ext.eq_ignore_ascii_case(extension)
        });
    if !valid_name {
        writer.losses.located_loss(
            layout.handle,
            format!(
                "Layout {:?}: incompatible or invalid plot-style table {name:?} omitted.",
                layout.name
            ),
        );
        return;
    }
    let styles = &mut writer.drawing.plot_styles;
    let source = styles
        .sources
        .iter()
        .find_map(|(&id, source)| (source.kind == kind && source.file_name == *name).then_some(id));
    let source = source.unwrap_or_else(|| {
        let id = styles.next_source_id;
        styles.next_source_id += 1;
        styles.sources.insert(
            id,
            IfccadPlotStyleSource {
                kind,
                file_name: name.clone(),
                sha256: None,
            },
        );
        id
    });
    styles.layout_sources.insert(id, source);
}

pub(super) fn source_in(
    drawing: &IfccadDocument,
    id: u64,
    layout: &mut Layout,
    losses: &mut Losses,
) {
    let styles = &drawing.plot_styles;
    if let Some(source) = styles
        .layout_sources
        .get(&id)
        .and_then(|id| styles.sources.get(id))
    {
        layout.plot_style_sheet.clone_from(&source.file_name);
        if source.sha256.is_some() {
            losses.located_loss(
                layout.handle,
                format!(
                    "Layout {:?}: CAD stores the table name {:?} without its SHA-256.",
                    layout.name, source.file_name,
                ),
            );
        }
    }
}

fn named_out(document: &Document, handle: Handle) -> Option<Style> {
    let name = plot_style_name(document, handle)?;
    if name.eq_ignore_ascii_case("Normal") {
        return Some(Style::Normal);
    }
    if name.trim().is_empty()
        || name.chars().any(char::is_control)
        || ["ByLayer", "ByBlock"]
            .iter()
            .any(|reserved| name.eq_ignore_ascii_case(reserved))
    {
        return None;
    }
    Some(Style::Named {
        name: name.to_owned(),
    })
}

fn default_out(document: &Document) -> Option<Style> {
    let dictionary = document.get_subdictionary("ACAD_PLOTSTYLENAME")?;
    match document.objects.get(&dictionary)? {
        ObjectType::DictionaryWithDefault(table) if !table.default_handle.is_null() => {
            named_out(document, table.default_handle)
        }
        _ => Some(Style::Normal),
    }
}

pub(super) fn assignments_out(writer: &mut Writer<'_>) {
    let document = writer.document;
    let styles = &mut writer.drawing.plot_styles;
    for layer in document.layers.iter() {
        let Some(&id) = writer.layers.get(&layer.name.to_ascii_lowercase()) else {
            continue;
        };
        let handle = layer.plotstyle_handle;
        if handle.is_null() {
            continue;
        }
        match named_out(document, handle) {
            Some(Style::Normal) => {}
            Some(style) if !document.header.plotstyle_mode => {
                styles.layer_assignments.insert(id, style);
            }
            _ => writer.losses.located_loss(
                layer.handle,
                format!(
                "Layer {:?}: unresolved or incompatible plot-style assignment {handle} omitted.",
                layer.name,
            ),
            ),
        }
    }
    for (&handle, &id) in &writer.entity_ids {
        let Some(entity) = document.get_entity(handle) else {
            continue;
        };
        let common = entity.common();
        let style = match common.plotstyle_flags {
            0 => continue,
            1 => Some(Style::ByBlock),
            2 => default_out(document),
            3 => common
                .plotstyle_handle
                .and_then(|handle| named_out(document, handle)),
            _ => None,
        };
        match style {
            Some(style) if !document.header.plotstyle_mode => {
                styles.entity_assignments.insert(id, style);
            }
            _ => writer.losses.located_loss(
                handle,
                format!(
                    "{}: unresolved or incompatible plot-style assignment omitted.",
                    entity.as_entity().entity_type(),
                ),
            ),
        }
    }
}

pub(super) fn assignments_in(
    drawing: &IfccadDocument,
    document: &mut Document,
    names: &Names,
    handles: &HashMap<u64, Handle>,
    losses: &mut Losses,
) -> Result<(), Error> {
    let styles = &drawing.plot_styles;
    if styles.layer_assignments.is_empty() && styles.entity_assignments.is_empty() {
        return Ok(());
    }
    let dictionary = document.header.acad_plotstylename_dict_handle;
    let mut styles_by_name = BTreeMap::new();
    for style in styles
        .layer_assignments
        .values()
        .chain(styles.entity_assignments.values())
    {
        let Style::Named { name } = style else {
            continue;
        };
        if styles_by_name.contains_key(name) {
            continue;
        }
        let mut placeholder = PlaceHolder::new();
        let handle = document.allocate_handle();
        placeholder.handle = handle;
        placeholder.owner = dictionary;
        let Some(ObjectType::DictionaryWithDefault(table)) = document.objects.get_mut(&dictionary)
        else {
            return Err(annotation::invalid("missing CAD plot-style dictionary"));
        };
        table.add_entry(name, handle);
        document
            .objects
            .insert(handle, ObjectType::PlaceHolder(placeholder));
        styles_by_name.insert(name, handle);
    }
    for (id, style) in &styles.layer_assignments {
        if let Some(layer) = names
            .layers
            .get(id)
            .and_then(|name| document.layers.get_mut(name))
        {
            if let Style::Named { name } = style {
                layer.plotstyle_handle = styles_by_name[name];
            }
        }
    }
    for (id, style) in &styles.entity_assignments {
        let Some(handle) = handles.get(id) else {
            losses.note(format!("plot-style assignment of IFCCAD object {id}"));
            continue;
        };
        let Some(common) = attributes::common_mut(document, *handle) else {
            losses.located_loss(
                *handle,
                format!("Plot-style assignment of IFCCAD object {id} omitted."),
            );
            continue;
        };
        let (flags, style_handle) = match style {
            Style::ByLayer => (0, None),
            Style::ByBlock => (1, None),
            Style::Normal => (2, None),
            Style::Named { name } => (3, Some(styles_by_name[name])),
        };
        common.plotstyle_flags = flags;
        common.plotstyle_handle = style_handle;
    }
    Ok(())
}
