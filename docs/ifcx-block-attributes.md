# IFCX block attributes

This development mapping targets IFCCAD profile 0.6.0 / CBOR mapping 6.
Cargo.toml pins the matching native-model development revision.

## Stored content

ATTDEF maps to a native attribute definition. An INSERT owns an ordered list of
attribute values with drawing-wide identities. Tags and order are retained;
duplicate or renamed tags do not invent a definition association. A reliable
in-memory ATTDEF handle becomes an optional explicit definition reference.

Definitions and values retain flags, prompts/defaults where applicable,
position locks, advisory field length, text styles and placement. The drawing
model keeps no attribute thickness: a non-zero thickness read from IFCX is
reported as a loss. The outer Text and active embedded MText are separate records with
independent content and presentation. Empty active MText stays empty even when
the outer fallback contains older text.

Attribute coordinates already belong to the INSERT owner's scope. Import does
not apply the INSERT transform to them a second time. Constant definitions need
no invented child. Conversion performs neither ATTSYNC nor field evaluation.

Stored Text and MTextAttribute annotation contexts use the shared scale
registry. Outer and embedded context scales remain independent, including
disabled intent with retained presentation data.

## Selected fields and relationships

The bounded field mapping covers text containers, AcVar Filename and AcObjProp
Line.Length. It retains typed code parts, child/object dependencies, cached
values, status and selected auxiliary metadata. Import allocates identities
before rebinding field objects and their owner dictionaries. It does not call
editor helpers that evaluate fields or attach attributes by tag.

Other evaluators are rejected rather than replaced by cached literal text.
Broken required references are structural errors. Native secondary attribute
links are distinct from definition links and ownership.

## Physical transport boundaries

- AC1032 DWG and ASCII/binary DXF are the selected physical cases.
- Physical DWG/DXF does not store the codec's strong in-memory ATTDEF pointer.
  Export diagnoses its loss; readback does not guess a replacement from tags.
- Physical storage normalizes outer Text fallback to active embedded MText.
  The normalization is diagnosed, including the empty-active-MText case.
- AC1032 omits the separate pre-2007 Field.format scalar. Formatting in field
  code and typed caches remains available; the scalar loss is diagnosed.
- Generic native secondary links have no qualified CAD transport. Legacy
  ACAD_MLATT pointers instead describe the split lines of one multiline
  attribute; the focused AC1027 regression checks folding without duplication.
- MINSERT, dynamic blocks and general field evaluation remain outside this
  contribution. Existing unsupported source semantics still produce losses.

## Qualification and reproduction

Focused tests are `ifcx_block_attributes`, `ifcx_attribute_fields` and
`ifcx_attribute_gallery`, using `--features ifcx`. The gallery covers Text/MText,
flags, constant defaults, renamed tags, independent placement, nesting, Paper
layout and the selected contexts and fields. Its physical outputs are read
through production CAD readers and strict native validation.

`block_attributes_gallery` creates the grouped native input. `attribute_review`
produces the selected gallery and sample roundtrip files. Both examples require
the `ifcx` feature. The selected practice drawing is sample_AC1032; this is not
a corpus-wide or all-CAD-version fidelity claim.

Local editing and ATTSYNC qualification belongs to the CAD host. It is separate
from the storage/conversion contract and does not make conversion an editing
or synchronization engine.
