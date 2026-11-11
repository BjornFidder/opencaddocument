use super::{error, frame, Result, SelectionStorageError, SelectionStorageErrorKind as Kind};
use crate::selection::*;
use std::collections::{BTreeMap, BTreeSet};

type LocatedSlot = (SelectionRecordKey, (u8, usize));

fn slot_key(slot: &SelectionSlot) -> (u8, usize) {
    match slot {
        SelectionSlot::Owner => (0, 0),
        SelectionSlot::Layer => (1, 0),
        SelectionSlot::LineType => (2, 0),
        SelectionSlot::MLineStyle => (3, 0),
        SelectionSlot::StyleElementLineType(i) => (4, *i),
        SelectionSlot::StyleRegistration => (5, 0),
    }
}

fn at(path: String, message: impl Into<String>) -> SelectionStorageError {
    let mut err = error(Kind::InvalidSelection, message);
    err.path = Some(path);
    err
}

fn record_identity(payload: &SelectionPayload) -> SelectionRecordKey {
    use SelectionRecordKind::*;
    let (kind, handle) = match payload {
        SelectionPayload::Line(v) => (Entity, v.common.handle),
        SelectionPayload::MLine(v) => (Entity, v.common.handle),
        SelectionPayload::MLineStyle(v) => (Object, v.handle),
        SelectionPayload::Layer(v) => (Layer, v.handle),
        SelectionPayload::LineType(v) => (LineType, v.handle),
    };
    SelectionRecordKey { kind, handle }
}

fn record_name(payload: &SelectionPayload) -> Option<&str> {
    match payload {
        SelectionPayload::Layer(v) => Some(&v.name),
        SelectionPayload::LineType(v) => Some(&v.name),
        SelectionPayload::MLineStyle(v) => Some(&v.name),
        _ => None,
    }
}

fn slot_count(payload: &SelectionPayload) -> usize {
    match payload {
        SelectionPayload::Line(_) => 3,
        SelectionPayload::MLine(_) => 4,
        SelectionPayload::Layer(_) => 1,
        SelectionPayload::LineType(_) => 0,
        SelectionPayload::MLineStyle(v) => 1 + v.elements.len(),
    }
}

fn source_default(payload: &SelectionPayload) -> bool {
    matches!(payload, SelectionPayload::MLine(v) if v.style_handle.is_none_or(|h| h.is_null()))
}

fn linetype_role(name: &str) -> Option<ContextRole> {
    if name.is_empty() || name.eq_ignore_ascii_case("BYLAYER") {
        Some(ContextRole::ByLayer)
    } else if name.eq_ignore_ascii_case("BYBLOCK") {
        Some(ContextRole::ByBlock)
    } else {
        None
    }
}

fn matches_source(req: &SelectionRequirement, handle: Option<Handle>, name: &str) -> bool {
    req.source_handle == handle && req.source_name.as_deref() == Some(name)
}

fn resource_kind(
    payload: &SelectionPayload,
    req: &SelectionRequirement,
) -> std::result::Result<Option<SelectionRecordKind>, &'static str> {
    use RequirementDisposition::*;
    use SelectionRecordKind as RecordKind;
    use SelectionSlot::*;
    let common = match payload {
        SelectionPayload::Line(v) => Some(&v.common),
        SelectionPayload::MLine(v) => Some(&v.common),
        _ => None,
    };
    let (kind, name, handle) = match (&req.slot, payload, common) {
        (Owner, _, Some(c)) => {
            return if !c.owner_handle.is_null()
                && req.source_handle == Some(c.owner_handle)
                && req
                    .source_name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case("*Model_Space"))
                && req.disposition == Context(ContextRole::ModelSpace)
            {
                Ok(None)
            } else {
                Err("inconsistent ModelSpace owner requirement")
            };
        }
        (StyleRegistration, SelectionPayload::MLineStyle(v), _) => {
            return if matches_source(req, Some(v.owner), &v.name)
                && (req.disposition == Missing
                    || (!v.owner.is_null()
                        && req.disposition == Context(ContextRole::MLineStyleDictionary)))
            {
                Ok(None)
            } else {
                Err("inconsistent MLINESTYLE registration requirement")
            };
        }
        (Layer, _, Some(c)) => (RecordKind::Layer, c.layer.as_str(), c.layer_handle),
        (LineType, _, Some(c)) => (RecordKind::LineType, c.linetype.as_str(), c.linetype_handle),
        (LineType, SelectionPayload::Layer(v), _) => {
            (RecordKind::LineType, v.line_type.as_str(), None)
        }
        (StyleElementLineType(i), SelectionPayload::MLineStyle(v), _) => {
            let el = v
                .elements
                .get(*i)
                .ok_or("style element index is out of range")?;
            (RecordKind::LineType, el.linetype.as_str(), None)
        }
        (MLineStyle, SelectionPayload::MLine(v), _) => {
            let handle = v
                .style_handle
                .filter(|h| !h.is_null())
                .or(req.source_handle)
                .ok_or("effective MLINESTYLE handle is missing from requirement")?;
            if handle.is_null() && req.disposition != Missing {
                return Err("a null effective style handle requires a missing disposition");
            }
            (RecordKind::Object, v.style_name.as_str(), Some(handle))
        }
        _ => return Err("slot is incompatible with source payload"),
    };
    if !matches_source(req, handle, name) {
        return Err("requirement disagrees with the stored source handle or name");
    }
    if let Some(role) = (kind == RecordKind::LineType)
        .then(|| linetype_role(name))
        .flatten()
    {
        return if req.disposition == Context(role)
            || (req.disposition == Missing && handle.is_some_and(|h| !h.is_null()))
        {
            Ok(None)
        } else {
            Err("inconsistent inherited linetype requirement")
        };
    }
    if matches!(req.disposition, Context(_)) {
        return Err("ordinary resource slot cannot be a contextual role");
    }
    Ok(Some(kind))
}

