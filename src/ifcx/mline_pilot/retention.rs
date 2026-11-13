use super::{empty_preservation, error, Handle, IfccadDocument, IfccadEntity, IfccadLinePatternId};
use crate::ifcx::{Names, Writer};
use crate::{Document, Error};
use ifccad::{
    IfccadIdCounters, IfccadMode, IfccadOpaqueEntity, IfccadPreservationBinding,
    IfccadPreservationRecord, IfccadPreservationRecordId, IfccadPreservationSource,
    IfccadPreservationTarget,
};
use std::collections::{BTreeMap, HashMap};

/// IFCCAD state that belongs to the editing session of an imported document.
///
/// Keep this value with the document and pass it to the context-aware exporter.
/// CAD handles inside it refer to that session and are rebuilt on reopen.
#[derive(Clone, Debug)]
pub struct IfccadRoundtripContext {
    header_id: String,
    drawing_id: u64,
    counters: IfccadIdCounters,
    blocked: Vec<BlockedMline>,
    packages: BTreeMap<IfccadPreservationRecordId, IfccadPreservationRecord>,
    entities: HashMap<Handle, u64>,
    layers: HashMap<Handle, u64>,
    patterns: HashMap<Handle, IfccadLinePatternId>,
    model_id: u64,
    roots: HashMap<Handle, (IfccadPreservationRecordId, Vec<IfccadPreservationBinding>)>,
}

#[derive(Clone, Debug)]
struct BlockedMline {
    opaque: IfccadOpaqueEntity,
    record: IfccadPreservationRecord,
    source: IfccadPreservationSource,
    reason: String,
    resources: RetainedResources,
    following: Vec<Handle>,
}

#[derive(Clone, Debug)]
pub(super) struct RetainedResources {
    layer: Option<Handle>,
    appearance_pattern: Option<Handle>,
    root_pattern: Option<(IfccadLinePatternId, Option<Handle>)>,
}

impl RetainedResources {
    pub(super) fn from_import(
        document: &Document,
        names: &Names,
        opaque: &IfccadOpaqueEntity,
        pattern: Option<IfccadLinePatternId>,
    ) -> Result<Self, Error> {
        let layer = opaque
            .layer_id
            .map(|id| {
                names
                    .layer_handles
                    .get(&id)
                    .copied()
                    .ok_or_else(|| error("missing retained layer"))
            })
            .transpose()?;
        let appearance_pattern = match opaque
            .appearance
            .as_ref()
            .map(|a| &a.appearance.line_pattern)
        {
            Some(IfccadMode::Explicit(id)) => Some(
                names
                    .line_patterns
                    .get(id)
                    .and_then(|name| document.line_types.get(name))
                    .ok_or_else(|| error("missing retained appearance pattern"))?
                    .handle,
            ),
            _ => None,
        };
        Ok(Self {
            layer,
            appearance_pattern,
            root_pattern: pattern.map(|id| {
                let handle = names
                    .line_patterns
                    .get(&id)
                    .and_then(|name| document.line_types.get(name))
                    .map(|pattern| pattern.handle);
                (id, handle)
            }),
        })
    }

    fn remap(
        &self,
        writer: &Writer<'_>,
        opaque: &mut IfccadOpaqueEntity,
        record: &mut IfccadPreservationRecord,
    ) -> Result<(), Error> {
        if let Some(handle) = self.layer {
            let id = writer
                .document
                .layers
                .iter()
                .find(|entry| entry.handle == handle)
                .and_then(|layer| writer.layers.get(&layer.name.to_ascii_lowercase()))
                .copied()
                .ok_or_else(|| error("retained MLINE layer is unavailable for export"))?;
            opaque.layer_id = Some(id);
        }
        if let (Some(handle), Some(appearance)) = (self.appearance_pattern, &mut opaque.appearance)
        {
            let id = exported_pattern(writer, handle).ok_or_else(|| {
                error("retained MLINE appearance pattern is unavailable for export")
            })?;
            appearance.appearance.line_pattern = IfccadMode::Explicit(id);
        }
        if let (Some((_, Some(handle))), [binding]) =
            (self.root_pattern, record.bindings.as_mut_slice())
        {
            // If the resource was deleted, the original reserved ID stays
            // unresolved. A new same-named pattern cannot take its place.
            if let Some(id) = exported_pattern(writer, handle) {
                binding.target = IfccadPreservationTarget::LinePattern(id);
            }
        }
        Ok(())
    }
}

