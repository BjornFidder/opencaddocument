use std::collections::BTreeSet;

use crate::objects::{Dictionary, ObjectType, SortEntitiesTable};

use super::super::references::SourceIndex;
use super::{construction, Document, Handle, ReconstructionError, SelectionDrawOrder};

pub(super) struct Order {
    owner: Handle,
    members: Vec<Handle>,
    live: BTreeSet<Handle>,
    dictionary: Option<Handle>,
    table: Option<Handle>,
    natural: bool,
}

fn unsafe_target(message: &str) -> ReconstructionError {
    ReconstructionError::UnsafeTarget(message.into())
}

impl Order {
    pub(super) fn read(
        doc: &Document,
        owner: Handle,
        index: &SourceIndex,
    ) -> Result<Self, ReconstructionError> {
        let block = doc
            .block_records
            .get("*Model_Space")
            .filter(|b| {
                !owner.is_null()
                    && b.handle == owner
                    && doc.header.model_space_block_handle == owner
            })
            .ok_or_else(|| unsafe_target("inconsistent ModelSpace identity"))?;
        if index.counts.get(&owner) != Some(&1) || index.inconsistent.contains(&owner) {
            return Err(unsafe_target("ambiguous ModelSpace identity"));
        }
        let mut members = block.entity_handles.clone();
        let member_keys: BTreeSet<_> = members.iter().copied().collect();
        if member_keys.len() != members.len() || member_keys.contains(&Handle::NULL) {
            return Err(unsafe_target("duplicate or null ModelSpace member"));
        }
        let flat: Vec<_> = doc
            .entities()
            .filter(|e| {
                e.common().owner_handle == owner
                    && !matches!(
                        e,
                        crate::EntityType::Block(_)
                            | crate::EntityType::BlockEnd(_)
                            | crate::EntityType::AttributeEntity(_)
                    )
            })
            .map(|e| e.common().handle)
            .collect();
        let live: BTreeSet<_> = flat.iter().copied().collect();
        if live.len() != flat.len()
            || !live.is_subset(&member_keys)
            || members
                .iter()
                .any(|&h| doc.get_entity(h).is_some() != live.contains(&h))
            || live.iter().any(|h| {
                index
                    .memberships
                    .get(h)
                    .is_none_or(|owners| owners.as_slice() != [owner])
            })
        {
            return Err(unsafe_target("inconsistent live ModelSpace membership"));
        }
        let dictionaries: Vec<_> = doc
            .objects
            .iter()
            .filter_map(|(key, object)| match object {
                ObjectType::Dictionary(d) if d.owner == owner => Some((*key, d)),
                _ => None,
            })
            .collect();
        let dictionary = doc.extension_dictionary_handle(owner);
        let mut table = None;
        match (dictionary, dictionaries.as_slice()) {
            (None, []) => {}
            (Some(h), [(key, d)]) if h == *key && d.handle == h => {
                let mut entries = d.entries.iter().filter(|(name, _)| {
                    name.eq_ignore_ascii_case(SortEntitiesTable::DICTIONARY_KEY)
                });
                table = entries.next().map(|(_, h)| *h);
                if entries.next().is_some() {
                    return Err(unsafe_target("duplicate ACAD_SORTENTS registration"));
                }
            }
            _ => return Err(unsafe_target("ambiguous ModelSpace extension dictionary")),
        }
        for (key, object) in &doc.objects {
            if let ObjectType::SortEntitiesTable(v) = object {
                if v.block_owner_handle == owner && Some(*key) != table {
                    return Err(unsafe_target("unregistered ModelSpace SortEnts table"));
                }
            }
        }
        let natural = flat
            .iter()
            .copied()
            .eq(members.iter().copied().filter(|h| live.contains(h)))
            && members.windows(2).all(|pair| pair[0] < pair[1]);
        if let Some(h) = table {
            let Some(ObjectType::SortEntitiesTable(v)) = doc.objects.get(&h) else {
                return Err(unsafe_target("invalid SortEnts dictionary entry"));
            };
            if v.handle != h || Some(v.owner_handle) != dictionary || v.block_owner_handle != owner
            {
                return Err(unsafe_target("inconsistent SortEnts registration"));
            }
            if v.entries().any(|e| !member_keys.contains(&e.entity_handle)) {
                return Err(unsafe_target(
                    "SortEnts entry outside current ModelSpace membership",
                ));
            }
            members.sort_by_key(|&h| v.effective_sort_handle(h).value());
            if members
                .windows(2)
                .any(|p| v.effective_sort_handle(p[0]) == v.effective_sort_handle(p[1]))
            {
                return Err(unsafe_target("ambiguous current SortEnts rank"));
            }
        } else if !natural {
            // Without an explicit table the two stores must agree on their live order.
            if !flat
                .iter()
                .copied()
                .eq(members.iter().copied().filter(|h| live.contains(h)))
            {
                return Err(unsafe_target("flat and block ModelSpace order disagree"));
            }
        }
        Ok(Self {
            owner,
            members,
            live,
            dictionary,
            table,
            natural,
        })
    }