fn names_match(a: &str, b: &str) -> bool {
    a.chars()
        .flat_map(char::to_uppercase)
        .eq(b.chars().flat_map(char::to_uppercase))
}

pub(super) fn selection(s: &DocumentSelection) -> Result<()> {
    if [
        s.roots.len(),
        s.records.len(),
        s.requirements.len(),
        s.report.len(),
    ]
    .into_iter()
    .any(|n| n > frame::COUNT_LIMIT)
        || s.namespace.len() > frame::STRING_LIMIT
    {
        return Err(error(
            Kind::LimitExceeded,
            "selection exceeds top-level storage limits",
        ));
    }
    if s.namespace.trim().is_empty() {
        return Err(at("/namespace".into(), "selection namespace is empty"));
    }
    let mut records = BTreeMap::new();
    let mut identities = BTreeSet::new();
    let mut model_space = None;
    let mut style_dictionary = None;
    for (i, r) in s.records.iter().enumerate() {
        if r.key.handle.is_null()
            || r.key != record_identity(&r.payload)
            || !identities.insert(r.key.handle)
        {
            return Err(at(
                format!("/records/{i}/key"),
                "invalid or duplicate record identity",
            ));
        }
        records.insert(r.key, &r.payload);
        let owner = match &r.payload {
            SelectionPayload::Line(v) => Some((&mut model_space, v.common.owner_handle)),
            SelectionPayload::MLine(v) => Some((&mut model_space, v.common.owner_handle)),
            SelectionPayload::MLineStyle(v) if !v.owner.is_null() => {
                Some((&mut style_dictionary, v.owner))
            }
            _ => None,
        };
        if let Some((shared, handle)) = owner {
            if *shared.get_or_insert(handle) != handle {
                return Err(at(
                    format!("/records/{i}/payload"),
                    "conflicting contextual owner",
                ));
            }
        }
    }
    let owners = [model_space, style_dictionary];
    if (model_space.is_some() && model_space == style_dictionary)
        || owners.iter().flatten().any(|h| identities.contains(h))
    {
        return Err(at(
            "/records".into(),
            "contextual owner collides with a record identity",
        ));
    }
    let mut roots = BTreeSet::new();
    for (i, &key) in s.roots.iter().enumerate() {
        if key.kind != SelectionRecordKind::Entity
            || !records.contains_key(&key)
            || !roots.insert(key)
        {
            return Err(at(
                format!("/roots/{i}"),
                "root must identify one unique entity record",
            ));
        }
    }
    if records
        .keys()
        .any(|k| k.kind == SelectionRecordKind::Entity && !roots.contains(k))
    {
        return Err(at("/records".into(), "entity record is absent from roots"));
    }

    let mut slots = BTreeMap::new();
    let mut slot_counts = BTreeMap::new();
    let mut outgoing = BTreeMap::<_, Vec<_>>::new();
    let mut external = BTreeMap::new();
    let mut missing = Vec::new();
    for (i, req) in s.requirements.iter().enumerate() {
        let fail = |message: &str| {
            at(
                format!("/requirements/{i}"),
                format!("{:?}/{:?}: {message}", req.source, req.slot),
            )
        };
        let payload = records
            .get(&req.source)
            .ok_or_else(|| fail("missing source record"))?;
        if slots
            .insert((req.source, slot_key(&req.slot)), &req.disposition)
            .is_some()
        {
            return Err(fail("duplicate source slot"));
        }
        *slot_counts.entry(req.source).or_insert(0) += 1;
        let kind = resource_kind(payload, req).map_err(fail)?;
        if let RequirementDisposition::Included(k) | RequirementDisposition::External(k) =
            req.disposition
        {
            if Some(k.kind) != kind
                || k.handle.is_null()
                || owners.contains(&Some(k.handle))
                || req
                    .source_handle
                    .is_some_and(|h| !h.is_null() && h != k.handle)
            {
                return Err(fail("invalid resource endpoint identity"));
            }
            if matches!(req.disposition, RequirementDisposition::Included(_)) {
                let target = records
                    .get(&k)
                    .ok_or_else(|| fail("included endpoint has no record"))?;
                if let Some(name) = req.source_name.as_deref().filter(|name| !name.is_empty()) {
                    if !record_name(target).is_some_and(|actual| names_match(name, actual)) {
                        return Err(fail("included endpoint name disagrees with source name"));
                    }
                }
                outgoing.entry(req.source).or_default().push(k);
            } else if identities.contains(&k.handle)
                || external
                    .insert(k.handle, k.kind)
                    .is_some_and(|kind| kind != k.kind)
            {
                return Err(fail("external endpoint conflicts with a known identity"));
            }
        }
        if req.disposition == RequirementDisposition::Missing {
            if let Some(h) = req.source_handle.filter(|h| !h.is_null()) {
                missing.push((i, h));
            }
        }
    }
    for (i, handle) in missing {
        if identities.contains(&handle) || external.contains_key(&handle) {
            return Err(at(
                format!("/requirements/{i}"),
                "missing endpoint is a known record",
            ));
        }
    }
    for (i, r) in s.records.iter().enumerate() {
        if slot_counts.get(&r.key).copied().unwrap_or(0) != slot_count(&r.payload) {
            return Err(at(
                format!("/records/{i}"),
                "record does not have exactly its required slots",
            ));
        }
    }

    let mut reached = BTreeSet::new();
    let mut pending = s.roots.clone();
    while let Some(key) = pending.pop() {
        if reached.insert(key) {
            if let Some(targets) = outgoing.get(&key) {
                pending.extend_from_slice(targets);
            }
        }
    }
    if reached.len() != records.len() {
        return Err(at(
            "/records".into(),
            "record is not reachable from the roots",
        ));
    }
    evidence(s, &records, &slots)
}

