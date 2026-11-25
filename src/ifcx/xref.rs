//! XREF definitions remain external even when the CAD host has resolved bodies.
use super::*;
use ifccad::{
    IfccadExternalSource, IfccadExternalSourceContent, IfccadExternalSourceId, IfccadXref,
    IfccadXrefReferenceType,
};

pub(super) fn diagnose_dependency_losses(writer: &mut Writer<'_>) {
    for dependency in &writer.document.file_dependencies {
        let binding = writer.document.block_records.iter().find(|record| {
            if !writer.external_sources.contains_key(&record.handle)
                || !dependency.feature.eq_ignore_ascii_case("Acad:XRef")
            {
                return false;
            }
            let matches = dependency
                .file_name
                .replace('\\', "/")
                .eq_ignore_ascii_case(&record.xref_path.replace('\\', "/"));
            #[cfg(feature = "mline-pilot")]
            let matches = matches
                || writer
                    .roundtrip
                    .and_then(|context| context.external_source(record.handle))
                    .is_some_and(|source| {
                        external_sources::external_source_location_to_cad_path(&source.location)
                            .is_ok_and(|path| {
                                dependency
                                    .file_name
                                    .replace('\\', "/")
                                    .eq_ignore_ascii_case(&path.replace('\\', "/"))
                            })
                    });
            matches
        });
        let Some(binding) = binding else {
            writer.losses.note(format!(
                "unbound FileDepList record {}: {}",
                dependency.feature, dependency.file_name
            ));
            continue;
        };
        let source_id = writer.external_sources[&binding.handle];
        let expected = writer
            .drawing
            .external_sources
            .iter()
            .find(|source| source.id == source_id)
            .and_then(|source| source.source.fingerprint_guid());
        if dependency
            .fingerprint_guid
            .as_deref()
            .and_then(ifccad::normalize_ifccad_fingerprint_guid)
            .as_deref()
            != expected
        {
            writer.losses.note(format!(
                "FileDepList identity differs from retained native source {}",
                dependency.file_name
            ));
        }
        let references = writer
            .drawing
            .model
            .entities
            .iter()
            .filter_map(IfccadEntity::as_native)
            .filter(|e| matches!(&e.kind, IfccadEntityKind::Xref(x) if x.source_id == source_id))
            .count();
        if dependency
            .fingerprint_guid
            .as_deref()
            .is_some_and(|g| ifccad::normalize_ifccad_fingerprint_guid(g).is_none())
        {
            writer.losses.note(format!(
                "invalid FileDepList FingerprintGUID for {}",
                dependency.file_name
            ));
        }
        if !dependency.found_path.is_empty()
            || dependency.version_guid.is_some()
            || dependency.timestamp != 0
            || dependency.file_size != 0
            || !dependency.affects_graphics
            || dependency.reference_count as usize != references
        {
            writer.losses.note(format!(
                "FileDepList metadata beyond native source identity for {}",
                dependency.file_name
            ));
        }
    }
}

