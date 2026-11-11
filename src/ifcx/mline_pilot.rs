//! Local pilot for preserving independent ModelSpace MLINE snapshots.
use super::*;
use crate::objects::{ObjectType, SortEntitiesTable};
use crate::selection::*;
use crate::selection_storage;

mod retention;
pub use retention::IfccadRoundtripContext;
use retention::RetainedResources;
use std::collections::HashSet;

const PROVIDER: &str = "opencaddocument";
const SCHEMA: &str = "opencaddocument.mline-pilot";
const ROOT_LINETYPE: &str = "root-linetype";

fn error(message: impl std::fmt::Display) -> Error {
    annotation::invalid(&format!("MLINE preservation pilot: {message}"))
}

pub(super) fn capture(writer: &mut Writer<'_>, entity: &EntityType) -> Result<IfccadEntity, Error> {
    use ifccad::*;
    let common = entity.common();
    let layer = writer
        .document
        .layers
        .get(&common.layer)
        .ok_or_else(|| error("missing source layer"))?;
    let layer_id = *writer
        .layers
        .get(&layer.name.to_ascii_lowercase())
        .ok_or_else(|| error("source layer did not convert"))?;
    let id = writer.drawing.id_counters.allocate_entity_id()?;
    let record_id = writer
        .drawing
        .id_counters
        .allocate_preservation_record_id()?;
    let namespace = format!("mline-{id}");
    let mut request = CaptureRequest::new(&namespace, vec![common.handle]);
    request.external.insert(SelectionRecordKey {
        kind: SelectionRecordKind::Layer,
        handle: layer.handle,
    });
    let selection = DocumentSelection::capture(writer.document, &request)
        .map_err(error)?
        .selection;
    let mut dependency_coverage = IfccadPreservationDependencyCoverage::Qualified;
    for evidence in selection.capture_report() {
        match evidence.code {
            CaptureEvidenceCode::ResolvedSourceDefault => continue,
            CaptureEvidenceCode::MissingDependency => {
                dependency_coverage = IfccadPreservationDependencyCoverage::Unknown;
            }
            _ => {}
        }
        let message = match &evidence.slot {
            Some(slot) => format!("MLINE snapshot {slot:?}: {}", evidence.message),
            None => format!("MLINE snapshot: {}", evidence.message),
        };
        writer.losses.retained_note(Loss {
            handle: Some(common.handle),
            message,
        });
    }
    let bytes = selection_storage::encode(&selection).map_err(error)?;
    let appearance = appearance_out(common, &writer.line_patterns, &mut writer.losses);
    let preservation = writer
        .drawing
        .preservation
        .get_or_insert_with(empty_preservation);
    preservation.sources.push(IfccadPreservationSource {
        id: namespace.clone(),
        provider: PROVIDER.into(),
        provider_revision: "mline-pilot-1".into(),
        origin: IfccadPreservationOrigin::CadDocument,
        source_version: None,
    });
    preservation.records.push(IfccadPreservationRecord {
        id: record_id,
        source_id: namespace,
        source_key: common.handle.value().to_string(),
        category: IfccadPreservationCategory::Entity,
        role: IfccadPreservationRole::Complete,
        representation: IfccadPreservationRepresentation::CodecTyped,
        subject: Some(IfccadPreservationTarget::Entity(id)),
        dependency_coverage,
        bindings: vec![],
        conditions: vec![],
        payload: IfccadPreservationPayload {
            schema: SCHEMA.into(),
            version: 1,
            kind: IfccadPreservationPayloadKind::AdapterSnapshot,
            bytes,
        },
    });
    writer.entity_ids.insert(common.handle, id);
    Ok(IfccadEntity::Opaque(IfccadOpaqueEntity {
        id,
        preservation_record_id: record_id,
        layer_id: Some(layer_id),
        visible: !common.invisible,
        appearance: Some(IfccadOpaqueAppearance {
            appearance,
            line_pattern_scale: common.linetype_scale,
        }),
    }))
}

fn empty_preservation() -> ifccad::IfccadPreservation {
    ifccad::IfccadPreservation {
        version: 1,
        sources: Vec::new(),
        records: Vec::new(),
    }
}

