//! Compact slice-A review files, read back through production readers.
use opencaddocument::{ifccad::*, *};
#[path = "../tests/support/xref_placement.rs"]
mod support;
fn main() -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let root = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("output directory required")?,
    );
    std::fs::create_dir_all(root.join("Refs"))?;
    let mut drawing = support::mixed_owners();
    drawing.external_sources[0].location = IfccadExternalSourceLocation::UriReference {
        uri: "Refs/Detail.dwg".into(),
    };
    let mut source = Document::new();
    source.header.insertion_units = 6;
    source.header.model_space_insertion_base = Vector3::new(10., 20., 0.);
    source.header.fingerprint_guid = "{E5B4A9AF-CF20-4FC9-A30F-FFB20FD987E1}".into();
    source.add_entity(EntityType::Line(entities::Line::from_coords(
        10., 20., 0., 12., 20., 0.,
    )))?;
    source.add_entity(EntityType::Circle(entities::Circle::from_center_radius(
        Vector3::new(11., 21., 0.),
        0.5,
    )))?;
    let mut text = entities::Text::new();
    text.value = "XREF".into();
    text.insertion_point = Vector3::new(10., 22., 0.);
    text.height = 0.5;
    source.add_entity(EntityType::Text(text))?;
    for (name, format) in [
        ("Detail.dwg", &Dwg as &dyn Format),
        ("Detail.dxf", &Dxf as &dyn Format),
    ] {
        let (bytes, losses) = format.write(&source)?;
        assert!(losses.is_empty(), "{losses:?}");
        format.read(&bytes)?;
        std::fs::write(root.join("Refs").join(name), bytes)?;
    }
    let native = Ifcx.export_ifccad(&source)?.drawing;
    let json = encode_ifccad_document(&native)?;
    load_ifccad_bytes(json.bytes(), Default::default())?;
    std::fs::write(root.join("Refs/Detail.ifccad"), json.bytes())?;
    for (name, binary) in [("Gallery.ifccad", false), ("Gallery.cbor", true)] {
        let bytes = if binary {
            encode_ifccad_cbor_document(&drawing)?.bytes().to_vec()
        } else {
            encode_ifccad_document(&drawing)?.bytes().to_vec()
        };
        if binary {
            load_ifccad_cbor_bytes(&bytes, Default::default())?;
        } else {
            load_ifccad_bytes(&bytes, Default::default())?;
        }
        std::fs::write(root.join(name), bytes)?;
    }
    let imported = Ifcx.import_ifccad(&drawing)?;
    for (name, format) in [
        ("Gallery.dwg", &Dwg as &dyn Format),
        ("Gallery.dxf", &Dxf as &dyn Format),
    ] {
        let (bytes, losses) = format.write(&imported.document)?;
        assert!(losses.is_empty(), "{losses:?}");
        let (cad, _) = format.read(&bytes)?;
        validate_ifccad_document(&Ifcx.export_ifccad(&cad)?.drawing)?;
        std::fs::write(root.join(name), bytes)?;
    }
    println!("Review files: {}", root.display());
    Ok(())
}