pub(super) fn sources_in(
    drawing: &IfccadDocument,
    document: &mut Document,
    names: &mut Names,
    losses: &mut Losses,
) -> Result<(), Error> {
    for source in &drawing.external_sources {
        if document.block_records.get(&source.name).is_some()
            || drawing
                .blocks
                .iter()
                .any(|b| b.name.eq_ignore_ascii_case(&source.name))
        {
            losses.note(format!(
                "XREF source {} conflicts with a CAD block name",
                source.name
            ));
            continue;
        }
        let path = match external_sources::external_source_location_to_cad_path(&source.location) {
            Ok(path) => path,
            Err(error) => {
                // Keep an unresolved editing representation. CAD file export
                // separately diagnoses this unqualified locator.
                losses.retained.push(Loss {
                    handle: None,
                    message: format!(
                        "XREF location retained unresolved; CAD export unavailable: {error}"
                    ),
                });
                match &source.location {
                    ifccad::IfccadExternalSourceLocation::UriReference { uri } => uri.clone(),
                    _ => return Err(error),
                }
            }
        };
        let mut types: Vec<_> = drawing
            .model
            .entities
            .iter()
            .filter_map(IfccadEntity::as_native)
            .filter_map(|e| match &e.kind {
                IfccadEntityKind::Xref(x) if x.source_id == source.id => Some(x.reference_type),
                _ => None,
            })
            .collect();
        types.sort_by_key(|t| matches!(t, IfccadXrefReferenceType::Overlay));
        types.dedup();
        if types.is_empty() {
            types.push(IfccadXrefReferenceType::Attach);
        }
        for (index, reference_type) in types.into_iter().enumerate() {
            let mut name = source.name.clone();
            if index != 0 {
                let base = format!("{}$XREF_OVERLAY", source.name);
                name = base.clone();
                let mut suffix = 1;
                while document.block_records.get(&name).is_some()
                    || drawing
                        .blocks
                        .iter()
                        .any(|b| b.name.eq_ignore_ascii_case(&name))
                    || drawing
                        .external_sources
                        .iter()
                        .any(|s| s.id != source.id && s.name.eq_ignore_ascii_case(&name))
                {
                    name = format!("{base}_{suffix}");
                    suffix += 1;
                }
            }
            let mut record = BlockRecord::new(name.clone());
            record.handle = document.allocate_handle();
            record.block_entity_handle = document.allocate_handle();
            record.block_end_handle = document.allocate_handle();
            record.flags.is_xref = true;
            record.flags.is_xref_overlay = reference_type == IfccadXrefReferenceType::Overlay;
            record.xref_path = path.clone();
            record.base_point = vector(source.source.base_point());
            record.units = source.source.insertion_unit().map(unit_code).unwrap_or(0);
            let mut block = Block::new(record.name.clone(), record.base_point);
            block.common.handle = record.block_entity_handle;
            block.common.owner_handle = record.handle;
            let mut end = BlockEnd::new();
            end.common.handle = record.block_end_handle;
            end.common.owner_handle = record.handle;
            names
                .external_sources
                .insert((source.id, reference_type), name);
            document.block_records.add(record)?;
            document.add_entity(EntityType::Block(block))?;
            document.add_entity(EntityType::BlockEnd(end))?;
            if let Some(guid) = source.source.fingerprint_guid() {
                document
                .file_dependencies
                .push(acadrust::document::FileDependency {
                feature: "Acad:XRef".into(),
                file_name: path.clone(),
                found_path: String::new(),
                fingerprint_guid: Some(format!("{{{}}}", guid.to_ascii_uppercase())),
                version_guid: None,
                timestamp: 0,
                file_size: 0,
                affects_graphics: true,
                reference_count: drawing
                    .model
                    .entities
                    .iter()
                    .filter_map(IfccadEntity::as_native)
                    .filter(
                        |e| matches!(&e.kind,IfccadEntityKind::Xref(x) if x.source_id==source.id),
                    )
                    .count()
                    .try_into()?,
            });
            }
        }
    }
    Ok(())
}

pub(super) fn insert_in(xref: &IfccadXref, names: &Names) -> Option<EntityType> {
    let name = names
        .external_sources
        .get(&(xref.source_id, xref.reference_type))?;
    let (origin, normal, rotation) = placement_in(&xref.placement);
    let mut insert = Insert::new(name.clone(), origin);
    insert.normal = normal;
    insert.rotation = rotation;
    insert.set_x_scale(xref.scale);
    insert.set_y_scale(xref.scale);
    insert.set_z_scale(xref.scale);
    (insert.x_scale().to_bits() == xref.scale.to_bits()).then_some(EntityType::Insert(insert))
}

