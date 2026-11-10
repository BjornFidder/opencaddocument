#![cfg(feature = "ifcx")]

use std::io::Cursor;

use opencaddocument::entities::Spline;
use opencaddocument::ifccad::{self, IfccadEntityKind, IfccadSplineDefinition};
use opencaddocument::types::{DxfVersion, Vector3};
use opencaddocument::{
    Document, DwgReader, DwgWriter, DxfReader, DxfWriter, EntityType, Ifcx, SplineExchangeTarget,
};

fn mixed() -> (Document, Spline) {
    let mut source = Document::with_version(DxfVersion::AC1032);
    let mut spline = Spline::new();
    spline.degree = 2;
    spline.knots = vec![0., 0., 0., 1., 1., 1.];
    spline.control_points = vec![
        Vector3::new(0., 0., 0.),
        Vector3::new(1., 2., 0.),
        Vector3::new(3., 0., 1.),
    ];
    spline.fit_points = vec![spline.control_points[0], spline.control_points[2]];
    spline.begin_tangent = Vector3::new(1., 2., 0.);
    source
        .add_entity(EntityType::Spline(spline.clone()))
        .unwrap();
    (source, spline)
}

#[test]
fn mixed_spline_keeps_control_and_fit_through_physical_dxf() {
    let (source, spline) = mixed();
    let dxf = DxfWriter::new(&source).write_to_vec().unwrap();
    let physical = DxfReader::from_reader(Cursor::new(dxf))
        .unwrap()
        .read()
        .unwrap();

    let native = Ifcx.export_ifccad(&physical).unwrap();
    let encoded = ifccad::encode_ifccad_document(&native.drawing).unwrap();
    let read = ifccad::load_ifccad_bytes(encoded.bytes(), Default::default()).unwrap();
    let entity = read.document().model.entities[0].as_native().unwrap();
    let IfccadEntityKind::Spline(value) = &entity.kind else {
        panic!("native spline")
    };
    assert!(matches!(
        value.definition,
        IfccadSplineDefinition::Control { fit: Some(_), .. }
    ));

    let output = Ifcx.import_ifccad(read.document()).unwrap();
    let dxf = DxfWriter::new(&output.document).write_to_vec().unwrap();
    let physical = DxfReader::from_reader(Cursor::new(dxf))
        .unwrap()
        .read()
        .unwrap();
    let observed = physical
        .entities()
        .find_map(|entity| match entity {
            EntityType::Spline(spline) => Some(spline),
            _ => None,
        })
        .unwrap();
    assert_eq!(observed.control_points, spline.control_points);
    assert_eq!(observed.fit_points, spline.fit_points);
    assert_eq!(observed.knots, spline.knots);
    assert_eq!(observed.begin_tangent, spline.begin_tangent);
}

#[test]
fn dxf_refuses_a_spline_count_that_would_wrap_the_record_field() {
    let mut source = Document::with_version(DxfVersion::AC1032);
    let points = (0..=i16::MAX)
        .map(|i| Vector3::new(f64::from(i), 1., 0.))
        .collect();
    source
        .add_entity(EntityType::Spline(Spline::from_fit_points(points)))
        .unwrap();
    let native = Ifcx.export_ifccad(&source).unwrap();
    let mut target = Ifcx.import_ifccad(&native.drawing).unwrap();
    target.document.version = DxfVersion::AC1032;
    let losses = target
        .prepare_spline_exchange(SplineExchangeTarget::Dxf, false)
        .unwrap_err();
    assert_eq!(losses.len(), 1);
    assert!(losses[0].message.contains("16-bit DXF record field"));
}

#[test]
fn closed_fit_only_dwg_diagnoses_a_physically_added_endpoint() {
    let mut source = Document::with_version(DxfVersion::AC1032);
    let mut spline = Spline::from_fit_points(vec![
        Vector3::new(0., 0., 0.),
        Vector3::new(1., 2., 0.),
        Vector3::new(5., 1., 0.),
    ]);
    spline.flags.closed = true;
    source.add_entity(EntityType::Spline(spline)).unwrap();
    let native = Ifcx.export_ifccad(&source).unwrap();
    let mut target = Ifcx.import_ifccad(&native.drawing).unwrap();
    target.document.version = DxfVersion::AC1032;
    let losses = target
        .prepare_spline_exchange(SplineExchangeTarget::Dwg, true)
        .unwrap_err();
    assert!(losses[0].message.contains("endpoint sequence"));
    target
        .prepare_spline_exchange(SplineExchangeTarget::Dwg, false)
        .unwrap();
    let bytes = DwgWriter::write_to_vec(&target.document).unwrap();
    let returned = DwgReader::from_stream(Cursor::new(bytes)).read().unwrap();
    let spline = returned
        .entities()
        .find_map(|entity| match entity {
            EntityType::Spline(spline) => Some(spline),
            _ => None,
        })
        .unwrap();
    assert!(spline.flags.closed);
    assert_eq!(spline.fit_points.len(), 4);
    assert_eq!(spline.fit_points.first(), spline.fit_points.last());
}

