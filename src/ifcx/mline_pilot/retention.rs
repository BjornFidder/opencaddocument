use super::{
    empty_preservation, error, Handle, IfccadDocument, IfccadEntity, IfccadLinePatternId, Owner,
};
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
    external_source_history: HashMap<(Handle, String), ifccad::IfccadExternalSource>,
    external_sources: HashMap<Handle, ifccad::IfccadExternalSource>,
    header_id: String,
    drawing_id: u64,
    counters: IfccadIdCounters,
    blocked: Vec<BlockedMline>,
    packages: BTreeMap<IfccadPreservationRecordId, IfccadPreservationRecord>,
    entities: HashMap<Handle, u64>,
    layers: HashMap<Handle, u64>,
    layouts: HashMap<Handle, u64>,
    blocks: HashMap<Handle, u64>,
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
    placement: Placement,
}

#[derive(Clone, Debug)]
pub(super) struct Placement {
    pub(super) owner: Owner,
    pub(super) following: Vec<Handle>,
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
            external_source_history: HashMap::new(),
            external_sources: HashMap::new(),
            header_id: drawing.header.id.clone(),
            drawing_id: drawing.drawing_id,
            counters: drawing.id_counters,
            blocked: Vec::new(),
            packages: BTreeMap::new(),
            entities: HashMap::new(),
            roots: HashMap::new(),
            layers: HashMap::new(),
            layouts: HashMap::new(),
            blocks: HashMap::new(),
            patterns: HashMap::new(),
            model_id: drawing.model.id,
        }
    }

    /// Native source metadata for this CAD block, independent of resolved bytes.
    pub fn external_source(&self, handle: Handle) -> Option<&ifccad::IfccadExternalSource> {
        self.external_sources.get(&handle)
    }

    /// Recover the authored source for a live CAD locator, including earlier
    /// locator spellings retained for undo after a native save.
    pub fn external_source_for_path(
        &self,
        handle: Handle,
        path: &str,
    ) -> Result<Option<ifccad::IfccadExternalSource>, Error> {
        if let Some(source) = self.external_source_history.get(&(handle, path.to_owned())) {
            return Ok(Some(source.clone()));
        }
        let Some(mut source) = self.external_sources.get(&handle).cloned() else {
            return Ok(None);
        };
        source.location = crate::external_source_location_from_cad_path(path)?;
        Ok(Some(source))
    }

    /// Register an explicitly attached source. Deleting/undoing its CAD block
    /// does not lower allocation history; redo reuses the same source identity.
    pub fn register_external_source(
        &mut self,
        handle: Handle,
        name: String,
        cad_path: String,
        content: ifccad::IfccadExternalSourceContent,
    ) -> Result<ifccad::IfccadExternalSourceId, Error> {
        let existing = self.external_sources.get(&handle).map(|s| s.id);
        let id = existing.unwrap_or(ifccad::IfccadExternalSourceId(
            self.counters.next_external_source_id,
        ));
        let source = ifccad::IfccadExternalSource {
            id,
            name,
            location: crate::external_source_location_from_cad_path(&cad_path)?,
            source: content,
        };
        source.validate()?;
        if existing.is_none() {
            self.counters.allocate_external_source_id()?;
        }
        self.external_source_history
            .insert((handle, cad_path), source.clone());
        self.external_sources.insert(handle, source);
        Ok(id)
    }

    pub(in crate::ifcx) fn remember_external_sources(
        &mut self,
        drawing: &IfccadDocument,
        document: &Document,
        names: &Names,
    ) {
        for source in &drawing.external_sources {
            if let Some(record) = names
                .external_sources
                .get(&source.id)
                .and_then(|name| document.block_records.get(name))
            {
                self.external_source_history
                    .insert((record.handle, record.xref_path.clone()), source.clone());
                self.external_sources.insert(record.handle, source.clone());
            }
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
        placement: Placement,
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
            placement,
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

    pub(super) fn remember_layout(&mut self, handle: Handle, id: u64) {
        self.layouts.insert(handle, id);
    }

    pub(in crate::ifcx) fn layout_id(&self, handle: Handle) -> Option<u64> {
        self.layouts.get(&handle).copied()
    }

    pub(super) fn remember_block(&mut self, handle: Handle, id: u64) {
        self.blocks.insert(handle, id);
    }

    pub(in crate::ifcx) fn block_id(&self, handle: Handle) -> Option<u64> {
        self.blocks.get(&handle).copied()
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
        self.external_sources.extend(saved.external_sources);
        self.external_source_history
            .extend(saved.external_source_history);
        self.entities.extend(saved.entities);
        self.layers.extend(saved.layers);
        self.layouts.extend(saved.layouts);
        self.blocks.extend(saved.blocks);
        self.patterns.extend(saved.patterns);
        self.roots.extend(saved.roots);
    }

    pub(in crate::ifcx) fn emit(&self, writer: &mut Writer<'_>) -> Result<(), Error> {
        for item in &self.blocked {
            let mut opaque = item.opaque.clone();
            let mut record = item.record.clone();
            item.resources.remap(writer, &mut opaque, &mut record)?;
            let entities = match item.placement.owner {
                Owner::Model => &mut writer.drawing.model.entities,
                Owner::Paper(layout) => {
                    let id = writer.layout_ids.get(&layout).ok_or_else(|| {
                        error("retained MLINE paper layout is unavailable for export")
                    })?;
                    &mut writer
                        .drawing
                        .paper_layouts
                        .iter_mut()
                        .find(|paper| paper.id == *id)
                        .ok_or_else(|| {
                            error("retained MLINE paper layout is unavailable for export")
                        })?
                        .entities
                }
                Owner::Block(handle) => {
                    let valid = writer.document.block_records.iter().any(|block| {
                        block.handle == handle
                            && crate::selection::block_definition::is_local(writer.document, block)
                    });
                    let id = writer
                        .block_ids
                        .get(&handle)
                        .filter(|_| valid)
                        .ok_or_else(|| error("retained MLINE block is unavailable for export"))?;
                    &mut writer
                        .drawing
                        .blocks
                        .iter_mut()
                        .find(|block| block.id == *id)
                        .ok_or_else(|| error("retained MLINE block is unavailable for export"))?
                        .entities
                }
            };
            let position = item
                .placement
                .following
                .iter()
                .find_map(|handle| {
                    let id = writer.entity_ids.get(handle)?;
                    entities.iter().position(|entity| entity.id() == *id)
                })
                .unwrap_or(entities.len());
            entities.insert(position, IfccadEntity::Opaque(opaque));
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
    external_source_history: HashMap<(Handle, String), ifccad::IfccadExternalSource>,
    external_sources: HashMap<Handle, ifccad::IfccadExternalSource>,
    counters: IfccadIdCounters,
    entities: HashMap<Handle, u64>,
    layers: HashMap<Handle, u64>,
    layouts: HashMap<Handle, u64>,
    blocks: HashMap<Handle, u64>,
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
            external_source_history: writer
                .external_sources
                .iter()
                .filter_map(|(&h, id)| {
                    let source = writer
                        .drawing
                        .external_sources
                        .iter()
                        .find(|s| s.id == *id)?;
                    let record = writer
                        .document
                        .block_records
                        .iter()
                        .find(|b| b.handle == h)?;
                    Some(((h, record.xref_path.clone()), source.clone()))
                })
                .collect(),
            external_sources: writer
                .external_sources
                .iter()
                .filter_map(|(&h, id)| {
                    writer
                        .drawing
                        .external_sources
                        .iter()
                        .find(|s| s.id == *id)
                        .map(|s| (h, s.clone()))
                })
                .collect(),
            counters: writer.drawing.id_counters,
            entities,
            layers,
            layouts: writer.layout_ids.clone(),
            blocks: writer.block_ids.clone(),
            patterns,
            roots,
        }
    }
}
