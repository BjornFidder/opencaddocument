# Reconstructing an owned selection

`DocumentSelection::reconstruct_available` adds explicitly requested
Line and MLINE roots to a candidate cloned from the current Document.
Enable the `selection` Cargo feature to use this API.
Capture or reopen the selection before calling it. Reopening uses the
optional `selection-storage` feature.

```rust
use std::collections::BTreeMap;
use opencaddocument::{
    Document, EntityType, Line, Vector3,
    selection::{
        CaptureRequest, CoordinatePolicy, DocumentSelection, ReconstructionRequest,
        RootAppearanceOverrides, RootReconstruction, RootReconstructionStatus,
        SelectionDrawOrder, SelectionPlacement, SelectionRecordKey, SelectionRecordKind,
    },
};

let mut source = Document::new();
let line = source.add_entity(EntityType::Line(Line::from_points(
    Vector3::ZERO, Vector3::new(1.0, 0.0, 0.0),
)))?;
let mut capture = CaptureRequest::new("source-drawing", vec![line]);
capture.external.insert(SelectionRecordKey {
    kind: SelectionRecordKind::Layer,
    handle: source.layers.get("0").unwrap().handle,
});
let selection = DocumentSelection::capture(&source, &capture)?.selection;
let target = Document::new();
let layer = target.layers.get("0").unwrap().handle;
let result = selection.reconstruct_available(&target, &ReconstructionRequest {
    model_space: target.header.model_space_block_handle,
    coordinates: CoordinatePolicy::DrawingCoordinates,
    roots: vec![RootReconstruction {
        root: selection.roots()[0],
        transform: SelectionPlacement::Identity,
        order: SelectionDrawOrder::Append,
        layer: Some(layer),
        linetype: None,
        appearance: RootAppearanceOverrides::default(),
    }],
    resources: BTreeMap::new(),
    external: vec![],
})?;
match &result.roots[0].status {
    RootReconstructionStatus::Applied { entity, .. } => {
        assert!(result.document.get_entity(*entity).is_some());
    }
    RootReconstructionStatus::Blocked { reasons } => panic!("{reasons:?}"),
}
assert_eq!(target.entity_count(), 0);
# Ok::<(), Box<dyn std::error::Error>>(())
```

The result contains one status in request order per root, with the new entity
and effective layer handles for Applied roots. The resource map identifies
the included records used by those roots, including explicitly reused
resources. Each included resource is planned once per request. Roots keep
only resource keys for shared dependencies. A blocked root contributes no
records. The caller retains the
selection and decides whether to use the candidate. A request with no applied
roots leaves the allocator floors unchanged. Invalid requests, unsafe target
state, reservation failures and construction failures return an error.

## Current bindings and resources

Per-root layer, linetype, color, line weight, transparency and visibility
choices take precedence when supplied. An external dependency needs a
binding at its source record and slot. A missing source style cannot be
recovered from the target default. For an included named resource supply
an explicit reuse or create action. MLINESTYLE reuse compares the captured
style after mapping the linetypes of its ordered elements. A creation uses
the requested name in the style dictionary and in every rebound MLINE.

The creation check compares case-insensitive names against both occupied
names and existing references. New names must also fit DWG text lengths
in the target version and encoding. The check inspects typed Line and MLINE records,
block markers, qualified tables and constructor scaffold, including the
current layer, linetype and multiline style names in the header. A target
with an unqualified name carrier blocks creation. For instance, an existing
entity with a dangling style name reserves that name against new creation.
An independently valid existing-resource binding can still be used. The
numeric allocator separately checks global handle coverage and protects
dangling numeric endpoints.

## Geometry and draw order

The first reconstruction scope is ModelSpace. `Identity` retains captured
geometry bits. `Translate` changes points while retaining directions, normals,
miter vectors and segment parameters. The source and target must have equal
positive finite global linetype scales. `MatchingUnits` requires equal known
units. `DrawingCoordinates` preserves numeric coordinates across drawing
units. Invalid geometry, unsupported references and stale anchors produce
located root blockers.

Append uses the natural order when the target stores agree. Insertion before
a live ModelSpace entity uses the current effective order. Rebuilding the
typed SortEnts order covers existing fallback ranks as well as new entities
so that DXF rank renumbering retains the relative order. DWG and DXF readback
tests exercise a populated target and sparse existing SortEnts. The allocator
refuses a target containing unqualified raw DXF sort codes. That error leaves
the original data intact.

The candidate has independent drawing content. Its allocator reserves one
batch for the required entities, resources and scaffold. Reserved handles are
local to that candidate. History or preservation retention remains the
responsibility of the caller.