/// Refuse editable import when the exporter would drop preservation it does
/// not understand. Core validation has already checked opaque subjects.
pub(super) fn check_preservation_scope(drawing: &IfccadDocument) -> Result<(), Error> {
    let Some(envelope) = &drawing.preservation else {
        return Ok(());
    };
    let model_records: HashSet<_> = drawing
        .model
        .entities
        .iter()
        .filter_map(|entity| {
            entity
                .as_opaque()
                .map(|opaque| opaque.preservation_record_id)
        })
        .collect();
    let mut used_sources = HashSet::new();
    for record in &envelope.records {
        if !model_records.contains(&record.id) || record.payload.schema != SCHEMA {
            return Err(error(format!(
                "unsupported preservation record {}; editing could discard it",
                record.id.0
            )));
        }
        used_sources.insert(record.source_id.as_str());
    }
    for source in &envelope.sources {
        if source.provider != PROVIDER || !used_sources.contains(source.id.as_str()) {
            return Err(error(format!(
                "unsupported preservation source {:?}; editing could discard it",
                source.id
            )));
        }
    }
    Ok(())
}

fn record<'a>(
    drawing: &'a IfccadDocument,
    opaque: &ifccad::IfccadOpaqueEntity,
) -> Option<&'a ifccad::IfccadPreservationRecord> {
    drawing
        .preservation
        .as_ref()?
        .records
        .iter()
        .find(|r| r.id == opaque.preservation_record_id && r.payload.schema == SCHEMA)
}

pub(super) fn recognizes(drawing: &IfccadDocument, entity: &IfccadEntity) -> bool {
    entity
        .as_opaque()
        .and_then(|e| record(drawing, e))
        .is_some()
}

pub(super) fn restore(
    drawing: &IfccadDocument,
    document: &mut Document,
    names: &Names,
    handles: &mut HashMap<u64, Handle>,
    losses: &mut Losses,
    context: Option<&mut IfccadRoundtripContext>,
) -> Result<(), Error> {
    let mut blocked = Vec::new();
    // Insert from back to front so consecutive roots share the same order
    // semantics as roots separated by native entities.
    for (i, opaque, record) in
        drawing
            .model
            .entities
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(i, e)| {
                let opaque = e.as_opaque()?;
                Some((i, opaque, record(drawing, opaque)?))
            })
    {
        let following = &drawing.model.entities[i + 1..];
        let order = following
            .iter()
            .find_map(|e| handles.get(&e.id()).copied())
            .map_or(SelectionDrawOrder::Append, SelectionDrawOrder::Before);
        let target = RestoreTarget {
            document,
            names,
            handles,
            losses,
        };
        let pattern = root_pattern_binding(record)?;
        if let Some(reason) = restore_one(opaque, record, pattern, order, target)? {
            blocked.push((opaque, record, pattern, reason, following));
        }
    }
    if let Some(context) = context {
        for (opaque, record, pattern, reason, following) in blocked.into_iter().rev() {
            let anchors = following
                .iter()
                .filter_map(|e| handles.get(&e.id()).copied())
                .collect();
            let resources = RetainedResources::from_import(document, names, opaque, pattern)?;
            context.retain(drawing, opaque, record, reason, resources, anchors)?;
        }
    } else if !blocked.is_empty() {
        return Err(error("blocked MLINE requires an IFCCAD roundtrip context"));
    }
    Ok(())
}

fn root_pattern_binding(
    record: &ifccad::IfccadPreservationRecord,
) -> Result<Option<IfccadLinePatternId>, Error> {
    use ifccad::IfccadPreservationTarget;
    match record.bindings.as_slice() {
        [] => Ok(None),
        [binding] if binding.slot == ROOT_LINETYPE => match binding.target {
            IfccadPreservationTarget::LinePattern(id) => Ok(Some(id)),
            _ => Err(error("root linetype must target a line pattern")),
        },
        _ => Err(error("unsupported preservation bindings")),
    }
}

struct RestoreTarget<'a> {
    document: &'a mut Document,
    names: &'a Names,
    handles: &'a mut HashMap<u64, Handle>,
    losses: &'a mut Losses,
}