fn exported_pattern(writer: &Writer<'_>, handle: Handle) -> Option<IfccadLinePatternId> {
    writer
        .document
        .line_types
        .iter()
        .find(|entry| entry.handle == handle)
        .and_then(|pattern| writer.line_patterns.get(&pattern.name.to_ascii_lowercase()))
        .copied()
}

impl IfccadRoundtripContext {
    pub(in crate::ifcx) fn new(drawing: &IfccadDocument) -> Self {
        Self {
            header_id: drawing.header.id.clone(),
            drawing_id: drawing.drawing_id,
            counters: drawing.id_counters,
            blocked: Vec::new(),
            packages: BTreeMap::new(),
            entities: HashMap::new(),
            roots: HashMap::new(),
            layers: HashMap::new(),
            patterns: HashMap::new(),
            model_id: drawing.model.id,
        }
    }

    /// IFCCAD identities and diagnostic reasons for live roots without CAD geometry.
    pub fn blocked_entities(&self) -> impl Iterator<Item = (u64, &str)> {
        self.blocked
            .iter()
            .map(|item| (item.opaque.id, item.reason.as_str()))
    }

    pub(super) fn retain(
        &mut self,
        drawing: &IfccadDocument,
        opaque: &IfccadOpaqueEntity,
        record: &IfccadPreservationRecord,
        reason: String,
        resources: RetainedResources,
        following: Vec<Handle>,
    ) -> Result<(), Error> {
        let source = drawing
            .preservation
            .iter()
            .flat_map(|envelope| &envelope.sources)
            .find(|source| source.id == record.source_id)
            .ok_or_else(|| error("missing preservation source"))?;
        let backing = drawing
            .preservation
            .iter()
            .flat_map(|envelope| &envelope.records)
            .find(|backing| Some(backing.id) == record.storage_backing)
            .ok_or_else(|| error("missing preservation backing"))?;
        self.packages
            .entry(backing.id)
            .or_insert_with(|| backing.clone());
        self.blocked.push(BlockedMline {
            opaque: opaque.clone(),
            record: record.clone(),
            source: source.clone(),
            reason,
            resources,
            following,
        });
        Ok(())
    }

    pub(in crate::ifcx) fn remember_entities(&mut self, handles: &HashMap<u64, Handle>) {
        self.entities
            .extend(handles.iter().map(|(&id, &handle)| (handle, id)));
    }

    pub(super) fn remember_mline(&mut self, handle: Handle, record: &IfccadPreservationRecord) {
        self.roots
            .insert(handle, (record.id, record.bindings.clone()));
    }

    pub(in crate::ifcx) fn entity_id(&self, handle: Handle) -> Option<u64> {
        self.entities.get(&handle).copied()
    }

    pub(in crate::ifcx) fn layer_id(&self, handle: Handle) -> Option<u64> {
        self.layers.get(&handle).copied()
    }

    pub(in crate::ifcx) fn pattern_id(&self, handle: Handle) -> Option<IfccadLinePatternId> {
        self.patterns.get(&handle).copied()
    }

    pub(super) fn package_namespace(&self, id: IfccadPreservationRecordId) -> String {
        let base = format!("selection-{}", id.0);
        let used = |name: &str| {
            self.packages.values().any(|r| r.source_key == name)
                || self.blocked.iter().any(|root| root.source.id == name)
        };
        if !used(&base) {
            return base;
        }
        (1..)
            .map(|suffix| format!("{base}-{suffix}"))
            .find(|name| !used(name))
            .expect("finite set of package names")
    }

    pub(in crate::ifcx) fn remember_resources(&mut self, document: &Document, names: &Names) {
        self.layers.extend(
            names
                .layer_handles
                .iter()
                .map(|(&id, &handle)| (handle, id)),
        );
        self.patterns
            .extend(names.line_patterns.iter().filter_map(|(&id, name)| {
                document
                    .line_types
                    .get(name)
                    .map(|pattern| (pattern.handle, id))
            }));
    }

