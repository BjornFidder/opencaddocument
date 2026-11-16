//! Preserve MLINE roots through owner-scoped DocumentSelection backing.
use super::*;
use crate::objects::{ObjectType, SortEntitiesTable};
use crate::selection::*;
use crate::selection_storage;

mod retention;
pub use retention::IfccadRoundtripContext;
pub(super) use retention::SavedIdentities;
use retention::{Placement, RetainedResources};
use std::collections::HashSet;

const PROVIDER: &str = "opencaddocument";
const SCHEMA: &str = "opencaddocument.document-selection";
const ROOT_LINETYPE: &str = "root-linetype";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Owner<T = Handle> {
    Model,
    Paper(T),
    Block(T),
}

impl<T> Owner<T> {
    fn storage_version(self) -> u32 {
        match self {
            Self::Model => 1,
            Self::Paper(_) => 2,
            Self::Block(_) => 3,
        }
    }

    fn role(self) -> ContextRole {
        match self {
            Self::Model => ContextRole::ModelSpace,
            Self::Paper(_) => ContextRole::PaperSpace,
            Self::Block(_) => ContextRole::BlockDefinition,
        }
    }
}

fn error(message: impl std::fmt::Display) -> Error {
    annotation::invalid(&format!("MLINE preservation pilot: {message}"))
}

