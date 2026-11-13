//! Preserve ModelSpace MLINE roots through shared DocumentSelection backing.
use super::*;
use crate::objects::{ObjectType, SortEntitiesTable};
use crate::selection::*;
use crate::selection_storage;

mod retention;
pub use retention::IfccadRoundtripContext;
use retention::RetainedResources;
pub(super) use retention::SavedIdentities;
use std::collections::HashSet;

const PROVIDER: &str = "opencaddocument";
const SCHEMA: &str = "opencaddocument.document-selection";
const ROOT_LINETYPE: &str = "root-linetype";

fn error(message: impl std::fmt::Display) -> Error {
    annotation::invalid(&format!("MLINE preservation pilot: {message}"))
}

pub(super) fn capture_group(
    writer: &mut Writer<'_>,
    entities: &[&EntityType],
) -> Result<HashMap<Handle, IfccadEntity>, Error> {
    use ifccad::*;
    let roots: Vec<_> = entities
        .iter()
        .copied()
        .filter(|e| matches!(e, EntityType::MLine(_)))
        .collect();
    if roots.is_empty() {
        return Ok(HashMap::new());
    }
    let backing = writer
        .drawing
        .id_counters
        .allocate_preservation_record_id()?;
    let namespace = writer.roundtrip.map_or_else(
        || format!("selection-{}", backing.0),
        |context| context.package_namespace(backing),
    );
    let mut request = CaptureRequest::new(
        &namespace,
        roots.iter().map(|e| e.common().handle).collect(),
    );
    for e in &roots {
        let layer = writer
            .document
            .layers
            .get(&e.common().layer)
            .ok_or_else(|| error("missing source layer"))?;
        request.external.insert(SelectionRecordKey {
            kind: SelectionRecordKind::Layer,
            handle: layer.handle,
        });
    }
    let selection = DocumentSelection::capture(writer.document, &request)
        .map_err(error)?
        .selection;
    let bytes = selection_storage::encode(&selection).map_err(error)?;
    let mut output = HashMap::new();
    let mut records = vec![IfccadPreservationRecord {
        id: backing,
        source_id: namespace.clone(),
        source_key: namespace.clone(),
        category: IfccadPreservationCategory::Shared,
        role: IfccadPreservationRole::Shared,
        representation: IfccadPreservationRepresentation::CodecTyped,
        subject: None,
        dependency_coverage: IfccadPreservationDependencyCoverage::Conservative,
        bindings: vec![],
        conditions: vec![],
        storage_backing: None,
        payload: Some(IfccadPreservationPayload {
            schema: SCHEMA.into(),
            version: 1,
            kind: IfccadPreservationPayloadKind::AdapterSnapshot,
            bytes,
        }),
    }];
    for entity in roots {
        let common = entity.common();
        let layer_id = *writer
            .layers
            .get(&common.layer.to_ascii_lowercase())
            .ok_or_else(|| error("source layer did not convert"))?;
        let id = writer.entity_id(common.handle)?;
        let (record_id, bindings) = writer.mline_identity(common.handle)?;
        let mut reachable = HashSet::from([SelectionRecordKey {
            kind: SelectionRecordKind::Entity,
            handle: common.handle,
        }]);
        let mut pending: Vec<_> = reachable.iter().copied().collect();
        while let Some(key) = pending.pop() {
            for requirement in selection.requirements().iter().filter(|r| r.source == key) {
                if let RequirementDisposition::Included(key) = requirement.disposition {
                    if reachable.insert(key) {
                        pending.push(key);
                    }
                }
            }
        }
        let mut coverage = IfccadPreservationDependencyCoverage::Qualified;
        for evidence in selection
            .capture_report()
            .iter()
            .filter(|e| e.source.is_none_or(|key| reachable.contains(&key)))
        {
            if evidence.code == CaptureEvidenceCode::ResolvedSourceDefault {
                continue;
            }
            if evidence.code == CaptureEvidenceCode::MissingDependency {
                coverage = IfccadPreservationDependencyCoverage::Unknown;
            }
            writer.losses.retained_note(Loss {
                handle: Some(common.handle),
                message: evidence.slot.as_ref().map_or_else(
                    || evidence.message.clone(),
                    |slot| format!("{slot:?}: {}", evidence.message),
                ),
            });
        }
        let appearance = appearance_out(common, &writer.line_patterns, &mut writer.losses);
        records.push(IfccadPreservationRecord {
            id: record_id,
            source_id: namespace.clone(),
            source_key: format!("{namespace}/entity/{}", common.handle.value()),
            category: IfccadPreservationCategory::Entity,
            role: IfccadPreservationRole::Complete,
            representation: IfccadPreservationRepresentation::CodecTyped,
            subject: Some(IfccadPreservationTarget::Entity(id)),
            dependency_coverage: coverage,
            bindings,
            conditions: vec![],
            storage_backing: Some(backing),
            payload: None,
        });
        output.insert(
            common.handle,
            IfccadEntity::Opaque(IfccadOpaqueEntity {
                id,
                preservation_record_id: record_id,
                layer_id: Some(layer_id),
                visible: !common.invisible,
                appearance: Some(IfccadOpaqueAppearance {
                    appearance,
                    line_pattern_scale: common.linetype_scale,
                }),
            }),
        );
    }
    let preservation = writer
        .drawing
        .preservation
        .get_or_insert_with(empty_preservation);
    preservation.sources.push(IfccadPreservationSource {
        id: namespace,
        provider: PROVIDER.into(),
        provider_revision: "mline-selection-1".into(),
        origin: IfccadPreservationOrigin::CadDocument,
        source_version: None,
    });
    preservation.records.extend(records);
    Ok(output)
}

