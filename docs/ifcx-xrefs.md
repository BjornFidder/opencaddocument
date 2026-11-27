# External sources and XREF placements

The development mapping pairs with IFCCAD profile 0.13.0 / CBOR 13. The core
owns native source/location validation. Document maps DWG XREF definitions to
drawing-local external sources and INSERTs to native XREF placements.
`import_ifccad_preserving` and `export_ifccad_preserving` retain native source
and entity IDs, original URI spelling and optional expected FingerprintGUID.
Context history retains earlier authored locators for undo after a native save.
Resolving a file at another location does not implicitly rewrite its locator.

Supported placements use the shared block transform: an orthonormal owner
frame, rotation in radians and finite nonzero signed XYZ effective scale.
Model, Paper layouts and local blocks may own XREFs. Scale includes insertion
unit conversion once. INSERT coordinates use CAD OCS; native O belongs to the
owner frame. Frame angle and explicit rotation combine once. Source base
points belong to the external source and are subtracted once by the host.
Source Model alone is loaded even for Paper owners. Unknown units stay absent.
A definition without an INSERT is retained. Loaded external bodies and their
dependent blocks are not emitted as authored native geometry.

Attach and Overlay can share one native source ID. CAD requires distinct block
records for the two relationship types; context retains their shared source ID
when saving native files. A standalone CAD roundtrip has distinct source IDs for
those definitions: path/GUID equality never silently merges authored identities.
FileDepList identity is emitted once per source, with a count across all owners.

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

Arrays and INSERT attributes remain explicit conversion losses. Nesting and
cycles retain the existing host-owned graph behavior. Binding, packaging,
remote download and additional source formats are outside this slice. The core
stays independent of CAD runtimes; Document owns CAD conversion and the host
owns resolution and IO. This is an unpublished native13 draft with an explicit
local IFCCAD patch, pending native staging and final downstream pins.
