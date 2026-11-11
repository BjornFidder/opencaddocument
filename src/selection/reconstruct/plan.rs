use super::*;
use resources::{Binding, Catalog, Resource};

pub(super) struct Root {
    pub entity: EntityType,
    pub layer: Binding,
    pub linetype: Binding,
    pub style: Option<Binding>,
    pub resources: BTreeSet<SelectionRecordKey>,
}

pub(super) fn validate_request(
    selection: &DocumentSelection,
    request: &ReconstructionRequest,
    records: &BTreeMap<SelectionRecordKey, &SelectionPayload>,
) -> Result<(), ReconstructionError> {
    let error = |s: &str| ReconstructionError::InvalidRequest(s.into());
    let roots: BTreeSet<_> = selection.roots().iter().copied().collect();
    let mut seen = BTreeSet::new();
    for r in &request.roots {
        if !roots.contains(&r.root) || !seen.insert(r.root) {
            return Err(error("unknown, non-root or duplicate root key"));
        }
    }
    for key in request.resources.keys() {
        if key.kind == SelectionRecordKind::Entity || !records.contains_key(key) {
            return Err(error("resource action must identify an included resource"));
        }
    }
    for (i, b) in request.external.iter().enumerate() {
        if request.external[..i]
            .iter()
            .any(|prev| prev.source == b.source && prev.slot == b.slot)
            || !selection.requirements().iter().any(|r| {
                r.source == b.source
                    && r.slot == b.slot
                    && matches!(r.disposition, RequirementDisposition::External(_))
            })
        {
            return Err(error("invalid or duplicate external binding slot"));
        }
    }
    Ok(())
}