    pub(super) fn has_anchor(&self, h: Handle) -> bool {
        self.live.contains(&h)
    }

    pub(super) fn scaffold(
        &self,
        orders: impl Iterator<Item = SelectionDrawOrder>,
    ) -> (bool, usize) {
        let needed = self.table.is_some()
            || !self.natural
            || orders
                .into_iter()
                .any(|o| matches!(o, SelectionDrawOrder::Before(_)));
        let count = if needed {
            usize::from(self.dictionary.is_none()) + usize::from(self.table.is_none())
        } else {
            0
        };
        (needed, count)
    }

    pub(super) fn apply(
        self,
        doc: &mut Document,
        mut handles: impl Iterator<Item = Handle>,
        added: &[(SelectionDrawOrder, Handle)],
    ) -> Result<(), ReconstructionError> {
        let dictionary = match self.dictionary {
            Some(h) => h,
            None => {
                let h = handles
                    .next()
                    .ok_or_else(|| construction("missing extension dictionary handle"))?;
                let mut d = Dictionary::new();
                d.handle = h;
                d.owner = self.owner;
                d.hard_owner = true;
                doc.objects.insert(h, ObjectType::Dictionary(d));
                if doc.extension_dictionary_handle(self.owner) != Some(h) {
                    return Err(construction(
                        "extension dictionary is not registered for ModelSpace",
                    ));
                }
                h
            }
        };
        let h = match self.table {
            Some(h) => h,
            None => {
                let h = handles
                    .next()
                    .ok_or_else(|| construction("missing SortEnts handle"))?;
                let Some(ObjectType::Dictionary(dict)) = doc.objects.get_mut(&dictionary) else {
                    return Err(construction("missing ModelSpace extension dictionary"));
                };
                dict.add_entry(SortEntitiesTable::DICTIONARY_KEY, h);
                h
            }
        };
        let mut before = std::collections::BTreeMap::<Handle, Vec<_>>::new();
        let mut append = Vec::new();
        for &(order, entity) in added {
            match order {
                SelectionDrawOrder::Before(anchor) => {
                    before.entry(anchor).or_default().push(entity)
                }
                SelectionDrawOrder::Append => append.push(entity),
            }
        }
        let mut sort = SortEntitiesTable::for_block(self.owner);
        sort.handle = h;
        sort.owner_handle = dictionary;
        let mut rank = 1u64;
        for entity in self
            .members
            .into_iter()
            .flat_map(|member| {
                before
                    .remove(&member)
                    .into_iter()
                    .flatten()
                    .chain(std::iter::once(member))
            })
            .chain(append)
        {
            sort.add_entry(entity, Handle::new(rank));
            rank = rank
                .checked_add(1)
                .ok_or_else(|| construction("SortEnts rank overflow"))?;
        }
        if !before.is_empty() || handles.next().is_some() {
            return Err(construction("inconsistent SortEnts plan"));
        }
        doc.objects.insert(h, ObjectType::SortEntitiesTable(sort));
        Ok(())
    }
}
