# Owned document selections

`selection::DocumentSelection` captures explicit ModelSpace `Line` and `MLine`
roots and their supported dependencies without retaining the source `Document`.
Enable the `selection` Cargo feature for capture and reconstruction. The API
is a building block for preservation, clipboard transfer and selective exchange.
Enable `selection-storage` to
[write and reopen a selection](selection-storage.md) as versioned bytes.
Use [reconstruction](selection-reconstruction.md) to apply explicit roots to a current drawing.

```rust
use opencaddocument::{Document, entities::{EntityType, MLine}, selection::{
    CaptureRequest, DocumentSelection, SelectionPayload,
}};

let selection = {
    let mut source = Document::new();
    let handle = source.add_entity(EntityType::MLine(MLine::new()))?;
    let request = CaptureRequest::new("source-drawing-123", vec![handle]);
    DocumentSelection::capture(&source, &request)?.selection
}; // source has been dropped

assert_eq!(selection.roots().len(), 1);
assert!(selection.records().iter().any(|record|
    matches!(record.payload, SelectionPayload::MLineStyle(_))));
// Inspect capture_report() and requirements(), including any Missing entries.
# Ok::<(), Box<dyn std::error::Error>>(())
```

Choose a namespace that identifies the source identity domain. A record key is
only local to that namespace; the same numeric handle in another selection is
not proof of shared identity. Handles retain their full `u64` precision. Capture
does not assign destination handles, mutate the source or repair its references.
Floating-point geometry is copied without normalization, including signed zero
and NaN payload bits. This is capture, not a drawing-validity certification.

Roots are ordered, unique, non-null existing drawable identities. Owning
ModelSpace is context, so selecting one entity does not select its siblings.
Only ModelSpace roots are supported. Included record identities are unique;
every incoming reference slot remains separate. For example, two MLINE roots
sharing a layer and a style produce one layer and one style record, but two
root-layer requirements and two root-style requirements. Style elements retain
their own linetype requirements, even when they share one pattern.

## Dependency boundaries and gaps

Supported dependencies are ordinary layers, simple linetypes and
`objects::MLineStyle`. `SelectionRequirement` identifies its source record and
slot, the original/effective source handle and name, and one disposition:

- `Included(key)`: its typed payload is in `records()`.
- `External(key)`: an existing dependency was deliberately left to the caller.
- `Missing`: the source reference cannot be resolved in its supported role.
- `Context(role)`: ModelSpace ownership, style-dictionary registration or the
  ByLayer/ByBlock inheritance role; no complete owner/dictionary is captured.

To leave a layer external, put its `SelectionRecordKey` in
`CaptureRequest::external` before capture. Each referencing root keeps a separate
external requirement; an application can later bind each root to its actual
current layer. The external record's content and outgoing closure are not copied
or audited as included content. Capture still checks the endpoint's identity,
type and any supplied name/handle coherence. This is not proof that a future
binding is geometrically compatible. Roots cannot be external; unused boundary
keys are request errors. A requested boundary key reached through a dangling
reference remains `Missing`, rather than becoming a valid external endpoint.

The production DWG/DXF route can attach a `Normal` plot-style object reference
even to an otherwise ordinary layer 0. Copying that layer is outside the current
owned-layer boundary. Bind it externally, as an IFCCAD caller can do for its
native layer, or handle the capture error. D1 does not silently remove the
plot-style reference or claim generic preservation of such layer payloads.

Missing supported references are preserved as located gaps with evidence. A
dangling non-null handle never falls back to a matching name. A handle resolving
to another kind or disagreeing with a supplied nonempty name is a conflict.
Ambiguous names/identities are errors. Names are compared case-insensitively
using uppercase Unicode, matching the source table lookup convention; original
spelling is preserved in the owned payload and requirements.

An absent/null MLINE style handle uses the actual source header's current style
with explicit evidence. The owned MLINE keeps its original handle field; its
style requirement carries the effective source handle. A missing current style
is a gap, and a contradictory style name remains a conflict. No destination's
`Standard` style or other default is consulted. Included styles capture only
their relevant named registration in the source `ACAD_MLINESTYLE` dictionary
role. Other dictionary entries, the named-object dictionary and unrelated
styles are not included. Units and global linetype scale are source context;
unitless or unrecognized units are reported without guessing.

## Coverage and evidence

Capture owns the typed payload, not verbatim DWG storage. The copied entity's
`raw_record`, `graphic_data` and derived `has_ds_data` are excluded, each with
evidence when present. The source retains them. Meaningful unreviewed content
such as XDATA/EED, reactors, extension dictionaries, material/visual-style/color
object references, R12 metadata, xrefs and complex or DGN linetypes is rejected
on included records. Selected style and table-record attachments are also
checked through the semantic inventory and public side views. Unrelated
unsupported entities do not prevent capture of supported roots.

`SelectionError` locates capture failures; there is no partially returned
selection on error. The immutable selection holds the authoritative report
through `capture_report()`; there is no duplicate mutable report. A successful
capture with `Missing` requirements is intentionally incomplete and does not
promise successful reconstruction. See the [field audit](selection-fields.md)
for the exact supported boundary.