impl Writer<'_> {
    pub(super) fn external_source(
        &mut self,
        record: &BlockRecord,
    ) -> Result<Option<IfccadExternalSourceId>, Error> {
        if let Some(id) = self.external_sources.get(&record.handle) {
            return Ok(Some(*id));
        }
        #[cfg(feature = "mline-pilot")]
        let retained = self
            .roundtrip
            .map(|c| c.external_source_for_path(record.handle, &record.xref_path))
            .transpose()?
            .flatten();
        #[cfg(not(feature = "mline-pilot"))]
        let retained: Option<IfccadExternalSource> = None;
        let source = if let Some(source) = retained {
            #[cfg(feature = "mline-pilot")]
            let source = {
                let mut source = source;
                if self
                    .roundtrip
                    .as_ref()
                    .is_some_and(|c| c.external_source_name_changed(record))
                {
                    source.name = record.name.clone();
                }
                source
            };
            source
        } else {
            let location =
                match external_sources::external_source_location_from_cad_path(&record.xref_path) {
                    Ok(v) => v,
                    Err(e) => {
                        self.losses
                            .note(format!("XREF {} location: {e}", record.name));
                        return Ok(None);
                    }
                };
            let fingerprint_guid =
                external_sources::dwg_xref_fingerprint_guid(self.document, record)?;
            IfccadExternalSource {
                id: self.drawing.id_counters.allocate_external_source_id()?,
                name: record.name.clone(),
                location,
                source: if record.xref_path.to_ascii_lowercase().ends_with(".dxf") {
                    IfccadExternalSourceContent::Dxf {
                        base_point: array(record.base_point),
                        insertion_unit: (record.units != 0).then(|| unit_name(record.units)),
                        fingerprint_guid,
                    }
                } else {
                    IfccadExternalSourceContent::Dwg {
                        base_point: array(record.base_point),
                        insertion_unit: (record.units != 0).then(|| unit_name(record.units)),
                        fingerprint_guid,
                    }
                },
            }
        };
        let id = source.id;
        if let Some(existing) = self.drawing.external_sources.iter().find(|s| s.id == id) {
            if existing != &source {
                return Err(annotation::invalid(
                    "conflicting edits to shared XREF source",
                ));
            }
        } else {
            self.drawing.external_sources.push(source);
        }
        self.external_sources.insert(record.handle, id);
        Ok(Some(id))
    }

    pub(super) fn xref_kind(
        &mut self,
        insert: &Insert,
        record: &BlockRecord,
    ) -> Result<Option<IfccadEntityKind>, Error> {
        let owner = insert.common.owner_handle;
        let model = owner.is_null()
            || self
                .document
                .block_records
                .iter()
                .any(|b| b.handle == owner && b.is_model_space());
        let scales = [insert.x_scale(), insert.y_scale(), insert.z_scale()];
        let placement = placement_out(insert.insert_point, insert.normal, insert.rotation);
        if !model
            || insert.row_count != 1
            || insert.column_count != 1
            || !insert.attributes.is_empty()
            || !scales[0].is_finite()
            || scales[0] <= 0.
            || scales.iter().any(|s| *s != scales[0])
            || placement.x_axis[2].abs() > 1e-12
            || placement.y_axis[2].abs() > 1e-12
            || placement.x_axis[0] * placement.y_axis[1] - placement.x_axis[1] * placement.y_axis[0]
                <= 0.
        {
            self.losses.note(format!("XREF {} requires Model ownership, one attachment, no attributes, Z rotation and positive uniform scale",record.name));
            return Ok(None);
        }
        let Some(source_id) = self.external_source(record)? else {
            return Ok(None);
        };
        Ok(Some(IfccadEntityKind::Xref(IfccadXref {
            source_id,
            placement,
            scale: scales[0],
            reference_type: if record.flags.is_xref_overlay {
                IfccadXrefReferenceType::Overlay
            } else {
                IfccadXrefReferenceType::Attach
            },
        })))
    }
}
