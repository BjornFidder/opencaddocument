# MLINE preservation in layouts and local block definitions

Enable `mline-pilot` to capture supported MLINEs into one shared package per
layout or local block definition when writing IFCCAD. Each live opaque
entity has a reciprocal complete record in preservation envelope 2. Its
`storageBacking` selects a subject-free Shared record. The Shared record
uses provider `opencaddocument`, revision `mline-selection-1` and payload
schema `opencaddocument.document-selection`, kind `AdapterSnapshot`.
ModelSpace uses payload version 1, PaperSpace uses version 2, and local block
definitions use version 3.
The bytes use the corresponding [DocumentSelection storage](selection-storage.md).

The Shared source key is the selection namespace. An entity record uses
`<namespace>/entity/<positive-decimal-source-handle>`. Import verifies the
namespace and the selected MLINE root. It requests only live opaque roots,
with current native layer, appearance and draw order in their owning layout
or definition. A shared package is scoped to one owner. Supported style
dependencies are shared within the selection. Root-local capture diagnostics
report missing dependencies and typed capture exclusions.

A `root-linetype` binding with a missing native target blocks that root.
Present resources follow their session identity. The original typed ID stays
reserved when a resource disappears. Native appearance takes precedence for
the rendered linetype once the declared binding is satisfied.

Use `Ifcx::import_ifccad_preserving` and keep its context with the editable
Document. Pass it to `Ifcx::export_ifccad_preserving` on save. Applied MLINEs
are captured again from the current Document. Blocked roots keep their
original package bytes, live records and owner. Layout and block identities
follow their CAD handles during a session. A local block member is restored in
definition coordinates and all existing INSERTs use that definition with their
current transforms. Resource reuse and creation follow the existing
DocumentSelection safety rules. Save reports an error if an owner with
retained roots is unavailable. Deleting an applied MLINE removes its
live root; an older package retained for another root may still contain its
historical capture. The core can transport unknown provider shapes. Editable
import covers supported MLINEs in ModelSpace, PaperSpace and local block
definitions.
