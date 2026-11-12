use super::*;
use crate::tables::TableEntry;
use crate::{
    SemanticEntityV1, SemanticNodeV1, SemanticPartV1, SemanticReferenceV1,
    SemanticRelationshipKindV1, SemanticTableRecordV1,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn table_handle(record: SemanticTableRecordV1<'_>) -> Handle {
    match record {
        SemanticTableRecordV1::Layer(v) => v.handle(),
        SemanticTableRecordV1::LineType(v) => v.handle(),
        SemanticTableRecordV1::TextStyle(v) => v.handle(),
        SemanticTableRecordV1::BlockRecord(v) => v.handle(),
        SemanticTableRecordV1::DimStyle(v) => v.handle(),
        SemanticTableRecordV1::AppId(v) => v.handle(),
        SemanticTableRecordV1::View(v) => v.handle(),
        SemanticTableRecordV1::VPort(v) => v.handle(),
        SemanticTableRecordV1::Ucs(v) => v.handle(),
        SemanticTableRecordV1::Vx(v) => v.handle(),
    }
}
fn reference_handle(reference: SemanticReferenceV1<'_>) -> Option<Handle> {
    match reference {
        SemanticReferenceV1::Resolved(SemanticNodeV1::Entity(e)) => Some(e.common().handle),
        SemanticReferenceV1::Resolved(SemanticNodeV1::Object(o)) => Some(o.intrinsic_handle()),
        SemanticReferenceV1::Resolved(SemanticNodeV1::TableRecord(t)) => Some(table_handle(t)),
        _ => None,
    }
}

/// Scan once, but reject only identities/attachments actually used by capture.
pub(super) struct SourceIndex {
    pub counts: BTreeMap<Handle, usize>,
    pub attached: BTreeSet<Handle>,
    pub inconsistent: BTreeSet<Handle>,
    pub memberships: BTreeMap<Handle, Vec<Handle>>,
}
impl SourceIndex {
    pub fn new(doc: &Document) -> Self {
        let mut index = Self {
            counts: BTreeMap::new(),
            attached: BTreeSet::new(),
            inconsistent: BTreeSet::new(),
            memberships: BTreeMap::new(),
        };
        doc.semantic_inventory_v1().visit(|part| match part {
            SemanticPartV1::Entity(entity) => {
                let h = match entity {
                    SemanticEntityV1::Typed(e) => e.common().handle,
                    SemanticEntityV1::Unsupported { common, .. } => common.handle,
                };
                *index.counts.entry(h).or_default() += 1;
            }
            SemanticPartV1::TableRecord(t) => {
                *index.counts.entry(table_handle(t)).or_default() += 1;
            }
            SemanticPartV1::Relationship {
                kind:
                    SemanticRelationshipKindV1::ExtensionDictionary
                    | SemanticRelationshipKindV1::Reactor,
                source,
                ..
            }
            | SemanticPartV1::NonEntityExtendedData { owner: source, .. } => {
                if let Some(h) = reference_handle(source) {
                    index.attached.insert(h);
                }
            }
            _ => {}
        });
        for (h, object) in &doc.objects {
            *index.counts.entry(*h).or_default() += 1;
            if object.intrinsic_handle() != *h {
                index.inconsistent.insert(*h);
                index.inconsistent.insert(object.intrinsic_handle());
            }
        }
        for block in doc.block_records.iter() {
            for h in &block.entity_handles {
                index.memberships.entry(*h).or_default().push(block.handle);
            }
        }
        // Table-control objects also occupy the global CAD handle namespace.
        for h in [
            doc.layers.handle(),
            doc.line_types.handle(),
            doc.text_styles.handle(),
            doc.block_records.handle(),
            doc.dim_styles.handle(),
            doc.app_ids.handle(),
            doc.views.handle(),
            doc.vports.handle(),
            doc.ucss.handle(),
            doc.vx_table.handle(),
        ] {
            if !h.is_null() {
                *index.counts.entry(h).or_default() += 1;
            }
        }
        index
    }
    pub fn unique(
        &self,
        key: SelectionRecordKey,
        slot: Option<SelectionSlot>,
    ) -> Result<(), SelectionError> {
        if key.handle.is_null()
            || self.counts.get(&key.handle) != Some(&1)
            || self.inconsistent.contains(&key.handle)
        {
            return Err(SelectionError {
                kind: SelectionErrorKind::AmbiguousIdentity,
                source: Some(key),
                slot,
                message: "source handle is null, duplicated, or disagrees with its storage key"
                    .into(),
            });
        }
        Ok(())
    }
    pub fn audit(&self, doc: &Document, key: SelectionRecordKey) -> Result<(), SelectionError> {
        self.unique(key, None)?;
        if self.attached.contains(&key.handle)
            || doc
                .object_xdata
                .get(&key.handle)
                .is_some_and(|x| !x.records().is_empty() || !x.raw_dwg_eed.is_empty())
            || !doc.object_reactors(key.handle).is_empty()
            || doc.extension_dictionary_handle(key.handle).is_some()
        {
            return Err(SelectionError {
                kind: SelectionErrorKind::UnsupportedContent,
                source: Some(key),
                slot: None,
                message:
                    "attached XDATA, reactors or extension dictionary is outside capture coverage"
                        .into(),
            });
        }
        Ok(())
    }
}

/// One descriptor supplies both requirement evidence and closure traversal.
pub(super) struct Reference {
    pub source: SelectionRecordKey,
    pub slot: SelectionSlot,
    pub handle: Option<Handle>,
    pub name: Option<String>,
    pub kind: SelectionRecordKind,
}