#[test]
fn mixed_dwg_prioritizes_control_and_strict_refuses_the_fit_loss() {
    let (source, spline) = mixed();
    let drawing = Ifcx.export_ifccad(&source).unwrap().drawing;
    let mut target = Ifcx.import_ifccad(&drawing).unwrap();
    target.document.version = DxfVersion::AC1032;
    let result = target.prepare_spline_exchange(SplineExchangeTarget::Dwg, true);
    let losses = result.unwrap_err();
    assert_eq!(losses.len(), 1);
    assert!(losses[0].handle.is_some());
    assert!(losses[0].message.contains("fit data"));
    target
        .prepare_spline_exchange(SplineExchangeTarget::Dwg, false)
        .unwrap();
    let bytes = DwgWriter::write_to_vec(&target.document).unwrap();
    let read = DwgReader::from_stream(Cursor::new(bytes)).read().unwrap();
    let observed = read
        .entities()
        .find_map(|entity| match entity {
            EntityType::Spline(spline) => Some(spline),
            _ => None,
        })
        .unwrap();
    assert_eq!(observed.control_points, spline.control_points);
    assert_eq!(observed.knots, spline.knots);
    assert!(observed.fit_points.is_empty());
}

#[test]
fn leader_picture_added_to_document_survives_ifcx() {
    use opencaddocument::entities::Leader;
    use opencaddocument::ifccad::{IfccadLeaderGraphicCommand, IfccadLeaderGraphicPrimitive};
    use opencaddocument::{CheckedProxyGraphicRecord, CheckedProxyGraphics, ProxyGraphicCommand};
    let mut document = Document::new();
    let start = Vector3::new(1., 2., 0.);
    let end = Vector3::new(3., 4., 0.);
    let handle = document
        .add_entity(EntityType::Leader(Leader::two_point(start, end)))
        .unwrap();
    let original = Ifcx.export_ifccad(&document).unwrap();
    let get = |drawing: &ifccad::IfccadDocument| {
        let entity = drawing.model.entities[0].as_native().unwrap();
        let IfccadEntityKind::Leader(leader) = &entity.kind else {
            panic!("expected leader")
        };
        leader.graphics.clone()
    };
    assert!(get(&original.drawing).is_none());
    document
        .get_entity_mut(handle)
        .unwrap()
        .common_mut()
        .graphic_data = Some(
        CheckedProxyGraphics {
            records: vec![CheckedProxyGraphicRecord::Command(
                ProxyGraphicCommand::Polyline(vec![start, end]),
            )],
        }
        .encode_checked()
        .unwrap(),
    );
    let captured = Ifcx.export_ifccad(&document).unwrap();
    let bytes = ifccad::encode_ifccad_document(&captured.drawing).unwrap();
    let read = ifccad::load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    let graphics = get(read.document()).unwrap();
    let [IfccadLeaderGraphicCommand::Draw(IfccadLeaderGraphicPrimitive::Polyline {
        vertices, ..
    })] = graphics.commands.as_slice()
    else {
        panic!("expected supplied polyline")
    };
    assert_eq!(vertices, &[[1., 2., 0.], [3., 4., 0.]]);
}

#[test]
fn compact_weighted_cycle_can_use_an_exact_full_dxf_basis() {
    let mut source = Document::with_version(DxfVersion::AC1032);
    let mut spline = Spline::new();
    spline.degree = 3;
    spline.flags.closed = true;
    spline.flags.periodic = true;
    spline.flags.rational = true;
    spline.knots = vec![2., 2.5, 4., 7., 9.];
    spline.control_points = vec![
        Vector3::new(0., 0., 0.),
        Vector3::new(3., -2., 1.),
        Vector3::new(5., 4., 0.),
        Vector3::new(-1., 2., -1.),
    ];
    spline.weights = vec![1., 2., 0.75, 1.25];
    source.add_entity(EntityType::Spline(spline)).unwrap();
    let native = Ifcx.export_ifccad(&source).unwrap().drawing;
    let mut output = Ifcx.import_ifccad(&native).unwrap();
    output.document.version = DxfVersion::AC1032;
    output
        .prepare_spline_exchange(SplineExchangeTarget::Dxf, true)
        .unwrap();
    let bytes = DxfWriter::new(&output.document).write_to_vec().unwrap();
    let read = DxfReader::from_reader(Cursor::new(bytes))
        .unwrap()
        .read()
        .unwrap();
    let returned = Ifcx.export_ifccad(&read).unwrap().drawing;
    let basis = |drawing: &ifccad::IfccadDocument| {
        let entity = drawing.model.entities[0].as_native().unwrap();
        let IfccadEntityKind::Spline(spline) = &entity.kind else {
            panic!("native spline")
        };
        spline.control_basis().unwrap().clone()
    };
    assert_eq!(basis(&returned), basis(&native));
}

