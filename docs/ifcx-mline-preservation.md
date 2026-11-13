# ModelSpace MLINE preservation

Enable `mline-pilot` to capture the supported ModelSpace MLINEs into one
shared package per capture group when writing IFCCAD. Each live opaque
entity has a reciprocal complete record in preservation envelope 2. Its
`storageBacking` selects a subject-free Shared record. The Shared record
uses provider `opencaddocument`, revision `mline-selection-1` and payload
schema `opencaddocument.document-selection` version 1, kind
`AdapterSnapshot`. Its bytes use [DocumentSelection storage V1](selection-storage.md).

The Shared source key is the selection namespace. An entity record uses
`<namespace>/entity/<positive-decimal-source-handle>`. Import verifies the
namespace and the selected MLINE root. It requests only live opaque roots,
with current native layer, appearance and draw order. Supported style
dependencies are shared within the selection. Root-local capture diagnostics
report missing dependencies and typed capture exclusions.

A `root-linetype` binding with a missing native target blocks that root.
Present resources follow their session identity. The original typed ID stays
reserved when a resource disappears. Native appearance takes precedence for
the rendered linetype once the declared binding is satisfied.

Use `Ifcx::import_ifccad_preserving` and keep its context with the editable
Document. Pass it to `Ifcx::export_ifccad_preserving` on save. Applied MLINEs
are captured again from the current Document. Blocked roots keep their
original package bytes and live records. Deleting an applied MLINE removes its
live root; an older package retained for another root may still contain its
historical capture. Unknown provider shapes are transportable through the
core; editable import is limited to the supported ModelSpace MLINE scope.
