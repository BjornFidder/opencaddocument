use crate::{objects::ObjectType, tables::BlockRecord, Document};

pub(super) fn is_definition_name(name: &str) -> bool {
    !name.trim().is_empty()
        && !name.contains('|')
        && !name.eq_ignore_ascii_case("*Model_Space")
        && !name
            .get(.."*Paper_Space".len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("*Paper_Space"))
}

pub(crate) fn is_local(doc: &Document, block: &BlockRecord) -> bool {
    let handle = block.handle;
    if !is_definition_name(&block.name)
        || !block.layout.is_null()
        || handle == doc.header.model_space_block_handle
        || handle == doc.header.paper_space_block_handle
        || block.flags.is_xref
        || block.flags.is_xref_overlay
        || block.flags.is_external
        || block.flags.is_xref_unloaded
        || !block.xref_path.is_empty()
        || doc.objects.values().any(
            |object| matches!(object, ObjectType::Layout(layout) if layout.block_record == handle),
        )
    {
        return false;
    }
    let name = block.name.to_uppercase();
    doc.block_records
        .iter()
        .filter(|record| record.name.to_uppercase() == name)
        .count()
        == 1
}
