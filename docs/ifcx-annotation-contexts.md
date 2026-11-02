# IFCX annotation contexts

This contribution maps the unpublished IFCCAD profile 0.4.0 / CBOR mapping 4.
It requires the matching native model/reader and the HatchView SoftPointer codec
fix. The existing manifest pins are deliberately retained until those sources
are available as portable upstream revisions. Dependency pin updates are required before merge;
an unpatched checkout with the old native dependency cannot build the new API.

The initial accepted development pair starts from Document
`28f1ad1214816e6b5c21162b732df0604bdf38bd`, native
`8367458a2976346fabab1659ad9ea5fcde8a19a0` plus the annotation contract, and codec
`0a5542baa6052a2b07fc700b7ba041549650a997` plus the HatchView pointer correction.
The native package version alone does not distinguish this development contract
from the released profile 0.3.0. Use explicit local Cargo patches for both native
and codec checkouts; do not edit Cargo's source cache. The contribution is now based on Document main
`3a3676b12e824292bcf46b29f4eaf02d9264c3a4`, selecting codec `ef27529c` with
the same explicit HatchView pointer correction. The full serde/IFCX suite is
checked against that pair before submission. Explorer's accepted CAD review
files and embedded-host build retain their separately recorded earlier pair.

## Retained state

- Drawing-local scales retain names, both authored units, unit designation and
  unused identities; equal ratios remain separate definitions.
- Text and MText retain memberships, independent default, explicit object mode
  and paper orientation, and saved anchors/directions/rectangles/columns.
- All nine Dimension kinds retain contextual XYZ points, text choices, five
  binary override hints with independent activity/value bits, arrow hints and
  independent saved pictures. Canonical geometry, overrides and source links
  remain separate from those snapshots.
- Hatch scale and view collections retain independent defaults. View leaves keep
  repeated/null targets, named views/model-window/Paper-viewport identities,
  view state, explicit pattern values and shared/local boundaries with fourteen
  named classification flags.
- Block references retain full contextual transforms and definition policy.
  TextStyle/BlockDefinition creation policy does not retroactively annotate text.
- Saved drawing policies retain ANNOTATIVEDWG and MSLTSCALE; Model and Paper
  visibility/current scale remain independent. Paper's current scale is owned
  by its overall ID1 viewport, independently of floating viewport choices. Named
  VIEW has its own scale; Model VPORT windows share Model's choice.

## Boundaries and diagnostics

No layout regeneration or editing engine is introduced. Unknown Dimension
reserved bits refuse conversion; binary hints are not promoted to the style's
multi-valued fitting/movement enums. ArcLength reuses the Angular context
mapping under the accepted SDK-based assumption; no independently authored
multi-scale ArcLength source is available yet.

Native ByDefinition with memberships materializes its current effective CAD
object flag and reports the lost future inheritance. Nonempty local Hatch
context source IDs and dormant MText column heights without CAD fields report
explicit losses. Nonzero MText layout-extents caches are omitted with a diagnostic;
rectangles/column semantics remain retained. Unsupported object families stay
outside this contribution. Context pictures use the existing bounded graphics
qualifier and retain its numerical/rendering limitations.

Original BricsCAD files in [this public attachment](https://forum.bricsys.com/discussion/36137/can-i-have-different-multileader-text-box-sizes-at-different-annotation-scales)
prove ANNOTATIVEDWG values 1/0 as DICTIONARYVAR entries under
AcDbVariableDictionary. It is not the annotative marker of ModelSpace's block
record. Those downloaded source files are not committed here.

## Verification

`ifcx_annotation`, `ifcx_annotation_dimensions` and `ifcx_annotation_policy`
exercise memory, actual DWG, ASCII/binary DXF and strict native JSON/CBOR
readback. They compare semantic values and independent defaults rather than
unstable handles. The compact `annotation_gallery` example reproduces the
accepted CAD review files. Native and Explorer qualification remain distinct.
The paired focused suite has 19 passing tests; the user accepted the small CAD
roundtrips and reported opening/converting existing DWG/DXF in local Explorer.
The embedded Studio rebuild and final integration gates are not claimed complete.
