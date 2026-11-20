//! Small XREF review project; all files pass their production reader.
use opencaddocument::{ifccad::*, *};
use std::path::PathBuf;

fn main() -> std::result::Result<(), Error> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("output directory required")?;
    let drawings = root.join("Tekeningen");
    let references = root.join("Referenties");
    std::fs::create_dir_all(&drawings)?;
    std::fs::create_dir_all(&references)?;
    let mut d = load_ifccad_bytes(
        include_bytes!("../tests/fixtures/xref/attachment.ifcx"),
        Default::default(),
    )?
    .into_document();
    d.paper_layouts.clear();
    let workspace = load_ifccad_bytes(
        include_bytes!("../tests/fixtures/workspace-state.ifcx"),
        Default::default(),
    )?
    .into_document();
    let mut window = workspace.model_windows[0].clone();
    window.id = d.id_counters.allocate_model_window_id()?;
    window.view.target = ifccad::geometry_kernel::Point3::new(10000., 5500., 0.);
    window.view.height = 2500.;
    window.stored_ucs = IfccadUcsSelection::World;
    window.use_stored_ucs = false;
    window.grid.enabled = false;
    d.model_view_state = Some(IfccadModelViewState {
        current_model_ucs: Some(IfccadUcsSelection::World),
        active_model_window_id: Some(window.id),
    });
    d.model_windows.push(window);
    let mut source = Document::new();
    source.header.insertion_units = 6;
    source.header.model_space_insertion_base = Vector3::new(10., 20., 0.);
    source.header.fingerprint_guid = "{E5B4A9AF-CF20-4FC9-A30F-FFB20FD987E1}".into();
    source.add_entity(EntityType::Line(entities::Line::from_coords(
        10., 20., 0., 12., 20., 0.,
    )))?;
    let (bytes, losses) = Dwg.write(&source)?;
    assert!(losses.is_empty());
    let (read, _) = Dwg.read(&bytes)?;
    assert_eq!(
        read.header.model_space_insertion_base,
        source.header.model_space_insertion_base
    );
    std::fs::write(references.join("grondplan.dwg"), &bytes)?;
    let variants = root.join("Variants");
    std::fs::create_dir_all(&variants)?;
    let mut other_identity = source.clone();
    other_identity.header.fingerprint_guid = "{11111111-2222-3333-4444-555555555555}".into();
    let (variant, losses) = Dwg.write(&other_identity)?;
    assert!(losses.is_empty());
    assert_eq!(
        Dwg.read(&variant)?.0.header.fingerprint_guid,
        other_identity.header.fingerprint_guid
    );
    std::fs::write(variants.join("other-guid.dwg"), variant)?;
    if let Some(public) = std::env::args_os().nth(2).map(PathBuf::from) {
        std::fs::create_dir_all(&public)?;
        std::fs::write(public.join("xref-source.dwg"), &bytes)?;
        let mut gallery = d.clone();
        gallery.model_windows[0].view.target =
            ifccad::geometry_kernel::Point3::new(13000., 5500., 0.);
        gallery.model_windows[0].view.height = 6000.;
        let prototype = gallery.model.entities[0].clone();
        gallery.external_sources.clear();
        gallery.model.entities.clear();
        // This is a newly authored public drawing, independent of fixture history.
        gallery.id_counters.next_entity_id = 1;
        gallery.id_counters.next_external_source_id = 1;
        gallery.id_counters.next_layout_id = gallery.model.id + 1;
        let mut layers = std::collections::BTreeMap::new();
        let mut next_layer = 1;
        for layer in &mut gallery.layers {
            let id = if layer.id == 0 {
                0
            } else {
                let id = next_layer;
                next_layer += 1;
                id
            };
            layers.insert(layer.id, id);
            layer.id = id;
        }
        gallery.id_counters.next_layer_id = next_layer;
        if let Some(workspace) = &mut gallery.workspace_state {
            workspace.current_layer_id = workspace.current_layer_id.map(|id| layers[&id]);
        }
        for (index, location) in [
            IfccadExternalSourceLocation::UriReference {
                uri: "./xref-source.dwg".into(),
            },
            IfccadExternalSourceLocation::SearchFileName {
                file_name: "xref-source.dwg".into(),
            },
            IfccadExternalSourceLocation::UriReference {
                uri: "file:///C:/CAD/xref-source.dwg".into(),
            },
            IfccadExternalSourceLocation::UriReference {
                uri: "example:offline-xref".into(),
            },
        ]
        .into_iter()
        .enumerate()
        {
            let mut definition = d.external_sources[0].clone();
            definition.id = gallery.id_counters.allocate_external_source_id()?;
            definition.name = [
                "Relative",
                "Search filename",
                "Absolute file URI",
                "Unresolved URI",
            ][index]
                .into();
            definition.location = location;
            let mut entity = prototype.clone();
            let IfccadEntity::Native(native) = &mut entity else {
                unreachable!()
            };
            native.id = gallery.id_counters.allocate_entity_id()?;
            native.layer_id = layers[&native.layer_id];
            let IfccadEntityKind::Xref(xref) = &mut native.kind else {
                unreachable!()
            };
            xref.source_id = definition.id;
            xref.placement.origin[0] += index as f64 * 2000.;
            gallery.external_sources.push(definition);
            gallery.model.entities.push(entity);
        }
        validate_ifccad_document(&gallery)?;
        let encoded = encode_ifccad_document(&gallery)?;
        load_ifccad_bytes(encoded.bytes(), Default::default())?;
        std::fs::write(public.join("xrefs.ifcx"), encoded.bytes())?;
    }
    let (native, mut context) = Ifcx.import_ifccad_preserving(&d)?;
    assert!(native.losses.is_empty());
    let back = Ifcx.export_ifccad_preserving(&native.document, &mut context)?;
    assert_eq!(back.drawing.external_sources, d.external_sources);
    std::fs::write(
        drawings.join("hoofd.ifcx"),
        encode_ifccad_document(&d)?.bytes(),
    )?;
    std::fs::write(
        drawings.join("hoofd.ifccad.cbor"),
        encode_ifccad_cbor_document(&d)?.bytes(),
    )?;
    for (name, format, guid) in [
        ("hoofd-retour.dwg", &Dwg as &dyn Format, true),
        ("hoofd-retour.dxf", &Dxf as &dyn Format, false),
    ] {
        let (bytes, losses) = format.write(&native.document)?;
        assert!(losses.is_empty());
        let (read, _) = format.read(&bytes)?;
        let back = Ifcx.export_ifccad(&read)?;
        validate_ifccad_document(&back.drawing)?;
        assert_eq!(back.drawing.external_sources.len(), 1);
        let IfccadExternalSourceContent::Dwg {
            fingerprint_guid, ..
        } = &back.drawing.external_sources[0].source;
        assert_eq!(fingerprint_guid.is_some(), guid);
        std::fs::write(drawings.join(name), bytes)?;
    }
    println!("{}", root.display());
    Ok(())
}