pub(super) fn capture_group(
    writer: &mut Writer<'_>,
    entities: &[&EntityType],
    owner: Owner,
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
            version: owner.storage_version(),
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
    let live_records: HashMap<_, _> = drawing
        .model
        .entities
        .iter()
        .filter_map(|e| e.as_opaque().map(|e| e.preservation_record_id))
        .map(|id| (id, Owner::Model))
        .chain(drawing.paper_layouts.iter().flat_map(|paper| {
            paper.entities.iter().filter_map(|e| {
                e.as_opaque()
                    .map(|e| (e.preservation_record_id, Owner::Paper(paper.id)))
            })
        }))
        .chain(drawing.blocks.iter().flat_map(|block| {
            block.entities.iter().filter_map(|e| {
                e.as_opaque()
                    .map(|e| (e.preservation_record_id, Owner::Block(block.id)))
            })
        }))
        .collect();
    let mut backing_records = HashMap::new();
    for record in &envelope.records {
        if let (Some(&owner), Some(backing)) =
            (live_records.get(&record.id), record.storage_backing)
        {
            if backing_records
                .insert(backing, owner)
                .is_some_and(|prior| prior != owner)
            {
                return Err(error("preservation backing is shared across owners"));
            }
        }
    }
    let mut used_sources = HashSet::new();
    for record in &envelope.records {
        let supported = if live_records.contains_key(&record.id) {
            record.storage_backing.is_some()
                && record.conditions.is_empty()
                && record.representation == ifccad::IfccadPreservationRepresentation::CodecTyped
        } else if let Some(&owner) = backing_records.get(&record.id) {
            record.bindings.is_empty()
                && record.conditions.is_empty()
                && record.representation == ifccad::IfccadPreservationRepresentation::CodecTyped
                && record.payload.as_ref().is_some_and(|p| {
                    p.schema == SCHEMA
                        && p.version == owner.storage_version()
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

pub(super) fn restore(
    drawing: &IfccadDocument,
    document: &mut Document,
    names: &Names,
    handles: &mut HashMap<u64, Handle>,
    losses: &mut Losses,
    mut context: Option<&mut IfccadRoundtripContext>,
    sheets: &[(Handle, &IfccadPaperLayout)],
) -> Result<(), Error> {
    let mut scopes = vec![(Owner::Model, drawing.model.entities.as_slice())];
    for &(block, sheet) in sheets {
        let layout = layout::layout_handle(document, block)
            .ok_or_else(|| error("missing imported paper layout"))?;
        if let Some(context) = context.as_deref_mut() {
            context.remember_layout(layout, sheet.id);
        }
        scopes.push((Owner::Paper(layout), &sheet.entities));
    }
    for block in &drawing.blocks {
        let record = names
            .blocks
            .get(&block.id)
            .and_then(|name| document.block_records.get(name));
        let Some(record) = record else {
            if block
                .entities
                .iter()
                .any(|entity| entity.as_opaque().is_some())
            {
                return Err(error("missing imported block for retained MLINE"));
            }
            continue;
        };
        if block
            .entities
            .iter()
            .any(|entity| entity.as_opaque().is_some())
            && !crate::selection::block_definition::is_local(document, record)
        {
            return Err(error("retained MLINE requires a local block definition"));
        }
        if let Some(context) = context.as_deref_mut() {
            context.remember_block(record.handle, block.id);
        }
        scopes.push((Owner::Block(record.handle), &block.entities));
    }
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
    for &(owner, entities) in &scopes {
        for opaque in entities.iter().filter_map(IfccadEntity::as_opaque) {
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
            if !selection.requirements().iter().any(|requirement| {
                requirement.source == key
                    && requirement.slot == SelectionSlot::Owner
                    && requirement.disposition == RequirementDisposition::Context(owner.role())
            }) {
                return Err(error(
                    "preservation root owner does not match its drawing scope",
                ));
            }
            roots.insert(record.id, key);
        }
    }
    let mut blocked = Vec::new();
    // Insert from back to front so consecutive roots share the same order
    // semantics as roots separated by native entities.
    for (owner, entities) in scopes {
        for (i, opaque, record) in entities.iter().enumerate().rev().filter_map(|(i, e)| {
            let opaque = e.as_opaque()?;
            Some((i, opaque, records[&opaque.preservation_record_id]))
        }) {
            let following = &entities[i + 1..];
            let order = following
                .iter()
                .find_map(|e| handles.get(&e.id()).copied())
                .map_or(SelectionDrawOrder::Append, SelectionDrawOrder::Before);
            let target = RestoreTarget {
                owner,
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
                blocked.push((opaque, record, pattern, reason, owner, following));
            } else if let Some(context) = context.as_deref_mut() {
                context.remember_mline(handles[&opaque.id], record);
            }
        }
    }
    if let Some(context) = context {
        context.remember_entities(handles);
        context.remember_resources(document, names);
        for (opaque, record, pattern, reason, owner, following) in blocked.into_iter().rev() {
            let anchors = following
                .iter()
                .filter_map(|e| handles.get(&e.id()).copied())
                .collect();
            let resources = RetainedResources::from_import(document, names, opaque, pattern)?;
            context.retain(
                drawing,
                opaque,
                record,
                reason,
                resources,
                Placement {
                    owner,
                    following: anchors,
                },
            )?;
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
    owner: Owner,
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
        owner,
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
    let roots = vec![RootReconstruction {
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
    }];
    let outcome = match owner {
        Owner::Model => selection.reconstruct_available(
            document,
            &ReconstructionRequest {
                model_space: document.header.model_space_block_handle,
                coordinates: CoordinatePolicy::DrawingCoordinates,
                roots,
                resources,
                external: vec![],
            },
        ),
        Owner::Paper(layout) => selection.reconstruct_paperspace(
            document,
            &PaperSpaceReconstructionRequest {
                layout,
                coordinates: CoordinatePolicy::DrawingCoordinates,
                roots,
                resources,
                external: vec![],
            },
        ),
        Owner::Block(block_record) => selection.reconstruct_in_block_definition(
            document,
            &BlockDefinitionReconstructionRequest {
                block_record,
                coordinates: CoordinatePolicy::DrawingCoordinates,
                roots,
                resources,
                external: vec![],
            },
        ),
    }
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
    ordered_entities(
        document,
        document.header.model_space_block_handle,
        document.model_space_entities().collect(),
    )
}

pub(super) fn ordered_entities<'a>(
    document: &Document,
    block: Handle,
    mut entities: Vec<&'a EntityType>,
) -> Vec<&'a EntityType> {
    let Some(dict) = document
        .extension_dictionary_handle(block)
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
