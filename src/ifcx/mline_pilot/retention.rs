use super::{empty_preservation, error, Handle, IfccadDocument, IfccadEntity, IfccadLinePatternId};
use crate::ifcx::{Names, Writer};
use crate::{Document, Error};
use ifccad::{
    IfccadIdCounters, IfccadMode, IfccadOpaqueEntity, IfccadPreservationRecord,
    IfccadPreservationSource, IfccadPreservationTarget,
};

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

    pub(in crate::ifcx) fn reserve_ids(&self, drawing: &mut IfccadDocument) -> Result<(), Error> {
        if drawing.drawing_id != self.drawing_id || drawing.header.id != self.header_id {
            return Err(error(
                "roundtrip context belongs to a different IFCCAD drawing",
            ));
        }
        drawing.id_counters = self.counters;
        for item in &self.blocked {
            let Some((id, _)) = item.resources.root_pattern else {
                continue;
            };
            let next =
                id.0.checked_add(1)
                    .ok_or_else(|| error("cannot reserve retained line pattern ID"))?;
            drawing.id_counters.next_line_pattern_id =
                drawing.id_counters.next_line_pattern_id.max(next);
        }
        Ok(())
    }

    pub(in crate::ifcx) fn advance_ids(&mut self, counters: IfccadIdCounters) {
        self.counters = counters;
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
            envelope.records.push(record);
        }
        Ok(())
    }
}