fn restore_one(
    opaque: &ifccad::IfccadOpaqueEntity,
    record: &ifccad::IfccadPreservationRecord,
    pattern: Option<IfccadLinePatternId>,
    order: SelectionDrawOrder,
    target: RestoreTarget<'_>,
) -> Result<Option<String>, Error> {
    let RestoreTarget {
        document,
        names,
        handles,
        losses,
    } = target;
    if record.payload.version != 1
        || record.payload.kind != ifccad::IfccadPreservationPayloadKind::AdapterSnapshot
        || !record.conditions.is_empty()
    {
        return Err(error("unsupported preservation record"));
    }
    let selection = selection_storage::decode(&record.payload.bytes).map_err(error)?;
    let [root] = selection.roots() else {
        return Err(error("expected one selection root"));
    };
    if selection.namespace() != record.source_id
        || root.handle.value().to_string() != record.source_key
    {
        return Err(error("preservation record does not match its selection"));
    }
    let Some(SelectionPayload::MLine(source)) = selection
        .records()
        .iter()
        .find(|r| r.key == *root)
        .map(|r| &r.payload)
    else {
        return Err(error("selection root is not an MLINE"));
    };
    let Some(layer) = opaque
        .layer_id
        .and_then(|id| names.layer_handles.get(&id))
        .copied()
    else {
        return Ok(Some("Missing current layer".into()));
    };
    let mut current = source.common.clone();
    if let Some(id) = pattern {
        let Some(name) = names.line_patterns.get(&id) else {
            return Ok(Some(format!("Missing required line pattern {}", id.0)));
        };
        current.linetype.clone_from(name);
    }
    if let Some(appearance) = &opaque.appearance {
        appearance_in(&appearance.appearance, names, &mut current, losses);
        current.linetype_scale = appearance.line_pattern_scale;
    }
    let linetype = match current.linetype.to_ascii_lowercase().as_str() {
        "" | "bylayer" => RootLineType::ByLayer,
        "byblock" => RootLineType::ByBlock,
        _ => {
            let Some(pattern) = document.line_types.get(&current.linetype) else {
                return Ok(Some("Missing current linetype".into()));
            };
            RootLineType::Explicit(pattern.handle)
        }
    };
    let mut resources = BTreeMap::new();
    for r in selection.records() {
        let name = match &r.payload {
            SelectionPayload::MLineStyle(style) if style.name.eq_ignore_ascii_case("Standard") => {
                resources.insert(r.key, ResourceAction::Reuse(document.header.current_multiline_style_handle));
                continue;
            }
            SelectionPayload::MLineStyle(style) => {
                if document.objects.values().any(|v| matches!(v, ObjectType::MLineStyle(x) if x.name.eq_ignore_ascii_case(&style.name))) {
                    format!("Preserved MLINE {}", opaque.id)
                } else {
                    style.name.clone()
                }
            }
            SelectionPayload::LineType(pattern) => {
                if let Some(existing) = document.line_types.get(&pattern.name) {
                    // D3 checks compatibility before binding the style to it.
                    resources.insert(r.key, ResourceAction::Reuse(existing.handle));
                    continue;
                }
                pattern.name.clone()
            }
            _ => continue,
        };
        resources.insert(r.key, ResourceAction::Create { name });
    }
    let request = ReconstructionRequest {
        model_space: document.header.model_space_block_handle,
        coordinates: CoordinatePolicy::DrawingCoordinates,
        roots: vec![RootReconstruction {
            root: *root,
            transform: SelectionPlacement::Identity,
            order,
            layer: Some(layer),
            linetype: Some(linetype),
            appearance: RootAppearanceOverrides {
                color: opaque.appearance.as_ref().map(|_| current.color),
                line_weight: Some(current.line_weight),
                transparency: Some(current.transparency),
                invisible: Some(!opaque.visible),
            },
        }],
        resources,
        external: vec![],
    };
    let outcome = selection
        .reconstruct_available(document, &request)
        .map_err(error)?;
    let entity = match &outcome.roots[0].status {
        RootReconstructionStatus::Applied { entity, .. } => *entity,
        RootReconstructionStatus::Blocked { reasons } => {
            let reason = reasons
                .iter()
                .map(|reason| reason.message.as_str())
                .collect::<Vec<_>>();
            return Ok(Some(reason.join(", ")));
        }
    };
    let mut candidate = outcome.document;
    candidate
        .get_entity_mut(entity)
        .ok_or_else(|| error("missing reconstructed MLINE"))?
        .common_mut()
        .linetype_scale = current.linetype_scale;
    handles.insert(opaque.id, entity);
    *document = candidate;
    Ok(None)
}

pub(super) fn model_entities(document: &Document) -> Vec<&EntityType> {
    let mut entities: Vec<_> = document.model_space_entities().collect();
    let Some(dict) = document
        .extension_dictionary_handle(document.header.model_space_block_handle)
        .and_then(|h| document.objects.get(&h))
    else {
        return entities;
    };
    let ObjectType::Dictionary(dict) = dict else {
        return entities;
    };
    let Some(ObjectType::SortEntitiesTable(sort)) = dict
        .get(SortEntitiesTable::DICTIONARY_KEY)
        .and_then(|h| document.objects.get(&h))
    else {
        return entities;
    };
    entities.sort_by_key(|e| sort.effective_sort_handle(e.common().handle).value());
    entities
}