fn evidence(
    s: &DocumentSelection,
    records: &BTreeMap<SelectionRecordKey, &SelectionPayload>,
    slots: &BTreeMap<LocatedSlot, &RequirementDisposition>,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    let mut missing = 0;
    let mut defaulted = 0;
    let mut unknown_units = 0;
    let needs_units_evidence =
        !s.roots.is_empty() && !(1..=24).contains(&s.context.insertion_units);
    for (i, e) in s.report.iter().enumerate() {
        use CaptureEvidenceCode::*;
        let slot = e.slot.as_ref().map(slot_key);
        let located = e.source.zip(slot);
        let payload = e.source.and_then(|k| records.get(&k).copied());
        let (tag, valid) = match e.code {
            MissingDependency => {
                missing += 1;
                (
                    0,
                    located.and_then(|k| slots.get(&k).copied())
                        == Some(&RequirementDisposition::Missing),
                )
            }
            ExcludedRawRecord | ExcludedGraphics | ExcludedDerivedData => {
                let tag = match e.code {
                    ExcludedRawRecord => 1,
                    ExcludedGraphics => 2,
                    _ => 3,
                };
                (
                    tag,
                    slot.is_none()
                        && matches!(
                            payload,
                            Some(SelectionPayload::Line(_) | SelectionPayload::MLine(_))
                        ),
                )
            }
            ResolvedSourceDefault => {
                defaulted += 1;
                (
                    4,
                    e.slot == Some(SelectionSlot::MLineStyle)
                        && payload.is_some_and(source_default),
                )
            }
            UnknownUnits => {
                unknown_units += 1;
                (
                    5,
                    e.source.is_none() && e.slot.is_none() && needs_units_evidence,
                )
            }
        };
        if !valid || !seen.insert((e.source, slot, tag)) {
            return Err(at(format!("/report/{i}"), "invalid or duplicate evidence"));
        }
    }
    if missing
        != slots
            .values()
            .filter(|d| ***d == RequirementDisposition::Missing)
            .count()
        || defaulted != records.values().filter(|p| source_default(p)).count()
        || unknown_units != usize::from(needs_units_evidence)
    {
        return Err(at("/report".into(), "required evidence is missing"));
    }
    Ok(())
}