fn finite(v: Vector3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

fn position(p: &mut Vector3, transform: SelectionPlacement) -> bool {
    if let SelectionPlacement::Translate(delta) = transform {
        *p = *p + delta;
    }
    finite(*p)
}

pub(super) fn prepare_root(
    selection: &DocumentSelection,
    catalog: &mut Catalog<'_>,
    order: &order::Order,
    root: &RootReconstruction,
) -> Result<Root, ReconstructionBlocker> {
    use ReconstructionBlockerKind as K;
    let fail = |kind, slot, handle, msg| blocked(kind, root.root, slot, handle, msg);
    if let SelectionDrawOrder::Before(h) = root.order {
        if !order.has_anchor(h) {
            return Err(fail(
                K::InvalidOrderAnchor,
                None,
                Some(h),
                "anchor is not a current ModelSpace entity",
            ));
        }
    }
    let context = selection.context();
    if catalog.request.coordinates == CoordinatePolicy::MatchingUnits
        && (!(1..=24).contains(&context.insertion_units)
            || context.insertion_units != catalog.doc.header.insertion_units)
    {
        return Err(fail(
            K::IncompatibleUnits,
            None,
            None,
            "source and target need matching known units",
        ));
    }
    if !(context.linetype_scale.is_finite() && context.linetype_scale > 0.0)
        || context.linetype_scale != catalog.doc.header.linetype_scale
    {
        return Err(fail(
            K::IncompatibleUnits,
            Some(SelectionSlot::LineType),
            None,
            "global linetype scales differ or are invalid",
        ));
    }
    if matches!(root.transform, SelectionPlacement::Translate(delta) if !finite(delta)) {
        return Err(fail(
            K::InvalidPlacement,
            None,
            None,
            "translation must be finite",
        ));
    }
    let mut resources = BTreeSet::new();
    let layer = match root.layer {
        Some(h) => catalog
            .direct(SelectionRecordKind::Layer, h)
            .map(Binding::Existing)
            .ok_or_else(|| {
                fail(
                    K::InvalidBinding,
                    Some(SelectionSlot::Layer),
                    Some(h),
                    "target layer is stale or incompatible",
                )
            })?,
        None => catalog.binding(
            root.root,
            &SelectionSlot::Layer,
            SelectionRecordKind::Layer,
            &mut resources,
        )?,
    };
    let linetype = match root.linetype {
        Some(choice) => {
            let resolved = match choice {
                RootLineType::ByLayer => catalog.builtin(ContextRole::ByLayer),
                RootLineType::ByBlock => catalog.builtin(ContextRole::ByBlock),
                RootLineType::Explicit(h) => catalog.direct(SelectionRecordKind::LineType, h),
            };
            resolved.map(Binding::Existing).ok_or_else(|| {
                fail(
                    K::InvalidBinding,
                    Some(SelectionSlot::LineType),
                    None,
                    "target linetype is stale or incompatible",
                )
            })?
        }
        None => catalog.binding(
            root.root,
            &SelectionSlot::LineType,
            SelectionRecordKind::LineType,
            &mut resources,
        )?,
    };
    let geometry = || {
        fail(
            K::InvalidGeometry,
            None,
            None,
            "invalid geometry, nonfinite coordinate or incompatible segment count",
        )
    };
    let mut style = None;
    let mut entity = match catalog.records[&root.root] {
        SelectionPayload::Line(v) => {
            let mut v = (**v).clone();
            if !finite(v.normal)
                || v.normal == Vector3::ZERO
                || !v.thickness.is_finite()
                || !position(&mut v.start, root.transform)
                || !position(&mut v.end, root.transform)
            {
                return Err(geometry());
            }
            EntityType::Line(v)
        }
        SelectionPayload::MLine(v) => {
            let mut v = (**v).clone();
            let key = match catalog
                .slot(root.root, &SelectionSlot::MLineStyle)
                .map(|r| &r.disposition)
            {
                Some(RequirementDisposition::Included(key)) => *key,
                _ => {
                    return Err(fail(
                        K::MissingDependency,
                        Some(SelectionSlot::MLineStyle),
                        None,
                        "MLINE needs its included style definition",
                    ))
                }
            };
            let SelectionPayload::MLineStyle(source_style) = catalog.records[&key] else {
                return Err(geometry());
            };
            let max = i16::MAX as usize;
            if !finite(v.normal)
                || v.normal == Vector3::ZERO
                || !v.scale_factor.is_finite()
                || v.scale_factor == 0.0
                || !position(&mut v.start_point, root.transform)
                || !(2..=max).contains(&v.vertices.len())
                || !(1..=u8::MAX as usize).contains(&v.style_element_count)
                || v.style_element_count != source_style.elements.len()
            {
                return Err(geometry());
            }
            for vertex in &mut v.vertices {
                if !position(&mut vertex.position, root.transform)
                    || !finite(vertex.direction)
                    || !finite(vertex.miter)
                    || vertex.segments.len() != v.style_element_count
                    || vertex.segments.iter().any(|s| {
                        s.parameters.len() > max
                            || s.area_fill_parameters.len() > max
                            || s.parameters
                                .iter()
                                .chain(&s.area_fill_parameters)
                                .any(|v| !v.is_finite())
                    })
                {
                    return Err(geometry());
                }
            }
            style = Some(catalog.binding(
                root.root,
                &SelectionSlot::MLineStyle,
                SelectionRecordKind::Object,
                &mut resources,
            )?);
            EntityType::MLine(v)
        }
        _ => return Err(geometry()),
    };
    let common = entity.common_mut();
    if !common.linetype_scale.is_finite() || common.linetype_scale <= 0.0 {
        return Err(geometry());
    }
    if let Some(color) = root.appearance.color {
        common.color = color;
        common.color_name = None;
    }
    if let Some(v) = root.appearance.line_weight {
        common.line_weight = v;
    }
    if let Some(v) = root.appearance.transparency {
        common.transparency = v;
    }
    if let Some(v) = root.appearance.invisible {
        common.invisible = v;
    }
    Ok(Root {
        entity,
        layer,
        linetype,
        style,
        resources,
    })
}

pub(super) fn used_resources(
    roots: &[Result<Root, ReconstructionBlocker>],
) -> BTreeSet<SelectionRecordKey> {
    roots
        .iter()
        .filter_map(|root| root.as_ref().ok())
        .flat_map(|root| root.resources.iter().copied())
        .collect()
}

pub(super) fn block_conflicts(
    roots: &mut [Result<Root, ReconstructionBlocker>],
    planned: &BTreeMap<SelectionRecordKey, Resource>,
) {
    let mut names = BTreeMap::<_, Vec<_>>::new();
    for key in used_resources(roots) {
        let resource = &planned[&key];
        if resource.payload.is_some() {
            names
                .entry(resources::name_key(key, &resource.name))
                .or_default()
                .push(key);
        }
    }
    let conflicts: BTreeSet<_> = names
        .into_values()
        .filter(|keys| keys.len() > 1)
        .flatten()
        .collect();
    for root in roots {
        if let Ok(prepared) = root {
            if let Some(&key) = prepared.resources.iter().find(|k| conflicts.contains(k)) {
                *root = Err(blocked(
                    ReconstructionBlockerKind::NameConflict,
                    key,
                    None,
                    None,
                    "different source resources request the same target name",
                ));
            }
        }
    }
}
