#![cfg(feature = "ifcx")]
#[path = "../examples/block_attributes_gallery.rs"]
mod gallery;
use ifccad::*;
use opencaddocument::{EntityType, Ifcx};

#[test]
fn paper_gallery_is_an_initialized_standalone_sheet() {
    let n = gallery::gallery().unwrap();
    let paper = n
        .paper_layouts
        .iter()
        .find(|p| p.name == "Attribute titleblock")
        .unwrap();
    let media=paper.settings.media.as_ref().expect("Paper review needs explicit media; otherwise OCS initializes a new layout and creates a model viewport");
    assert_eq!((media.width, media.height), (297., 210.));
    let canvas = paper
        .canvas
        .as_ref()
        .expect("Paper review needs its own saved camera");
    let center = canvas.frame.unwrap().center;
    assert_eq!((center.x(), center.y(), center.z()), (148.5, 105., 0.));
    let d = Ifcx.import_ifccad(&n).unwrap().document;
    let layout = d
        .objects
        .values()
        .find_map(|o| {
            if let opencaddocument::objects::ObjectType::Layout(l) = o {
                (l.name == paper.name).then_some(l)
            } else {
                None
            }
        })
        .unwrap();
    assert!(
        !(layout.paper_size.trim().is_empty()
            && layout.paper_width <= 0.
            && layout.paper_height <= 0.),
        "OCS page_setup_uninitialized must be false"
    );
    assert!(
        d.entities()
            .filter(|e| e.common().owner_handle == layout.block_record)
            .all(|e| !matches!(e,EntityType::Viewport(v) if v.id>1)),
        "No Model viewport belongs on the standalone paper review"
    );
}
#[test]
fn multiline_review_rows_have_room_and_matching_top_alignment() {
    let n = gallery::gallery().unwrap();
    for e in n
        .model
        .entities
        .iter()
        .chain(n.blocks.iter().flat_map(|b| &b.entities))
        .chain(n.paper_layouts.iter().flat_map(|p| &p.entities))
        .filter_map(|e| e.as_native())
    {
        if let IfccadEntityKind::BlockInstance { attributes, .. } = &e.kind {
            for a in attributes.iter().filter(|a| a.value.tag == "ROOM") {
                assert!(
                    a.value
                        .m_text
                        .as_ref()
                        .unwrap()
                        .text
                        .wrap_width
                        .unwrap_or(0.)
                        >= 60.,
                    "10-unit default box wraps ROOM into adjoining rows"
                );
                assert!(
                    matches!(
                        a.value.text.layout,
                        ifccad::text::TextLayout::Anchored {
                            vertical: ifccad::text::TextVerticalAlignment::Top,
                            ..
                        }
                    ),
                    "Outer baseline makes OCS lay multiline text upwards through CODE"
                );
                let code = attributes
                    .iter()
                    .find(|a| a.value.tag == "CODE" || a.value.tag == "RENAMED_CODE")
                    .unwrap();
                assert!(
                    code.value.text.placement.origin[1] - a.value.text.placement.origin[1] >= 14.
                );
            }
        }
    }
}

#[test]
fn physical_gallery_keeps_page_setup_camera_and_room_layout() {
    let n = gallery::gallery().unwrap();
    let d = Ifcx.import_ifccad(&n).unwrap().document;
    for kind in ["dwg", "ascii", "binary"] {
        let b = match kind {
            "dwg" => opencaddocument::DwgWriter::write_to_vec(&d).unwrap(),
            "binary" => opencaddocument::DxfWriter::new_binary(&d)
                .write_to_vec()
                .unwrap(),
            _ => opencaddocument::DxfWriter::new(&d).write_to_vec().unwrap(),
        };
        let r = if kind == "dwg" {
            opencaddocument::DwgReader::from_stream(std::io::Cursor::new(b))
                .read_with_stats()
                .unwrap()
        } else {
            opencaddocument::DxfReader::from_reader(std::io::Cursor::new(b))
                .unwrap()
                .read_with_stats()
                .unwrap()
        };
        assert_eq!(r.stats.recovered_errors, 0);
        assert_eq!(r.stats.skipped_source_records, 0);
        assert!(r.stats.stream_completed);
        let l = r
            .document
            .objects
            .values()
            .find_map(|o| {
                if let opencaddocument::objects::ObjectType::Layout(l) = o {
                    (l.name == "Attribute titleblock").then_some(l)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(
            (l.paper_width, l.paper_height),
            (297., 210.),
            "{kind}: must not initialize a new page in OCS"
        );
        let page_unit_mm = if l.plot_paper_units == 0 { 25.4 } else { 1.0 };
        let scope_units_per_mm = l.plot_scale_denominator / l.plot_scale_numerator / page_unit_mm;
        assert_eq!((l.paper_width*scope_units_per_mm,l.paper_height*scope_units_per_mm),(297.,210.),"{kind}: physical sheet and 297x210 Paper coordinates must agree; inherited inch plot units shrink the sheet by 25.4");
        let paper = r
            .document
            .entities()
            .filter(|e| e.common().owner_handle == l.block_record)
            .collect::<Vec<_>>();
        assert!(!paper
            .iter()
            .any(|e| matches!(e,EntityType::Viewport(v) if v.id>1)));
        assert_eq!(
            paper
                .iter()
                .filter(|e| matches!(e, EntityType::Insert(_)))
                .count(),
            1
        );
        let EntityType::Insert(i) = paper
            .iter()
            .find(|e| matches!(e, EntityType::Insert(_)))
            .unwrap()
        else {
            unreachable!()
        };
        let room = i.attributes.iter().find(|a| a.tag == "ROOM").unwrap();
        assert_eq!(
            room.insertion_point,
            opencaddocument::Vector3::new(40., 162., 0.)
        );
        assert_eq!(
            room.vertical_alignment,
            opencaddocument::entities::attribute_definition::VerticalAlignment::Top
        );
        assert_eq!(room.embedded_mtext.as_ref().unwrap().rectangle_width, 80.);
        let exported = Ifcx.export_ifccad(&r.document).unwrap();
        let native = ifccad::encode_ifccad_document(&exported.drawing).unwrap();
        ifccad::load_ifccad_bytes(native.bytes(), Default::default()).unwrap();
        let returned = exported
            .drawing
            .paper_layouts
            .iter()
            .find(|l| l.name == "Attribute titleblock")
            .unwrap();
        let canvas = returned.canvas.as_ref().unwrap();
        let center = canvas.frame.unwrap().center;
        assert_eq!((center.x(), center.y()), (148.5, 105.));
        assert_eq!(canvas.view.height, 230.);
    }
}