#[test]
fn spline_viewport_boundary_keeps_its_owner_and_active_reference() {
    let mut source: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/viewport-bulged-clip.ifcx")).unwrap();
    let boundary = source["data"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|node| node["path"] == "/cad/d1/e1002")
        .unwrap();
    let attributes = boundary["attributes"].as_object_mut().unwrap();
    attributes.remove("ifccad::geom::placement");
    attributes.remove("ifccad::geom::planarPolyline");
    attributes.insert(
        "ifccad::geom::spline".into(),
        serde_json::json!({
            "topology":"closed",
            "definition":{"kind":"control","basis":{
                "degree":2,"knotKind":"full","knots":[0,0,0,1,1,1,2,2,2],
                "points":[[80,75],[100,120],[120,75],[120,75],[100,30],[80,75]]
            }}
        }),
    );
    let input = serde_json::to_vec(&source).unwrap();
    let loaded = ifccad::load_ifccad_bytes(&input, Default::default()).unwrap();
    let target = Ifcx.import_ifccad(loaded.document()).unwrap();
    let bytes = DxfWriter::new(&target.document).write_to_vec().unwrap();
    let dxf = DxfReader::from_reader(Cursor::new(bytes))
        .unwrap()
        .read()
        .unwrap();
    let output = Ifcx.export_ifccad(&dxf).unwrap();
    let bytes = ifccad::encode_ifccad_document(&output.drawing).unwrap();
    let read = ifccad::load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    let paper = &read.document().paper_layouts[0];
    let clip = paper
        .entities
        .iter()
        .filter_map(|e| e.as_native())
        .find_map(|e| match &e.kind {
            IfccadEntityKind::Viewport(viewport) => Some(&viewport.paper_clip),
            _ => None,
        })
        .unwrap();
    assert!(clip.enabled);
    let id = clip.boundary_entity_id.unwrap();
    assert!(paper
        .entities
        .iter()
        .filter_map(|e| e.as_native())
        .any(|entity| { entity.id == id && matches!(entity.kind, IfccadEntityKind::Spline(_)) }));
}

#[test]
fn hatch_spline_and_fit_only_source_survive_physical_dwg() {
    use opencaddocument::entities::{BoundaryEdge, BoundaryPath, Hatch, LineEdge, SplineEdge};
    use opencaddocument::types::Vector2;
    let mut document = Document::with_version(DxfVersion::AC1032);
    let source = document
        .add_entity(EntityType::Spline(Spline::from_fit_points(vec![
            Vector3::ZERO,
            Vector3::new(1., 1., 0.),
            Vector3::new(2., 0., 0.),
        ])))
        .unwrap();
    let mut path = BoundaryPath::external();
    path.boundary_handles.push(source);
    path.add_edge(BoundaryEdge::Spline(SplineEdge {
        degree: 2,
        rational: true,
        periodic: false,
        knots: vec![0., 0., 0., 1., 1., 1.],
        control_points: vec![
            Vector3::new(0., 0., 1.),
            Vector3::new(1., 2., 0.75),
            Vector3::new(2., 0., 1.),
        ],
        fit_points: vec![Vector2::ZERO, Vector2::new(2., 0.)],
        start_tangent: Vector2::new(1., 2.),
        end_tangent: Vector2::new(1., -2.),
    }));
    path.add_edge(BoundaryEdge::Line(LineEdge {
        start: Vector2::new(2., 0.),
        end: Vector2::ZERO,
    }));
    let mut hatch = Hatch::new();
    hatch.is_solid = true;
    hatch.is_associative = true;
    hatch.add_path(path);
    document
        .add_entity(EntityType::Hatch(Box::new(hatch)))
        .unwrap();

    let native = Ifcx.export_ifccad(&document).unwrap().drawing;
    let bytes = ifccad::encode_ifccad_document(&native).unwrap();
    let checked = ifccad::load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    let mut target = Ifcx.import_ifccad(checked.document()).unwrap();
    target.document.version = DxfVersion::AC1032;
    target
        .prepare_spline_exchange(SplineExchangeTarget::Dwg, true)
        .unwrap();
    let bytes = DwgWriter::write_to_vec(&target.document).unwrap();
    let read = DwgReader::from_stream(Cursor::new(bytes)).read().unwrap();
    let drawing = Ifcx.export_ifccad(&read).unwrap().drawing;
    let source = drawing.model.entities.iter().filter_map(|e| e.as_native()).find(|e| {
        matches!(&e.kind, IfccadEntityKind::Spline(v) if matches!(v.definition, IfccadSplineDefinition::FitOnly { .. }))
    }).unwrap();
    let hatch = drawing
        .model
        .entities
        .iter()
        .filter_map(|e| e.as_native())
        .find_map(|e| {
            if let IfccadEntityKind::Hatch(hatch) = &e.kind {
                Some(hatch)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(hatch.loops[0].source_entity_ids, vec![source.id]);
    let ifccad::geometry_kernel::hatch::HatchBoundary2::Edges(edges) = &hatch.loops[0].boundary
    else {
        panic!("hatch edges")
    };
    let ifccad::geometry_kernel::hatch::HatchEdge2::Spline { control, fit, .. } = &edges[0] else {
        panic!("hatch spline")
    };
    assert_eq!(control.weights.as_deref(), Some(&[1., 0.75, 1.][..]));
    assert_eq!(fit.as_ref().unwrap().points, vec![[0., 0.], [2., 0.]]);
}