fn empty_preservation() -> ifccad::IfccadPreservation {
    ifccad::IfccadPreservation {
        version: 2,
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
        .filter_map(|e| e.as_opaque().map(|e| e.preservation_record_id))
        .collect();
    let backing_records: HashSet<_> = envelope
        .records
        .iter()
        .filter(|r| model_records.contains(&r.id))
        .filter_map(|r| r.storage_backing)
        .collect();
    let mut used_sources = HashSet::new();
    for record in &envelope.records {
        let supported = if model_records.contains(&record.id) {
            record.storage_backing.is_some()
                && record.conditions.is_empty()
                && record.representation == ifccad::IfccadPreservationRepresentation::CodecTyped
        } else if backing_records.contains(&record.id) {
            record.bindings.is_empty()
                && record.conditions.is_empty()
                && record.representation == ifccad::IfccadPreservationRepresentation::CodecTyped
                && record.payload.as_ref().is_some_and(|p| {
                    p.schema == SCHEMA
                        && p.version == 1
                        && p.kind == ifccad::IfccadPreservationPayloadKind::AdapterSnapshot
                })
        } else {
            false
        };
        if !supported {
            return Err(error(format!(
                "unsupported preservation record {}",
                record.id.0
            )));
        }
        used_sources.insert(record.source_id.as_str());
    }
    for source in &envelope.sources {
        if source.provider != PROVIDER
            || source.provider_revision != "mline-selection-1"
            || !used_sources.contains(source.id.as_str())
        {
            return Err(error(format!(
                "unsupported preservation source {:?}",
                source.id
            )));
        }
    }
    Ok(())
}

pub(super) fn recognizes(_drawing: &IfccadDocument, entity: &IfccadEntity) -> bool {
    entity.as_opaque().is_some()
}

pub(super) fn restore(
    drawing: &IfccadDocument,
    document: &mut Document,
    names: &Names,
    handles: &mut HashMap<u64, Handle>,
    losses: &mut Losses,
    context: Option<&mut IfccadRoundtripContext>,
) -> Result<(), Error> {
    let Some(envelope) = &drawing.preservation else {
        if let Some(context) = context {
            context.remember_entities(handles);
            context.remember_resources(document, names);
        }
        return Ok(());
    };
    let records: HashMap<_, _> = envelope.records.iter().map(|r| (r.id, r)).collect();
    let mut selections = HashMap::new();
    let mut claimed = HashSet::new();
    let mut roots = HashMap::new();
    let mut resources = HashMap::<_, BTreeMap<SelectionRecordKey, Handle>>::new();
    for opaque in drawing
        .model
        .entities
        .iter()
        .filter_map(IfccadEntity::as_opaque)
    {
        let record = records[&opaque.preservation_record_id];
        let id = record.storage_backing.expect("checked backing");
        let backing = records[&id];
        if let std::collections::hash_map::Entry::Vacant(entry) = selections.entry(id) {
            let selection = selection_storage::decode(
                &backing.payload.as_ref().expect("checked payload").bytes,
            )
            .map_err(error)?;
            if selection.namespace() != backing.source_key {
                return Err(error("preservation package namespace mismatch"));
            }
            entry.insert(selection);
        }
        let selection = &selections[&id];
        let prefix = format!("{}/entity/", selection.namespace());
        let handle = record
            .source_key
            .strip_prefix(&prefix)
            .and_then(|key| {
                let value = key.parse::<u64>().ok()?;
                (value != 0 && value.to_string() == key).then_some(Handle::new(value))
            })
            .ok_or_else(|| error("invalid preservation root key"))?;
        let key = SelectionRecordKey {
            kind: SelectionRecordKind::Entity,
            handle,
        };
        if !selection.roots().contains(&key)
            || !selection
                .records()
                .iter()
                .any(|r| r.key == key && matches!(r.payload, SelectionPayload::MLine(_)))
        {
            return Err(error("preservation root is not a captured MLINE"));
        }
        if !claimed.insert((id, key)) {
            return Err(error("duplicate preservation root"));
        }
        roots.insert(record.id, key);
    }
    let mut blocked = Vec::new();
    let mut context = context;
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
                Some((i, opaque, records[&opaque.preservation_record_id]))
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
        let id = record.storage_backing.expect("checked backing");
        let root = roots[&record.id];
        if let Some(reason) = restore_one(
            opaque,
            &selections[&id],
            root,
            resources.entry(id).or_default(),
            pattern,
            order,
            target,
        )? {
            blocked.push((opaque, record, pattern, reason, following));
        } else if let Some(context) = context.as_deref_mut() {
            context.remember_mline(handles[&opaque.id], record);
        }
    }
    if let Some(context) = context {
        context.remember_entities(handles);
        context.remember_resources(document, names);
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
    selection: &DocumentSelection,
    root: SelectionRecordKey,
    applied_resources: &mut BTreeMap<SelectionRecordKey, Handle>,
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
    let Some(SelectionPayload::MLine(source)) = selection
        .records()
        .iter()
        .find(|r| r.key == root)
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
        if let Some(handle) = applied_resources.get(&r.key) {
            resources.insert(r.key, ResourceAction::Reuse(*handle));
            continue;
        }
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
            root,
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
    applied_resources.extend(outcome.resources);
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
