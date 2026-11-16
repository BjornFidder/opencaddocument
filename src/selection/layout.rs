use crate::{objects::ObjectType, tables::BlockRecord, Document, Handle};

use super::references::SourceIndex;

pub(super) fn paper_block_name(name: &str) -> bool {
    const PREFIX: &str = "*Paper_Space";
    name.get(..PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(PREFIX))
        && name[PREFIX.len()..].bytes().all(|c| c.is_ascii_digit())
}

pub(super) fn paper_block<'a>(
    doc: &'a Document,
    index: &SourceIndex,
    layout_handle: Handle,
) -> Option<&'a BlockRecord> {
    let ObjectType::Layout(layout) = doc.objects.get(&layout_handle)? else {
        return None;
    };
    let owner = layout.block_record;
    if owner == doc.header.model_space_block_handle
        || [owner, layout_handle].into_iter().any(|h| {
            h.is_null() || index.counts.get(&h) != Some(&1) || index.inconsistent.contains(&h)
        })
        || doc.objects.values().any(|object| {
            matches!(object, ObjectType::Layout(other)
                if other.handle != layout_handle && other.block_record == owner)
        })
    {
        return None;
    }
    doc.block_records.iter().find(|block| {
        block.handle == owner
            && block.layout == layout_handle
            && paper_block_name(&block.name)
            && !block.flags.is_xref
            && !block.flags.is_xref_overlay
            && block.xref_path.is_empty()
    })
}
