# ModelSpace MLINE preservation

Enable `mline-pilot` to capture ModelSpace MLINEs when writing IFCCAD. Each
opaque entity points to one complete entity record in preservation envelope 1.
The record uses provider `opencaddocument`, provider revision `mline-pilot-1`,
and payload schema `opencaddocument.mline-pilot` version 1 with kind
`AdapterSnapshot`. The payload bytes use
[DocumentSelection storage version 1](selection-storage.md).

The selection contains one MLINE root. Its namespace matches the record's
source ID and its original CAD handle, written in decimal, matches the source
key. The snapshot includes the style and its supported dependencies. The
drawing supplies the current layer and common appearance. Capture reports
unresolved dependencies through export diagnostics and writes coverage
`Unknown` when a dependency is missing. The selection records the unresolved
reference so it can be examined after reopening. Diagnostics also identify
excluded DWG source bytes, saved graphics and derived flags. These exclusions
are part of typed capture and do not by themselves change dependency coverage.

The writer emits snapshots without bindings or conditions. The reader also
accepts one optional `root-linetype` binding to a drawing line pattern. A
missing target blocks the MLINE. If a retained binding has an existing target,
export follows that target by its CAD handle and keeps its ID reserved if the
target disappears.

Use `Ifcx::import_ifccad_preserving` to obtain the editable document and its
roundtrip context. A valid snapshot that cannot be reconstructed stays in the
context with a reason. Pass the same context to
`Ifcx::export_ifccad_preserving` so the blocked record and its payload survive
while native entities can be edited. Context-free import fails if it would
have to discard a blocked root. The pilot also refuses editable import if
the envelope contains preservation outside its supported ModelSpace MLINE
scope. Invalid payloads and unsafe reconstruction targets produce errors.
