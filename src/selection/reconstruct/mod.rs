#![doc = include_str!("../../../docs/selection-reconstruction.md")]

use std::collections::{BTreeMap, BTreeSet};

use crate::types::{Color, LineWeight, Transparency, Vector3};
use crate::{Document, EntityType, Handle, HandleReservationError};

use super::{
    ContextRole, DocumentSelection, RequirementDisposition, SelectionPayload, SelectionRecordKey,
    SelectionRecordKind, SelectionRequirement, SelectionSlot,
};

mod order;
mod resources;

#[derive(Debug, Clone)]
pub struct ReconstructionRequest {
    pub model_space: Handle,
    pub coordinates: CoordinatePolicy,
    pub roots: Vec<RootReconstruction>,
    pub resources: BTreeMap<SelectionRecordKey, ResourceAction>,
    pub external: Vec<ExternalBinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinatePolicy {
    MatchingUnits,
    DrawingCoordinates,
}

#[derive(Debug, Clone)]
pub struct RootReconstruction {
    pub root: SelectionRecordKey,
    pub transform: SelectionPlacement,
    pub order: SelectionDrawOrder,
    pub layer: Option<Handle>,
    pub linetype: Option<RootLineType>,
    pub appearance: RootAppearanceOverrides,
}

#[derive(Debug, Clone, Copy)]
pub enum SelectionPlacement {
    Identity,
    Translate(Vector3),
}

#[derive(Debug, Clone, Copy)]
pub enum SelectionDrawOrder {
    Append,
    Before(Handle),
}

#[derive(Debug, Clone, Copy)]
pub enum RootLineType {
    ByLayer,
    ByBlock,
    Explicit(Handle),
}

#[derive(Debug, Clone, Default)]
pub struct RootAppearanceOverrides {
    pub color: Option<Color>,
    pub line_weight: Option<LineWeight>,
    pub transparency: Option<Transparency>,
    pub invisible: Option<bool>,
}

#[derive(Debug, Clone)]
pub enum ResourceAction {
    Reuse(Handle),
    Create { name: String },
}

#[derive(Debug, Clone)]
pub struct ExternalBinding {
    pub source: SelectionRecordKey,
    pub slot: SelectionSlot,
    pub target: Handle,
}

#[must_use]
#[derive(Debug)]
pub struct ReconstructionOutcome {
    pub document: Document,
    pub roots: Vec<RootReconstructionResult>,
    pub resources: BTreeMap<SelectionRecordKey, Handle>,
}

#[derive(Debug, Clone)]
pub struct RootReconstructionResult {
    pub root: SelectionRecordKey,
    pub status: RootReconstructionStatus,
}

#[derive(Debug, Clone)]
pub enum RootReconstructionStatus {
    Applied { entity: Handle, layer: Handle },
    Blocked { reasons: Vec<ReconstructionBlocker> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReconstructionBlockerKind {
    MissingBinding,
    MissingDependency,
    InvalidBinding,
    MissingResourceAction,
    NameConflict,
    UnsupportedNameCoverage,
    IncompatibleResource,
    InvalidGeometry,
    IncompatibleUnits,
    InvalidPlacement,
    InvalidOrderAnchor,
}

#[derive(Debug, Clone)]
pub struct ReconstructionBlocker {
    pub kind: ReconstructionBlockerKind,
    pub source: SelectionRecordKey,
    pub slot: Option<SelectionSlot>,
    pub target: Option<Handle>,
    pub message: String,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ReconstructionError {
    InvalidRequest(String),
    UnsafeTarget(String),
    Reservation(HandleReservationError),
    Construction(String),
}

impl std::fmt::Display for ReconstructionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRequest(s) => write!(f, "invalid reconstruction request: {s}"),
            Self::UnsafeTarget(s) => write!(f, "unsafe reconstruction target: {s}"),
            Self::Reservation(e) => write!(f, "cannot reserve reconstruction handles: {e}"),
            Self::Construction(s) => write!(f, "cannot construct reconstruction candidate: {s}"),
        }
    }
}

impl std::error::Error for ReconstructionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Reservation(e) => Some(e),
            _ => None,
        }
    }
}

