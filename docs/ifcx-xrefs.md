# External sources and Model XREF attachments

The development mapping pairs with IFCCAD profile 0.12.0 / CBOR 12. The core
owns native source/location validation. Document maps DWG XREF definitions to
drawing-local external sources and INSERTs to native XREF placements.
`import_ifccad_preserving` and `export_ifccad_preserving` retain native source
and entity IDs, original URI spelling and optional expected FingerprintGUID.
Context history retains earlier authored locators for undo after a native save.
Resolving a file at another location does not implicitly rewrite its locator.

Supported placements are Model attachments with Z rotation and positive uniform
effective scale. The scale already includes any insertion-unit conversion.
Neither direction multiplies the unit factor again. Source base points belong
to the external source; placement is stored once as O/X/Y. Unknown offline
source units remain absent. A source definition without an INSERT is retained.
Loaded external bodies are not emitted as native blocks or Model entities.

The pure location helpers distinguish a direct URI-reference from a search
filename. Relative CAD paths map to escaped relative URI-references; drive
absolute and UNC paths map to `file:` URIs. A literal filename percent sign is
escaped once, and URI escapes are decoded once. A direct one-component relative
URI exports with `./`, keeping its direct lookup meaning on reimport. Native
URI spellings are retained in context. URI schemes without a qualified CAD
mapping, queries and fragments remain unresolved editing references; DWG/DXF
file export reports an error rather than silently changing their meaning.

DWG expected identity comes from the typed codec FileDepList using its
`Acad:XRef` feature and stored filename. Multiple matching records are an
explicit ambiguity error. The main drawing header GUID is independent.
The native expected GUID is optional and canonicalized from a valid stored
DWG GUID. `dwg_external_source_content` reads actual opened-source header
metadata; it never generates an identity.

Physical DWG tests cover AC1018, AC1021 and AC1032 dependency identity.
Physical DXF return currently has no FileDepList GUID. This matches the existing
Document/OCS DWG-to-DXF policy and adds no special GUID-loss message. This
local writer/reader evidence does not replace independent AutoCAD/ODA fixture
qualification, which remains pending in the first-slice working record.

The matched host has also been checked with LibreDWG geometry/header readback
and an independent literal FileDepList section parser. The latter is necessary
for empty fields and the legacy feature list because of identified LibreDWG
decoder limitations. An AutoCAD-authored dependency identity fixture remains
separate qualification. Extra physical dependency metadata and unbound records
produce native conversion loss diagnostics; the first native source model
retains only the qualified expected FingerprintGUID.

Overlay, arrays, Paper ownership, XREF inside a local block, nonuniform or
reflected scaling and INSERT attributes produce explicit conversion losses.
Native core validation enforces the same placement/ownership boundary.
Nested reference graphs, binding, packaging, remote download and additional
source formats remain outside this slice. The core stays independent of CAD
runtimes; Document owns CAD conversion, and the host owns resolution and I/O.