    pub(in crate::ifcx) fn mline_identity(
        &self,
        handle: Handle,
    ) -> Option<(IfccadPreservationRecordId, Vec<IfccadPreservationBinding>)> {
        self.roots.get(&handle).cloned()
    }

    pub(in crate::ifcx) fn reserve_ids(
        &self,
        document: &Document,
        drawing: &mut IfccadDocument,
    ) -> Result<(), Error> {
        if drawing.drawing_id != self.drawing_id || drawing.header.id != self.header_id {
            return Err(error(
                "roundtrip context belongs to a different IFCCAD drawing",
            ));
        }
        drawing.id_counters = self.counters;
        drawing.model.id = self.model_id;
        if !document
            .line_types
            .iter()
            .any(|pattern| self.pattern_id(pattern.handle) == Some(IfccadLinePatternId(0)))
        {
            drawing.line_patterns.retain(|pattern| pattern.id.0 != 0);
        }
        Ok(())
    }

    pub(in crate::ifcx) fn remember_export(&mut self, saved: SavedIdentities) {
        self.counters = saved.counters;
        self.entities.extend(saved.entities);
        self.layers.extend(saved.layers);
        self.patterns.extend(saved.patterns);
        self.roots.extend(saved.roots);
    }

    pub(in crate::ifcx) fn emit(&self, writer: &mut Writer<'_>) -> Result<(), Error> {
        for item in &self.blocked {
            let mut opaque = item.opaque.clone();
            let mut record = item.record.clone();
            item.resources.remap(writer, &mut opaque, &mut record)?;
            let model = &mut writer.drawing.model.entities;
            let position = item
                .following
                .iter()
                .find_map(|handle| {
                    let id = writer.entity_ids.get(handle)?;
                    model.iter().position(|entity| entity.id() == *id)
                })
                .unwrap_or(model.len());
            model.insert(position, IfccadEntity::Opaque(opaque));
            let envelope = writer
                .drawing
                .preservation
                .get_or_insert_with(empty_preservation);
            match envelope
                .sources
                .iter()
                .find(|source| source.id == item.source.id)
            {
                Some(source) if source != &item.source => {
                    return Err(error("preservation source identity collision"));
                }
                Some(_) => {}
                None => envelope.sources.push(item.source.clone()),
            }
            let backing = record.storage_backing.expect("retained backing");
            if !envelope.records.iter().any(|r| r.id == backing) {
                envelope.records.push(self.packages[&backing].clone());
            }
            envelope.records.push(record);
        }
        if let Some(envelope) = &mut writer.drawing.preservation {
            envelope.records.sort_by_key(|r| r.id);
        }
        Ok(())
    }
}

pub(in crate::ifcx) struct SavedIdentities {
    counters: IfccadIdCounters,
    entities: HashMap<Handle, u64>,
    layers: HashMap<Handle, u64>,
    patterns: HashMap<Handle, IfccadLinePatternId>,
    roots: HashMap<Handle, (IfccadPreservationRecordId, Vec<IfccadPreservationBinding>)>,
}

impl SavedIdentities {
    pub(in crate::ifcx) fn from_writer(writer: &Writer<'_>) -> Self {
        let entities = writer.entity_ids.clone();
        let layers = writer
            .document
            .layers
            .iter()
            .filter_map(|layer| {
                writer
                    .layers
                    .get(&layer.name.to_ascii_lowercase())
                    .map(|&id| (layer.handle, id))
            })
            .collect();
        let patterns = writer
            .document
            .line_types
            .iter()
            .filter_map(|pattern| {
                exported_pattern(writer, pattern.handle).map(|id| (pattern.handle, id))
            })
            .collect();
        let handles: HashMap<_, _> = entities.iter().map(|(&h, &id)| (id, h)).collect();
        let roots = writer
            .drawing
            .preservation
            .iter()
            .flat_map(|p| &p.records)
            .filter_map(|record| {
                let Some(IfccadPreservationTarget::Entity(id)) = record.subject else {
                    return None;
                };
                handles
                    .get(&id)
                    .map(|&handle| (handle, (record.id, record.bindings.clone())))
            })
            .collect();
        Self {
            counters: writer.drawing.id_counters,
            entities,
            layers,
            patterns,
            roots,
        }
    }
}