fn blocked(
    kind: ReconstructionBlockerKind,
    source: SelectionRecordKey,
    slot: Option<SelectionSlot>,
    target: Option<Handle>,
    message: &str,
) -> ReconstructionBlocker {
    ReconstructionBlocker {
        kind,
        source,
        slot,
        target,
        message: message.into(),
    }
}

fn construction(error: impl std::fmt::Display) -> ReconstructionError {
    ReconstructionError::Construction(error.to_string())
}

mod plan;
use super::references::SourceIndex;

pub(super) fn run(
    selection: &DocumentSelection,
    target: &Document,
    request: &ReconstructionRequest,
) -> Result<ReconstructionOutcome, ReconstructionError> {
    run_with_order(selection, target, request, order::Order::apply)
}

fn run_with_order(
    selection: &DocumentSelection,
    target: &Document,
    request: &ReconstructionRequest,
    apply_order: impl FnOnce(
        order::Order,
        &mut Document,
        std::vec::IntoIter<Handle>,
        &[(SelectionDrawOrder, Handle)],
    ) -> Result<(), ReconstructionError>,
) -> Result<ReconstructionOutcome, ReconstructionError> {
    let records = selection
        .records()
        .iter()
        .map(|r| (r.key, &r.payload))
        .collect();
    plan::validate_request(selection, request, &records)?;
    if request.roots.is_empty() {
        return Ok(ReconstructionOutcome {
            document: target.clone(),
            roots: Vec::new(),
            resources: BTreeMap::new(),
        });
    }
    let index = SourceIndex::new(target);
    let order = order::Order::read(target, request.model_space, &index)?;
    let mut catalog = resources::Catalog::new(selection, target, &index, request, records);
    let mut roots: Vec<_> = request
        .roots
        .iter()
        .map(|root| plan::prepare_root(selection, &mut catalog, &order, root))
        .collect();
    plan::block_conflicts(&mut roots, &catalog.planned);
    if roots.iter().any(|r| {
        matches!(r, Err(b) if b.kind == ReconstructionBlockerKind::IncompatibleResource &&
        b.source.kind == SelectionRecordKind::Object &&
        matches!(request.resources.get(&b.source), Some(ResourceAction::Reuse(_))))
    }) {
        resources::style_dictionary(target)?;
    }
    let orders: Vec<_> = request
        .roots
        .iter()
        .zip(&roots)
        .filter(|(_, root)| root.is_ok())
        .map(|(r, _)| r.order)
        .collect();
    let applied = orders.len();
    let (needs_order, order_count) = order.scaffold(orders.iter().copied());
    let used_keys = plan::used_resources(&roots);
    let mut used = catalog.planned;
    used.retain(|key, _| used_keys.contains(key));
    let needs_style = used
        .iter()
        .any(|(key, r)| key.kind == SelectionRecordKind::Object && r.payload.is_some());
    let existing_style = if needs_style {
        resources::style_dictionary(target)?
    } else {
        None
    };
    let new_style = needs_style && existing_style.is_none();
    let mut candidate = target.clone();
    let count = if applied == 0 {
        0
    } else {
        applied
            .checked_add(used.values().filter(|r| r.payload.is_some()).count())
            .and_then(|n| n.checked_add(order_count + usize::from(new_style)))
            .ok_or_else(|| construction("reservation count overflow"))?
    };
    let reserved = if count == 0 {
        None
    } else {
        Some(
            candidate
                .try_reserve_handles(count)
                .map_err(ReconstructionError::Reservation)?,
        )
    };
    let mut ids = reserved.into_iter().flatten();
    let mut next = || {
        ids.next()
            .ok_or_else(|| construction("reservation range shorter than plan"))
    };
    let style = if new_style {
        Some(next()?)
    } else {
        existing_style
    };
    let mut resources = BTreeMap::new();
    let mut creations = Vec::new();
    for kind in [
        SelectionRecordKind::LineType,
        SelectionRecordKind::Layer,
        SelectionRecordKind::Object,
    ] {
        for (&key, r) in used.iter().filter(|(key, _)| key.kind == kind) {
            let h = if let Some(h) = r.handle {
                h
            } else {
                creations.push(key);
                next()?
            };
            resources.insert(key, h);
        }
    }
    let mut placements = Vec::with_capacity(applied);
    for order in orders {
        placements.push((order, next()?));
    }
    let order_handles: Vec<_> = if applied == 0 {
        Vec::new()
    } else {
        (0..order_count).map(|_| next()).collect::<Result<_, _>>()?
    };
    if ids.next().is_some() {
        return Err(construction("reservation range longer than plan"));
    }

    if new_style {
        resources::create_style_dictionary(&mut candidate, style.unwrap())?;
    }
    for key in creations {
        let resource = used
            .get_mut(&key)
            .ok_or_else(|| construction("missing planned resource"))?;
        resources::insert(&mut candidate, resource, resources[&key], style)?;
    }
    let mut handles = placements.iter().map(|(_, h)| *h);
    let mut outcomes = Vec::with_capacity(roots.len());
    for (request_root, plan) in request.roots.iter().zip(roots) {
        let status = match plan {
            Ok(plan::Root {
                mut entity,
                layer,
                linetype,
                style,
                ..
            }) => {
                let h = handles
                    .next()
                    .ok_or_else(|| construction("missing planned entity handle"))?;
                let layer_handle = layer
                    .handle(&resources)
                    .ok_or_else(|| construction("missing layer handle"))?;
                let common = entity.common_mut();
                common.handle = h;
                common.owner_handle = request.model_space;
                layer.name(&used).clone_into(&mut common.layer);
                common.layer_handle = Some(layer_handle);
                common.linetype_handle = Some(
                    linetype
                        .handle(&resources)
                        .ok_or_else(|| construction("missing linetype handle"))?,
                );
                linetype.name(&used).clone_into(&mut common.linetype);
                if let EntityType::MLine(v) = &mut entity {
                    let style = style.ok_or_else(|| construction("missing planned MLINESTYLE"))?;
                    v.style_handle = Some(
                        style
                            .handle(&resources)
                            .ok_or_else(|| construction("missing MLINESTYLE handle"))?,
                    );
                    style.name(&used).clone_into(&mut v.style_name);
                }
                let added = candidate.add_entity(entity).map_err(construction)?;
                if added != h
                    || candidate
                        .get_entity(h)
                        .is_none_or(|e| e.common().owner_handle != request.model_space)
                {
                    return Err(construction(
                        "entity did not land at its reserved identity and owner",
                    ));
                }
                RootReconstructionStatus::Applied {
                    entity: h,
                    layer: layer_handle,
                }
            }
            Err(reason) => RootReconstructionStatus::Blocked {
                reasons: vec![reason],
            },
        };
        outcomes.push(RootReconstructionResult {
            root: request_root.root,
            status,
        });
    }
    if applied != 0 {
        let block = candidate
            .block_records
            .get("*Model_Space")
            .ok_or_else(|| construction("missing ModelSpace after insertion"))?;
        if !block
            .entity_handles
            .iter()
            .rev()
            .take(applied)
            .copied()
            .eq(placements.iter().rev().map(|(_, handle)| *handle))
        {
            return Err(construction(
                "ModelSpace membership differs from planned insertions",
            ));
        }
    }
    if needs_order && applied != 0 {
        apply_order(
            order,
            &mut candidate,
            order_handles.into_iter(),
            &placements,
        )?;
    }
    Ok(ReconstructionOutcome {
        document: candidate,
        roots: outcomes,
        resources,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::ObjectType;
    use crate::selection::CaptureRequest;
    use crate::tables::Layer;
    use crate::Line;

    #[test]
    fn late_order_construction_failure_discards_candidate() {
        let line = || {
            EntityType::Line(Line::from_points(
                Vector3::ZERO,
                Vector3::new(1.0, 0.0, 0.0),
            ))
        };
        let mut source = Document::new();
        let mut layer = Layer::new("Source walls");
        layer.line_type = "ByLayer".into();
        layer.handle = source.allocate_handle();
        let layer_key = SelectionRecordKey {
            kind: SelectionRecordKind::Layer,
            handle: layer.handle,
        };
        source.layers.add(layer).unwrap();
        let mut selected = line();
        selected.common_mut().layer = "Source walls".into();
        selected.common_mut().layer_handle = Some(layer_key.handle);
        let selected = source.add_entity(selected).unwrap();
        let selection =
            DocumentSelection::capture(&source, &CaptureRequest::new("source", vec![selected]))
                .unwrap()
                .selection;
        let original_selection = selection.clone();

        let mut target = Document::new();
        let anchor = target.add_entity(line()).unwrap();
        let model_space = target.header.model_space_block_handle;
        let dictionary = target.ensure_extension_dictionary(model_space);
        let original_target = target.clone();
        let floor = target.next_handle();
        let seed = target.header.handle_seed;
        let request = ReconstructionRequest {
            model_space,
            coordinates: CoordinatePolicy::DrawingCoordinates,
            roots: vec![RootReconstruction {
                root: selection.roots()[0],
                transform: SelectionPlacement::Identity,
                order: SelectionDrawOrder::Before(anchor),
                layer: None,
                linetype: None,
                appearance: RootAppearanceOverrides::default(),
            }],
            resources: BTreeMap::from([(
                layer_key,
                ResourceAction::Create {
                    name: "Restored walls".into(),
                },
            )]),
            external: vec![],
        };

        let result = run_with_order(
            &selection,
            &target,
            &request,
            |order, candidate, handles, placements| {
                assert!(candidate.next_handle() > floor);
                assert!(candidate.header.handle_seed > seed);
                assert_eq!(placements.len(), 1);
                let restored = candidate.get_entity(placements[0].1).unwrap();
                assert_eq!(restored.common().layer, "Restored walls");
                assert!(candidate.layers.get("Restored walls").is_some());

                // Simulate a late construction invariant failure on the clone.
                assert!(matches!(
                    candidate.objects.remove(&dictionary),
                    Some(ObjectType::Dictionary(_))
                ));
                order.apply(candidate, handles, placements)
            },
        );
        assert!(matches!(
            result,
            Err(ReconstructionError::Construction(message))
                if message == "missing ModelSpace extension dictionary"
        ));
        assert_eq!(target, original_target);
        assert_eq!(target.next_handle(), floor);
        assert_eq!(target.header.handle_seed, seed);
        assert_eq!(selection, original_selection);
    }

    #[test]
    fn shared_resources_are_planned_once_and_blocked_only_resources_are_pruned() {
        use crate::tables::LineType;

        let mut source = Document::new();
        let mut pattern = LineType::new("Shared pattern");
        pattern.handle = source.allocate_handle();
        let pattern_key = SelectionRecordKey {
            kind: SelectionRecordKind::LineType,
            handle: pattern.handle,
        };
        source.line_types.add(pattern).unwrap();
        let description = "Shared metadata. ".repeat(4_096);
        let mut add_layer = |name: &str, linetype: &str| {
            let mut layer = Layer::new(name);
            layer.handle = source.allocate_handle();
            layer.description.clone_from(&description);
            layer.line_type = linetype.into();
            let handle = layer.handle;
            source.layers.add(layer).unwrap();
            SelectionRecordKey {
                kind: SelectionRecordKind::Layer,
                handle,
            }
        };
        let shared = add_layer("Shared walls", "Shared pattern");
        let unused = add_layer("Unused walls", "ByLayer");
        let mut add_line = |layer: SelectionRecordKey, name: &str, x: f64| {
            let mut line = Line::from_points(Vector3::ZERO, Vector3::new(x, 0.0, 0.0));
            line.common.layer = name.into();
            line.common.layer_handle = Some(layer.handle);
            source.add_entity(EntityType::Line(line)).unwrap()
        };
        let a = add_line(shared, "Shared walls", 1.0);
        let b = add_line(shared, "Shared walls", 2.0);
        let blocked_shared = add_line(shared, "Shared walls", f64::NAN);
        let blocked_unused = add_line(unused, "Unused walls", f64::NAN);
        let selection = DocumentSelection::capture(
            &source,
            &CaptureRequest::new("source", vec![a, b, blocked_shared, blocked_unused]),
        )
        .unwrap()
        .selection;
        let target = Document::new();
        let request = ReconstructionRequest {
            model_space: target.header.model_space_block_handle,
            coordinates: CoordinatePolicy::DrawingCoordinates,
            roots: selection
                .roots()
                .iter()
                .copied()
                .map(|root| RootReconstruction {
                    root,
                    transform: SelectionPlacement::Identity,
                    order: SelectionDrawOrder::Append,
                    layer: None,
                    linetype: None,
                    appearance: RootAppearanceOverrides::default(),
                })
                .collect(),
            resources: BTreeMap::from([
                (
                    shared,
                    ResourceAction::Create {
                        name: "Restored walls".into(),
                    },
                ),
                // The blocked-only resource must not compete for this destination name.
                (
                    unused,
                    ResourceAction::Create {
                        name: "Restored walls".into(),
                    },
                ),
                (
                    pattern_key,
                    ResourceAction::Create {
                        name: "Restored pattern".into(),
                    },
                ),
            ]),
            external: vec![],
        };

        let index = SourceIndex::new(&target);
        let order = order::Order::read(&target, request.model_space, &index).unwrap();
        let records = selection
            .records()
            .iter()
            .map(|r| (r.key, &r.payload))
            .collect();
        let mut catalog = resources::Catalog::new(&selection, &target, &index, &request, records);
        let prepare = |catalog: &mut resources::Catalog<'_>, i: usize| {
            plan::prepare_root(&selection, catalog, &order, &request.roots[i])
        };
        let layer_buffer = |catalog: &resources::Catalog<'_>| {
            let Some(SelectionPayload::Layer(layer)) = &catalog.planned[&shared].payload else {
                panic!("expected the planned shared layer");
            };
            layer.description.as_ptr()
        };
        let first = prepare(&mut catalog, 0).unwrap();
        let buffer = layer_buffer(&catalog);
        let second = prepare(&mut catalog, 1).unwrap();
        assert_eq!(
            layer_buffer(&catalog),
            buffer,
            "the shared payload was replanned"
        );
        let expected = BTreeSet::from([shared, pattern_key]);
        assert_eq!(first.resources, expected);
        assert_eq!(second.resources, expected);
        for i in [2, 3] {
            assert!(
                matches!(prepare(&mut catalog, i), Err(b) if b.kind == ReconstructionBlockerKind::InvalidGeometry)
            );
        }
        assert!(catalog.planned.contains_key(&unused));

        let outcome = run(&selection, &target, &request).unwrap();
        assert_eq!(
            outcome.resources.keys().copied().collect::<BTreeSet<_>>(),
            expected
        );
        assert_eq!(outcome.document.next_handle(), target.next_handle() + 4);
        let layer_handle = outcome.resources[&shared];
        for root in &outcome.roots[..2] {
            assert!(
                matches!(root.status, RootReconstructionStatus::Applied { layer, .. } if layer == layer_handle)
            );
        }
        for root in &outcome.roots[2..] {
            assert!(
                matches!(&root.status, RootReconstructionStatus::Blocked { reasons }
                if reasons[0].kind == ReconstructionBlockerKind::InvalidGeometry)
            );
        }
        let layer = outcome.document.layers.get("Restored walls").unwrap();
        assert_eq!(layer.description, description);
        assert_eq!(layer.line_type, "Restored pattern");
    }
}
