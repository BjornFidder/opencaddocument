use opencaddocument::ifccad::*;
pub fn mixed_owners() -> IfccadDocument {
    let mut d = load_ifccad_bytes(
        include_bytes!("../fixtures/xref/paper-layouts.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    let source = load_ifccad_bytes(
        include_bytes!("../fixtures/xref/attachment.ifcx"),
        Default::default(),
    )
    .unwrap()
    .into_document();
    d.external_sources = source.external_sources;
    d.id_counters.next_external_source_id = source.id_counters.next_external_source_id;
    let mut template = source.model.entities[0].as_native().unwrap().clone();
    if let IfccadEntityKind::Xref(x) = &mut template.kind {
        x.transform = IfccadBlockTransform {
            placement: IfccadPlacement {
                origin: [10., 20., 30.],
                x_axis: [-1., 0., 0.],
                y_axis: [0., 0., 1.],
            },
            rotation: 0.5,
            scale: [-2., 3., 4.],
        };
    }
    for paper in &mut d.paper_layouts {
        let mut e = template.clone();
        e.id = d.id_counters.allocate_entity_id().unwrap();
        if let IfccadEntityKind::Xref(x) = &mut e.kind {
            x.reference_type = IfccadXrefReferenceType::Overlay;
        }
        paper.entities = vec![IfccadEntity::Native(e)];
        paper.bounds = None;
        paper.bounds_quality = None;
    }
    for name in ["LOCAL", "UNUSED"] {
        let mut e = template.clone();
        e.id = d.id_counters.allocate_entity_id().unwrap();
        let id = d.id_counters.allocate_block_id().unwrap();
        d.blocks.push(IfccadBlockDefinition {
            annotation: Default::default(),
            description: String::new(),
            anonymous: false,
            explodable: true,
            uniform_scaling: false,
            bounds_quality: None,
            id,
            name: name.into(),
            base_point: [1., 2., 3.],
            insertion_unit: d.length_unit.clone(),
            bounds: None,
            entities: vec![IfccadEntity::Native(e)],
        });
        if name == "LOCAL" {
            for origin in [[100., 0., 0.], [0., 100., 0.]] {
                let mut e = template.clone();
                e.id = d.id_counters.allocate_entity_id().unwrap();
                e.kind = IfccadEntityKind::BlockInstance {
                    definition_id: id,
                    attributes: vec![],
                    annotation: None,
                    transform: IfccadBlockTransform {
                        placement: IfccadPlacement {
                            origin,
                            x_axis: [1., 0., 0.],
                            y_axis: [0., 1., 0.],
                        },
                        rotation: 0.7,
                        scale: [2., 3., 4.],
                    },
                };
                d.model.entities.push(IfccadEntity::Native(e));
            }
        }
    }
    template.id = d.id_counters.allocate_entity_id().unwrap();
    d.model.entities.push(IfccadEntity::Native(template));
    recompute_ifccad_document_bounds(&mut d).unwrap();
    validate_ifccad_document(&d).unwrap();
    d
}
